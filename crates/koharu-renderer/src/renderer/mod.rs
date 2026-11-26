//! The renderer: a page's frame from the scene, each layer's node drawing
//! content retained for the next render.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock, Once};

use anyhow::{Result, bail};
use icu_locale::LocaleExpander;
use icu_locale_core::subtags::script;
use icu_properties::{CodePointMapData, props::Script as Unicode};
use koharu_scene::{
    Block, CharStyle, Content as SceneContent, Kernel, LanguageIdentifier, Layer, LayerId, PageId,
    Project, Script, Session, Text,
};
use kurbo::{Affine, Rect};
use rayon::prelude::*;

use crate::{
    fonts,
    frame::{Content, Frame, FrameData, MaskNode, Node},
    pixels::check_size,
    text::{self, Lettering, ResolvedText},
};

mod retained;

/// The process's renderer.
static RENDERER: LazyLock<Renderer> = LazyLock::new(Renderer::new);

/// The ISO 15924 script of `text`'s language, as `LocaleExpander` maximises
/// it; for an undetermined one, the language its kana or Hangul imply, or
/// Chinese for Han alone, as the font fallback reads it, else Latin.
fn text_script(text: &Text) -> Option<Script> {
    let mut language = text.language.clone();
    if language == LanguageIdentifier::UNKNOWN {
        let content = text.content.text();
        let scripts = CodePointMapData::<Unicode>::new();
        let implied = crate::script::implied_language(&content).or_else(|| {
            let han = content.chars().any(|c| scripts.get(c) == Unicode::Han);
            han.then_some("zh")
        });
        if let Some(implied) = implied {
            language = implied.parse().ok()?;
        }
    }
    LocaleExpander::new_extended().maximize(&mut language);
    language.script
}

/// `text`'s face filled in where its `style.chars` sets no family, weight,
/// or italic: its kernel's style in `project` for its script, with what that
/// leaves out from antique's, and antique's for a text with no kernel. And the
/// families its glyphs fall back on: the project's antique family for its
/// script, then Latin's.
fn lettered<'t>(project: &Project, text: &'t Text) -> (Cow<'t, Text>, Vec<String>) {
    let face = |kernel: Kernel, script: Script| {
        let face = (project.meta().styles.get(&kernel)).and_then(|faces| faces.get(&script));
        face.map_or_else(CharStyle::default, |face| CharStyle {
            family: face.family.clone(),
            weight: face.weight,
            italic: face.italic,
            ..CharStyle::default()
        })
    };
    let script = text_script(text);
    let style = |kernel| script.map_or_else(CharStyle::default, |script| face(kernel, script));
    let antique = style(Kernel::Antique);
    let own = text.style.kernel.map_or_else(CharStyle::default, style);
    let latin = face(Kernel::Antique, script!("Latn"));
    let chars = text.style.chars.or(&own.or(&antique));
    let families = [antique.family, latin.family]
        .into_iter()
        .flatten()
        .collect();
    if chars == text.style.chars {
        return (Cow::Borrowed(text), families);
    }
    let mut text = text.clone();
    text.style.chars = chars;
    (Cow::Owned(text), families)
}

/// Renders pages. Cloning it shares its retained content.
#[derive(Clone)]
pub struct Renderer(Arc<Inner>);

struct Inner {
    cache: retained::Cache,
}

impl Renderer {
    /// The process's renderer, as `Gpu::get` is its GPU: what the app's
    /// canvas, exports, and pipeline draw with, so they share one retained
    /// cache.
    pub fn get() -> &'static Self {
        &RENDERER
    }

    /// What the first text would otherwise wait for loads in the background:
    /// Chinese line breaking's dictionary, the installed fonts, and the
    /// bundled font catalog. They load once, on a thread of their own rather
    /// than rayon's global pool, which the app may not have set up yet; should
    /// the thread fail to start, the first text that needs each loads it.
    #[must_use]
    pub fn new() -> Self {
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
            cache: retained::cache(),
        }))
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Renders `page` as `session` holds it: its background, then its layers,
    /// built across rayon. Content that is unchanged reuses what an earlier
    /// render built. A text whose bundled font isn't in yet waits while it
    /// downloads.
    pub fn render(&self, session: &Session, page: PageId) -> Result<Frame> {
        let project = session.project();
        let project = &project;
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
                    .map(|layer| self.layer(session, project, layer))
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

    /// Lays out `text`, which `layer` of `project` holds, as a render would,
    /// retaining nothing: for a text nothing draws, such as a block's source
    /// in a PSD, or one tried at a size. `None` when it has no characters.
    pub fn layout(
        &self,
        project: &Project,
        layer: LayerId,
        text: &Text,
    ) -> Result<Option<ResolvedText>> {
        let (text, families) = lettered(project, text);
        Ok(retained::laid(layer, &text, &families)?.map(|layout| layout.resolved))
    }

    /// `text`, which `layer` of `project` holds, laid out as a render lays it
    /// out, for typing into it; a content and style laid out once, as a
    /// render keeps them.
    pub fn lettering(&self, project: &Project, layer: LayerId, text: &Text) -> Result<Lettering> {
        Lettering::of(text, |text| {
            Ok(match self.text(project, layer, text)? {
                Some(Content::Text(layout)) => Some(layout),
                _ => None,
            })
        })
    }

    /// `layer`'s node, its children built across rayon.
    fn layer(&self, session: &Session, project: &Project, layer: &Layer) -> Result<Node> {
        let mut children = layer
            .children
            .par_iter()
            .filter(|child| !child.is_mask())
            .map(|child| self.layer(session, project, child))
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
                children = self.block(session, project, layer.id, block)?;
                None
            }
            SceneContent::Text(text) => self
                .text(project, layer.id, text)?
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
    fn block(
        &self,
        session: &Session,
        project: &Project,
        id: LayerId,
        block: &Block,
    ) -> Result<Vec<Node>> {
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
                .text(project, id, text)?
                .map(|content| (content, Affine::from(text.transform)));
            nodes.push(Node::new(None, true, content));
        }
        Ok(nodes)
    }
}
