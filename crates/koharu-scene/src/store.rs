//! The project file: one SQLite database holding a project, page images
//! included. Rows are plain: a page is its id and its JSON, its layers inside
//! it, and a tile is its place and the hash of its bytes. This is the only
//! module that speaks SQL.

use std::{
    collections::HashSet,
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use imbl::{OrdMap, Vector, ordmap::DiffItem};
use indoc::indoc;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, params};
use semver::Version;
use serde::Deserialize;
use serde_rusqlite::from_rows;
use serde_with::{json::JsonString, serde_as};

use crate::{
    Content, Error, Hash, LayerId, Page, PageId, Project, ProjectId, Result, Uuid,
    project::{TileMap, depth_first, ids},
    tiles,
};

/// "KHRU" in SQLite's header, which SQLite reserves for a file format built
/// on it, so opening refuses any other database before reading its rows.
const APPLICATION_ID: i32 = 0x4B48_5255;
/// The format version this build writes.
const VERSION: Version = Version::new(1, 0, 0);
/// Most compressed tiles fit on a page of this size, which keeps them out of
/// overflow chains, each of whose pages is a read of its own.
const PAGE_SIZE: u32 = 16384;

/// `pages` and `blobs` keep their rowids: without one, a row holds only a
/// quarter of a page before spilling, which read tiles five times slower when
/// tried, and a page's row holds its whole layer tree.
const SCHEMA: &str = indoc! {"
    CREATE TABLE project(id TEXT PRIMARY KEY, version TEXT NOT NULL, pages TEXT NOT NULL) STRICT;
    CREATE TABLE pages(id TEXT PRIMARY KEY, page TEXT NOT NULL) STRICT;
    CREATE TABLE tiles(
      layer TEXT NOT NULL,
      x INTEGER NOT NULL,
      y INTEGER NOT NULL,
      hash TEXT NOT NULL,
      PRIMARY KEY(layer, x, y)
    ) STRICT, WITHOUT ROWID;
    CREATE TABLE blobs(hash TEXT PRIMARY KEY, bytes BLOB NOT NULL) STRICT;
"};

#[serde_as]
#[derive(Deserialize)]
struct ProjectRow {
    id: ProjectId,
    version: String,
    #[serde_as(as = "JsonString")]
    pages: Vector<PageId>,
}

#[serde_as]
#[derive(Deserialize)]
struct PageRow {
    id: PageId,
    #[serde_as(as = "JsonString")]
    page: Page,
}

#[derive(Deserialize)]
struct TileRow {
    layer: LayerId,
    x: i32,
    y: i32,
    hash: Hash,
}

impl From<serde_rusqlite::Error> for Error {
    fn from(error: serde_rusqlite::Error) -> Self {
        Self::corrupt(error)
    }
}

/// The open file. The connection sits in a mutex because the renderer reads
/// tiles from rayon threads.
pub(crate) struct Store {
    conn: Mutex<Connection>,
}

/// The journals SQLite may leave beside a project file. Exclusive locking
/// keeps WAL's index in memory, so there's no `-shm`.
const JOURNALS: [&str; 2] = ["-wal", "-journal"];

/// One writer holds a project for as long as it's open, and a second finds it
/// locked at once rather than after a wait.
fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::ZERO)?;
    conn.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA synchronous=NORMAL;")
}

fn flags() -> OpenFlags {
    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
}

fn json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("project values serialize")
}

impl Store {
    /// Creates a project at `path`, and fails if `path` exists. A create that
    /// fails removes the file, which would refuse both another create and an
    /// open, and its journals, which SQLite would replay into the next
    /// project made there.
    pub(crate) fn create(path: &Path) -> Result<(Self, Project)> {
        fs_err::File::create_new(path)?;
        let created = Self::initialize(path);
        if created.is_err() {
            // What can't be removed doesn't change what failed.
            let _ = Self::remove(path);
        }
        created
    }

