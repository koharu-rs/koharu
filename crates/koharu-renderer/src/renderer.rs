//! The renderer: fonts, retained layer content, and the GPU, shared by every
//! frame it renders.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use image::RgbaImage;
use koharu_scene::{
    Align, Background, Block, Content as SceneContent, Direction, FillRule, Frame as TextFrame,
    Hash, LanguageIdentifier, Layer as SceneLayer, LayerId, Mask, Offset, PageId, PathFit,
    PixelFormat, Project, Session, Shape, Text, Tiles,
};
use parking_lot::RwLock;
use rayon::prelude::*;
use vello::{
    Scene,
    kurbo::{Affine, BezPath, Cap, Join, Rect, Shape as _, Stroke},
    peniko::{Color, Fill},
};

use crate::{
    Error, Result, TypesettingConfig,
    fonts::{FontPreview, FontRequest, Fonts},
    frame::{Content, Frame, FrameData, Layer, MaskNode, MaskShape, Node, NodeId, View},
    images,
    raster::Gpu,
    text::{self, Input, ResolvedText, Room, Span},
    tiles::Tiled,
    types::FontFamily,
};

const DEFAULT_SIZE: f64 = 24.0;
const DEFAULT_LINE_HEIGHT: f64 = 1.2;
/// What retained content may take, in bytes: pixels count with their tiles;
/// a vector scene counts as a kilobyte.
const CACHE_BYTES: u64 = 2 << 30;

/// Renders pages. Cloning it shares its fonts, caches, and GPU.
#[derive(Clone)]
pub struct Renderer(Arc<Inner>);

struct Inner {
    fonts: Fonts,
    typesetting: RwLock<TypesettingConfig>,
    /// Bumped when the typesetting preference or the loaded fonts change, so
    /// text lays out again.
    generation: AtomicU64,
    cache: quick_cache::sync::Cache<Key, Cached, Weight>,
    gpu: OnceLock<std::result::Result<Arc<Gpu>, String>>,
}

/// A raster: RGBA pixels and where their corner sits in page pixels.
#[derive(Debug)]
pub struct Raster {
    pub image: RgbaImage,
    pub left: i32,
    pub top: i32,
}

/// What retained content is keyed by: only what changes it. Texts and shapes
/// key on the hash of their canonical JSON (RFC 8785), so the key stays the
/// same for equal values whatever order a map's keys come in.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    /// A text's hash and the font generation.
    Text(Hash, u64),
    Shape(Hash),
    Image(Hash),
    Tiles(Hash),
}

