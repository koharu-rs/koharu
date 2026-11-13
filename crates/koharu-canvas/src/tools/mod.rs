//! Each tool's presses and the gestures they start.

pub(crate) mod brush;
pub(crate) mod pan;
pub(crate) mod picker;
pub(crate) mod select;
pub(crate) mod text;
pub(crate) mod transform;

use koharu_scene::{Attributes, Content, LayerId, NewLayer, Op, Opacity, Page, Ref, Uuid};
use ui_events::keyboard::Modifiers;

/// A new layer of `content` at the top of `page`, under an id the canvas
/// picks, so it can select the layer with the edit that makes it, as
/// Photoshop and Clip Studio Paint select a layer they make.
pub(crate) fn added(page: &Page, content: Content) -> (LayerId, Op) {
    let id = LayerId(Uuid::now_v7());
    let op = Op::AddLayer {
        id: Some(Ref::from(id)),
        page: Ref::from(page.id),
        parent: None,
        index: page.layers.len(),
        layer: Box::new(NewLayer {
            name: String::new(),
            visible: true,
            locked: false,
            opacity: Opacity::OPAQUE,
            blend: None,
            attributes: Attributes::new(),
            content,
        }),
    };
    (id, op)
}

/// Ctrl, or Command on macOS.
pub(crate) fn command(modifiers: Modifiers) -> bool {
    modifiers.ctrl() || modifiers.meta()
}
