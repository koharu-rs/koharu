//! A page's tree: a node for each layer, the page's background, and each
//! block's clean-up and translation, holding its content, how it composites,
//! and what it drew.

use std::sync::Arc;

use koharu_scene::{Blend, LayerId, MaskMode};
use kurbo::{Affine, BezPath, Rect, Shape as _, Stroke};
use vello_gpu::peniko::{Color, Fill};

use crate::{
    pixels::Pixels,
    text::{ResolvedText, TextLayout},
};

/// A shape layer's path and how it's painted, in its own coordinates.
pub(crate) struct ShapeVector {
    pub path: BezPath,
    pub fill_rule: Fill,
    pub fill: Option<Color>,
    pub stroke: Option<(Stroke, Color)>,
    /// What it covers, its stroke included.
    pub bounds: Rect,
}

/// What a node draws, in its own coordinates, as the renderer keeps it.
#[derive(Clone)]
pub(crate) enum Content {
    /// A text's glyphs as its layout placed them.
    Text(Arc<TextLayout>),
    Shape(Arc<ShapeVector>),
    Pixels(Arc<Pixels>),
    /// A white shape: a mask frame's area.
    Coverage(Arc<BezPath>),
}

impl Content {
    /// What it covers through `transform`; `None` when it draws nothing.
    fn bounds(&self, transform: Affine) -> Option<Rect> {
        let covered = match self {
            Self::Text(layout) => layout.bounds,
            Self::Shape(shape) => shape.bounds,
            Self::Pixels(pixels) => return Some(pixels.bounds(transform)),
            Self::Coverage(path) => path.bounding_box(),
        };
        (covered.area() > 0.0).then(|| transform.transform_rect_bbox(covered))
    }
}

#[derive(Clone)]
pub(crate) struct Node {
    /// The layer it draws, a block's clean-up by its own id; `None` for the
    /// page's background, the first node at the top, and a block's
    /// translation, among the block's children.
    pub layer: Option<LayerId>,
    pub visible: bool,
    pub opacity: f32,
    pub blend: Blend,
    /// What it draws itself, and where on the page; `None` for a group, a
    /// panel, or a block, which draw their children.
    pub content: Option<(Content, Affine)>,
    /// Whether a text's strokes draw: a PSD writes them as a stroke effect.
    pub strokes: bool,
    pub children: Vec<Node>,
    pub masks: Vec<MaskNode>,
    /// What it draws, in page pixels: its content's, or a container's
    /// children's; `None` when it draws nothing.
    pub bounds: Option<Rect>,
}

impl Node {
    /// A node drawing `content` as it is, with no children or masks.
    pub(crate) fn new(
        layer: Option<LayerId>,
        visible: bool,
        content: Option<(Content, Affine)>,
    ) -> Self {
        Self {
            layer,
            visible,
            opacity: 1.0,
            blend: Blend::Normal,
            bounds: content
                .as_ref()
                .and_then(|(content, transform)| content.bounds(*transform)),
            content,
            strokes: true,
            children: Vec::new(),
            masks: Vec::new(),
        }
    }

    /// For a text or a translation, the laid-out runs.
    pub(crate) fn text(&self) -> Option<&ResolvedText> {
        match &self.content {
            Some((Content::Text(layout), _)) => Some(&layout.resolved),
            _ => None,
        }
    }
}

/// A mask: what it covers, drawn white, and its bounds in page pixels.
#[derive(Clone)]
pub(crate) struct MaskNode {
    pub visible: bool,
    pub mode: MaskMode,
    pub content: Option<(Content, Affine)>,
    pub bounds: Rect,
}

impl MaskNode {
    pub(crate) fn new(visible: bool, mode: MaskMode, content: Option<(Content, Affine)>) -> Self {
        Self {
            visible,
            mode,
            bounds: content
                .as_ref()
                .and_then(|(content, transform)| content.bounds(*transform))
                .unwrap_or(Rect::ZERO),
            content,
        }
    }
}
