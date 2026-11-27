//! Lines fitted to a text's room: at its size when they fit, or in a best-fit
//! frame at the largest scale above it that fits; else, as the frame allows,
//! at the largest scale down to half that fits with whole words, then at half
//! with a path's inset halved, unless one hyphen keeps the text a tenth
//! larger, then hyphenated, and a best-fit text below half as it last
//! broke until it fits. A word is never split without a hyphen: what still
//! doesn't fit runs past its room, hyphenated where that runs it less far.

use std::ops::{Range, RangeInclusive};

use koharu_scene::{Align, Anchor};
use kurbo::Rect;

use super::{
    breaks::{Break, Breaking, Segment},
    lines::{self, Balance},
    room::{self, Outline, Room, Spans, fills, logical, physical, shrinks},
    style::Styled,
};

/// The largest scale a best-fit text grows to, and its inverse the smallest
/// it shrinks to, bounds that only text no room limits, such as one with no
/// line height, or a room too small for anything, reaches.
const CEILING: f32 = 256.0;

/// Whole words drawn at less of their size than this take a hyphen when that
/// makes them `GAIN` times larger: under it a text looks smaller than its
/// neighbours.
const SMALL: f32 = 0.9;
const GAIN: f32 = 1.1;

/// Of an em, how much further past where one of them could start a best-fit
/// text's lines may start, sharing an edge, than at its set size. Growing to
/// the largest size its room held put a text's short last column in its
/// balloon's curved corner, which held every column's head 1.4 em down.
const SUNK: f32 = 0.5;

/// Of an em, how far along them a path's columns may sit from the middle of
/// their room: a text off it shrinks, down to `CENTRED` of its size, until it
/// sits there, and grows only while it sits no more than that further off. A
/// short column fitted only where a tapering balloon is widest, well above
/// its middle.
const DRIFT: f32 = 0.25;
const CENTRED: f32 = 0.85;

/// Lines placed at one scale.
#[derive(Clone, Debug)]
pub(super) struct Placed {
    pub scale: f32,
    pub lines: Vec<PlacedLine>,
    /// The box the text was laid out in, in its own coordinates.
    pub frame: Rect,
    pub overflowed: bool,
    /// How far the edge lines in a path share lies past where the earliest of
    /// them could start, as a curve of the path holds one back.
    pub sunk: f32,
    /// How far a path's columns sit along them from the middle of their room.
    pub off: f32,
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
    /// How much of the box, at its top, is leading its ruby and emphasis
    /// marks added.
    pub lead: f32,
    /// How much of the box, at its foot, is leading emphasis marks under it
    /// added.
    pub trail: f32,
    /// The paragraph ends with this line.
    pub last: bool,
}

