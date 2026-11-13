//! Where a block's translation goes. Translation writes it, and the editor asks
//! again when a balloon or a source frame changes, so the text reflows.

mod room;

use icu_locale_core::{LanguageIdentifier, subtags::language};
use koharu_scene::{
    Align, Content, Direction, Frame, Layer, LayerId, Op, Project, Role, Transform,
};
use kurbo::{Affine, Shape as _};
use serde_json::{Value, json};

use crate::{Error, Result};

/// The op placing `block`'s translation into `target`: in its room
/// in its balloon, the share of the outline its source's box gets beside the
/// other texts standing in the balloon, or in its source's frame when it's
/// free or a sound effect. It fills its frame at every render, centred on the
/// frame's block axis. A new translation starts at the page's lettering
/// size, the median of its texts' sources, as a letterer keeps one size for a
/// page's dialogue and fits each balloon from it; its lines centred, as a
/// balloon's are lettered; and with its source's outline as its stroke, the
/// width scaled from the source's size to its own. What its style already
/// sets is kept. It is vertical only when its source is and `target` is
/// Japanese or Chinese, which are written so; any other language runs
/// horizontally, turned by the source's tilt as the source is.
pub fn placement(project: &Project, block: LayerId, target: &LanguageIdentifier) -> Result<Op> {
    let (page, holder) = project.parent(block)?;
    let Content::Block(this) = &project.layer(block)?.content else {
        return Err(Error::invalid(format!("layer {block} isn't a block")));
    };
    let room = match holder {
        Some(Layer {
            content: Content::Balloon(balloon),
            children,
            ..
        }) if this.role == Role::Text => {
            room::room(&balloon.outline, children, &page.layers, block)?
        }
        _ => None,
    };
    let (frame, transform) = match room {
        Some(room) => {
            // The source's tilt about the room's centre, and the room turned
            // back by it into the text's own coordinates, which the turn
            // takes onto the page again.
            let [[a, _, _], [d, _, _]] = this.source.transform.0;
            let turn = Affine::rotate_about(d.atan2(a), room.bounding_box().center());
            (
                json!({"path": {"path": (turn.inverse() * room).to_svg(), "fit": "best", "anchor": "center"}, "box": null}),
                json!(Transform::from(turn)),
            )
        }
        None => (filled(&this.source.frame), json!(this.source.transform)),
    };
    let given = this
        .translation
        .as_ref()
        .map(|translation| &translation.style);
    let sized = given.and_then(|style| style.chars.size);
    let size = sized.or_else(|| lettering(&page.layers));
    let vertical = this.source.direction == Direction::Vertical
        && [language!("ja"), language!("zh")].contains(&target.language);
    let mut translation = json!({
        "frame": frame,
        "transform": transform,
        "direction": if vertical { "vertical" } else { "horizontal" },
    });
    let mut chars = serde_json::Map::new();
    if let (None, Some(size)) = (sized, size) {
        chars.insert("size".to_owned(), json!(size));
    }
    let source = &this.source.style.chars;
    if given.and_then(|style| style.chars.stroke).is_none()
        && let (Some(stroke), Some(width)) = (source.stroke, source.stroke_width)
    {
        let scale = size.zip(source.size).map_or(1.0, |(to, from)| to / from);
        chars.insert("stroke".to_owned(), json!(stroke));
        chars.insert("stroke_width".to_owned(), json!(width * scale));
    }
    let mut style = json!({"chars": chars});
    if given.and_then(|style| style.align).is_none() {
        style["align"] = json!(Align::Center);
    }
    translation["style"] = style;
    Ok(Op::UpdateLayer {
        layer: block.into(),
        patch: json!({"translation": translation}),
    })
}

/// The page's lettering size: the median size of its texts' sources, if any
/// has one.
fn lettering(layers: &[Layer]) -> Option<f64> {
    fn sizes(layers: &[Layer], found: &mut Vec<f64>) {
        for layer in layers {
            if let Content::Block(block) = &layer.content
                && block.role == Role::Text
                && let Some(size) = block.source.style.chars.size
            {
                found.push(size);
            }
            sizes(&layer.children, found);
        }
    }
    let mut found = Vec::new();
    sizes(layers, &mut found);
    found.sort_unstable_by(f64::total_cmp);
    found.get(found.len().checked_sub(1)? / 2).copied()
}

