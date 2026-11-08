//! Line breaking: which break opportunities end lines, chosen over the whole
//! text so lines fill their room the way the text's balance asks.

use std::ops::Range;

use super::breaks::{Break, Segment};

/// How lines fill their room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Balance {
    /// Every line, the last included, fills the same share of its room: path
    /// text keeps its outline's shape, and centred text is balanced.
    Even,
    /// Lines fill their room and a paragraph's last line may stop short.
    Ragged,
    /// Lines break only where the text does.
    Unbounded,
}

/// The lines chosen: each a range of segments, and whether any is longer
/// than its room.
#[derive(Clone, Debug)]
pub(super) struct Lines {
    pub lines: Vec<Range<usize>>,
    pub overflowed: bool,
}

/// How many lines in a row may end with a hyphen, plus one: Photoshop's
/// default hyphen limit is two.
const RUNS: usize = 3;

fn penalty(segment: &Segment) -> f32 {
    match segment.after {
        Break::Mandatory | Break::End => 0.0,
        Break::Normal if segment.punctuation => 0.0,
        // A break inside a phrase rather than after punctuation.
        Break::Normal => 2.0,
        Break::Hyphen => 30.0,
        Break::Anywhere => 300.0,
    }
}

/// The length of segments `range` set on one line, at `scale`.
pub(super) fn length(segments: &[Segment], range: Range<usize>, scale: f32) -> f32 {
    let Some(last) = range.end.checked_sub(1).map(|at| &segments[at]) else {
        return 0.0;
    };
    let advance: f32 = segments[range.clone()].iter().map(|m| m.advance).sum();
    ended(advance, last, scale)
}

/// The length of a line whose segments advance `advance`, ending with
/// `last`, at `scale`.
fn ended(advance: f32, last: &Segment, scale: f32) -> f32 {
    let hyphen = if last.after == Break::Hyphen {
        last.hyphen
    } else {
        0.0
    };
    (advance - last.trailing + hyphen) * scale
}

/// The cost of a line `used` long ending with `last`: its unfilled share
/// squared, plus its break's penalty. A line longer than its room costs far
/// more, so the least overflowing breaks win when nothing fits.
fn line_cost(last: &Segment, used: f32, room: f32, balance: Balance) -> f32 {
    let paragraph_end = matches!(last.after, Break::Mandatory | Break::End);
    let mut cost = penalty(last);
    if used > room + 0.01 {
        cost += 1_000_000.0 * (1.0 + (used - room) / room.max(1.0));
    } else if room > 0.0 && !(balance == Balance::Ragged && paragraph_end) {
        let slack = (room - used) / room;
        cost += 100.0 * slack * slack;
    }
    cost
}

/// The segments a line starting at `start` may end after, each with the
/// line's length: never past a break in the text, and not much longer than
/// its room, though always reaching a break without a hyphen, so the hyphen
/// limit can be met. The advances add up in order, as `length` adds them.
fn ends(
    segments: &[Segment],
    start: usize,
    scale: f32,
    room: f32,
) -> impl Iterator<Item = (usize, f32)> + '_ {
    let mut advance = 0.0;
    let mut stop = false;
    (start + 1..=segments.len()).map_while(move |end| {
        if stop {
            return None;
        }
        let last = &segments[end - 1];
        advance += last.advance;
        let used = ended(advance, last, scale);
        let long = used > room * 2.0 + 1.0;
        stop = last.after == Break::Mandatory
            || (long && end > start + 1 && last.after != Break::Hyphen);
        Some((end, used))
    })
}

/// Breaks `segments` into lines. With `count`, the text takes exactly that
/// many lines and line `i` has room `room(i)`, as in a balloon; without it,
/// every line has room `room(0)`. `None` when no `count` lines hold it.
pub(super) fn break_lines(
    segments: &[Segment],
    scale: f32,
    balance: Balance,
    count: Option<usize>,
    room: impl Fn(usize) -> f32,
) -> Option<Lines> {
    let lines = match balance {
        _ if segments.is_empty() => Vec::new(),
        Balance::Unbounded => paragraphs(segments),
        Balance::Even | Balance::Ragged => solve(segments, scale, balance, count, &room)?,
    };
    let room_of = |line: usize| room(if count.is_some() { line } else { 0 });
    let overflowed = balance != Balance::Unbounded
        && lines.iter().enumerate().any(|(line, range)| {
            length(segments, range.clone(), scale) > room_of(line).max(0.0) + 0.01
        });
    Some(Lines { lines, overflowed })
}

/// Each paragraph on a line of its own.
fn paragraphs(segments: &[Segment]) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (at, segment) in segments.iter().enumerate() {
        if matches!(segment.after, Break::Mandatory | Break::End) {
            lines.push(start..at + 1);
            start = at + 1;
        }
    }
    if start < segments.len() {
        lines.push(start..segments.len());
    }
    lines
}

