//! Edits. An edit is checked in full against a `State`, planned into
//! low-level writes, and only then written, because Loro has no rollback.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use loro::{LoroDoc, LoroMap, LoroText, LoroTree, LoroValue, TextDelta, TreeID, UpdateOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use uuid::Uuid;

use crate::{
    Action, ActionRecord, Created, Error, Hash, Layer, LayerId, LayerKind, NamedStyle, PageChanges,
    PageId, Pointer, PointerBuf, ProjectId, Read, Rect, Ref, Relationship, Result, RichText, Size,
    SoftwareAgent, StyleId,
    doc::{self, Generated, Key, PageData, StyleNode},
    error::Kind,
    model::CONTENT,
    overrides,
    state::{Index, Slot, State},
    tiles::{self, Changes},
};

/// One change to the project, committed as one undo step.
#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub project: ProjectId,
    /// The undo menu label.
    pub label: String,
    pub author: Author,
    /// What the author read, checked before anything is written.
    #[serde(default)]
    pub reads: Vec<Read>,
    pub ops: Vec<Op>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    /// The user, directly or through an agent or MCP client acting for them.
    User {
        #[serde(default)]
        via: Option<SoftwareAgent>,
    },
    /// A model run.
    Model { action: Action },
}

/// One op of an edit. Variants and fields skipped by schemars carry bytes,
/// editor deltas, or model ingredients, which tools never send.
#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Add a page of `size` at `index`.
    #[schemars(skip)]
    AddPage {
        #[serde(default)]
        id: Option<Ref>,
        index: usize,
        size: Size,
    },
    /// Move a page to `index`, counted after the page is taken out.
    MovePage { page: Ref, index: usize },
    /// Remove a page with its layers.
    RemovePage { page: Ref },
    /// Add a layer under `parent`, or at the top level of `page`. A mask's
    /// `index` counts among the parent's masks; any other layer's among its
    /// drawn children, first drawn first.
    AddLayer {
        #[serde(default)]
        id: Option<Ref>,
        page: Ref,
        #[serde(default)]
        parent: Option<Ref>,
        index: usize,
        layer: Box<Layer>,
        #[serde(default)]
        #[schemars(skip)]
        ingredients: Vec<Read>,
    },
    /// Copy a layer with its children, masks, text, and pixels, keeping what
    /// models wrote and what the user overrode. The copy goes just above the
    /// original unless `index` says where.
    DuplicateLayer {
        layer: Ref,
        #[serde(default)]
        id: Option<Ref>,
        #[serde(default)]
        index: Option<usize>,
    },
    /// Move a layer within its page, to `index` counted after it is taken
    /// out.
    MoveLayer {
        layer: Ref,
        #[serde(default)]
        parent: Option<Ref>,
        index: usize,
    },
    /// Remove a layer and everything inside it.
    RemoveLayer { layer: Ref },
    /// Change a layer's properties with a JSON Merge Patch (RFC 7396);
    /// `null` clears an optional property.
    UpdateLayer {
        layer: Ref,
        #[schemars(schema_with = "crate::tools::layer_patch")]
        patch: Value,
        #[serde(default)]
        #[schemars(skip)]
        ingredients: Vec<Read>,
    },
    /// Replace a text layer's text; styling on unchanged spans is kept.
    SetText {
        layer: Ref,
        text: String,
        #[serde(default)]
        #[schemars(skip)]
        ingredients: Vec<Read>,
    },
    /// Apply a Loro text delta from the rich editor. The edit must read the
    /// layer's `/content`.
    #[schemars(skip)]
    EditText {
        layer: Ref,
        #[specta(type = Vec<Value>)]
        #[schemars(with = "Vec<Value>")]
        delta: Vec<TextDelta>,
    },
    /// Replace pixels: RGBA8 for a raster, grey8 for a pixel mask, in the
    /// layer's pixel coordinates. Pixels are not a property, so no action
    /// records writing them.
    #[schemars(skip)]
    WritePixels {
        layer: Ref,
        rect: Rect,
        pixels: Vec<u8>,
    },
    /// Delete the overrides at `paths`, or all of them, so the model's values
    /// show again.
    Reset {
        layer: Ref,
        #[serde(default)]
        #[specta(type = Option<Vec<String>>)]
        #[schemars(with = "Option<Vec<String>>")]
        paths: Option<Vec<PointerBuf>>,
    },
    /// Add an imported file, stored once by its content hash.
    #[schemars(skip)]
    AddFile { media_type: String, bytes: Vec<u8> },
    /// Add a text style, which texts name in `text_style`.
    AddTextStyle {
        #[serde(default)]
        id: Option<Ref>,
        style: NamedStyle,
        #[serde(default)]
        #[schemars(skip)]
        ingredients: Vec<Read>,
    },
    /// Change a text style with a JSON Merge Patch, for every text naming it.
    UpdateTextStyle {
        style: Ref,
        #[schemars(schema_with = "crate::tools::style_patch")]
        patch: Value,
        #[serde(default)]
        #[schemars(skip)]
        ingredients: Vec<Read>,
    },
    /// Delete a text style's overrides at `paths`, or all of them, so the
    /// model's values show again.
    ResetTextStyle {
        style: Ref,
        #[serde(default)]
        #[specta(type = Option<Vec<String>>)]
        #[schemars(with = "Option<Vec<String>>")]
        paths: Option<Vec<PointerBuf>>,
    },
    /// Remove a text style no text names.
    RemoveTextStyle { style: Ref },
}

impl Op {
    fn ingredients(&self) -> &[Read] {
        match self {
            Self::AddLayer { ingredients, .. }
            | Self::UpdateLayer { ingredients, .. }
            | Self::SetText { ingredients, .. }
            | Self::AddTextStyle { ingredients, .. }
            | Self::UpdateTextStyle { ingredients, .. } => ingredients,
            _ => &[],
        }
    }
}

/// A layer as a write names it: its tree node if it is in Loro, or its id if
/// this edit creates it.
#[derive(Clone, Copy)]
pub(crate) enum NodeRef {
    Tree(TreeID),
    New(LayerId),
}

