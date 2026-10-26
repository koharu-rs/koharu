//! The Loro layout.
//!
//! - `project` holds the project id and the schema version.
//! - `files` holds imported files by content hash, each its own map.
//! - `pages` is a movable list of page maps. A page map holds its id, its
//!   size, a `layers` tree, and the `tiles` of its layers by layer id.
//! - Each tree node's map holds the layer's stored id, `kind`, properties,
//!   `content` text, `generated` values, and `origin` actions.
//!
//! Large data never sits in a tree node's map: Loro keeps a deleted node's
//! map, and everything it reaches, through compaction, because a peer can
//! revive the node (`loro-internal` 1.17.1, `state.rs`).

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Display,
    str::FromStr,
};

use loro::{
    Container, ExpandType, Index, LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue,
    StyleConfig, TextDelta, TreeID, ValueOrContainer,
};
use semver::Version;
use serde_json::{Map, Value};
use strum::{AsRefStr, EnumString};

use crate::{
    ActionRecord, CharStyle, Error, File, Hash, Layer, LayerId, LayerKind, PageId, PixelFormat,
    Pointer, PointerBuf, Result, Revision, RichText, Size, Span,
    error::Kind,
    tiles::{self, TileMap},
};

/// The schema version this build reads and writes. A minor version only adds
/// core data and a major one removes or redefines it; the patch stays 0, since
/// stored data has no fixes.
pub(crate) const VERSION: Version = Version::new(1, 0, 0);

/// The keys of the layout above.
#[derive(Clone, Copy, AsRefStr, EnumString)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum Key {
    Project,
    Files,
    Pages,
    Id,
    Version,
    MediaType,
    Bytes,
    Width,
    Height,
    Layers,
    Tiles,
    Kind,
    Content,
    Generated,
    Origin,
    /// The field a generated value is wrapped in.
    V,
}

/// Loro keeps no mark configuration, so it is set on every load. Every mark
/// expands after its span, so typing at a span's end continues it.
pub(crate) fn configure(doc: &LoroDoc) {
    doc.config_default_text_style(Some(StyleConfig {
        expand: ExpandType::After,
    }));
}

pub(crate) fn version(doc: &LoroDoc) -> Result<Version> {
    string(&project(doc), Key::Version)?
        .parse()
        .map_err(Error::corrupt)
}

pub(crate) fn project(doc: &LoroDoc) -> LoroMap {
    doc.get_map(Key::Project.as_ref())
}

fn files(doc: &LoroDoc) -> LoroMap {
    doc.get_map(Key::Files.as_ref())
}

pub(crate) fn pages(doc: &LoroDoc) -> LoroMovableList {
    doc.get_movable_list(Key::Pages.as_ref())
}

