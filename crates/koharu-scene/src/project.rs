//! The project: an immutable value of `imbl` maps, so a copy is O(1) and two
//! versions share every page one edit didn't touch. A page holds its layers,
//! and a layer its children, as values, so the tree can't disagree with
//! itself; an index of which page holds each layer is derived from them.

use std::slice;

use imbl::{OrdMap, Vector};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    Content, Error, Hash, Layer, LayerId, LayerKind, Offset, PageId, PixelFormat, ProjectId,
    Result, Size,
};

/// A picture's stored tiles: tile position, then the hash of its compressed
/// bytes.
pub(crate) type TileMap = OrdMap<(i32, i32), Hash>;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub(crate) id: ProjectId,
    pub(crate) order: Vector<PageId>,
    pub(crate) pages: OrdMap<PageId, Page>,
    /// Which page holds each layer, a block's parts included: derived from
    /// `pages` and never stored. Only `insert` and `take` write it.
    pub(crate) index: OrdMap<LayerId, PageId>,
    /// Each picture's tiles, under the id of the layer that owns it.
    pub(crate) tiles: OrdMap<LayerId, TileMap>,
}

/// A page: its size, the scan under it, and its layers with everything
/// inside them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: PageId,
    pub size: Size,
    pub background: Option<Background>,
    /// Drawn first to last.
    pub layers: Vec<Layer>,
}

/// The imported file a page was made from, drawn under every layer, as
/// Photoshop's one Background is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Background {
    pub file: Hash,
    pub visible: bool,
}

/// A picture a layer owns: a raster's or a mask's, or a block's mask's or
/// clean-up's.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Picture {
    pub format: PixelFormat,
    /// Where its grid sits on the page.
    pub offset: Offset,
    /// The layer in the tree holding it: its own, or a part's block.
    pub holder: LayerId,
    /// Whether a block's part is locked itself; the tree's locks are the
    /// holder's.
    pub locked: bool,
}

/// That a `child` may sit in `parent`, a layer of that kind or the page.
pub(crate) fn may_hold(
    parent: Option<LayerKind>,
    child: LayerKind,
) -> std::result::Result<(), &'static str> {
    use LayerKind::*;
    match (parent, child) {
        (Some(Block | Mask), _) => Err("a block or a mask holds nothing"),
        (Some(Group | Image | Raster | Shape | Text), Mask) => Ok(()),
        (_, Mask) => Err("a mask goes on a group, an image, a raster, a shape, or a text"),
        (None | Some(Group | Panel), _) => Ok(()),
        (Some(Balloon), Block | Text) => Ok(()),
        (Some(Balloon), _) => Err("a balloon holds only blocks and texts"),
        (Some(Image | Raster | Shape | Text), _) => {
            Err("only a page, a group, a panel, or a balloon holds drawn layers")
        }
    }
}

/// `layers` and everything inside them, depth first: a layer, then its
/// children, masks among them in their order.
pub(crate) fn depth_first(layers: &[Layer]) -> Vec<&Layer> {
    fn walk<'l>(layers: &'l [Layer], out: &mut Vec<&'l Layer>) {
        for layer in layers {
            out.push(layer);
            walk(&layer.children, out);
        }
    }
    let mut out = Vec::new();
    walk(layers, &mut out);
    out
}

/// The ids in `layers` and everything inside them, a block's parts included.
pub(crate) fn ids(layers: &[Layer]) -> Vec<LayerId> {
    depth_first(layers)
        .into_iter()
        .flat_map(|layer| {
            let parts = match &layer.content {
                Content::Block(block) => vec![block.mask.id, block.cleanup.id],
                _ => Vec::new(),
            };
            std::iter::once(layer.id).chain(parts)
        })
        .collect()
}

/// The first locked layer in `layer`'s subtree.
pub(crate) fn locked_inside(layer: &Layer) -> Option<LayerId> {
    depth_first(slice::from_ref(layer))
        .into_iter()
        .find(|layer| layer.locked)
        .map(|layer| layer.id)
}

