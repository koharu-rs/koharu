//! A rendered page: the tree of what each layer drew, painted onto the
//! canvas's drawing or rasterized for an export, both on the GPU.

use std::{fmt, sync::Arc};

use anyhow::Result;
use image::RgbaImage;
use koharu_scene::{Blend, LayerId, MaskMode, Offset, PageId, Revision};
use kurbo::{Affine, Rect, Shape as _};

use crate::{Drawing, Gpu, pixels::check_size, text::ResolvedText};

mod paint;
mod tree;

pub(crate) use self::tree::{Content, MaskNode, Node, ShapeVector};

/// A page at one revision, immutable and cheap to clone.
#[derive(Clone)]
pub struct Frame(pub(crate) Arc<FrameData>);

pub(crate) struct FrameData {
    pub page: PageId,
    pub revision: Revision,
    /// What the frame draws, in page pixels: the page, or one layer's bounds.
    pub area: Rect,
    pub nodes: Vec<Node>,
}

/// A raster: RGBA pixels and where their corner sits in page pixels.
#[derive(Debug)]
pub struct Raster {
    pub image: RgbaImage,
    pub offset: Offset,
}

/// One thing a frame drew: a layer, a block's clean-up among them, the
/// page's background, or a block's translation, which a block draws apart as
/// PSD export takes it apart.
#[derive(Clone, Copy)]
pub struct Drawn<'a> {
    frame: &'a Frame,
    node: &'a Node,
    /// It and everything above it show.
    shown: bool,
}

impl<'a> Drawn<'a> {
    /// The layer it draws; `None` for the page's background and a block's
    /// translation.
    #[must_use]
    pub fn layer(&self) -> Option<LayerId> {
        self.node.layer
    }

    /// Whether it and everything above it show.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.shown
    }

    /// What it draws, in page pixels; `None` when it draws nothing.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.node.bounds
    }

    /// For a text layer or a translation, the laid-out runs.
    #[must_use]
    pub fn text(&self) -> Option<&'a ResolvedText> {
        self.node.text()
    }

    /// Only its own content, shown, over its bounds rounded out to whole
    /// pixels: without its opacity, blend mode, or masks, which a PSD records
    /// as layer properties.
    #[must_use]
    pub fn cropped(&self) -> Frame {
        self.own(true)
    }

    /// `cropped`, but a text without its strokes, which a PSD writes as a
    /// stroke effect.
    #[must_use]
    pub fn unstroked(&self) -> Frame {
        self.own(false)
    }

    fn own(&self, strokes: bool) -> Frame {
        let node = self.node;
        self.frame.with(
            node.bounds.map_or(Rect::ZERO, Rect::expand),
            Node {
                visible: true,
                opacity: 1.0,
                blend: Blend::Normal,
                strokes,
                masks: Vec::new(),
                ..node.clone()
            },
        )
    }

    /// Its masks merged into one coverage, as white with the coverage in
    /// alpha; `None` when it has no masks. When the first shown mask adds,
    /// the coverage starts empty, so the frame covers only what the adding
    /// masks do; otherwise it covers the page.
    #[must_use]
    pub fn mask(&self) -> Option<Frame> {
        let node = self.node;
        if node.masks.is_empty() {
            return None;
        }
        let shown: Vec<_> = node.masks.iter().filter(|mask| mask.visible).collect();
        let page = self.frame.0.area;
        let area = match shown.first() {
            Some(first) if first.mode == MaskMode::Add => shown
                .iter()
                .filter(|mask| mask.mode == MaskMode::Add)
                .map(|mask| mask.bounds)
                .filter(|bounds| bounds.area() > 0.0)
                .reduce(|a, b| a.union(b))
                .map_or(Rect::ZERO, |bounds| bounds.intersect(page).expand()),
            _ => page,
        };
        Some(self.frame.with(
            area,
            Node {
                masks: node.masks.clone(),
                bounds: Some(area),
                ..Node::new(
                    node.layer,
                    true,
                    Some((
                        Content::Coverage(Arc::new(area.to_path(0.0))),
                        Affine::IDENTITY,
                    )),
                )
            },
        ))
    }
}

impl fmt::Debug for Drawn<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Drawn")
            .field("layer", &self.node.layer)
            .field("visible", &self.shown)
            .field("bounds", &self.node.bounds)
            .finish_non_exhaustive()
    }
}

impl Frame {
    #[must_use]
    pub fn page(&self) -> PageId {
        self.0.page
    }

    #[must_use]
    pub fn revision(&self) -> &Revision {
        &self.0.revision
    }

    /// What the frame draws, in page pixels.
    #[must_use]
    pub fn area(&self) -> Rect {
        self.0.area
    }

