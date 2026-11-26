//! The size a page is lettered at, measured as the renderer draws it, and how
//! a text that draws smaller is brought up to it, or kept at it past a
//! balloon too narrow for its words, off the page's other balloons and texts.
//! One its room holds larger still fills its room.

use std::collections::HashMap;

use anyhow::Result;
use koharu_renderer::Renderer;
use koharu_scene::{
    Direction, Fit, Frame, LanguageIdentifier, LayerId, PathFit, Project, Role, Text,
};
use kurbo::{Affine, BezPath, Line, ParamCurve as _, Point, Rect, Shape as _};
use rayon::prelude::*;

use super::{Lettering, boxed};

/// The margins a balloon's text may have, in ems, the widest first. Three
/// quarters keeps a gap from the balloon's border the eye sees; half held a
/// text 15 to 17% larger but set letters against the border.
const MARGINS: [f64; 3] = [1.0, 0.875, 0.75];
/// Of a balloon's text's size, how large a narrower margin lets it draw before
/// the margin stops narrowing.
const ROOMY: f64 = 1.15;
/// The steps a frame widens in, along its lines and across them.
const ALONG: u32 = 16;
const ACROSS: u32 = 4;
/// Of the page's size, the least a text draws at rather than run past its
/// room: under it, as under the share the renderer starts a hyphen at, it
/// looks smaller than its neighbours.
const NEAR: f64 = 0.9;
/// Of its room's height, the most a balloon's text at the page's size may
/// take and leave the balloon empty, as a word as wide as the room holds it
/// small. Of a chapter's 92 balloon texts, those that looked empty took at
/// most 0.38 of it, and the rest 0.59 or more.
const EMPTY: f64 = 0.5;
/// The margin, in ems, of a text lifted toward its source's size: up to the
/// balloon's border but off it. Half an em held "Leave it to me!" at 24 px in
/// a balloon its 40 px original filled.
const EDGE: f64 = 0.125;
/// Of the larger of two sources' sizes, the least the smaller is for a
/// balloon's two texts to be set alike: a remark lettered at 19 pixels beside
/// 23 is set apart, and 24 beside 26 alike.
const ALIKE: f64 = 0.85;

/// The middle of `values`, the larger of two.
pub(super) fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_unstable_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The size of the page `letterings` are on: the median of the largest its
/// dialogue's rooms hold in whole words, each measured from its sources' size,
/// `source`. Its dialogue is its texts in balloons, or every text when none is
/// in one: a running head's or a title's widened frame holds far more than a
/// balloon, and two set a page's one balloon text past its balloon at nearly
/// three times its source's size. Page medians agree within a work, where a
/// size over its pages followed the tightest one.
pub(super) fn page(letterings: &[Lettering], source: Option<f64>) -> Result<Option<f64>> {
    let texts: Vec<&Lettering> = (letterings.iter())
        .filter(|lettering| lettering.block.role == Role::Text)
        .collect();
    let in_balloons: Vec<&Lettering> = (texts.iter().copied())
        .filter(|lettering| lettering.balloon.is_some())
        .collect();
    let dialogue = match in_balloons.is_empty() {
        true => texts,
        false => in_balloons,
    };
    let mut held = dialogue
        .par_iter()
        .map(|lettering| {
            let mut text = lettering.text.clone();
            if let Some(widest) = lettering.widest {
                text.frame = boxed(widest);
            }
            text.style.chars.size = source.or(text.style.chars.size);
            // A text of no language has no hyphens.
            text.language = LanguageIdentifier::UNKNOWN;
            drawn(lettering.project, lettering.id, &text)
        })
        .collect::<Result<Vec<_>>>()?;
    // A text of no characters draws at none.
    held.retain(|size| *size > 0.0);
    Ok(median(held))
}

