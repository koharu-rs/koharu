//! A page's tree: a node for each layer, the page's background, and each
//! block's clean-up and translation, holding its content, how it composites,
//! and what it drew. Its content comes from a function, which the renderer
//! answers from what it retains.

use std::sync::Arc;

use koharu_scene::{
    Background, Blend, Block, Content as SceneContent, Hash, Layer, LayerId, MaskMode, Offset,
    Panel, Shape, Text,
};
use vello::{
    Scene,
    kurbo::{Affine, BezPath, Rect, Shape as _},
};

use crate::{Error, Result, fonts::FontRequest, text::ResolvedText, tiles::Tiled};

/// Vector content, a text's glyphs or a shape, in its own coordinates.
pub(crate) struct Vector {
    pub scene: Scene,
    /// A text's strokes, drawn under `scene`.
    pub strokes: Option<Scene>,
    /// What it covers.
    pub bounds: Rect,
}

#[derive(Clone)]
pub(crate) enum Content {
    None,
    Vector {
        vector: Arc<Vector>,
        /// Whether its strokes draw.
        strokes: bool,
        transform: Affine,
    },
    Pixels {
        tiled: Arc<Tiled>,
        transform: Affine,
    },
    /// A white shape: a panel's outline as its children's mask, or a mask
    /// frame's area.
    Coverage(Arc<BezPath>),
}

#[derive(Clone)]
pub(crate) struct Node {
    /// The layer it draws; `None` for the page's background, at the top, and
    /// a block's translation, among the block's children.
    pub layer: Option<LayerId>,
    pub visible: bool,
    pub opacity: f32,
    pub blend: Blend,
    pub content: Content,
    pub children: Vec<Node>,
    pub masks: Vec<MaskNode>,
    /// What it draws, in page pixels: its content's, or a container's
    /// children's; `None` when it draws nothing.
    pub bounds: Option<Rect>,
    /// For a text or a translation, the laid-out runs.
    pub text: Option<Arc<ResolvedText>>,
}

impl Node {
    /// A node drawing `content` as it is, with no children or masks.
    pub(crate) fn plain(layer: Option<LayerId>, visible: bool, content: Content) -> Self {
        Self {
            layer,
            visible,
            opacity: 1.0,
            blend: Blend::Normal,
            content,
            children: Vec::new(),
            masks: Vec::new(),
            bounds: None,
            text: None,
        }
    }
}

/// A mask: what it covers, drawn white, and its bounds in page pixels.
#[derive(Clone)]
pub(crate) struct MaskNode {
    pub visible: bool,
    pub mode: MaskMode,
    pub content: Content,
    pub bounds: Rect,
}

/// What a layer's content is made from.
#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    Text(LayerId, &'a Text),
    Shape(&'a Shape),
    Image(Hash),
    /// The picture of the layer or block part with this id.
    Tiles(LayerId),
}

/// Content as the renderer retains it.
#[derive(Clone)]
pub(crate) enum Retained {
    Vector {
        vector: Arc<Vector>,
        text: Option<Arc<ResolvedText>>,
        /// Faces the text asked for that aren't installed or loaded yet.
        missing: Vec<FontRequest>,
    },
    Pixels(Option<Arc<Tiled>>),
}

/// A node's own drawing: its content, bounds, and text.
type Own = (Content, Option<Rect>, Option<Arc<ResolvedText>>);

/// Builds a page's nodes, each layer's content from `content`.
pub(crate) struct Tree<F> {
    content: F,
    /// Faces some text asked for that aren't installed or loaded yet.
    pub missing: Vec<FontRequest>,
}

impl<F: Fn(Source) -> Result<Retained>> Tree<F> {
    pub(crate) fn new(content: F) -> Self {
        Self {
            content,
            missing: Vec::new(),
        }
    }

    pub(crate) fn background(&mut self, background: Background) -> Result<Node> {
        let (content, bounds, _) = self.pixels(Source::Image(background.file), Affine::IDENTITY)?;
        Ok(Node {
            bounds,
            ..Node::plain(None, background.visible, content)
        })
    }

