//! Writes a Koharu page as a layered PSD with photocraft-psd.

mod blocks;
mod engine_data;

use koharu_renderer::{Drawn, Frame, Raster, Renderer};
use koharu_scene::{
    Background, Balloon, Blend, Block, Content, FillRule, Layer, LayerId, Offset, PageId, Path,
    Rgba, Session, Shape,
};
use kurbo::BezPath;
use photocraft_psd::{
    BlendMode, ChannelData, Compression, GroupSpec, LayerMask, LayerRecord, LayerSpec, MaskData,
    MaskSpec, PixelData, PsdBuilder, PsdError, SectionType, TaggedBlock, Version,
};

/// The most pixels a PSD holds on a side; larger documents need PSB.
const MAX_SIDE: u32 = 30_000;
/// `lspf`'s "lock all" flag, as ag-psd reads it.
const LOCK_ALL: u32 = 0x8000_0000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Scene(#[from] koharu_scene::Error),

    #[error(transparent)]
    Render(#[from] koharu_renderer::Error),

    #[error("the PSD couldn't be written")]
    Psd(#[from] PsdError),

    #[error("invalid path data: {0}")]
    Path(String),

    #[error("a {width}×{height} page is larger than a PSD's {MAX_SIDE} pixels on a side")]
    TooLarge { width: u32, height: u32 },

    #[error("the rendered page has no {0}")]
    NotDrawn(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The names of the layers a PSD needs where the scene has none, in the app's
/// language. A type layer takes its text as its name, as Photoshop names it.
#[derive(Clone, Debug)]
pub struct Labels {
    /// The page's background, the bottom pixel layer.
    pub background: String,
    /// A block's clean-up.
    pub cleanup: String,
    /// The hidden shape holding a balloon's outline.
    pub outline: String,
}

/// `page` as a PSD: the renderer's composite as the merged image, and each
/// layer with its own pixels, properties, and masks; shapes and text also
/// keep what lets Photoshop edit them. PSD has no panels, balloons, or
/// blocks, so each becomes a folder of ordinary layers. The page is rendered
/// once, so call it off the UI thread.
pub fn export(
    session: &Session,
    renderer: &Renderer,
    page: PageId,
    labels: &Labels,
) -> Result<Vec<u8>> {
    let project = session.project();
    let view = project.page(page)?;
    let (width, height) = (view.size.width, view.size.height);
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(Error::TooLarge { width, height });
    }
    let frame = renderer.render(session, page)?;
    let mut export = Export {
        renderer,
        frame: &frame,
        labels,
        size: (width, height),
        builder: PsdBuilder::new(width, height).resolution(72.0),
        folders: Vec::new(),
        texts: 0,
    };
    if let Some(background) = view.background {
        export.background(background)?;
    }
    for layer in &view.layers {
        export.layer(layer)?;
    }
    let composite = renderer.rasterize(&frame)?;
    export
        .builder
        .composite(PixelData::Rgba8(composite.image.into_raw()));
    let mut file = export.builder.build()?;
    // A group's folder record follows its children's, so folder records
    // come in the order `layer` finished their groups.
    let folders = file
        .layers_mut()
        .iter_mut()
        .filter(|record| record.section_type() == SectionType::OpenFolder);
    for (record, (blocks, mask)) in folders.zip(export.folders) {
        record.blocks.extend(blocks);
        if let Some(mask) = mask {
            add_mask(record, mask)?;
        }
    }
    for record in file.layers_mut() {
        // Photoshop marks a shape's pixels as derived from its path, and
        // readers such as psd-tools take the mark for a shape layer; only a
        // shape has a fill block.
        if record.block(b"SoCo").is_some() || record.block(b"vscg").is_some() {
            record.flags.set_pixel_data_irrelevant(true);
        }
    }
    Ok(file.to_bytes()?)
}

struct Export<'a> {
    renderer: &'a Renderer,
    frame: &'a Frame,
    labels: &'a Labels,
    size: (u32, u32),
    builder: PsdBuilder,
    /// Each group's blocks and mask, which the builder can't give its folder
    /// record, in the order its groups end.
    folders: Vec<(Vec<TaggedBlock>, Option<MaskSpec>)>,
    texts: i32,
}

/// A pixel layer's properties.
struct Leaf<'n> {
    name: &'n str,
    visible: bool,
    opacity: u8,
    blend: Blend,
}

fn alpha(opacity: impl Into<f32>) -> u8 {
    (opacity.into() * 255.0).round() as u8
}

/// What a frame drew, which a render draws for every layer, the page's
/// background, and each translation; `what` names it when it's missing.
fn drawn<'f>(found: Option<Drawn<'f>>, what: impl FnOnce() -> String) -> Result<Drawn<'f>> {
    found.ok_or_else(|| Error::NotDrawn(what()))
}

