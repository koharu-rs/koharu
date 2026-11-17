//! Lines fitted to a text's room: at its size when they fit, or in a best-fit
//! frame at the largest scale above it that fits; else, as the frame allows,
//! at the largest scale down to half that fits with whole words, then at half
//! with a path's inset halved, then hyphenated, and a best-fit text below half
//! as it last broke until it fits. A word is never split without a hyphen:
//! what still doesn't fit runs past its room whole.

use std::ops::{Range, RangeInclusive};

use koharu_scene::{Align, Anchor};
use kurbo::Rect;

use super::{
    breaks::{Break, Breaking, Segment},
    lines::{self, Balance},
    room::{self, Outline, Room, Spans, fills, physical, shrinks},
    style::Styled,
};

/// The largest scale a best-fit text grows to, and its inverse the smallest
/// it shrinks to, bounds that only text no room limits, such as one with no
/// line height, or a room too small for anything, reaches.
const CEILING: f32 = 256.0;

/// Lines placed at one scale.
#[derive(Clone, Debug)]
pub(super) struct Placed {
    pub scale: f32,
    pub lines: Vec<PlacedLine>,
    /// The box the text was laid out in, in its own coordinates.
    pub frame: Rect,
    pub overflowed: bool,
}

#[derive(Clone, Debug)]
pub(super) struct PlacedLine {
    pub segments: Range<usize>,
    /// Where the line starts and the room it has, along the inline axis.
    pub start: f32,
    pub room: f32,
    /// The line box along the block axis.
    pub top: f32,
    pub pitch: f32,
    /// The paragraph ends with this line.
    pub last: bool,
}

/// `styled`'s segments and its lines in `room`, the segments broken as
/// `segments` gives them for each way of breaking.
pub(super) fn fit(
    styled: &Styled,
    room: &Room,
    mut segments: impl FnMut(Breaking) -> Vec<Segment>,
) -> (Vec<Segment>, Placed) {
    let words = segments(Breaking::Words);
    let set = place(styled, room, &words, 1.0);
    if !set.overflowed {
        let placed = match fills(&styled.frame) {
            true => grow(styled, room, &words, set),
            false => set,
        };
        return (words, placed);
    }
    // Shrinking stops at half the set size. Whole words shrink first, as
    // Comicraft avoids a hyphen whenever it can.
    let floor = if shrinks(&styled.frame) { 0.5 } else { 1.0 };
    let placed = largest_fit(
        floor,
        |scale| overflows(styled, room, &words, scale),
        |scale| match scale == 1.0 {
            // The text at its size is placed already.
            true => set.clone(),
            false => place(styled, room, &words, scale),
        },
    );
    if !placed.overflowed {
        return (words, placed);
    }
    // At the floor, a balloon's margin narrows to half, before a hyphen and
    // short of letters on its border, which read as cut off; then words
    // hyphenate, where the fewest need it, as text whose language hyphenates
    // does, vertical too; CJK breaks between characters already.
    let (placed, inset) = match room {
        Room::Path {
            outline,
            anchor,
            inset,
        } => {
            let inset = inset / 2.0;
            let narrowed = in_path(styled, &words, floor, outline, *anchor, inset).place();
            if !narrowed.overflowed {
                return (words, narrowed);
            }
            (narrowed, Some((outline, *anchor, inset)))
        }
        Room::Point | Room::Box { .. } => (placed, None),
    };
    let at = |segments: &[Segment], scale: f32| match inset {
        Some((outline, anchor, inset)) => {
            in_path(styled, segments, scale, outline, anchor, inset).place()
        }
        None => place(styled, room, segments, scale),
    };
    let hyphens = match styled.hyphenation {
        Some(_) => {
            let hyphens = segments(Breaking::Hyphens);
            let hyphenated = at(&hyphens, floor);
            if !hyphenated.overflowed {
                return (hyphens, hyphenated);
            }
            Some(hyphens)
        }
        None => None,
    };
    // A best-fit text has no floor of its own: as Publisher's Best Fit, it
    // shrinks on, as it last broke, until its room holds it, as far below
    // its size as it may grow above.
    if fills(&styled.frame) {
        let segments = hyphens.unwrap_or(words);
        let sure = |scale: f32| match inset {
            Some((outline, anchor, inset)) => {
                let (fewest, most) =
                    in_path(styled, &segments, scale, outline, anchor, inset).counts();
                fewest > most
            }
            None => overflows(styled, room, &segments, scale),
        };
        let shrunk = largest_fit(
            1.0 / CEILING,
            |scale| sure(scale * floor),
            |scale| at(&segments, scale * floor),
        );
        return (segments, shrunk);
    }
    // Nothing fits: the words run past their room whole, centred, into its
    // margin first.
    (words, placed)
}