    pub(crate) fn layer(&mut self, layer: &Layer) -> Result<Node> {
        let mut children = layer
            .children
            .iter()
            .filter(|child| !child.is_mask())
            .map(|child| self.layer(child))
            .collect::<Result<Vec<_>>>()?;
        let mut masks = layer
            .children
            .iter()
            .filter(|child| child.is_mask())
            .map(|mask| self.mask(mask))
            .collect::<Result<Vec<_>>>()?;
        let (content, bounds, text) = match &layer.content {
            SceneContent::Group(_) | SceneContent::Balloon(_) => (Content::None, None, None),
            SceneContent::Panel(panel) => {
                masks.push(outline(panel)?);
                (Content::None, None, None)
            }
            SceneContent::Block(block) => {
                children = self.block(layer.id, block)?;
                (Content::None, None, None)
            }
            SceneContent::Text(text) => {
                self.vector(Source::Text(layer.id, text), Affine::from(text.transform))?
            }
            SceneContent::Shape(shape) => self.vector(Source::Shape(shape), Affine::IDENTITY)?,
            SceneContent::Image(image) => {
                self.pixels(Source::Image(image.file), Affine::from(image.transform))?
            }
            SceneContent::Raster(raster) => {
                self.pixels(Source::Tiles(layer.id), offset(raster.offset))?
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
                .filter_map(|child| child.bounds)
                .reduce(|a, b| a.union(b)),
            _ => bounds,
        };
        Ok(Node {
            layer: Some(layer.id),
            visible: layer.visible,
            opacity: f32::from(layer.opacity),
            blend: layer.blend,
            content,
            children,
            masks,
            bounds,
            text,
        })
    }

    /// Block `id` draws its clean-up through its mask, then its translation;
    /// its source is never drawn.
    fn block(&mut self, id: LayerId, block: &Block) -> Result<Vec<Node>> {
        let cleanup = &block.cleanup;
        let (content, bounds, _) =
            self.pixels(Source::Tiles(cleanup.id), offset(cleanup.content.offset))?;
        let mask = &block.mask;
        let (cover, covered, _) =
            self.pixels(Source::Tiles(mask.id), offset(mask.content.offset))?;
        let mut nodes = vec![Node {
            opacity: f32::from(cleanup.opacity),
            blend: cleanup.blend,
            masks: vec![MaskNode {
                visible: mask.visible,
                mode: mask.content.mode,
                content: cover,
                bounds: covered.unwrap_or(Rect::ZERO),
            }],
            bounds,
            ..Node::plain(Some(cleanup.id), cleanup.visible, content)
        }];
        if let Some(text) = &block.translation {
            let (content, bounds, text) =
                self.vector(Source::Text(id, text), Affine::from(text.transform))?;
            nodes.push(Node {
                bounds,
                text,
                ..Node::plain(None, true, content)
            });
        }
        Ok(nodes)
    }

    fn mask(&mut self, layer: &Layer) -> Result<MaskNode> {
        let SceneContent::Mask(mask) = &layer.content else {
            return Err(Error::invalid(format!("layer {} is not a mask", layer.id)));
        };
        // An empty mask covers nothing.
        let (content, bounds, _) = self.pixels(Source::Tiles(layer.id), offset(mask.offset))?;
        Ok(MaskNode {
            visible: layer.visible,
            mode: mask.mode,
            content,
            bounds: bounds.unwrap_or(Rect::ZERO),
        })
    }

    fn pixels(&self, source: Source, transform: Affine) -> Result<Own> {
        Ok(match (self.content)(source)? {
            Retained::Pixels(Some(tiled)) => {
                let bounds = tiled.bounds(transform);
                (Content::Pixels { tiled, transform }, Some(bounds), None)
            }
            _ => (Content::None, None, None),
        })
    }

    fn vector(&mut self, source: Source, transform: Affine) -> Result<Own> {
        let Retained::Vector {
            vector,
            text,
            missing,
        } = (self.content)(source)?
        else {
            return Ok((Content::None, None, None));
        };
        self.missing.extend(missing);
        let shown =
            (vector.bounds.area() > 0.0).then(|| transform.transform_rect_bbox(vector.bounds));
        Ok((
            Content::Vector {
                vector,
                strokes: true,
                transform,
            },
            shown,
            text,
        ))
    }
}

/// A panel's outline, the mask its children show inside, as a Clip Studio
/// Paint frame border folder does.
fn outline(panel: &Panel) -> Result<MaskNode> {
    let outline = parse(panel.outline.as_str())?;
    Ok(MaskNode {
        visible: true,
        mode: MaskMode::Add,
        bounds: outline.bounding_box(),
        content: Content::Coverage(Arc::new(outline)),
    })
}

pub(crate) fn parse(path: &str) -> Result<BezPath> {
    BezPath::from_svg(path).map_err(|error| Error::invalid(format!("invalid path data: {error}")))
}

/// Where a picture's grid sits on the page.
fn offset(offset: Offset) -> Affine {
    Affine::translate((f64::from(offset.x), f64::from(offset.y)))
}

#[cfg(test)]
mod tests {
    use koharu_scene::{Created, Edit, Op, Session};
    use serde_json::json;

