//! Lines fitted to a text's room: at its size when they fit; else, as the
//! frame allows, at the largest scale down to half that fits, breaking
//! between words, then at hyphenation points, then anywhere in a word.

use std::ops::{Range, RangeInclusive};

use koharu_scene::{Align, Anchor};
use vello::kurbo::Rect;

use super::{
    breaks::{Break, Breaking, Segment},
    lines::{self, Balance},
    room::{Outline, Room, physical, shrinks},
    style::Styled,
};

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
        return (words, set);
    }
    // Shrinking stops at half the set size.
    let floor = if shrinks(&styled.frame) { 0.5 } else { 1.0 };
    let mut modes = vec![words];
    // Vertical text is CJK, which breaks between characters and has no
    // hyphenation.
    if !styled.vertical && styled.hyphenation.is_some() {
        modes.push(segments(Breaking::Hyphens));
    }
    let overflows = |segments: &[Segment], scale| overflows(styled, room, segments, scale);
    let all = |scale| modes.iter().all(|segments| overflows(segments, scale));
    let (mut mode, mut placed) = largest_fit(floor, all, |scale| {
        // The text at its size is placed already.
        let words = match scale == 1.0 {
            true => set.clone(),
            false => place(styled, room, &modes[0], scale),
        };
        // A word wider than its line, alone on it, is hyphenated before the
        // text shrinks, since shrinking until it fits takes a balloon's
        // narrow lines toward the floor. Lines that are merely full shrink,
        // as Comicraft avoids a hyphen whenever it can.
        let long = words.lines.iter().any(|line| {
            line.segments.len() == 1
                && lines::length(&modes[0], line.segments.clone(), scale) > line.room + 0.01
        });
        match modes.get(1) {
            Some(hyphens) if words.overflowed && (long || scale <= floor) => {
                (1, place(styled, room, hyphens, scale))
            }
            _ => (0, words),
        }
    });
    // Breaking anywhere in a word is the last resort, once nothing else fits
    // at the floor.
    if placed.overflowed {
        modes.push(segments(Breaking::Anywhere));
        mode = modes.len() - 1;
        let segments = &modes[mode];
        placed = largest_fit(
            floor,
            |scale| overflows(segments, scale),
            |scale| ((), place(styled, room, segments, scale)),
        )
        .1;
    }
    (modes.swap_remove(mode), placed)
}