    /// Everything drawn, in the page's order, parents first.
    pub fn layers(&self) -> impl Iterator<Item = Drawn<'_>> {
        fn walk<'a>(frame: &'a Frame, nodes: &'a [Node], shown: bool, out: &mut Vec<Drawn<'a>>) {
            for node in nodes {
                let shown = shown && node.visible;
                out.push(Drawn { frame, node, shown });
                walk(frame, &node.children, shown, out);
            }
        }
        let mut drawn = Vec::new();
        walk(self, &self.0.nodes, true, &mut drawn);
        drawn.into_iter()
    }

    /// What `layer` drew; a block's clean-up is found by its id too.
    #[must_use]
    pub fn layer(&self, layer: LayerId) -> Option<Drawn<'_>> {
        self.layers().find(|drawn| drawn.node.layer == Some(layer))
    }

    /// The page's background, its first node, the only one at the top
    /// drawing no layer.
    #[must_use]
    pub fn background(&self) -> Option<Drawn<'_>> {
        let node = self.0.nodes.first().filter(|node| node.layer.is_none())?;
        Some(Drawn {
            frame: self,
            node,
            shown: node.visible,
        })
    }

    /// Block `block`'s translation, its one child drawing no layer.
    #[must_use]
    pub fn translation(&self, block: LayerId) -> Option<Drawn<'_>> {
        let block = self.layer(block)?;
        let node = block
            .node
            .children
            .iter()
            .find(|node| node.layer.is_none())?;
        Some(Drawn {
            frame: self,
            node,
            shown: block.shown && node.visible,
        })
    }

    /// Paints the frame through `to_output`, from page pixels to output
    /// pixels, within `clip`, a rectangle of output pixels.
    pub fn paint(&self, to_output: Affine, clip: Rect, drawing: &mut Drawing) -> Result<()> {
        paint::paint(&self.0.nodes, to_output, clip, drawing)
    }

    /// The same frame with `layers` left out. A block's id leaves out only
    /// its translation, the part the canvas drags in a move, since its
    /// clean-up and mask stay with the scan unless the clean-up's own id is
    /// listed too; a block with none, whose moving part is its undrawn
    /// source, isn't passed. Ids it doesn't draw are ignored.
    #[must_use]
    pub fn without(&self, layers: &[LayerId]) -> Frame {
        fn keep(nodes: &[Node], layers: &[LayerId]) -> Vec<Node> {
            nodes
                .iter()
                .filter_map(|node| {
                    let listed = node.layer.is_some_and(|layer| layers.contains(&layer));
                    let translated = node.children.iter().any(|child| child.layer.is_none());
                    let children = match (listed, translated) {
                        (true, false) => return None,
                        (true, true) => keep(&node.children, layers)
                            .into_iter()
                            .filter(|child| child.layer.is_some())
                            .collect(),
                        (false, _) => keep(&node.children, layers),
                    };
                    Some(Node {
                        children,
                        ..node.clone()
                    })
                })
                .collect()
        }
        Self(Arc::new(FrameData {
            page: self.0.page,
            revision: self.0.revision.clone(),
            area: self.0.area,
            nodes: keep(&self.0.nodes, layers),
        }))
    }

    /// A frame of this page drawing only `node` over `area`.
    fn with(&self, area: Rect, node: Node) -> Self {
        Self(Arc::new(FrameData {
            page: self.0.page,
            revision: self.0.revision.clone(),
            area,
            nodes: vec![node],
        }))
    }

    /// The frame's area drawn at one output pixel per page pixel.
    pub fn rasterize(&self) -> Result<Raster> {
        let area = self.area();
        Ok(Raster {
            image: self.rasterize_rect(area)?,
            offset: Offset {
                x: area.x0.floor() as i32,
                y: area.y0.floor() as i32,
            },
        })
    }

    /// `rect`, in page pixels, drawn at one output pixel per page pixel, its
    /// corner rounded down to a whole pixel: in pieces when it's larger than
    /// the GPU's largest texture.
    pub fn rasterize_rect(&self, rect: Rect) -> Result<RgbaImage> {
        let (width, height) = (rect.width().ceil() as u32, rect.height().ceil() as u32);
        // What draws nothing, such as a block with no clean-up, is empty.
        if width == 0 || height == 0 {
            return Ok(RgbaImage::new(width, height));
        }
        check_size(width, height)?;
        let gpu = Gpu::get()?;
        let side = gpu.largest_texture();
        let (x0, y0) = (rect.x0.floor(), rect.y0.floor());
        let piece = |left: u32, top: u32| {
            let (width, height) = (side.min(width - left), side.min(height - top));
            let mut drawing = gpu.drawing(width as u16, height as u16);
            self.paint(
                Affine::translate((-(x0 + f64::from(left)), -(y0 + f64::from(top)))),
                Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                &mut drawing,
            )?;
            drawing.read()
        };
        if width <= side && height <= side {
            return piece(0, 0);
        }
        let mut image = RgbaImage::new(width, height);
        for top in (0..height).step_by(side as usize) {
            for left in (0..width).step_by(side as usize) {
                image::imageops::replace(&mut image, &piece(left, top)?, left.into(), top.into());
            }
        }
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Edit, Op, Rect as PixelRect, Ref, Session};
    use serde_json::json;

    use super::*;
    use crate::{Renderer, RendererConfig};

    /// A view puts each page pixel at its output pixels, zoomed in on part of
    /// the page or out to all of it, where a raster draws from a halved
    /// level.
    #[test]
    fn a_view_maps_page_pixels_to_its_output() {
        // A white raster under a red square from (256, 256) to (512, 512).
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
        let ops = vec![
            json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 1024, "height": 1024}}),
            json!({"op": "add_layer", "id": "$paper", "page": "$page", "index": 0, "layer": {"kind": "raster"}}),
            json!({"op": "add_layer", "page": "$page", "index": 1, "layer": {
                "kind": "shape", "path": "M256 256 H512 V512 H256 Z", "fill": [1.0, 0.0, 0.0, 1.0],
            }}),
        ];
        let mut ops: Vec<Op> = ops
            .into_iter()
            .map(|op| serde_json::from_value(op).expect("an op"))
            .collect();
        ops.push(Op::WritePixels {
            layer: Ref::try_from("$paper".to_owned()).expect("a label"),
            rect: PixelRect {
                x: 0,
                y: 0,
                width: 1024,
                height: 1024,
            },
            pixels: vec![255; 1024 * 1024 * 4],
        });
        let change = session
            .apply(Edit {
                label: "test".into(),
                ops,
            })
            .expect("an edit");
        let renderer = Renderer::new(RendererConfig::default());
        let Ok(gpu) = Gpu::get() else {
            eprintln!("skipped without a GPU");
            return;
        };
        let frame = renderer
            .render(&session, change.pages.added[0])
            .expect("a frame");
        // `rect` of the page at `scale` output pixels per page pixel.
        let pixels = |rect: Rect, scale: f64| {
            let size = rect.size() * scale;
            let mut drawing = gpu.drawing(size.width.ceil() as u16, size.height.ceil() as u16);
            frame
                .paint(
                    Affine::scale(scale) * Affine::translate((-rect.x0, -rect.y0)),
                    size.to_rect(),
                    &mut drawing,
                )
                .expect("drawn");
            drawing.read().expect("pixels")
        };
        const RED: [u8; 4] = [255, 0, 0, 255];
        const WHITE: [u8; 4] = [255; 4];

        // (240, 240) to (272, 272) at twice its size: white, then red.
        let zoomed = pixels(Rect::new(240.0, 240.0, 272.0, 272.0), 2.0);
        assert_eq!(zoomed.dimensions(), (64, 64));
        assert_eq!(
            (zoomed.get_pixel(16, 16).0, zoomed.get_pixel(48, 48).0),
            (WHITE, RED)
        );

        // The page at a quarter of its size.
        let fitted = pixels(Rect::new(0.0, 0.0, 1024.0, 1024.0), 0.25);
        assert_eq!(fitted.dimensions(), (256, 256));
        let at = |x, y| fitted.get_pixel(x, y).0;
        assert_eq!((at(32, 32), at(96, 96), at(200, 200)), (WHITE, RED, WHITE));
    }

    /// The canvas draws a frame without the layers a move is dragging, so
    /// only their copy under the pointer shows.
    #[test]
    fn a_frame_without_a_layer_draws_what_is_under_it() {
        // A white raster under a red square from (16, 16) to (48, 48).
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
        let ops = vec![
            json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 64, "height": 64}}),
            json!({"op": "add_layer", "id": "$paper", "page": "$page", "index": 0, "layer": {"kind": "raster"}}),
            json!({"op": "add_layer", "page": "$page", "index": 1, "layer": {
                "kind": "shape", "path": "M16 16 H48 V48 H16 Z", "fill": [1.0, 0.0, 0.0, 1.0],
            }}),
        ];
        let mut ops: Vec<Op> = ops
            .into_iter()
            .map(|op| serde_json::from_value(op).expect("an op"))
            .collect();
        ops.push(Op::WritePixels {
            layer: Ref::try_from("$paper".to_owned()).expect("a label"),
            rect: PixelRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            pixels: vec![255; 64 * 64 * 4],
        });
        let change = session
            .apply(Edit {
                label: "test".into(),
                ops,
            })
            .expect("an edit");
        let page = change.pages.added[0];
        let square = session.project().page(page).expect("the page").layers[1].id;
        let renderer = Renderer::new(RendererConfig::default());
        if Gpu::get().is_err() {
            eprintln!("skipped without a GPU");
            return;
        }
        let frame = renderer.render(&session, page).expect("a frame");
        let at = |frame: &Frame| {
            let raster = frame.rasterize().expect("pixels");
            raster.image.get_pixel(32, 32).0
        };
        assert_eq!(at(&frame), [255, 0, 0, 255]);
        assert_eq!(at(&frame.without(&[square])), [255; 4]);
        let unknown = LayerId(koharu_scene::Uuid::now_v7());
        assert_eq!(at(&frame.without(&[unknown])), [255, 0, 0, 255]);
    }
}
