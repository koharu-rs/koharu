//! A rendered page: each layer's retained content in the page's tree, drawn
//! for a view of the canvas or for an export.

use std::{collections::HashMap, sync::Arc};

use koharu_scene::{Blend, LayerId, LayerKind, MaskMode, PageId, Revision};
use vello::{
    Scene,
    kurbo::{Affine, BezPath, Rect},
    peniko::{BlendMode, Color, Compose, Fill, Mix},
};

use crate::{Error, Result, fonts::FontRequest, text::ResolvedText, tiles::Tiled};

/// The image pixels one pass may draw: vello 0.11 packs every image of a
/// frame into one atlas of at most 8192×8192, and skips what doesn't fit.
const ATLAS: u64 = 8192 * 8192 * 7 / 8;

/// A page at one revision, immutable and cheap to clone.
#[derive(Clone)]
pub struct Frame(pub(crate) Arc<FrameData>);

pub(crate) struct FrameData {
    pub page: PageId,
    pub revision: Revision,
    /// What the frame draws, in page pixels: the page, or one layer's bounds.
    pub area: Rect,
    pub nodes: Vec<Node>,
    pub layers: HashMap<LayerId, Layer>,
    /// Layer ids in the page's order, parents first.
    pub order: Vec<LayerId>,
    pub missing: Vec<FontRequest>,
}

/// One layer as the frame drew it.
#[derive(Clone)]
pub struct Layer {
    pub(crate) id: LayerId,
    pub(crate) kind: LayerKind,
    pub(crate) visible: bool,
    pub(crate) bounds: Option<Rect>,
    pub(crate) text: Option<Arc<ResolvedText>>,
}

impl Layer {
    #[must_use]
    pub fn id(&self) -> LayerId {
        self.id
    }

    #[must_use]
    pub fn kind(&self) -> LayerKind {
        self.kind
    }

    /// Whether it and everything above it show.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// What it draws, in page pixels; `None` when it draws nothing.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// For a text layer, the laid-out runs.
    #[must_use]
    pub fn text(&self) -> Option<&ResolvedText> {
        self.text.as_deref()
    }
}

#[derive(Clone)]
pub(crate) enum Content {
    None,
    Vector {
        scene: Arc<Scene>,
        transform: Affine,
    },
    Pixels {
        tiled: Arc<Tiled>,
        transform: Affine,
    },
    /// A white rectangle, which a mask frame limits.
    Coverage(Rect),
}

#[derive(Clone)]
pub(crate) struct Node {
    pub id: LayerId,
    pub visible: bool,
    pub opacity: f32,
    pub blend: Blend,
    pub content: Content,
    pub children: Vec<Node>,
    pub masks: Vec<MaskNode>,
}

#[derive(Clone)]
pub(crate) struct MaskNode {
    pub visible: bool,
    pub mode: MaskMode,
    pub shape: MaskShape,
}

#[derive(Clone)]
pub(crate) enum MaskShape {
    Path(Arc<BezPath>),
    Pixels {
        tiled: Arc<Tiled>,
        transform: Affine,
    },
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