/// A low-level write, planned after every check passed.
pub(crate) enum Write {
    AddFile {
        hash: Hash,
        media_type: String,
        bytes: Vec<u8>,
    },
    RemoveFile(Hash),
    AddPage {
        id: PageId,
        index: usize,
        size: Size,
    },
    MovePage {
        id: PageId,
        index: usize,
    },
    RemovePage(PageId),
    AddNode {
        page: PageId,
        id: LayerId,
        parent: Option<NodeRef>,
        index: usize,
        kind: LayerKind,
        props: Map<String, Value>,
        text: Option<RichText>,
    },
    MoveNode {
        page: PageId,
        node: NodeRef,
        parent: Option<NodeRef>,
        index: usize,
    },
    RemoveNode {
        page: PageId,
        node: NodeRef,
    },
    Layer {
        page: PageId,
        node: NodeRef,
        write: LayerWrite,
    },
    AddStyle {
        id: StyleId,
        props: Map<String, Value>,
    },
    Style {
        id: StyleId,
        entry: Entry,
    },
    RemoveStyle(StyleId),
    /// Clears a layer's tiles first when `clear` is set.
    Tiles {
        page: PageId,
        layer: LayerId,
        clear: bool,
        tiles: Changes,
    },
}

/// Sets an entry of a layer's or style's map, or of its `generated` map;
/// `None` deletes it.
pub(crate) struct Entry {
    generated: bool,
    key: String,
    value: Option<LoroValue>,
}

impl Entry {
    fn origin(origin: &[ActionRecord]) -> Self {
        Self {
            generated: false,
            key: Key::Origin.as_ref().to_owned(),
            value: Some(doc::loro(
                serde_json::to_value(origin).expect("actions serialize"),
            )),
        }
    }

    fn write(self, map: LoroMap) -> Result<()> {
        let map = match self.generated {
            true => doc::generated(&map)?,
            false => map,
        };
        match self.value {
            Some(value) => map.insert(&self.key, value)?,
            None => map.delete(&self.key)?,
        }
        Ok(())
    }
}

/// The entries that turn shown values `old` into `new`.
fn props_entries(old: &Map<String, Value>, new: &Map<String, Value>) -> Vec<Entry> {
    new.iter()
        .filter(|(key, value)| old.get(*key) != Some(value))
        .map(|(key, value)| Entry {
            generated: false,
            key: key.clone(),
            value: Some(doc::loro(value.clone())),
        })
        .collect()
}

/// The entries that turn kept model values `old` into `new`. Kept texts are
/// written apart.
fn generated_entries(
    old: &BTreeMap<PointerBuf, Generated>,
    new: &BTreeMap<PointerBuf, Generated>,
) -> Vec<Entry> {
    let removed = old
        .keys()
        .filter(|path| !new.contains_key(*path))
        .map(|path| Entry {
            generated: true,
            key: path.as_str().to_owned(),
            value: None,
        });
    let set = new.iter().filter_map(|(path, generated)| match generated {
        Generated::Value(value) if old.get(path) != Some(generated) => Some(Entry {
            generated: true,
            key: path.as_str().to_owned(),
            value: Some(doc::wrap(value.clone())),
        }),
        _ => None,
    });
    removed.chain(set).collect()
}

pub(crate) enum LayerWrite {
    Set(Entry),
    /// Replaces one of the layer's texts in place, keeping the same
    /// `LoroText` so cursors into it stay valid.
    Text {
        store: TextStore,
        from: TextFrom,
    },
}

/// The text that shows, or the model's text kept under an override.
#[derive(Clone, Copy)]
pub(crate) enum TextStore {
    Shown,
    Kept,
}

pub(crate) enum TextFrom {
    /// A diff against the current text, so marks on unchanged text stay.
    Plain(String),
    /// The editor's delta, applied as is.
    Delta(Vec<TextDelta>),
    /// Another text, with its marks.
    Copy { node: NodeRef, store: TextStore },
}

/// A checked edit and what it changes.
#[derive(Default)]
pub(crate) struct Planned {
    pub writes: Vec<Write>,
    pub created: Vec<Created>,
    pub kept: Vec<LayerId>,
    pub pages: PageChanges,
    pub layers: BTreeSet<LayerId>,
    pub styles: BTreeSet<StyleId>,
    /// The index after the edit, or `None` when it is unchanged.
    pub index: Option<Index>,
}

/// What a model's action is recorded on.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Target {
    Layer(LayerId),
    Style(StyleId),
}

pub(crate) fn plan(doc: &LoroDoc, index: &Index, edit: &Edit) -> Result<Planned> {
    let mut state = State::new(doc, index);
    for read in edit
        .reads
        .iter()
        .chain(edit.ops.iter().flat_map(Op::ingredients))
    {
        let current = match state.layers.contains_key(&read.layer) {
            true => Some(state.hash(read.layer, &read.path)?),
            false => None,
        };
        if current.as_ref() != Some(&read.hash) {
            return Err(Error::Stale { read: read.clone() });
        }
    }
    let mut planner = Planner {
        state,
        edit,
        out: Planned::default(),
        labels: HashMap::new(),
        actions: BTreeMap::new(),
        files: BTreeSet::new(),
    };
    for (index, op) in edit.ops.iter().enumerate() {
        planner.op(index, op)?;
    }
    planner.finish(index)
}

struct Planner<'s, 'd> {
    state: State<'d>,
    edit: &'s Edit,
    out: Planned,
    labels: HashMap<String, Uuid>,
    /// This edit's action on each layer and style a model wrote.
    actions: BTreeMap<Target, ActionRecord>,
    /// Files that may have lost their last reference.
    files: BTreeSet<Hash>,
}

impl Planner<'_, '_> {
    fn model(&self) -> Option<&Action> {
        match &self.edit.author {
            Author::Model { action } => Some(action),
            Author::User { .. } => None,
        }
    }

    fn user_only(&self, op: usize, what: &str) -> Result<()> {
        match self.model() {
            Some(_) => Err(Error::invalid(op, format!("only the user can {what}"))),
            None => Ok(()),
        }
    }

    fn new_id(&mut self, op: usize, id: Option<&Ref>) -> Result<(Uuid, Option<String>)> {
        let Some(id) = id else {
            return Ok((Uuid::now_v7(), None));
        };
        if let Some(label) = id.label() {
            if self.labels.contains_key(label) {
                return Err(Error::invalid(
                    op,
                    format!("label ${label} is already used in this edit"),
                ));
            }
            let uuid = Uuid::now_v7();
            self.labels.insert(label.to_owned(), uuid);
            return Ok((uuid, Some(label.to_owned())));
        }
        let uuid = id.uuid().expect("a ref is a label or a UUID");
        if self.state.layers.contains_key(&LayerId(uuid))
            || self.state.position(PageId(uuid)).is_ok()
            || self.state.styles.contains(&StyleId(uuid))
        {
            return Err(Error::invalid(op, format!("id {uuid} already exists")));
        }
        Ok((uuid, None))
    }

