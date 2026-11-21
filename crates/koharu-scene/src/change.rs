//! What an edit, undo, or redo changed, found by comparing two projects.

use std::collections::{BTreeMap, BTreeSet};

use imbl::ordmap::DiffItem;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{Layer, LayerId, Page, PageId, Project, Revision, project::depth_first};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Created {
    Page { id: PageId, label: Option<String> },
    Layer { id: LayerId, label: Option<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct PageChanges {
    pub added: Vec<PageId>,
    pub removed: Vec<PageId>,
    pub reordered: bool,
    pub changed: BTreeSet<PageId>,
}

/// The session's steps, as Photoshop's History panel lists the states of the
/// current session, older first, and dims the ones a jump back would lose.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct History {
    /// Every step's undo label, oldest first.
    pub steps: Vec<String>,
    /// How many steps the project holds: 0 is the project as it opened, and
    /// the steps after it can be redone.
    pub current: usize,
}

impl History {
    /// The label of the step undo takes back.
    #[must_use]
    pub fn undo(&self) -> Option<&str> {
        self.current
            .checked_sub(1)
            .and_then(|at| self.steps.get(at))
            .map(String::as_str)
    }

    /// The label of the step redo makes again.
    #[must_use]
    pub fn redo(&self) -> Option<&str> {
        self.steps.get(self.current).map(String::as_str)
    }
}

/// What one edit, undo, or redo changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Change {
    pub revision: Revision,
    pub created: Vec<Created>,
    pub pages: PageChanges,
    /// Added, changed, or removed layers, pixels included. A layer that only
    /// moved among its siblings, or whose child list changed, changes its
    /// page, not itself.
    pub layers: BTreeSet<LayerId>,
    pub history: History,
}

/// A page's layers by id.
fn by_id(page: &Page) -> BTreeMap<LayerId, &Layer> {
    depth_first(&page.layers)
        .into_iter()
        .map(|layer| (layer.id, layer))
        .collect()
}

pub(crate) fn changed(before: &Project, after: &Project) -> (PageChanges, BTreeSet<LayerId>) {
    let mut layers = BTreeSet::new();
    let mut changed = BTreeSet::new();
    let (mut added, mut removed) = (Vec::new(), Vec::new());
    for item in before.pages.diff(&after.pages) {
        match item {
            DiffItem::Add(id, page) => {
                added.push(*id);
                layers.extend(by_id(page).into_keys());
            }
            DiffItem::Remove(id, page) => {
                removed.push(*id);
                layers.extend(by_id(page).into_keys());
            }
            // A layer that only moved, or whose children changed, changes
            // its page and not itself.
            DiffItem::Update { old, new } => {
                changed.insert(*new.0);
                let (old, new) = (by_id(old.1), by_id(new.1));
                for (id, layer) in &new {
                    if old.get(id).is_none_or(|was| was.own() != layer.own()) {
                        layers.insert(*id);
                    }
                }
                layers.extend(old.keys().filter(|id| !new.contains_key(id)));
            }
        }
    }
    for item in before.tiles.diff(&after.tiles) {
        let layer = match item {
            DiffItem::Add(layer, _) | DiffItem::Remove(layer, _) => *layer,
            DiffItem::Update { new, .. } => *new.0,
        };
        layers.insert(layer);
        let owner = after.index.get(&layer).or_else(|| before.index.get(&layer));
        changed.extend(owner.copied());
    }
    let stayed_before = before
        .order
        .iter()
        .filter(|page| after.pages.contains_key(*page));
    let stayed_after = after
        .order
        .iter()
        .filter(|page| before.pages.contains_key(*page));
    let reordered = !stayed_before.eq(stayed_after);
    changed.retain(|page| after.pages.contains_key(page) && !added.contains(page));
    (
        PageChanges {
            added,
            removed,
            reordered,
            changed,
        },
        layers,
    )
}