/// What a piece of retained content is built from.
#[derive(Clone, Copy)]
enum Source<'a> {
    Text(LayerId, &'a Text),
    Shape(&'a Shape),
    Image(Hash),
    /// The picture of the layer or block part with this id.
    Tiles(LayerId),
}

#[derive(Clone)]
enum Cached {
    Vector {
        scene: Arc<Scene>,
        /// A text's strokes, drawn under `scene`.
        strokes: Option<Arc<Scene>>,
        bounds: Rect,
        text: Option<Arc<ResolvedText>>,
        missing: Vec<FontRequest>,
    },
    Pixels(Option<Arc<Tiled>>),
}

#[derive(Clone)]
struct Weight;

impl quick_cache::Weighter<Key, Cached> for Weight {
    fn weight(&self, _: &Key, cached: &Cached) -> u64 {
        match cached {
            Cached::Pixels(Some(tiled)) => tiled.bytes(),
            _ => 1024,
        }
    }
}

fn canonical(value: &impl serde::Serialize) -> Result<Hash> {
    let bytes = serde_json_canonicalizer::to_vec(value)
        .map_err(|error| Error::invalid(format!("a value has no canonical JSON: {error}")))?;
    Ok(Hash::of(&bytes))
}

impl Renderer {
    /// Fonts load and the GPU starts on first use, so this is cheap.
    #[must_use]
    pub fn new(typesetting: TypesettingConfig) -> Self {
        Self(Arc::new(Inner {
            fonts: Fonts::new(),
            typesetting: RwLock::new(typesetting),
            generation: AtomicU64::new(0),
            cache: quick_cache::sync::Cache::with_weighter(1024, CACHE_BYTES, Weight),
            gpu: OnceLock::new(),
        }))
    }

    /// Follows a changed typesetting preference; text lays out again.
    pub fn set_typesetting(&self, typesetting: TypesettingConfig) {
        *self.0.typesetting.write() = typesetting;
        self.0.generation.fetch_add(1, Ordering::Relaxed);
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
    /// Content that is unchanged reuses what an earlier render built.
    pub fn render(&self, session: &Session, page: PageId) -> Result<Frame> {
        let project = session.project();
        let view = project.page(page)?;
        images::check_size(view.size.width, view.size.height)?;
        // Build what isn't retained across rayon first; assembling the tree
        // then finds it all.
        sources(&project, page)?.par_iter().for_each(|source| {
            let _ = self.content(session, *source);
        });
        let mut builder = Builder {
            renderer: self,
            session,
            layers: HashMap::new(),
            order: Vec::new(),
            missing: Vec::new(),
        };
        let mut nodes = Vec::new();
        if let Some(background) = view.background {
            nodes.push(builder.background(background)?);
        }
        for layer in &view.layers {
            nodes.push(builder.node(layer, true)?);
        }
        let mut missing = builder.missing;
        missing.sort();
        missing.dedup();
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
            layers: Arc::new(builder.layers),
            order: builder.order.into(),
            missing: missing.into(),
        })))
    }

    /// Lays out `text`, which `layer` holds, as a render would: for a text
    /// nothing draws, such as a block's source in a PSD. `None` when it has no
    /// characters.
    pub fn lay_out(&self, layer: LayerId, text: &Text) -> Result<Option<Arc<ResolvedText>>> {
        match self.text_content(layer, text)? {
            Cached::Vector { text, .. } => Ok(text),
            Cached::Pixels(_) => Ok(None),
        }
    }

    /// Draws a frame's area at one output pixel per page pixel and reads it
    /// back.
    pub fn rasterize(&self, frame: &Frame) -> Result<Raster> {
        let area = frame.area();
        let (width, height) = (area.width().ceil() as u32, area.height().ceil() as u32);
        // What draws nothing, such as a block with no clean-up, is empty.
        if width == 0 || height == 0 {
            return Ok(Raster {
                image: RgbaImage::new(width, height),
                left: area.x0.floor() as i32,
                top: area.y0.floor() as i32,
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
        Ok(Raster {
            image,
            left: area.x0.floor() as i32,
            top: area.y0.floor() as i32,
        })
    }

    /// Downloads the bundled fonts `frame`'s text asked for, so the next
    /// render draws with them.
    pub async fn load_fonts(&self, frame: &Frame) -> Result<()> {
        self.0
            .fonts
            .load(&frame.0.missing)
            .await
            .map_err(Error::Fonts)?;
        self.0.generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn available_fonts(&self) -> Result<Vec<FontFamily>> {
        self.0.fonts.families().await.map_err(Error::Fonts)
    }

    /// A WebP of `family`'s name set in it, 96 pixels tall.
    pub async fn font_preview(&self, family: &str) -> Result<Vec<u8>> {
        const SIZE: f32 = 24.0;
        const HEIGHT: u32 = 96;
        let font = match self.0.fonts.preview(family).await.map_err(Error::Fonts)? {
            FontPreview::Webp(bytes) => return Ok(bytes),
            FontPreview::System(font) => *font,
        };
        let label = family.to_owned();
        let fonts = if font.renders(&label, SIZE) {
            vec![font]
        } else {
            self.0
                .fonts
                .resolve(Some("Arial"), 400, false, &[], &label, None)
                .map_err(Error::Fonts)?
                .0
        };
        let renderer = self.clone();
        tokio::task::spawn_blocking(move || {
            let laid = text::layout(&Input {
                spans: vec![Span {
                    range: 0..label.len(),
                    fonts,
                    size: SIZE,
                    fill: Color::BLACK,
                    stroke: None,
                }],
                text: label,
                vertical: false,
                align: Align::Start,
                line_height: DEFAULT_LINE_HEIGHT as f32,
                language: None,
                room: Room::Point,
            })
            .map_err(Error::Fonts)?;
            let frame = laid.resolved.frame;
            let scale = f64::from(HEIGHT) / frame.height().max(1.0);
            let width = (frame.width() * scale).ceil().max(1.0) as u32;
            let mut scene = Scene::new();
            scene.append(
                &laid.scene,
                Some(Affine::scale(scale) * Affine::translate((-frame.x0, -frame.y0))),
            );
            let image = renderer
                .gpu()?
                .read(&scene, width, HEIGHT)
                .map_err(Error::Backend)?;
            Ok(
                webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                    .encode(85.0)
                    .to_vec(),
            )
        })
        .await
        .map_err(|error| Error::Backend(error.into()))?
    }

    /// Content, retained by what it depends on.
    fn content(&self, session: &Session, source: Source) -> Result<Cached> {
        let key = match source {
            Source::Text(layer, text) => return self.text_content(layer, text),
            Source::Shape(shape) => Key::Shape(canonical(shape)?),
            Source::Image(file) => Key::Image(file),
            Source::Tiles(layer) => Key::Tiles(session.tiles_hash(layer)?),
        };
        if let Some(cached) = self.0.cache.get(&key) {
            return Ok(cached);
        }
        let cached = match source {
            Source::Tiles(layer) => {
                Cached::Pixels(assemble(session.tiles(layer, None)?)?.map(Arc::new))
            }
            Source::Image(file) => {
                let decoded = images::decode(&file, &session.file(&file)?)?;
                Cached::Pixels(Some(Arc::new(Tiled::new((0, 0), decoded))))
            }
            Source::Shape(shape) => {
                let path = parse(shape.path.as_str())?;
                let mut scene = Scene::new();
                let fill = match shape.fill_rule {
                    FillRule::NonZero => Fill::NonZero,
                    FillRule::EvenOdd => Fill::EvenOdd,
                };
                if let Some(color) = shape.fill {
                    scene.fill(fill, Affine::IDENTITY, Color::from(color), None, &path);
                }
                let mut bounds = path.bounding_box();
                if let Some(stroke) = &shape.stroke {
                    // Centred on the path, with miter joins and butt caps.
                    let style = Stroke::new(stroke.width)
                        .with_join(Join::Miter)
                        .with_caps(Cap::Butt);
                    scene.stroke(
                        &style,
                        Affine::IDENTITY,
                        Color::from(stroke.color),
                        None,
                        &path,
                    );
                    bounds = bounds.inflate(stroke.width * 2.0, stroke.width * 2.0);
                }
                Cached::Vector {
                    scene: Arc::new(scene),
                    strokes: None,
                    bounds,
                    text: None,
                    missing: Vec::new(),
                }
            }
            Source::Text(..) => unreachable!("texts return above"),
        };
        self.0.cache.insert(key, cached.clone());
        Ok(cached)
    }

    fn text_content(&self, id: LayerId, layer: &Text) -> Result<Cached> {
        // The transform places the layout when it's drawn, so moving a text
        // keeps its layout. Naming every field fails to build when one is
        // added, until the key says whether it counts.
        let Text {
            content,
            style,
            frame,
            direction,
            language,
            transform: _,
        } = layer;
        let laid_out = canonical(&(content, style, frame, direction, language))?;
        let key = Key::Text(laid_out, self.0.generation.load(Ordering::Relaxed));
        if let Some(cached) = self.0.cache.get(&key) {
            return Ok(cached);
        }
        let cached = self.text(id, layer)?;
        self.0.cache.insert(key, cached.clone());
        Ok(cached)
    }

    fn text(&self, id: LayerId, layer: &Text) -> Result<Cached> {
        let style = &layer.style;
        let language =
            (layer.language != LanguageIdentifier::UNKNOWN).then(|| layer.language.to_string());
        let families = self.0.typesetting.read().font_families.clone();
        let mut text = String::new();
        let mut spans = Vec::new();
        let mut missing = Vec::new();
        for span in &layer.content.0 {
            if span.insert.is_empty() {
                continue;
            }
            let chars = span.attributes.or(&style.chars);
            let (fonts, request) = self
                .0
                .fonts
                .resolve(
                    chars.family.as_deref(),
                    chars.weight.unwrap_or(400),
                    chars.italic.unwrap_or(false),
                    &families,
                    &span.insert,
                    language.as_deref(),
                )
                .map_err(|source| Error::Text { layer: id, source })?;
            missing.extend(request);
            let start = text.len();
            text.push_str(&span.insert);
            spans.push(Span {
                range: start..text.len(),
                fonts,
                size: chars.size.unwrap_or(DEFAULT_SIZE) as f32,
                fill: chars.fill.map_or(Color::BLACK, Color::from),
                stroke: match (chars.stroke, chars.stroke_width) {
                    (Some(color), Some(width)) if width > 0.0 => {
                        Some((Color::from(color), width as f32))
                    }
                    _ => None,
                },
            });
        }
        let room = match &layer.frame {
            TextFrame::Point => Room::Point,
            TextFrame::Box {
                bounds,
                fit,
                anchor,
            } => Room::Box {
                bounds: Rect::new(
                    bounds.x,
                    bounds.y,
                    bounds.x + bounds.width,
                    bounds.y + bounds.height,
                ),
                shrink: *fit == koharu_scene::Fit::Shrink,
                grow: *fit == koharu_scene::Fit::Grow,
                anchor: *anchor,
            },
            TextFrame::Path {
                path,
                fit,
                anchor,
                inset,
            } => Room::Path {
                outline: parse(path.as_str())?,
                shrink: *fit == PathFit::Shrink,
                anchor: *anchor,
                inset: *inset as f32,
            },
        };
        if spans.is_empty() {
            return Ok(Cached::Vector {
                scene: Arc::new(Scene::new()),
                strokes: None,
                bounds: Rect::ZERO,
                text: None,
                missing,
            });
        }
        let laid = text::layout(&Input {
            text,
            spans,
            vertical: layer.direction == Direction::Vertical,
            align: style.align.unwrap_or_default(),
            line_height: style.line_height.unwrap_or(DEFAULT_LINE_HEIGHT) as f32,
            language,
            room,
        })
        .map_err(|source| Error::Text { layer: id, source })?;
        Ok(Cached::Vector {
            scene: Arc::new(laid.scene),
            strokes: laid.strokes.map(Arc::new),
            bounds: laid.bounds,
            text: Some(Arc::new(laid.resolved)),
            missing,
        })
    }
}

/// Everything a page's content is built from: its background, and each
/// layer's pixels, image, shape, or text.
fn sources(project: &Project, page: PageId) -> Result<Vec<Source<'_>>> {
    let mut out: Vec<Source> = project
        .page(page)?
        .background
        .map(|background| Source::Image(background.file))
        .into_iter()
        .collect();
    for layer in project.layers_of(page)? {
        match &layer.content {
            SceneContent::Raster(_) | SceneContent::Mask(_) => {
                out.push(Source::Tiles(layer.id));
            }
            SceneContent::Image(image) => out.push(Source::Image(image.file)),
            SceneContent::Shape(shape) => out.push(Source::Shape(shape)),
            SceneContent::Text(text) => out.push(Source::Text(layer.id, text)),
            SceneContent::Block(block) => {
                out.push(Source::Tiles(block.cleanup.id));
                out.push(Source::Tiles(block.mask.id));
                if let Some(text) = &block.translation {
                    out.push(Source::Text(layer.id, text));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn parse(path: &str) -> Result<BezPath> {
    BezPath::from_svg(path).map_err(|error| Error::invalid(format!("invalid path data: {error}")))
}

/// Where a picture's grid sits on the page.
fn offset(offset: Offset) -> Affine {
    Affine::translate((f64::from(offset.x), f64::from(offset.y)))
}

/// The scene's 64×64 tiles as one image, cropped to its painted pixels and
/// placed relative to the plane's offset; a grey plane becomes alpha under
/// white. Tiles spread wider than a page could hold fail.
fn assemble(tiles: Tiles) -> Result<Option<Tiled>> {
    const TILE: i32 = 64;
    let Some(x0) = tiles.tiles.iter().map(|tile| tile.x).min() else {
        return Ok(None);
    };
    let y0 = tiles.tiles.iter().map(|tile| tile.y).min().unwrap_or(0);
    let x1 = tiles.tiles.iter().map(|tile| tile.x).max().unwrap_or(0) + 1;
    let y1 = tiles.tiles.iter().map(|tile| tile.y).max().unwrap_or(0) + 1;
    let extent = |from: i32, to: i32| {
        u32::try_from(i64::from(to - from) * i64::from(TILE)).unwrap_or(u32::MAX)
    };
    let (width, height) = (extent(x0, x1), extent(y0, y1));
    images::check_size(width, height)?;
    let mut image = RgbaImage::new(width, height);
    let row = TILE as usize * 4;
    for tile in &tiles.tiles {
        let (left, top) = (
            ((tile.x - x0) * TILE) as usize,
            ((tile.y - y0) * TILE) as usize,
        );
        let raw: &mut [u8] = &mut image;
        for line in 0..TILE as usize {
            let at = ((top + line) * width as usize + left) * 4;
            let target = &mut raw[at..at + row];
            match tiles.format {
                PixelFormat::Rgba8 => {
                    target.copy_from_slice(&tile.pixels[line * row..(line + 1) * row])
                }
                PixelFormat::Gray8 => {
                    let source = &tile.pixels[line * TILE as usize..(line + 1) * TILE as usize];
                    for (pixel, grey) in target.as_chunks_mut::<4>().0.iter_mut().zip(source) {
                        *pixel = [255, 255, 255, *grey];
                    }
                }
            }
        }
    }
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] > 0 {
            (left, top) = (left.min(x), top.min(y));
            (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
        }
    }
    if right <= left {
        return Ok(None);
    }
    let painted =
        image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image();
    let origin = (x0 * TILE + left as i32, y0 * TILE + top as i32);
    Ok(Some(Tiled::new(origin, painted)))
}

struct Builder<'r, 's> {
    renderer: &'r Renderer,
    session: &'s Session,
    layers: HashMap<NodeId, Layer>,
    order: Vec<NodeId>,
    missing: Vec<FontRequest>,
}

impl Builder<'_, '_> {
    fn record(
        &mut self,
        id: NodeId,
        visible: bool,
        bounds: Option<Rect>,
        text: Option<Arc<ResolvedText>>,
    ) {
        self.layers.insert(
            id,
            Layer {
                id,
                visible,
                bounds,
                text,
            },
        );
    }

    fn pixels(&self, source: Source, transform: Affine) -> Result<(Content, Option<Rect>)> {
        Ok(match self.renderer.content(self.session, source)? {
            Cached::Pixels(Some(tiled)) => {
                let bounds = tiled.bounds(transform);
                (Content::Pixels { tiled, transform }, Some(bounds))
            }
            _ => (Content::None, None),
        })
    }

    fn vector(
        &mut self,
        source: Source,
        transform: Affine,
    ) -> Result<(Content, Option<Rect>, Option<Arc<ResolvedText>>)> {
        let Cached::Vector {
            scene,
            strokes,
            bounds,
            text,
            missing,
        } = self.renderer.content(self.session, source)?
        else {
            return Ok((Content::None, None, None));
        };
        self.missing.extend(missing);
        let shown = (bounds.area() > 0.0).then(|| transform.transform_rect_bbox(bounds));
        Ok((
            Content::Vector {
                scene,
                strokes,
                transform,
            },
            shown,
            text,
        ))
    }

    fn background(&mut self, background: Background) -> Result<Node> {
        let id = NodeId::Background;
        self.order.push(id);
        let (content, bounds) = self.pixels(Source::Image(background.file), Affine::IDENTITY)?;
        self.record(id, background.visible, bounds, None);
        Ok(Node {
            id,
            visible: background.visible,
            opacity: 1.0,
            blend: koharu_scene::Blend::Normal,
            content,
            children: Vec::new(),
            masks: Vec::new(),
        })
    }

    fn node(&mut self, layer: &SceneLayer, shown: bool) -> Result<Node> {
        let id = NodeId::Layer(layer.id);
        let shown_here = shown && layer.visible;
        self.order.push(id);
        let mut children = layer
            .children
            .iter()
            .filter(|child| !child.is_mask())
            .map(|child| self.node(child, shown_here))
            .collect::<Result<Vec<_>>>()?;
        let mut masks = layer
            .children
            .iter()
            .filter(|child| child.is_mask())
            .map(|mask| self.mask(mask, shown_here))
            .collect::<Result<Vec<_>>>()?;
        let (content, bounds, text) = match &layer.content {
            SceneContent::Group(_) | SceneContent::Balloon(_) => (Content::None, None, None),
            // A panel shows its children only inside its outline, as a Clip
            // Studio Paint frame border folder does.
            SceneContent::Panel(panel) => {
                masks.push(MaskNode {
                    visible: true,
                    mode: koharu_scene::MaskMode::Add,
                    shape: MaskShape::Path(Arc::new(parse(panel.outline.as_str())?)),
                });
                (Content::None, None, None)
            }
            SceneContent::Block(block) => {
                children = self.block(layer.id, block, shown_here)?;
                (Content::None, None, None)
            }
            SceneContent::Text(text) => {
                self.vector(Source::Text(layer.id, text), Affine::from(text.transform))?
            }
            SceneContent::Shape(shape) => self.vector(Source::Shape(shape), Affine::IDENTITY)?,
            SceneContent::Image(image) => {
                let (content, bounds) =
                    self.pixels(Source::Image(image.file), Affine::from(image.transform))?;
                (content, bounds, None)
            }
            SceneContent::Raster(raster) => {
                let (content, bounds) =
                    self.pixels(Source::Tiles(layer.id), offset(raster.offset))?;
                (content, bounds, None)
            }
            SceneContent::Mask(_) => {
                return Err(Error::invalid(format!(
                    "mask {} is drawn only as its parent's",
                    layer.id
                )));
            }
        };
        let bounds = match content {
            Content::None => children
                .iter()
                .filter_map(|child| self.layers.get(&child.id).and_then(Layer::bounds))
                .reduce(|a, b| a.union(b)),
            _ => bounds,
        };
        self.record(id, shown_here, bounds, text);
        Ok(Node {
            id,
            visible: layer.visible,
            opacity: f32::from(layer.opacity),
            blend: layer.blend,
            content,
            children,
            masks,
        })
    }

    /// A block draws its clean-up through its mask, then its translation; its
    /// source is never drawn.
    fn block(&mut self, block: LayerId, value: &Block, shown: bool) -> Result<Vec<Node>> {
        let cleanup = &value.cleanup;
        let id = NodeId::Layer(cleanup.id);
        self.order.push(id);
        let (content, bounds) =
            self.pixels(Source::Tiles(cleanup.id), offset(cleanup.content.offset))?;
        let (shape, _) = self.mask_shape(value.mask.id, &value.mask.content)?;
        self.record(id, shown && cleanup.visible, bounds, None);
        let mut nodes = vec![Node {
            id,
            visible: cleanup.visible,
            opacity: f32::from(cleanup.opacity),
            blend: cleanup.blend,
            content,
            children: Vec::new(),
            masks: vec![MaskNode {
                visible: value.mask.visible,
                mode: value.mask.content.mode,
                shape,
            }],
        }];
        if let Some(text) = &value.translation {
            let translation = NodeId::Translation(block);
            self.order.push(translation);
            let (content, bounds, resolved) =
                self.vector(Source::Text(block, text), Affine::from(text.transform))?;
            self.record(translation, shown, bounds, resolved);
            nodes.push(Node {
                id: translation,
                visible: true,
                opacity: 1.0,
                blend: koharu_scene::Blend::Normal,
                content,
                children: Vec::new(),
                masks: Vec::new(),
            });
        }
        Ok(nodes)
    }

    /// The pixels of a mask, a mask layer's or a block's, as the shape it
    /// covers.
    fn mask_shape(&self, id: LayerId, mask: &Mask) -> Result<(MaskShape, Option<Rect>)> {
        Ok(
            match self.renderer.content(self.session, Source::Tiles(id))? {
                Cached::Pixels(Some(tiled)) => {
                    let transform = offset(mask.offset);
                    let bounds = tiled.bounds(transform);
                    (MaskShape::Pixels { tiled, transform }, Some(bounds))
                }
                // An empty mask covers nothing.
                _ => (MaskShape::Path(Arc::new(BezPath::new())), None),
            },
        )
    }

    fn mask(&mut self, layer: &SceneLayer, shown: bool) -> Result<MaskNode> {
        let SceneContent::Mask(mask) = &layer.content else {
            return Err(Error::invalid(format!("layer {} is not a mask", layer.id)));
        };
        let id = NodeId::Layer(layer.id);
        self.order.push(id);
        let (shape, bounds) = self.mask_shape(layer.id, mask)?;
        self.record(id, shown && layer.visible, bounds, None);
        Ok(MaskNode {
            visible: layer.visible,
            mode: mask.mode,
            shape,
        })
    }
}
