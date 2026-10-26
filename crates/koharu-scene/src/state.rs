//! A lazily loaded view of the document. `apply` checks every op against it
//! before writing, and queries read through it.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use loro::LoroDoc;
use serde_json::Value;

use crate::{
    Error, Hash, LayerId, PageId, Pointer, PointerBuf, Result,
    doc::{self, Node, PageData},
    error::Kind,
    model::CONTENT,
    overrides,
    tiles::TileMap,
};

/// What the session keeps between edits so that nothing rescans the
/// document: the page order, each layer's page, and the stored files.
#[derive(Clone, Debug, Default)]
pub(crate) struct Index {
    pub pages: Vec<PageId>,
    pub layers: HashMap<LayerId, PageId>,
    pub files: HashSet<Hash>,
}

impl Index {
    pub fn load(doc: &LoroDoc) -> Result<Self> {
        let (pages, layers) = doc::layer_pages(doc)?;
        Ok(Self {
            pages,
            layers,
            files: doc::file_hashes(doc)?,
        })
    }
}

pub(crate) struct Slot {
    pub id: PageId,
    pub data: Option<PageData>,
    /// The page's position in Loro before the edit; `None` for a page the
    /// edit being planned adds.
    pub position: Option<usize>,
}

pub(crate) struct State<'d> {
    pub doc: &'d LoroDoc,
    pub pages: Vec<Slot>,
    /// Borrowed from the session, and copied only if an edit changes it.
    pub layers: Cow<'d, HashMap<LayerId, PageId>>,
    pub files: Cow<'d, HashSet<Hash>>,
    tiles: HashMap<LayerId, TileMap>,
    /// Hashes are only taken before a state changes: `plan` checks every
    /// read before its first op, and queries never change the state.
    hashes: HashMap<(LayerId, PointerBuf), Hash>,
}

impl<'d> State<'d> {
    pub fn new(doc: &'d LoroDoc, index: &'d Index) -> Self {
        Self {
            doc,
            pages: index
                .pages
                .iter()
                .enumerate()
                .map(|(position, id)| Slot {
                    id: *id,
                    data: None,
                    position: Some(position),
                })
                .collect(),
            layers: Cow::Borrowed(&index.layers),
            files: Cow::Borrowed(&index.files),
            tiles: HashMap::new(),
            hashes: HashMap::new(),
        }
    }

    /// The index after the edit, or `None` when it is unchanged.
    pub fn index(self, original: &Index) -> Option<Index> {
        let pages: Vec<PageId> = self.pages.iter().map(|slot| slot.id).collect();
        let unchanged = matches!(self.layers, Cow::Borrowed(_))
            && matches!(self.files, Cow::Borrowed(_))
            && pages == original.pages;
        (!unchanged).then(|| Index {
            pages,
            layers: self.layers.into_owned(),
            files: self.files.into_owned(),
        })
    }

    fn page_map(&self, position: usize) -> Result<loro::LoroMap> {
        doc::page_maps_at(self.doc, position)
    }

    pub fn position(&self, page: PageId) -> Result<usize> {
        self.pages
            .iter()
            .position(|slot| slot.id == page)
            .ok_or_else(|| Error::not_found(Kind::Page, page))
    }

    pub fn page(&mut self, page: PageId) -> Result<&mut PageData> {
        let position = self.position(page)?;
        if self.pages[position].data.is_none() {
            let at = self.pages[position].position.expect("a new page is loaded");
            self.pages[position].data = Some(doc::load_page(&self.page_map(at)?)?);
        }
        Ok(self.pages[position].data.as_mut().expect("just loaded"))
    }

    pub fn page_of(&self, layer: LayerId) -> Result<PageId> {
        self.layers
            .get(&layer)
            .copied()
            .ok_or_else(|| Error::not_found(Kind::Layer, layer))
    }

    pub fn node(&mut self, layer: LayerId) -> Result<&mut Node> {
        let page = self.page_of(layer)?;
        self.page(page)?
            .nodes
            .get_mut(&layer)
            .ok_or_else(|| Error::not_found(Kind::Layer, layer))
    }