/// The largest scale above 1 at which `segments`, which fit at 1 as `set`,
/// still fit, breaking between words only: the scale doubles until they
/// don't, then the gap is halved as shrinking halves it. A word that outgrows
/// its line ends the growth rather than being hyphenated.
fn grow(styled: &Styled, room: &Room, segments: &[Segment], set: Placed) -> Placed {
    let overflows = |scale| overflows(styled, room, segments, scale);
    let at = |scale| place(styled, room, segments, scale);
    let (mut low, mut best) = (1.0, set);
    while low < CEILING {
        let high = low * 2.0;
        let candidate = match overflows(high) {
            true => None,
            false => Some(at(high)).filter(|placed| !placed.overflowed),
        };
        match candidate {
            Some(placed) => (low, best) = (high, placed),
            None => return halve(&overflows, &at, low, high, best),
        }
    }
    best
}

/// The largest scale from 1 down to `floor` at which `at` fits the text, else
/// the text at `floor`. It probes down from
/// the top in eighths, because a path's room changes with the line count, so
/// a smaller size can fail where a larger one fits; then halves the step
/// eight times. Above the floor only whether the text fits matters, so a
/// scale `overflows` is sure of isn't laid out.
fn largest_fit(floor: f32, overflows: impl Fn(f32) -> bool, at: impl Fn(f32) -> Placed) -> Placed {
    if floor >= 1.0 || !overflows(1.0) {
        let set = at(1.0);
        if !set.overflowed || floor >= 1.0 {
            return set;
        }
    }
    let step = (1.0 - floor) / 8.0;
    let mut above = 1.0;
    for probe in 1..8 {
        let scale = 1.0 - step * probe as f32;
        if overflows(scale) {
            above = scale;
            continue;
        }
        let candidate = at(scale);
        if !candidate.overflowed {
            return halve(&overflows, &at, scale, above, candidate);
        }
        above = scale;
    }
    let scale = 1.0 - step * 8.0;
    let candidate = at(scale);
    match candidate.overflowed {
        true => candidate,
        false => halve(&overflows, &at, scale, above, candidate),
    }
}

/// The largest scale between `low`, where the text fits as `best`, and
/// `high`, where it doesn't, found by halving the gap eight times.
fn halve(
    overflows: &impl Fn(f32) -> bool,
    at: &impl Fn(f32) -> Placed,
    mut low: f32,
    mut high: f32,
    mut best: Placed,
) -> Placed {
    for _ in 0..8 {
        let middle = (low + high) * 0.5;
        if overflows(middle) {
            high = middle;
            continue;
        }
        let candidate = at(middle);
        if candidate.overflowed {
            high = middle;
        } else {
            low = middle;
            best = candidate;
        }
    }
    best
}

/// Whether `segments` are sure to overflow `room` at `scale`, short of
/// placing them: when a path holds fewer lines than the text needs.
fn overflows(styled: &Styled, room: &Room, segments: &[Segment], scale: f32) -> bool {
    match room {
        Room::Path {
            outline,
            anchor,
            inset,
        } => {
            let (fewest, most) =
                in_path(styled, segments, scale, outline, *anchor, *inset).counts();
            fewest > most
        }
        Room::Point | Room::Box { .. } => false,
    }
}

/// `segments` broken into lines and placed in `room` at `scale`.
fn place(styled: &Styled, room: &Room, segments: &[Segment], scale: f32) -> Placed {
    match room {
        Room::Point => point(styled, segments, scale),
        Room::Box {
            bounds,
            anchor,
            grow,
        } => boxed(styled, segments, scale, *bounds, *anchor, *grow),
        Room::Path {
            outline,
            anchor,
            inset,
        } => in_path(styled, segments, scale, outline, *anchor, *inset).place(),
    }
}