/// Brings `lettering` up to `size`, the page's, or toward `source`, its
/// source's, when that's larger and its balloon would look empty at the page's
/// size, as the original fills it: as large as its balloon holds up to
/// `source`, within `EDGE` of its border; and when the balloon still looks
/// empty there, past its border too, up to the size at which it no longer
/// does or `source`. Taking `source` whole overfilled tall balloons whose
/// lines it multiplied, and stopping at the border left a word as wide as a
/// narrow balloon at a fraction of its original. Gives the size it's set at,
/// and what its room holds when it runs past the room.
pub(super) fn lift(
    lettering: &mut Lettering,
    size: f64,
    source: Option<f64>,
) -> Result<(f64, Option<f64>)> {
    let given = lettering.text.frame.clone();
    let held = reach(lettering, size)?;
    let Some(source) = source.filter(|source| *source > size) else {
        return Ok((size, held));
    };
    if !empty(lettering.project, lettering.id, &at(&lettering.text, size))? {
        return Ok((size, held));
    }
    let placed = std::mem::replace(&mut lettering.text.frame, given);
    if reach(lettering, source)?.is_none() {
        return Ok((source, None));
    }
    // What the balloon holds up to its border, as the original's letters
    // reach it; shrinking from its source's size finds that in a narrow
    // balloon, where growing from the page's stopped short of it.
    if let Frame::Path { fit, inset, .. } = &mut lettering.text.frame {
        *fit = PathFit::Best;
        *inset = inset.min(EDGE);
    }
    let fills = drawn(
        lettering.project,
        lettering.id,
        &at(&lettering.text, source),
    )?;
    let (frame, start, set, holds) = match fills >= size {
        true => (lettering.text.frame.clone(), fills, source, None),
        false => (placed, size, size, held),
    };
    // Past the border, in the same room.
    let Frame::Path {
        path,
        anchor,
        inset,
        ..
    } = &frame
    else {
        lettering.text.frame = frame;
        return Ok((set, holds));
    };
    let mut past = lettering.text.clone();
    past.frame = Frame::Path {
        path: path.clone(),
        fit: PathFit::None,
        anchor: *anchor,
        inset: *inset,
    };
    if start >= 0.999 * source || !empty(lettering.project, lettering.id, &at(&past, start))? {
        lettering.text.frame = frame;
        return Ok((set, holds));
    }
    let mut grown = source;
    if !empty(lettering.project, lettering.id, &at(&past, source))? {
        let (mut low, mut high) = (start, source);
        for _ in 0..8 {
            let middle = (low + high) / 2.0;
            match empty(lettering.project, lettering.id, &at(&past, middle))? {
                true => low = middle,
                false => high = middle,
            }
        }
        grown = high;
    }
    lettering.text.frame = past.frame;
    Ok((grown, Some(holds.unwrap_or(start))))
}

/// Narrows the margin of `lettering`, a balloon's text set at `size` and
/// filling its room, a step of `MARGINS` at a time, until it draws `ROOMY`
/// times that size or the margin is the narrowest: one its share holds only
/// at about the page's size looked small beside texts that grow to fill
/// roomier balloons.
pub(super) fn roomier(lettering: &mut Lettering, size: f64) -> Result<()> {
    let Frame::Path {
        fit: PathFit::Best,
        inset,
        ..
    } = lettering.text.frame
    else {
        return Ok(());
    };
    for margin in std::iter::once(inset).chain(MARGINS.into_iter().filter(|m| *m < inset)) {
        if let Frame::Path { inset, .. } = &mut lettering.text.frame {
            *inset = margin;
        }
        let draws = drawn(lettering.project, lettering.id, &at(&lettering.text, size))?;
        if draws >= ROOMY * size {
            break;
        }
    }
    Ok(())
}

/// `text` set at `size`.
fn at(text: &Text, size: f64) -> Text {
    let mut text = text.clone();
    text.style.chars.size = Some(size);
    text
}

/// Whether `text`, block `id`'s, takes under `EMPTY` of its room's extent
/// across its lines.
fn empty(project: &Project, id: LayerId, text: &Text) -> Result<bool> {
    let Frame::Path { path, .. } = &text.frame else {
        return Ok(false);
    };
    let Some(laid) = Renderer::get().layout(project, id, text)? else {
        return Ok(false);
    };
    let room = BezPath::from(path).bounding_box();
    let across = |rect: Rect| match text.direction {
        Direction::Vertical => (rect.x0, rect.x1),
        Direction::Horizontal => (rect.y0, rect.y1),
    };
    let (low, high) = (laid.lines.iter().map(|line| across(*line)))
        .fold((f64::MAX, f64::MIN), |(low, high), (from, to)| {
            (low.min(from), high.max(to))
        });
    let (from, to) = across(room);
    Ok(high - low < EMPTY * (to - from))
}

