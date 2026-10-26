//! A text layer's spans set in its frame: shaped, broken into lines, fitted,
//! and drawn as glyph runs, with the runs PSD export writes.

use std::ops::Range;

use anyhow::Result;
use harfrust::{Direction, Feature, Tag};
use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};
use icu_segmenter::GraphemeClusterSegmenter;
use koharu_scene::{Align, Anchor, Color};
use skrifa::{MetadataProvider, instance::Size};
use unicode_bidi::{BidiInfo, Level};
use vello::{
    FontEmbolden, Glyph, Scene,
    kurbo::{Affine, BezPath, Diagonal2, Join, PathEl, Point, Rect},
    peniko::Fill,
};

use crate::{
    fonts::Font,
    layout::{self, Break, Measure, Shape},
    segment::{HyphenationOptions, LineBreaker, hyphenation_lang_from_tag},
    shape::{ShapingOptions, TextShaper, shape_script_runs},
};

/// The floor `shrink` stops at: half the set size.
const FLOOR: f32 = 0.5;

/// One span as the renderer resolved it: its fonts, size, and paint.
#[derive(Clone, Debug)]
pub(crate) struct Span {
    pub range: Range<usize>,
    /// The font chain: the face asked for, then fallbacks.
    pub fonts: Vec<Font>,
    pub size: f32,
    pub fill: Color,
    pub stroke: Option<(Color, f32)>,
}

/// Where a text sets, in its own coordinates.
#[derive(Clone, Debug)]
pub(crate) enum Room {
    Point,
    Box {
        bounds: Rect,
        shrink: bool,
        grow: bool,
        anchor: Anchor,
    },
    Path {
        outline: BezPath,
        shrink: bool,
        anchor: Anchor,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Input {
    pub text: String,
    pub spans: Vec<Span>,
    pub vertical: bool,
    pub align: Align,
    pub line_height: f32,
    pub language: Option<String>,
    pub room: Room,
}

/// Laid-out text, as a run of one face at one size and paint.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// UTF-16 code units of the text.
    pub range: Range<usize>,
    pub post_script_name: String,
    /// The size it was drawn at.
    pub size: f32,
    pub fill: Color,
    /// The stroke colour and width.
    pub stroke: Option<(Color, f32)>,
}

/// What laying out a text resolved, for the inspector and PSD export.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedText {
    pub runs: Vec<TextRun>,
    /// The box the text was laid out in, in its own coordinates: the frame's
    /// box, a grown box, a path's bounds, or a point text's lines.
    pub frame: Rect,
    /// The scale `shrink` drew it at; 1 at its set size.
    pub scale: f32,
    /// UTF-16 offsets where a line ends with an added hyphen.
    pub hyphens: Vec<usize>,
    /// UTF-16 offsets where a line breaks inside a word without one.
    pub broken: Vec<usize>,
    pub overflowed: bool,
}

pub(crate) struct Laid {
    pub scene: Scene,
    pub resolved: ResolvedText,
    /// What the text covers, in its own coordinates.
    pub bounds: Rect,
}

/// The break modes tried in order: words only, then hyphens, then breaks
/// anywhere in a word.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    Words,
    Hyphens,
    Anywhere,
}

#[derive(Clone, Debug)]
struct Shaped {
    /// The index of the font in `Prepared::fonts`.
    font: usize,
    id: u32,
    cluster: usize,
    advance: f32,
    x_offset: f32,
    y_offset: f32,
}

#[derive(Clone, Debug)]
struct Piece {
    span: usize,
    rtl: bool,
    /// Two or three `!` and `?` set side by side in one vertical cell.
    upright_group: bool,
    glyphs: Vec<Shaped>,
    advance: f32,
}

#[derive(Clone, Debug)]
struct Segment {
    range: Range<usize>,
    /// The bytes before trailing white space.
    visible_end: usize,
    pieces: Vec<Piece>,
    hyphen: Option<Piece>,
    measure: Measure,
    size: f32,
}

struct Prepared<'a> {
    input: &'a Input,
    fonts: Vec<Font>,
    /// The first paragraph runs right to left.
    rtl: bool,
    /// Each byte's bidi embedding level.
    levels: Vec<Level>,
}