impl Export<'_> {
    fn background(&mut self, background: Background) -> Result<()> {
        let name = self.labels.background.clone();
        let drawn = drawn(self.frame.background(), || "background".to_owned())?;
        self.leaf(
            Some(drawn),
            Leaf {
                name: &name,
                visible: background.visible,
                opacity: 255,
                blend: Blend::Normal,
            },
            Vec::new(),
            None,
        )
    }

    fn layer(&mut self, layer: &Layer) -> Result<()> {
        let leaf = Leaf {
            name: &layer.name,
            visible: layer.visible,
            opacity: alpha(layer.opacity),
            blend: layer.blend,
        };
        let mut blocks = Vec::new();
        if layer.locked {
            blocks.push(TaggedBlock::protection(LOCK_ALL));
        }
        // A mask is its layer's, never one of the drawn children.
        if let Content::Mask(_) = layer.content {
            return Ok(());
        }
        let drawn = drawn(self.frame.layer(layer.id), || format!("layer {}", layer.id))?;
        match &layer.content {
            Content::Group(_) => {
                self.folder(layer, blocks, |export| export.children(layer))?;
                return Ok(());
            }
            // A panel's folder is masked by its outline.
            Content::Panel(panel) => {
                blocks.push(blocks::vector_mask(
                    &parse(&panel.outline)?,
                    FillRule::NonZero,
                    self.size,
                ));
                self.folder(layer, blocks, |export| export.children(layer))?;
                return Ok(());
            }
            Content::Balloon(balloon) => {
                self.folder(layer, blocks, |export| {
                    export.outline(balloon)?;
                    export.children(layer)
                })?;
                return Ok(());
            }
            Content::Block(block) => {
                self.folder(layer, blocks, |export| export.block(layer.id, block))?;
                return Ok(());
            }
            Content::Shape(shape) => blocks.extend(blocks::shape(shape, self.size)?),
            // A text that draws nothing has no runs to style, so it stays an
            // empty pixel layer.
            Content::Text(text) => {
                if let Some(resolved) = drawn.text()
                    && !resolved.runs.is_empty()
                {
                    blocks.extend(blocks::type_layer(text, resolved, self.texts));
                    self.texts += 1;
                }
            }
            Content::Mask(_) | Content::Image(_) | Content::Raster(_) => {}
        }
        let mask = self.masks(layer)?;
        self.leaf(Some(drawn), leaf, blocks, mask)
    }

    fn children(&mut self, layer: &Layer) -> Result<()> {
        for child in &layer.children {
            self.layer(child)?;
        }
        Ok(())
    }

    /// A folder for `layer`, with what `inside` adds, its blocks, and its
    /// masks.
    fn folder(
        &mut self,
        layer: &Layer,
        blocks: Vec<TaggedBlock>,
        inside: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        self.builder.begin_group(GroupSpec {
            name: layer.name.clone(),
            blend_mode: blend_mode(layer.blend),
            opacity: alpha(layer.opacity),
            visible: layer.visible,
            open: true,
        });
        inside(self)?;
        self.builder.end_group()?;
        let mask = self.masks(layer)?;
        self.folders.push((blocks, mask));
        Ok(())
    }

    /// A balloon's outline as a hidden shape layer, white like a new Clip
    /// Studio Paint balloon, so showing it draws the balloon.
    fn outline(&mut self, balloon: &Balloon) -> Result<()> {
        let shape = Shape {
            path: balloon.outline.clone(),
            fill_rule: FillRule::NonZero,
            fill: Some(Rgba([1.0, 1.0, 1.0, 1.0])),
            stroke: None,
        };
        let name = self.labels.outline.clone();
        let blocks = blocks::shape(&shape, self.size)?;
        self.leaf(
            None,
            Leaf {
                name: &name,
                visible: false,
                opacity: 255,
                blend: Blend::Normal,
            },
            blocks,
            None,
        )
    }

    /// A block's parts, bottom to top: its clean-up, masked by its mask; its
    /// source as a hidden type layer; and its translation as a type layer.
    fn block(&mut self, id: LayerId, block: &Block) -> Result<()> {
        let cleanup = &block.cleanup;
        let cleaned = drawn(self.frame.layer(cleanup.id), || {
            format!("clean-up {}", cleanup.id)
        })?;
        let mask = self.user_mask(cleaned)?;
        let name = match cleanup.name.is_empty() {
            true => self.labels.cleanup.clone(),
            false => cleanup.name.clone(),
        };
        self.leaf(
            Some(cleaned),
            Leaf {
                name: &name,
                visible: cleanup.visible,
                opacity: alpha(cleanup.opacity),
                blend: cleanup.blend,
            },
            Vec::new(),
            mask,
        )?;

        let source = &block.source;
        let mut blocks = Vec::new();
        if let Some(resolved) = self.renderer.lay_out(id, source)?
            && !resolved.runs.is_empty()
        {
            blocks.extend(blocks::type_layer(source, &resolved, self.texts));
            self.texts += 1;
        }
        let name = source.content.text();
        self.leaf(
            None,
            Leaf {
                name: &name,
                visible: false,
                opacity: 255,
                blend: Blend::Normal,
            },
            blocks,
            None,
        )?;

        let Some(text) = &block.translation else {
            return Ok(());
        };
        let translation = drawn(self.frame.translation(id), || {
            format!("translation of block {id}")
        })?;
        let mut blocks = Vec::new();
        if let Some(resolved) = translation.text()
            && !resolved.runs.is_empty()
        {
            blocks.extend(blocks::type_layer(text, resolved, self.texts));
            self.texts += 1;
        }
        let name = text.content.text();
        self.leaf(
            Some(translation),
            Leaf {
                name: &name,
                visible: true,
                opacity: 255,
                blend: Blend::Normal,
            },
            blocks,
            None,
        )
    }

    /// A pixel layer: what `drawn` is, or nothing for what no frame draws.
    fn leaf(
        &mut self,
        drawn: Option<Drawn<'_>>,
        leaf: Leaf,
        blocks: Vec<TaggedBlock>,
        mask: Option<MaskSpec>,
    ) -> Result<()> {
        let Raster { image, offset } = match drawn {
            // Under a stroke effect the pixels are the glyphs alone, as
            // Photoshop draws the effect around them.
            Some(drawn) if blocks.iter().any(|block| &block.key == b"lfx2") => {
                self.renderer.rasterize(&drawn.unstroked())?
            }
            Some(drawn) => self.renderer.rasterize(&drawn.cropped())?,
            None => Raster {
                image: Default::default(),
                offset: Offset::default(),
            },
        };
        let (width, height) = image.dimensions();
        self.builder.push_layer(LayerSpec {
            name: leaf.name.to_owned(),
            left: offset.x,
            top: offset.y,
            width,
            height,
            pixels: PixelData::Rgba8(image.into_raw()),
            // Pass-through only means something for a group; a single layer
            // composites the same in normal.
            blend_mode: match leaf.blend {
                Blend::PassThrough => BlendMode::Normal,
                blend => blend_mode(blend),
            },
            opacity: leaf.opacity,
            fill_opacity: None,
            visible: leaf.visible,
            clipping: false,
            mask,
            extra_blocks: blocks,
        });
        Ok(())
    }

    /// A layer's shown masks merged into its one user mask, as PSD has one
    /// layer mask a layer.
    fn masks(&self, layer: &Layer) -> Result<Option<MaskSpec>> {
        match layer
            .children
            .iter()
            .any(|child| child.is_mask() && child.visible)
        {
            true => self.user_mask(drawn(self.frame.layer(layer.id), || {
                format!("layer {}", layer.id)
            })?),
            false => Ok(None),
        }
    }

    /// `drawn`'s masks merged into one user mask, cropped to what it covers.
    fn user_mask(&self, drawn: Drawn<'_>) -> Result<Option<MaskSpec>> {
        let Some(mask) = drawn.mask() else {
            return Ok(None);
        };
        let coverage = self.renderer.rasterize(&mask)?;
        let image = &coverage.image;
        let (mut x0, mut y0, mut x1, mut y1) = (image.width(), image.height(), 0, 0);
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel[3] > 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        let (x0, y0) = (x0.min(x1), y0.min(y1));
        let data = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| image.get_pixel(x, y)[3]))
            .collect();
        Ok(Some(MaskSpec {
            rect: photocraft_psd::Rect::from_xywh(
                coverage.offset.x + x0 as i32,
                coverage.offset.y + y0 as i32,
                x1 - x0,
                y1 - y0,
            ),
            data,
            default_color: 0,
            disabled: false,
        }))
    }
}

