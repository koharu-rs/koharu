//! Writes a Koharu page as a layered PSD with photocraft-psd. The README maps
//! each scene element to what the PSD holds, with sources.

mod blocks;
mod engine_data;

use koharu_renderer::{Frame, Raster, Renderer};
use koharu_scene::{Blend, FillRule, Layer, LayerView, MaskContent, PageId, Path, Session};
use kurbo::BezPath;
use photocraft_psd::{
    BlendMode, ChannelData, Compression, GroupSpec, LayerMask, LayerRecord, LayerSpec, MaskData,
    MaskSpec, PixelData, PsdBuilder, PsdError, SectionType, TaggedBlock, Version,
};

use crate::blocks::Component;

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
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// `page` as a PSD: the renderer's composite as the merged image, and each
/// layer with its own pixels, properties, and masks; shapes and text also
/// keep what lets Photoshop edit them. The page is rendered once, so call it
/// off the UI thread.
pub fn export_page(session: &Session, renderer: &Renderer, page: PageId) -> Result<Vec<u8>> {
    let view = session.page(page)?;
    let (width, height) = (view.size.width, view.size.height);
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(Error::TooLarge { width, height });
    }
    let frame = renderer.render(session, page)?;
    let mut export = Export {
        renderer,
        frame: &frame,
        size: (width, height),
        builder: PsdBuilder::new(width, height).resolution(72.0),
        folders: Vec::new(),
        texts: 0,
    };
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
    size: (u32, u32),
    builder: PsdBuilder,
    /// Each group's blocks and mask, which the builder can't give its folder
    /// record, in the order its groups end.
    folders: Vec<(Vec<TaggedBlock>, Option<MaskSpec>)>,
    texts: i32,
}

impl Export<'_> {
    fn layer(&mut self, view: &LayerView) -> Result<()> {
        let (name, visible, locked, opacity, blend) = match &view.layer {
            Layer::Group(layer) => (
                &layer.name,
                layer.visible,
                layer.locked,
                layer.opacity,
                layer.blend,
            ),
            Layer::Image(layer) => (
                &layer.name,
                layer.visible,
                layer.locked,
                layer.opacity,
                layer.blend,
            ),
            Layer::Raster(layer) => (
                &layer.name,
                layer.visible,
                layer.locked,
                layer.opacity,
                layer.blend,
            ),
            Layer::Shape(layer) => (
                &layer.name,
                layer.visible,
                layer.locked,
                layer.opacity,
                layer.blend,
            ),
            Layer::Text(layer) => (
                &layer.name,
                layer.visible,
                layer.locked,
                layer.opacity,
                layer.blend,
            ),
            // A mask is its layer's, never one of the drawn children.
            Layer::Mask(_) => return Ok(()),
        };
        let opacity = (opacity * 255.0).round() as u8;
        let mut blocks = Vec::new();
        if locked {
            blocks.push(TaggedBlock::protection(LOCK_ALL));
        }
        match &view.layer {
            Layer::Group(_) => {
                self.builder.begin_group(GroupSpec {
                    name: name.clone(),
                    blend_mode: blend_mode(blend),
                    opacity,
                    visible,
                    open: true,
                });
                for child in &view.children {
                    self.layer(child)?;
                }
                self.builder.end_group()?;
                let (mask, vector) = self.masks(view, false)?;
                blocks.extend(vector);
                self.folders.push((blocks, mask));
                return Ok(());
            }
            Layer::Shape(shape) => blocks.extend(blocks::shape(shape, self.size)?),
            // A text that draws nothing has no runs to style, so it stays an
            // empty pixel layer.
            Layer::Text(text) => {
                if let Some(drawn) = self.frame.layer(view.id)
                    && let Some(resolved) = drawn.text()
                    && !resolved.runs.is_empty()
                {
                    blocks.extend(blocks::type_layer(text, resolved, self.texts));
                    self.texts += 1;
                }
            }
            _ => {}
        }
        let has_vector_mask = blocks.iter().any(|block| &block.key == b"vmsk");
        let (mask, vector) = self.masks(view, has_vector_mask)?;
        blocks.extend(vector);
        // Under a stroke effect the pixels are the glyphs alone, as Photoshop
        // draws the effect around them.
        let own = if blocks.iter().any(|block| &block.key == b"lfx2") {
            self.frame.unstroked(view.id)?
        } else {
            self.frame.cropped(view.id)?
        };
        let Raster { image, left, top } = self.renderer.rasterize(&own)?;
        let (width, height) = image.dimensions();
        self.builder.push_layer(LayerSpec {
            name: name.clone(),
            left,
            top,
            width,
            height,
            pixels: PixelData::Rgba8(image.into_raw()),
            // Pass-through only means something for a group; a single layer
            // composites the same in normal.
            blend_mode: match blend {
                Blend::PassThrough => BlendMode::Normal,
                blend => blend_mode(blend),
            },
            opacity,
            fill_opacity: None,
            visible,
            clipping: false,
            mask,
            extra_blocks: blocks,
        });
        Ok(())
    }

    /// A layer's shown masks: as one vector mask when every one is a path
    /// and the layer has no vector mask of its own, else merged into one user
    /// mask the renderer draws, cropped to what it covers.
    fn masks(
        &self,
        view: &LayerView,
        has_vector_mask: bool,
    ) -> Result<(Option<MaskSpec>, Option<TaggedBlock>)> {
        let shown: Vec<_> = view
            .masks
            .iter()
            .filter_map(|mask| match &mask.layer {
                Layer::Mask(mask) if mask.visible => Some(mask),
                _ => None,
            })
            .collect();
        if shown.is_empty() {
            return Ok((None, None));
        }
        if !has_vector_mask {
            let components = shown
                .iter()
                .map(|mask| match &mask.content {
                    // An empty path has no subpath to carry its operation.
                    MaskContent::Path { path } => Ok(Some(parse(path)?)
                        .filter(|path| !path.elements().is_empty())
                        .map(|path| Component {
                            path,
                            mode: mask.mode,
                            // The renderer fills path masks by the non-zero
                            // rule.
                            fill_rule: FillRule::NonZero,
                        })),
                    MaskContent::Pixels { .. } => Ok(None),
                })
                .collect::<Result<Option<Vec<_>>>>()?;
            if let Some(components) = components {
                return Ok((None, Some(blocks::vector_mask(&components, self.size))));
            }
        }
        let Some(mask) = self.frame.mask(view.id)? else {
            return Ok((None, None));
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
        Ok((
            Some(MaskSpec {
                rect: photocraft_psd::Rect::from_xywh(
                    coverage.left + x0 as i32,
                    coverage.top + y0 as i32,
                    x1 - x0,
                    y1 - y0,
                ),
                data,
                default_color: 0,
                disabled: false,
            }),
            None,
        ))
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