pub(crate) fn lay_out(input: &Input) -> Result<Laid> {
    let BidiInfo {
        levels, paragraphs, ..
    } = BidiInfo::new(&input.text, None);
    let mut prepared = Prepared {
        input,
        fonts: Vec::new(),
        rtl: !input.vertical
            && paragraphs
                .first()
                .is_some_and(|paragraph| paragraph.level.is_rtl()),
        levels,
    };
    let shrink = match input.room {
        Room::Point => false,
        Room::Box { shrink, .. } | Room::Path { shrink, .. } => shrink,
    };
    let floor = if shrink { FLOOR } else { 1.0 };
    // Each mode is tried only when the ones before can't fit the text.
    // Vertical text is CJK, which breaks between characters and has no
    // hyphenation.
    let mut placed = None;
    for mode in [Mode::Words, Mode::Hyphens, Mode::Anywhere] {
        if mode == Mode::Hyphens && input.vertical {
            continue;
        }
        let segments = prepared.segments(mode)?;
        let fitted = largest_fit(floor, |scale| prepared.place(&segments, scale))?;
        let fits = !fitted.overflowed;
        placed = Some((segments, fitted));
        if fits {
            break;
        }
    }
    let (segments, placed) = placed.expect("a mode ran");
    Ok(prepared.draw(&segments, placed))
}

/// The largest scale from 1 down to `floor` at which the text fits, else the
/// text at `floor`. It probes down from the top, because a path's room
/// changes with the line count, so a smaller size can fail where a larger one
/// fits.
fn largest_fit(floor: f32, at: impl Fn(f32) -> Result<Placed>) -> Result<Placed> {
    const PROBES: usize = 8;
    let mut last = at(1.0)?;
    if !last.overflowed || floor >= 1.0 {
        return Ok(last);
    }
    let step = (1.0 - floor) / PROBES as f32;
    let mut above = 1.0;
    for probe in 1..=PROBES {
        let scale = 1.0 - step * probe as f32;
        let candidate = at(scale)?;
        if candidate.overflowed {
            above = scale;
            last = candidate;
            continue;
        }
        let (mut low, mut high, mut best) = (scale, above, candidate);
        for _ in 0..8 {
            let middle = (low + high) * 0.5;
            let candidate = at(middle)?;
            if candidate.overflowed {
                high = middle;
            } else {
                low = middle;
                best = candidate;
            }
        }
        return Ok(best);
    }
    Ok(last)
}

/// Lines placed at one scale.
#[derive(Clone, Debug)]
struct Placed {
    scale: f32,
    lines: Vec<PlacedLine>,
    frame: Rect,
    overflowed: bool,
}

#[derive(Clone, Debug)]
struct PlacedLine {
    segments: Range<usize>,
    /// Where the line starts and the room it has, along the inline axis.
    start: f32,
    room: f32,
    /// The line box along the block axis.
    top: f32,
    pitch: f32,
    /// The paragraph ends with this line.
    last: bool,
}

/// A point in logical coordinates: `u` along the inline axis and `v` along
/// the block axis. Horizontal text maps them to x and y; vertical text sets
/// lines top to bottom and columns right to left, so u is y and v is -x.
fn logical(point: Point, vertical: bool) -> Point {
    if vertical {
        Point::new(point.y, -point.x)
    } else {
        point
    }
}

fn physical(u: f64, v: f64, vertical: bool) -> Point {
    if vertical {
        Point::new(-v, u)
    } else {
        Point::new(u, v)
    }
}

fn logical_rect(rect: Rect, vertical: bool) -> Rect {
    Rect::from_points(
        logical(rect.origin(), vertical),
        logical(Point::new(rect.x1, rect.y1), vertical),
    )
}

/// A closed outline's inside, in logical coordinates.
struct Outline {
    edges: Vec<(Point, Point)>,
    bounds: Rect,
}

impl Outline {
    fn new(path: &BezPath, vertical: bool) -> Self {
        let mut edges = Vec::new();
        let (mut first, mut last) = (Point::ZERO, Point::ZERO);
        vello::kurbo::flatten(path.iter(), 0.25, |element| match element {
            PathEl::MoveTo(point) => {
                if last != first {
                    edges.push((last, first));
                }
                first = logical(point, vertical);
                last = first;
            }
            PathEl::LineTo(point) => {
                let point = logical(point, vertical);
                edges.push((last, point));
                last = point;
            }
            PathEl::ClosePath => {
                if last != first {
                    edges.push((last, first));
                }
                last = first;
            }
            _ => {}
        });
        if last != first {
            edges.push((last, first));
        }
        let bounds = edges.iter().flat_map(|(a, b)| [*a, *b]).fold(
            Rect::new(
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ),
            |rect, point| rect.union_pt(point),
        );
        Self { edges, bounds }
    }

