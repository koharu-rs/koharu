//! Where a page's translations go and how large they're lettered, written when
//! they're translated. A frame is its room, simplified, so a later edit of the
//! balloon leaves it as is.

mod free;
mod room;
mod simplify;
mod size;

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use icu_locale_core::{LanguageIdentifier, subtags::language};
use koharu_scene::{
    Align, Anchor, Block, Bounds, Color, Content, Direction, Fit, Frame, Layer, LayerId, PageId,
    Path, PathFit, Project, Rgba, Role, Text, Transform,
};
use kurbo::{Affine, BezPath, Rect, Shape as _};
use rayon::prelude::*;
use serde_json::{Value, json};

/// A text of the page, its translation in the room it has.
struct Lettering<'p> {
    id: LayerId,
    block: &'p Block,
    /// Whether it's in a balloon.
    enclosed: bool,
    text: Text,
    /// The widest box it may take outside a balloon.
    widest: Option<Rect>,
}

/// The patch of each of `translated`, blocks of `page` and their new wordings,
/// that places its translation into `target`: in its room in its balloon, the
/// share of the outline its source's box gets beside the other texts standing
/// in the balloon, or in its source's frame when it's free or a sound effect.
/// It fills its frame at every render, centred on the frame's block axis, its
/// lines centred, as a Western balloon's are lettered, or in Japanese and
/// Chinese started together at the top or the left, as JLREQ aligns a line to
/// its head. A text that would draw smaller than the page's size, which every
/// text of the page has a say in, is brought up to it, and a sound effect to
/// its source's size when that's smaller.
pub(super) fn place(
    project: &Project,
    page: PageId,
    translated: &[(LayerId, String)],
    target: &LanguageIdentifier,
) -> Result<Vec<(LayerId, Value)>> {
    let blocks = project.blocks(page)?;
    let words: HashMap<LayerId, &str> = translated
        .iter()
        .map(|(id, words)| (*id, words.as_str()))
        .collect();
    let source = size::median(
        blocks
            .iter()
            .filter(|(_, block)| block.role == Role::Text)
            .filter_map(|(_, block)| block.source.style.chars.size)
            .collect(),
    );
    // A balloon's room split is a flood over its pixels, run for each of its
    // texts, so the texts are placed in parallel.
    let mut letterings = blocks
        .into_par_iter()
        .filter_map(|(id, block)| {
            let text = match words.get(&id) {
                Some(words) => translation(block, words, target),
                None => block.translation.clone()?,
            };
            Some(lettering(project, id, block, text, source))
        })
        .collect::<Result<Vec<_>>>()?;
    let size = size::page(&letterings, source)?;
    letterings.retain(|lettering| words.contains_key(&lettering.id));
    letterings.par_iter_mut().try_for_each(|lettering| {
        let own = lettering.block.source.style.chars.size;
        let size = match lettering.block.role {
            Role::Text => size,
            Role::Sfx => [own, size].into_iter().flatten().reduce(f64::min),
        };
        lettering.letter(size);
        size.map_or(Ok(()), |size| size::lift(lettering, size))
    })?;
    Ok(letterings
        .iter()
        .map(|lettering| (lettering.id, patch(&lettering.text)))
        .collect())
}

/// `block`'s translation worded `words` in `target`.
fn translation(block: &Block, words: &str, target: &LanguageIdentifier) -> Text {
    let given = block
        .translation
        .clone()
        .unwrap_or_else(|| block.source.blank());
    Text {
        content: given.content.replaced(words),
        language: target.clone(),
        ..given
    }
}

