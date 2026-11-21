//! The session: the open project, its file, and its undo history. Every
//! committed edit is durable, so there's no save.

use std::path::{Path, PathBuf};

use tokio::sync::broadcast;

use crate::{
    Change, Edit, Error, Hash, History, LayerId, Project, ProjectId, Rect, Result, Revision, Tiles,
    change, edit, project::Picture, store::Store, tiles,
};

/// A project file's extension, without the dot.
pub const EXTENSION: &str = "khr";

/// How many steps history keeps before it drops the oldest, Photoshop's
/// History panel default.
const STEPS: usize = 20;

/// How many changes a slow listener can fall behind by.
const CHANGES: usize = 256;

/// One open project, holding the file's lock until it's dropped.
pub struct Session {
    store: Store,
    path: PathBuf,
    project: Project,
    revision: Revision,
    undo: Vec<Step>,
    redo: Vec<Step>,
    changes: broadcast::Sender<Change>,
}

/// A project to go back to: the one before an edit, on the undo stack, or the
/// one after it, on the redo stack.
struct Step {
    label: String,
    project: Project,
    revision: Revision,
}

impl Session {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let (store, project) = Store::create(path)?;
        Ok(Self::assemble(store, path, project))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let (store, project) = Store::open(path)?;
        Ok(Self::assemble(store, path, project))
    }

    /// Deletes the project file at `path`, which no session may hold, with
    /// the journals SQLite left beside it.
    pub fn remove(path: impl AsRef<Path>) -> Result<()> {
        Store::remove(path.as_ref())
    }

    fn assemble(store: Store, path: &Path, project: Project) -> Self {
        Self {
            store,
            path: path.to_owned(),
            project,
            revision: Revision::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            changes: broadcast::channel(CHANGES).0,
        }
    }

    /// Every change from now on, whichever writer made it: each edit that
    /// changed the project, undo, and redo, as Figma's `documentchange`
    /// tells a plugin.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.changes.subscribe()
    }

    #[must_use]
    pub fn id(&self) -> ProjectId {
        self.project.id
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's format version.
    pub(crate) fn version(&self) -> Result<String> {
        self.store.version()
    }

    /// The project as it is now, in O(1). It takes no lock and stays as it is
    /// while edits go on.
    #[must_use]
    pub fn project(&self) -> Project {
        self.project.clone()
    }

    /// Lasts as long as this session: equal revisions are the same document.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision.clone()
    }

    #[must_use]
    pub fn history(&self) -> History {
        History {
            steps: self
                .undo
                .iter()
                .chain(self.redo.iter().rev())
                .map(|step| step.label.clone())
                .collect(),
            current: self.undo.len(),
        }
    }

    /// Runs `edit` as one undo step. If it fails, nothing changes.
    pub fn apply(&mut self, edit: Edit) -> Result<Change> {
        let tx = self.store.begin()?;
        let (next, created) = edit::run(&self.project, &edit, &tx)?;
        tx.write_diff(&self.project, &next)?;
        // An edit that changes nothing still commits, so a file it added is
        // kept, but it adds no undo step.
        tx.commit()?;
        let (pages, layers) = change::changed(&self.project, &next);
        let changed = next != self.project;
        if changed {
            let before = std::mem::replace(&mut self.project, next);
            self.undo.push(Step {
                label: edit.label,
                project: before,
                revision: std::mem::replace(&mut self.revision, Revision::new()),
            });
            if self.undo.len() > STEPS {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        let change = Change {
            revision: self.revision(),
            created,
            pages,
            layers,
            history: self.history(),
        };
        if changed {
            // No one listening isn't a failure.
            let _ = self.changes.send(change.clone());
        }
        Ok(change)
    }

    /// The picture `layer` owns: a raster's or a mask's, or a block's mask's
    /// or clean-up's.
    fn picture(&self, layer: LayerId) -> Result<Picture> {
        self.project
            .picture(layer)
            .ok_or_else(|| match self.project.index.contains_key(&layer) {
                true => Error::Rejected(format!("layer {layer} has no pixels")),
                false => Error::NotFound(format!("layer {layer}")),
            })
    }

    /// A picture's tiles overlapping `rect`, or all of them. The lock is held
    /// only to read bytes, because decoding fans out on rayon, whose threads
    /// may be waiting for it.
    pub fn tiles(&self, layer: LayerId, rect: Option<Rect>) -> Result<Tiles> {
        let Picture { format, offset, .. } = self.picture(layer)?;
        let range = match rect {
            Some(rect) => match tiles::covered(rect) {
                Some(range) => Some(range),
                None => {
                    return Ok(Tiles {
                        offset,
                        format,
                        tiles: Vec::new(),
                    });
                }
            },
            None => None,
        };
        let wanted: Vec<(&(i32, i32), &Hash)> = self
            .project
            .tiles(layer)
            .into_iter()
            .flatten()
            .filter(|((x, y), _)| {
                range.is_none_or(|((x0, x1), (y0, y1))| {
                    (x0..=x1).contains(&i64::from(*x)) && (y0..=y1).contains(&i64::from(*y))
                })
            })
            .collect();
        let bytes = self.store.blobs(wanted.iter().map(|(_, hash)| *hash))?;
        let stored: Vec<_> = wanted
            .iter()
            .zip(bytes)
            .map(|(((x, y), _), bytes)| (*x, *y, bytes))
            .collect();
        Ok(Tiles {
            offset,
            format,
            tiles: tiles::decode(&stored, format)?,
        })
    }

    /// A picture's pixels over `rect`, in its own coordinates and format, row
    /// by row, and zero where it has no tiles: what `write_pixels` wrote.
    pub fn pixels(&self, layer: LayerId, rect: Rect) -> Result<Vec<u8>> {
        let Tiles {
            format,
            tiles: stored,
            ..
        } = self.tiles(layer, Some(rect))?;
        let bytes = tiles::bytes_per_pixel(format);
        let width = rect.width as usize;
        let mut pixels = vec![0; width * rect.height as usize * bytes];
        let (left, top) = (i64::from(rect.x), i64::from(rect.y));
        let (right, bottom) = (left + i64::from(rect.width), top + i64::from(rect.height));
        for tile in stored {
            let (x, y) = (
                i64::from(tile.x) * tiles::SIZE,
                i64::from(tile.y) * tiles::SIZE,
            );
            let (x0, x1) = (x.max(left), (x + tiles::SIZE).min(right));
            let length = (x1 - x0) as usize * bytes;
            for row in y.max(top)..(y + tiles::SIZE).min(bottom) {
                let from = ((row - y) * tiles::SIZE + x0 - x) as usize * bytes;
                let to = ((row - top) as usize * width + (x0 - left) as usize) * bytes;
                pixels[to..to + length].copy_from_slice(&tile.pixels[from..from + length]);
            }
        }
        Ok(pixels)
    }

    /// A hash of a picture's tiles, which changes whenever its pixels do,
    /// taken without reading them.
    pub fn tiles_hash(&self, layer: LayerId) -> Result<Hash> {
        self.picture(layer)?;
        let mut hasher = blake3::Hasher::new();
        for ((x, y), hash) in self.project.tiles(layer).into_iter().flatten() {
            hasher.update(&x.to_le_bytes());
            hasher.update(&y.to_le_bytes());
            hasher.update(hash.as_bytes());
        }
        Ok(hasher.finalize().into())
    }

    /// An imported file's bytes.
    pub fn file(&self, hash: &Hash) -> Result<Vec<u8>> {
        let mut bytes = self.store.blobs([hash])?;
        Ok(bytes.pop().expect("one hash reads one blob"))
    }

    pub fn undo(&mut self) -> Result<Option<Change>> {
        match self.undo.len() {
            0 => Ok(None),
            current => self.go_to(current - 1),
        }
    }

    pub fn redo(&mut self) -> Result<Option<Change>> {
        match self.redo.is_empty() {
            true => Ok(None),
            false => self.go_to(self.undo.len() + 1),
        }
    }

    /// Goes to the project as it was after `step` of the history's steps, 0
    /// being as it opened, as selecting a state in Photoshop's History panel
    /// does. Every step between is undone or redone in one write, which can't
    /// conflict: the file holds the current project, whatever it was.
    pub fn go_to(&mut self, step: usize) -> Result<Option<Change>> {
        let current = self.undo.len();
        let total = current + self.redo.len();
        if step > total {
            return Err(Error::Rejected(format!(
                "step {step} is past the last of {total}"
            )));
        }
        if step == current {
            return Ok(None);
        }
        let target = match step < current {
            true => &self.undo[step].project,
            false => &self.redo[total - step].project,
        };
        let tx = self.store.begin()?;
        tx.write_diff(&self.project, target)?;
        tx.commit()?;
        let before = self.project.clone();
        while self.undo.len() > step {
            shift(
                &mut self.project,
                &mut self.revision,
                &mut self.undo,
                &mut self.redo,
            );
        }
        while self.undo.len() < step {
            shift(
                &mut self.project,
                &mut self.revision,
                &mut self.redo,
                &mut self.undo,
            );
        }
        let (pages, layers) = change::changed(&before, &self.project);
        let change = Change {
            revision: self.revision(),
            created: Vec::new(),
            pages,
            layers,
            history: self.history(),
        };
        let _ = self.changes.send(change.clone());
        Ok(Some(change))
    }
}

/// Moves the top step of `from` to `to`, making its project current and
/// keeping the one it replaces in its place.
fn shift(project: &mut Project, revision: &mut Revision, from: &mut Vec<Step>, to: &mut Vec<Step>) {
    let step = from.pop().expect("a step to move");
    to.push(Step {
        label: step.label,
        project: std::mem::replace(project, step.project),
        revision: std::mem::replace(revision, step.revision),
    });
}

// This limits the file behind the session, which only the crate can do.
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Created, Op};

    #[test]
    fn an_edit_that_runs_out_of_space_changes_nothing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
        let ops = [
            json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 512, "height": 512}}),
            json!({"op": "add_layer", "page": "$p", "index": 0, "layer": {"kind": "raster"}}),
        ];
        let change = session
            .apply(Edit {
                label: "test".to_owned(),
                ops: ops
                    .into_iter()
                    .map(|op| serde_json::from_value(op).expect("an op"))
                    .collect(),
            })
            .expect("an edit");
        let Created::Layer { id: raster, .. } = change.created[1] else {
            panic!("{change:?}")
        };
        let limit = |session: &Session, pages: i64| {
            session
                .store
                .conn()
                .query_row(&format!("PRAGMA max_page_count = {pages}"), [], |_| Ok(()))
                .expect("a limit");
        };
        let pages: i64 = session
            .store
            .conn()
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .expect("a page count");
        limit(&session, pages);

        // Noise doesn't compress, so these tiles can't fit.
        let mut state = 1u32;
        let pixels: Vec<u8> = (0..512 * 512 * 4)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        let paint = Edit {
            label: "paint".to_owned(),
            ops: vec![Op::WritePixels {
                layer: raster.into(),
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 512,
                    height: 512,
                },
                pixels,
            }],
        };
        let (project, history, revision) =
            (session.project(), session.history(), session.revision());
        let error = session.apply(paint.clone()).expect_err("the disk is full");
        assert!(
            matches!(&error, Error::Sql(error) if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull)),
            "{error:?}"
        );
        assert_eq!(session.project(), project);
        assert_eq!((session.history(), session.revision()), (history, revision));

        // With room again, the next edit works, and so does undo.
        limit(&session, 1 << 30);
        session.apply(paint).expect("a paint");
        let painted = session.tiles(raster, None).expect("tiles");
        assert_eq!(painted.tiles.len(), 64);
        session.undo().expect("undo").expect("a step");
        assert!(session.tiles(raster, None).expect("tiles").tiles.is_empty());
    }
}