    fn resolve(&self, id: &Ref) -> Result<Uuid> {
        match id.label() {
            Some(label) => self
                .labels
                .get(label)
                .copied()
                .ok_or_else(|| Error::not_found(Kind::Label, id)),
            None => Ok(id.uuid().expect("a ref is a label or a UUID")),
        }
    }

    fn page_ref(&self, id: &Ref) -> Result<PageId> {
        let page = PageId(self.resolve(id)?);
        self.state.position(page)?;
        Ok(page)
    }

    fn layer_ref(&self, id: &Ref) -> Result<(LayerId, PageId)> {
        let layer = LayerId(self.resolve(id)?);
        Ok((layer, self.state.page_of(layer)?))
    }

    fn style_ref(&mut self, id: &Ref) -> Result<StyleId> {
        let style = StyleId(self.resolve(id)?);
        self.state.style(style)?;
        Ok(style)
    }

    fn unlocked(&mut self, op: usize, layer: LayerId) -> Result<()> {
        match self.state.locked(layer)? {
            Some(locked) => Err(Error::Locked { op, layer: locked }),
            None => Ok(()),
        }
    }

    fn node_ref(&mut self, id: LayerId) -> Result<NodeRef> {
        Ok(self
            .state
            .node(id)?
            .tree_id
            .map_or(NodeRef::New(id), NodeRef::Tree))
    }

    fn touch(&mut self, page: PageId, layer: LayerId) {
        self.out.pages.changed.insert(page);
        self.out.layers.insert(layer);
    }

    fn push_layer(&mut self, page: PageId, id: LayerId, write: LayerWrite) -> Result<()> {
        let node = self.node_ref(id)?;
        self.out.writes.push(Write::Layer { page, node, write });
        self.touch(page, id);
        Ok(())
    }

    fn push_entries(&mut self, page: PageId, id: LayerId, entries: Vec<Entry>) -> Result<()> {
        for entry in entries {
            self.push_layer(page, id, LayerWrite::Set(entry))?;
        }
        Ok(())
    }

    fn push_style(&mut self, id: StyleId, entries: Vec<Entry>) {
        for entry in entries {
            self.out.writes.push(Write::Style { id, entry });
        }
        self.out.styles.insert(id);
    }

    /// Records a model's write on a layer or style in this edit's action.
    fn record(
        &mut self,
        op: usize,
        target: Target,
        writes: Vec<PointerBuf>,
        ingredients: &[Read],
    ) -> Result<()> {
        let Some(action) = self.model().cloned() else {
            return Ok(());
        };
        let record = self.actions.entry(target).or_insert_with(|| ActionRecord {
            action,
            writes: Vec::new(),
            ingredients: Vec::new(),
        });
        for path in writes {
            if !record.writes.contains(&path) {
                record.writes.push(path);
            }
        }
        for read in ingredients {
            if !record.ingredients.contains(read) {
                record.ingredients.push(read.clone());
            }
        }
        if record
            .ingredients
            .iter()
            .filter(|read| read.relationship == Relationship::ParentOf)
            .count()
            > 1
        {
            let what = match target {
                Target::Layer(id) => format!("layer {id}"),
                Target::Style(id) => format!("style {id}"),
            };
            return Err(Error::invalid(
                op,
                format!("{what} would have more than one parentOf ingredient"),
            ));
        }
        Ok(())
    }

    fn op(&mut self, index: usize, op: &Op) -> Result<()> {
        match op {
            Op::AddPage {
                id,
                index: at,
                size,
            } => self.add_page(index, id.as_ref(), *at, *size),
            Op::MovePage { page, index: to } => self.move_page(index, page, *to),
            Op::RemovePage { page } => self.remove_page(page),
            Op::AddLayer {
                id,
                page,
                parent,
                index: at,
                layer,
                ingredients,
            } => self.add_layer(
                index,
                id.as_ref(),
                page,
                parent.as_ref(),
                *at,
                layer,
                ingredients,
            ),
            Op::DuplicateLayer {
                layer,
                id,
                index: at,
            } => self.duplicate_layer(index, layer, id.as_ref(), *at),
            Op::MoveLayer {
                layer,
                parent,
                index: to,
            } => self.move_layer(index, layer, parent.as_ref(), *to),
            Op::RemoveLayer { layer } => self.remove_layer(index, layer),
            Op::UpdateLayer {
                layer,
                patch,
                ingredients,
            } => self.update_layer(index, layer, patch, ingredients),
            Op::SetText {
                layer,
                text,
                ingredients,
            } => self.set_text(index, layer, text, ingredients),
            Op::EditText { layer, delta } => self.edit_text(index, layer, delta),
            Op::WritePixels {
                layer,
                rect,
                pixels,
            } => self.write_pixels(index, layer, *rect, pixels),
            Op::Reset { layer, paths } => self.reset(index, layer, paths.as_deref()),
            Op::AddFile { media_type, bytes } => self.add_file(index, media_type, bytes),
            Op::AddTextStyle {
                id,
                style,
                ingredients,
            } => self.add_style(index, id.as_ref(), style, ingredients),
            Op::UpdateTextStyle {
                style,
                patch,
                ingredients,
            } => self.update_style(index, style, patch, ingredients),
            Op::ResetTextStyle { style, paths } => self.reset_style(index, style, paths.as_deref()),
            Op::RemoveTextStyle { style } => self.remove_style(index, style),
        }
    }