    use super::*;

    /// A shape whose content covers `bounds`.
    fn shape(bounds: Rect) -> Retained {
        Retained::Vector {
            vector: Arc::new(Vector {
                scene: Scene::new(),
                strokes: None,
                bounds,
            }),
            text: None,
            missing: Vec::new(),
        }
    }

    #[test]
    fn a_group_covers_its_children_and_a_block_draws_its_translation_apart() -> Result<()> {
        let dir = tempfile::tempdir().expect("a folder");
        let mut session = Session::create(dir.path().join("tree.khr"))?;
        let ops = [
            json!({"op": "add_page", "id": "$page", "index": 0, "size": {"width": 100, "height": 100}}),
            json!({"op": "add_layer", "id": "$group", "page": "$page", "index": 0, "layer": {"kind": "group"}}),
            json!({"op": "add_layer", "page": "$page", "parent": "$group", "index": 0, "layer": {"kind": "shape", "path": "M0 0 H1 V1 Z"}}),
            json!({"op": "add_layer", "id": "$block", "page": "$page", "index": 1, "layer": {
                "kind": "block", "source": {"content": "やあ"}, "translation": {"content": "Hi"},
            }}),
        ];
        let change = session.apply(Edit {
            label: "tree".into(),
            ops: ops
                .into_iter()
                .map(|op| serde_json::from_value::<Op>(op).expect("an op"))
                .collect(),
        })?;
        let project = session.project();
        let page = project.page(change.pages.added[0])?;
        let mut tree = Tree::new(|source| {
            Ok(match source {
                Source::Shape(_) => shape(Rect::new(10.0, 10.0, 20.0, 30.0)),
                Source::Text(..) => shape(Rect::new(0.0, 0.0, 5.0, 5.0)),
                _ => Retained::Pixels(None),
            })
        });
        let nodes: Vec<Node> = page
            .layers
            .iter()
            .map(|layer| tree.layer(layer))
            .collect::<Result<_>>()?;
        let label = |name: &str| {
            change.created.iter().find_map(|created| match created {
                Created::Layer {
                    id,
                    label: Some(label),
                } if label == name => Some(*id),
                _ => None,
            })
        };
        assert_eq!(nodes[0].layer, label("group"));
        assert_eq!(nodes[0].bounds, Some(Rect::new(10.0, 10.0, 20.0, 30.0)));
        let block = &nodes[1];
        assert_eq!(block.children.len(), 2);
        let translation = &block.children[1];
        assert_eq!(translation.layer, None);
        assert_eq!(translation.bounds, Some(Rect::new(0.0, 0.0, 5.0, 5.0)));
        // The block covers what its translation does, its clean-up empty.
        assert_eq!(block.bounds, translation.bounds);
        Ok(())
    }
}