pub(crate) fn revision(doc: &LoroDoc) -> Revision {
    let bytes = doc.state_frontiers().encode();
    Revision(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn child(map: &LoroMap, key: impl AsRef<str>) -> Option<Container> {
    map.get(key.as_ref())?.into_container().ok()
}

pub(crate) fn value(map: &LoroMap, key: impl AsRef<str>) -> Option<LoroValue> {
    map.get(key.as_ref())?.into_value().ok()
}

/// The child at `key`, inserted fresh if missing, so a cleared key never
/// brings its old content back.
pub(crate) fn child_map(parent: &LoroMap, key: impl AsRef<str>) -> Result<LoroMap> {
    let key = key.as_ref();
    match child(parent, key) {
        Some(Container::Map(map)) => Ok(map),
        _ => Ok(parent.insert_container(key, LoroMap::new())?),
    }
}

pub(crate) fn text(map: &LoroMap, key: impl AsRef<str>) -> Result<LoroText> {
    let key = key.as_ref();
    child(map, key)
        .and_then(|container| container.into_text().ok())
        .ok_or_else(|| Error::corrupt(format!("`{key}` is not a text")))
}

pub(crate) fn content(meta: &LoroMap) -> Result<LoroText> {
    text(meta, Key::Content)
}

pub(crate) fn generated(meta: &LoroMap) -> Result<LoroMap> {
    child_map(meta, Key::Generated)
}

fn string(map: &LoroMap, key: impl AsRef<str>) -> Result<String> {
    let key = key.as_ref();
    match value(map, key) {
        Some(LoroValue::String(value)) => Ok(value.to_string()),
        _ => Err(Error::corrupt(format!(
            "`{key}` is missing or not a string"
        ))),
    }
}

/// The stored id of a project, page, or layer map.
pub(crate) fn id<T: FromStr<Err: Display>>(map: &LoroMap) -> Result<T> {
    string(map, Key::Id)?.parse().map_err(Error::corrupt)
}

fn uint(map: &LoroMap, key: impl AsRef<str>) -> Result<u32> {
    let key = key.as_ref();
    match value(map, key) {
        Some(LoroValue::I64(value)) => u32::try_from(value).map_err(Error::corrupt),
        _ => Err(Error::corrupt(format!(
            "`{key}` is missing or not a number"
        ))),
    }
}

pub(crate) fn json(value: LoroValue) -> Value {
    value.into()
}

pub(crate) fn loro(value: Value) -> LoroValue {
    value.into()
}

pub(crate) fn page_maps(doc: &LoroDoc) -> Result<Vec<LoroMap>> {
    let pages = pages(doc);
    (0..pages.len())
        .map(|index| page_maps_at(doc, index))
        .collect()
}

pub(crate) fn page_maps_at(doc: &LoroDoc, position: usize) -> Result<LoroMap> {
    match pages(doc).get(position) {
        Some(ValueOrContainer::Container(Container::Map(map))) => Ok(map),
        _ => Err(Error::corrupt(format!("page {position} is not a map"))),
    }
}

/// A page's position and map.
pub(crate) fn find_page(doc: &LoroDoc, page: PageId) -> Result<(usize, LoroMap)> {
    for (index, map) in page_maps(doc)?.into_iter().enumerate() {
        if id::<PageId>(&map)? == page {
            return Ok((index, map));
        }
    }
    Err(Error::not_found(Kind::Page, page))
}

pub(crate) fn add_page(doc: &LoroDoc, index: usize, page: PageId, size: Size) -> Result<LoroMap> {
    let map = pages(doc).insert_container(index, LoroMap::new())?;
    map.insert(Key::Id.as_ref(), page.to_string())?;
    map.insert(Key::Width.as_ref(), i64::from(size.width))?;
    map.insert(Key::Height.as_ref(), i64::from(size.height))?;
    map.insert_container(Key::Layers.as_ref(), LoroTree::new())?;
    map.insert_container(Key::Tiles.as_ref(), LoroMap::new())?;
    Ok(map)
}

pub(crate) fn tree(page: &LoroMap) -> Result<LoroTree> {
    let tree = child(page, Key::Layers)
        .and_then(|container| container.into_tree().ok())
        .ok_or_else(|| Error::corrupt("a page has no layer tree"))?;
    tree.enable_fractional_index(0);
    Ok(tree)
}

/// A page's tiles, each layer's by layer id.
pub(crate) fn tile_store(page: &LoroMap) -> Result<LoroMap> {
    child(page, Key::Tiles)
        .and_then(|container| container.into_map().ok())
        .ok_or_else(|| Error::corrupt("a page has no tile store"))
}

pub(crate) fn read_tiles(page: &LoroMap, layer: LayerId) -> Result<TileMap> {
    let mut tiles = TileMap::new();
    let Some(Container::Map(map)) = child(&tile_store(page)?, layer.to_string()) else {
        return Ok(tiles);
    };
    let mut error = None;
    map.for_each(|key, value| match (tiles::parse_key(key), value) {
        (Some(position), ValueOrContainer::Value(LoroValue::Binary(bytes))) => {
            tiles.insert(position, bytes);
        }
        _ => {
            error = Some(Error::corrupt(format!(
                "tile {key} of layer {layer} is invalid"
            )))
        }
    });
    error.map_or(Ok(tiles), Err)
}

pub(crate) fn file_hashes(doc: &LoroDoc) -> Result<HashSet<Hash>> {
    files(doc)
        .keys()
        .map(|key| Hash::try_from(key.to_string()).map_err(Error::corrupt))
        .collect()
}

pub(crate) fn file(doc: &LoroDoc, hash: &Hash) -> Result<File> {
    let Some(Container::Map(map)) = child(&files(doc), hash.as_str()) else {
        return Err(Error::not_found(Kind::File, hash));
    };
    match (value(&map, Key::MediaType), value(&map, Key::Bytes)) {
        (Some(LoroValue::String(media_type)), Some(LoroValue::Binary(bytes))) => Ok(File {
            media_type: media_type.to_string(),
            bytes: bytes.to_vec(),
        }),
        _ => Err(Error::corrupt(format!("file {hash} is invalid"))),
    }
}

pub(crate) fn add_file(doc: &LoroDoc, hash: &Hash, media_type: &str, bytes: Vec<u8>) -> Result<()> {
    let file = files(doc).insert_container(hash.as_str(), LoroMap::new())?;
    file.insert(Key::MediaType.as_ref(), media_type)?;
    file.insert(Key::Bytes.as_ref(), bytes)?;
    Ok(())
}

pub(crate) fn remove_file(doc: &LoroDoc, hash: &Hash) -> Result<()> {
    Ok(files(doc).delete(hash.as_str())?)
}

pub(crate) fn style_of(attributes: Map<String, Value>) -> Result<CharStyle> {
    serde_json::from_value(Value::Object(attributes)).map_err(Error::corrupt)
}

pub(crate) fn rich_text(text: &LoroText) -> Result<RichText> {
    text.to_delta()
        .into_iter()
        .map(|delta| match delta {
            TextDelta::Insert { insert, attributes } => Ok(Span {
                insert,
                attributes: style_of(
                    attributes
                        .into_iter()
                        .flatten()
                        .map(|(key, value)| (key, json(value)))
                        .collect(),
                )?,
            }),
            _ => Err(Error::corrupt("a text's delta is not all inserts")),
        })
        .collect::<Result<_>>()
        .map(RichText)
}

/// The delta that writes `rich` into an empty text.
fn delta(rich: &RichText) -> Vec<TextDelta> {
    rich.0
        .iter()
        .map(|span| {
            let Value::Object(marks) =
                serde_json::to_value(&span.attributes).expect("styles serialize")
            else {
                unreachable!("a style serializes to an object")
            };
            let marks: Vec<(String, LoroValue)> = marks
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, loro(value)))
                .collect();
            TextDelta::Insert {
                insert: span.insert.clone(),
                attributes: (!marks.is_empty()).then(|| marks.into_iter().collect()),
            }
        })
        .collect()
}