/// `text`, block `id`'s translation, in its room. It is vertical only when its
/// source is and it's Japanese or Chinese, which are written so; any other
/// language runs horizontally, turned by the source's tilt as the source is.
fn lettering<'p>(
    project: &'p Project,
    id: LayerId,
    block: &'p Block,
    text: Text,
    line: Option<f64>,
) -> Result<Lettering<'p>> {
    let (page, holder) = project.parent(id)?;
    let source = &block.source;
    let (enclosed, room) = match holder {
        Some(Layer {
            content: Content::Balloon(balloon),
            children,
            ..
        }) => {
            let room = match block.role {
                Role::Text => room::room(&balloon.outline, children, &page.layers, id)?,
                Role::Sfx => None,
            };
            (true, room)
        }
        _ => (false, None),
    };
    let (frame, transform) = match room {
        Some(room) => turned(room, source.transform)?,
        None => (filled(&source.frame), source.transform),
    };
    let cjk = [language!("ja"), language!("zh")].contains(&text.language.language);
    let vertical = source.direction == Direction::Vertical && cjk;
    let mut text = Text {
        frame,
        transform,
        direction: match vertical {
            true => Direction::Vertical,
            false => Direction::Horizontal,
        },
        ..text
    };
    text.style.align.get_or_insert(match cjk {
        true => Align::Start,
        false => Align::Center,
    });
    let widest = match enclosed || vertical {
        true => None,
        false => free::widest(page, holder, id, source, line),
    };
    Ok(Lettering {
        id,
        block,
        enclosed,
        text,
        widest,
    })
}

impl Lettering<'_> {
    /// Sets it at `size`, in its source's ink as its fill unless its style
    /// sets one, and an outline in proportion: its own, or its source's, or
    /// for one lying on art, as a sound effect does and a text outside a
    /// balloon may, one unlike its fill, which keeps it legible over it: a
    /// sixteenth of a sound effect's size, and an eighth of a text's, which is
    /// smaller. On paper the outline's colour it can't be seen.
    fn letter(&mut self, size: Option<f64>) {
        let source = &self.block.source.style.chars;
        let chars = &mut self.text.style.chars;
        chars.fill = chars.fill.or(source.fill);
        let outlined = [&*chars, source].into_iter().find_map(|chars| {
            let (stroke, width) = chars.stroke.zip(chars.stroke_width)?;
            let scale = size.zip(chars.size).map_or(1.0, |(to, from)| to / from);
            Some((stroke, width * scale))
        });
        let share = match self.block.role {
            Role::Sfx => Some(1.0 / 16.0),
            Role::Text if !self.enclosed => Some(1.0 / 8.0),
            Role::Text => None,
        };
        let outlined = outlined.or_else(|| {
            let (size, share) = size.zip(share).filter(|_| chars.stroke.is_none())?;
            Some((unlike(chars.fill), size * share))
        });
        if let Some((stroke, width)) = outlined {
            chars.stroke = Some(stroke);
            chars.stroke_width = Some(width);
        }
        chars.size = size.or(chars.size);
    }
}

/// The colour of an outline around `fill`: black around a light one, and white
/// around any other, the renderer's black included.
fn unlike(fill: Option<Rgba>) -> Rgba {
    let luminance = fill.map_or(0.0, |fill| {
        Color::from(fill).discard_alpha().relative_luminance()
    });
    match luminance > 0.5 {
        true => Rgba([0.0, 0.0, 0.0, 1.0]),
        false => Rgba([1.0; 4]),
    }
}

/// What places `text`, as a merge patch of a block: its frame's other variant
/// cleared.
fn patch(text: &Text) -> Value {
    let mut frame = json!(text.frame);
    if let Some(variants) = frame.as_object_mut() {
        for variant in ["box", "path"] {
            variants.entry(variant).or_insert(Value::Null);
        }
    }
    let chars = &text.style.chars;
    json!({"translation": {
        "frame": frame,
        "transform": text.transform,
        "direction": text.direction,
        "style": {
            "chars": {
                "size": chars.size,
                "fill": chars.fill,
                "stroke": chars.stroke,
                "stroke_width": chars.stroke_width,
            },
            "align": text.style.align,
        },
    }})
}

/// A frame of `room`, filled at every render, and where it lies: `source`'s
/// tilt about the room's centre, the room turned back by it into the text's
/// own coordinates, which the turn takes onto the page again.
fn turned(
    room: BezPath,
    Transform([[a, _, _], [d, _, _]]): Transform,
) -> Result<(Frame, Transform)> {
    let turn = Affine::rotate_about(d.atan2(a), room.bounding_box().center());
    let path = Path::try_from((turn.inverse() * room).to_svg()).map_err(|error| anyhow!(error))?;
    let frame = Frame::Path {
        path,
        fit: PathFit::Best,
        anchor: Anchor::Center,
        inset: Frame::DEFAULT_INSET,
    };
    Ok((frame, Transform::from(turn)))
}

