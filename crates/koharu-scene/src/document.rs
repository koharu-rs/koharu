//! How a project lives in Loro containers.
//!
//! ```text
//! project   LoroMap          { schema, id }
//! pages     LoroMovableList  one LoroMap per page, in project order:
//!   id      the page's Id
//!   meta    LoroMap   Page
//!   images  LoroMap   Id -> LoroMap { media_type, width, height, bytes }
//!   regions LoroMap   Id -> LoroMap Region
//!   texts   LoroMap   Id -> LoroMap Text
//!   layers  LoroTree  node meta: { id } + Layer
//! ```
//!
//! Every image has its own container, so Loro decodes an image only when it
//! is read, never when its page or its neighbours are.

use std::collections::{BTreeMap, BTreeSet};

use loro::{
    Container, ContainerID, LoroDoc, LoroMap, LoroMovableList, LoroTree, LoroValue, TreeID,
    UpdateOptions, ValueOrContainer,
};

use crate::{
    Error, Id, Image, LayerContent, LayerNode, PageSummary, PageView, Result, schema::Record,
};

pub(crate) const SCHEMA: i64 = 1;
pub(crate) const SCHEMA_KEY: &str = "schema";
pub(crate) const ID: &str = "id";
const PROJECT: &str = "project";
const PAGES: &str = "pages";

pub(crate) fn project(doc: &LoroDoc) -> LoroMap {
    doc.get_map(PROJECT)
}

pub(crate) fn page_list(doc: &LoroDoc) -> LoroMovableList {
    doc.get_movable_list(PAGES)
}

/// Fails closed on a document this version does not understand.
pub(crate) fn validate(doc: &LoroDoc) -> Result<()> {
    let project = project(doc);
    let Some(ValueOrContainer::Value(LoroValue::I64(SCHEMA))) = project.get(SCHEMA_KEY) else {
        return Err(Error::Schema("unsupported project schema".into()));
    };
    read_id(&project)?;
    let pages = page_list(doc);
    for index in 0..pages.len() {
        let page = PageMap::at(&pages, index)?;
        check_references(&page.view()?, &page.image_ids()?)?;
    }
    Ok(())
}

pub(crate) fn is_page_list(id: &ContainerID) -> bool {
    matches!(id, ContainerID::Root { name, .. } if name.as_str() == PAGES)
}

/// The containers of one page.
pub(crate) struct PageMap {
    pub(crate) id: Id,
    map: LoroMap,
}

impl PageMap {
    /// Lays out an empty page at `index`.
    pub(crate) fn insert(pages: &LoroMovableList, index: usize, id: Id) -> Result<Self> {
        let map = pages.insert_container(index, LoroMap::new())?;
        map.insert(ID, id.to_string())?;
        for key in ["meta", "images", "regions", "texts"] {
            map.insert_container(key, LoroMap::new())?;
        }
        map.insert_container("layers", LoroTree::new())?;
        Ok(Self { id, map })
    }

    pub(crate) fn at(pages: &LoroMovableList, index: usize) -> Result<Self> {
        let Some(ValueOrContainer::Container(Container::Map(map))) = pages.get(index) else {
            return Err(Error::Schema(format!("page {index} is not a map")));
        };
        Ok(Self {
            id: read_id(&map)?,
            map,
        })
    }

    /// Finds a page and its position in project order.
    pub(crate) fn find(doc: &LoroDoc, id: Id) -> Result<(usize, Self)> {
        let pages = page_list(doc);
        for index in 0..pages.len() {
            let page = Self::at(&pages, index)?;
            if page.id == id {
                return Ok((index, page));
            }
        }
        Err(Error::NotFound { kind: "page", id })
    }

    pub(crate) fn meta(&self) -> Result<LoroMap> {
        child_map(&self.map, "meta")
    }

    pub(crate) fn images(&self) -> Result<LoroMap> {
        child_map(&self.map, "images")
    }

