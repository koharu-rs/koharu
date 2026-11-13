use koharu_ml::koharu_layout_highres::rooms;
use koharu_scene::{Content, Layer, LayerId, Role};
use kurbo::{BezPath, Shape as _};

use crate::{Error, Result};

/// `block`'s room in a balloon with this `outline` holding `children` on a
/// page of `layers`: the layout model's split of the outline among the source
/// boxes of the texts standing in it. Those are its own and any other whose
/// box reaches in, such as a remark written over its edge, which a later
/// layer's clean-up would otherwise cover the balloon's lines with.
pub(super) fn room(
    outline: &koharu_scene::Path,
    children: &[Layer],
    layers: &[Layer],
    block: LayerId,
) -> Result<Option<BezPath>> {
    let outline = BezPath::from_svg(outline.as_str())
        .map_err(|_| Error::invalid("a balloon's outline isn't a path"))?;
    let mut texts = Vec::new();
    collect_texts(children, false, &mut texts);
    let at = texts.iter().position(|(id, _)| *id == block);
    let own = texts.len();
    let reach = outline.bounding_box();
    collect_texts(layers, true, &mut texts);
    let groups: Vec<Vec<kurbo::Rect>> = texts
        .iter()
        .enumerate()
        .filter(|(index, (id, rect))| {
            *index < own
                || (!texts[..own].iter().any(|(mine, _)| mine == id)
                    && rect.intersect(reach).area() > 0.0)
        })
        .map(|(_, (_, rect))| vec![*rect])
        .collect();
    Ok(at.and_then(|at| rooms(&outline, &groups).into_iter().nth(at)))
}

/// The text blocks among `layers`, and among their children when `deep`,
/// with where their sources lie.
fn collect_texts(layers: &[Layer], deep: bool, found: &mut Vec<(LayerId, kurbo::Rect)>) {
    for layer in layers {
        if let Content::Block(text) = &layer.content
            && text.role == Role::Text
            && let Some(rect) = text.source.bounds()
        {
            found.push((layer.id, rect));
        }
        if deep {
            collect_texts(&layer.children, deep, found);
        }
    }
}
