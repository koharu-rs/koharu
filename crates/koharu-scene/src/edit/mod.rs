//! Edits. An edit's ops run in order on a copy of the project, so one that
//! fails leaves the project as it was.

mod layers;
mod pages;
mod pixels;
mod schema;
mod text;

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{
    Background, Content, Created, Error, Hash, Layer, LayerId, NewLayer, PageId, Project, Rect,
    Ref, Result, Size, Span, Text, Uuid, store::Tx,
};

/// Where an op found a layer: its page and its path there. It holds only
/// until the op changes the page's tree.
pub(crate) struct Place {
    pub id: LayerId,
    pub page: PageId,
    pub path: Vec<usize>,
}

impl Place {
    /// The path of the list holding the layer, and its index there.
    fn holder(&self) -> (&[usize], usize) {
        let (at, list) = self.path.split_last().expect("a path names a layer");
        (list, *at)
    }
}

/// One change to the project, committed as one undo step, from whoever made
/// it: the editor, the pipeline, or an agent.
#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    /// The undo menu label.
    pub label: String,
    pub ops: Vec<Op>,
}

/// One op of an edit. Variants skipped by schemars carry bytes or the rich
/// editor's spans, which tools never send.
#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Add a page at `index`: of `size`, or made from an added file, which
    /// becomes its background and gives it its size.
    #[schemars(skip)]
    AddPage {
        #[serde(default)]
        id: Option<Ref>,
        index: usize,
        #[serde(default)]
        size: Option<Size>,
        #[serde(default)]
        background: Option<Hash>,
    },
    /// Set or clear a page's background. A new file must be an image of the
    /// page's size.
    SetBackground {
        page: Ref,
        background: Option<Background>,
    },
    /// Move a page to `index`, counted after the page is taken out.
    MovePage { page: Ref, index: usize },
    /// Remove a page with its layers.
    RemovePage { page: Ref },
    /// Add a layer under `parent`, or at the top level of `page`, at `index`
    /// among the parent's children, masks included, first drawn first.
    AddLayer {
        #[serde(default)]
        id: Option<Ref>,
        page: Ref,
        #[serde(default)]
        parent: Option<Ref>,
        index: usize,
        layer: Box<NewLayer>,
    },
    /// Copy a layer with everything inside it, its pixels, text, and
    /// attributes. The copy goes just above the original unless `index` says
    /// where.
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
    /// Change a layer's values with a JSON Merge Patch (RFC 7396): `null`
    /// clears an optional value, and `{"translation": null}` removes a
    /// block's translation.
    UpdateLayer {
        layer: Ref,
        #[schemars(schema_with = "schema::layer_patch")]
        patch: Value,
    },
    /// Replace a text layer's text, or a block's translation, adding one if
    /// the block has none. Styling on unchanged characters is kept.
    SetText { layer: Ref, text: String },
    /// Replace a text's spans, from the rich editor.
    #[schemars(skip)]
    SetRichText { layer: Ref, spans: Vec<Span> },
    /// Set one attribute of a layer, or remove it when `value` is null.
    /// Attributes are data for tools: provenance goes under `c2pa`, by step.
    SetAttribute {
        layer: Ref,
        namespace: String,
        key: String,
        #[serde(default)]
        value: Option<Value>,
    },
    /// Replace pixels of a layer's picture, in its own pixel coordinates:
    /// RGBA8 for a raster or a block's `cleanup`, grey8 for a mask or a
    /// block's `mask`, each named by its own id. A block's parts take the
    /// labels `$label.mask` and `$label.cleanup` from the block's.
    #[schemars(skip)]
    WritePixels {
        layer: Ref,
        rect: Rect,
        pixels: Vec<u8>,
    },
    /// Add an imported file, stored once by its content hash.
    #[schemars(skip)]
    AddFile { bytes: Vec<u8> },
}

pub(crate) struct Run<'a, 'c> {
    pub project: Project,
    pub tx: &'a Tx<'c>,
    labels: HashMap<String, Uuid>,
    pub created: Vec<Created>,
}

/// Runs `edit` on a copy of `project`, inside the caller's transaction, which
/// holds what blobs the ops write.
pub(crate) fn run(project: &Project, edit: &Edit, tx: &Tx) -> Result<(Project, Vec<Created>)> {
    let mut run = Run {
        project: project.clone(),
        tx,
        labels: HashMap::new(),
        created: Vec::new(),
    };
    for (index, op) in edit.ops.iter().enumerate() {
        run.op(index, op)?;
    }
    Ok((run.project, run.created))
}