    /// The inside's spans along the inline axis at block position `v`, by the
    /// even-odd rule.
    fn spans(&self, v: f64) -> Vec<(f64, f64)> {
        let mut crossings: Vec<f64> = self
            .edges
            .iter()
            .filter(|(a, b)| (a.y <= v) != (b.y <= v))
            .map(|(a, b)| a.x + (v - a.y) / (b.y - a.y) * (b.x - a.x))
            .collect();
        crossings.sort_by(f64::total_cmp);
        crossings
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[a, b]| (*a, *b))
            .collect()
    }

    /// The widest span inside the outline all along the band `v0..v1`. It is
    /// sampled at the middles of equal parts of the band, never at its ends,
    /// so a band reaching a flat edge of the outline isn't cut to nothing.
    fn widest(&self, v0: f64, v1: f64) -> Option<(f64, f64)> {
        const SAMPLES: usize = 5;
        let at = |sample: usize| v0 + (v1 - v0) * (sample as f64 + 0.5) / SAMPLES as f64;
        let mut spans = self.spans(at(0));
        for sample in 1..SAMPLES {
            let next = self.spans(at(sample));
            spans = spans
                .iter()
                .flat_map(|(a0, a1)| {
                    next.iter().filter_map(move |(b0, b1)| {
                        let (start, end) = (a0.max(*b0), a1.min(*b1));
                        (end > start).then_some((start, end))
                    })
                })
                .collect();
        }
        spans
            .into_iter()
            .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
    }
}