/// `frame` filled at every render, centred.
fn filled(frame: &Frame) -> Frame {
    match frame {
        Frame::Point => Frame::Point,
        Frame::Box { bounds, .. } => boxed(Rect::from(*bounds)),
        Frame::Path { path, inset, .. } => Frame::Path {
            path: path.clone(),
            fit: PathFit::Best,
            anchor: Anchor::Center,
            inset: *inset,
        },
    }
}

/// A box filled at every render, centred.
fn boxed(rect: Rect) -> Frame {
    Frame::Box {
        bounds: Bounds {
            x: rect.x0,
            y: rect.y0,
            width: rect.width(),
            height: rect.height(),
        },
        fit: Fit::Best,
        anchor: Anchor::Center,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context as _, Result, bail};
    use icu_locale_core::langid;
    use koharu_renderer::{Renderer, RendererConfig};
    use koharu_scene::{Created, Edit, Op, Session};

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

    /// The translations `words`' blocks get once placed into `target`, in
    /// installed fonts, so no test downloads one.
    fn lettered(
        session: &mut Session,
        words: &[(LayerId, &str)],
        target: &LanguageIdentifier,
    ) -> Result<Vec<Text>> {
        Renderer::get().set_config(RendererConfig {
            font_families: Vec::new(),
        });
        let project = session.project();
        let page = project.pages().next().context("a page")?.id;
        let translated: Vec<_> = words
            .iter()
            .map(|(id, words)| (*id, (*words).to_owned()))
            .collect();
        let patches = place(&project, page, &translated, target)?;
        let ops = translated
            .into_iter()
            .map(|(id, text)| Op::SetText {
                layer: id.into(),
                text,
            })
            .chain(patches.into_iter().map(|(id, patch)| Op::UpdateLayer {
                layer: id.into(),
                patch,
            }))
            .collect();
        session.apply(Edit {
            label: "test".to_owned(),
            ops,
        })?;
        let project = session.project();
        words
            .iter()
            .map(|(id, _)| match &project.layer(*id)?.content {
                Content::Block(block) => block.translation.clone().context("a translation"),
                _ => bail!("not a block"),
            })
            .collect()
    }

    /// The translation `block` gets once placed into `target`.
    fn placed(session: &mut Session, block: LayerId, target: &LanguageIdentifier) -> Result<Text> {
        let mut texts = lettered(session, &[(block, "HO")], target)?;
        texts.pop().context("a text")
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
    fn japanese_and_chinese_start_their_lines_and_others_centre_them() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("ja", None, block(20.0, "text")),
                ("zh", None, block(120.0, "text")),
                ("en", None, block(220.0, "text")),
            ],
        )?;
        let japanese = placed(&mut session, ids[0], &langid!("ja"))?;
        assert_eq!(japanese.style.align, Some(Align::Start));
        let chinese = placed(&mut session, ids[1], &langid!("zh-Hans"))?;
        assert_eq!(chinese.style.align, Some(Align::Start));
        let english = placed(&mut session, ids[2], &langid!("en"))?;
        assert_eq!(english.style.align, Some(Align::Center));
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
    fn a_new_translation_is_centred_in_its_source_colours_with_the_outline_scaled_to_its_size()
    -> Result<()> {
        let source = |x: f64, size: f64, outline: bool| {
            let mut chars = json!({"size": size});
            if outline {
                chars["fill"] = json!([1.0, 0.0, 0.0, 1.0]);
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

        // At the page's size, what its room holds and not its sources' 20,
        // its outline is as broad for it as for its source's 40, and stays so
        // when it's placed again.
        for _ in 0..2 {
            let text = placed(&mut session, ids[0], &langid!("en"))?;
            let chars = &text.style.chars;
            assert_eq!(text.style.align, Some(Align::Center));
            assert_eq!(chars.fill, Some(Rgba([1.0, 0.0, 0.0, 1.0])));
            assert_eq!(chars.stroke, Some(Rgba([1.0; 4])));
            let (size, width) = chars.size.zip(chars.stroke_width).context("sizes")?;
            assert!(
                size > 40.0 && (width - size / 10.0).abs() < 1e-6,
                "{chars:?}"
            );
        }
        let text = placed(&mut session, ids[1], &langid!("en"))?;
        assert_eq!(text.style.chars.fill, None);
        Ok(())
    }

    #[test]
    fn a_text_on_art_without_an_outline_gets_one_unlike_its_fill() -> Result<()> {
        let source = |x: f64, role: &str, fill: [f32; 4]| {
            json!({"kind": "block", "role": role, "source": {
                "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 90.0, "height": 120.0}}},
                "style": {"chars": {"size": 32.0, "fill": fill}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("b", None, json!({"kind": "balloon", "outline": BALLOON})),
                (
                    "speech",
                    Some("b"),
                    source(60.0, "text", [0.2, 0.2, 0.2, 1.0]),
                ),
                ("sound", None, source(0.0, "sfx", [0.97, 0.94, 0.93, 1.0])),
                ("remark", None, source(210.0, "text", [0.2, 0.2, 0.2, 1.0])),
            ],
        )?;
        let (black, white) = (Rgba([0.0, 0.0, 0.0, 1.0]), Rgba([1.0; 4]));

        // A balloon's paper is behind its text already; on art, a sixteenth
        // of its size for a sound effect, and an eighth for a text.
        let mut outline = |block| -> Result<_> {
            let chars = placed(&mut session, block, &langid!("en"))?.style.chars;
            let size = chars.size.context("a size")?;
            let width = chars.stroke_width.map(|width| width / size);
            Ok(chars.stroke.zip(width))
        };
        assert_eq!(outline(ids[1])?, None);
        assert_eq!(outline(ids[2])?, Some((black, 1.0 / 16.0)));
        assert_eq!(outline(ids[3])?, Some((white, 1.0 / 8.0)));
        Ok(())
    }

    #[test]
    fn a_text_smaller_than_its_page_s_narrows_its_margin_as_far_as_that_takes() -> Result<()> {
        // Two balloons of one width, and a narrower one whose word reaches
        // their texts' size with a margin under an em.
        let balloon = |x: f64, y: f64, width: f64| {
            let (right, bottom) = (x + width, y + 190.0);
            json!({"kind": "balloon", "outline":
                format!("M{x} {y} L{right} {y} L{right} {bottom} L{x} {bottom} Z")})
        };
        let text = |x: f64, y: f64| {
            json!({"kind": "block", "source": {
                "frame": {"box": {"bounds": {"x": x, "y": y, "width": 60.0, "height": 100.0}}},
                "style": {"chars": {"size": 40.0}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("a", None, balloon(0.0, 0.0, 200.0)),
                ("roomy", Some("a"), text(70.0, 40.0)),
                ("b", None, balloon(200.0, 0.0, 200.0)),
                ("other", Some("b"), text(270.0, 40.0)),
                ("c", None, balloon(0.0, 200.0, 184.0)),
                ("tight", Some("c"), text(55.0, 240.0)),
            ],
        )?;
        let (roomy, other, tight) = (ids[1], ids[3], ids[5]);
        let margin = |text: &Text| match text.frame {
            Frame::Path { inset, .. } => Ok(inset),
            _ => bail!("{:?}", text.frame),
        };

        let texts = lettered(
            &mut session,
            &[(roomy, "HHHH"), (other, "HHHH"), (tight, "HHHH")],
            &langid!("en"),
        )?;
        let size = texts[2].style.chars.size.context("a size")?;
        assert_eq!((margin(&texts[0])?, margin(&texts[1])?), (1.0, 1.0));
        assert!((0.5..1.0).contains(&margin(&texts[2])?), "{:?}", texts[2]);
        assert!(size::drawn(tight, &texts[2])? >= 0.99 * size);
        // Placed alone, the texts it leaves set the size still.
        let alone = placed(&mut session, tight, &langid!("en"))?;
        let again = alone.style.chars.size.context("a size")?;
        assert!((again - size).abs() < 0.01 * size, "{again} against {size}");
        Ok(())
    }

    #[test]
    fn a_free_text_widens_in_its_panel_and_off_a_balloon() -> Result<()> {
        // Beside a roomy text, a column that holds no word at its size of 20,
        // with its panel's edge 30 to its left and a balloon 20 to its right;
        // and one written over a balloon's edge, 20 off the balloon's text.
        let dir = tempfile::tempdir()?;
        let text = |x: f64, y: f64, width: f64| {
            json!({"kind": "block", "source": {
                "frame": {"box": {"bounds": {"x": x, "y": y, "width": width, "height": 120.0}}},
                "direction": "vertical",
                "style": {"chars": {"size": 20.0}},
            }})
        };
        let (mut session, ids) = page(
            &dir,
            &[
                ("roomy", None, text(0.0, 220.0, 120.0)),
                ("other", None, text(0.0, 220.0, 120.0)),
                (
                    "panel",
                    None,
                    json!({"kind": "panel", "outline": "M150 200 L400 200 L400 400 L150 400 Z"}),
                ),
                ("column", Some("panel"), text(180.0, 220.0, 30.0)),
                (
                    "b",
                    Some("panel"),
                    json!({"kind": "balloon", "outline": "M230 200 L400 200 L400 400 L230 400 Z"}),
                ),
                (
                    "c",
                    None,
                    json!({"kind": "balloon", "outline": "M110 0 L300 0 L300 100 L110 100 Z"}),
                ),
                ("inside", Some("c"), text(150.0, 20.0, 60.0)),
                ("across", None, text(100.0, 20.0, 30.0)),
            ],
        )?;
        let texts = lettered(
            &mut session,
            &[
                (ids[0], "HO"),
                (ids[1], "HO"),
                (ids[3], "INCREDIBLE"),
                (ids[7], "INCREDIBLE"),
            ],
            &langid!("en"),
        )?;
        let bounds = |text: &Text| match text.frame {
            Frame::Box { bounds, .. } => Ok(Rect::from(bounds)),
            _ => bail!("{:?}", text.frame),
        };
        assert_eq!(bounds(&texts[0])?, Rect::new(0.0, 220.0, 120.0, 340.0));
        // 5 off its panel's edge and the balloon.
        let widened = bounds(&texts[2])?;
        assert!((widened.x1 - 225.0).abs() < 0.01, "{widened:?}");
        assert!((widened.x0 - 155.0).abs() < 0.01, "{widened:?}");
        assert_eq!((widened.y0, widened.y1), (220.0, 340.0));
        // And 5 off the text in the balloon it lies across, two and a half
        // times as wide.
        let widened = bounds(&texts[3])?;
        assert!((widened.x1 - 145.0).abs() < 0.01, "{widened:?}");
        assert!((widened.x0 - 70.0).abs() < 0.01, "{widened:?}");
        Ok(())
    }

    #[test]
    fn a_frame_widens_before_its_word_hyphenates_and_a_sound_effect_s_to_its_source_s_size()
    -> Result<()> {
        let block = |role: &str, y: f64, width: f64, height: f64, size: f64| {
            json!({"kind": "block", "role": role, "source": {
                "frame": {"box": {"bounds": {"x": 150.0, "y": y, "width": width, "height": height}}},
                "direction": "vertical",
                "style": {"chars": {"size": size}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("long", None, block("text", 0.0, 110.0, 190.0, 20.0)),
                ("broad", None, block("text", 200.0, 190.0, 90.0, 20.0)),
                ("sound", None, block("sfx", 300.0, 20.0, 80.0, 16.0)),
            ],
        )?;
        let texts = lettered(
            &mut session,
            &[
                (ids[0], "COMMUNICATION"),
                (ids[1], "COMMUNICATION"),
                (ids[2], "SURI"),
            ],
            &langid!("en"),
        )?;
        let long = Renderer::get()
            .layout(ids[0], &texts[0])?
            .context("a layout")?;
        assert!(long.hyphens.is_empty() && long.scale >= 0.999, "{long:?}");
        assert_eq!(texts[2].style.chars.size, Some(16.0));
        assert!(size::drawn(ids[2], &texts[2])? >= 15.9);
        Ok(())
    }
}