/// A layer's place in its page's tree is a path of child indexes from the
/// page's top level. Paths come from `position` and are used before the
/// tree changes, so a bad one is a bug.
impl Page {
    /// The path to `id`, if this page's tree holds it.
    pub(crate) fn position(&self, id: LayerId) -> Option<Vec<usize>> {
        fn search(layers: &[Layer], id: LayerId, path: &mut Vec<usize>) -> bool {
            for (at, layer) in layers.iter().enumerate() {
                path.push(at);
                if layer.id == id || search(&layer.children, id, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        search(&self.layers, id, &mut path).then_some(path)
    }

    /// The list a path to a layer names: the page's own layers for the empty
    /// path, else that layer's children.
    pub(crate) fn list(&self, path: &[usize]) -> &Vec<Layer> {
        path.iter()
            .fold(&self.layers, |list, &at| &list[at].children)
    }

    pub(crate) fn list_mut(&mut self, path: &[usize]) -> &mut Vec<Layer> {
        path.iter()
            .fold(&mut self.layers, |list, &at| &mut list[at].children)
    }

    pub(crate) fn at(&self, path: &[usize]) -> &Layer {
        let (at, list) = path.split_last().expect("a path names a layer");
        &self.list(list)[*at]
    }

    pub(crate) fn at_mut(&mut self, path: &[usize]) -> &mut Layer {
        let (at, list) = path.split_last().expect("a path names a layer");
        &mut self.list_mut(list)[*at]
    }
}

impl Project {
    pub(crate) fn new(id: ProjectId) -> Self {
        Self {
            id,
            order: Vector::new(),
            pages: OrdMap::new(),
            index: OrdMap::new(),
            tiles: OrdMap::new(),
        }
    }

    #[must_use]
    pub fn id(&self) -> ProjectId {
        self.id
    }

    /// The pages in order.
    pub fn pages(&self) -> impl Iterator<Item = &Page> {
        self.order.iter().filter_map(|id| self.pages.get(id))
    }

    pub fn page(&self, id: PageId) -> Result<&Page> {
        self.pages
            .get(&id)
            .ok_or_else(|| Error::NotFound(format!("page {id}")))
    }

    /// A layer in the tree, with everything inside it.
    pub fn layer(&self, id: LayerId) -> Result<&Layer> {
        self.place(id)
            .map(|(page, path)| self.pages[&page].at(&path))
            .ok_or_else(|| Error::NotFound(format!("layer {id}")))
    }

    /// The page holding `id` in its tree, and the path to it there.
    pub(crate) fn place(&self, id: LayerId) -> Option<(PageId, Vec<usize>)> {
        let page = *self.index.get(&id)?;
        Some((page, self.pages.get(&page)?.position(id)?))
    }

    /// The picture `id` owns, if it owns one.
    pub(crate) fn picture(&self, id: LayerId) -> Option<Picture> {
        let page = self.pages.get(self.index.get(&id)?)?;
        depth_first(&page.layers).into_iter().find_map(|layer| {
            let picture = |format, offset, locked| Picture {
                format,
                offset,
                holder: layer.id,
                locked,
            };
            match &layer.content {
                Content::Raster(raster) if layer.id == id => {
                    Some(picture(PixelFormat::Rgba8, raster.offset, false))
                }
                Content::Mask(mask) if layer.id == id => {
                    Some(picture(PixelFormat::Gray8, mask.offset, false))
                }
                Content::Block(block) if block.mask.id == id => Some(picture(
                    PixelFormat::Gray8,
                    block.mask.content.offset,
                    block.mask.locked,
                )),
                Content::Block(block) if block.cleanup.id == id => Some(picture(
                    PixelFormat::Rgba8,
                    block.cleanup.content.offset,
                    block.cleanup.locked,
                )),
                _ => None,
            }
        })
    }

    /// Every layer of `page`, depth first. Without masks this is reading
    /// order.
    pub fn layers_of(&self, page: PageId) -> Result<Vec<&Layer>> {
        Ok(depth_first(&self.page(page)?.layers))
    }

    pub(crate) fn tiles(&self, layer: LayerId) -> Option<&TileMap> {
        self.tiles.get(&layer)
    }

    /// The first of `layer` and the layers above it that is locked.
    pub(crate) fn locked_at(&self, layer: LayerId) -> Option<LayerId> {
        let (page, path) = self.place(layer)?;
        let page = &self.pages[&page];
        (1..=path.len())
            .rev()
            .map(|depth| page.at(&path[..depth]))
            .find(|layer| layer.locked)
            .map(|layer| layer.id)
    }

    /// Puts `layer`, with everything inside it, at `index` of the list `list`
    /// names on `page`, and indexes its ids.
    pub(crate) fn insert(&mut self, page: PageId, list: &[usize], index: usize, layer: Layer) {
        for id in ids(slice::from_ref(&layer)) {
            self.index.insert(id, page);
        }
        self.pages
            .get_mut(&page)
            .expect("an op's page exists")
            .list_mut(list)
            .insert(index, layer);
    }

    /// Takes the layer at `path` on `page` out, with everything inside it,
    /// and drops its ids from the index. Its tiles are the caller's.
    pub(crate) fn take(&mut self, page: PageId, path: &[usize]) -> Layer {
        let (at, list) = path.split_last().expect("a path names a layer");
        let layer = self
            .pages
            .get_mut(&page)
            .expect("an op's page exists")
            .list_mut(list)
            .remove(*at);
        for id in ids(slice::from_ref(&layer)) {
            self.index.remove(&id);
        }
        layer
    }
}
