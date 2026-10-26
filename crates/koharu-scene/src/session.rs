//! The session: the only owner of an open project's document.

use std::{
    collections::{BTreeSet, HashSet},
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use loro::{
    CommitOptions, ExportMode, LoroDoc, LoroValue, Subscription, TreeID, UndoItemMeta, UndoManager,
    event::{Diff, DiffEvent},
};

use crate::{
    Change, Edit, Error, History, LayerId, PageChanges, PageId, ProjectId, Result,
    doc::{self, Key},
    op::{self, Planned},
    state::{Index, State},
};

/// One open project. Edits, undo, and redo go through it, and it saves when
/// its caller says so: when edits pause, and when the project closes.
pub struct Session {
    file: koharu_storage::Document,
    pub(crate) doc: LoroDoc,
    pub(crate) id: ProjectId,
    /// Page order, layer pages, and files, kept so nothing rescans.
    pub(crate) index: Index,
    undo: UndoManager,
    /// The label of the undo step being pushed.
    label: Arc<Mutex<String>>,
    /// Where undo and redo landed; recorded only while one runs.
    events: Arc<Events>,
    _subscription: Subscription,
    /// Set when a write failed partway; nothing more is written or saved.
    poisoned: bool,
}

#[derive(Default)]
struct Events {
    recording: AtomicBool,
    touched: Mutex<Touched>,
}

#[derive(Default)]
struct Touched {
    pages: BTreeSet<usize>,
    nodes: Vec<(usize, TreeID)>,
    layers: BTreeSet<LayerId>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Session {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let doc = LoroDoc::new();
        doc::configure(&doc);
        let id = ProjectId::new();
        let project = doc::project(&doc);
        project.insert(Key::Id.as_ref(), id.to_string())?;
        project.insert(Key::Version.as_ref(), doc::VERSION.to_string())?;
        doc.commit();
        let file = koharu_storage::Document::create(path, &doc.export(ExportMode::Snapshot)?)?;
        Ok(Self::assemble(file, doc, id, Index::default()))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let (file, doc) = koharu_storage::Document::open(path)?;
        doc::configure(&doc);
        let (id, index) = validate(&doc)?;
        Ok(Self::assemble(file, doc, id, index))
    }

    fn assemble(file: koharu_storage::Document, doc: LoroDoc, id: ProjectId, index: Index) -> Self {
        let label = Arc::new(Mutex::new(String::new()));
        let mut undo = UndoManager::new(&doc);
        let pushed = label.clone();
        undo.set_on_push(Some(Box::new(move |_, _, _| UndoItemMeta {
            value: LoroValue::from(lock(&pushed).clone()),
            cursors: Vec::new(),
        })));
        let popped = label.clone();
        undo.set_on_pop(Some(Box::new(move |_, _, meta| {
            if let LoroValue::String(value) = meta.value {
                *lock(&popped) = value.to_string();
            }
        })));
        let events = Arc::new(Events::default());
        let recorder = events.clone();
        let subscription = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
            if !recorder.recording.load(Ordering::Relaxed) {
                return;
            }
            let mut touched = lock(&recorder.touched);
            for diff in &event.events {
                let (page, nodes, layer) = doc::locate(diff.path);
                touched.layers.extend(layer);
                let Some(page) = page else {
                    continue;
                };
                touched.pages.insert(page);
                touched
                    .nodes
                    .extend(nodes.into_iter().map(|node| (page, node)));
                match &diff.diff {
                    Diff::Tree(tree) => touched
                        .nodes
                        .extend(tree.diff.iter().map(|item| (page, item.target))),
                    Diff::Map(map) if doc::is_store(diff.path) => {
                        touched.layers.extend(
                            map.updated
                                .keys()
                                .filter_map(|key| key.parse::<LayerId>().ok()),
                        );
                    }
                    _ => {}
                }
            }
        }));
        Self {
            file,
            doc,
            id,
            index,
            undo,
            label,
            events,
            _subscription: subscription,
            poisoned: false,
        }
    }

    #[must_use]
    pub fn id(&self) -> ProjectId {
        self.id
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    #[must_use]
    pub fn revision(&self) -> crate::Revision {
        doc::revision(&self.doc)
    }

    /// Saves with history. Containers never read are exported from their
    /// loaded bytes, so this decodes no pixels.
    pub fn save(&mut self) -> Result<()> {
        self.usable()?;
        Ok(self.file.save(&self.doc.export(ExportMode::Snapshot)?)?)
    }

    /// Saves without history, dropping files and tiles nothing names any
    /// more. Loro decodes every container to do it, so it suits rare moments
    /// such as closing.
    pub fn compact(&mut self) -> Result<()> {
        self.usable()?;
        let frontiers = self.doc.oplog_frontiers();
        Ok(self
            .file
            .save(&self.doc.export(ExportMode::shallow_snapshot(&frontiers))?)?)
    }

    #[must_use]
    pub fn history(&self) -> History {
        let label = |meta: Option<UndoItemMeta>| match meta?.value {
            LoroValue::String(value) => Some(value.to_string()),
            _ => None,
        };
        History {
            undo: label(self.undo.top_undo_meta()),
            redo: label(self.undo.top_redo_meta()),
        }
    }

    /// Checks `edit` in full, then writes it as one undo step.
    pub fn apply(&mut self, edit: Edit) -> Result<Change> {
        self.usable()?;
        if edit.project != self.id {
            return Err(Error::WrongProject {
                expected: self.id,
                got: edit.project,
            });
        }
        let Planned {
            writes,
            created,
            kept,
            pages,
            layers,
            index,
        } = op::plan(&self.doc, &self.index, &edit)?;
        if !writes.is_empty() {
            if let Err(error) = op::execute(&self.doc, writes) {
                self.poisoned = true;
                return Err(error);
            }
            *lock(&self.label) = edit.label.clone();
            self.doc
                .commit_with(CommitOptions::new().commit_msg(&edit.label));
            if let Some(index) = index {
                self.index = index;
            }
        }
        Ok(Change {
            revision: self.revision(),
            created,
            pages,
            layers,
            kept,
            history: self.history(),
        })
    }

    pub fn undo(&mut self) -> Result<Option<Change>> {
        self.step(UndoManager::undo)
    }

    pub fn redo(&mut self) -> Result<Option<Change>> {
        self.step(UndoManager::redo)
    }

    fn step(
        &mut self,
        step: fn(&mut UndoManager) -> loro::LoroResult<bool>,
    ) -> Result<Option<Change>> {
        self.usable()?;
        *lock(&self.events.touched) = Touched::default();
        self.events.recording.store(true, Ordering::Relaxed);
        // A failed undo or redo rolls back and keeps its step (Loro 1.17).
        let stepped = step(&mut self.undo);
        self.events.recording.store(false, Ordering::Relaxed);
        if !stepped? {
            return Ok(None);
        }
        let touched = std::mem::take(&mut *lock(&self.events.touched));
        // The document already changed, so an index that can't be rebuilt
        // must stop further edits.
        let after = Index::load(&self.doc).inspect_err(|_| self.poisoned = true)?;
        let before = std::mem::replace(&mut self.index, after);
        let after = &self.index;

        let maps = doc::page_maps(&self.doc)?;
        let mut layers = touched.layers;
        for (page, node) in touched.nodes {
            if let Some(map) = maps.get(page)
                && let Ok(meta) = doc::tree(map).and_then(|tree| Ok(tree.get_meta(node)?))
                && let Ok(id) = doc::id(&meta)
            {
                layers.insert(id);
            }
        }
        let before_layers: HashSet<&LayerId> = before.layers.keys().collect();
        let after_layers: HashSet<&LayerId> = after.layers.keys().collect();
        layers.extend(before_layers.symmetric_difference(&after_layers).copied());

        let mut changed: BTreeSet<PageId> = touched
            .pages
            .iter()
            .filter_map(|page| after.pages.get(*page).copied())
            .collect();
        changed.extend(
            layers
                .iter()
                .filter_map(|layer| after.layers.get(layer).or_else(|| before.layers.get(layer))),
        );
        let (before_pages, after_pages): (HashSet<_>, HashSet<_>) =
            (before.pages.iter().collect(), after.pages.iter().collect());
        let added: Vec<PageId> = after
            .pages
            .iter()
            .filter(|page| !before_pages.contains(page))
            .copied()
            .collect();
        let removed: Vec<PageId> = before
            .pages
            .iter()
            .filter(|page| !after_pages.contains(page))
            .copied()
            .collect();
        let kept_before = before
            .pages
            .iter()
            .filter(|page| after_pages.contains(page));
        let kept_after = after
            .pages
            .iter()
            .filter(|page| before_pages.contains(page));
        let reordered = !kept_before.eq(kept_after);
        changed.retain(|page| after_pages.contains(page) && !added.contains(page));
        Ok(Some(Change {
            revision: self.revision(),
            created: Vec::new(),
            pages: PageChanges {
                added,
                removed,
                reordered,
                changed,
            },
            layers,
            kept: Vec::new(),
            history: self.history(),
        }))
    }

    fn usable(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::Poisoned)
        } else {
            Ok(())
        }
    }
}

/// Checks a loaded document: a version this build reads, then every page and
/// layer, so unknown core data fails to open.
fn validate(doc: &LoroDoc) -> Result<(ProjectId, Index)> {
    // Koharu never writes a pre-release, which would sort below its release.
    let version = doc::version(doc)?;
    if version.major != doc::VERSION.major || version > doc::VERSION || !version.pre.is_empty() {
        return Err(Error::Version(version.to_string()));
    }
    let id = doc::id(&doc::project(doc))?;
    let index = Index::load(doc)?;
    let mut state = State::new(doc, &index);
    for page in &index.pages {
        for node in state.page(*page)?.nodes.values() {
            node.layer()?;
        }
    }
    Ok((id, index))
}