/// Brings `lettering` up to `size` in the least of its rooms that holds it so,
/// in whole words if one does, or else in the most it may take. A balloon's
/// text that room holds under `NEAR` of `size` keeps `size` there, running past
/// the room, and what the room holds is given, the least `clear` shrinks it
/// to. A free text doesn't: its frame has widened as far as it may, and one
/// still smaller, such as a chapter's subtitle under its title, is a smaller
/// text.
fn reach(lettering: &mut Lettering, size: f64) -> Result<Option<f64>> {
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
            drawn(lettering.project, lettering.id, &text)
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

/// Keeps the texts that may move off the others' letters, off `fixed`, those
/// of the page's texts kept as they are, and off the other texts' `sources`,
/// whose clean-ups hid an overflowing letter, each shrinking only as far as
/// that takes: a free text or a sound effect from what fills its frame down to
/// half of it, as less is too small to read, and a balloon's text set above
/// what fills its room, running past it or lifted to its source's size, from
/// its size, of `sizes`, down to its `floors`, what its room holds or the
/// page's size, which also keeps off the page's other `balloons`; all on the
/// page, `sheet`. A
/// sound effect lying across dialogue takes the size covering least of it.
/// Sound effects move first, then free texts, then those balloons' texts,
/// each the largest first, so a balloon's text keeps its size before
/// what lies on art does. Sound effects drawn over balloons and over each
/// other, and remarks beside them, had covered their texts.
pub(super) fn clear(
    letterings: &mut [Lettering],
    sizes: &mut [Option<f64>],
    floors: &[Option<f64>],
    fixed: &[Rect],
    sources: &[(LayerId, Rect)],
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
                lettering.project,
                lettering.id,
                &at_size(lettering, &lettering.text.frame, *size),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let rank = |at: usize| match (&letterings[at].text.frame, floors[at]) {
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
            .chain(
                (sources.iter())
                    .filter(|(id, _)| *id != lettering.id)
                    .map(|(_, source)| (*source, false)),
            )
            .collect();
        let walls: Vec<&BezPath> = match floors[at] {
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
        let (least, most, frame) = match (&lettering.text.frame, floors[at], sizes[at]) {
            (
                Frame::Path {
                    path,
                    anchor,
                    inset,
                    ..
                },
                Some(floor),
                Some(size),
            ) => {
                let frame = Frame::Path {
                    path: path.clone(),
                    fit: PathFit::None,
                    anchor: *anchor,
                    inset: *inset,
                };
                // No larger than it draws, which a lifted text's room holds.
                let draws = drawn(
                    lettering.project,
                    lettering.id,
                    &at_size(lettering, &lettering.text.frame, Some(size)),
                )?;
                (floor, size.min(draws), frame)
            }
            (Frame::Box { bounds, anchor, .. }, _, size) => {
                let fills = drawn(
                    lettering.project,
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
        let trial = |size: f64| {
            lines(
                lettering.project,
                lettering.id,
                &at_size(lettering, &frame, Some(size)),
            )
        };
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
        // With nothing clear past its floor, it fills its room from there.
        if let (Some(size), Some(floor)) = (found, floors[at])
            && size <= 1.01 * floor
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
        match (found, floors[at]) {
            (Some(size), _) => {
                lettering.text.frame = frame;
                sizes[at] = Some(size);
            }
            (None, Some(floor)) => {
                if let Frame::Path { fit, .. } = &mut lettering.text.frame {
                    *fit = PathFit::Best;
                }
                sizes[at] = Some(floor);
            }
            // A free text that no size clears, lying across another, stays.
            (None, None) => continue,
        }
        current[at] = lines(
            lettering.project,
            lettering.id,
            &at_size(lettering, &lettering.text.frame, sizes[at]),
        )?;
    }
    Ok(())
}

/// Sets the texts of a balloon, of `letterings` at `sizes`, whose sources are
/// set alike at one size, the smallest any of them draws at, as a letterer
/// sets one speaker's balloon: a short text filling its share grew past the
/// long one beside it, which the original set alike. A remark its original
/// set smaller, under `ALIKE` of another's size, stays apart: matched, it held
/// the balloon's main text at its own narrow share's size. One that would draw
/// larger stops filling its room.
pub(super) fn together(letterings: &mut [Lettering], sizes: &mut [Option<f64>]) -> Result<()> {
    let mut balloons: HashMap<LayerId, Vec<(usize, f64, f64)>> = HashMap::new();
    for (index, (lettering, size)) in letterings.iter().zip(&*sizes).enumerate() {
        let (Some(balloon), Role::Text, Some(size), Some(source)) = (
            lettering.balloon,
            lettering.block.role,
            size,
            lettering.block.source.style.chars.size,
        ) else {
            continue;
        };
        let draws = drawn(lettering.project, lettering.id, &at(&lettering.text, *size))?;
        balloons
            .entry(balloon)
            .or_default()
            .push((index, draws, source));
    }
    for texts in balloons.values().filter(|texts| texts.len() > 1) {
        for &(index, draws, source) in texts {
            let least = (texts.iter())
                .filter(|(_, _, other)| source.min(*other) >= ALIKE * source.max(*other))
                .map(|(_, draws, _)| *draws)
                .fold(f64::MAX, f64::min);
            if draws > 1.001 * least
                && let Frame::Path { fit, .. } = &mut letterings[index].text.frame
                && *fit == PathFit::Best
            {
                *fit = PathFit::Shrink;
                sizes[index] = Some(least);
            }
        }
    }
    Ok(())
}

/// The boxes of `text`'s lines, block `id`'s, on the page, each grown by an
/// eighth of the text's size for its outline or the air beside it.
pub(super) fn lines(project: &Project, id: LayerId, text: &Text) -> Result<Vec<Rect>> {
    let onto = Affine::from(text.transform);
    Ok(Renderer::get()
        .layout(project, id, text)?
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
pub(super) fn drawn(project: &Project, id: LayerId, text: &Text) -> Result<f64> {
    let laid = Renderer::get().layout(project, id, text)?;
    Ok(laid.map_or(0.0, |laid| {
        laid.runs
            .iter()
            .map(|run| f64::from(run.size))
            .fold(0.0, f64::max)
    }))
}