/// The cheapest lines, found over states of (line, end, lines a hyphen ends
/// in a row there); without `count`, every line is line 0.
fn solve(
    segments: &[Segment],
    scale: f32,
    balance: Balance,
    count: Option<usize>,
    room: &impl Fn(usize) -> f32,
) -> Option<Vec<Range<usize>>> {
    let n = segments.len();
    let at = |line: usize, end: usize, run: usize| (line * (n + 1) + end) * RUNS + run;
    let mut cost = vec![f32::INFINITY; (count.unwrap_or(0) + 1) * (n + 1) * RUNS];
    let mut from = vec![0; cost.len()];
    cost[0] = 0.0;
    let passes: Vec<(usize, usize)> = match count {
        None => vec![(0, 0)],
        Some(count) => (0..count).map(|line| (line, line + 1)).collect(),
    };
    for (line, next) in passes {
        let room = room(line).max(0.0);
        for start in 0..n {
            if (0..RUNS).all(|run| !cost[at(line, start, run)].is_finite()) {
                continue;
            }
            for (end, used) in ends(segments, start, scale, room) {
                let last = &segments[end - 1];
                let line_cost = line_cost(last, used, room, balance);
                for run in 0..RUNS {
                    let state = at(line, start, run);
                    let run = match last.after {
                        Break::Hyphen => run + 1,
                        _ => 0,
                    };
                    if !cost[state].is_finite() || run == RUNS {
                        continue;
                    }
                    let total = cost[state] + line_cost;
                    let target = at(next, end, run);
                    if total < cost[target] {
                        cost[target] = total;
                        from[target] = state;
                    }
                }
            }
        }
    }
    let last = (0..RUNS)
        .map(|run| at(count.unwrap_or(0), n, run))
        .min_by(|a, b| cost[*a].total_cmp(&cost[*b]))
        .expect("there is a state");
    cost[last].is_finite().then(|| walk_back(&from, last, n))
}

/// The lines ending at each state on the way from `state` back to the
/// first, in order.
fn walk_back(from: &[usize], mut state: usize, n: usize) -> Vec<Range<usize>> {
    let end_of = |state: usize| state / RUNS % (n + 1);
    let mut lines = Vec::new();
    while state != 0 {
        lines.push(end_of(from[state])..end_of(state));
        state = from[state];
    }
    lines.reverse();
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(advance: f32) -> Segment {
        Segment {
            range: 0..0,
            visible_end: 0,
            advance: advance + 1.0,
            trailing: 1.0,
            hyphen: 0.0,
            after: Break::Normal,
            punctuation: false,
            size: 0.0,
        }
    }

    fn text(words: &[f32]) -> Vec<Segment> {
        let mut segments: Vec<Segment> = words.iter().map(|w| word(*w)).collect();
        segments.last_mut().expect("a word").after = Break::End;
        segments
    }

    #[test]
    fn even_lines_share_their_room_and_ragged_ones_fill_first() {
        // Six words of 10 in a room of 45: ragged takes 4 + 2, even 3 + 3.
        let segments = text(&[10.0; 6]);
        let ragged = break_lines(&segments, 1.0, Balance::Ragged, None, |_| 45.0).expect("lines");
        assert_eq!(ragged.lines, [0..4, 4..6]);
        let even = break_lines(&segments, 1.0, Balance::Even, None, |_| 45.0).expect("lines");
        assert_eq!(even.lines, [0..3, 3..6]);
        assert!(!even.overflowed);
    }

    #[test]
    fn a_line_never_runs_past_a_break_in_the_text() {
        let mut segments = text(&[5.0; 4]);
        segments[0].after = Break::Mandatory;
        let lines = break_lines(&segments, 1.0, Balance::Even, None, |_| 100.0).expect("lines");
        assert_eq!(lines.lines[0], 0..1);
    }

    #[test]
    fn at_most_two_lines_in_a_row_end_with_a_hyphen() {
        // One word in five hyphenated pieces, each filling a line.
        let mut segments = text(&[18.0; 5]);
        for segment in &mut segments[..4] {
            segment.trailing = 0.0;
            segment.advance = 18.0;
            segment.after = Break::Hyphen;
            segment.hyphen = 2.0;
        }
        let lines = break_lines(&segments, 1.0, Balance::Ragged, None, |_| 20.0).expect("lines");
        let hyphenated: Vec<bool> = lines
            .lines
            .iter()
            .map(|range| segments[range.end - 1].after == Break::Hyphen)
            .collect();
        assert!(!hyphenated.windows(3).any(|run| run.iter().all(|h| *h)));
    }

    #[test]
    fn a_word_too_long_for_its_room_overflows() {
        let segments = text(&[50.0]);
        let lines = break_lines(&segments, 1.0, Balance::Even, None, |_| 20.0).expect("lines");
        assert!(lines.overflowed);
    }
}
