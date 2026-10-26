//! Line breaking: which break opportunities end lines, chosen over the whole
//! text so lines fill their room the way the text's shape asks.

use std::ops::Range;

/// What may end a line after a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Break {
    /// A line break in the text.
    Mandatory,
    /// Between words, or between CJK characters.
    Normal,
    /// Inside a word at a hyphenation point; a hyphen shows if a line ends here.
    Hyphen,
    /// Inside a word with no hyphenation point.
    Anywhere,
    /// The end of the text.
    End,
}

/// One unbreakable piece's measures along the inline axis, at the set size.
#[derive(Clone, Debug)]
pub(crate) struct Measure {
    pub advance: f32,
    /// Trailing white space, which hangs past a line's end.
    pub trailing: f32,
    /// The hyphen drawn when a line ends after a `Hyphen` break.
    pub hyphen: f32,
    pub after: Break,
    /// The segment ends with punctuation, the best place for a break.
    pub punctuation: bool,
}

/// How lines should fill their room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
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
pub(crate) struct Lines {
    pub lines: Vec<Range<usize>>,
    pub overflowed: bool,
}

const SLACK_WEIGHT: f32 = 100.0;
const OVERFLOW_WEIGHT: f32 = 1_000_000.0;
/// At most two lines in a row end with a hyphen, Photoshop's default
/// hyphen limit.
const HYPHEN_LIMIT: usize = 2;

fn penalty(measure: &Measure) -> f32 {
    match measure.after {
        Break::Mandatory | Break::End => 0.0,
        Break::Normal if measure.punctuation => 0.0,
        // A break inside a phrase rather than after punctuation.
        Break::Normal => 2.0,
        Break::Hyphen => 30.0,
        Break::Anywhere => 300.0,
    }
}

/// The length of segments `range` set on one line, at `scale`.
pub(crate) fn length(measures: &[Measure], range: Range<usize>, scale: f32) -> f32 {
    let Some(last) = range.end.checked_sub(1).map(|at| &measures[at]) else {
        return 0.0;
    };
    let advance: f32 = measures[range.clone()].iter().map(|m| m.advance).sum();
    let hyphen = if last.after == Break::Hyphen {
        last.hyphen
    } else {
        0.0
    };
    (advance - last.trailing + hyphen) * scale
}

/// The cost of one line: its unfilled share squared, plus its break's
/// penalty. A line longer than its room costs far more, so the least
/// overflowing breaks win when nothing fits.
fn line_cost(
    measures: &[Measure],
    range: Range<usize>,
    scale: f32,
    room: f32,
    shape: Shape,
) -> f32 {
    let last = &measures[range.end - 1];
    let used = length(measures, range, scale);
    let paragraph_end = matches!(last.after, Break::Mandatory | Break::End);
    let mut cost = penalty(last);
    if used > room + 0.01 {
        cost += OVERFLOW_WEIGHT * (1.0 + (used - room) / room.max(1.0));
    } else if room > 0.0 && !(shape == Shape::Ragged && paragraph_end) {
        let slack = (room - used) / room;
        cost += SLACK_WEIGHT * slack * slack;
    }
    cost
}

/// The segments a line starting at `start` may end after: never past a break
/// in the text, and not much longer than its room, though always reaching a
/// break without a hyphen, so the hyphen limit can be met.
fn ends(
    measures: &[Measure],
    start: usize,
    scale: f32,
    room: f32,
) -> impl Iterator<Item = usize> + '_ {
    let mut stop = false;
    (start + 1..=measures.len()).take_while(move |end| {
        if stop {
            return false;
        }
        let after = measures[end - 1].after;
        let long = length(measures, start..*end, scale) > room * 2.0 + 1.0;
        stop = after == Break::Mandatory || (long && *end > start + 1 && after != Break::Hyphen);
        true
    })
}

