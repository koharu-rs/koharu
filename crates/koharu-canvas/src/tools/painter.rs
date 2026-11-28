//! The Format Painter, InDesign's Eyedropper for type. A press
//! on a text tells the host, which holds the copied style and pastes it, as
//! the colour picker's colour goes to the host's brush.

use koharu_scene::LayerId;
use kurbo::Point;

use crate::{
    Effect, PainterPress,
    canvas::text_at,
    gesture::{Context, Press},
    view::Sample,
};

/// The text under `sample`, a block's translation among them, with Alt to
/// copy its style afresh; nothing off a text.
pub(crate) fn press(cx: &Context, sample: Sample) -> Press {
    let Some((layer, _)) = text_at(cx, sample.page) else {
        return Press::default();
    };
    Press::answers(vec![Effect::Painter(PainterPress {
        layer,
        reload: sample.modifiers.alt(),
    })])
}

/// The text a press would take, outlined as the select tool's hover is.
pub(crate) fn hover(cx: &Context, page: Point) -> Option<LayerId> {
    text_at(cx, page).map(|(layer, _)| layer)
}