    /// Deletes a project file and the journals beside it, which SQLite would
    /// replay into the next project made there.
    pub(crate) fn remove(path: &Path) -> Result<()> {
        fs_err::remove_file(path)?;
        for suffix in JOURNALS {
            let mut journal = path.as_os_str().to_owned();
            journal.push(suffix);
            match fs_err::remove_file(journal) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn initialize(path: &Path) -> Result<(Self, Project)> {
        let conn = Connection::open_with_flags(path, flags())?;
        configure(&conn)?;
        conn.execute_batch(&format!(
            "PRAGMA page_size={PAGE_SIZE};
             PRAGMA auto_vacuum=INCREMENTAL;
             PRAGMA journal_mode=WAL;
             PRAGMA application_id={APPLICATION_ID};
             {SCHEMA}"
        ))?;
        let project = Project::new(ProjectId(Uuid::now_v7()));
        conn.execute(
            "INSERT INTO project VALUES (?1, ?2, ?3)",
            params![
                project.id.to_string(),
                VERSION.to_string(),
                json(&project.order)
            ],
        )?;
        Ok((
            Self {
                conn: Mutex::new(conn),
            },
            project,
        ))
    }

    /// Opens a project and takes its lock. Nothing is written until the file
    /// has shown it is a valid project this build reads.
    pub(crate) fn open(path: &Path) -> Result<(Self, Project)> {
        let conn = Connection::open_with_flags(path, flags())?;
        let opening = |error: rusqlite::Error| match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
                Error::ProjectLocked(path.to_owned())
            }
            Some(ErrorCode::NotADatabase) => Error::corrupt("this is not a Koharu project"),
            _ => error.into(),
        };
        configure(&conn).map_err(opening)?;
        let application_id: i32 = conn
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .map_err(opening)?;
        if application_id != APPLICATION_ID {
            return Err(Error::corrupt("this is not a Koharu project"));
        }
        let project = load(&conn)?;
        sweep(&conn, &project)?;
        Ok((
            Self {
                conn: Mutex::new(conn),
            },
            project,
        ))
    }

    /// The format version the file holds: the one it was opened at, until
    /// this build first writes to it.
    pub(crate) fn version(&self) -> Result<String> {
        Ok(self
            .conn()
            .query_row("SELECT version FROM project", [], |row| row.get(0))?)
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Blobs' bytes, in order, under one hold of the lock.
    pub(crate) fn blobs<'h>(
        &self,
        hashes: impl IntoIterator<Item = &'h Hash>,
    ) -> Result<Vec<Vec<u8>>> {
        let conn = self.conn();
        hashes.into_iter().map(|hash| blob(&conn, hash)).collect()
    }

    pub(crate) fn begin(&mut self) -> Result<Tx<'_>> {
        let conn = self.conn.get_mut().unwrap_or_else(PoisonError::into_inner);
        Ok(Tx(conn.transaction()?))
    }
}

/// Every blob the project names: its tiles, its images' files, and its pages'
/// backgrounds.
fn named(project: &Project) -> HashSet<Hash> {
    let tiles = project
        .tiles
        .values()
        .flat_map(|tiles| tiles.values().copied());
    let files = project
        .pages
        .values()
        .flat_map(|page| depth_first(&page.layers))
        .filter_map(|layer| match &layer.content {
            Content::Image(image) => Some(image.file),
            _ => None,
        });
    let backgrounds = project
        .pages
        .values()
        .filter_map(|page| page.background.map(|background| background.file));
    tiles.chain(files).chain(backgrounds).collect()
}

/// Fails if a blob the project names is missing, then removes the ones it
/// doesn't name. Undo steps name blobs too, but none survive closing, so this
/// only runs on open, when no step can need them.
fn sweep(conn: &Connection, project: &Project) -> Result<()> {
    let named = named(project);
    let mut stored = HashSet::new();
    let mut statement = conn.prepare("SELECT hash FROM blobs")?;
    for hash in from_rows::<(Hash,)>(statement.query([])?) {
        stored.insert(hash?.0);
    }
    if let Some(missing) = named.iter().find(|hash| !stored.contains(hash)) {
        return Err(Error::corrupt(format!("blob {missing} is missing")));
    }
    let unnamed: Vec<&Hash> = stored.iter().filter(|hash| !named.contains(hash)).collect();
    if unnamed.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for hash in unnamed {
        tx.prepare_cached("DELETE FROM blobs WHERE hash = ?1")?
            .execute([hash.to_string()])?;
    }
    tx.commit()?;
    // Each row returned is a freed page, and it stops if nothing steps it.
    let mut vacuum = conn.prepare("PRAGMA incremental_vacuum")?;
    let mut steps = vacuum.query([])?;
    while steps.next()?.is_some() {}
    Ok(())
}

fn blob(conn: &Connection, hash: &Hash) -> Result<Vec<u8>> {
    conn.prepare_cached("SELECT bytes FROM blobs WHERE hash = ?1")?
        .query_row([hash.to_string()], |row| row.get(0))
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("file {hash}")))
}