/// Fills a new tree node's map.
pub(crate) fn write_node(
    meta: &LoroMap,
    id: LayerId,
    kind: LayerKind,
    props: Map<String, Value>,
    text: Option<&RichText>,
) -> Result<()> {
    meta.insert(Key::Id.as_ref(), id.to_string())?;
    meta.insert(Key::Kind.as_ref(), kind.as_ref())?;
    for (key, value) in props {
        meta.insert(&key, loro(value))?;
    }
    if let Some(text) = text {
        meta.insert_container(Key::Content.as_ref(), LoroText::new())?
            .apply_delta(&delta(text))?;
    }
    Ok(())
}

/// A model value kept while the user's override shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Generated {
    /// A property value, or `None` where the model had none.
    Value(Option<Value>),
    /// The model's text, with marks.
    Text(RichText),
}

/// The stored form of a generated property value: `{"v": value}`, or `{}`
/// for none.
pub(crate) fn wrap(value: Option<Value>) -> LoroValue {
    let mut map = Map::new();
    if let Some(value) = value {
        map.insert(Key::V.as_ref().to_owned(), value);
    }
    loro(Value::Object(map))
}

/// One loaded layer.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub id: LayerId,
    /// `None` until the node exists in Loro.
    pub tree_id: Option<TreeID>,
    pub parent: Option<LayerId>,
    pub kind: LayerKind,
    /// Every property but `content`, as it shows.
    pub props: Map<String, Value>,
    pub text: Option<RichText>,
    pub generated: BTreeMap<PointerBuf, Generated>,
    pub origin: Vec<ActionRecord>,
    pub children: Vec<LayerId>,
    pub masks: Vec<LayerId>,
}

impl Node {
    pub fn is_mask(&self) -> bool {
        self.kind == LayerKind::Mask
    }