/// Breaks `measures` into lines. With `count`, the text takes exactly that
/// many lines and line `i` has room `room(i)`, as in a balloon; without it,
/// every line has room `room(0)`.
pub(crate) fn break_lines(
    measures: &[Measure],
    scale: f32,
    shape: Shape,
    count: Option<usize>,
    room: impl Fn(usize) -> f32,
) -> Option<Lines> {
    let n = measures.len();
    if n == 0 {
        return Some(Lines {
            lines: Vec::new(),
            overflowed: false,
        });
    }
    if shape == Shape::Unbounded {
        let mut lines = Vec::new();
        let mut start = 0;
        for (at, measure) in measures.iter().enumerate() {
            if matches!(measure.after, Break::Mandatory | Break::End) {
                lines.push(start..at + 1);
                start = at + 1;
            }
        }
        if start < n {
            lines.push(start..n);
        }
        return Some(Lines {
            lines,
            overflowed: false,
        });
    }
    // States are (line, end, hyphenated lines in a row ending there); without
    // `count`, every line is line 0.
    const RUNS: usize = HYPHEN_LIMIT + 1;
    let at = |line: usize, end: usize, run: usize| (line * (n + 1) + end) * RUNS + run;
    let end_of = |state: usize| state / RUNS % (n + 1);
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
            for run in 0..RUNS {
                let state = at(line, start, run);
                if !cost[state].is_finite() {
                    continue;
                }
                for end in ends(measures, start, scale, room) {
                    let run = match measures[end - 1].after {
                        Break::Hyphen => run + 1,
                        _ => 0,
                    };
                    if run == RUNS {
                        continue;
                    }
                    let total = cost[state] + line_cost(measures, start..end, scale, room, shape);
                    let target = at(next, end, run);
                    if total < cost[target] {
                        cost[target] = total;
                        from[target] = state;
                    }
                }
            }
        }
    }
    let mut state = (0..RUNS)
        .map(|run| at(count.unwrap_or(0), n, run))
        .min_by(|a, b| cost[*a].total_cmp(&cost[*b]))
        .expect("there is a state");
    if !cost[state].is_finite() {
        return None;
    }
    let mut ranges = Vec::new();
    while state != 0 {
        ranges.push(end_of(from[state])..end_of(state));
        state = from[state];
    }
    ranges.reverse();
    let room_of = |line: usize| match count {
        Some(_) => room(line),
        None => room(0),
    };
    let overflowed = ranges.iter().enumerate().any(|(line, range)| {
        length(measures, range.clone(), scale) > room_of(line).max(0.0) + 0.01
    });
    Some(Lines {
        lines: ranges,
        overflowed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(advance: f32) -> Measure {
        Measure {
            advance: advance + 1.0,
            trailing: 1.0,
            hyphen: 0.0,
            after: Break::Normal,
            punctuation: false,
        }
    }

    fn text(words: &[f32]) -> Vec<Measure> {
        let mut measures: Vec<Measure> = words.iter().map(|w| word(*w)).collect();
        measures.last_mut().unwrap().after = Break::End;
        measures
    }

    #[test]
    fn even_lines_share_their_room_and_ragged_ones_fill_first() {
        // Six words of 10 in a room of 45: ragged takes 4 + 2, even 3 + 3.
        let measures = text(&[10.0; 6]);
        let ragged = break_lines(&measures, 1.0, Shape::Ragged, None, |_| 45.0).unwrap();
        assert_eq!(ragged.lines, [0..4, 4..6]);
        let even = break_lines(&measures, 1.0, Shape::Even, None, |_| 45.0).unwrap();
        assert_eq!(even.lines, [0..3, 3..6]);
        assert!(!even.overflowed);
    }

    #[test]
    fn a_line_never_runs_past_a_break_in_the_text() {
        let mut measures = text(&[5.0; 4]);
        measures[0].after = Break::Mandatory;
        let lines = break_lines(&measures, 1.0, Shape::Even, None, |_| 100.0).unwrap();
        assert_eq!(lines.lines[0], 0..1);
    }

    #[test]
    fn at_most_two_lines_in_a_row_end_with_a_hyphen() {
        // One word in five hyphenated pieces, each filling a line.
        let mut measures = text(&[18.0; 5]);
        for measure in &mut measures[..4] {
            measure.trailing = 0.0;
            measure.advance = 18.0;
            measure.after = Break::Hyphen;
            measure.hyphen = 2.0;
        }
        let lines = break_lines(&measures, 1.0, Shape::Ragged, None, |_| 20.0).unwrap();
        let hyphenated: Vec<bool> = lines
            .lines
            .iter()
            .map(|range| measures[range.end - 1].after == Break::Hyphen)
            .collect();
        assert!(!hyphenated.windows(3).any(|run| run.iter().all(|h| *h)));
    }

    #[test]
    fn a_word_too_long_for_its_room_overflows() {
        let measures = text(&[50.0]);
        let lines = break_lines(&measures, 1.0, Shape::Even, None, |_| 20.0).unwrap();
        assert!(lines.overflowed);
    }
}
