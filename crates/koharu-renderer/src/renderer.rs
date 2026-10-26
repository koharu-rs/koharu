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
    Align, Direction, FillRule, Frame as TextFrame, Hash, LanguageIdentifier, Layer as SceneLayer,
    LayerId, LayerKind, LayerView, MaskContent, PageId, PathFit, PixelFormat, Session, Tiles,
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
    frame::{Content, Frame, FrameData, Layer, MaskNode, MaskShape, Node, View},
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

/// What a layer's retained content is keyed by: only what changes it.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    /// The layer's hash, its text style's, and the font generation.
    Text(Hash, Option<Hash>, u64),
    Shape(Hash),
    Image(Hash),
    Tiles(Hash),
}

#[derive(Clone)]
enum Cached {
    Vector {
        scene: Arc<Scene>,
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

    /// Renders `page` as `session` holds it. Layers whose content is
    /// unchanged reuse what an earlier render built.
    pub fn render(&self, session: &Session, page: PageId) -> Result<Frame> {
        let view = session.page(page)?;
        images::check_size(view.size.width, view.size.height)?;
        // Build what isn't retained across rayon first; assembling the tree
        // then finds it all.
        let mut flat = Vec::new();
        flatten(&view.layers, &mut flat);
        flat.par_iter().for_each(|layer| {
            let _ = self.content(session, layer);
        });
        let mut builder = Builder {
            renderer: self,
            session,
            layers: HashMap::new(),
            order: Vec::new(),
            missing: Vec::new(),
        };
        let nodes = view
            .layers
            .iter()
            .map(|layer| builder.node(layer, true))
            .collect::<Result<Vec<_>>>()?;
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

    /// Draws a frame's area at one output pixel per page pixel and reads it
    /// back.
    pub fn rasterize(&self, frame: &Frame) -> Result<Raster> {
        let area = frame.area();
        let (width, height) = (area.width().ceil() as u32, area.height().ceil() as u32);
        images::check_size(width, height)?;
        if width == 0 || height == 0 {
            return Ok(Raster {
                image: RgbaImage::new(width, height),
                left: area.x0.floor() as i32,
                top: area.y0.floor() as i32,
            });
        }
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
        tokio_rayon::spawn(move || {
            let laid = text::lay_out(&Input {
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
    }

    /// A layer's content, retained by what it depends on.
    fn content(&self, session: &Session, view: &LayerView) -> Result<Option<Cached>> {
        let key = match &view.layer {
            SceneLayer::Group(_) => return Ok(None),
            SceneLayer::Mask(mask) => match &mask.content {
                MaskContent::Path { .. } => return Ok(None),
                MaskContent::Pixels { .. } => Key::Tiles(session.tiles_hash(view.id)?),
            },
            SceneLayer::Raster(_) => Key::Tiles(session.tiles_hash(view.id)?),
            SceneLayer::Image(image) => Key::Image(image.file.clone()),
            SceneLayer::Shape(_) => Key::Shape(view.hash.clone()),
            SceneLayer::Text(text) => {
                let style = text
                    .text_style
                    .map(|style| session.style(style))
                    .transpose()?
                    .map(|style| style.hash);
                Key::Text(
                    view.hash.clone(),
                    style,
                    self.0.generation.load(Ordering::Relaxed),
                )
            }
        };
        if let Some(cached) = self.0.cache.get(&key) {
            return Ok(Some(cached));
        }
        let cached = match (&view.layer, &key) {
            (SceneLayer::Raster(_) | SceneLayer::Mask(_), _) => {
                Cached::Pixels(assemble(session.tiles(view.id, None)?)?.map(Arc::new))
            }
            (SceneLayer::Image(image), _) => {
                let file = session.file(&image.file)?;
                let decoded = images::decode(&image.file, &file.bytes)?;
                Cached::Pixels(Some(Arc::new(Tiled::new((0, 0), decoded))))
            }
            (SceneLayer::Shape(shape), _) => {
                let path = parse(shape.path.as_str())?;
                let mut scene = Scene::new();
                let fill = match shape.fill_rule {
                    FillRule::NonZero => Fill::NonZero,
                    FillRule::EvenOdd => Fill::EvenOdd,
                };
                if let Some(color) = shape.fill {
                    scene.fill(fill, Affine::IDENTITY, color, None, &path);
                }
                let mut bounds = path.bounding_box();
                if let Some(stroke) = &shape.stroke {
                    // Centred on the path, with miter joins and butt caps.
                    let style = Stroke::new(stroke.width)
                        .with_join(Join::Miter)
                        .with_caps(Cap::Butt);
                    scene.stroke(&style, Affine::IDENTITY, stroke.color, None, &path);
                    bounds = bounds.inflate(stroke.width * 2.0, stroke.width * 2.0);
                }
                Cached::Vector {
                    scene: Arc::new(scene),
                    bounds,
                    text: None,
                    missing: Vec::new(),
                }
            }
            (SceneLayer::Text(layer), _) => self.text(session, view.id, layer)?,
            _ => unreachable!("every keyed layer is built"),
        };
        self.0.cache.insert(key, cached.clone());
        Ok(Some(cached))
    }

    fn text(
        &self,
        session: &Session,
        id: LayerId,
        layer: &koharu_scene::TextLayer,
    ) -> Result<Cached> {
        let style = match layer.text_style {
            Some(named) => layer.style.or(&session.style(named)?.style.style),
            None => layer.style.clone(),
        };
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
                fill: chars.fill.unwrap_or(Color::BLACK),
                stroke: match (chars.stroke, chars.stroke_width) {
                    (Some(color), Some(width)) if width > 0.0 => Some((color, width as f32)),
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
            TextFrame::Path { path, fit, anchor } => Room::Path {
                outline: parse(path.as_str())?,
                shrink: *fit == PathFit::Shrink,
                anchor: *anchor,
            },
        };
        if spans.is_empty() {
            return Ok(Cached::Vector {
                scene: Arc::new(Scene::new()),
                bounds: Rect::ZERO,
                text: None,
                missing,
            });
        }
        let laid = text::lay_out(&Input {
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
            bounds: laid.bounds,
            text: Some(Arc::new(laid.resolved)),
            missing,
        })
    }
}

/// Every layer and mask in a page's tree.
fn flatten<'a>(layers: &'a [LayerView], out: &mut Vec<&'a LayerView>) {
    for layer in layers {
        out.push(layer);
        flatten(&layer.children, out);
        flatten(&layer.masks, out);
    }
}

fn parse(path: &str) -> Result<BezPath> {
    BezPath::from_svg(path).map_err(|error| Error::invalid(format!("invalid path data: {error}")))
}

/// The scene's 64×64 tiles as one image, cropped to its painted pixels and
/// placed relative to the layer's offset; a pixel mask's grey becomes alpha
/// under white. Tiles spread wider than a page could hold fail.
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
    layers: HashMap<LayerId, Layer>,
    order: Vec<LayerId>,
    missing: Vec<FontRequest>,
}

impl Builder<'_, '_> {
    fn node(&mut self, view: &LayerView, shown: bool) -> Result<Node> {
        let (visible, opacity, blend) = match &view.layer {
            SceneLayer::Group(layer) => (layer.visible, layer.opacity, layer.blend),
            SceneLayer::Image(layer) => (layer.visible, layer.opacity, layer.blend),
            SceneLayer::Raster(layer) => (layer.visible, layer.opacity, layer.blend),
            SceneLayer::Shape(layer) => (layer.visible, layer.opacity, layer.blend),
            SceneLayer::Text(layer) => (layer.visible, layer.opacity, layer.blend),
            SceneLayer::Mask(layer) => (layer.visible, 1.0, koharu_scene::Blend::Normal),
        };
        self.order.push(view.id);
        let children = view
            .children
            .iter()
            .map(|child| self.node(child, shown && visible))
            .collect::<Result<Vec<_>>>()?;
        let masks = view
            .masks
            .iter()
            .map(|mask| self.mask(mask, shown && visible))
            .collect::<Result<Vec<_>>>()?;
        let (content, bounds, text) = match self.renderer.content(self.session, view)? {
            None => {
                let bounds = children
                    .iter()
                    .filter_map(|child| self.layers.get(&child.id).and_then(Layer::bounds))
                    .reduce(|a, b| a.union(b));
                (Content::None, bounds, None)
            }
            Some(Cached::Vector {
                scene,
                bounds,
                text,
                missing,
            }) => {
                self.missing.extend(missing);
                let transform = match &view.layer {
                    SceneLayer::Text(layer) => Affine::from(layer.transform),
                    _ => Affine::IDENTITY,
                };
                let shown_bounds =
                    (bounds.area() > 0.0).then(|| transform.transform_rect_bbox(bounds));
                (Content::Vector { scene, transform }, shown_bounds, text)
            }
            Some(Cached::Pixels(None)) => (Content::None, None, None),
            Some(Cached::Pixels(Some(tiled))) => {
                let transform = match &view.layer {
                    SceneLayer::Image(layer) => Affine::from(layer.transform),
                    SceneLayer::Raster(layer) => {
                        Affine::translate((f64::from(layer.offset.x), f64::from(layer.offset.y)))
                    }
                    _ => Affine::IDENTITY,
                };
                let bounds = transform.transform_rect_bbox(Rect::new(
                    f64::from(tiled.origin.0),
                    f64::from(tiled.origin.1),
                    f64::from(tiled.origin.0) + f64::from(tiled.width),
                    f64::from(tiled.origin.1) + f64::from(tiled.height),
                ));
                (Content::Pixels { tiled, transform }, Some(bounds), None)
            }
        };
        self.layers.insert(
            view.id,
            Layer {
                id: view.id,
                kind: LayerKind::from(&view.layer),
                visible: shown && visible,
                bounds,
                text,
            },
        );
        Ok(Node {
            id: view.id,
            visible,
            opacity,
            blend,
            content,
            children,
            masks,
        })
    }

    fn mask(&mut self, view: &LayerView, shown: bool) -> Result<MaskNode> {
        let SceneLayer::Mask(mask) = &view.layer else {
            return Err(Error::invalid(format!("layer {} is not a mask", view.id)));
        };
        self.order.push(view.id);
        let (shape, bounds) = match &mask.content {
            MaskContent::Path { path } => {
                let path = parse(path.as_str())?;
                let bounds = path.bounding_box();
                (MaskShape::Path(Arc::new(path)), Some(bounds))
            }
            MaskContent::Pixels { offset } => match self.renderer.content(self.session, view)? {
                Some(Cached::Pixels(Some(tiled))) => {
                    let transform = Affine::translate((f64::from(offset.x), f64::from(offset.y)));
                    let bounds = transform.transform_rect_bbox(Rect::new(
                        f64::from(tiled.origin.0),
                        f64::from(tiled.origin.1),
                        f64::from(tiled.origin.0) + f64::from(tiled.width),
                        f64::from(tiled.origin.1) + f64::from(tiled.height),
                    ));
                    (MaskShape::Pixels { tiled, transform }, Some(bounds))
                }
                // An empty pixel mask covers nothing.
                _ => (MaskShape::Path(Arc::new(BezPath::new())), None),
            },
        };
        self.layers.insert(
            view.id,
            Layer {
                id: view.id,
                kind: LayerKind::Mask,
                visible: shown && mask.visible,
                bounds,
                text: None,
            },
        );
        Ok(MaskNode {
            visible: mask.visible,
            mode: mask.mode,
            shape,
        })
    }
}