/// `frame` filled at every render, centred.
fn filled(frame: &Frame) -> Value {
    match frame {
        Frame::Point => json!("point"),
        Frame::Box { bounds, .. } => {
            json!({"box": {"bounds": bounds, "fit": "best", "anchor": "center"}, "path": null})
        }
        Frame::Path { path, inset, .. } => json!({
            "path": {"path": path, "fit": "best", "anchor": "center", "inset": inset},
            "box": null,
        }),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context as _, Result, bail};
    use icu_locale_core::langid;
    use koharu_scene::{Created, Edit, Frame, Session, Text};
    use kurbo::BezPath;

    use super::*;

    fn block(x: f64, role: &str) -> Value {
        json!({"kind": "block", "role": role, "source": {
            "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 40.0, "height": 120.0}}},
            "direction": "vertical",
        }})
    }

    /// A project of one 400-pixel page holding `layers`, each `(label,
    /// parent label, layer)`, and the labelled layers' ids in order.
    fn page(
        dir: &tempfile::TempDir,
        layers: &[(&str, Option<&str>, Value)],
    ) -> Result<(Session, Vec<LayerId>)> {
        let mut session = Session::create(dir.path().join("test.khr"))?;
        let mut ops = vec![
            json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 400}}),
        ];
        for (at, (label, parent, layer)) in layers.iter().enumerate() {
            // Last among its siblings.
            let index = layers[..at]
                .iter()
                .filter(|(_, other, _)| other == parent)
                .count();
            let mut op = json!({"op": "add_layer", "id": format!("${label}"), "page": "$p", "index": index, "layer": layer});
            if let Some(parent) = parent {
                op["parent"] = json!(format!("${parent}"));
            }
            ops.push(op);
        }
        let change = session.apply(Edit {
            label: "test".to_owned(),
            ops: ops
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()?,
        })?;
        let ids = layers
            .iter()
            .map(|(label, ..)| {
                change
                    .created
                    .iter()
                    .find_map(|created| match created {
                        Created::Layer {
                            id,
                            label: Some(found),
                        } if found == label => Some(*id),
                        _ => None,
                    })
                    .with_context(|| format!("no layer {label}"))
            })
            .collect::<Result<_>>()?;
        Ok((session, ids))
    }

    /// The translation `block` gets once placed into `target`.
    fn placed(session: &mut Session, block: LayerId, target: &LanguageIdentifier) -> Result<Text> {
        let op = placement(&session.project(), block, target)?;
        session.apply(Edit {
            label: "test".to_owned(),
            ops: vec![op],
        })?;
        match &session.project().layer(block)?.content {
            Content::Block(block) => block.translation.clone().context("a translation"),
            _ => bail!("not a block"),
        }
    }

    const BALLOON: &str = "M0 0 L200 0 L200 160 L0 160 Z";

    #[test]
    fn a_balloon_text_fills_its_room_and_a_free_one_its_source_box() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("b", None, json!({"kind": "balloon", "outline": BALLOON})),
                ("right", Some("b"), block(140.0, "text")),
                ("left", Some("b"), block(20.0, "text")),
                ("free", None, block(300.0, "text")),
            ],
        )?;
        let (right, free) = (ids[1], ids[3]);

        let text = placed(&mut session, right, &langid!("en"))?;
        let Frame::Path { path, fit, .. } = text.frame else {
            bail!("{:?}", text.frame);
        };
        // The right half of the balloon, as the right text's room.
        let room = BezPath::from_svg(path.as_str())?.bounding_box();
        assert!(room.x0 > 50.0 && room.x1 <= 200.5, "{room:?}");
        assert_eq!(fit, koharu_scene::PathFit::Best);
        assert_eq!(text.direction, Direction::Horizontal);

        let text = placed(&mut session, free, &langid!("ja"))?;
        let Frame::Box { bounds, fit, .. } = text.frame else {
            bail!("{:?}", text.frame);
        };
        assert_eq!((bounds.x, fit), (300.0, koharu_scene::Fit::Best));
        assert_eq!(text.direction, Direction::Vertical);
        Ok(())
    }

    #[test]
    fn a_balloon_text_keeps_off_a_free_text_standing_in_its_balloon() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("b", None, json!({"kind": "balloon", "outline": BALLOON})),
                ("own", Some("b"), block(140.0, "text")),
                ("remark", None, block(20.0, "text")),
            ],
        )?;

        let text = placed(&mut session, ids[1], &langid!("en"))?;
        let Frame::Path { path, .. } = text.frame else {
            bail!("{:?}", text.frame);
        };
        // The remark keeps the balloon's left half.
        let room = BezPath::from_svg(path.as_str())?.bounding_box();
        assert!(room.x0 > 50.0, "{room:?}");
        Ok(())
    }

    #[test]
    fn only_japanese_and_chinese_follow_a_vertical_source() -> Result<()> {
        // A column a word can't cross, and a tall sound effect.
        let column = |x: f64, role: &str| {
            json!({"kind": "block", "role": role, "source": {
                "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 30.0, "height": 160.0}}},
                "direction": "vertical",
                "style": {"chars": {"size": 20.0}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("column", None, column(20.0, "text")),
                ("sound", None, column(300.0, "sfx")),
            ],
        )?;
        for block in ids {
            let english = placed(&mut session, block, &langid!("en"))?;
            assert_eq!(english.direction, Direction::Horizontal);
            let chinese = placed(&mut session, block, &langid!("zh-Hans"))?;
            assert_eq!(chinese.direction, Direction::Vertical);
        }
        Ok(())
    }

    #[test]
    fn a_tilted_source_tilts_its_balloon_translation() -> Result<()> {
        // A source turned 10 degrees clockwise about its centre.
        let [a, b, c, d, e, f] =
            Affine::rotate_about(10f64.to_radians(), kurbo::Point::new(60.0, 80.0)).as_coeffs();
        let text = json!({"kind": "block", "role": "text", "source": {
            "frame": {"box": {"bounds": {"x": 40.0, "y": 20.0, "width": 40.0, "height": 120.0}}},
            "direction": "vertical",
            "transform": [[a, c, e], [b, d, f]],
        }});
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("b", None, json!({"kind": "balloon", "outline": BALLOON})),
                ("t", Some("b"), text),
            ],
        )?;
        let text = placed(&mut session, ids[1], &langid!("en"))?;
        let Transform([[a, _, _], [d, _, _]]) = text.transform;
        assert!((d.atan2(a).to_degrees() - 10.0).abs() < 1e-6);
        // Turned onto the page, its frame is the balloon again.
        let Frame::Path { path, .. } = text.frame else {
            bail!("{:?}", text.frame);
        };
        let onto = Affine::from(text.transform) * BezPath::from_svg(path.as_str())?;
        let room = onto.bounding_box();
        assert!(
            room.x0.abs() < 0.01 && (room.x1 - 200.0).abs() < 0.01,
            "{room:?}"
        );
        assert!(
            room.y0.abs() < 0.01 && (room.y1 - 160.0).abs() < 0.01,
            "{room:?}"
        );
        Ok(())
    }

    #[test]
    fn a_new_translation_is_centred_with_its_source_outline_scaled_to_its_size() -> Result<()> {
        let source = |x: f64, size: f64, outline: bool| {
            let mut chars = json!({"size": size});
            if outline {
                chars["stroke"] = json!([1.0, 1.0, 1.0, 1.0]);
                chars["stroke_width"] = json!(4.0);
            }
            json!({"kind": "block", "role": "text", "source": {
                "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 120.0, "height": 120.0}}},
                "style": {"chars": chars},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("outlined", None, source(0.0, 40.0, true)),
                ("plain", None, source(130.0, 20.0, false)),
                ("other", None, source(260.0, 20.0, false)),
            ],
        )?;

        // At the page's size of 20, half the source's 40, the outline halves.
        let text = placed(&mut session, ids[0], &langid!("en"))?;
        assert_eq!(text.style.align, Some(Align::Center));
        assert_eq!(text.style.chars.size, Some(20.0));
        assert_eq!(text.style.chars.stroke, Some(koharu_scene::Rgba([1.0; 4])));
        assert_eq!(text.style.chars.stroke_width, Some(2.0));
        let text = placed(&mut session, ids[1], &langid!("en"))?;
        assert_eq!(text.style.chars.stroke, None);
        Ok(())
    }
}
