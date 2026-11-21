//! The renderer: a page's frame from the scene, each layer's node drawing
//! content retained for the next render.

use std::sync::{Arc, LazyLock, Once};

use anyhow::{Result, bail};
use koharu_scene::{Block, Content as SceneContent, Layer, LayerId, PageId, Session, Text};
use kurbo::{Affine, Rect};
use parking_lot::RwLock;
use rayon::prelude::*;

use crate::{
    RendererConfig, fonts,
    frame::{Content, Frame, FrameData, MaskNode, Node},
    pixels::check_size,
    text::{self, Lettering, ResolvedText},
};

mod retained;

/// The process's renderer, with the default preference until the app sets
/// its own.
static RENDERER: LazyLock<Renderer> = LazyLock::new(|| Renderer::new(RendererConfig::default()));

/// Renders pages. Cloning it shares its retained content.
#[derive(Clone)]
pub struct Renderer(Arc<Inner>);

struct Inner {
    config: RwLock<RendererConfig>,
    cache: retained::Cache,
}

impl Renderer {
    /// The process's renderer, as `Gpu::get` is its GPU: what the app's
    /// canvas, exports, and pipeline draw with, so they share one retained
    /// cache. A test or a tool with a preference of its own makes its own.
    pub fn get() -> &'static Self {
        &RENDERER
    }

    /// What the first text would otherwise wait for loads in the background:
    /// Chinese line breaking's dictionary, the installed fonts, and the
    /// bundled font catalog. They load once, on a thread of their own rather
    /// than rayon's global pool, which the app may not have set up yet; should
    /// the thread fail to start, the first text that needs each loads it.
    #[must_use]
    pub fn new(config: RendererConfig) -> Self {
        static PRELOAD: Once = Once::new();
        PRELOAD.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("renderer preload".to_owned())
                .spawn(|| {
                    text::preload_dictionary();
                    fonts::preload();
                });
        });
        Self(Arc::new(Inner {
            config: RwLock::new(config),
            cache: retained::cache(),
        }))
    }

    /// Follows a changed `[renderer]` preference. A text's layout is keyed on
    /// the families it falls back on, so text lays out again.
    pub fn set_config(&self, config: RendererConfig) {
        *self.0.config.write() = config;
    }

    /// Renders `page` as `session` holds it: its background, then its layers,
    /// built across rayon. Content that is unchanged reuses what an earlier
    /// render built. A text whose bundled font isn't in yet waits while it
    /// downloads.
    pub fn render(&self, session: &Session, page: PageId) -> Result<Frame> {
        let project = session.project();
        let view = project.page(page)?;
        check_size(view.size.width, view.size.height)?;
        let (background, layers) = rayon::join(
            || {
                view.background
                    .map(|background| -> Result<Node> {
                        let content = self.image(session, background.file)?;
                        Ok(Node::new(
                            None,
                            background.visible,
                            content.map(|content| (content, Affine::IDENTITY)),
                        ))
                    })
                    .transpose()
            },
            || {
                view.layers
                    .par_iter()
                    .map(|layer| self.layer(session, layer))
                    .collect::<Result<Vec<Node>>>()
            },
        );
        Ok(Frame(Arc::new(FrameData {
            page,
            revision: session.revision(),
            area: Rect::new(
                0.0,
                0.0,
                f64::from(view.size.width),
                f64::from(view.size.height),
            ),
            nodes: background?.into_iter().chain(layers?).collect(),
        })))
    }

    /// Lays out `text`, which `layer` holds, as a render would: for a text
    /// nothing draws, such as a block's source in a PSD. `None` when it has no
    /// characters.
    pub fn layout(&self, layer: LayerId, text: &Text) -> Result<Option<ResolvedText>> {
        Ok(match self.text(layer, text)? {
            Some(Content::Text(layout)) => Some(layout.resolved.clone()),
            _ => None,
        })
    }

    /// `text`, which `layer` holds, laid out as a render lays it out, for
    /// typing into it; a content and style laid out once, as a render keeps
    /// them.
    pub fn lettering(&self, layer: LayerId, text: &Text) -> Result<Lettering> {
        Lettering::of(text, |text| {
            Ok(match self.text(layer, text)? {
                Some(Content::Text(layout)) => Some(layout),
                _ => None,
            })
        })
    }

    /// `layer`'s node, its children built across rayon.
    fn layer(&self, session: &Session, layer: &Layer) -> Result<Node> {
        let mut children = layer
            .children
            .par_iter()
            .filter(|child| !child.is_mask())
            .map(|child| self.layer(session, child))
            .collect::<Result<Vec<_>>>()?;
        // An empty mask covers nothing.
        let masks = layer
            .children
            .iter()
            .filter_map(|child| match &child.content {
                SceneContent::Mask(mask) => Some((child, mask)),
                _ => None,
            })
            .map(|(child, mask)| {
                let cover = self.tiles(session, child.id)?;
                Ok(MaskNode::new(
                    child.visible,
                    mask.mode,
                    cover.map(|cover| (cover, Affine::translate(mask.offset))),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let content = match &layer.content {
            SceneContent::Group(_) | SceneContent::Panel(_) | SceneContent::Balloon(_) => None,
            SceneContent::Block(block) => {
                children = self.block(session, layer.id, block)?;
                None
            }
            SceneContent::Text(text) => self
                .text(layer.id, text)?
                .map(|content| (content, Affine::from(text.transform))),
            SceneContent::Shape(shape) => self
                .shape(shape)?
                .map(|content| (content, Affine::IDENTITY)),
            SceneContent::Image(image) => self
                .image(session, image.file)?
                .map(|content| (content, Affine::from(image.transform))),
            SceneContent::Raster(raster) => self
                .tiles(session, layer.id)?
                .map(|content| (content, Affine::translate(raster.offset))),
            SceneContent::Mask(_) => bail!("mask {} is drawn only as its parent's", layer.id),
        };
        let container = content.is_none();
        let mut node = Node {
            opacity: f32::from(layer.opacity),
            blend: layer.blend,
            children,
            masks,
            ..Node::new(Some(layer.id), layer.visible, content)
        };
        if container {
            node.bounds = node
                .children
                .iter()
                .filter_map(|child| child.bounds)
                .reduce(|a, b| a.union(b));
        }
        Ok(node)
    }

    /// Block `id` draws its clean-up through its mask, then its translation;
    /// its source is never drawn.
    fn block(&self, session: &Session, id: LayerId, block: &Block) -> Result<Vec<Node>> {
        let (cleanup, mask) = (&block.cleanup, &block.mask);
        let cover = self
            .tiles(session, mask.id)?
            .map(|cover| (cover, Affine::translate(mask.content.offset)));
        let cleaned = self
            .tiles(session, cleanup.id)?
            .map(|cleaned| (cleaned, Affine::translate(cleanup.content.offset)));
        let mut nodes = vec![Node {
            opacity: f32::from(cleanup.opacity),
            blend: cleanup.blend,
            masks: vec![MaskNode::new(mask.visible, mask.content.mode, cover)],
            ..Node::new(Some(cleanup.id), cleanup.visible, cleaned)
        }];
        if let Some(text) = &block.translation {
            let content = self
                .text(id, text)?
                .map(|content| (content, Affine::from(text.transform)));
            nodes.push(Node::new(None, true, content));
        }
        Ok(nodes)
    }
}
