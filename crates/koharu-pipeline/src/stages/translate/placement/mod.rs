//! Where a page's translations go and how large they're lettered, written when
//! they're translated. A frame is its room, simplified, so a later edit of the
//! balloon leaves it as is.

mod free;
mod kernel;
mod room;
mod simplify;
mod size;

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use icu_locale_core::{LanguageIdentifier, subtags::language};
use koharu_scene::{
    Align, Anchor, Block, Bounds, Color, Content, Direction, Fit, Frame, Kernel, Layer, LayerId,
    PageId, Path, PathFit, Project, Rgba, Role, Text, Transform,
};
use kurbo::{Affine, BezPath, Rect, Shape as _};
use rayon::prelude::*;
use serde_json::{Value, json};

/// A text of the page, its translation in the room it has.
struct Lettering<'p> {
    /// The project it's in, whose faces it's laid out in.
    project: &'p Project,
    id: LayerId,
    block: &'p Block,
    /// The balloon it's in.
    balloon: Option<LayerId>,
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
/// its source's size when that's smaller; a balloon's text its room can't hold
/// near that size keeps it, running past the room. Then whatever may move, a
/// free text, a sound effect, or a text running past its balloon, shrinks as
/// far as keeps it off the others' letters.
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
    let kernels = kernel::Kernels::new(project)?;
    // A balloon's room split is a flood over its pixels, run for each of its
    // texts, so the texts are placed in parallel.
    let mut letterings = blocks
        .into_par_iter()
        .filter_map(|(id, block)| {
            let text = match words.get(&id) {
                Some(words) => translation(block, words, target, kernels.of(id)),
                None => block.translation.clone()?,
            };
            Some(lettering(project, id, block, text, source, 0.0))
        })
        .collect::<Result<Vec<_>>>()?;
    let size = size::page(&letterings, source)?;
    // The split's reach is a share of the page's size, which the balloons'
    // texts measure in their shares.
    if let Some(size) = size {
        letterings = (letterings.into_par_iter())
            .map(|lettering| match lettering.balloon {
                Some(_) => self::lettering(
                    project,
                    lettering.id,
                    lettering.block,
                    lettering.text,
                    source,
                    SPLIT * size,
                ),
                None => Ok(lettering),
            })
            .collect::<Result<Vec<_>>>()?;
    }
    // The page, its balloons, the lines of its texts kept as they are, and
    // where every text's source lies, which its clean-up covers, drawn over
    // an earlier layer's lettering: what those that move keep within and off.
    let on = project.page(page)?;
    let sheet = Rect::new(
        0.0,
        0.0,
        f64::from(on.size.width),
        f64::from(on.size.height),
    );
    let mut outlines = Vec::new();
    balloons(&on.layers, &mut outlines);
    let fixed = (letterings.iter())
        .filter(|lettering| !words.contains_key(&lettering.id))
        .filter_map(|lettering| {
            let text = lettering.block.translation.as_ref()?;
            Some(size::lines(project, lettering.id, text))
        })
        .collect::<Result<Vec<_>>>()?
        .concat();
    let sources: Vec<(LayerId, Rect)> = (project.blocks(page)?.into_iter())
        .filter_map(|(id, block)| Some((id, block.source.bounds()?)))
        .collect();
    letterings.retain(|lettering| words.contains_key(&lettering.id));
    // Each text's size, and what it may shrink back to: what its room holds
    // when it runs past it, or the page's size when it's lifted above that.
    let (mut sizes, floors): (Vec<_>, Vec<_>) = letterings
        .par_iter_mut()
        .map(|lettering| {
            let own = lettering.block.source.style.chars.size;
            let (size, source) = match lettering.block.role {
                Role::Text => (size, own),
                Role::Sfx => ([own, size].into_iter().flatten().reduce(f64::min), None),
            };
            Ok(match size {
                Some(page) => {
                    let (size, held) = size::lift(lettering, page, source)?;
                    if held.is_none() && size <= page && lettering.balloon.is_some() {
                        size::roomier(lettering, size)?;
                    }
                    (Some(size), held.or((size > page).then_some(page)))
                }
                None => (None, None),
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .unzip();
    size::clear(
        &mut letterings,
        &mut sizes,
        &floors,
        &fixed,
        &sources,
        &outlines,
        sheet,
    )?;
    size::together(&mut letterings, &mut sizes)?;
    for (lettering, size) in letterings.iter_mut().zip(sizes) {
        lettering.letter(size);
    }
    Ok(letterings
        .iter()
        .map(|lettering| (lettering.id, patch(&lettering.text)))
        .collect())
}

/// The outlines of the balloons among `layers`, each with its layer.
fn balloons(layers: &[Layer], found: &mut Vec<(LayerId, BezPath)>) {
    for layer in layers {
        if let Content::Balloon(balloon) = &layer.content {
            found.push((layer.id, BezPath::from(&balloon.outline)));
        }
        balloons(&layer.children, found);
    }
}

/// `block`'s translation worded `words` in `target`, lettered in `kernel`'s
/// style, and at `weight` when its source is heavy.
fn translation(
    block: &Block,
    words: &str,
    target: &LanguageIdentifier,
    (kernel, weight): (Kernel, Option<u16>),
) -> Text {
    let mut text = (block.translation.clone()).unwrap_or_else(|| block.source.blank());
    text.content = text.content.replaced(words);
    text.language = target.clone();
    text.style.kernel = Some(kernel);
    text.style.chars.weight = weight.or(text.style.chars.weight);
    text
}

/// Of the page's size, how far a balloon text's share reaches past the lines
/// splitting the balloon among its texts, which aren't drawn: within a
/// three-quarter margin, its letters keep a quarter of an em off one, and
/// half an em off the other text's. A short text beside another, its share
/// narrowing away from the balloon's side, fitted only at the wide end.
const SPLIT: f64 = 0.5;

/// `text`, block `id`'s translation, in its room, which reaches `reach` past
/// the lines splitting its balloon. It is vertical only when its source is
/// and it's Japanese or Chinese, which are written so; any other language
/// runs horizontally, turned by the source's tilt as the source is.
fn lettering<'p>(
    project: &'p Project,
    id: LayerId,
    block: &'p Block,
    text: Text,
    line: Option<f64>,
    reach: f64,
) -> Result<Lettering<'p>> {
    let (page, holder) = project.parent(id)?;
    let source = &block.source;
    let (balloon, room) = match holder {
        Some(Layer {
            id: holder,
            content: Content::Balloon(balloon),
            children,
            ..
        }) => match block.role {
            // A remark across its edge is lettered as a free text.
            Role::Text => match room::room(&balloon.outline, children, &page.layers, id, reach)? {
                Some(room) => (Some(*holder), Some(room)),
                None => (None, None),
            },
            Role::Sfx => (Some(*holder), None),
        },
        _ => (None, None),
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
    let widest = match balloon.is_some() || vertical {
        true => None,
        false => free::widest(page, holder, id, source, line),
    };
    Ok(Lettering {
        project,
        id,
        block,
        balloon,
        text,
        widest,
    })
}

impl Lettering<'_> {
    /// Sets it at `size`, in its source's ink as its fill unless its style
    /// sets one, and an outline in proportion, at most `OUTLINE` of its size:
    /// its own, or its source's, or one unlike its fill, which keeps it legible
    /// over art, tone, or a balloon's border, a sixteenth of a sound effect's
    /// size and an eighth of a text's. On paper the outline's colour it can't
    /// be seen, so a balloon's text has one too: telling plain paper from tone
    /// takes the page's pixels, which placement doesn't read. A grey fill is
    /// black or white instead, and a grey outline white or black.
    fn letter(&mut self, size: Option<f64>) {
        let source = &self.block.source.style.chars;
        let chars = &mut self.text.style.chars;
        // A fill of no hue is black or white, the nearer: read off a scan,
        // black ink comes out grey, which looks faded.
        chars.fill = (chars.fill.or(source.fill)).map(|fill| match grey(fill) {
            true if luminance(fill) > 0.5 => WHITE,
            true => BLACK,
            false => fill,
        });
        let outlined = [&*chars, source].into_iter().find_map(|chars| {
            let (stroke, width) = chars.stroke.zip(chars.stroke_width)?;
            let scale = size.zip(chars.size).map_or(1.0, |(to, from)| to / from);
            Some((stroke, width * scale))
        });
        let share = match self.block.role {
            Role::Sfx => 1.0 / 16.0,
            Role::Text => 1.0 / 8.0,
        };
        let outlined = outlined.or_else(|| {
            let size = size.filter(|_| chars.stroke.is_none())?;
            Some((unlike(chars.fill), size * share))
        });
        if let Some((stroke, width)) = outlined {
            let most = size.or(chars.size).map_or(width, |size| OUTLINE * size);
            // An outline of no hue is white or black, unlike its fill: read
            // off a scan, a white or black one comes out grey, which looks
            // dirty around the letters.
            chars.stroke = Some(match grey(stroke) {
                true => unlike(chars.fill),
                false => stroke,
            });
            chars.stroke_width = Some(width.min(most));
        }
        chars.size = size.or(chars.size);
    }
}

/// Of a text's size, the widest outline it's given. A sound effect's measured
/// outline, scaled to a size read small from its source, had come to a third.
const OUTLINE: f64 = 1.0 / 8.0;

/// The most a colour's channels may differ for it to have no hue, and be set
/// in black or white.
const GREY: f32 = 0.1;

const BLACK: Rgba = Rgba([0.0, 0.0, 0.0, 1.0]);
const WHITE: Rgba = Rgba([1.0; 4]);

/// Whether `colour` has no hue.
fn grey(Rgba([red, green, blue, _]): Rgba) -> bool {
    red.max(green).max(blue) - red.min(green).min(blue) < GREY
}

fn luminance(colour: Rgba) -> f32 {
    Color::from(colour).discard_alpha().relative_luminance()
}

/// The colour of an outline around `fill`: black around a light one, and white
/// around any other, the renderer's black included.
fn unlike(fill: Option<Rgba>) -> Rgba {
    match fill.is_some_and(|fill| luminance(fill) > 0.5) {
        true => BLACK,
        false => WHITE,
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
                "weight": chars.weight,
                "fill": chars.fill,
                "stroke": chars.stroke,
                "stroke_width": chars.stroke_width,
            },
            "align": text.style.align,
            "kernel": text.style.kernel,
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
    use koharu_renderer::Renderer;
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
        let mut session =
            Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
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
    /// installed fonts, as a project with no styles sets them, so no test
    /// downloads one.
    fn lettered(
        session: &mut Session,
        words: &[(LayerId, &str)],
        target: &LanguageIdentifier,
    ) -> Result<Vec<Text>> {
        let project = session.project();
        let page = project.pages().next().context("a page")?.id;
        let translated: Vec<_> = words
            .iter()
            .map(|(id, words)| (*id, (*words).to_owned()))
            .collect();
        let patches = place(&project, page, &translated, target)?;
        // In `target`, as the stage writes them, which lays them out in it.
        let language = json!({"translation": {"language": target}});
        let ops = translated
            .into_iter()
            .flat_map(|(id, text)| {
                [
                    Op::SetText {
                        layer: id.into(),
                        text,
                    },
                    Op::UpdateLayer {
                        layer: id.into(),
                        patch: language.clone(),
                    },
                ]
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
        // The right half of the balloon, as the right text's room, reaching
        // past the split by a share of the page's size, the text's own.
        let size = text.style.chars.size.context("a size")?;
        let room = BezPath::from_svg(path.as_str())?.bounding_box();
        assert!(
            room.x0 > 50.0 - SPLIT * size && room.x1 <= 200.5,
            "{room:?}"
        );
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
        // The remark keeps the balloon's left half, which the text's room
        // reaches into by a share of the page's size, the text's own.
        let size = text.style.chars.size.context("a size")?;
        let room = BezPath::from_svg(path.as_str())?.bounding_box();
        assert!(room.x0 > 50.0 - SPLIT * size, "{room:?}");
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
    fn a_text_gets_an_outline_unlike_its_fill_no_wider_than_an_eighth_of_it() -> Result<()> {
        let source = |x: f64, role: &str, fill: [f32; 4]| {
            json!({"kind": "block", "role": role, "source": {
                "frame": {"box": {"bounds": {"x": x, "y": 20.0, "width": 90.0, "height": 120.0}}},
                "style": {"chars": {"size": 32.0, "fill": fill}},
            }})
        };
        // A sound effect whose outline measured a third of its size.
        let mut bold = source(0.0, "sfx", [0.0, 0.0, 0.0, 1.0]);
        bold["source"]["frame"]["box"]["bounds"]["y"] = json!(250.0);
        bold["source"]["style"]["chars"]["stroke"] = json!([1.0, 1.0, 1.0, 1.0]);
        bold["source"]["style"]["chars"]["stroke_width"] = json!(32.0 / 3.0);
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
                ("bold", None, bold),
            ],
        )?;
        let (black, white) = (Rgba([0.0, 0.0, 0.0, 1.0]), Rgba([1.0; 4]));

        // A sixteenth of its size for a sound effect, and an eighth for a
        // text, in a balloon too, whose tone or border it keeps off.
        let mut outline = |block| -> Result<_> {
            let chars = placed(&mut session, block, &langid!("en"))?.style.chars;
            let size = chars.size.context("a size")?;
            let width = chars.stroke_width.map(|width| width / size);
            Ok(chars.stroke.zip(width))
        };
        assert_eq!(outline(ids[1])?, Some((white, 1.0 / 8.0)));
        assert_eq!(outline(ids[2])?, Some((black, 1.0 / 16.0)));
        assert_eq!(outline(ids[3])?, Some((white, 1.0 / 8.0)));
        assert_eq!(outline(ids[4])?, Some((white, 1.0 / 8.0)));
        Ok(())
    }

    #[test]
    fn a_text_smaller_than_its_page_s_narrows_its_margin_as_far_as_that_takes() -> Result<()> {
        // Two balloons of one width, and a narrower one whose word reaches
        // their texts' size with a margin under an em; sources smaller than
        // the page's size, which they'd be brought up to in empty balloons.
        let balloon = |x: f64, y: f64, width: f64| {
            let (right, bottom) = (x + width, y + 190.0);
            json!({"kind": "balloon", "outline":
                format!("M{x} {y} L{right} {y} L{right} {bottom} L{x} {bottom} Z")})
        };
        let text = |x: f64, y: f64| {
            json!({"kind": "block", "source": {
                "frame": {"box": {"bounds": {"x": x, "y": y, "width": 60.0, "height": 100.0}}},
                "style": {"chars": {"size": 20.0}},
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
            &[(roomy, "HMHM"), (other, "HMHM"), (tight, "HMHM")],
            &langid!("en"),
        )?;
        let size = texts[2].style.chars.size.context("a size")?;
        // Held only at the page's size, which they set, they narrow theirs to
        // three quarters without drawing 15% larger.
        assert_eq!((margin(&texts[0])?, margin(&texts[1])?), (0.75, 0.75));
        assert!((0.5..1.0).contains(&margin(&texts[2])?), "{:?}", texts[2]);
        assert!(size::drawn(&session.project(), tight, &texts[2])? >= 0.99 * size);
        // Placed alone, the texts it leaves set the size still.
        let alone = placed(&mut session, tight, &langid!("en"))?;
        let again = alone.style.chars.size.context("a size")?;
        assert!((again - size).abs() < 0.01 * size, "{again} against {size}");
        Ok(())
    }

    #[test]
    fn a_word_too_long_for_its_balloon_keeps_the_page_s_size_past_it_short_of_others() -> Result<()>
    {
        // Two roomy balloons, a narrow one in the open below them and another
        // 10 right of the second, each 60 wide for a word with no hyphenation
        // point.
        let balloon = |x: f64, y: f64, width: f64| {
            let (right, bottom) = (x + width, y + 190.0);
            json!({"kind": "balloon", "outline": format!("M{x} {y} L{right} {y} L{right} {bottom} L{x} {bottom} Z")})
        };
        let text = |x: f64, y: f64| {
            json!({"kind": "block", "source": {
                "frame": {"box": {"bounds": {"x": x, "y": y + 40.0, "width": 40.0, "height": 100.0}}},
                "style": {"chars": {"size": 20.0}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("a", None, balloon(0.0, 0.0, 160.0)),
                ("roomy", Some("a"), text(60.0, 0.0)),
                ("b", None, balloon(160.0, 0.0, 160.0)),
                ("other", Some("b"), text(220.0, 0.0)),
                ("c", None, balloon(170.0, 205.0, 60.0)),
                ("open", Some("c"), text(180.0, 205.0)),
                ("d", None, balloon(330.0, 0.0, 60.0)),
                ("beside", Some("d"), text(340.0, 0.0)),
            ],
        )?;
        let word = "HMHMHMHMHM";
        let texts = lettered(
            &mut session,
            &[
                (ids[1], "HMHM"),
                (ids[3], "HMHM"),
                (ids[5], word),
                (ids[7], word),
            ],
            &langid!("en"),
        )?;
        let fit = |text: &Text| match text.frame {
            Frame::Path { fit, .. } => Ok(fit),
            _ => bail!("{:?}", text.frame),
        };
        let size = texts[0].style.chars.size.context("a size")?;
        assert_eq!(fit(&texts[0])?, PathFit::Best);
        // In the open, at the page's size past its balloon, outlined.
        let open = &texts[2].style.chars;
        assert_eq!(fit(&texts[2])?, PathFit::None);
        assert_eq!(open.size, Some(size));
        assert_eq!(open.stroke, Some(Rgba([1.0; 4])));
        assert_eq!(open.stroke_width, Some(size / 8.0));
        // Beside a balloon, as large as stays off it, its outline included.
        let beside = texts[3].style.chars.size.context("a size")?;
        assert_eq!(fit(&texts[3])?, PathFit::None);
        assert!(beside < 0.9 * size, "{beside} against {size}");
        let laid = Renderer::get()
            .layout(&session.project(), ids[7], &texts[3])?
            .context("a layout")?;
        let left = laid
            .lines
            .iter()
            .map(|line| line.x0)
            .fold(f64::MAX, f64::min);
        assert!(left >= 320.0, "{left}");
        Ok(())
    }

    #[test]
    fn a_text_its_balloon_would_leave_empty_grows_past_it_until_it_fills_it() -> Result<()> {
        // Two narrow balloons, and two whose word draws a line across a
        // fraction of them at the page's size, its source at 40: a tall one
        // it can't fill below that, and a short one it fills at less.
        let balloon = |x: f64, y: f64, width: f64, height: f64| {
            let (right, bottom) = (x + width, y + height);
            json!({"kind": "balloon", "outline": format!("M{x} {y} L{right} {y} L{right} {bottom} L{x} {bottom} Z")})
        };
        let text = |x: f64, y: f64, size: f64| {
            json!({"kind": "block", "source": {
                "frame": {"box": {"bounds": {"x": x, "y": y, "width": 40.0, "height": 60.0}}},
                "style": {"chars": {"size": size}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("a", None, balloon(0.0, 0.0, 80.0, 120.0)),
                ("one", Some("a"), text(20.0, 30.0, 10.0)),
                ("b", None, balloon(0.0, 130.0, 80.0, 120.0)),
                ("two", Some("b"), text(20.0, 160.0, 10.0)),
                ("c", None, balloon(110.0, 0.0, 120.0, 400.0)),
                ("tall", Some("c"), text(150.0, 170.0, 40.0)),
                ("d", None, balloon(260.0, 150.0, 100.0, 80.0)),
                ("short", Some("d"), text(290.0, 160.0, 40.0)),
            ],
        )?;
        let texts = lettered(
            &mut session,
            &[
                (ids[1], "HMHM"),
                (ids[3], "HMHM"),
                (ids[5], "HMHM"),
                (ids[7], "HMHM"),
            ],
            &langid!("en"),
        )?;
        let page = size::drawn(&session.project(), ids[1], &texts[0])?;
        let past = |text: &Text| -> Result<f64> {
            let Frame::Path { fit, .. } = text.frame else {
                bail!("{:?}", text.frame);
            };
            assert_eq!(fit, PathFit::None);
            assert_eq!(text.style.chars.stroke, Some(Rgba([1.0; 4])));
            text.style.chars.size.context("a size")
        };
        let (tall, short) = (past(&texts[2])?, past(&texts[3])?);
        assert_eq!(tall, 40.0);
        assert!(short > 1.3 * page && short < 38.0, "{short} against {page}");
        Ok(())
    }

    #[test]
    fn a_sound_effect_over_a_balloon_shrinks_off_its_text() -> Result<()> {
        // A balloon's text, and a sound effect whose box reaches over it.
        let block = |role: &str, x: f64, y: f64, width: f64, height: f64| {
            json!({"kind": "block", "role": role, "source": {
                "frame": {"box": {"bounds": {"x": x, "y": y, "width": width, "height": height}}},
                "style": {"chars": {"size": 40.0}},
            }})
        };
        let dir = tempfile::tempdir()?;
        let (mut session, ids) = page(
            &dir,
            &[
                ("b", None, json!({"kind": "balloon", "outline": BALLOON})),
                ("speech", Some("b"), block("text", 60.0, 40.0, 40.0, 100.0)),
                ("sound", None, block("sfx", 140.0, 30.0, 250.0, 100.0)),
            ],
        )?;
        let texts = lettered(
            &mut session,
            &[(ids[1], "HELLO THERE"), (ids[2], "BOOM!")],
            &langid!("en"),
        )?;
        let Frame::Box { fit, .. } = texts[1].frame else {
            bail!("{:?}", texts[1].frame);
        };
        assert_eq!(fit, Fit::Shrink);
        // As drawn, outlines included.
        let drawn = |id: LayerId, text: &Text| -> Result<Vec<Rect>> {
            let laid = Renderer::get()
                .layout(&session.project(), id, text)?
                .context("a layout")?;
            Ok(laid.lines)
        };
        let (speech, sound) = (drawn(ids[1], &texts[0])?, drawn(ids[2], &texts[1])?);
        assert!(
            sound
                .iter()
                .all(|a| speech.iter().all(|b| a.intersect(*b).area() == 0.0)),
            "{sound:?} against {speech:?}"
        );
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
            .layout(&session.project(), ids[0], &texts[0])?
            .context("a layout")?;
        assert!(long.hyphens.is_empty() && long.scale >= 0.999, "{long:?}");
        assert_eq!(texts[2].style.chars.size, Some(16.0));
        assert!(size::drawn(&session.project(), ids[2], &texts[2])? >= 15.9);
        Ok(())
    }
}
