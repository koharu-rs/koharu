//! A layer: the values every layer has, and its kind's.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{Blend, Content, LayerId, LayerKind, Opacity, Uuid};

/// Namespace, then key, then value. The renderer never reads attributes.
pub type Attributes = BTreeMap<String, BTreeMap<String, Value>>;

/// A layer as the project holds it, which is also how it's stored and sent,
/// with everything inside it. `C` is what it holds: any kind for a layer in
/// the tree, or one fixed kind for a block's mask and clean-up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Layer<C = Content> {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: Opacity,
    pub blend: Blend,
    /// Written only by `set_attribute`.
    pub attributes: Attributes,
    #[serde(flatten)]
    pub content: C,
    /// Drawn first to last, as an OpenRaster stack holds its layers. The masks
    /// among them, as Krita keeps masks as child nodes, apply in order and are
    /// never drawn. A block's parts hold none.
    #[serde(default)]
    #[schemars(skip)]
    pub children: Vec<Layer>,
}

/// What `add_layer` takes, with every default a new layer gets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct NewLayer {
    #[serde(default)]
    pub name: String,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub opacity: Opacity,
    /// None gives a group, a panel, a balloon, or a block pass-through, and
    /// any other layer normal.
    #[serde(default)]
    pub blend: Option<Blend>,
    #[serde(default)]
    pub attributes: Attributes,
    #[serde(flatten)]
    pub content: Content,
}

fn visible() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Rgba8,
    Gray8,
}

/// A block's part as a new block gets it: visible, opaque, and empty.
pub(crate) fn part<C: Default>() -> Layer<C> {
    Layer {
        id: LayerId(Uuid::now_v7()),
        name: String::new(),
        visible: true,
        locked: false,
        opacity: Opacity::OPAQUE,
        blend: Blend::Normal,
        attributes: Attributes::new(),
        content: C::default(),
        children: Vec::new(),
    }
}

impl Layer {
    pub(crate) fn new(id: LayerId, new: NewLayer) -> Self {
        let container = matches!(
            new.content,
            Content::Group(_) | Content::Panel(_) | Content::Balloon(_) | Content::Block(_)
        );
        let blend = match new.blend {
            Some(blend) => blend,
            None if container => Blend::PassThrough,
            None => Blend::Normal,
        };
        Self {
            id,
            name: new.name,
            visible: new.visible,
            locked: new.locked,
            opacity: new.opacity,
            blend,
            attributes: new.attributes,
            content: new.content,
            children: Vec::new(),
        }
    }

    /// This layer's own values, without its children. Naming every field
    /// fails to build when one is added, until it's copied here too.
    pub(crate) fn own(&self) -> Self {
        let Self {
            id,
            name,
            visible,
            locked,
            opacity,
            blend,
            attributes,
            content,
            children: _,
        } = self;
        Self {
            id: *id,
            name: name.clone(),
            visible: *visible,
            locked: *locked,
            opacity: *opacity,
            blend: *blend,
            attributes: attributes.clone(),
            content: content.clone(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> LayerKind {
        LayerKind::from(&self.content)
    }

    #[must_use]
    pub fn is_mask(&self) -> bool {
        self.kind() == LayerKind::Mask
    }
}