impl PlacedLine {
    /// Its box without the leading its ruby and emphasis marks added: its
    /// top and pitch.
    pub(super) fn letters(&self) -> (f32, f32) {
        (self.top + self.lead, self.pitch - self.lead - self.trail)
    }
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
        let set = centred(styled, room, &words, set);
        let placed = match fills(&styled.frame) {
            true => grow(styled, room, &words, set),
            false => set,
        };
        return (words, placed);
    }
    // Shrinking stops at half the set size. Whole words shrink first, as
    // Comicraft avoids a hyphen whenever it can, and at the floor a balloon's
    // margin narrows to half, short of letters on its border, which read as
    // cut off.
    let floor = if shrinks(&styled.frame) { 0.5 } else { 1.0 };
    let mut placed = largest_fit(
        floor,
        |scale| overflows(styled, room, &words, scale),
        |scale| match scale == 1.0 {
            // The text at its size is placed already.
            true => set.clone(),
            false => place(styled, room, &words, scale),
        },
    );
    let inset = match room {
        Room::Path {
            outline,
            anchor,
            inset,
        } if placed.overflowed => {
            let inset = inset / 2.0;
            placed = in_path(styled, &words, floor, outline, *anchor, inset).place();
            Some((outline, *anchor, inset))
        }
        _ => None,
    };
    // A hyphen is the lesser evil where a long word would leave the text
    // small, but only one: a narrow balloon's text took four to grow a tenth.
    if styled.hyphenation.is_some()
        && shrinks(&styled.frame)
        && (placed.overflowed || placed.scale < SMALL)
    {
        let least = match placed.overflowed {
            true => floor,
            false => GAIN * placed.scale,
        };
        let halves = segments(Breaking::Halves);
        let halved = largest_fit(
            least,
            |scale| overflows(styled, room, &halves, scale),
            |scale| {
                let mut placed = place(styled, room, &halves, scale);
                placed.overflowed |= hyphens(&halves, &placed) > 1;
                placed
            },
        );
        if !halved.overflowed {
            return (halves, halved);
        }
    }
    if !placed.overflowed {
        return (words, placed);
    }
    // Then words hyphenate at the floor, where the fewest need it, as text
    // whose language hyphenates does, vertical too; CJK breaks between
    // characters already.
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
            Some((hyphens, hyphenated))
        }
        None => None,
    };
    // A best-fit text has no floor of its own: as Publisher's Best Fit, it
    // shrinks on, as it last broke, until its room holds it, as far below
    // its size as it may grow above.
    if fills(&styled.frame) {
        let segments = hyphens.map_or(words, |(hyphens, _)| hyphens);
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
    // Nothing fits: the text runs past its room, centred, into its margin
    // first, and hyphenated when that keeps it nearer the room, as a word too
    // long for a narrow balloon reads better broken than running over art.
    let past = |segments: &[Segment], placed: &Placed| {
        let frame = logical(styled.vertical).transform_rect_bbox(placed.frame);
        placed
            .lines
            .iter()
            .map(|line| {
                let along = lines::length(segments, line.segments.clone(), placed.scale);
                let across =
                    (frame.y0 as f32 - line.top).max(line.top + line.pitch - frame.y1 as f32);
                (along - line.room).max(across)
            })
            .fold(0.0, f32::max)
    };
    match hyphens {
        Some((hyphens, hyphenated)) if past(&hyphens, &hyphenated) < past(&words, &placed) => {
            (hyphens, hyphenated)
        }
        _ => (words, placed),
    }
}

/// `set`, or when its columns sit more than `DRIFT` off the middle of their
/// room, the largest scale down to `CENTRED` at which `segments` fit in it.
fn centred(styled: &Styled, room: &Room, segments: &[Segment], set: Placed) -> Placed {
    let size = segments
        .iter()
        .map(|segment| segment.size)
        .fold(0.0, f32::max);
    let off = |placed: &Placed| placed.off / (size * placed.scale).max(f32::EPSILON);
    if off(&set) <= DRIFT {
        return set;
    }
    let steps = ((1.0 - CENTRED) / 0.03).round() as usize;
    (1..=steps)
        .map(|step| 1.0 - 0.03 * step as f32)
        .map(|scale| place(styled, room, segments, scale))
        .find(|placed| !placed.overflowed && off(placed) <= DRIFT)
        .unwrap_or(set)
}