    pub(crate) fn regions(&self) -> Result<LoroMap> {
        child_map(&self.map, "regions")
    }

    pub(crate) fn texts(&self) -> Result<LoroMap> {
        child_map(&self.map, "texts")
    }

    pub(crate) fn layers(&self) -> Result<LoroTree> {
        match self.map.get("layers") {
            Some(ValueOrContainer::Container(Container::Tree(tree))) => Ok(tree),
            _ => Err(Error::Schema(format!("page {} has no layer tree", self.id))),
        }
    }

    pub(crate) fn image_ids(&self) -> Result<BTreeSet<Id>> {
        self.images()?.keys().map(|key| parse_id(&key)).collect()
    }

    /// Everything about the page except its image bytes.
    pub(crate) fn view(&self) -> Result<PageView> {
        Ok(PageView {
            id: self.id,
            page: read(&self.meta()?, &[])?,
            layers: layer_nodes(&self.layers()?, None)?,
            texts: records(&self.texts()?)?,
            regions: records(&self.regions()?)?,
        })
    }
}

pub(crate) fn child_map(map: &LoroMap, key: &str) -> Result<LoroMap> {
    match map.get(key) {
        Some(ValueOrContainer::Container(Container::Map(child))) => Ok(child),
        _ => Err(Error::Schema(format!("{key} is not a map"))),
    }
}

pub(crate) fn read_id(map: &LoroMap) -> Result<Id> {
    match map.get(ID) {
        Some(ValueOrContainer::Value(LoroValue::String(id))) => parse_id(&id),
        _ => Err(Error::Schema("a record has no id".into())),
    }
}

fn parse_id(id: &str) -> Result<Id> {
    id.parse()
        .map_err(|_| Error::Schema(format!("{id} is not an id")))
}

/// Reads a record from `map`, ignoring the bookkeeping keys in `skip`.
pub(crate) fn read<T: Record>(map: &LoroMap, skip: &[&str]) -> Result<T> {
    let mut value = map.get_deep_value();
    if let LoroValue::Map(fields) = &mut value {
        let fields = fields.make_mut();
        for key in skip {
            fields.remove(*key);
        }
    }
    decode(&value)
}

fn decode<T: Record>(value: &LoroValue) -> Result<T> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|source| Error::Record {
            kind: T::KIND,
            source,
        })
}