/// Gives a folder record the user mask the builder gives layers.
fn add_mask(record: &mut LayerRecord, mask: MaskSpec) -> Result<()> {
    let (width, height) = (mask.rect.width() as usize, mask.rect.height() as usize);
    record.channels.push(ChannelData::encode(
        -2,
        Compression::Rle,
        &mask.data,
        width,
        height,
        8,
        Version::Psd,
    )?);
    record.mask = MaskData::Mask(LayerMask::new(mask.rect, mask.default_color, 0));
    Ok(())
}

fn parse(path: &Path) -> Result<BezPath> {
    BezPath::from_svg(path.as_str()).map_err(|error| Error::Path(error.to_string()))
}

fn blend_mode(blend: Blend) -> BlendMode {
    match blend {
        Blend::PassThrough => BlendMode::PassThrough,
        Blend::Normal => BlendMode::Normal,
        Blend::Multiply => BlendMode::Multiply,
        Blend::Screen => BlendMode::Screen,
        Blend::Overlay => BlendMode::Overlay,
        Blend::Darken => BlendMode::Darken,
        Blend::Lighten => BlendMode::Lighten,
        Blend::ColorDodge => BlendMode::ColorDodge,
        Blend::ColorBurn => BlendMode::ColorBurn,
        Blend::HardLight => BlendMode::HardLight,
        Blend::SoftLight => BlendMode::SoftLight,
        Blend::Difference => BlendMode::Difference,
        Blend::Exclusion => BlendMode::Exclusion,
        Blend::Hue => BlendMode::Hue,
        Blend::Saturation => BlendMode::Saturation,
        Blend::Color => BlendMode::Color,
        Blend::Luminosity => BlendMode::Luminosity,
    }
}