/// A line's pitch: its largest size, scaled, times the line height.
fn pitch(segments: &[Segment], scale: f32, line_height: f32) -> f32 {
    segments.iter().map(|s| s.size).fold(0.0, f32::max) * scale * line_height
}

/// Point text: each paragraph on one line, as long as the longest, lines
/// going down from y = 0 and columns left from x = 0.
fn point(styled: &Styled, segments: &[Segment], scale: f32) -> Placed {
    let lines = lines::break_lines(segments, scale, Balance::Unbounded, None, |_| f32::INFINITY)
        .expect("point text always breaks");
    let room = lines
        .lines
        .iter()
        .map(|range| lines::length(segments, range.clone(), scale))
        .fold(0.0, f32::max);
    let (lines, total) = stack(styled, segments, scale, lines.lines, 0.0, room, 0.0);
    Placed {
        scale,
        lines,
        frame: physical(styled.vertical, 0.0, 0.0, f64::from(room), f64::from(total)),
        overflowed: false,
    }
}

/// Box text: lines as wide as the box, anchored along the block axis in the
/// box, or in what it grows to.
fn boxed(
    styled: &Styled,
    segments: &[Segment],
    scale: f32,
    bounds: Rect,
    anchor: Anchor,
    grow: bool,
) -> Placed {
    let room = bounds.width() as f32;
    let balance = match styled.align {
        Align::Center => Balance::Even,
        _ => Balance::Ragged,
    };
    let lines = lines::break_lines(segments, scale, balance, None, |_| room)
        .expect("box text always breaks");
    let total: f32 = lines
        .lines
        .iter()
        .map(|range| pitch(&segments[range.clone()], scale, styled.line_height))
        .sum();
    let (v0, v1) = (bounds.y0 as f32, bounds.y1 as f32);
    let (frame_v0, frame_v1) = match (grow, anchor) {
        (false, _) => (v0, v1),
        (true, Anchor::Start) => (v0, v0 + total),
        (true, Anchor::Center) => {
            let middle = (v0 + v1) * 0.5;
            (middle - total * 0.5, middle + total * 0.5)
        }
        (true, Anchor::End) => (v1 - total, v1),
    };
    let top = match anchor {
        Anchor::Start => frame_v0,
        Anchor::Center => frame_v0 + (frame_v1 - frame_v0 - total) * 0.5,
        Anchor::End => frame_v1 - total,
    };
    let overflowed = lines.overflowed || (!grow && total > bounds.height() as f32 + 0.01);
    let (lines, _) = stack(
        styled,
        segments,
        scale,
        lines.lines,
        bounds.x0 as f32,
        room,
        top,
    );
    Placed {
        scale,
        lines,
        frame: physical(
            styled.vertical,
            bounds.x0,
            f64::from(frame_v0),
            bounds.x1,
            f64::from(frame_v1),
        ),
        overflowed,
    }
}

/// `lines` stacked from `top`, each starting at `start` with `room`, and
/// their total pitch.
fn stack(
    styled: &Styled,
    segments: &[Segment],
    scale: f32,
    lines: Vec<Range<usize>>,
    start: f32,
    room: f32,
    mut top: f32,
) -> (Vec<PlacedLine>, f32) {
    let mut placed = Vec::with_capacity(lines.len());
    let mut total = 0.0;
    for range in lines {
        let pitch = pitch(&segments[range.clone()], scale, styled.line_height);
        placed.push(line(segments, range, start, room, top, pitch));
        top += pitch;
        total += pitch;
    }
    (placed, total)
}

fn line(
    segments: &[Segment],
    range: Range<usize>,
    start: f32,
    room: f32,
    top: f32,
    pitch: f32,
) -> PlacedLine {
    let last = range
        .end
        .checked_sub(1)
        .is_some_and(|at| matches!(segments[at].after, Break::Mandatory | Break::End));
    PlacedLine {
        segments: range,
        start,
        room,
        top,
        pitch,
        last,
    }
}

