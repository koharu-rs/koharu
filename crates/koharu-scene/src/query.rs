//! Reads. Everything returned is owned; nothing borrows the document.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{
    Error, File, FileHash, History, LanguageTag, Layer, LayerAnswer, LayerId, LayerView,
    MaskContent, Offset, PageId, PageView, Pointer, ProjectView, Read, Rect, Relationship, Result,
    Session, StoryEntry, Tiles,
    doc::{self, Node},
    state::State,
    tiles,
};

#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// List the pages in order, with their sizes and layer counts.
    GetProject,
    /// Get one page's layer tree, without pixels or file bytes.
    GetPage { page: PageId },
    /// Get one layer and where it sits.
    GetLayer { layer: LayerId },
    /// List each translation on a page, paired with the OCR text it came
    /// from.
    GetStory { page: PageId },
    /// Get the undo and redo labels.
    GetHistory,
}

#[derive(Clone, Debug, Serialize, Type, JsonSchema)]
#[serde(untagged)]
pub enum Answer {
    Project(ProjectView),
    Page(PageView),
    Layer(Box<LayerAnswer>),
    Story(Vec<StoryEntry>),
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
            Query::GetStory { page } => Answer::Story(self.story(page)?),
            Query::GetHistory => Answer::History(self.history()),
        })
    }

    pub fn project(&self) -> Result<ProjectView> {
        Ok(ProjectView {
            id: self.id,
            version: doc::version(&self.doc)?.to_string(),
            pages: doc::page_maps(&self.doc)?
                .iter()
                .enumerate()
                .map(|(index, page)| doc::page_summary(page, index))
                .collect::<Result<_>>()?,
        })
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

    /// Each text a model translated from another text, in drawing order.
    pub fn story(&self, page: PageId) -> Result<Vec<StoryEntry>> {
        let mut state = self.state();
        let data = state.page(page)?;
        let mut order = Vec::new();
        let mut stack: Vec<LayerId> = data.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(data.nodes[&id].children.iter().rev());
        }
        let mut entries = Vec::new();
        for id in order {
            let node = state.node(id)?;
            let source = node
                .origin
                .iter()
                .flat_map(|action| &action.ingredients)
                .find(|read| read.relationship == Relationship::ParentOf)
                .map(|read| read.layer);
            let Some(source) = source.filter(|source| state.layers.contains_key(source)) else {
                continue;
            };
            let (Some((text, language)), Some((source_text, source_language))) =
                (text_of(state.node(id)?), text_of(state.node(source)?))
            else {
                continue;
            };
            entries.push(StoryEntry {
                translation: id,
                translation_hash: state.hash(id, &Pointer::root())?,
                text,
                language,
                source,
                source_hash: state.hash(source, &Pointer::root())?,
                source_text,
                source_language,
                stale: !state.stale(id)?.is_empty(),
            });
        }
        Ok(entries)
    }

    /// The value at `path` and a read of it with the relationship `inputTo`.
    /// `/pixels` reads as `null`; its hash covers the decoded tiles.
    pub fn read(&self, layer: LayerId, path: &Pointer) -> Result<(Value, Read)> {
        let mut state = self.state();
        let hash = state.hash(layer, path)?;
        let value = state.value(layer, path)?.unwrap_or(Value::Null);
        let relationship = Relationship::InputTo;
        Ok((
            value,
            Read {
                layer,
                path: path.clone(),
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

    pub fn file(&self, hash: &FileHash) -> Result<File> {
        doc::file(&self.doc, hash)
    }
}

/// A text layer's plain text and language; `None` for other kinds.
fn text_of(node: &Node) -> Option<(String, LanguageTag)> {
    let text = node.text.as_ref()?.text.clone();
    let language = LanguageTag::new(node.props.get("language")?.as_str()?).ok()?;
    Some((text, language))
}

fn view(state: &mut State<'_>, id: LayerId) -> Result<LayerView> {
    let node = state.node(id)?;
    let (children, masks) = (node.children.clone(), node.masks.clone());
    let json = node.json();
    let layer = Layer::deserialize(&json)
        .map_err(|error| Error::corrupt(format!("layer {id}: {error}")))?;
    let origin = node.origin.clone();
    let overrides = node.generated.keys().cloned().collect();
    Ok(LayerView {
        id,
        hash: crate::state::hash_value(Some(&json)),
        layer,
        origin,
        overrides,
        stale: state.stale(id)?,
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