impl Prepared<'_> {
    fn font(&mut self, font: &Font) -> usize {
        match self.fonts.iter().position(|known| known.same(font)) {
            Some(at) => at,
            None => {
                self.fonts.push(font.clone());
                self.fonts.len() - 1
            }
        }
    }

    fn span_at(&self, offset: usize) -> usize {
        self.input
            .spans
            .iter()
            .position(|span| span.range.contains(&offset))
            .unwrap_or(self.input.spans.len().saturating_sub(1))
    }

    /// Breaks between words, and from `Hyphens` on, at hyphenation points
    /// for the text's language, when it has patterns.
    fn breaker(&self, mode: Mode) -> LineBreaker {
        let breaker = LineBreaker::new();
        if mode == Mode::Words {
            return breaker;
        }
        match self
            .input
            .language
            .as_deref()
            .and_then(hyphenation_lang_from_tag)
        {
            // Photoshop's defaults: words of more than five letters, keeping
            // two on each side.
            Some(lang) => breaker
                .with_hyphenation(HyphenationOptions::new(lang, 6).with_fragment_bounds(2, 2)),
            None => breaker,
        }
    }

    fn segments(&mut self, mode: Mode) -> Result<Vec<Segment>> {
        let text = &self.input.text;
        let mut ranges = Vec::new();
        for segment in self.breaker(mode).line_segments(text) {
            let after = if segment.next_offset >= text.len() {
                Break::End
            } else if segment.is_mandatory {
                Break::Mandatory
            } else if segment.break_suffix.is_some() {
                Break::Hyphen
            } else {
                Break::Normal
            };
            if mode == Mode::Anywhere && after != Break::Hyphen {
                // A word may break between any two of its characters.
                let word = &text[segment.range.clone()];
                let visible = word.trim_end().len();
                let mut cuts: Vec<usize> = GraphemeClusterSegmenter::new()
                    .segment_str(word)
                    .filter(|at| *at > 0 && *at < visible)
                    .collect();
                cuts.push(word.len());
                let mut start = 0;
                for (index, cut) in cuts.iter().enumerate() {
                    let last = index + 1 == cuts.len();
                    let end = segment.range.start + cut;
                    let range = segment.range.start + start..end;
                    let next = if last { segment.next_offset } else { end };
                    ranges.push((range, next, if last { after } else { Break::Anywhere }));
                    start = *cut;
                }
            } else {
                ranges.push((segment.range, segment.next_offset, after));
            }
        }
        let categories = CodePointMapData::<GeneralCategory>::new();
        let mut segments = Vec::with_capacity(ranges.len());
        for (range, next, after) in ranges {
            let piece_text = &text[range.clone()];
            let visible_end = range.start + piece_text.trim_end().len();
            let pieces = self.shape(range.clone())?;
            let advance = pieces.iter().map(|piece| piece.advance).sum();
            let trailing = pieces
                .iter()
                .flat_map(|piece| &piece.glyphs)
                .filter(|glyph| glyph.cluster >= visible_end)
                .map(|glyph| glyph.advance)
                .sum();
            let hyphen = match after {
                Break::Hyphen => Some(self.hyphen(range.end.saturating_sub(1))?),
                _ => None,
            };
            let punctuation = text[range.start..visible_end]
                .chars()
                .next_back()
                .is_some_and(|ch| GeneralCategoryGroup::Punctuation.contains(categories.get(ch)));
            let size = pieces
                .iter()
                .map(|piece| self.input.spans[piece.span].size)
                .fold(0.0, f32::max);
            segments.push(Segment {
                range: range.start..next.max(range.end),
                visible_end,
                measure: Measure {
                    advance,
                    trailing,
                    hyphen: hyphen.as_ref().map_or(0.0, |piece| piece.advance),
                    after,
                    punctuation,
                },
                pieces,
                hyphen,
                size,
            });
        }
        Ok(segments)
    }

    /// Shapes `range` into pieces of one span and one direction.
    fn shape(&mut self, range: Range<usize>) -> Result<Vec<Piece>> {
        let text = &self.input.text;
        let mut pieces = Vec::new();
        let mut start = range.start;
        while start < range.end {
            let span = self.span_at(start);
            let rtl = !self.input.vertical && self.levels[start].is_rtl();
            let mut end = start;
            for (offset, ch) in text[start..range.end].char_indices() {
                let at = start + offset;
                if self.span_at(at) != span
                    || (!self.input.vertical && self.levels[at].is_rtl() != rtl)
                {
                    break;
                }
                end = at + ch.len_utf8();
            }
            if self.input.vertical {
                pieces.extend(self.shape_vertical(start..end, span)?);
            } else {
                pieces.push(self.shape_piece(start..end, span, rtl, false)?);
            }
            start = end;
        }
        Ok(pieces)
    }

    /// Vertical text, with runs of two or three `!` and `?` set upright side
    /// by side in one cell.
    fn shape_vertical(&mut self, range: Range<usize>, span: usize) -> Result<Vec<Piece>> {
        let text = &self.input.text;
        let mut pieces = Vec::new();
        let mut start = range.start;
        let mut chars = text[range.clone()].char_indices().peekable();
        while let Some((offset, ch)) = chars.next() {
            if !is_exclamation(ch) {
                continue;
            }
            let group_start = range.start + offset;
            let mut group_end = group_start + ch.len_utf8();
            let mut count = 1;
            while let Some(&(next, next_ch)) = chars.peek() {
                if !is_exclamation(next_ch) || count == 3 {
                    break;
                }
                group_end = range.start + next + next_ch.len_utf8();
                count += 1;
                chars.next();
            }
            if count < 2 {
                continue;
            }
            if start < group_start {
                pieces.push(self.shape_piece(start..group_start, span, false, false)?);
            }
            pieces.push(self.shape_piece(group_start..group_end, span, false, true)?);
            start = group_end;
        }
        if start < range.end {
            pieces.push(self.shape_piece(start..range.end, span, false, false)?);
        }
        Ok(pieces)
    }

    fn shape_piece(
        &mut self,
        range: Range<usize>,
        span: usize,
        rtl: bool,
        upright_group: bool,
    ) -> Result<Piece> {
        let input = self.input;
        let source = &input.text[range.clone()];
        let vertical = input.vertical && !upright_group;
        // Vertical text sets `!` and `?` in their full-width forms, which
        // stand upright in a column, and a group of them in their
        // proportional forms, side by side in one cell.
        let (text, clusters) = if input.vertical {
            exclamations(source, upright_group)
        } else {
            (source.to_owned(), (0..source.len()).collect())
        };
        let size = input.spans[span].size;
        let fonts: Vec<&Font> = input.spans[span].fonts.iter().collect();
        let features = [
            Feature::new(Tag::new(b"vert"), 1, ..),
            Feature::new(Tag::new(b"vrt2"), 1, ..),
        ];
        let options = ShapingOptions {
            direction: if vertical {
                Direction::TopToBottom
            } else if rtl {
                Direction::RightToLeft
            } else {
                Direction::LeftToRight
            },
            script: None,
            font_size: size,
            features: if vertical { &features } else { &[] },
        };
        let mut glyphs = Vec::new();
        for run in shape_script_runs(&TextShaper::new(), &text, &fonts, &options)? {
            for glyph in run.glyphs {
                let font = self.font(glyph.font);
                glyphs.push(Shaped {
                    font,
                    id: glyph.glyph_id,
                    cluster: range.start
                        + clusters.get(glyph.cluster as usize).copied().unwrap_or(0),
                    advance: if vertical {
                        glyph.y_advance.abs()
                    } else {
                        glyph.x_advance
                    },
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                });
            }
        }
        let advance = if upright_group {
            size
        } else {
            glyphs.iter().map(|glyph| glyph.advance).sum()
        };
        Ok(Piece {
            span,
            rtl,
            upright_group,
            glyphs,
            advance,
        })
    }

    fn hyphen(&mut self, at: usize) -> Result<Piece> {
        let input = self.input;
        let span = self.span_at(at);
        let size = input.spans[span].size;
        let fonts: Vec<&Font> = input.spans[span].fonts.iter().collect();
        let options = ShapingOptions {
            direction: Direction::LeftToRight,
            script: None,
            font_size: size,
            features: &[],
        };
        let mut glyphs = Vec::new();
        for run in shape_script_runs(&TextShaper::new(), "-", &fonts, &options)? {
            for glyph in run.glyphs {
                let font = self.font(glyph.font);
                glyphs.push(Shaped {
                    font,
                    id: glyph.glyph_id,
                    cluster: at,
                    advance: glyph.x_advance,
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                });
            }
        }
        let advance = glyphs.iter().map(|glyph| glyph.advance).sum();
        Ok(Piece {
            span,
            rtl: false,
            upright_group: false,
            glyphs,
            advance,
        })
    }

    fn pitch(&self, segments: &[Segment], scale: f32) -> f32 {
        let size = segments.iter().map(|s| s.size).fold(0.0, f32::max);
        size * scale * self.input.line_height
    }

    fn shape_for(&self) -> Shape {
        match (&self.input.room, self.input.align) {
            (Room::Point, _) => Shape::Unbounded,
            (Room::Path { .. }, _) | (_, Align::Center) => Shape::Even,
            _ => Shape::Ragged,
        }
    }

    fn place(&self, segments: &[Segment], scale: f32) -> Result<Placed> {
        let measures: Vec<Measure> = segments.iter().map(|s| s.measure.clone()).collect();
        let vertical = self.input.vertical;
        let shape = self.shape_for();
        let line_pitch = |range: &Range<usize>| {
            segments[range.clone()]
                .iter()
                .map(|s| s.size)
                .fold(0.0, f32::max)
                * scale
                * self.input.line_height
        };
        Ok(match &self.input.room {
            Room::Point => {
                let lines = layout::break_lines(&measures, scale, shape, None, |_| f32::INFINITY)
                    .expect("point text always breaks");
                let room = lines
                    .lines
                    .iter()
                    .map(|range| layout::length(&measures, range.clone(), scale))
                    .fold(0.0, f32::max);
                let pitches: Vec<f32> = lines.lines.iter().map(line_pitch).collect();
                let total: f32 = pitches.iter().sum();
                // Lines go down from y = 0, and columns left from x = 0.
                let mut top = 0.0;
                let mut placed = Vec::new();
                for (range, pitch) in lines.lines.into_iter().zip(pitches) {
                    placed.push(self.line(&measures, range, 0.0, room, top, pitch));
                    top += pitch;
                }
                let frame = Rect::from_points(
                    physical(0.0, 0.0, vertical),
                    physical(f64::from(room), f64::from(total), vertical),
                );
                Placed {
                    scale,
                    lines: placed,
                    frame,
                    overflowed: false,
                }
            }
            Room::Box {
                bounds,
                grow,
                anchor,
                ..
            } => {
                let logical_box = logical_rect(*bounds, vertical);
                let room = logical_box.width() as f32;
                let lines = layout::break_lines(&measures, scale, shape, None, |_| room)
                    .expect("box text always breaks");
                let pitches: Vec<f32> = lines.lines.iter().map(line_pitch).collect();
                let total: f32 = pitches.iter().sum();
                let extent = logical_box.height() as f32;
                let (v0, v1) = (logical_box.y0 as f32, logical_box.y1 as f32);
                let (frame_v0, frame_v1) = if *grow {
                    match anchor {
                        Anchor::Start => (v0, v0 + total),
                        Anchor::Center => {
                            let middle = (v0 + v1) * 0.5;
                            (middle - total * 0.5, middle + total * 0.5)
                        }
                        Anchor::End => (v1 - total, v1),
                    }
                } else {
                    (v0, v1)
                };
                let mut top = match anchor {
                    Anchor::Start => frame_v0,
                    Anchor::Center => frame_v0 + (frame_v1 - frame_v0 - total) * 0.5,
                    Anchor::End => frame_v1 - total,
                };
                let mut placed = Vec::new();
                for (range, pitch) in lines.lines.into_iter().zip(pitches) {
                    placed.push(self.line(
                        &measures,
                        range,
                        logical_box.x0 as f32,
                        room,
                        top,
                        pitch,
                    ));
                    top += pitch;
                }
                let frame = Rect::from_points(
                    physical(logical_box.x0, f64::from(frame_v0), vertical),
                    physical(logical_box.x1, f64::from(frame_v1), vertical),
                );
                Placed {
                    scale,
                    lines: placed,
                    frame,
                    overflowed: lines.overflowed || (!*grow && total > extent + 0.01),
                }
            }
            Room::Path {
                outline, anchor, ..
            } => {
                let outline = Outline::new(outline, vertical);
                self.place_in(&outline, &measures, segments, scale, *anchor)
            }
        })
    }

    fn line(
        &self,
        measures: &[Measure],
        segments: Range<usize>,
        start: f32,
        room: f32,
        top: f32,
        pitch: f32,
    ) -> PlacedLine {
        let last = segments
            .end
            .checked_sub(1)
            .is_some_and(|at| matches!(measures[at].after, Break::Mandatory | Break::End));
        PlacedLine {
            segments,
            start,
            room,
            top,
            pitch,
            last,
        }
    }

    /// Lines in a closed path: every line count the path's block extent holds
    /// is tried, each line taking the widest span of the inside along its
    /// band, less half the size of air on each side, and the cheapest count
    /// that fits wins.
    fn place_in(
        &self,
        outline: &Outline,
        measures: &[Measure],
        segments: &[Segment],
        scale: f32,
        anchor: Anchor,
    ) -> Placed {
        let pitch = self.pitch(segments, scale).max(0.01);
        let air = segments.iter().map(|s| s.size).fold(0.0, f32::max) * scale * 0.5;
        let bounds = outline.bounds;
        let extent = bounds.height() as f32;
        let most = (((extent - 2.0 * air) / pitch).floor() as usize).max(1);
        let mut best: Option<(f32, Placed)> = None;
        for count in 1..=most.min(64) {
            let total = pitch * count as f32;
            let top = match anchor {
                Anchor::Start => bounds.y0 as f32 + air,
                Anchor::Center => (bounds.y0 + bounds.y1) as f32 * 0.5 - total * 0.5,
                Anchor::End => bounds.y1 as f32 - air - total,
            };
            let spans: Vec<(f32, f32)> = (0..count)
                .map(|line| {
                    let v0 = top + pitch * line as f32;
                    outline
                        .widest(f64::from(v0 - air), f64::from(v0 + pitch + air))
                        .map_or((0.0, 0.0), |(start, end)| {
                            let (start, end) = (start as f32 + air, end as f32 - air);
                            (start, (end - start).max(0.0))
                        })
                })
                .collect();
            let Some(lines) =
                layout::break_lines(measures, scale, Shape::Even, Some(count), |line| {
                    spans[line].1
                })
            else {
                continue;
            };
            let cost = cost_of(measures, &lines.lines, scale, |line| spans[line].1);
            let placed = Placed {
                scale,
                lines: lines
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(line, range)| {
                        self.line(
                            measures,
                            range.clone(),
                            spans[line].0,
                            spans[line].1,
                            top + pitch * line as f32,
                            pitch,
                        )
                    })
                    .collect(),
                frame: Rect::from_points(
                    physical(bounds.x0, bounds.y0, self.input.vertical),
                    physical(bounds.x1, bounds.y1, self.input.vertical),
                ),
                overflowed: lines.overflowed,
            };
            let better = match &best {
                None => true,
                Some((best_cost, best)) => {
                    (best.overflowed && !placed.overflowed)
                        || (best.overflowed == placed.overflowed && cost < *best_cost)
                }
            };
            if better {
                best = Some((cost, placed));
            }
        }
        if let Some((_, placed)) = best {
            return placed;
        }
        // No line count holds the text: it sets in the middle line's span,
        // running past the outline as overflowing box text does.
        let middle = (bounds.y0 + bounds.y1) * 0.5;
        let (start, end) = outline
            .widest(
                middle - f64::from(pitch) * 0.5,
                middle + f64::from(pitch) * 0.5,
            )
            .unwrap_or((bounds.x0, bounds.x1));
        let room = (end - start) as f32;
        let lines = layout::break_lines(measures, scale, Shape::Even, None, |_| room)
            .expect("lines without a count always break");
        let total = pitch * lines.lines.len() as f32;
        let top = match anchor {
            Anchor::Start => bounds.y0 as f32 + air,
            Anchor::Center => middle as f32 - total * 0.5,
            Anchor::End => bounds.y1 as f32 - air - total,
        };
        Placed {
            scale,
            lines: lines
                .lines
                .into_iter()
                .enumerate()
                .map(|(line, range)| {
                    self.line(
                        measures,
                        range,
                        start as f32,
                        room,
                        top + pitch * line as f32,
                        pitch,
                    )
                })
                .collect(),
            frame: Rect::from_points(
                physical(bounds.x0, bounds.y0, self.input.vertical),
                physical(bounds.x1, bounds.y1, self.input.vertical),
            ),
            overflowed: true,
        }
    }

    fn draw(&self, segments: &[Segment], placed: Placed) -> Laid {
        let vertical = self.input.vertical;
        let scale = placed.scale;
        let mut runs: Vec<(usize, usize, Vec<Glyph>, Range<usize>)> = Vec::new();
        let mut hyphens = Vec::new();
        let mut broken = Vec::new();
        let mut bounds: Option<Rect> = None;
        let measures: Vec<Measure> = segments.iter().map(|s| s.measure.clone()).collect();
        for line in &placed.lines {
            let Some(last) = line.segments.end.checked_sub(1) else {
                continue;
            };
            let used = layout::length(&measures, line.segments.clone(), scale);
            let mut pieces: Vec<&Piece> = segments[line.segments.clone()]
                .iter()
                .flat_map(|segment| &segment.pieces)
                .collect();
            let after = segments[last].measure.after;
            if after == Break::Hyphen && !line.last {
                if let Some(hyphen) = &segments[last].hyphen {
                    pieces.push(hyphen);
                }
                hyphens.push(segments[last].range.end);
            } else if after == Break::Anywhere {
                broken.push(segments[last].range.end);
            }
            // Trailing white space hangs past the line's end.
            let hang = segments[last].visible_end;
            if !vertical {
                pieces = visual_order(pieces);
            }
            let justify = self.input.align == Align::Justify && !line.last;
            let gaps = if justify {
                let spaces = pieces
                    .iter()
                    .flat_map(|piece| &piece.glyphs)
                    .filter(|glyph| glyph.cluster < hang && self.is_space(glyph.cluster))
                    .count();
                if spaces > 0 {
                    Some((true, spaces))
                } else {
                    Some((
                        false,
                        pieces
                            .iter()
                            .map(|p| p.glyphs.len())
                            .sum::<usize>()
                            .saturating_sub(1),
                    ))
                }
            } else {
                None
            };
            let extra = match gaps {
                Some((_, count)) if count > 0 => (line.room - used).max(0.0) / count as f32,
                _ => 0.0,
            };
            let free = (line.room - used).max(0.0);
            let flipped = self.rtl;
            let offset = match (self.input.align, flipped) {
                (Align::Center, _) => free * 0.5,
                (Align::Start | Align::Justify, false) | (Align::End, true) => 0.0,
                (Align::End, false) | (Align::Start | Align::Justify, true) => {
                    if justify {
                        0.0
                    } else {
                        free
                    }
                }
            };
            let mut pen = line.start + offset;
            let block_center = line.top + line.pitch * 0.5;
            for piece in pieces {
                let span = &self.input.spans[piece.span];
                let size = span.size * scale;
                let font = piece.glyphs.first().map(|g| g.font);
                let (ascent, descent) = font
                    .and_then(|font| metrics(&self.fonts[font], size))
                    .unwrap_or((size * 0.8, size * 0.2));
                let baseline = line.top + (line.pitch - (ascent + descent)) * 0.5 + ascent;
                if piece.upright_group {
                    let width: f32 = piece.glyphs.iter().map(|g| g.advance * scale).sum();
                    let mut across = -width * 0.5;
                    let cell_baseline = pen + (size - (ascent + descent)) * 0.5 + ascent;
                    for glyph in &piece.glyphs {
                        let x = -block_center + across + glyph.x_offset * scale;
                        let y = cell_baseline - glyph.y_offset * scale;
                        push_glyph(&mut runs, glyph, piece.span, x, y);
                        across += glyph.advance * scale;
                    }
                    pen += size;
                    continue;
                }
                for glyph in &piece.glyphs {
                    if glyph.cluster >= hang {
                        continue;
                    }
                    let (x, y) = if vertical {
                        (
                            -block_center + glyph.x_offset * scale,
                            pen - glyph.y_offset * scale,
                        )
                    } else {
                        (
                            pen + glyph.x_offset * scale,
                            baseline - glyph.y_offset * scale,
                        )
                    };
                    push_glyph(&mut runs, glyph, piece.span, x, y);
                    pen += glyph.advance * scale;
                    if let Some((spaces_only, _)) = gaps
                        && (!spaces_only || (glyph.cluster < hang && self.is_space(glyph.cluster)))
                    {
                        pen += extra;
                    }
                }
            }
            let line_box = Rect::from_points(
                physical(f64::from(line.start), f64::from(line.top), vertical),
                physical(
                    f64::from(line.start + line.room.max(used)),
                    f64::from(line.top + line.pitch),
                    vertical,
                ),
            );
            bounds = Some(bounds.map_or(line_box, |b| b.union(line_box)));
        }

        let mut scene = Scene::new();
        let mut resolved_runs = Vec::new();
        for (font, span, glyphs, range) in runs {
            let style = &self.input.spans[span];
            let font = &self.fonts[font];
            let size = style.size * scale;
            let data = font.font_data();
            let skew = font
                .synthetic_skew()
                .map(|angle| Affine::skew(-f64::from(angle.to_radians().tan()), 0.0));
            let bold = if font.synthetic_bold() {
                f64::from(size) * 0.02
            } else {
                0.0
            };
            if let Some((color, width)) = style.stroke {
                let amount = bold + f64::from(width * scale);
                scene
                    .draw_glyphs(&data)
                    .font_size(size)
                    .normalized_coords(font.normalized_coords())
                    .glyph_transform(skew)
                    .font_embolden(
                        FontEmbolden::new(Diagonal2::new(amount, amount)).with_join(Join::Round),
                    )
                    .brush(color)
                    .draw(Fill::NonZero, glyphs.iter().copied());
            }
            scene
                .draw_glyphs(&data)
                .font_size(size)
                .normalized_coords(font.normalized_coords())
                .glyph_transform(skew)
                .font_embolden(FontEmbolden::new(Diagonal2::new(bold, bold)))
                .brush(style.fill)
                .draw(Fill::NonZero, glyphs.iter().copied());
            resolved_runs.push(TextRun {
                range: utf16(&self.input.text, range.start)..utf16(&self.input.text, range.end),
                post_script_name: font.post_script_name().to_owned(),
                size,
                fill: style.fill,
                stroke: style.stroke.map(|(color, width)| (color, width * scale)),
            });
        }
        let stroke = self
            .input
            .spans
            .iter()
            .filter_map(|span| span.stroke.map(|(_, width)| width * scale))
            .fold(0.0, f32::max);
        let bounds = bounds
            .unwrap_or(placed.frame)
            .inflate(f64::from(stroke), f64::from(stroke));
        Laid {
            scene,
            resolved: ResolvedText {
                runs: resolved_runs,
                frame: placed.frame,
                scale,
                hyphens: hyphens
                    .iter()
                    .map(|at| utf16(&self.input.text, *at))
                    .collect(),
                broken: broken
                    .iter()
                    .map(|at| utf16(&self.input.text, *at))
                    .collect(),
                overflowed: placed.overflowed,
            },
            bounds,
        }
    }

    fn is_space(&self, offset: usize) -> bool {
        self.input.text[offset..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    }
}

