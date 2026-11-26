//! The size a page is lettered at, measured as the renderer draws it, and how
//! a text that draws smaller is brought up to it, or kept at it past a
//! balloon too narrow for its words, off the page's other balloons and texts.
//! One its room holds larger still fills its room.

use anyhow::Result;
use koharu_renderer::Renderer;
use koharu_scene::{Fit, Frame, LanguageIdentifier, LayerId, PathFit, Role, Text};
use kurbo::{Affine, BezPath, Line, ParamCurve as _, Point, Rect, Shape as _};
use rayon::prelude::*;

use super::{Lettering, boxed};

/// The margins a balloon's text may have, in ems, the widest first. Half holds
/// it 15 to 17% larger than the whole, and less puts letters on the balloon's
/// border.
const MARGINS: [f64; 5] = [1.0, 0.875, 0.75, 0.625, 0.5];
/// The steps a frame widens in, along its lines and across them.
const ALONG: u32 = 16;
const ACROSS: u32 = 4;
/// Of the page's size, the least a text draws at rather than run past its
/// room: under it, as under the share the renderer starts a hyphen at, it
/// looks smaller than its neighbours.
const NEAR: f64 = 0.9;

/// The middle of `values`, the larger of two.
pub(super) fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_unstable_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The size of the page `letterings` are on: the median of the largest its
/// texts' rooms hold in whole words, each measured from its sources' size,
/// `source`. Page medians agree within a work, where a size over its pages
/// followed the tightest one.
pub(super) fn page(letterings: &[Lettering], source: Option<f64>) -> Result<Option<f64>> {
    let mut held = letterings
        .par_iter()
        .filter(|lettering| lettering.block.role == Role::Text)
        .map(|lettering| {
            let mut text = lettering.text.clone();
            if let Some(widest) = lettering.widest {
                text.frame = boxed(widest);
            }
            text.style.chars.size = source.or(text.style.chars.size);
            // A text of no language has no hyphens.
            text.language = LanguageIdentifier::UNKNOWN;
            drawn(lettering.id, &text)
        })
        .collect::<Result<Vec<_>>>()?;
    // A text of no characters draws at none.
    held.retain(|size| *size > 0.0);
    Ok(median(held))
}

/// Brings `lettering` up to `size` in the least of its rooms that holds it so,
/// in whole words if one does, or else in the most it may take. A balloon's
/// text that room holds under `NEAR` of `size` keeps `size` there, running past
/// the room, and what the room holds is given, the least `clear` shrinks it
/// to. A free text doesn't: its frame has widened as far as it may, and one
/// still smaller, such as a chapter's subtitle under its title, is a smaller
/// text.
pub(super) fn lift(lettering: &mut Lettering, size: f64) -> Result<Option<f64>> {
    let rooms = rooms(lettering);
    let Some(most) = rooms.last() else {
        return Ok(None);
    };
    let mut text = lettering.text.clone();
    text.style.chars.size = Some(size);
    let mut held = 0.0;
    for language in [LanguageIdentifier::UNKNOWN, text.language.clone()] {
        text.language = language;
        let mut holds = |room: &Frame| -> Result<f64> {
            text.frame = room.clone();
            drawn(lettering.id, &text)
        };
        held = holds(most)?;
        if held < 0.999 * size {
            continue;
        }
        for room in &rooms {
            if holds(room)? >= 0.999 * size {
                lettering.text.frame = room.clone();
                return Ok(None);
            }
        }
    }
    lettering.text.frame = most.clone();
    if let Frame::Path { fit, .. } = &mut lettering.text.frame
        && held < NEAR * size
    {
        *fit = PathFit::None;
        return Ok(Some(held));
    }
    Ok(None)
}