/// Lines in a closed path at one scale, `pitch` apart, kept `air` inside the
/// outline: the outline eroded by the inset, band by band.
struct InPath<'a> {
    segments: &'a [Segment],
    scale: f32,
    outline: &'a Outline,
    anchor: Anchor,
    /// The side along the inline axis the lines share, unless they're
    /// centred.
    edge: Option<Side>,
    pitch: f32,
    air: f32,
    frame: Rect,
}

/// A side of the lines along the inline axis: where `Low` lines start or
/// `High` ones end.
#[derive(Clone, Copy)]
enum Side {
    Low,
    High,
}

fn in_path<'a>(
    styled: &Styled,
    segments: &'a [Segment],
    scale: f32,
    outline: &'a Outline,
    anchor: Anchor,
    inset: f32,
) -> InPath<'a> {
    let size = segments.iter().map(|s| s.size).fold(0.0, f32::max);
    // A right-to-left text starts its lines on the right, as arranging them
    // does by its first paragraph's direction.
    let rtl = !styled.vertical
        && unicode_bidi::get_base_direction(styled.text.as_str()) == unicode_bidi::Direction::Rtl;
    InPath {
        segments,
        scale,
        outline,
        anchor,
        edge: match (styled.align, rtl) {
            (Align::Center, _) => None,
            (Align::Start | Align::Justify, false) | (Align::End, true) => Some(Side::Low),
            (Align::End, false) | (Align::Start | Align::Justify, true) => Some(Side::High),
        },
        pitch: pitch(segments, scale, styled.line_height).max(0.01),
        air: size * scale * inset,
        frame: physical(
            styled.vertical,
            outline.bounds.x0,
            outline.bounds.y0,
            outline.bounds.x1,
            outline.bounds.y1,
        ),
    }
}

