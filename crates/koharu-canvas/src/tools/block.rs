//! The text block tool: a block over the box a drag draws, for a text
//! detection missed.

use koharu_scene::{Edit, Layer, LayerKind, Op, Rect, Ref};
use kurbo::{Point, Shape as _};
use serde_json::json;

use super::{added, text::Draft};
use crate::{Effect, gesture::Context};

/// The step adding the block: its source's frame and its mask are the box,
/// and it goes in the balloon under the box's middle, else in the panel, else
/// on the page. A click adds none.
pub(crate) fn finish(draft: &Draft, cx: &Context) -> anyhow::Result<Vec<Effect>> {
    let (page, size, drawn) = (cx.snapshot.page(), cx.snapshot.size(), draft.rect());
    let Some(rect) =
        Rect::covering(drawn, size.width as u32, size.height as u32).filter(|_| !draft.clicked(cx))
    else {
        return Ok(Vec::new());
    };
    let content = serde_json::from_value(json!({
        "kind": "block",
        "source": {
            "frame": {"box": {"bounds": rect}},
            "direction": if rect.height > rect.width { "vertical" } else { "horizontal" },
        },
    }))?;
    let holder = [LayerKind::Balloon, LayerKind::Panel]
        .into_iter()
        .find_map(|kind| under(cx, &page.layers, kind, drawn.center()));
    let (id, add) = added(page, holder, content);
    let mask = Op::WritePixels {
        layer: Ref::try_from(format!("${}.mask", id.0)).map_err(anyhow::Error::msg)?,
        rect,
        pixels: vec![255; rect.width as usize * rect.height as usize],
    };
    let edit = Edit {
        label: "add_text".to_owned(),
        ops: vec![add, mask],
    };
    Ok(vec![Effect::Edit(edit), Effect::Selection(vec![id])])
}

/// The topmost unlocked layer of `kind` whose outline holds `at`.
fn under<'l>(cx: &Context, layers: &'l [Layer], kind: LayerKind, at: Point) -> Option<&'l Layer> {
    layers
        .iter()
        .rev()
        .filter(|layer| !layer.locked)
        .find_map(|layer| {
            under(cx, &layer.children, kind, at).or_else(|| {
                let outline = cx.snapshot.outline(layer.id);
                (layer.kind() == kind && outline.is_some_and(|outline| outline.contains(at)))
                    .then_some(layer)
            })
        })
}
