//! Reads. Everything returned is owned; nothing borrows the document.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{
    Error, File, Hash, History, Layer, LayerAnswer, LayerId, LayerView, MaskContent, Offset,
    PageId, PageView, Pointer, PointerBuf, ProjectView, Read, Rect, Relationship, Result, Session,
    StyleId, StyleView, Tiles, doc,
    state::{State, hash_value},
    tiles,
};

#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// List the pages in order, each with its layer tree, without pixels or
    /// file bytes.
    GetProject,
    /// Get one page's layer tree, without pixels or file bytes.
    GetPage { page: PageId },
    /// Get one layer and where it sits.
    GetLayer { layer: LayerId },
    /// Get one text style.
    GetStyle { style: StyleId },
    /// Get the undo and redo labels.
    GetHistory,
}

#[derive(Clone, Debug, Serialize, Type, JsonSchema)]
#[serde(untagged)]
pub enum Answer {
    Project(ProjectView),
    Page(PageView),
    Layer(Box<LayerAnswer>),
    Style(Box<StyleView>),
    History(History),
}

impl Session {
    fn state(&self) -> State<'_> {
        State::new(&self.doc, &self.index)
    }

    pub fn query(&self, query: Query) -> Result<Answer> {
        Ok(match query {
            Query::GetProject => Answer::Project(self.project()?),
            Query::GetPage { page } => Answer::Page(self.page(page)?),
            Query::GetLayer { layer } => Answer::Layer(Box::new(self.layer(layer)?)),
            Query::GetStyle { style } => Answer::Style(Box::new(self.style(style)?)),
            Query::GetHistory => Answer::History(self.history()),
        })
    }

    pub fn project(&self) -> Result<ProjectView> {
        Ok(ProjectView {
            id: self.id,
            version: doc::version(&self.doc)?.to_string(),
            pages: self
                .index
                .pages
                .iter()
                .map(|page| self.page(*page))
                .collect::<Result<_>>()?,
            styles: {
                let mut state = self.state();
                let mut ids: Vec<StyleId> = self.index.styles.iter().copied().collect();
                ids.sort_unstable();
                ids.into_iter()
                    .map(|style| style_view(&mut state, style))
                    .collect::<Result<_>>()?
            },
        })
    }

    pub fn style(&self, style: StyleId) -> Result<StyleView> {
        style_view(&mut self.state(), style)
    }

    pub fn page(&self, page: PageId) -> Result<PageView> {
        let mut state = self.state();
        let data = state.page(page)?;
        let (size, roots) = (data.size, data.roots.clone());
        Ok(PageView {
            id: page,
            size,
            layers: roots
                .into_iter()
                .map(|root| view(&mut state, root))
                .collect::<Result<_>>()?,
        })
    }

    pub fn layer(&self, layer: LayerId) -> Result<LayerAnswer> {
        let mut state = self.state();
        let page = state.page_of(layer)?;
        let parent = state.node(layer)?.parent;
        Ok(LayerAnswer {
            page,
            parent,
            view: view(&mut state, layer)?,
        })
    }

    /// The value at `path` and a read of it with the relationship `inputTo`.
    pub fn read(&self, layer: LayerId, path: &Pointer) -> Result<(Value, Read)> {
        let mut state = self.state();
        let hash = state.hash(layer, path)?;
        let value = state.value(layer, path)?.unwrap_or(Value::Null);
        let relationship = Relationship::InputTo;
        Ok((
            value,
            Read {
                layer,
                path: path.to_buf(),
                hash,
                relationship,
            },
        ))
    }

    pub fn tiles(&self, layer: LayerId, rect: Option<Rect>) -> Result<Tiles> {
        let mut state = self.state();
        let node = state.node(layer)?;
        let Some(format) = node.pixel_format() else {
            return Err(Error::rejected(format!("layer {layer} has no pixels")));
        };
        let offset = match node.layer()? {
            Layer::Raster(raster) => raster.offset,
            Layer::Mask(mask) => match mask.content {
                MaskContent::Pixels { offset } => offset,
                MaskContent::Path { .. } => Offset::default(),
            },
            _ => Offset::default(),
        };
        let tiles = tiles::decode(state.tiles(layer)?, format, rect)?;
        Ok(Tiles {
            offset,
            format,
            tiles,
        })
    }

    /// A hash of a layer's stored tiles, which changes whenever its pixels
    /// do, taken without decoding them.
    pub fn tiles_hash(&self, layer: LayerId) -> Result<Hash> {
        let mut state = self.state();
        if state.node(layer)?.pixel_format().is_none() {
            return Err(Error::rejected(format!("layer {layer} has no pixels")));
        }
        let mut hasher = blake3::Hasher::new();
        for ((x, y), bytes) in state.tiles(layer)?.iter() {
            hasher.update(&x.to_le_bytes());
            hasher.update(&y.to_le_bytes());
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        Ok(hasher.finalize().into())
    }

    pub fn file(&self, hash: &Hash) -> Result<File> {
        doc::file(&self.doc, hash)
    }
}

fn style_view(state: &mut State<'_>, id: StyleId) -> Result<StyleView> {
    let node = state.style(id)?.clone();
    let style = node
        .style()
        .map_err(|error| Error::corrupt(format!("style {id}: {error}")))?;
    Ok(StyleView {
        id,
        hash: hash_value(Some(&node.json())),
        style,
        overrides: node.generated.keys().cloned().collect(),
        stale: state.stale_in(node.origin.clone())?,
        origin: node.origin,
    })
}

/// The paths under `path` that `value` sets: its non-null leaves.
fn set_paths(value: &Value, path: &Pointer) -> Vec<PointerBuf> {
    match value {
        Value::Null => Vec::new(),
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, value)| set_paths(value, &path.with_trailing_token(key)))
            .collect(),
        _ => vec![path.to_buf()],
    }
}

fn view(state: &mut State<'_>, id: LayerId) -> Result<LayerView> {
    let node = state.node(id)?;
    let (children, masks) = (node.children.clone(), node.masks.clone());
    let json = node.json();
    let layer = Layer::deserialize(&json)
        .map_err(|error| Error::corrupt(format!("layer {id}: {error}")))?;
    let origin = node.origin.clone();
    let overrides = node.generated.keys().cloned().collect();
    let style_overrides = match (node.props.get("text_style"), node.props.get("style")) {
        (Some(Value::String(_)), Some(style)) => set_paths(style, Pointer::from_static("/style")),
        _ => Vec::new(),
    };
    Ok(LayerView {
        id,
        hash: hash_value(Some(&json)),
        layer,
        origin,
        overrides,
        stale: state.stale(id)?,
        style_overrides,
        children: children
            .into_iter()
            .map(|child| view(state, child))
            .collect::<Result<_>>()?,
        masks: masks
            .into_iter()
            .map(|mask| view(state, mask))
            .collect::<Result<_>>()?,
    })
}