/// Writes `record` into `map`, touching only the fields that differ.
pub(crate) fn write<T: Record>(map: &LoroMap, record: &T) -> Result<()> {
    let value = serde_json::to_value(record)
        .and_then(serde_json::from_value::<LoroValue>)
        .map_err(|source| Error::Record {
            kind: T::KIND,
            source,
        })?;
    let LoroValue::Map(fields) = value else {
        return Err(Error::Schema(format!("{} is not a map", T::KIND)));
    };
    for (key, value) in fields.iter() {
        if T::TEXT_FIELDS.contains(&key.as_str()) {
            let LoroValue::String(value) = value else {
                return Err(Error::Schema(format!("{} {key} is not text", T::KIND)));
            };
            let text = map.ensure_mergeable_text(key)?;
            if text.to_string() != **value {
                text.update(value, UpdateOptions::default())
                    .expect("a text update without a timeout cannot time out");
            }
            continue;
        }
        let current = match map.get(key) {
            Some(ValueOrContainer::Value(current)) => Some(current),
            _ => None,
        };
        match value {
            LoroValue::Null if current.is_some() => map.delete(key)?,
            LoroValue::Null => {}
            value if current.as_ref() != Some(value) => map.insert(key, value.clone())?,
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn insert_image(images: &LoroMap, id: Id, image: &Image) -> Result<()> {
    let map = images.insert_container(&id.to_string(), LoroMap::new())?;
    map.insert("media_type", image.media_type.as_str())?;
    map.insert("width", i64::from(image.width))?;
    map.insert("height", i64::from(image.height))?;
    map.insert("bytes", LoroValue::Binary(image.bytes.clone()))?;
    Ok(())
}

pub(crate) fn read_image(images: &LoroMap, id: Id) -> Result<Image> {
    let map =
        child_map(images, &id.to_string()).map_err(|_| Error::NotFound { kind: "image", id })?;
    let field = |key: &str| match map.get(key) {
        Some(ValueOrContainer::Value(value)) => Some(value),
        _ => None,
    };
    let dimension = |key: &str| match field(key) {
        Some(LoroValue::I64(value)) => u32::try_from(value).ok(),
        _ => None,
    };
    match (
        field("media_type"),
        dimension("width"),
        dimension("height"),
        field("bytes"),
    ) {
        (
            Some(LoroValue::String(media_type)),
            Some(width),
            Some(height),
            Some(LoroValue::Binary(bytes)),
        ) => Ok(Image {
            media_type: media_type.to_string(),
            width,
            height,
            bytes,
        }),
        _ => Err(Error::Schema(format!("image {id} is invalid"))),
    }
}

/// Finds the tree node whose meta carries `id`.
pub(crate) fn find_node(tree: &LoroTree, id: Id) -> Result<TreeID> {
    for node in tree.get_nodes(false) {
        if read_id(&tree.get_meta(node.id)?)? == id {
            return Ok(node.id);
        }
    }
    Err(Error::NotFound { kind: "layer", id })
}

pub(crate) fn summaries(doc: &LoroDoc) -> Result<Vec<PageSummary>> {
    let pages = page_list(doc);
    (0..pages.len())
        .map(|index| {
            let page = PageMap::at(&pages, index)?;
            Ok(PageSummary {
                id: page.id,
                page: read(&page.meta()?, &[])?,
            })
        })
        .collect()
}

fn layer_nodes(tree: &LoroTree, parent: Option<TreeID>) -> Result<Vec<LayerNode>> {
    tree.children(parent)
        .unwrap_or_default()
        .into_iter()
        .map(|node| {
            let meta = tree.get_meta(node)?;
            Ok(LayerNode {
                id: read_id(&meta)?,
                layer: read(&meta, &[ID])?,
                children: layer_nodes(tree, Some(node))?,
            })
        })
        .collect()
}

fn records<T: Record>(map: &LoroMap) -> Result<BTreeMap<Id, T>> {
    let LoroValue::Map(entries) = map.get_deep_value() else {
        return Err(Error::Schema(format!("{} records are not a map", T::KIND)));
    };
    entries
        .iter()
        .map(|(key, value)| Ok((parse_id(key)?, decode(value)?)))
        .collect()
}

/// Checks that every id a page refers to exists, so a reader never meets a
/// dangling reference.
pub(crate) fn check_references(view: &PageView, images: &BTreeSet<Id>) -> Result<()> {
    let found = |kind: &'static str, id: Id, found: bool| {
        found
            .then_some(())
            .ok_or_else(|| Error::Schema(format!("missing {kind} {id}")))
    };
    let page = &view.page;
    for image in [Some(page.source), page.text_mask, page.bubble_mask]
        .into_iter()
        .flatten()
    {
        found("image", image, images.contains(&image))?;
    }
    for inside in view.regions.values().flat_map(|region| &region.inside) {
        found("region", *inside, view.regions.contains_key(inside))?;
    }
    for region in view.texts.values().filter_map(|text| text.region) {
        found("region", region, view.regions.contains_key(&region))?;
    }
    let mut stack = view.layers.iter().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        match &node.layer.content {
            LayerContent::Text {
                text,
                fit_to,
                flows_in,
                ..
            } => {
                found("text", *text, view.texts.contains_key(text))?;
                for region in [fit_to, flows_in].into_iter().flatten() {
                    found("region", *region, view.regions.contains_key(region))?;
                }
            }
            LayerContent::Raster { image, .. } => found("image", *image, images.contains(image))?,
            LayerContent::Group | LayerContent::TextGroup => {}
        }
        stack.extend(&node.children);
    }
    Ok(())
}