/// Keeps the texts that may move off the others' letters, and off `fixed`,
/// those of the page's texts kept as they are, each shrinking only as far as
/// that takes: a free text or a sound effect from what fills its frame down to
/// half of it, as less is too small to read, and a balloon's text running past
/// it from its size, of `sizes`, down to what its room holds, of `held`, which
/// also keeps off the page's other `balloons`; all on the page, `sheet`. A
/// sound effect lying across dialogue takes the size covering least of it.
/// Sound effects move first, then free texts, then those running past a
/// balloon, each the largest first, so a balloon's text keeps its size before
/// what lies on art does. Sound effects drawn over balloons and over each
/// other, and remarks beside them, had covered their texts.
pub(super) fn clear(
    letterings: &mut [Lettering],
    sizes: &mut [Option<f64>],
    held: &[Option<f64>],
    fixed: &[Rect],
    balloons: &[(LayerId, BezPath)],
    sheet: Rect,
) -> Result<()> {
    let at_size = |lettering: &Lettering, frame: &Frame, size: Option<f64>| {
        let mut text = lettering.text.clone();
        text.frame = frame.clone();
        text.style.chars.size = size.or(text.style.chars.size);
        text
    };
    let mut current = letterings
        .iter()
        .zip(&*sizes)
        .map(|(lettering, size)| {
            lines(
                lettering.id,
                &at_size(lettering, &lettering.text.frame, *size),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let rank = |at: usize| match (&letterings[at].text.frame, held[at]) {
        (_, Some(_)) => Some(2),
        (Frame::Box { .. }, None) if letterings[at].block.role == Role::Sfx => Some(0),
        (Frame::Box { .. }, None) => Some(1),
        _ => None,
    };
    let area = |lines: &[Rect]| lines.iter().map(Rect::area).sum::<f64>();
    let mut order: Vec<usize> = (0..letterings.len())
        .filter(|&at| rank(at).is_some())
        .collect();
    order.sort_by(|&a, &b| {
        rank(a)
            .cmp(&rank(b))
            .then(area(&current[b]).total_cmp(&area(&current[a])))
    });
    for at in order {
        let lettering = &letterings[at];
        // The others' lines, each with whether a sound effect's.
        let others: Vec<(Rect, bool)> = (current.iter().enumerate())
            .filter(|(other, _)| *other != at)
            .flat_map(|(other, lines)| {
                let sound = letterings[other].block.role == Role::Sfx;
                lines.iter().map(move |line| (*line, sound))
            })
            .chain(fixed.iter().map(|line| (*line, false)))
            .collect();
        let walls: Vec<&BezPath> = match held[at] {
            Some(_) => (balloons.iter())
                .filter(|(id, _)| Some(*id) != lettering.balloon)
                .map(|(_, outline)| outline)
                .collect(),
            None => Vec::new(),
        };
        let clear = |lines: &[Rect], sounds: bool| {
            lines.iter().all(|line| {
                sheet.contains_rect(*line)
                    && !others.iter().any(|(other, sound)| {
                        (sounds || !sound) && line.intersect(*other).area() > 0.0
                    })
                    && !walls.iter().any(|wall| meets(*line, wall))
            })
        };
        if clear(&current[at], true) {
            continue;
        }
        // The least and most it's set at, in a frame that sets it so.
        let (least, most, frame) = match (&lettering.text.frame, held[at], sizes[at]) {
            (frame, Some(held), Some(size)) => (held, size, frame.clone()),
            (Frame::Box { bounds, anchor, .. }, _, size) => {
                let fills = drawn(
                    lettering.id,
                    &at_size(lettering, &lettering.text.frame, size),
                )?;
                let frame = Frame::Box {
                    bounds: *bounds,
                    fit: Fit::Shrink,
                    anchor: *anchor,
                };
                (fills / 2.0, fills, frame)
            }
            _ => continue,
        };
        let trial = |size: f64| lines(lettering.id, &at_size(lettering, &frame, Some(size)));
        let smallest = trial(least)?;
        // A sound effect lying across another keeps off the rest at least, as
        // a letterer lays one sound effect over another.
        let sounds = lettering.block.role != Role::Sfx || clear(&smallest, true);
        let mut found = None;
        if clear(&smallest, sounds) {
            let (mut low, mut high) = (least, most);
            for _ in 0..8 {
                let middle = (low + high) / 2.0;
                match clear(&trial(middle)?, sounds) {
                    true => low = middle,
                    false => high = middle,
                }
            }
            found = Some(low);
        }
        // With nothing clear past what its room holds, it fills the room.
        if let (Some(size), Some(held)) = (found, held[at])
            && size <= 1.01 * held
        {
            found = None;
        }
        // A sound effect no size keeps off the dialogue covers the least of it.
        if found.is_none() && lettering.block.role == Role::Sfx {
            let covered = |lines: &[Rect]| -> f64 {
                (lines.iter())
                    .flat_map(|line| {
                        (others.iter())
                            .filter(|(_, sound)| !sound)
                            .map(|(other, _)| line.intersect(*other).area())
                    })
                    .sum()
            };
            let mut least_covered = covered(&current[at]);
            for step in 0..=8 {
                let size = least + (most - least) * f64::from(step) / 8.0;
                let covers = covered(&trial(size)?);
                if covers < least_covered {
                    (found, least_covered) = (Some(size), covers);
                }
            }
        }
        let lettering = &mut letterings[at];
        match (found, held[at]) {
            (Some(size), _) => {
                lettering.text.frame = frame;
                sizes[at] = Some(size);
            }
            (None, Some(_)) => {
                if let Frame::Path { fit, .. } = &mut lettering.text.frame {
                    *fit = PathFit::Best;
                }
            }
            // A free text that no size clears, lying across another, stays.
            (None, None) => continue,
        }
        current[at] = lines(
            lettering.id,
            &at_size(lettering, &lettering.text.frame, sizes[at]),
        )?;
    }
    Ok(())
}

/// The boxes of `text`'s lines, block `id`'s, on the page, each grown by an
/// eighth of the text's size for its outline or the air beside it.
pub(super) fn lines(id: LayerId, text: &Text) -> Result<Vec<Rect>> {
    let onto = Affine::from(text.transform);
    Ok(Renderer::get()
        .layout(id, text)?
        .map_or_else(Vec::new, |laid| {
            let size = laid
                .runs
                .iter()
                .map(|run| f64::from(run.size))
                .fold(0.0, f64::max);
            laid.lines
                .iter()
                .map(|line| onto.transform_rect_bbox(line.inflate(size / 8.0, size / 8.0)))
                .collect()
        }))
}

/// Whether `rect` and the inside of `outline` share any area: a corner of
/// either inside the other, or an edge across one of the other's.
fn meets(rect: Rect, outline: &BezPath) -> bool {
    if !rect.overlaps(outline.bounding_box()) {
        return false;
    }
    let corners = [
        Point::new(rect.x0, rect.y0),
        Point::new(rect.x1, rect.y0),
        Point::new(rect.x1, rect.y1),
        Point::new(rect.x0, rect.y1),
    ];
    corners.iter().any(|corner| outline.contains(*corner))
        || outline.segments().any(|segment| {
            rect.contains(segment.start())
                || (0..4).any(|at| {
                    !segment
                        .intersect_line(Line::new(corners[at], corners[(at + 1) % 4]))
                        .is_empty()
                })
        })
}

/// The rooms `lettering`'s text may take, from the one it has to the most: in a
/// balloon narrower margins, and outside one wider frames, along its lines
/// first and then across them.
fn rooms(lettering: &Lettering) -> Vec<Frame> {
    match (&lettering.text.frame, lettering.widest) {
        (
            Frame::Path {
                path,
                fit,
                anchor,
                inset,
            },
            _,
        ) => MARGINS
            .into_iter()
            .map(|margin| Frame::Path {
                path: path.clone(),
                fit: *fit,
                anchor: *anchor,
                inset: inset.min(margin),
            })
            .collect(),
        (Frame::Box { bounds, .. }, Some(widest)) => {
            let own = Rect::from(*bounds);
            let mix = |from: f64, to: f64, share: f64| from + (to - from) * share;
            (0..=ACROSS)
                .flat_map(|across| (0..=ALONG).map(move |along| (along, across)))
                .map(|(along, across)| {
                    let along = f64::from(along) / f64::from(ALONG);
                    let across = f64::from(across) / f64::from(ACROSS);
                    boxed(Rect::new(
                        mix(own.x0, widest.x0, along),
                        mix(own.y0, widest.y0, across),
                        mix(own.x1, widest.x1, along),
                        mix(own.y1, widest.y1, across),
                    ))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The size `text`, block `id`'s, draws at; 0 when it has no characters.
pub(super) fn drawn(id: LayerId, text: &Text) -> Result<f64> {
    let laid = Renderer::get().layout(id, text)?;
    Ok(laid.map_or(0.0, |laid| {
        laid.runs
            .iter()
            .map(|run| f64::from(run.size))
            .fold(0.0, f64::max)
    }))
}
