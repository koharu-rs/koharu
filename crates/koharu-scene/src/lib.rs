//! Koharu's project model on Loro. A project is one Loro document (see
//! `document` for its layout). Readers get owned views; every change is an
//! [`Edit`], and each edit is one undo step.

mod document;
mod edit;
mod error;
mod schema;

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
};

use loro::{CommitOptions, ExportMode, Index, LoroDoc, Subscription, UndoManager};
use serde::Serialize;
use specta::Type;

use document::{ID, PageMap, SCHEMA, SCHEMA_KEY};

pub use edit::Edit;
pub use error::{Error, Result};
pub use schema::{
    DetectionLabel, FontStyle, Generation, Id, Image, Layer, LayerContent, LayerNode, Ocr, Origin,
    Page, PageSummary, PageView, Point, RasterRole, Region, RegionKind, Revision, Text,
    TextAlignment, TextDirection, TextLayout, Typography, WritingMode,
};

/// What one edit, undo, or redo changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Type)]
pub struct Change {
    pub revision: Revision,
    /// Pages were added, removed, or reordered.
    pub pages: bool,
    /// Pages whose content changed.
    pub content: BTreeSet<Id>,
}

/// One open project and the only owner of its document. Saving is the
/// caller's choice of moment, between edits.
pub struct Session {
    file: koharu_storage::Document,
    doc: LoroDoc,
    undo: UndoManager,
    touched: Arc<Mutex<Touched>>,
    _events: Subscription,
    /// Set when an edit failed after writing; nothing more is committed or
    /// saved.
    poisoned: bool,
}

#[derive(Default)]
struct Touched {
    pages: bool,
    content: BTreeSet<usize>,
}

impl Session {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let doc = LoroDoc::new();
        let project = document::project(&doc);
        project.insert(SCHEMA_KEY, SCHEMA)?;
        project.insert(ID, Id::new().to_string())?;
        doc.commit();
        let file = koharu_storage::Document::create(path, &doc.export(ExportMode::Snapshot)?)?;
        Ok(Self::assemble(file, doc))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let (file, doc) = koharu_storage::Document::open(path)?;
        document::validate(&doc)?;
        Ok(Self::assemble(file, doc))
    }

    fn assemble(file: koharu_storage::Document, doc: LoroDoc) -> Self {
        let touched = Arc::<Mutex<Touched>>::default();
        let recorder = touched.clone();
        let events = doc.subscribe_root(Arc::new(move |event| {
            let mut touched = recorder.lock().unwrap_or_else(PoisonError::into_inner);
            for diff in event.events {
                if document::is_page_list(diff.target) {
                    touched.pages = true;
                } else if let [(root, _), (_, Index::Seq(page)), ..] = diff.path
                    && document::is_page_list(root)
                {
                    touched.content.insert(*page);
                }
            }
        }));
        Self {
            undo: UndoManager::new(&doc),
            file,
            doc,
            touched,
            _events: events,
            poisoned: false,
        }
    }

    pub fn id(&self) -> Result<Id> {
        document::read_id(&document::project(&self.doc))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        Revision::of(&self.doc)
    }

    pub fn pages(&self) -> Result<Vec<PageSummary>> {
        document::summaries(&self.doc)
    }

    pub fn page(&self, id: Id) -> Result<PageView> {
        PageMap::find(&self.doc, id)?.1.view()
    }

    pub fn image(&self, page: Id, image: Id) -> Result<Image> {
        document::read_image(&PageMap::find(&self.doc, page)?.1.images()?, image)
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Runs one edit as `author` and commits it as one undo step.
    pub fn edit<T>(
        &mut self,
        author: Origin,
        label: &str,
        edit: impl FnOnce(&mut Edit<'_>) -> Result<T>,
    ) -> Result<(T, Change)> {
        self.usable()?;
        let mut writes = Edit::new(&self.doc, author);
        match edit(&mut writes) {
            Ok(value) => {
                self.doc.commit_with(CommitOptions::new().commit_msg(label));
                Ok((value, self.change()))
            }
            Err(error) => {
                self.poisoned |= writes.wrote;
                Err(error)
            }
        }
    }

    pub fn undo(&mut self) -> Result<Option<Change>> {
        self.usable()?;
        let undone = self.undo.undo().inspect_err(|_| self.poisoned = true)?;
        Ok(undone.then(|| self.change()))
    }

    pub fn redo(&mut self) -> Result<Option<Change>> {
        self.usable()?;
        let redone = self.undo.redo().inspect_err(|_| self.poisoned = true)?;
        Ok(redone.then(|| self.change()))
    }

    /// Saves with history. Containers that were never read are exported from
    /// their loaded bytes, so this does not decode images.
    pub fn save(&self) -> Result<()> {
        self.usable()?;
        Ok(self.file.save(&self.doc.export(ExportMode::Snapshot)?)?)
    }

    /// Saves without history, dropping every image an edit has replaced. Loro
    /// decodes every container to do this, so it is meant for rare moments
    /// such as closing a project.
    pub fn compact(&self) -> Result<()> {
        self.usable()?;
        let frontiers = self.doc.oplog_frontiers();
        Ok(self
            .file
            .save(&self.doc.export(ExportMode::shallow_snapshot(&frontiers))?)?)
    }

    fn usable(&self) -> Result<()> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        Ok(())
    }

    fn change(&self) -> Change {
        let touched =
            std::mem::take(&mut *self.touched.lock().unwrap_or_else(PoisonError::into_inner));
        let pages = document::page_list(&self.doc);
        Change {
            revision: self.revision(),
            pages: touched.pages,
            content: touched
                .content
                .into_iter()
                .filter_map(|index| PageMap::at(&pages, index).ok().map(|page| page.id))
                .collect(),
        }
    }
}
