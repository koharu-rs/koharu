//! A chapter an extension reads, as a project: its name, how it reads, and
//! loading its pages into it in order, which the user then translates when
//! they choose.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use koharu_extensions::{Chapter, Extension, PageSide, ReadingDirection};
use koharu_scene::{Created, Edit, Hash, Op, PageId, Session};
use parking_lot::Mutex;
use sanitize_filename::Options;
use serde::Serialize;
use specta::Type;
use tokio_util::task::AbortOnDropHandle;
use url::Url;

use crate::{app::App, events};

/// The most bytes of a series and title a project's name keeps, so its path
/// stays under Windows' 260 characters, and its file's name under the 255
/// bytes of Linux and macOS.
const TITLE: usize = 150;

/// A chapter read into a project: the project, and how its pages read, as
/// its extension said, which the interface keeps for the reader.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub project: String,
    pub direction: Option<ReadingDirection>,
    /// Which side of a two-page spread its first page takes.
    pub first_page: Option<PageSide>,
}

/// How far a chapter's loading went, which the `chapter` event carries.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Loading {
    /// Its pages in the project so far, and in the chapter.
    pub loaded: u32,
    pub total: u32,
    /// Why it stopped, when it failed.
    pub error: Option<String>,
}

/// Opens the chapter at `url` as the open project, named after it, and loads
/// its pages into it through `extension`, which reads it, and `http`; says
/// how the chapter reads.
pub async fn read(
    app: &Arc<App>,
    extension: &'static dyn Extension,
    http: reqwest::Client,
    url: &Url,
) -> Result<Reading> {
    let chapter = extension.chapter(&http, url).await?;
    let name = name(&chapter, url);
    tracing::info!(pages = chapter.pages.len(), "read chapter");
    let reading = Reading {
        project: name.clone(),
        direction: chapter.direction,
        first_page: chapter.first_page,
    };
    let opening = Arc::clone(app);
    let opened = name.clone();
    tokio::task::spawn_blocking(move || {
        let session = opening.library.open_or_create(&opened)?;
        opening.open(opened, session)
    })
    .await??;
    let total = u32::try_from(chapter.pages.len())?;
    let telling = Arc::clone(app);
    let loading = AbortOnDropHandle::new(tokio::spawn(load(
        extension,
        http,
        chapter,
        app.session()?,
        move |loaded, error| {
            let loading = Loading {
                loaded,
                total,
                error,
            };
            telling.emit(events::CHAPTER, loading);
        },
    )));
    if let Some(project) = app
        .project
        .lock()
        .as_mut()
        .filter(|project| project.name == name)
    {
        project.loading = Some(loading);
    }
    Ok(reading)
}

/// The project "‹series› ‹title› – ‹site›", made a file name on every
/// platform. The series is cut before the title, which tells its chapters
/// apart.
fn name(chapter: &Chapter, url: &Url) -> String {
    let site = url.host_str().unwrap_or_default();
    let title = &chapter.title[..chapter.title.floor_char_boundary(TITLE)];
    let title = match &chapter.series {
        Some(series) => {
            let end = series.floor_char_boundary(TITLE - title.len());
            format!("{} {title}", &series[..end])
        }
        None => title.to_owned(),
    };
    let title = title.trim();
    let name = if title.is_empty() {
        site.to_owned()
    } else {
        format!("{title} – {site}")
    };
    let options = Options {
        windows: true,
        truncate: true,
        replacement: "",
    };
    sanitize_filename::sanitize_with_options(name, options)
}

/// Loads `chapter`'s pages into `session` in order, through `extension` and
/// `http`, telling `tell` how many are in after each, and why it stopped
/// when one fails.
async fn load(
    extension: &'static dyn Extension,
    http: reqwest::Client,
    chapter: Chapter,
    session: Arc<Mutex<Session>>,
    tell: impl Fn(u32, Option<String>) + Send,
) {
    let mut loaded = 0;
    let mut previous = None;
    tell(loaded, None);
    for page in &chapter.pages {
        match self::page(extension, &http, page, &session, previous).await {
            Ok(page) => {
                previous = Some(page);
                loaded += 1;
                tell(loaded, None);
            }
            Err(error) => {
                let error = format!("failed to load page {}: {error:#}", loaded + 1);
                tracing::error!("{error}");
                tell(loaded, Some(error));
                return;
            }
        }
    }
}

/// Loads `page`, the chapter's page after `previous`.
async fn page(
    extension: &dyn Extension,
    http: &reqwest::Client,
    page: &str,
    session: &Arc<Mutex<Session>>,
    previous: Option<PageId>,
) -> Result<PageId> {
    let bytes = extension.page(http, page).await?;
    let placing = Arc::clone(session);
    tokio::task::spawn_blocking(move || place(&placing, bytes, previous)).await?
}

/// The page the image `bytes` is: the page already made from it, or else a
/// new one, added as import adds one, right after `previous`, at the start
/// when there's none, and at the end when it's gone.
fn place(session: &Mutex<Session>, bytes: Vec<u8>, previous: Option<PageId>) -> Result<PageId> {
    let hash = Hash::of(&bytes);
    let mut session = session.lock();
    let project = session.project();
    if let Some(page) = project.pages().find(|page| {
        page.background
            .is_some_and(|background| background.file == hash)
    }) {
        return Ok(page.id);
    }
    let index = match previous {
        None => 0,
        Some(previous) => project
            .pages()
            .position(|page| page.id == previous)
            .map_or_else(|| project.pages().count(), |index| index + 1),
    };
    let change = session.apply(Edit {
        label: "import".to_owned(),
        ops: vec![
            Op::AddFile { bytes },
            Op::AddPage {
                id: None,
                index,
                size: None,
                background: Some(hash),
            },
        ],
    })?;
    let page = change
        .created
        .iter()
        .find_map(|created| match created {
            Created::Page { id, .. } => Some(*id),
            Created::Layer { .. } => None,
        })
        .context("adding a page made none")?;
    Ok(page)
}
