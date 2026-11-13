//! A rendered page: the tree of what each layer drew, drawn for a view of
//! the canvas or for an export.

use std::{fmt, sync::Arc};

use koharu_scene::{Blend, LayerId, MaskMode, PageId, Revision};
use vello::{
    Scene,
    kurbo::{Rect, Shape as _},
};

use crate::{fonts::FontRequest, text::ResolvedText};

mod compose;
mod tree;

pub(crate) use tree::{Content, Node, Retained, Source, Tree, Vector, parse};

/// A page at one revision, immutable and cheap to clone.
#[derive(Clone)]
pub struct Frame(pub(crate) Arc<FrameData>);

pub(crate) struct FrameData {
    pub page: PageId,
    pub revision: Revision,
    /// What the frame draws, in page pixels: the page, or one layer's bounds.
    pub area: Rect,
    pub nodes: Vec<Node>,
    /// The first node is the page's background: never in a frame of one
    /// thing alone.
    pub background: bool,
    pub missing: Arc<[FontRequest]>,
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
        self.node.text.as_deref()
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
        let content = match &node.content {
            Content::Vector {
                vector, transform, ..
            } => Content::Vector {
                vector: vector.clone(),
                strokes,
                transform: *transform,
            },
            content => content.clone(),
        };
        self.frame.with(
            node.bounds.map_or(Rect::ZERO, Rect::expand),
            Node {
                visible: true,
                opacity: 1.0,
                blend: Blend::Normal,
                content,
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
                ..Node::plain(
                    node.layer,
                    true,
                    Content::Coverage(Arc::new(area.to_path(0.0))),
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

/// A view of the page: the page-pixel rectangle it shows, at `scale` output
/// pixels per page pixel.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub rect: Rect,
    pub scale: f64,
}

/// One render pass: a scene for the output pixels `rect`, its origin at
/// that rectangle's corner.
pub struct Strip {
    pub rect: Rect,
    pub scene: Scene,
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
        fn find<'a>(
            frame: &'a Frame,
            nodes: &'a [Node],
            shown: bool,
            layer: LayerId,
        ) -> Option<Drawn<'a>> {
            nodes.iter().find_map(|node| {
                let shown = shown && node.visible;
                match node.layer == Some(layer) {
                    true => Some(Drawn { frame, node, shown }),
                    false => find(frame, &node.children, shown, layer),
                }
            })
        }
        find(self, &self.0.nodes, true, layer)
    }

    /// The page's background, its first node.
    #[must_use]
    pub fn background(&self) -> Option<Drawn<'_>> {
        let node = self.0.nodes.first().filter(|_| self.0.background)?;
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

    /// Whether some text waits for a font `Renderer::load_fonts` would
    /// download; until then it draws with a fallback.
    #[must_use]
    pub fn needs_fonts(&self) -> bool {
        !self.0.missing.is_empty()
    }

    /// The passes that draw `view`: one, unless its images need more of
    /// vello's atlas than one pass has, when they are strips across it.
    #[must_use]
    pub fn draw(&self, view: View) -> Vec<Strip> {
        compose::strips(&self.0.nodes, view)
    }

    /// A frame of this page drawing only `node` over `area`.
    fn with(&self, area: Rect, node: Node) -> Self {
        Self(Arc::new(FrameData {
            page: self.0.page,
            revision: self.0.revision.clone(),
            area,
            nodes: vec![node],
            background: false,
            missing: self.0.missing.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Edit, Op, Rect as PixelRect, Ref, Session};
    use serde_json::json;

    use super::*;
    use crate::{Renderer, RendererConfig};

    /// A view puts each page pixel at its output pixels, zoomed in on part of
    /// the page or out to all of it, where a raster draws from halved tiles.
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
        let Ok(gpu) = renderer.gpu() else {
            eprintln!("skipped without a GPU");
            return;
        };
        let frame = renderer
            .render(&session, change.pages.added[0])
            .expect("a frame");
        let pixels = |view: View| {
            let strips = frame.draw(view);
            assert_eq!(strips.len(), 1);
            let (width, height) = (
                strips[0].rect.width() as u32,
                strips[0].rect.height() as u32,
            );
            gpu.read(&strips[0].scene, width, height).expect("pixels")
        };
        const RED: [u8; 4] = [255, 0, 0, 255];
        const WHITE: [u8; 4] = [255; 4];

        // (240, 240) to (272, 272) at twice its size: white, then red.
        let zoomed = pixels(View {
            rect: Rect::new(240.0, 240.0, 272.0, 272.0),
            scale: 2.0,
        });
        assert_eq!(zoomed.dimensions(), (64, 64));
        assert_eq!(
            (zoomed.get_pixel(16, 16).0, zoomed.get_pixel(48, 48).0),
            (WHITE, RED)
        );

        // The page at a quarter of its size.
        let fitted = pixels(View {
            rect: Rect::new(0.0, 0.0, 1024.0, 1024.0),
            scale: 0.25,
        });
        assert_eq!(fitted.dimensions(), (256, 256));
        let at = |x, y| fitted.get_pixel(x, y).0;
        assert_eq!((at(32, 32), at(96, 96), at(200, 200)), (WHITE, RED, WHITE));
    }

    /// A large page of many small layers that composite apart, as a
    /// translated page's blocks and panels do, still draws: vello 0.11's
    /// buffers hold only so many tiles, and a layer clipped to the whole page
    /// took them all.
    #[test]
    fn many_small_layers_on_a_large_page_draw() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut session = Session::create(dir.path().join("test.khr")).expect("a project");
        let mut ops = vec![
            json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 2400, "height": 3200}}),
        ];
        for index in 0..150 {
            let (x, y) = (100 + (index % 10) * 200, 100 + (index / 10) * 200);
            ops.push(
                json!({"op": "add_layer", "page": "$page", "index": index, "layer": {
                    "kind": "shape", "opacity": 0.5,
                    "path": format!("M{x} {y} h20 v20 h-20 Z"), "fill": [1.0, 0.0, 0.0, 1.0],
                }}),
            );
        }
        let change = session
            .apply(Edit {
                label: "test".into(),
                ops: ops
                    .into_iter()
                    .map(|op| serde_json::from_value(op).expect("an op"))
                    .collect(),
            })
            .expect("an edit");
        let renderer = Renderer::new(RendererConfig::default());
        if renderer.gpu().is_err() {
            eprintln!("skipped without a GPU");
            return;
        }
        let frame = renderer
            .render(&session, change.pages.added[0])
            .expect("a frame");
        let raster = renderer.rasterize(&frame).expect("pixels");
        // The last square, red at half opacity.
        let [red, green, blue, alpha] = raster.image.get_pixel(1910, 2910).0;
        assert_eq!([red, green, blue], [255, 0, 0]);
        assert!((127..=128).contains(&alpha), "{alpha}");
    }
}