    #[must_use]
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.0.layers.get(&id)
    }

    /// Every layer in the page's order, parents first.
    pub fn layers(&self) -> impl Iterator<Item = &Layer> {
        self.0.order.iter().filter_map(|id| self.0.layers.get(id))
    }

    /// Whether some text waits for a font `Renderer::load_fonts` would
    /// download; until then it draws with a fallback.
    #[must_use]
    pub fn needs_fonts(&self) -> bool {
        !self.0.missing.is_empty()
    }

    /// Only `layer`'s own content, shown, over its bounds rounded out to
    /// whole pixels: without its opacity, blend mode, or masks, which a PSD
    /// records as layer properties.
    pub fn cropped(&self, layer: LayerId) -> Result<Self> {
        let node = self.node(layer)?;
        Ok(self.with(
            self.0.layers[&layer]
                .bounds
                .map_or(Rect::ZERO, Rect::expand),
            Node {
                visible: true,
                opacity: 1.0,
                blend: Blend::Normal,
                masks: Vec::new(),
                ..node.clone()
            },
        ))
    }

    /// `layer`'s masks merged into one coverage over the page, as white with
    /// the coverage in alpha; `None` when it has no masks.
    pub fn mask(&self, layer: LayerId) -> Result<Option<Self>> {
        let node = self.node(layer)?;
        if node.masks.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.with(
            self.0.area,
            Node {
                id: layer,
                visible: true,
                opacity: 1.0,
                blend: Blend::Normal,
                content: Content::Coverage(self.0.area),
                children: Vec::new(),
                masks: node.masks.clone(),
            },
        )))
    }

    fn node(&self, id: LayerId) -> Result<&Node> {
        fn find(nodes: &[Node], id: LayerId) -> Option<&Node> {
            nodes.iter().find_map(|node| {
                (node.id == id)
                    .then_some(node)
                    .or_else(|| find(&node.children, id))
            })
        }
        find(&self.0.nodes, id)
            .ok_or_else(|| Error::invalid(format!("layer {id} isn't drawn in this frame")))
    }

    /// A frame of this page drawing only `node` over `area`.
    fn with(&self, area: Rect, node: Node) -> Self {
        Self(Arc::new(FrameData {
            page: self.0.page,
            revision: self.0.revision.clone(),
            area,
            nodes: vec![node],
            layers: self.0.layers.clone(),
            order: self.0.order.clone(),
            missing: self.0.missing.clone(),
        }))
    }

    /// The passes that draw `view`: one, unless its images need more of
    /// vello's atlas than one pass has, when they are strips across it.
    #[must_use]
    pub fn draw(&self, view: View) -> Vec<Strip> {
        let output = Rect::new(
            0.0,
            0.0,
            (view.rect.width() * view.scale).ceil(),
            (view.rect.height() * view.scale).ceil(),
        );
        let mut strips = Vec::new();
        self.strips(view, output, &mut strips);
        strips
    }

    fn strips(&self, view: View, rect: Rect, out: &mut Vec<Strip>) {
        let to_output = Affine::translate((-rect.x0, -rect.y0))
            * Affine::scale(view.scale)
            * Affine::translate((-view.rect.x0, -view.rect.y0));
        let clip = Rect::new(0.0, 0.0, rect.width(), rect.height());
        let mut pass = Pass {
            to_output,
            clip,
            pixels: 0,
        };
        let cost = self.0.nodes.iter().map(|node| pass.cost(node)).sum::<u64>();
        if cost > ATLAS && rect.height() > 64.0 {
            let middle = (rect.y0 + (rect.height() * 0.5).floor()).floor();
            self.strips(view, Rect::new(rect.x0, rect.y0, rect.x1, middle), out);
            self.strips(view, Rect::new(rect.x0, middle, rect.x1, rect.y1), out);
            return;
        }
        let scene = pass.compose(&self.0.nodes);
        out.push(Strip { rect, scene });
    }
}

struct Pass {
    /// From page pixels to this pass's output pixels.
    to_output: Affine,
    /// The pass's output rectangle.
    clip: Rect,
    pixels: u64,
}

impl Pass {
    fn cost(&self, node: &Node) -> u64 {
        if !node.visible {
            return 0;
        }
        let own = match &node.content {
            Content::Pixels { tiled, transform } => {
                tiled.cost(self.to_output * *transform, self.clip)
            }
            _ => 0,
        };
        let masks: u64 = node
            .masks
            .iter()
            .filter(|mask| mask.visible)
            .map(|mask| match &mask.shape {
                MaskShape::Pixels { tiled, transform } => {
                    tiled.cost(self.to_output * *transform, self.clip)
                }
                MaskShape::Path(_) => 0,
            })
            .sum();
        own + masks
            + node
                .children
                .iter()
                .map(|child| self.cost(child))
                .sum::<u64>()
    }

    /// Draws nodes into a fresh scene: an isolated group's content.
    fn compose(&mut self, nodes: &[Node]) -> Scene {
        let mut scene = Scene::new();
        for node in nodes {
            self.node(node, &mut scene);
        }
        scene
    }