impl Run<'_, '_> {
    /// A new id, given or labelled.
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
        if self.project.pages.contains_key(&PageId(uuid))
            || self.project.index.contains_key(&LayerId(uuid))
        {
            return Err(Error::invalid(op, format!("id {uuid} already exists")));
        }
        Ok((uuid, None))
    }

    /// Gives a block's parts new ids, listed as created and labelled
    /// `$label.mask` and `$label.cleanup` when the block has a label, and
    /// returns each part's old id with its new one.
    fn number_parts(
        &mut self,
        op: usize,
        layer: &mut Layer,
        label: Option<&str>,
    ) -> Result<Vec<(LayerId, LayerId)>> {
        let Content::Block(block) = &mut layer.content else {
            return Ok(Vec::new());
        };
        let mut renamed = Vec::new();
        for (part, id) in [
            ("mask", &mut block.mask.id),
            ("cleanup", &mut block.cleanup.id),
        ] {
            let new = LayerId(Uuid::now_v7());
            renamed.push((*id, new));
            *id = new;
            let label = label.map(|label| format!("{label}.{part}"));
            if let Some(label) = &label {
                if self.labels.contains_key(label) {
                    return Err(Error::invalid(
                        op,
                        format!("label ${label} is already used in this edit"),
                    ));
                }
                self.labels.insert(label.clone(), new.0);
            }
            self.created.push(Created::Layer { id: new, label });
        }
        Ok(renamed)
    }

    fn resolve(&self, id: &Ref) -> Result<Uuid> {
        match id.label() {
            Some(label) => self
                .labels
                .get(label)
                .copied()
                .ok_or_else(|| Error::NotFound(format!("label {id}"))),
            None => Ok(id.uuid().expect("a ref is a label or a UUID")),
        }
    }

    fn page(&self, id: &Ref) -> Result<PageId> {
        Ok(self.project.page(PageId(self.resolve(id)?))?.id)
    }

    fn place(&self, id: &Ref) -> Result<Place> {
        let id = LayerId(self.resolve(id)?);
        let (page, path) = self
            .project
            .place(id)
            .ok_or_else(|| Error::NotFound(format!("layer {id}")))?;
        Ok(Place { id, page, path })
    }

    fn get(&self, place: &Place) -> &Layer {
        self.project.pages[&place.page].at(&place.path)
    }

    /// The layer holding the one at `place`, if it isn't at the top of its
    /// page.
    fn parent(&self, place: &Place) -> Option<LayerId> {
        let (list, _) = place.holder();
        (!list.is_empty()).then(|| self.project.pages[&place.page].at(list).id)
    }

    /// That `layer` and every layer above it are unlocked.
    fn unlocked(&self, op: usize, layer: LayerId) -> Result<()> {
        match self.project.locked_at(layer) {
            Some(layer) => Err(Error::Locked { op, layer }),
            None => Ok(()),
        }
    }

    /// Readies a layer an op wrote: each text's spans are joined however the
    /// op wrote them. A value JSON can't hold, such as a NaN from a Rust
    /// caller, would leave a file that can't open, so it fails here.
    fn settle(op: usize, layer: &mut Layer) -> Result<()> {
        let texts: Vec<&mut Text> = match &mut layer.content {
            Content::Text(text) => vec![text],
            Content::Block(block) => std::iter::once(&mut block.source)
                .chain(block.translation.as_mut())
                .collect(),
            _ => Vec::new(),
        };
        for text in texts {
            text.content = crate::text::joined(std::mem::take(&mut text.content.0));
        }
        let json = serde_json::to_value(layer.own()).expect("layers serialize");
        if serde_json::from_value::<Layer>(json).is_err() {
            return Err(Error::invalid(
                op,
                format!("layer {} holds a value JSON can't store", layer.id),
            ));
        }
        Ok(())
    }

    /// Writes `layer`'s own values over the layer at `place`, the one way an
    /// op changes a layer in its page; the layer keeps its children.
    fn put(&mut self, op: usize, place: &Place, mut layer: Layer) -> Result<()> {
        Self::settle(op, &mut layer)?;
        let stored = self
            .project
            .pages
            .get_mut(&place.page)
            .expect("an op's page exists")
            .at_mut(&place.path);
        layer.children = std::mem::take(&mut stored.children);
        *stored = layer;
        Ok(())
    }

    fn op(&mut self, index: usize, op: &Op) -> Result<()> {
        match op {
            Op::AddPage {
                id,
                index: at,
                size,
                background,
            } => self.add_page(index, id.as_ref(), *at, *size, background.as_ref()),
            Op::SetBackground { page, background } => {
                self.set_background(index, page, background.as_ref())
            }
            Op::MovePage { page, index: to } => self.move_page(index, page, *to),
            Op::RemovePage { page } => self.remove_page(page),
            Op::AddLayer {
                id,
                page,
                parent,
                index: at,
                layer,
            } => self.add_layer(index, id.as_ref(), page, parent.as_ref(), *at, layer),
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
            Op::UpdateLayer { layer, patch } => self.update_layer(index, layer, patch),
            Op::SetText { layer, text } => self.set_text(index, layer, text),
            Op::SetRichText { layer, spans } => self.set_rich_text(index, layer, spans),
            Op::SetAttribute {
                layer,
                namespace,
                key,
                value,
            } => self.set_attribute(index, layer, namespace, key, value.as_ref()),
            Op::WritePixels {
                layer,
                rect,
                pixels,
            } => self.write_pixels(index, layer, *rect, pixels),
            Op::AddFile { bytes } => self.add_file(bytes),
        }
    }
}
