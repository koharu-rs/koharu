//! The renderer: fonts, retained layer content, and the GPU, shared by every
//! frame it renders.

use std::{
    collections::HashSet,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use image::RgbaImage;
use koharu_scene::{LayerId, Offset, PageId, Session, Text};
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use tokio::sync::watch;
use vello::kurbo::Rect;

use crate::{
    Error, RendererConfig, Result,
    fonts::{FontRequest, Fonts},
    frame::{Frame, FrameData, Retained, Tree, View},
    gpu::Gpu,
    images,
    text::{self, ResolvedText},
    types::FontFamily,
};

mod preview;
mod retained;

/// Renders pages. Cloning it shares its fonts, caches, and GPU.
#[derive(Clone)]
pub struct Renderer(Arc<Inner>);

struct Inner {
    fonts: Fonts,
    config: RwLock<RendererConfig>,
    /// Bumped when the `[renderer]` preference or the loaded fonts change, so
    /// text lays out again.
    generation: AtomicU64,
    /// Tells `subscribe`'s listeners each new generation.
    changes: watch::Sender<u64>,
    /// The fonts a background download is fetching, so a render while it runs
    /// doesn't start another.
    fetching: Mutex<HashSet<FontRequest>>,
    cache: retained::Cache,
    gpu: OnceLock<std::result::Result<Arc<Gpu>, String>>,
}

/// A raster: RGBA pixels and where their corner sits in page pixels.
#[derive(Debug)]
pub struct Raster {
    pub image: RgbaImage,
    pub offset: Offset,
}

impl Renderer {
    /// Fonts load and the GPU starts on first use, so this is cheap. Chinese
    /// line breaking's dictionary starts loading in the background.
    #[must_use]
    pub fn new(config: RendererConfig) -> Self {
        text::preload_dictionary();
        Self(Arc::new(Inner {
            fonts: Fonts::new(),
            config: RwLock::new(config),
            generation: AtomicU64::new(0),
            changes: watch::channel(0).0,
            fetching: Mutex::default(),
            cache: retained::cache(),
            gpu: OnceLock::new(),
        }))
    }

    /// Follows a changed `[renderer]` preference; text lays out again.
    pub fn set_config(&self, config: RendererConfig) {
        *self.0.config.write() = config;
        self.bump();
    }

    /// Each time text lays out differently from now on: a changed
    /// `[renderer]`, or fonts a render found missing coming in. A page shown
    /// with fallbacks renders again then, as Photoshop swaps in a font Adobe
    /// Fonts activated.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.changes.subscribe()
    }

    fn bump(&self) {
        let generation = self.0.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.0.changes.send_replace(generation);
    }

    /// The GPU device and vello renderer, which the desktop canvas draws
    /// frames with.
    pub fn gpu(&self) -> Result<Arc<Gpu>> {
        self.0
            .gpu
            .get_or_init(|| {
                Gpu::new()
                    .map(Arc::new)
                    .map_err(|error| format!("{error:#}"))
            })
            .clone()
            .map_err(|error| Error::Backend(anyhow::anyhow!(error)))
    }

    /// Renders `page` as `session` holds it: its background, then its layers.
    /// Content that is unchanged reuses what an earlier render built. Text
    /// whose bundled font isn't in yet draws with a fallback while the font
    /// downloads; `subscribe` tells when it's in.
    pub fn render(&self, session: &Session, page: PageId) -> Result<Frame> {
        let project = session.project();
        let view = project.page(page)?;
        images::check_size(view.size.width, view.size.height)?;
        // Build what isn't retained across rayon first; building the tree
        // then finds it all.
        retained::sources(&project, page)?
            .par_iter()
            .for_each(|source| {
                let _ = self.content(session, *source);
            });
        let mut tree = Tree::new(|source| self.content(session, source));
        let mut nodes = Vec::new();
        if let Some(background) = view.background {
            nodes.push(tree.background(background)?);
        }
        for layer in &view.layers {
            nodes.push(tree.layer(layer)?);
        }
        let mut missing = tree.missing;
        missing.sort();
        missing.dedup();
        self.fetch(&missing);
        Ok(Frame(Arc::new(FrameData {
            page,
            revision: session.revision(),
            area: Rect::new(
                0.0,
                0.0,
                f64::from(view.size.width),
                f64::from(view.size.height),
            ),
            nodes,
            background: view.background.is_some(),
            missing: missing.into(),
        })))
    }

    /// Lays out `text`, which `layer` holds, as a render would: for a text
    /// nothing draws, such as a block's source in a PSD. `None` when it has no
    /// characters.
    pub fn lay_out(&self, layer: LayerId, text: &Text) -> Result<Option<Arc<ResolvedText>>> {
        match self.text_content(layer, text)? {
            Retained::Vector { text, .. } => Ok(text),
            Retained::Pixels(_) => Ok(None),
        }
    }

    /// Draws a frame's area at one output pixel per page pixel and reads it
    /// back.
    pub fn rasterize(&self, frame: &Frame) -> Result<Raster> {
        let area = frame.area();
        let (width, height) = (area.width().ceil() as u32, area.height().ceil() as u32);
        let offset = Offset {
            x: area.x0.floor() as i32,
            y: area.y0.floor() as i32,
        };
        // What draws nothing, such as a block with no clean-up, is empty.
        if width == 0 || height == 0 {
            return Ok(Raster {
                image: RgbaImage::new(width, height),
                offset,
            });
        }
        images::check_size(width, height)?;
        let gpu = self.gpu()?;
        let mut image = RgbaImage::new(width, height);
        for strip in frame.draw(View {
            rect: area,
            scale: 1.0,
        }) {
            let part = gpu
                .read(
                    &strip.scene,
                    strip.rect.width() as u32,
                    strip.rect.height() as u32,
                )
                .map_err(Error::Backend)?;
            image::imageops::replace(
                &mut image,
                &part,
                strip.rect.x0 as i64,
                strip.rect.y0 as i64,
            );
        }
        Ok(Raster { image, offset })
    }

    /// Renders `page` once the bundled fonts its text asks for are in, for
    /// what needs the page as it finally looks: an export, or the agent's
    /// view of it.
    pub async fn render_loaded(&self, session: Arc<Mutex<Session>>, page: PageId) -> Result<Frame> {
        let frame = self.render_blocking(Arc::clone(&session), page).await?;
        if frame.0.missing.is_empty() {
            return Ok(frame);
        }
        self.0
            .fonts
            .load(&frame.0.missing)
            .await
            .map_err(Error::Fonts)?;
        self.bump();
        self.render_blocking(session, page).await
    }

    async fn render_blocking(&self, session: Arc<Mutex<Session>>, page: PageId) -> Result<Frame> {
        let renderer = self.clone();
        tokio::task::spawn_blocking(move || renderer.render(&session.lock(), page))
            .await
            .map_err(|error| Error::Backend(error.into()))?
    }

    /// Downloads in the background the bundled fonts a render found missing,
    /// then tells `subscribe`'s listeners. A render outside a Tokio runtime
    /// leaves them to `render_loaded`.
    fn fetch(&self, missing: &[FontRequest]) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let requests: Vec<FontRequest> = {
            let mut fetching = self.0.fetching.lock();
            missing
                .iter()
                .filter(|request| fetching.insert((*request).clone()))
                .cloned()
                .collect()
        };
        if requests.is_empty() {
            return;
        }
        let renderer = self.clone();
        runtime.spawn(async move {
            let loaded = renderer.0.fonts.load(&requests).await;
            {
                let mut fetching = renderer.0.fetching.lock();
                for request in &requests {
                    fetching.remove(request);
                }
            }
            match loaded {
                Ok(()) => renderer.bump(),
                Err(error) => tracing::warn!("failed to download fonts: {error:#}"),
            }
        });
    }

    pub async fn available_fonts(&self) -> Result<Vec<FontFamily>> {
        self.0.fonts.families().await.map_err(Error::Fonts)
    }
}
