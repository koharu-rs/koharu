//! The selected layers, in the order they were picked, the one the select
//! tool would pick under the pointer, and the anchors the direct selection
//! tool picked on the one selected text's frame. Tools read it and answer
//! the next ids; only the canvas sets them.

use koharu_scene::LayerId;

#[derive(Debug, Default)]
pub(crate) struct Selection {
    ids: Vec<LayerId>,
    hover: Option<LayerId>,
    /// Indices into the selected text's frame's anchors, ring by ring;
    /// another selection drops them.
    anchors: Vec<usize>,
}

impl Selection {
    pub fn ids(&self) -> &[LayerId] {
        &self.ids
    }

    pub fn contains(&self, id: LayerId) -> bool {
        self.ids.contains(&id)
    }

    /// The one selected layer, when exactly one is.
    pub fn single(&self) -> Option<LayerId> {
        match self.ids[..] {
            [id] => Some(id),
            _ => None,
        }
    }

    /// The ids with `id` taken out if it's selected, or added if not.
    pub fn toggled(&self, id: LayerId) -> Vec<LayerId> {
        match self.contains(id) {
            true => self
                .ids
                .iter()
                .copied()
                .filter(|&kept| kept != id)
                .collect(),
            false => self.ids.iter().copied().chain([id]).collect(),
        }
    }

    /// The ids with those of `ids` not already among them added.
    pub fn with(&self, ids: impl IntoIterator<Item = LayerId>) -> Vec<LayerId> {
        let mut next = self.ids.clone();
        for id in ids {
            if !next.contains(&id) {
                next.push(id);
            }
        }
        next
    }

    /// Whether `ids` changed the selection.
    pub fn set(&mut self, ids: Vec<LayerId>) -> bool {
        let changed = ids != self.ids;
        if changed {
            self.anchors.clear();
        }
        self.ids = ids;
        changed
    }

    pub fn anchors(&self) -> &[usize] {
        &self.anchors
    }

    pub fn set_anchors(&mut self, anchors: Vec<usize>) {
        self.anchors = anchors;
    }

    pub fn hover(&self) -> Option<LayerId> {
        self.hover
    }

    /// Whether `id` changed the hovered layer.
    pub fn set_hover(&mut self, id: Option<LayerId>) -> bool {
        let changed = id != self.hover;
        self.hover = id;
        changed
    }
}