    /// The first locked layer among `layer` and its ancestors.
    pub fn locked(&mut self, layer: LayerId) -> Result<Option<LayerId>> {
        let mut current = Some(layer);
        while let Some(id) = current {
            let node = self.node(id)?;
            if node.locked() {
                return Ok(Some(id));
            }
            current = node.parent;
        }
        Ok(None)
    }

    /// A layer and everything inside it, parents first.
    pub fn subtree(&mut self, layer: LayerId) -> Result<Vec<LayerId>> {
        let mut ids = vec![layer];
        let mut index = 0;
        while index < ids.len() {
            let node = self.node(ids[index])?;
            ids.extend(
                node.children
                    .iter()
                    .chain(&node.masks)
                    .copied()
                    .collect::<Vec<_>>(),
            );
            index += 1;
        }
        Ok(ids)
    }

    pub fn tiles(&mut self, layer: LayerId) -> Result<&mut TileMap> {
        if !self.tiles.contains_key(&layer) {
            let page = self.page_of(layer)?;
            let tiles = match self.pages[self.position(page)?].position {
                Some(at) => doc::read_tiles(&self.page_map(at)?, layer)?,
                None => TileMap::new(),
            };
            self.tiles.insert(layer, tiles);
        }
        Ok(self.tiles.get_mut(&layer).expect("just loaded"))
    }

    pub fn forget_tiles(&mut self, layer: LayerId) {
        self.tiles.remove(&layer);
    }

    /// What a read at `path` sees: `""` is the layer with its text, and a
    /// text layer's `/content` is its plain text, a store of its own that is
    /// read whole.
    pub fn value(&mut self, layer: LayerId, path: &Pointer) -> Result<Option<Value>> {
        let node = self.node(layer)?;
        if let Some(text) = &node.text
            && path.starts_with(CONTENT)
        {
            return match path == CONTENT {
                true => Ok(Some(Value::String(text.text()))),
                false => Err(Error::rejected(format!(
                    "{:?} can only be read whole",
                    path.as_str()
                ))),
            };
        }
        Ok(match path.is_root() {
            true => Some(node.json()),
            false => overrides::get(&node.props, path).cloned(),
        })
    }

    /// The hash of what a read at `path` sees.
    pub fn hash(&mut self, layer: LayerId, path: &Pointer) -> Result<Hash> {
        if let Some(hash) = self.hashes.get(&(layer, path.to_buf())) {
            return Ok(hash.clone());
        }
        let hash = hash_value(self.value(layer, path)?.as_ref());
        self.hashes.insert((layer, path.to_buf()), hash.clone());
        Ok(hash)
    }

    /// The paths of `layer` whose model ingredients have changed or gone.
    pub fn stale(&mut self, layer: LayerId) -> Result<Vec<PointerBuf>> {
        let origin = self.node(layer)?.origin.clone();
        let mut stale = Vec::new();
        for action in origin {
            for read in &action.ingredients {
                if !self.layers.contains_key(&read.layer)
                    || self.hash(read.layer, &read.path)? != read.hash
                {
                    stale.extend(action.writes);
                    break;
                }
            }
        }
        Ok(stale)
    }

    /// Every file a layer names, reading unloaded pages without loading them.
    pub fn files_in_use(&self) -> Result<HashSet<Hash>> {
        let mut used = HashSet::new();
        for slot in &self.pages {
            match (&slot.data, slot.position) {
                (Some(page), _) => used.extend(page.nodes.values().flat_map(Node::files)),
                (None, Some(at)) => used.extend(doc::page_files(&self.page_map(at)?)?),
                (None, None) => {}
            }
        }
        Ok(used)
    }
}

/// The hash of a value: BLAKE3 over its RFC 8785 canonical JSON, because
/// Loro maps iterate in hash order and the app build turns on serde_json's
/// `preserve_order`. An absent value hashes no bytes, which no JSON value
/// encodes to.
pub(crate) fn hash_value(value: Option<&Value>) -> Hash {
    let bytes = value
        .map(|value| serde_json_canonicalizer::to_vec(value).expect("JSON values canonicalize"))
        .unwrap_or_default();
    blake3::hash(&bytes).into()
}