/// Appends a glyph to the last run if it shares its face and span, else
/// starts a run.
fn push_glyph(
    runs: &mut Vec<(usize, usize, Vec<Glyph>, Range<usize>)>,
    glyph: &Shaped,
    span: usize,
    x: f32,
    y: f32,
) {
    let drawn = Glyph { id: glyph.id, x, y };
    match runs.last_mut() {
        Some((font, run_span, glyphs, range)) if *font == glyph.font && *run_span == span => {
            glyphs.push(drawn);
            range.start = range.start.min(glyph.cluster);
            range.end = range.end.max(glyph.cluster + 1);
        }
        _ => runs.push((
            glyph.font,
            span,
            vec![drawn],
            glyph.cluster..glyph.cluster + 1,
        )),
    }
}

/// The cost `break_lines` gave the chosen lines, to compare line counts.
fn cost_of(
    measures: &[Measure],
    lines: &[Range<usize>],
    scale: f32,
    room: impl Fn(usize) -> f32,
) -> f32 {
    lines
        .iter()
        .enumerate()
        .map(|(line, range)| {
            let room = room(line);
            let used = layout::length(measures, range.clone(), scale);
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

/// Pieces in the order they show: runs of right-to-left pieces reversed.
fn visual_order(pieces: Vec<&Piece>) -> Vec<&Piece> {
    let mut ordered = Vec::with_capacity(pieces.len());
    let mut rtl = Vec::new();
    for piece in pieces {
        if piece.rtl {
            rtl.push(piece);
        } else {
            ordered.extend(rtl.drain(..).rev());
            ordered.push(piece);
        }
    }
    ordered.extend(rtl.into_iter().rev());
    ordered
}

fn metrics(font: &Font, size: f32) -> Option<(f32, f32)> {
    let metrics = font
        .skrifa_ref()
        .ok()?
        .metrics(Size::new(size), font.location());
    Some((metrics.ascent, -metrics.descent))
}

fn is_exclamation(ch: char) -> bool {
    matches!(ch, '!' | '?' | '！' | '？')
}

/// `text` with `!` and `?` in their full-width forms, or in their
/// proportional ones for an upright group, and the source byte of each byte
/// of the result.
fn exclamations(text: &str, upright: bool) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut clusters = Vec::with_capacity(text.len());
    for (offset, ch) in text.char_indices() {
        out.push(match (ch, upright) {
            ('!', false) => '！',
            ('?', false) => '？',
            ('！', true) => '!',
            ('？', true) => '?',
            (other, _) => other,
        });
        clusters.resize(out.len(), offset);
    }
    (out, clusters)
}

/// The UTF-16 offset of byte `offset`, rounded up to a character boundary.
fn utf16(text: &str, offset: usize) -> usize {
    text[..text.ceil_char_boundary(offset)]
        .encode_utf16()
        .count()
}