fn load(conn: &Connection) -> Result<Project> {
    let mut statement = conn.prepare("SELECT id, version, pages FROM project")?;
    let rows: Vec<ProjectRow> =
        from_rows(statement.query([])?).collect::<std::result::Result<_, _>>()?;
    let [row] = <[ProjectRow; 1]>::try_from(rows)
        .map_err(|_| Error::corrupt("a project has one project row"))?;
    // A reader opens any version up to its own with the same major version,
    // since an addition needs no decoder. The rows of any other version mean
    // something this build can't know, so they're never read.
    let readable = Version::parse(&row.version).is_ok_and(|stored| {
        stored.major == VERSION.major && stored <= VERSION && stored.pre.is_empty()
    });
    if !readable {
        return Err(Error::Version(row.version));
    }
    let mut project = Project::new(row.id);
    project.order = row.pages;

    let mut statement = conn.prepare("SELECT id, page FROM pages")?;
    for row in from_rows::<PageRow>(statement.query([])?) {
        let row = row?;
        if row.page.id != row.id {
            return Err(Error::corrupt(format!(
                "page {} is stored under {}",
                row.page.id, row.id
            )));
        }
        for id in ids(&row.page.layers) {
            project.index.insert(id, row.id);
        }
        project.pages.insert(row.id, row.page);
    }

    // A tile whose pixels lie past what an `i32` holds would overflow every
    // read of it.
    let fits = |at: i32| i32::try_from(i64::from(at) * tiles::SIZE).is_ok();
    let mut statement = conn.prepare("SELECT layer, x, y, hash FROM tiles")?;
    for row in from_rows::<TileRow>(statement.query([])?) {
        let row = row?;
        if !fits(row.x) || !fits(row.y) {
            return Err(Error::corrupt(format!(
                "a tile of layer {} sits out of range",
                row.layer
            )));
        }
        project
            .tiles
            .entry(row.layer)
            .or_default()
            .insert((row.x, row.y), row.hash);
    }
    Ok(project)
}

/// A write in progress. Dropping it without `commit` rolls it back, blobs
/// included.
pub(crate) struct Tx<'c>(Transaction<'c>);

impl Tx<'_> {
    pub(crate) fn commit(self) -> Result<()> {
        Ok(self.0.commit()?)
    }

    /// Stores `bytes` under their hash, once.
    pub(crate) fn put_blob(&self, bytes: &[u8]) -> Result<Hash> {
        let hash = Hash::of(bytes);
        self.0
            .prepare_cached("INSERT OR IGNORE INTO blobs VALUES (?1, ?2)")?
            .execute(params![hash.to_string(), bytes])?;
        Ok(hash)
    }

    pub(crate) fn blob(&self, hash: &Hash) -> Result<Vec<u8>> {
        blob(&self.0, hash)
    }

    pub(crate) fn has_blob(&self, hash: &Hash) -> Result<bool> {
        Ok(self
            .0
            .prepare_cached("SELECT 1 FROM blobs WHERE hash = ?1")?
            .exists([hash.to_string()])?)
    }

    /// Writes what differs between `before`, as the file holds it, and
    /// `after`. `OrdMap::diff` skips the parts the two share.
    pub(crate) fn write_diff(&self, before: &Project, after: &Project) -> Result<()> {
        let conn = &self.0;
        // The rows written are this build's version, so the file becomes it,
        // and an older build refuses it as too new rather than as corrupt.
        conn.prepare_cached("UPDATE project SET version = ?1 WHERE version <> ?1")?
            .execute([VERSION.to_string()])?;
        if before.order != after.order {
            conn.prepare_cached("UPDATE project SET pages = ?1")?
                .execute([json(&after.order)])?;
        }
        for item in before.pages.diff(&after.pages) {
            match item {
                DiffItem::Add(id, page)
                | DiffItem::Update {
                    new: (id, page), ..
                } => {
                    conn.prepare_cached("INSERT OR REPLACE INTO pages VALUES (?1, ?2)")?
                        .execute([id.to_string(), json(page)])?;
                }
                DiffItem::Remove(id, _) => {
                    conn.prepare_cached("DELETE FROM pages WHERE id = ?1")?
                        .execute([id.to_string()])?;
                }
            }
        }
        let empty = OrdMap::new();
        for item in before.tiles.diff(&after.tiles) {
            let (layer, old, new): (&LayerId, &TileMap, &TileMap) = match item {
                DiffItem::Add(layer, new) => (layer, &empty, new),
                DiffItem::Update { old, new } => (new.0, old.1, new.1),
                DiffItem::Remove(layer, old) => (layer, old, &empty),
            };
            self.write_tiles(*layer, old, new)?;
        }
        Ok(())
    }

    fn write_tiles(&self, layer: LayerId, old: &TileMap, new: &TileMap) -> Result<()> {
        let layer = layer.to_string();
        for item in old.diff(new) {
            match item {
                DiffItem::Add((x, y), hash)
                | DiffItem::Update {
                    new: ((x, y), hash),
                    ..
                } => {
                    self.0
                        .prepare_cached("INSERT OR REPLACE INTO tiles VALUES (?1, ?2, ?3, ?4)")?
                        .execute(params![layer, x, y, hash.to_string()])?;
                }
                DiffItem::Remove((x, y), _) => {
                    self.0
                        .prepare_cached("DELETE FROM tiles WHERE layer = ?1 AND x = ?2 AND y = ?3")?
                        .execute(params![layer, x, y])?;
                }
            }
        }
        Ok(())
    }
}