/// The largest scale above 1 at which `segments`, which fit at 1 as `set`,
/// still fit, breaking between words only: the scale doubles until they
/// don't, then the gap is halved as shrinking halves it. A word that outgrows
/// its line ends the growth rather than being hyphenated, a drawn-out sound
/// too, which breaks only where its set size needs it, and so does a curve
/// sinking the edge its lines share `SUNK` further, or a path's shape moving
/// its columns `DRIFT` further off the middle of their room.
fn grow(styled: &Styled, room: &Room, segments: &[Segment], set: Placed) -> Placed {
    let overflows = |scale| overflows(styled, room, segments, scale);
    let set_hyphens = hyphens(segments, &set);
    let size = segments
        .iter()
        .map(|segment| segment.size)
        .fold(0.0, f32::max);
    let em = |placed: &Placed| (size * placed.scale).max(f32::EPSILON);
    let set_sunk = set.sunk / em(&set);
    let set_off = set.off / em(&set);
    let at = |scale| {
        let mut placed = place(styled, room, segments, scale);
        placed.overflowed |= hyphens(segments, &placed) > set_hyphens
            || placed.sunk / em(&placed) > set_sunk + SUNK
            || placed.off / em(&placed) > set_off + DRIFT;
        placed
    };
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

/// How many of `placed`'s lines of `segments` end in a hyphen.
fn hyphens(segments: &[Segment], placed: &Placed) -> usize {
    (placed.lines.iter())
        .filter(|line| {
            !line.last
                && (line.segments.clone().last())
                    .is_some_and(|at| segments[at].after == Break::Hyphen)
        })
        .count()
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

/// A line's pitch, and how much of it is leading its ruby and emphasis
/// marks added before and after its letters: its largest size, scaled, times
/// the line height, and as CSS adds leading to a line with ruby, and so with
/// emphasis marks, enough that what reaches past its letters would clear
/// those of a line of the same beside it.
fn pitch(segments: &[Segment], scale: f32, line_height: f32) -> (f32, f32, f32) {
    let largest = |of: fn(&Segment) -> f32| segments.iter().map(of).fold(0.0, f32::max);
    let size = largest(|segment| segment.size);
    let set = size * line_height;
    let leading = |reach: f32| match reach {
        0.0 => 0.0,
        reach => (reach + size * 0.5 - set).max(0.0),
    };
    let (lead, trail) = (
        leading(largest(|segment| segment.rise)),
        leading(largest(|segment| segment.sink)),
    );
    ((set + lead + trail) * scale, lead * scale, trail * scale)
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
        sunk: 0.0,
        off: 0.0,
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
        .map(|range| pitch(&segments[range.clone()], scale, styled.line_height).0)
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
        sunk: 0.0,
        off: 0.0,
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
        top += pitch.0;
        total += pitch.0;
    }
    (placed, total)
}

fn line(
    segments: &[Segment],
    range: Range<usize>,
    start: f32,
    room: f32,
    top: f32,
    (pitch, lead, trail): (f32, f32, f32),
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
        lead,
        trail,
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
    /// Whether the lines are columns, which slide across their room off a
    /// curve holding down the edge they share.
    vertical: bool,
    pitch: f32,
    lead: f32,
    trail: f32,
    /// The letters' em across a line, which keeps the air from the outline:
    /// with the pitch's leading too, a column kept a sixth of an em more.
    em: f32,
    air: f32,
    frame: Rect,
}

/// The edge every one of `spans` reaches on `side`, which lines aligned to it
/// share.
fn shared(side: Side, spans: &[(f32, f32)]) -> f32 {
    match side {
        Side::Low => spans.iter().map(|span| span.0).fold(f32::MIN, f32::max),
        Side::High => spans.iter().map(|span| span.1).fold(f32::MAX, f32::min),
    }
}

/// Where a block of lines sharing `side`'s `edge`, each `used` long in its
/// one of `spans`, starts along the inline axis: centred on `centre`, unless
/// that takes it past the shared edge or a line past its span's far end.
fn block(side: Side, edge: f32, centre: f32, spans: &[(f32, f32)], used: &[f32]) -> f32 {
    let longest = used.iter().copied().fold(0.0, f32::max);
    let centred = centre - longest * 0.5;
    match side {
        Side::Low => {
            let latest = (spans.iter().zip(used))
                .map(|(span, used)| span.1 - used)
                .fold(f32::MAX, f32::min);
            centred.min(latest).max(edge)
        }
        Side::High => {
            let earliest = (spans.iter().zip(used))
                .map(|(span, used)| span.0 + used)
                .fold(f32::MIN, f32::max);
            (centred + longest).max(earliest).min(edge) - longest
        }
    }
}

/// The places across their room columns sharing an edge are tried at, past
/// the one they're anchored at.
const SLIDES: usize = 8;

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
    let (pitch, lead, trail) = pitch(segments, scale, styled.line_height);
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
        vertical: styled.vertical,
        pitch: pitch.max(0.01),
        lead,
        trail,
        em: size * scale,
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
    /// cheapest count that fits wins; when none fits, the text runs past the
    /// outline across its lines, as a box's text runs past its foot, rather
    /// than along them past its sides, where a narrow balloon's lines had
    /// stuck out on both.
    fn place(&self) -> Placed {
        let (fewest, most) = self.counts();
        match self.best(fewest..=most) {
            Some((_, placed)) if !placed.overflowed => placed,
            _ => self.overflowing(),
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
    /// line's span is where its band holds the axis and keeps the air from
    /// the outline on every side. A centred line takes as
    /// much of it on both sides of the axis. Lines aligned to an edge start
    /// together, or end together, where Photoshop starts each where the shape
    /// does: they share the edge every span reaches, each runs from it to its
    /// own span's far end, so a lopsided balloon's longer side holds longer
    /// lines, and the block, as long as its longest line, is centred on the
    /// axis, or columns on the outline's middle along them, as far as that
    /// allows. `None` when `count` lines can't hold it.
    fn lines(&self, count: usize) -> Option<(f32, Placed)> {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let total = pitch * count as f32;
        let anchored = match self.anchor {
            Anchor::Start => bounds.y0 as f32 + air,
            Anchor::Center => (bounds.y0 + bounds.y1) as f32 * 0.5 - total * 0.5,
            Anchor::End => bounds.y1 as f32 - air - total,
        };
        let top = match (self.vertical, self.edge) {
            (true, Some(side)) => self.slid(anchored, count, side),
            _ => anchored,
        };
        let (axis, spans) = self.spans(top, count);
        let shared = self.edge.map(|side| (side, shared(side, &spans)));
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
        // What the lines' breaks inside words cost, each half a line left
        // empty, as `line_cost` weighs one, so a count that keeps its words
        // whole can win over one that splits them.
        let inside_words = (lines.lines.iter())
            .filter(|range| self.segments[range.end - 1].inside_word)
            .count() as f32
            * 0.25;
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
            .sum::<f32>()
            + inside_words;
        // Where a block sharing an edge starts: centred, unless that takes
        // it past the shared edge or a line past its span's far end.
        let middle = match self.vertical {
            true => self.middle(top, count, axis),
            false => None,
        };
        let centre = middle.unwrap_or(axis as f32);
        let longest = used.iter().copied().fold(0.0, f32::max);
        let block = shared.map(|(side, edge)| block(side, edge, centre, &spans, &used));
        let sunk = shared
            .map_or(0.0, |(side, edge)| {
                let held = spans.iter().filter(|(start, end)| end > start);
                match side {
                    Side::Low => edge - held.map(|span| span.0).fold(f32::MAX, f32::min),
                    Side::High => held.map(|span| span.1).fold(f32::MIN, f32::max) - edge,
                }
            })
            .max(0.0);
        let off = match (middle, block) {
            (Some(middle), Some(block)) => (block + longest * 0.5 - middle).abs(),
            _ => 0.0,
        };
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
                    (pitch, self.lead, self.trail),
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
                sunk,
                off,
            },
        ))
    }
    /// The axis `count` lines from `top` are on, and each line's span from
    /// start to end. A band with no room on the axis holds a line of none
    /// there, so a word forced into it overflows about the axis.
    fn spans(&self, top: f32, count: usize) -> (f64, Vec<(f32, f32)>) {
        let (pitch, air) = (self.pitch, self.air);
        let bands: Vec<Spans> = (0..count)
            .map(|line| {
                // The letters' em, in the middle of the line's box between its
                // ruby's and emphasis marks' leading.
                let middle = top + pitch * line as f32 + (self.lead - self.trail + pitch) * 0.5;
                let (v0, v1) = (middle - self.em * 0.5, middle + self.em * 0.5);
                self.outline
                    .padded(f64::from(v0), f64::from(v1), f64::from(air))
            })
            .collect();
        let axis = room::axis(&bands).unwrap_or(self.outline.bounds.center().x);
        let spans = bands
            .iter()
            .map(|spans| {
                match self.edge {
                    None => room::reach(spans, axis),
                    Some(_) => room::holding(spans, axis),
                }
                .map(|(start, end)| (start as f32, end as f32))
                .unwrap_or((axis as f32, axis as f32))
            })
            .collect();
        (axis, spans)
    }

    /// The middle along them of the outline on `axis`, an em wide across the
    /// middle of `count` columns from `top`. Their spans aren't the measure:
    /// the air leaves only the wide end of a tapering room, and a room's
    /// rounded foot ended a column's span sooner than the air its head kept.
    fn middle(&self, top: f32, count: usize, axis: f64) -> Option<f32> {
        let across = top + (self.pitch * count as f32 + self.lead - self.trail) * 0.5;
        let band = self.outline.band(
            f64::from(across - self.em * 0.5),
            f64::from(across + self.em * 0.5),
        );
        room::holding(&band, axis).map(|(start, end)| ((start + end) * 0.5) as f32)
    }

    /// Where `count` columns sharing `side` start across their room, from
    /// `anchored`: slid to where their block, each column an even share of
    /// the text, sits nearest the middle of the outline along them with as
    /// much room, the nearest to `anchored` of those within half a pixel,
    /// when it sits a quarter of a column off it there. A short column in a
    /// share tapering away from the balloon's side sat at the wide end until
    /// it moved over to that side. Sliding for centring alone cost a page's
    /// texts a tenth of their size. Centred across, a column beside a
    /// balloon's tail, which rounds the room's corner, held every head 40
    /// pixels down, and a short
    /// column cut off by the line splitting a balloon between two texts held
    /// a block 70 pixels up; a letterer sets the block off both.
    fn slid(&self, anchored: f32, count: usize, side: Side) -> f32 {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let length = (self.segments.iter())
            .map(|segment| segment.advance - segment.trailing)
            .sum::<f32>()
            * self.scale
            / count as f32;
        let used = vec![length; count];
        // How far off centre the block sits from `top`, and the room its lines
        // have from the edge they share.
        let key = |top: f32| {
            let (axis, spans) = self.spans(top, count);
            let edge = shared(side, &spans);
            let room: f32 = (spans.iter())
                .map(|&(start, end)| match side {
                    Side::Low => (end - edge).max(0.0),
                    Side::High => (edge - start).max(0.0),
                })
                .sum();
            let centre = self.middle(top, count, axis).unwrap_or(axis as f32);
            let block = block(side, edge, centre, &spans, &used);
            ((block + length * 0.5 - centre).abs(), room)
        };
        let (here, room) = key(anchored);
        if here <= pitch * 0.25 {
            return anchored;
        }
        // The air is kept from the letters' em, short of their boxes' ends.
        let shift = self.lead - self.trail;
        let (head, foot) = (
            (shift + pitch - self.em) * 0.5,
            (pitch - shift - self.em) * 0.5,
        );
        let (low, high) = (
            bounds.y0 as f32 + air - head,
            bounds.y1 as f32 - air - pitch * count as f32 + foot,
        );
        if high <= low {
            return anchored;
        }
        // Of the places that leave the lines as much room, so sliding doesn't
        // cost the text its size.
        let tops: Vec<(f32, f32)> = (0..=SLIDES)
            .map(|step| low + (high - low) * step as f32 / SLIDES as f32)
            .map(|top| (key(top), top))
            .filter(|((_, more), _)| *more >= room)
            .map(|((off, _), top)| (off, top))
            .chain([(here, anchored)])
            .collect();
        let best = tops.iter().map(|(off, _)| *off).fold(f32::MAX, f32::min);
        tops.iter()
            .filter(|(off, _)| *off <= best + 0.5)
            .map(|(_, top)| *top)
            .min_by(|a, b| (a - anchored).abs().total_cmp(&(b - anchored).abs()))
            .unwrap_or(anchored)
    }

    /// The text when no line count holds it: in the widest span a line's band
    /// across the outline reaches, less the air at its ends, or the bounds,
    /// running past the outline as overflowing box text does. The middle
    /// line's span, narrower in a balloon pinched there, had kept a word whole
    /// past its sides that its widest held in two lines.
    fn overflowing(&self) -> Placed {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let middle = (bounds.y0 + bounds.y1) * 0.5;
        let step = f64::from(pitch) * 0.5;
        let bands = ((bounds.height() - f64::from(pitch)) / step)
            .floor()
            .max(0.0) as usize;
        let (start, end) = (0..=bands)
            .filter_map(|at| {
                let v0 = bounds.y0 + step * at as f64;
                let band = self.outline.band(v0, v0 + f64::from(pitch));
                room::axis(std::slice::from_ref(&band)).and_then(|axis| room::reach(&band, axis))
            })
            .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
            .map(|(start, end)| (start + f64::from(air), end - f64::from(air)))
            .filter(|(start, end)| end > start)
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
        // Lines aligned to an edge start or end together in a block as long
        // as the longest, centred in the span as `lines` centres one: from
        // the span's start, a balloon's columns stood at its top.
        let longest = (lines.lines.iter())
            .map(|range| lines::length(self.segments, range.clone(), self.scale))
            .fold(0.0, f32::max);
        let (start, room) = match self.edge {
            Some(_) if longest < room => (start as f32 + (room - longest) * 0.5, longest),
            _ => (start as f32, room),
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
                        start,
                        room,
                        top + pitch * at as f32,
                        (pitch, self.lead, self.trail),
                    )
                })
                .collect(),
            frame: self.frame,
            overflowed: true,
            sunk: 0.0,
            off: 0.0,
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
            inside_word: false,
            size: 10.0,
            rise: 0.0,
            sink: 0.0,
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
                vietnamese: false,
                frame,
                ruby: Vec::new(),
                marks: Vec::new(),
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
            Breaking::Halves | Breaking::Hyphens => {
                vec![word(75.0, Break::Hyphen), word(75.0, Break::End)]
            }
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

    /// The lines centred `words` take at their set size in a balloon `path`
    /// outlines, with no margin.
    fn balloon_lines(path: &str, words: Vec<Segment>) -> Result<Vec<Range<usize>>> {
        let (mut styled, _) = in_box(0.0, 0.0, "none")?;
        styled.align = Align::Center;
        styled.frame = serde_json::from_value(serde_json::json!({
            "path": {"path": path, "fit": "none", "anchor": "center", "inset": 0}
        }))?;
        let (_, placed) = fit(&styled, &Room::new(&styled.frame, false), |_| words.clone());
        assert!(!placed.overflowed);
        Ok(placed.lines.into_iter().map(|line| line.segments).collect())
    }

    #[test]
    fn a_balloon_keeps_a_word_whole_when_both_layouts_fit() -> Result<()> {
        // The word of the second and third takes 50 pixels including its
        // space. At 51 both layouts fit; at 49 it must split to fit on two
        // lines.
        let lines = |width: u32, inside_word| {
            let words = vec![
                word(18.0, Break::Normal),
                Segment {
                    inside_word,
                    ..word(20.0, Break::Normal)
                },
                word(29.0, Break::End),
            ];
            balloon_lines(&format!("M0 0 H{width} V25 H0 Z"), words)
        };
        assert_eq!(lines(51, false)?, [0..2, 2..3]);
        assert_eq!(lines(51, true)?, [0..1, 1..3]);
        assert_eq!(lines(49, true)?, [0..2, 2..3]);
        Ok(())
    }

    #[test]
    fn a_balloon_takes_another_line_count_to_keep_a_word_whole() -> Result<()> {
        // A balloon 50 wide but where two lines' letters fall, from 7 to 17
        // and from 19 to 29 down, `wide` across. Six words 22 long fill
        // three lines of 50 two to a line, or two of 100 three to a line,
        // which leave more room unfilled; the second and third words make one,
        // which a line of the three splits.
        let lines = |wide: f64, inside_word| {
            let (left, right) = (50.0 - wide / 2.0, 50.0 + wide / 2.0);
            let path = format!(
                "M25 0 H75 V7 H{right} V17 H75 V19 H{right} V29 H75 V36 H25 V29 H{left} V19 \
                 H25 V17 H{left} V7 H25 Z"
            );
            let mut words = vec![word(22.0, Break::Normal); 6];
            words[1].inside_word = inside_word;
            words[5].after = Break::End;
            balloon_lines(&path, words)
        };
        assert_eq!(lines(100.0, false)?, [0..2, 2..4, 4..6]);
        assert_eq!(lines(100.0, true)?, [0..3, 3..6]);
        // Where two lines can't hold three words, the word splits.
        assert_eq!(lines(60.0, true)?, [0..2, 2..4, 4..6]);
        Ok(())
    }

    #[test]
    fn a_balloon_s_text_too_long_for_it_runs_past_its_top_and_foot_not_its_sides() -> Result<()> {
        // Eight words 25 long at their set size in a balloon 60 wide and 40
        // tall, whose margin leaves lines 40 long: a word to a line.
        let frame: Frame = serde_json::from_value(serde_json::json!({
            "path": {"path": "M0 0 L60 0 L60 40 L0 40 Z", "fit": "none", "anchor": "center"}
        }))?;
        let styled = Styled {
            text: String::new(),
            spans: Vec::new(),
            vertical: false,
            align: Align::Center,
            line_height: 1.2,
            hyphenation: None,
            chinese: false,
            vietnamese: false,
            frame: frame.clone(),
            ruby: Vec::new(),
            marks: Vec::new(),
        };
        let mut words = vec![word(25.0, Break::Normal); 7];
        words.push(word(25.0, Break::End));
        let (segments, placed) = fit(&styled, &Room::new(&frame, false), |_| words.clone());
        assert!(placed.overflowed);
        assert_eq!(placed.lines.len(), 8);
        for line in &placed.lines {
            let used = lines::length(&segments, line.segments.clone(), placed.scale);
            assert!(
                used <= line.room && line.room <= 60.0,
                "{used} in {}",
                line.room
            );
        }
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
            vietnamese: false,
            frame: frame.clone(),
            ruby: Vec::new(),
            marks: Vec::new(),
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
            vietnamese: false,
            frame: frame.clone(),
            ruby: Vec::new(),
            marks: Vec::new(),
        };
        let character = |after| Segment {
            range: 0..0,
            visible_end: 0,
            advance: 21.0,
            trailing: 0.0,
            hyphen: 0.0,
            after,
            punctuation: false,
            inside_word: false,
            size: 21.0,
            rise: 0.0,
            sink: 0.0,
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
    fn a_text_takes_one_hyphen_and_only_where_that_makes_it_a_tenth_larger() -> Result<()> {
        // In a box 90 wide it fits whole at 0.6 of its size, and its halves
        // at theirs.
        let (styled, room) = in_box(90.0, 100.0, "shrink")?;
        let (segments, placed) = fit(&styled, &room, long);
        assert_eq!((segments.len(), placed.scale), (2, 1.0));
        // In one 140 wide it fits whole above nine tenths.
        let (styled, room) = in_box(140.0, 100.0, "shrink")?;
        let (segments, placed) = fit(&styled, &room, long);
        assert_eq!(segments.len(), 1);
        assert!((0.93..0.94).contains(&placed.scale), "{}", placed.scale);
        // In one 90 wide and 15 tall, a line of it at 0.6 is 7.2 tall and
        // two of its halves fit at 0.625, not a tenth larger.
        let (styled, room) = in_box(90.0, 15.0, "shrink")?;
        let (segments, placed) = fit(&styled, &room, long);
        assert_eq!(segments.len(), 1);
        assert!((0.59..=0.6).contains(&placed.scale), "{}", placed.scale);
        // Two such words, which grow only with a hyphen each, stay whole.
        let (styled, room) = in_box(90.0, 100.0, "shrink")?;
        let (segments, placed) = fit(&styled, &room, |breaking| {
            let mut first = long(breaking);
            if let Some(last) = first.last_mut() {
                last.after = Break::Normal;
            }
            [first, long(breaking)].concat()
        });
        assert_eq!(segments.len(), 2);
        assert!((0.59..=0.6).contains(&placed.scale), "{}", placed.scale);
        Ok(())
    }

    #[test]
    fn a_vertical_latin_word_too_long_for_its_column_at_half_size_hyphenates() -> Result<()> {
        // Sideways Latin down a column 60 long, whose one word is 75 long at
        // half size, and whose halves with a hyphen are 60 long at three
        // quarters.
        let (styled, room) = in_box(60.0, 100.0, "shrink")?;
        let styled = Styled {
            vertical: true,
            ..styled
        };
        let (segments, placed) = fit(&styled, &room, long);
        assert_eq!(segments.len(), 2);
        assert!((0.74..=0.75).contains(&placed.scale), "{}", placed.scale);
        Ok(())
    }
}