    pub fn locked(&self) -> bool {
        self.props.get("locked") == Some(&Value::Bool(true))
    }

    pub fn pixel_format(&self) -> Option<PixelFormat> {
        match self.kind {
            LayerKind::Raster => Some(PixelFormat::Rgba8),
            LayerKind::Mask => self
                .props
                .get("content")?
                .get("pixels")
                .map(|_| PixelFormat::Gray8),
            _ => None,
        }
    }

    /// Files the layer names, including in model values kept under
    /// overrides.
    pub fn files(&self) -> Vec<Hash> {
        if self.kind != LayerKind::Image {
            return Vec::new();
        }
        let kept = match self.generated.get(Pointer::from_static("/file")) {
            Some(Generated::Value(value)) => value.as_ref(),
            _ => None,
        };
        [self.props.get("file"), kept]
            .into_iter()
            .flatten()
            .filter_map(|value| Hash::try_from(value.as_str()?.to_owned()).ok())
            .collect()
    }

    /// The layer as JSON: its properties, `kind`, and `content`.
    pub fn json(&self) -> Value {
        let mut map = self.props.clone();
        map.insert(Key::Kind.as_ref().to_owned(), self.kind.as_ref().into());
        if let Some(text) = &self.text {
            map.insert(
                Key::Content.as_ref().to_owned(),
                serde_json::to_value(text).expect("text serializes"),
            );
        }
        Value::Object(map)
    }

    pub fn layer(&self) -> Result<Layer> {
        serde_json::from_value(self.json())
            .map_err(|error| Error::corrupt(format!("layer {}: {error}", self.id)))
    }
}

/// One loaded page.
#[derive(Clone, Debug)]
pub(crate) struct PageData {
    pub size: Size,
    pub roots: Vec<LayerId>,
    pub nodes: HashMap<LayerId, Node>,
}

fn size(page: &LoroMap) -> Result<Size> {
    Ok(Size {
        width: uint(page, Key::Width)?,
        height: uint(page, Key::Height)?,
    })
}

pub(crate) fn load_page(page: &LoroMap) -> Result<PageData> {
    let tree = tree(page)?;
    let mut data = PageData {
        size: size(page)?,
        roots: Vec::new(),
        nodes: HashMap::new(),
    };
    for root in tree.roots() {
        let id = visit(&tree, root, None, &mut data)?;
        if data.nodes[&id].is_mask() {
            return Err(Error::corrupt(format!("mask {id} has no parent")));
        }
        data.roots.push(id);
    }
    Ok(data)
}

fn visit(
    tree: &LoroTree,
    tree_id: TreeID,
    parent: Option<LayerId>,
    data: &mut PageData,
) -> Result<LayerId> {
    let mut node = read_node(&tree.get_meta(tree_id)?)?;
    node.tree_id = Some(tree_id);
    node.parent = parent;
    for child in tree.children(tree_id).unwrap_or_default() {
        let id = visit(tree, child, Some(node.id), data)?;
        if data.nodes[&id].is_mask() {
            node.masks.push(id);
        } else if node.masks.is_empty() {
            node.children.push(id);
        } else {
            return Err(Error::corrupt(format!(
                "layer {id} sits after its parent's masks"
            )));
        }
    }
    let id = node.id;
    data.nodes.insert(id, node);
    Ok(id)
}

fn read_node(meta: &LoroMap) -> Result<Node> {
    let mut node = Node {
        id: id(meta)?,
        tree_id: None,
        parent: None,
        kind: string(meta, Key::Kind)?.parse().map_err(Error::corrupt)?,
        props: Map::new(),
        text: None,
        generated: BTreeMap::new(),
        origin: Vec::new(),
        children: Vec::new(),
        masks: Vec::new(),
    };
    let mut entries = Vec::new();
    meta.for_each(|key, value| entries.push((key.to_owned(), value)));
    for (key, value) in entries {
        match (key.parse(), value) {
            (Ok(Key::Id | Key::Kind), _) => {}
            (Ok(Key::Content), ValueOrContainer::Container(Container::Text(text))) => {
                node.text = Some(rich_text(&text)?)
            }
            (Ok(Key::Generated), ValueOrContainer::Container(Container::Map(map))) => {
                node.generated = read_generated(&map)?
            }
            (Ok(Key::Origin), ValueOrContainer::Value(origin)) => {
                node.origin = serde_json::from_value(json(origin)).map_err(Error::corrupt)?;
            }
            (_, ValueOrContainer::Value(value)) => {
                node.props.insert(key, json(value));
            }
            (_, ValueOrContainer::Container(_)) => {
                return Err(Error::corrupt(format!(
                    "layer {} has an unexpected `{key}`",
                    node.id
                )));
            }
        }
    }
    if (node.kind == LayerKind::Text) != node.text.is_some() {
        return Err(Error::corrupt(format!(
            "layer {} has the wrong content",
            node.id
        )));
    }
    Ok(node)
}