    fn add_page(&mut self, op: usize, id: Option<&Ref>, index: usize, size: Size) -> Result<()> {
        if size.width == 0 || size.height == 0 {
            return Err(Error::invalid(op, "a page needs a nonzero size"));
        }
        if index > self.state.pages.len() {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the last page"),
            ));
        }
        let (uuid, label) = self.new_id(op, id)?;
        let id = PageId(uuid);
        let data = PageData {
            size,
            roots: Vec::new(),
            nodes: HashMap::new(),
        };
        self.state.pages.insert(
            index,
            Slot {
                id,
                data: Some(data),
                position: None,
            },
        );
        self.out.writes.push(Write::AddPage { id, index, size });
        self.out.created.push(Created::Page { id, label });
        self.out.pages.added.push(id);
        Ok(())
    }

    fn move_page(&mut self, op: usize, page: &Ref, index: usize) -> Result<()> {
        let id = self.page_ref(page)?;
        let from = self.state.position(id)?;
        if index >= self.state.pages.len() {
            return Err(Error::invalid(
                op,
                format!("index {index} is past the last page"),
            ));
        }
        let slot = self.state.pages.remove(from);
        self.state.pages.insert(index, slot);
        self.out.writes.push(Write::MovePage { id, index });
        self.out.pages.reordered |= from != index;
        Ok(())
    }

    fn remove_page(&mut self, page: &Ref) -> Result<()> {
        let id = self.page_ref(page)?;
        self.state.page(id)?;
        let slot = self.state.pages.remove(self.state.position(id)?);
        let ids: Vec<LayerId> = slot
            .data
            .iter()
            .flat_map(|data| data.nodes.keys().copied())
            .collect();
        for node in slot.data.iter().flat_map(|data| data.nodes.values()) {
            self.files.extend(node.files());
        }
        self.forget(id, &ids);
        self.out.writes.push(Write::RemovePage(id));
        self.out.pages.changed.remove(&id);
        if slot.position.is_none() {
            self.out.pages.added.retain(|added| *added != id);
        } else {
            self.out.pages.removed.push(id);
        }
        Ok(())
    }

    /// Drops removed layers from the state and from this edit's actions.
    fn forget(&mut self, page: PageId, ids: &[LayerId]) {
        for id in ids {
            self.state.layers.to_mut().remove(id);
            self.state.forget_tiles(*id);
            self.actions.remove(&Target::Layer(*id));
            self.touch(page, *id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_layer(
        &mut self,
        op: usize,
        id: Option<&Ref>,
        page: &Ref,
        parent: Option<&Ref>,
        index: usize,
        layer: &Layer,
        ingredients: &[Read],
    ) -> Result<()> {
        let page = self.page_ref(page)?;
        let parent = match parent {
            Some(parent) => {
                let (parent, on) = self.layer_ref(parent)?;
                if on != page {
                    return Err(Error::invalid(
                        op,
                        format!("parent {parent} is on another page"),
                    ));
                }
                self.unlocked(op, parent)?;
                Some(parent)
            }
            None => None,
        };
        let kind = LayerKind::from(layer);
        // The round trip through JSON rejects what JSON can't hold, such as
        // a NaN from a Rust caller.
        let props = self.normalize(
            op,
            kind,
            serde_json::to_value(layer).expect("layers serialize"),
        )?;
        let text = match layer {
            Layer::Text(text) => Some(text.content.clone()),
            _ => None,
        };
        let (uuid, label) = self.new_id(op, id)?;
        let id = LayerId(uuid);
        let node = doc::Node {
            id,
            tree_id: None,
            parent,
            kind,
            props: props.clone(),
            text: text.clone(),
            generated: BTreeMap::new(),
            origin: Vec::new(),
            children: Vec::new(),
            masks: Vec::new(),
        };
        let index = place(self.state.page(page)?, node, index)
            .map_err(|reason| Error::invalid(op, reason))?;
        self.state.layers.to_mut().insert(id, page);
        let parent = parent.map(|parent| self.node_ref(parent)).transpose()?;
        self.out.writes.push(Write::AddNode {
            page,
            id,
            parent,
            index,
            kind,
            props,
            text,
        });
        self.out.created.push(Created::Layer { id, label });
        self.touch(page, id);
        self.record(op, Target::Layer(id), vec![PointerBuf::new()], ingredients)
    }

    fn duplicate_layer(
        &mut self,
        op: usize,
        layer: &Ref,
        id: Option<&Ref>,
        index: Option<usize>,
    ) -> Result<()> {
        self.user_only(op, "duplicate a layer")?;
        let (source, page) = self.layer_ref(layer)?;
        let source_node = self.state.node(source)?;
        let parent = source_node.parent;
        let mask = source_node.is_mask();
        if let Some(parent) = parent {
            self.unlocked(op, parent)?;
        }
        let list = siblings(self.state.page(page)?, parent, mask);
        let index = index.unwrap_or_else(|| {
            list.iter()
                .position(|id| *id == source)
                .map_or(0, |at| at + 1)
        });
        let (uuid, label) = self.new_id(op, id)?;
        let mut copies = vec![(source, LayerId(uuid), parent, Some(index))];
        let mut next = 0;
        while let Some(&(from, to, parent, index)) = copies.get(next) {
            let original = self.state.node(from)?.clone();
            for child in original.children.iter().chain(&original.masks) {
                copies.push((*child, LayerId::new(), Some(to), None));
            }
            let copy = doc::Node {
                id: to,
                tree_id: None,
                parent,
                children: Vec::new(),
                masks: Vec::new(),
                ..original.clone()
            };
            let data = self.state.page(page)?;
            let index = index.unwrap_or_else(|| siblings(data, parent, original.is_mask()).len());
            let tree_index =
                place(data, copy, index).map_err(|reason| Error::invalid(op, reason))?;
            self.state.layers.to_mut().insert(to, page);
            let parent = parent.map(|parent| self.node_ref(parent)).transpose()?;
            self.out.writes.push(Write::AddNode {
                page,
                id: to,
                parent,
                index: tree_index,
                kind: original.kind,
                props: original.props.clone(),
                text: original.text.as_ref().map(|_| RichText::default()),
            });
            if original.text.is_some() {
                let node = self.node_ref(from)?;
                let mut stores = vec![TextStore::Shown];
                if let Some(Generated::Text(_)) = original.generated.get(CONTENT) {
                    stores.push(TextStore::Kept);
                }
                for store in stores {
                    let from = TextFrom::Copy { node, store };
                    self.push_layer(page, to, LayerWrite::Text { store, from })?;
                }
            }
            let entries = generated_entries(&BTreeMap::new(), &original.generated);
            self.push_entries(page, to, entries)?;
            if !original.origin.is_empty() {
                let origin = Entry::origin(&original.origin);
                self.push_layer(page, to, LayerWrite::Set(origin))?;
            }
            if original.pixel_format().is_some() {
                let tiles = self.state.tiles(from)?.clone();
                self.out.writes.push(Write::Tiles {
                    page,
                    layer: to,
                    clear: false,
                    tiles: tiles
                        .iter()
                        .map(|(at, bytes)| (*at, Some(bytes.clone())))
                        .collect(),
                });
                *self.state.tiles(to)? = tiles;
            }
            self.out.created.push(Created::Layer {
                id: to,
                label: if next == 0 { label.clone() } else { None },
            });
            self.touch(page, to);
            next += 1;
        }
        Ok(())
    }

    fn move_layer(
        &mut self,
        op: usize,
        layer: &Ref,
        parent: Option<&Ref>,
        index: usize,
    ) -> Result<()> {
        let (id, page) = self.layer_ref(layer)?;
        self.unlocked(op, id)?;
        let parent = match parent {
            Some(parent) => {
                let (parent, on) = self.layer_ref(parent)?;
                if on != page {
                    return Err(Error::invalid(op, "a layer can only move within its page"));
                }
                if self.state.subtree(id)?.contains(&parent) {
                    return Err(Error::invalid(op, "a layer can't move inside itself"));
                }
                self.unlocked(op, parent)?;
                Some(parent)
            }
            None => None,
        };
        let data = self.state.page(page)?;
        let mut node = data.nodes.remove(&id).expect("loaded");
        siblings_mut(data, node.parent, node.is_mask()).retain(|sibling| *sibling != id);
        node.parent = parent;
        let node_ref = node.tree_id.map_or(NodeRef::New(id), NodeRef::Tree);
        let index = place(data, node, index).map_err(|reason| Error::invalid(op, reason))?;
        let parent = parent.map(|parent| self.node_ref(parent)).transpose()?;
        self.out.writes.push(Write::MoveNode {
            page,
            node: node_ref,
            parent,
            index,
        });
        self.touch(page, id);
        Ok(())
    }

    fn remove_layer(&mut self, op: usize, layer: &Ref) -> Result<()> {
        let (id, page) = self.layer_ref(layer)?;
        let ids = self.state.subtree(id)?;
        for each in &ids {
            if self.state.node(*each)?.locked() {
                return Err(Error::Locked { op, layer: *each });
            }
        }
        if let Some(parent) = self.state.node(id)?.parent {
            self.unlocked(op, parent)?;
        }
        if let Some(agent) = self
            .model()
            .map(|action| action.software_agent.name.clone())
        {
            // Only creating a layer writes all of it, and a rerun keeps that
            // write with the model's latest action. A layer created earlier in
            // this edit has its action in `actions` until `finish`.
            let origin = self.state.node(id)?.origin.clone();
            let created = origin
                .iter()
                .chain(self.actions.get(&Target::Layer(id)))
                .any(|record| {
                    record.action.software_agent.name == agent
                        && record.writes.iter().any(|path| path.is_root())
                });
            if !created {
                return Err(Error::invalid(
                    op,
                    "a model can only remove a layer it created",
                ));
            }
            for (position, each) in ids.iter().enumerate() {
                let node = self.state.node(*each)?;
                if !node.generated.is_empty() || (position > 0 && node.origin.is_empty()) {
                    self.out.kept.push(id);
                    return Ok(());
                }
            }
        }
        let node = self.node_ref(id)?;
        let data = self.state.page(page)?;
        let (parent, mask) = (data.nodes[&id].parent, data.nodes[&id].is_mask());
        siblings_mut(data, parent, mask).retain(|sibling| *sibling != id);
        let removed: Vec<doc::Node> = ids
            .iter()
            .filter_map(|each| data.nodes.remove(each))
            .collect();
        for node in &removed {
            self.files.extend(node.files());
            if node.pixel_format().is_some() {
                self.out.writes.push(Write::Tiles {
                    page,
                    layer: node.id,
                    clear: true,
                    tiles: Changes::new(),
                });
            }
        }
        self.forget(page, &ids);
        self.out.writes.push(Write::RemoveNode { page, node });
        Ok(())
    }

    fn update_layer(
        &mut self,
        op: usize,
        layer: &Ref,
        patch: &Value,
        ingredients: &[Read],
    ) -> Result<()> {
        let (id, page) = self.layer_ref(layer)?;
        let Value::Object(fields) = patch else {
            return Err(Error::invalid(op, "a patch must be an object"));
        };
        let node = self.state.node(id)?.clone();
        if fields.contains_key(Key::Kind.as_ref())
            || (node.kind == LayerKind::Text && fields.contains_key(Key::Content.as_ref()))
        {
            return Err(Error::invalid(
                op,
                "`kind` and a text's `content` can't be patched",
            ));
        }
        let merged = overrides::merge(
            &node.props,
            &node.generated,
            &node.origin,
            patch,
            self.model().is_some(),
            |value| self.normalize(op, node.kind, value),
        )?;
        if merged.changed.is_empty() {
            // A model's rerun with the same values still takes over what its
            // patch names, so its ingredients replace the old ones.
            return self.record(op, Target::Layer(id), named(fields), ingredients);
        }
        if merged.changed.iter().any(|path| path.as_str() != "/locked") {
            self.unlocked(op, id)?;
        }
        self.files.extend(node.files());
        let mut entries = generated_entries(&node.generated, &merged.generated);
        entries.extend(props_entries(&node.props, &merged.shown));
        self.push_entries(page, id, entries)?;
        let stored = self.state.node(id)?;
        stored.props = merged.shown;
        stored.generated = merged.generated;
        self.touch(page, id);
        self.record(op, Target::Layer(id), merged.changed, ingredients)
    }

    fn set_text(&mut self, op: usize, layer: &Ref, text: &str, ingredients: &[Read]) -> Result<()> {
        let (id, page) = self.text_layer(op, layer)?;
        self.unlocked(op, id)?;
        let kept = self.text_store(page, id)?;
        let node = self.state.node(id)?;
        match node.generated.get_mut(CONTENT) {
            Some(Generated::Text(model)) if kept => *model = RichText::from(text),
            _ => node.text = Some(RichText::from(text)),
        }
        self.push_layer(
            page,
            id,
            LayerWrite::Text {
                store: if kept {
                    TextStore::Kept
                } else {
                    TextStore::Shown
                },
                from: TextFrom::Plain(text.to_owned()),
            },
        )?;
        self.record(op, Target::Layer(id), vec![CONTENT.to_buf()], ingredients)
    }

    fn edit_text(&mut self, op: usize, layer: &Ref, delta: &[TextDelta]) -> Result<()> {
        self.user_only(op, "edit text with a delta")?;
        let (id, page) = self.text_layer(op, layer)?;
        if !self
            .edit
            .reads
            .iter()
            .any(|read| read.layer == id && read.path == CONTENT)
        {
            return Err(Error::invalid(
                op,
                "a text delta needs the edit to read the layer's /content",
            ));
        }
        self.unlocked(op, id)?;
        let current = self.state.node(id)?.text.clone().unwrap_or_default();
        let text =
            apply_delta(&current.text(), delta).map_err(|reason| Error::invalid(op, reason))?;
        self.text_store(page, id)?;
        self.state.node(id)?.text = Some(RichText::from(text.as_str()));
        self.push_layer(
            page,
            id,
            LayerWrite::Text {
                store: TextStore::Shown,
                from: TextFrom::Delta(delta.to_vec()),
            },
        )
    }

    fn text_layer(&mut self, op: usize, layer: &Ref) -> Result<(LayerId, PageId)> {
        let (id, page) = self.layer_ref(layer)?;
        if self.state.node(id)?.kind != LayerKind::Text {
            return Err(Error::invalid(
                op,
                format!("layer {id} is not a text layer"),
            ));
        }
        Ok((id, page))
    }

    /// Where a text write goes: a model's goes to its kept text if the user
    /// overrode it. Before the user's first edit of a model's text, the
    /// model's text is kept.
    fn text_store(&mut self, page: PageId, id: LayerId) -> Result<bool> {
        let model = self.model().is_some();
        let node = self.state.node(id)?;
        let overridden = node.generated.contains_key(CONTENT);
        if model {
            return Ok(overridden);
        }
        if !overridden && overrides::owned(&node.origin, CONTENT) {
            let text = Generated::Text(node.text.clone().unwrap_or_default());
            node.generated.insert(CONTENT.to_buf(), text);
            let node = self.node_ref(id)?;
            let from = TextFrom::Copy {
                node,
                store: TextStore::Shown,
            };
            let store = TextStore::Kept;
            self.push_layer(page, id, LayerWrite::Text { store, from })?;
        }
        Ok(false)
    }

    fn write_pixels(&mut self, op: usize, layer: &Ref, rect: Rect, pixels: &[u8]) -> Result<()> {
        let (id, page) = self.layer_ref(layer)?;
        let Some(format) = self.state.node(id)?.pixel_format() else {
            return Err(Error::invalid(op, format!("layer {id} has no pixels")));
        };
        let expected = rect.width as usize * rect.height as usize * tiles::bytes_per_pixel(format);
        if pixels.len() != expected {
            return Err(Error::invalid(
                op,
                format!(
                    "{} bytes of pixels, but the rectangle needs {expected}",
                    pixels.len()
                ),
            ));
        }
        self.unlocked(op, id)?;
        let tiles = tiles::write(self.state.tiles(id)?, format, rect, pixels)?;
        self.out.writes.push(Write::Tiles {
            page,
            layer: id,
            clear: false,
            tiles,
        });
        self.touch(page, id);
        Ok(())
    }

    fn reset(&mut self, op: usize, layer: &Ref, paths: Option<&[PointerBuf]>) -> Result<()> {
        self.user_only(op, "reset overrides")?;
        let (id, page) = self.layer_ref(layer)?;
        self.unlocked(op, id)?;
        let node = self.state.node(id)?.clone();
        let paths = paths.map_or_else(
            || node.generated.keys().cloned().collect(),
            <[PointerBuf]>::to_vec,
        );
        let mut shown = node.props.clone();
        let mut generated = node.generated.clone();
        let mut text = node.text.clone();
        for path in paths {
            match generated.remove(&path) {
                None => {
                    return Err(Error::Invalid {
                        op: Some(op),
                        path: Some(path),
                        reason: "nothing is overridden here".to_owned(),
                    });
                }
                Some(Generated::Text(model)) => {
                    text = Some(model);
                    let node = self.node_ref(id)?;
                    let from = TextFrom::Copy {
                        node,
                        store: TextStore::Kept,
                    };
                    let store = TextStore::Shown;
                    self.push_layer(page, id, LayerWrite::Text { store, from })?;
                }
                Some(Generated::Value(value)) => overrides::set(&mut shown, &path, value),
            }
            let write = LayerWrite::Set(Entry {
                generated: true,
                key: path.as_str().to_owned(),
                value: None,
            });
            self.push_layer(page, id, write)?;
        }
        let shown = self.normalize(op, node.kind, Value::Object(shown))?;
        self.files.extend(node.files());
        self.push_entries(page, id, props_entries(&node.props, &shown))?;
        let stored = self.state.node(id)?;
        stored.props = shown;
        stored.generated = generated;
        stored.text = text;
        self.touch(page, id);
        Ok(())
    }

    fn add_file(&mut self, op: usize, media_type: &str, bytes: &[u8]) -> Result<()> {
        if media_type.is_empty() {
            return Err(Error::invalid(op, "a file needs a media type"));
        }
        let hash = Hash::of(bytes);
        if self.state.files.to_mut().insert(hash.clone()) {
            self.out.writes.push(Write::AddFile {
                hash,
                media_type: media_type.to_owned(),
                bytes: bytes.to_vec(),
            });
        }
        Ok(())
    }

    fn add_style(
        &mut self,
        op: usize,
        id: Option<&Ref>,
        style: &NamedStyle,
        ingredients: &[Read],
    ) -> Result<()> {
        let props = normalize_style(op, serde_json::to_value(style).expect("styles serialize"))?;
        let (uuid, label) = self.new_id(op, id)?;
        let id = StyleId(uuid);
        let node = StyleNode {
            props: props.clone(),
            generated: BTreeMap::new(),
            origin: Vec::new(),
        };
        self.state.add_style(id, node);
        self.out.writes.push(Write::AddStyle { id, props });
        self.out.created.push(Created::Style { id, label });
        self.out.styles.insert(id);
        self.record(op, Target::Style(id), vec![PointerBuf::new()], ingredients)
    }

    fn update_style(
        &mut self,
        op: usize,
        style: &Ref,
        patch: &Value,
        ingredients: &[Read],
    ) -> Result<()> {
        let id = self.style_ref(style)?;
        let Value::Object(fields) = patch else {
            return Err(Error::invalid(op, "a patch must be an object"));
        };
        let node = self.state.style(id)?.clone();
        let merged = overrides::merge(
            &node.props,
            &node.generated,
            &node.origin,
            patch,
            self.model().is_some(),
            |value| normalize_style(op, value),
        )?;
        if merged.changed.is_empty() {
            return self.record(op, Target::Style(id), named(fields), ingredients);
        }
        let mut entries = generated_entries(&node.generated, &merged.generated);
        entries.extend(props_entries(&node.props, &merged.shown));
        self.push_style(id, entries);
        let stored = self.state.style(id)?;
        stored.props = merged.shown;
        stored.generated = merged.generated;
        self.record(op, Target::Style(id), merged.changed, ingredients)
    }

    fn reset_style(&mut self, op: usize, style: &Ref, paths: Option<&[PointerBuf]>) -> Result<()> {
        self.user_only(op, "reset overrides")?;
        let id = self.style_ref(style)?;
        let node = self.state.style(id)?.clone();
        let paths = paths.map_or_else(
            || node.generated.keys().cloned().collect(),
            <[PointerBuf]>::to_vec,
        );
        let mut shown = node.props.clone();
        let mut generated = node.generated.clone();
        for path in paths {
            match generated.remove(&path) {
                Some(Generated::Value(value)) => overrides::set(&mut shown, &path, value),
                _ => {
                    return Err(Error::Invalid {
                        op: Some(op),
                        path: Some(path),
                        reason: "nothing is overridden here".to_owned(),
                    });
                }
            }
        }
        let shown = normalize_style(op, Value::Object(shown))?;
        let mut entries = generated_entries(&node.generated, &generated);
        entries.extend(props_entries(&node.props, &shown));
        self.push_style(id, entries);
        let stored = self.state.style(id)?;
        stored.props = shown;
        stored.generated = generated;
        Ok(())
    }

    /// A style a text names stays, so no text loses its look silently.
    fn remove_style(&mut self, op: usize, style: &Ref) -> Result<()> {
        self.user_only(op, "remove a text style")?;
        let id = self.style_ref(style)?;
        if self.state.styles_in_use()?.contains(&id) {
            return Err(Error::invalid(op, format!("a text names style {id}")));
        }
        self.state.remove_style(id);
        self.actions.remove(&Target::Style(id));
        self.out.writes.push(Write::RemoveStyle(id));
        self.out.styles.insert(id);
        Ok(())
    }

    /// That a named file or text style exists.
    fn check_names(&self, op: usize, layer: &Layer) -> Result<()> {
        match layer {
            Layer::Image(image) if !self.state.files.contains(&image.file) => Err(Error::invalid(
                op,
                format!("file {} was not added", image.file),
            )),
            Layer::Text(text) => match text.text_style {
                Some(style) if !self.state.styles.contains(&style) => {
                    Err(Error::invalid(op, format!("style {style} doesn't exist")))
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// Deserializes properties into their kind's struct and returns them as
    /// stored: everything but `kind` and a text's `content`.
    fn normalize(&self, op: usize, kind: LayerKind, value: Value) -> Result<Map<String, Value>> {
        let Value::Object(mut map) = value else {
            return Err(Error::invalid(op, "a layer must be an object"));
        };
        map.insert(Key::Kind.as_ref().to_owned(), kind.as_ref().into());
        let layer: Layer = serde_json::from_value(Value::Object(map))
            .map_err(|error| Error::invalid(op, error.to_string()))?;
        self.check_names(op, &layer)?;
        let Value::Object(mut props) = serde_json::to_value(&layer).expect("layers serialize")
        else {
            unreachable!("a layer serializes to an object")
        };
        props.remove(Key::Kind.as_ref());
        if kind == LayerKind::Text {
            props.remove(Key::Content.as_ref());
        }
        Ok(props)
    }

    fn finish(mut self, original: &Index) -> Result<Planned> {
        for (target, action) in std::mem::take(&mut self.actions) {
            match target {
                Target::Layer(id) => {
                    let page = self.state.page_of(id)?;
                    let node = self.state.node(id)?;
                    overrides::take_over(&mut node.origin, action);
                    let origin = Entry::origin(&node.origin);
                    self.push_layer(page, id, LayerWrite::Set(origin))?;
                }
                Target::Style(id) => {
                    let node = self.state.style(id)?;
                    overrides::take_over(&mut node.origin, action);
                    let origin = Entry::origin(&node.origin);
                    self.push_style(id, vec![origin]);
                }
            }
        }
        if !self.files.is_empty() {
            let used = self.state.files_in_use()?;
            for hash in std::mem::take(&mut self.files) {
                if !used.contains(&hash) && self.state.files.to_mut().remove(&hash) {
                    self.out.writes.push(Write::RemoveFile(hash));
                }
            }
        }
        self.out.index = self.state.index(original);
        Ok(self.out)
    }
}

/// The top-level paths a patch names.
fn named(fields: &Map<String, Value>) -> Vec<PointerBuf> {
    fields
        .keys()
        .map(|key| Pointer::root().with_trailing_token(key))
        .collect()
}

/// Checks a text style's values and returns them as stored.
fn normalize_style(op: usize, value: Value) -> Result<Map<String, Value>> {
    let style: NamedStyle =
        serde_json::from_value(value).map_err(|error| Error::invalid(op, error.to_string()))?;
    match serde_json::to_value(style).expect("styles serialize") {
        Value::Object(map) => Ok(map),
        _ => unreachable!("a style serializes to an object"),
    }
}

/// The list a layer sits in: its parent's drawn children or masks, or the
/// page's top level.
fn siblings(data: &PageData, parent: Option<LayerId>, mask: bool) -> &[LayerId] {
    match parent {
        None => &data.roots,
        Some(parent) if mask => &data.nodes[&parent].masks,
        Some(parent) => &data.nodes[&parent].children,
    }
}

fn siblings_mut(data: &mut PageData, parent: Option<LayerId>, mask: bool) -> &mut Vec<LayerId> {
    match parent {
        None => &mut data.roots,
        Some(parent) => {
            let node = data.nodes.get_mut(&parent).expect("loaded");
            if mask {
                &mut node.masks
            } else {
                &mut node.children
            }
        }
    }
}

/// Puts `node` at `index` in its list and returns its position among all
/// the parent's tree children, where masks follow the drawn children.
fn place(data: &mut PageData, node: doc::Node, index: usize) -> Result<usize, String> {
    let (parent, mask) = (node.parent, node.is_mask());
    if mask && parent.is_none() {
        return Err("a mask needs a parent layer".to_owned());
    }
    if parent.is_some_and(|parent| data.nodes[&parent].is_mask()) {
        return Err("a mask can't hold layers".to_owned());
    }
    let list = siblings_mut(data, parent, mask);
    if index > list.len() {
        return Err(format!("index {index} is past the end ({})", list.len()));
    }
    list.insert(index, node.id);
    data.nodes.insert(node.id, node);
    Ok(match parent {
        Some(parent) if mask => index + data.nodes[&parent].children.len(),
        _ => index,
    })
}

/// Applies a delta to plain text, checking that it stays in bounds and uses
/// known marks.
fn apply_delta(text: &str, delta: &[TextDelta]) -> Result<String, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut position = 0;
    for item in delta {
        let attributes = match item {
            TextDelta::Retain { retain, attributes } => {
                let end = position + retain;
                out.extend(
                    chars
                        .get(position..end)
                        .ok_or("the delta runs past the end of the text")?,
                );
                position = end;
                attributes
            }
            TextDelta::Insert { insert, attributes } => {
                out.push_str(insert);
                attributes
            }
            TextDelta::Delete { delete } => {
                position += delete;
                if position > chars.len() {
                    return Err("the delta deletes past the end of the text".to_owned());
                }
                &None
            }
        };
        let marks: Map<String, Value> = attributes
            .iter()
            .flatten()
            .map(|(key, value)| (key.clone(), doc::json(value.clone())))
            .collect();
        doc::style_of(marks).map_err(|error| error.to_string())?;
    }
    out.extend(&chars[position..]);
    Ok(out)
}

/// Writes a planned edit. Any failure here leaves the document partly
/// written, so the caller poisons the session.
pub(crate) fn execute(doc: &LoroDoc, writes: Vec<Write>) -> Result<()> {
    let mut pages: HashMap<PageId, (LoroMap, LoroTree)> = HashMap::new();
    let mut created: HashMap<LayerId, TreeID> = HashMap::new();
    for write in writes {
        match write {
            Write::AddFile {
                hash,
                media_type,
                bytes,
            } => doc::add_file(doc, &hash, &media_type, bytes)?,
            Write::RemoveFile(hash) => doc::remove_file(doc, &hash)?,
            Write::AddPage { id, index, size } => {
                let map = doc::add_page(doc, index, id, size)?;
                let tree = doc::tree(&map)?;
                pages.insert(id, (map, tree));
            }
            Write::MovePage { id, index } => {
                let (from, _) = doc::find_page(doc, id)?;
                doc::pages(doc).mov(from, index)?;
            }
            Write::RemovePage(id) => {
                let (at, _) = doc::find_page(doc, id)?;
                doc::pages(doc).delete(at, 1)?;
                pages.remove(&id);
            }
            Write::AddNode {
                page,
                id,
                parent,
                index,
                kind,
                props,
                text,
            } => {
                let (_, tree) = page_of(doc, &mut pages, page)?;
                let parent = parent.map(|parent| resolve(&created, parent));
                let node = tree.create_at(parent, index)?;
                doc::write_node(&tree.get_meta(node)?, id, kind, props, text.as_ref())?;
                created.insert(id, node);
            }
            Write::MoveNode {
                page,
                node,
                parent,
                index,
            } => {
                let (_, tree) = page_of(doc, &mut pages, page)?;
                let parent = parent.map(|parent| resolve(&created, parent));
                tree.mov_to(resolve(&created, node), parent, index)?;
            }
            Write::RemoveNode { page, node } => {
                page_of(doc, &mut pages, page)?
                    .1
                    .delete(resolve(&created, node))?;
            }
            Write::Layer { page, node, write } => {
                let (_, tree) = page_of(doc, &mut pages, page)?;
                let meta = tree.get_meta(resolve(&created, node))?;
                match write {
                    LayerWrite::Set(entry) => entry.write(meta)?,
                    LayerWrite::Text { store, from } => {
                        let target = match store {
                            TextStore::Shown => doc::content(&meta)?,
                            // Made fresh when missing, so a deleted kept
                            // text never comes back.
                            TextStore::Kept => {
                                let generated = doc::generated(&meta)?;
                                match doc::text(&generated, CONTENT.as_str()) {
                                    Ok(text) => text,
                                    Err(_) => generated
                                        .insert_container(CONTENT.as_str(), LoroText::new())?,
                                }
                            }
                        };
                        match from {
                            TextFrom::Plain(text) => target
                                .update(&text, UpdateOptions::default())
                                .map_err(|_| Error::corrupt("a text update timed out"))?,
                            TextFrom::Delta(delta) => target.apply_delta(&delta)?,
                            TextFrom::Copy { node, store } => {
                                let source = tree.get_meta(resolve(&created, node))?;
                                let source = match store {
                                    TextStore::Shown => doc::content(&source)?,
                                    TextStore::Kept => {
                                        doc::text(&doc::generated(&source)?, CONTENT.as_str())?
                                    }
                                };
                                target.delete(0, target.len_unicode())?;
                                target.apply_delta(&source.to_delta())?;
                            }
                        }
                    }
                }
            }
            Write::AddStyle { id, props } => doc::add_style(doc, id, props)?,
            Write::Style { id, entry } => entry.write(doc::style_map(doc, id)?)?,
            Write::RemoveStyle(id) => doc::remove_style(doc, id)?,
            Write::Tiles {
                page,
                layer,
                clear,
                tiles,
            } => {
                let (map, _) = page_of(doc, &mut pages, page)?;
                let store = doc::tile_store(map)?;
                let key = layer.to_string();
                if clear && store.get(&key).is_some() {
                    store.delete(&key)?;
                }
                if !tiles.is_empty() {
                    let map = doc::child_map(&store, &key)?;
                    for (at, bytes) in tiles {
                        match bytes {
                            Some(bytes) => map.insert(&tiles::key(at), LoroValue::Binary(bytes))?,
                            None => map.delete(&tiles::key(at))?,
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn page_of<'a>(
    doc: &LoroDoc,
    pages: &'a mut HashMap<PageId, (LoroMap, LoroTree)>,
    page: PageId,
) -> Result<&'a (LoroMap, LoroTree)> {
    Ok(match pages.entry(page) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            let (_, map) = doc::find_page(doc, page)?;
            let tree = doc::tree(&map)?;
            entry.insert((map, tree))
        }
    })
}

fn resolve(created: &HashMap<LayerId, TreeID>, node: NodeRef) -> TreeID {
    match node {
        NodeRef::Tree(tree_id) => tree_id,
        NodeRef::New(id) => created[&id],
    }
}