impl InPath<'_> {
    /// Every line count the path's block extent holds is tried, and the
    /// cheapest count that fits wins; when none fits, the least overflowing,
    /// the fewer lines on a tie.
    fn place(&self) -> Placed {
        let (fewest, most) = self.counts();
        // Fewer lines than `fewest` overflow, so they're tried only when no
        // more fit.
        match self.best(fewest..=most) {
            Some((_, placed)) if !placed.overflowed => placed,
            later => match (self.best(1..=most.min(fewest - 1)), later) {
                (Some(earlier), Some(later)) if later.0 < earlier.0 => later.1,
                (Some((_, placed)), _) | (None, Some((_, placed))) => placed,
                (None, None) => self.overflowing(),
            },
        }
    }

    /// The fewest lines that might hold the text, and the most the outline's
    /// block extent holds. Lines together are at least as long as the text
    /// without its trailing white space, and none has more room than the
    /// outline is wide, less the air; a ten-thousandth spares rounding.
    fn counts(&self) -> (usize, usize) {
        let extent = self.outline.bounds.height() as f32;
        let most = (((extent - 2.0 * self.air) / self.pitch).floor() as usize)
            .clamp(1, 64)
            .min(self.segments.len().max(1));
        let length = self
            .segments
            .iter()
            .map(|segment| segment.advance - segment.trailing)
            .sum::<f32>()
            * self.scale
            * 0.9999;
        let room = (self.outline.bounds.width() as f32 - 2.0 * self.air).max(0.0) + 0.01;
        let fewest = (length / room).ceil().max(1.0) as usize;
        (fewest, most)
    }

    /// The cheapest of `counts` lines, one that fits over one that doesn't,
    /// the first on a tie, with its cost.
    fn best(&self, counts: RangeInclusive<usize>) -> Option<(f32, Placed)> {
        let mut best: Option<(f32, Placed)> = None;
        for count in counts {
            let Some((cost, placed)) = self.lines(count) else {
                continue;
            };
            let better = best.as_ref().is_none_or(|(best_cost, best)| {
                (best.overflowed && !placed.overflowed)
                    || (best.overflowed == placed.overflowed && cost < *best_cost)
            });
            if better {
                best = Some((cost, placed));
            }
        }
        best
    }

    /// The text in `count` lines about one axis, and what they cost. Each
    /// line's span is where the outline holds the axis all along its band
    /// widened by the air, less the air at both ends. A centred line takes as
    /// much of it on both sides of the axis. Lines aligned to an edge start
    /// together, or end together, where Photoshop starts each where the shape
    /// does: they share the edge every span reaches, each runs from it to its
    /// own span's far end, so a lopsided balloon's longer side holds longer
    /// lines, and the block, as long as its longest line, is centred on the
    /// axis as far as that allows. `None` when `count` lines can't hold it.
    fn lines(&self, count: usize) -> Option<(f32, Placed)> {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let total = pitch * count as f32;
        let top = match self.anchor {
            Anchor::Start => bounds.y0 as f32 + air,
            Anchor::Center => (bounds.y0 + bounds.y1) as f32 * 0.5 - total * 0.5,
            Anchor::End => bounds.y1 as f32 - air - total,
        };
        let bands: Vec<Spans> = (0..count)
            .map(|line| {
                let v0 = top + pitch * line as f32;
                self.outline
                    .band(f64::from(v0 - air), f64::from(v0 + pitch + air))
            })
            .collect();
        let axis = room::axis(&bands).unwrap_or(bounds.center().x);
        // Each line's span from start to end. A band with no room on the axis
        // holds a line of none there, so a word forced into it overflows
        // about the axis.
        let spans: Vec<(f32, f32)> = bands
            .iter()
            .map(|spans| {
                match self.edge {
                    None => room::reach(spans, axis),
                    Some(_) => room::holding(spans, axis),
                }
                .map(|(start, end)| (start as f32 + air, end as f32 - air))
                .filter(|(start, end)| end > start)
                .unwrap_or((axis as f32, axis as f32))
            })
            .collect();
        // The edge every span reaches, which lines aligned to it share.
        let shared = self.edge.map(|side| {
            let edge = match side {
                Side::Low => spans.iter().map(|span| span.0).fold(f32::MIN, f32::max),
                Side::High => spans.iter().map(|span| span.1).fold(f32::MAX, f32::min),
            };
            (side, edge)
        });
        let rooms: Vec<f32> = spans
            .iter()
            .map(|&(start, end)| match shared {
                None => end - start,
                Some((Side::Low, edge)) => (end - edge).max(0.0),
                Some((Side::High, edge)) => (edge - start).max(0.0),
            })
            .collect();
        let lines = lines::break_lines(
            self.segments,
            self.scale,
            Balance::Even,
            Some(count),
            |line| rooms[line],
        )?;
        let used: Vec<f32> = lines
            .lines
            .iter()
            .map(|range| lines::length(self.segments, range.clone(), self.scale))
            .collect();
        // The lines' unfilled shares squared, and far more for one longer
        // than its room, to compare line counts.
        let cost = used
            .iter()
            .enumerate()
            .map(|(line, &used)| {
                let room = rooms[line];
                if used > room {
                    1e6 * (1.0 + used - room)
                } else if room > 0.0 {
                    let slack = (room - used) / room;
                    slack * slack
                } else {
                    0.0
                }
            })
            .sum();
        // Where a block sharing an edge starts: centred on the axis, unless
        // that takes it past the shared edge or a line past its span's far
        // end.
        let longest = used.iter().copied().fold(0.0, f32::max);
        let block = shared.map(|(side, edge)| {
            let centred = axis as f32 - longest * 0.5;
            match side {
                Side::Low => {
                    let latest = spans
                        .iter()
                        .zip(&used)
                        .map(|(span, used)| span.1 - used)
                        .fold(f32::MAX, f32::min);
                    centred.min(latest).max(edge)
                }
                Side::High => {
                    let earliest = spans
                        .iter()
                        .zip(&used)
                        .map(|(span, used)| span.0 + used)
                        .fold(f32::MIN, f32::max);
                    (centred + longest).max(earliest).min(edge) - longest
                }
            }
        });
        let placed = lines
            .lines
            .into_iter()
            .enumerate()
            .map(|(at, range)| {
                let (start, room) = block
                    .map_or((spans[at].0, spans[at].1 - spans[at].0), |block| {
                        (block, longest)
                    });
                line(
                    self.segments,
                    range,
                    start,
                    room,
                    top + pitch * at as f32,
                    pitch,
                )
            })
            .collect();
        Some((
            cost,
            Placed {
                scale: self.scale,
                lines: placed,
                frame: self.frame,
                overflowed: lines.overflowed,
            },
        ))
    }
    /// The text when no line count holds it: in the middle line's widest
    /// span, or the bounds, running past the outline as overflowing box text
    /// does.
    fn overflowing(&self) -> Placed {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let middle = (bounds.y0 + bounds.y1) * 0.5;
        let band = self.outline.band(
            middle - f64::from(pitch) * 0.5,
            middle + f64::from(pitch) * 0.5,
        );
        let (start, end) = room::axis(std::slice::from_ref(&band))
            .and_then(|axis| room::reach(&band, axis))
            .unwrap_or((bounds.x0, bounds.x1));
        let room = (end - start) as f32;
        let lines = lines::break_lines(self.segments, self.scale, Balance::Even, None, |_| room)
            .expect("lines without a count always break");
        let total = pitch * lines.lines.len() as f32;
        let top = match self.anchor {
            Anchor::Start => bounds.y0 as f32 + air,
            Anchor::Center => middle as f32 - total * 0.5,
            Anchor::End => bounds.y1 as f32 - air - total,
        };
        Placed {
            scale: self.scale,
            lines: lines
                .lines
                .into_iter()
                .enumerate()
                .map(|(at, range)| {
                    line(
                        self.segments,
                        range,
                        start as f32,
                        room,
                        top + pitch * at as f32,
                        pitch,
                    )
                })
                .collect(),
            frame: self.frame,
            overflowed: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use hypher::Lang;
    use koharu_scene::Frame;

    use super::*;

    /// A word `advance` long with a space after it, ending as `after` says.
    fn word(advance: f32, after: Break) -> Segment {
        Segment {
            range: 0..0,
            visible_end: 0,
            advance: advance + 1.0,
            trailing: 1.0,
            hyphen: 5.0,
            after,
            punctuation: false,
            size: 10.0,
        }
    }

    /// Text in a box `width` by `height` that fits it as `fit` says.
    fn in_box(width: f64, height: f64, fit: &str) -> Result<(Styled, Room)> {
        let frame: Frame = serde_json::from_value(serde_json::json!({
            "box": {"bounds": {"x": 0.0, "y": 0.0, "width": width, "height": height}, "fit": fit}
        }))?;
        let room = Room::new(&frame, false);
        Ok((
            Styled {
                text: String::new(),
                spans: Vec::new(),
                vertical: false,
                align: Align::Start,
                line_height: 1.2,
                hyphenation: Some(Lang::English),
                chinese: false,
                frame,
            },
            room,
        ))
    }

    #[test]
    fn a_text_too_big_for_its_box_shrinks_to_the_largest_size_that_fits() -> Result<()> {
        // Two words 121 long together, in a box 100 wide and one line tall.
        let (styled, room) = in_box(100.0, 12.0, "shrink")?;
        let (_, placed) = fit(&styled, &room, |_| {
            vec![word(60.0, Break::Normal), word(60.0, Break::End)]
        });
        assert!(!placed.overflowed);
        assert_eq!(placed.lines.len(), 1);
        // 100 / 121 at most.
        assert!((0.8..0.83).contains(&placed.scale), "{}", placed.scale);
        Ok(())
    }

    /// A word 150 long, in two hyphenated halves when it may break.
    fn long(breaking: Breaking) -> Vec<Segment> {
        match breaking {
            Breaking::Words => vec![word(150.0, Break::End)],
            Breaking::Hyphens => vec![word(75.0, Break::Hyphen), word(75.0, Break::End)],
        }
    }

    #[test]
    fn a_best_fit_text_grows_to_the_largest_size_its_box_holds() -> Result<()> {
        // A word 40 long in a box 100 wide and 60 tall: its line holds it up
        // to 2.5 times, and the box's height up to 5.
        let (styled, room) = in_box(100.0, 60.0, "best")?;
        let (_, placed) = fit(&styled, &room, |_| vec![word(40.0, Break::End)]);
        assert!(!placed.overflowed);
        assert!((2.45..=2.5).contains(&placed.scale), "{}", placed.scale);
        Ok(())
    }

    #[test]
    fn a_best_fit_text_too_big_at_half_its_size_shrinks_on_until_it_fits() -> Result<()> {
        // Two words 60 long, with no hyphen in either, in a box 25 wide and
        // 12 tall: at half size a word is 30 long, and on two lines the
        // words fit up to 25 / 60 of their size.
        let (styled, room) = in_box(25.0, 12.0, "best")?;
        let (_, placed) = fit(&styled, &room, |_| {
            vec![word(60.0, Break::Normal), word(60.0, Break::End)]
        });
        assert!(!placed.overflowed);
        assert!((0.4..=0.417).contains(&placed.scale), "{}", placed.scale);
        Ok(())
    }

    #[test]
    fn lines_aligned_to_a_balloon_s_edge_start_together_inside_it() -> Result<()> {
        // Twelve words 30 long in a circle 200 across.
        let frame: Frame = serde_json::from_value(serde_json::json!({
            "path": {"path": "M0 100 A100 100 0 1 0 200 100 A100 100 0 1 0 0 100 Z", "anchor": "center"}
        }))?;
        let styled = Styled {
            text: String::new(),
            spans: Vec::new(),
            vertical: false,
            align: Align::Start,
            line_height: 1.2,
            hyphenation: None,
            chinese: false,
            frame: frame.clone(),
        };
        let mut words = vec![word(30.0, Break::Normal); 11];
        words.push(word(30.0, Break::End));
        let (segments, placed) = fit(&styled, &Room::new(&frame, false), |_| words.clone());
        assert!(!placed.overflowed);
        assert!(placed.lines.len() > 2, "{}", placed.lines.len());
        let start = placed.lines[0].start;
        for line in &placed.lines {
            let used = lines::length(&segments, line.segments.clone(), placed.scale);
            // The circle's half width where the line's band is narrowest.
            let far = (line.top - 100.0)
                .abs()
                .max((line.top + line.pitch - 100.0).abs());
            let half = (100.0f32 * 100.0 - far * far).sqrt();
            assert_eq!(line.start, start);
            assert!(
                start >= 100.0 - half && start + used <= 100.0 + half,
                "{start}..{} past {half} at {}",
                start + used,
                line.top
            );
        }
        Ok(())
    }

    #[test]
    fn top_aligned_columns_use_a_lopsided_balloon_s_longer_side() -> Result<()> {
        // A balloon on page 12 of a chapter, its lower right cut away where
        // hair crosses it: a column on the right reaches far less than one on
        // the left. Ten characters 21 high fitted best, as a translation is.
        let frame: Frame = serde_json::from_value(serde_json::json!({"path": {
            "path": "M285,575 L273,577 L255,584 L237,597 L232,608 L232,828 L239,840 L256,859 L277,865 L299,863 L318,856 L315,841 L320,833 L315,825 L315,795 L324,778 L331,740 L337,719 L344,702 L349,652 L354,636 L358,617 L352,606 L358,599 L348,598 L335,586 L315,578 Z",
            "fit": "best", "anchor": "center",
        }}))?;
        let styled = Styled {
            text: String::new(),
            spans: Vec::new(),
            vertical: true,
            align: Align::Start,
            line_height: 1.2,
            hyphenation: None,
            chinese: true,
            frame: frame.clone(),
        };
        let character = |after| Segment {
            range: 0..0,
            visible_end: 0,
            advance: 21.0,
            trailing: 0.0,
            hyphen: 0.0,
            after,
            punctuation: false,
            size: 21.0,
        };
        let mut characters = vec![character(Break::Normal); 9];
        characters.push(character(Break::End));
        let (_, placed) = fit(&styled, &Room::new(&frame, true), |_| characters.clone());
        assert!(!placed.overflowed);
        assert!(placed.scale >= 1.0, "{}", placed.scale);
        assert!(
            placed
                .lines
                .iter()
                .all(|line| line.start == placed.lines[0].start)
        );
        Ok(())
    }

    #[test]
    fn a_vertical_latin_word_too_long_for_its_column_at_half_size_hyphenates() -> Result<()> {
        // Sideways Latin down a column 60 long, whose one word is 75 long at
        // half size.
        let (styled, room) = in_box(60.0, 100.0, "shrink")?;
        let styled = Styled {
            vertical: true,
            ..styled
        };
        let (segments, placed) = fit(&styled, &room, long);
        assert_eq!((segments.len(), placed.scale), (2, 0.5));
        Ok(())
    }
}