fn read_generated(map: &LoroMap) -> Result<BTreeMap<PointerBuf, Generated>> {
    let mut entries = Vec::new();
    map.for_each(|key, value| entries.push((key.to_owned(), value)));
    entries
        .into_iter()
        .map(|(key, value)| {
            let path = PointerBuf::parse(key).map_err(Error::corrupt)?;
            let generated = match value {
                ValueOrContainer::Container(Container::Text(text))
                    if path == crate::model::CONTENT =>
                {
                    Generated::Text(rich_text(&text)?)
                }
                ValueOrContainer::Value(value) => match json(value) {
                    Value::Object(mut map) => Generated::Value(map.remove(Key::V.as_ref())),
                    _ => return Err(Error::corrupt("a generated value is not wrapped")),
                },
                _ => return Err(Error::corrupt("a generated value is invalid")),
            };
            Ok((path, generated))
        })
        .collect()
}

/// Every page's id in order, and every layer's page.
pub(crate) fn layer_pages(doc: &LoroDoc) -> Result<(Vec<PageId>, HashMap<LayerId, PageId>)> {
    let mut pages = Vec::new();
    let mut layers = HashMap::new();
    for map in page_maps(doc)? {
        let page = id(&map)?;
        let tree = tree(&map)?;
        for node in tree.get_nodes(false) {
            layers.insert(id(&tree.get_meta(node.id)?)?, page);
        }
        pages.push(page);
    }
    Ok((pages, layers))
}

/// The files an unloaded page's image layers name, read without loading the
/// page.
pub(crate) fn page_files(page: &LoroMap) -> Result<Vec<Hash>> {
    let tree = tree(page)?;
    let mut files = Vec::new();
    for node in tree.get_nodes(false) {
        let meta = tree.get_meta(node.id)?;
        if string(&meta, Key::Kind)? != LayerKind::Image.as_ref() {
            continue;
        }
        let kept = match child(&meta, Key::Generated) {
            Some(Container::Map(generated)) => value(&generated, "/file")
                .and_then(|value| json(value).get(Key::V.as_ref()).cloned()),
            _ => None,
        };
        let shown = value(&meta, "file").map(json);
        files.extend(
            [shown, kept]
                .into_iter()
                .flatten()
                .filter_map(|value| Hash::try_from(value.as_str()?.to_owned()).ok()),
        );
    }
    Ok(files)
}

/// Where a Loro event landed: the page's position, the tree nodes on its
/// path, and the layer whose tiles it touched.
pub(crate) fn locate(
    path: &[(loro::ContainerID, Index)],
) -> (Option<usize>, Vec<TreeID>, Option<LayerId>) {
    let mut page = None;
    let mut nodes = Vec::new();
    let mut layer = None;
    let mut in_store = false;
    for (index, (_, position)) in path.iter().enumerate() {
        match position {
            Index::Seq(at) if index == 1 => page = Some(*at),
            Index::Node(node) => nodes.push(*node),
            Index::Key(key) if in_store => layer = key.as_str().parse().ok(),
            _ => {}
        }
        in_store = matches!(position, Index::Key(key) if key.as_str() == Key::Tiles.as_ref());
    }
    (page, nodes, layer)
}

/// True when an event's target is a page's tile store, whose keys are layer
/// ids.
pub(crate) fn is_store(path: &[(loro::ContainerID, Index)]) -> bool {
    matches!(path.last(), Some((_, Index::Key(key))) if key.as_str() == Key::Tiles.as_ref())
}