/// The largest scale from 1 down to `floor` at which `at` fits the text, else
/// the text at `floor`, with what `at` returned beside it. It probes down from
/// the top in eighths, because a path's room changes with the line count, so
/// a smaller size can fail where a larger one fits; then halves the step
/// eight times. Above the floor only whether the text fits matters, so a
/// scale `overflows` is sure of isn't laid out.
fn largest_fit<T>(
    floor: f32,
    overflows: impl Fn(f32) -> bool,
    at: impl Fn(f32) -> (T, Placed),
) -> (T, Placed) {
    if floor >= 1.0 || !overflows(1.0) {
        let set = at(1.0);
        if !set.1.overflowed || floor >= 1.0 {
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
        if !candidate.1.overflowed {
            return halve(&overflows, &at, scale, above, candidate);
        }
        above = scale;
    }
    let scale = 1.0 - step * 8.0;
    let candidate = at(scale);
    match candidate.1.overflowed {
        true => candidate,
        false => halve(&overflows, &at, scale, above, candidate),
    }
}

/// The largest scale between `low`, where the text fits as `best`, and
/// `high`, where it doesn't, found by halving the gap eight times.
fn halve<T>(
    overflows: &impl Fn(f32) -> bool,
    at: &impl Fn(f32) -> (T, Placed),
    mut low: f32,
    mut high: f32,
    mut best: (T, Placed),
) -> (T, Placed) {
    for _ in 0..8 {
        let middle = (low + high) * 0.5;
        if overflows(middle) {
            high = middle;
            continue;
        }
        let candidate = at(middle);
        if candidate.1.overflowed {
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

fn in_path<'a>(
    styled: &Styled,
    segments: &'a [Segment],
    scale: f32,
    outline: &'a Outline,
    anchor: Anchor,
    inset: f32,
) -> InPath<'a> {
    let size = segments.iter().map(|s| s.size).fold(0.0, f32::max);
    InPath {
        segments,
        scale,
        outline,
        anchor,
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
    pitch: f32,
    air: f32,
    frame: Rect,
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

    /// The text in `count` lines, each taking the widest span inside the
    /// outline all along its band widened by the air, less the air at both
    /// ends; and what they cost. `None` when `count` lines can't hold it.
    fn lines(&self, count: usize) -> Option<(f32, Placed)> {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let total = pitch * count as f32;
        let top = match self.anchor {
            Anchor::Start => bounds.y0 as f32 + air,
            Anchor::Center => (bounds.y0 + bounds.y1) as f32 * 0.5 - total * 0.5,
            Anchor::End => bounds.y1 as f32 - air - total,
        };
        let spans: Vec<(f32, f32)> = (0..count)
            .map(|line| {
                let v0 = top + pitch * line as f32;
                self.outline
                    .widest(f64::from(v0 - air), f64::from(v0 + pitch + air))
                    .map_or((0.0, 0.0), |(start, end)| {
                        let (start, end) = (start as f32 + air, end as f32 - air);
                        (start, (end - start).max(0.0))
                    })
            })
            .collect();
        let lines = lines::break_lines(
            self.segments,
            self.scale,
            Balance::Even,
            Some(count),
            |line| spans[line].1,
        )?;
        let cost = cost_of(self.segments, &lines.lines, self.scale, |line| {
            spans[line].1
        });
        let placed = lines
            .lines
            .into_iter()
            .enumerate()
            .map(|(at, range)| {
                let (start, room) = spans[at];
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
    /// The text when no line count holds it: in the middle line's span,
    /// running past the outline as overflowing box text does.
    fn overflowing(&self) -> Placed {
        let (bounds, pitch, air) = (self.outline.bounds, self.pitch, self.air);
        let middle = (bounds.y0 + bounds.y1) * 0.5;
        let (start, end) = self
            .outline
            .widest(
                middle - f64::from(pitch) * 0.5,
                middle + f64::from(pitch) * 0.5,
            )
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

/// The cost `break_lines` gave the chosen lines, to compare line counts.
fn cost_of(
    segments: &[Segment],
    lines: &[Range<usize>],
    scale: f32,
    room: impl Fn(usize) -> f32,
) -> f32 {
    lines
        .iter()
        .enumerate()
        .map(|(line, range)| {
            let room = room(line);
            let used = lines::length(segments, range.clone(), scale);
            if used > room {
                1e6 * (1.0 + used - room)
            } else if room > 0.0 {
                let slack = (room - used) / room;
                slack * slack
            } else {
                0.0
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use hypher::Lang;
    use koharu_scene::Frame;

    use super::*;
    use crate::text::testing::styled;

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

    /// Text in a shrinking box `width` by `height`.
    fn in_box(width: f64, height: f64) -> Result<(Styled, Room)> {
        let frame: Frame = serde_json::from_value(serde_json::json!({
            "box": {"bounds": {"x": 0.0, "y": 0.0, "width": width, "height": height}, "fit": "shrink"}
        }))?;
        let room = Room::new(&frame, false)?;
        Ok((
            Styled {
                frame,
                hyphenation: Some(Lang::English),
                ..styled("", Vec::new())
            },
            room,
        ))
    }

    #[test]
    fn a_text_too_big_for_its_box_shrinks_to_the_largest_size_that_fits() -> Result<()> {
        // Two words 121 long together, in a box 100 wide and one line tall.
        let (styled, room) = in_box(100.0, 12.0)?;
        let (_, placed) = fit(&styled, &room, |_| {
            vec![word(60.0, Break::Normal), word(60.0, Break::End)]
        });
        assert!(!placed.overflowed);
        assert_eq!(placed.lines.len(), 1);
        // 100 / 121 at most.
        assert!((0.8..0.83).contains(&placed.scale), "{}", placed.scale);
        Ok(())
    }

    #[test]
    fn a_word_wider_than_its_line_hyphenates_before_the_text_shrinks() -> Result<()> {
        let (styled, room) = in_box(100.0, 100.0)?;
        let (segments, placed) = fit(&styled, &room, |breaking| match breaking {
            Breaking::Words => vec![word(150.0, Break::End)],
            Breaking::Hyphens | Breaking::Anywhere => {
                vec![word(75.0, Break::Hyphen), word(75.0, Break::End)]
            }
        });
        assert_eq!((segments.len(), placed.scale), (2, 1.0));
        assert!(!placed.overflowed);
        Ok(())
    }
}