    fn node(&mut self, node: &Node, scene: &mut Scene) {
        if !node.visible {
            return;
        }
        let masked = node.masks.iter().any(|mask| mask.visible);
        let group = matches!(node.content, Content::None);
        if group && node.blend == Blend::PassThrough {
            if !masked && node.opacity >= 1.0 {
                // Children blend straight into the backdrop.
                for child in &node.children {
                    self.node(child, scene);
                }
                return;
            }
            // The group's layer draws the backdrop again under its children,
            // as a non-isolated group includes its backdrop, so its opacity
            // and masks fade between the backdrop and the result.
            let mut content = Scene::new();
            content.append(scene, None);
            for child in &node.children {
                self.node(child, &mut content);
            }
            self.wrap(node, Mix::Normal, content, scene);
            return;
        }
        let content = if group {
            self.compose(&node.children)
        } else {
            let mut content = Scene::new();
            self.content(&node.content, &mut content);
            content
        };
        if !masked && node.opacity >= 1.0 && node.blend == Blend::Normal {
            scene.append(&content, None);
        } else {
            self.wrap(node, mix(node.blend), content, scene);
        }
    }

    fn content(&mut self, content: &Content, scene: &mut Scene) {
        match content {
            Content::None => {}
            Content::Vector {
                scene: vector,
                transform,
            } => scene.append(vector, Some(self.to_output * *transform)),
            Content::Pixels { tiled, transform } => {
                self.pixels += tiled.draw(scene, self.to_output * *transform, self.clip);
            }
            Content::Coverage(rect) => {
                scene.fill(Fill::NonZero, self.to_output, Color::WHITE, None, rect);
            }
        }
    }

    /// Composites `content` onto `scene` with the node's blend mode, opacity,
    /// and masks.
    fn wrap(&mut self, node: &Node, mix: Mix, content: Scene, scene: &mut Scene) {
        scene.push_layer(
            Fill::NonZero,
            BlendMode::new(mix, Compose::SrcOver),
            node.opacity,
            Affine::IDENTITY,
            &self.clip,
        );
        scene.append(&content, None);
        let masks: Vec<&MaskNode> = node.masks.iter().filter(|mask| mask.visible).collect();
        if !masks.is_empty() {
            // The masks build one coverage, and the content keeps only what
            // it covers.
            scene.push_layer(
                Fill::NonZero,
                BlendMode::new(Mix::Normal, Compose::DestIn),
                1.0,
                Affine::IDENTITY,
                &self.clip,
            );
            if masks[0].mode != MaskMode::Add {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    Color::WHITE,
                    None,
                    &self.clip,
                );
            }
            for mask in masks {
                let compose = match mask.mode {
                    MaskMode::Add => Compose::SrcOver,
                    MaskMode::Subtract => Compose::DestOut,
                    MaskMode::Intersect => Compose::DestIn,
                };
                scene.push_layer(
                    Fill::NonZero,
                    BlendMode::new(Mix::Normal, compose),
                    1.0,
                    Affine::IDENTITY,
                    &self.clip,
                );
                match &mask.shape {
                    MaskShape::Path(path) => {
                        scene.fill(
                            Fill::NonZero,
                            self.to_output,
                            Color::WHITE,
                            None,
                            path.as_ref(),
                        );
                    }
                    MaskShape::Pixels { tiled, transform } => {
                        self.pixels += tiled.draw(scene, self.to_output * *transform, self.clip);
                    }
                }
                scene.pop_layer();
            }
            scene.pop_layer();
        }
        scene.pop_layer();
    }
}

fn mix(blend: Blend) -> Mix {
    match blend {
        Blend::Normal | Blend::PassThrough => Mix::Normal,
        Blend::Multiply => Mix::Multiply,
        Blend::Screen => Mix::Screen,
        Blend::Overlay => Mix::Overlay,
        Blend::Darken => Mix::Darken,
        Blend::Lighten => Mix::Lighten,
        Blend::ColorDodge => Mix::ColorDodge,
        Blend::ColorBurn => Mix::ColorBurn,
        Blend::HardLight => Mix::HardLight,
        Blend::SoftLight => Mix::SoftLight,
        Blend::Difference => Mix::Difference,
        Blend::Exclusion => Mix::Exclusion,
        Blend::Hue => Mix::Hue,
        Blend::Saturation => Mix::Saturation,
        Blend::Color => Mix::Color,
        Blend::Luminosity => Mix::Luminosity,
    }
}
