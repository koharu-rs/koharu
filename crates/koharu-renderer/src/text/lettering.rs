//! A text laid out as a render lays it out, for typing into it: it draws
//! itself and says where its characters lie, as an editor's caret, hit test,
//! and selection need. Offsets are UTF-8 offsets into the text's characters;
//! "along" is a line's own axis, across the column in vertical text.

use std::{ops::Range, sync::Arc};

use anyhow::{Context as _, Result};
use koharu_scene::{Direction, Text};
use kurbo::{Affine, Line, Point, Rect};

use super::{
    TextLayout, draw,
    room::{logical, physical},
};
use crate::Drawing;

/// One line of a layout as typing sees it.
pub(crate) struct LineMap {
    /// Its characters, its trailing white space and line break included.
    pub text: Range<usize>,
    /// Where its visible characters end, before that white space.
    pub visible: usize,
    /// Where its pen starts along it, after its alignment.
    pub start: f32,
    /// Its box across the lines.
    pub top: f32,
    pub pitch: f32,
    /// Its visible clusters, in text order.
    pub clusters: Vec<Cluster>,
}

/// A cluster: characters that shape into glyphs together.
pub(crate) struct Cluster {
    pub text: Range<usize>,
    /// Where its glyphs lie along the line.
    pub along: Range<f32>,
    pub rtl: bool,
}

/// A glyph as placing its line gave it: its cluster offset, where it lies
/// along the line, and whether its run is right to left.
pub(crate) type PlacedGlyph = (usize, Range<f32>, bool);

impl LineMap {
    /// A line from the glyphs placing it gave.
    pub(crate) fn new(
        text: Range<usize>,
        visible: usize,
        start: f32,
        (top, pitch): (f32, f32),
        mut glyphs: Vec<PlacedGlyph>,
    ) -> Self {
        glyphs.sort_by_key(|glyph| glyph.0);
        let mut clusters: Vec<Cluster> = Vec::new();
        for (at, along, rtl) in glyphs {
            match clusters.last_mut() {
                Some(last) if last.text.start == at => {
                    last.along = last.along.start.min(along.start)..last.along.end.max(along.end);
                }
                _ => clusters.push(Cluster {
                    text: at..at,
                    along,
                    rtl,
                }),
            }
        }
        // Each cluster runs to the next one's start, the last to the visible
        // end.
        let ends: Vec<usize> = clusters
            .iter()
            .skip(1)
            .map(|cluster| cluster.text.start)
            .chain([visible])
            .collect();
        for (cluster, end) in clusters.iter_mut().zip(ends) {
            cluster.text.end = end.max(cluster.text.start);
        }
        Self {
            text,
            visible,
            start,
            top,
            pitch,
            clusters,
        }
    }

    /// Where a caret at `offset` stands along the line: within its cluster
    /// by its share of the cluster's characters, before the first cluster at
    /// its leading edge, and past the visible text at the last one's trailing
    /// edge.
    fn along(&self, offset: usize) -> f32 {
        if let Some(cluster) = self
            .clusters
            .iter()
            .find(|cluster| cluster.text.contains(&offset))
        {
            return cluster.within(offset);
        }
        match (self.clusters.first(), self.clusters.last()) {
            (Some(first), _) if offset < first.text.start => first.edge(true),
            (_, Some(last)) => last.edge(false),
            _ => self.start,
        }
    }

    /// The cluster edge nearest `along`, as an offset.
    fn nearest(&self, along: f32) -> usize {
        self.clusters
            .iter()
            .flat_map(|cluster| {
                [
                    (cluster.text.start, cluster.edge(true)),
                    (cluster.text.end, cluster.edge(false)),
                ]
            })
            .min_by(|a, b| (a.1 - along).abs().total_cmp(&(b.1 - along).abs()))
            .map_or(self.text.start, |(offset, _)| offset)
    }
}

impl Cluster {
    /// Its leading edge, where its text starts, or its trailing one.
    fn edge(&self, leading: bool) -> f32 {
        match self.rtl == leading {
            true => self.along.end,
            false => self.along.start,
        }
    }

    /// Where `offset`, within it, stands: from its leading edge by its share
    /// of the characters, which splits a ligature evenly.
    fn within(&self, offset: usize) -> f32 {
        let share = (offset - self.text.start) as f32 / self.text.len().max(1) as f32;
        let width = self.along.end - self.along.start;
        match self.rtl {
            true => self.along.end - share * width,
            false => self.along.start + share * width,
        }
    }
}

/// A text laid out as a render lays it out, in its own coordinates, which
/// its transform places on the page: drawn, and asked where its characters
/// lie.
pub struct Lettering {
    layout: Arc<TextLayout>,
    /// It has no characters, so it's laid out as one space, which draws
    /// nothing, for where its caret goes.
    empty: bool,
    vertical: bool,
}

impl Lettering {
    /// `text` laid out by `lay`, which lays a text out as a render does.
    pub(crate) fn of(
        text: &Text,
        lay: impl FnOnce(&Text) -> Result<Option<Arc<TextLayout>>>,
    ) -> Result<Self> {
        let empty = text.content.text().is_empty();
        let layout = match empty {
            true => lay(&Text {
                content: " ".into(),
                ..text.clone()
            }),
            false => lay(text),
        }?
        .context("a text with characters has a layout")?;
        Ok(Self {
            layout,
            empty,
            vertical: text.direction == Direction::Vertical,
        })
    }

    /// What it covers, its strokes included; an empty text's caret.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        match self.empty {
            true => {
                let caret = self.caret(0);
                Rect::from_points(caret.p0, caret.p1)
            }
            false => self.layout.bounds,
        }
    }

    /// What fitting drew it at; 1 at its set size.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.layout.resolved.scale
    }

    /// Draws it onto `drawing` through `transform`, its strokes under its
    /// fills, as a render draws it.
    pub fn draw(&self, drawing: &mut Drawing, transform: Affine) -> Result<()> {
        match self.empty {
            true => Ok(()),
            false => draw(drawing, &self.layout, transform, true),
        }
    }

    #[must_use]
    pub fn lines(&self) -> usize {
        self.layout.lines.len()
    }

    /// The line a caret at `offset` stands on.
    #[must_use]
    pub fn line(&self, offset: usize) -> usize {
        self.layout
            .lines
            .iter()
            .rposition(|line| line.text.start <= offset)
            .unwrap_or(0)
    }

    /// Line `line`'s characters through its last visible one, or through the
    /// text's end on the last line: where Home and End go.
    #[must_use]
    pub fn line_text(&self, line: usize) -> Range<usize> {
        if self.empty {
            return 0..0;
        }
        let map = &self.layout.lines[line];
        let last = line + 1 == self.layout.lines.len();
        map.text.start..if last { map.text.end } else { map.visible }
    }

    /// Where a caret at `offset` stands along its line.
    #[must_use]
    pub fn along(&self, offset: usize) -> f64 {
        let offset = self.offset(offset);
        f64::from(self.layout.lines[self.line(offset)].along(offset))
    }

    /// The offset on line `line` nearest `along`, as moving up or down a line
    /// lands.
    #[must_use]
    pub fn nearest(&self, line: usize, along: f64) -> usize {
        match self.empty {
            true => 0,
            false => self.layout.lines[line].nearest(along as f32),
        }
    }

    /// A caret at `offset`: a segment across its line's box.
    #[must_use]
    pub fn caret(&self, offset: usize) -> Line {
        let offset = self.offset(offset);
        let line = &self.layout.lines[self.line(offset)];
        let along = f64::from(line.along(offset));
        let to_text = logical(self.vertical).inverse();
        Line::new(
            to_text * Point::new(along, f64::from(line.top)),
            to_text * Point::new(along, f64::from(line.top + line.pitch)),
        )
    }

    /// The offset a press at `point` puts the caret at: on the line whose box
    /// it's across from, the nearest cluster edge.
    #[must_use]
    pub fn hit(&self, point: Point) -> usize {
        if self.empty {
            return 0;
        }
        let point = logical(self.vertical) * point;
        let lines = &self.layout.lines;
        let line = lines
            .iter()
            .position(|line| point.y < f64::from(line.top + line.pitch))
            .unwrap_or(lines.len() - 1);
        lines[line].nearest(point.x as f32)
    }

    /// The rectangles `range` covers, one a run of touching clusters on a
    /// line, across the line's box.
    #[must_use]
    pub fn selection(&self, range: Range<usize>) -> Vec<Rect> {
        if self.empty || range.is_empty() {
            return Vec::new();
        }
        let mut rects = Vec::new();
        for line in &self.layout.lines {
            let mut spans: Vec<(f32, f32)> = line
                .clusters
                .iter()
                .filter(|cluster| cluster.text.start < range.end && range.start < cluster.text.end)
                .map(|cluster| {
                    let a = cluster.within(range.start.max(cluster.text.start));
                    let b = match range.end < cluster.text.end {
                        true => cluster.within(range.end),
                        false => cluster.edge(false),
                    };
                    (a.min(b), a.max(b))
                })
                .collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut runs: Vec<(f32, f32)> = Vec::new();
            for (from, to) in spans {
                match runs.last_mut() {
                    // Kerning can part touching clusters by a hair.
                    Some(last) if from <= last.1 + 0.5 => last.1 = last.1.max(to),
                    _ => runs.push((from, to)),
                }
            }
            rects.extend(runs.into_iter().map(|(from, to)| {
                physical(
                    self.vertical,
                    f64::from(from),
                    f64::from(line.top),
                    f64::from(to),
                    f64::from(line.top + line.pitch),
                )
            }));
        }
        rects
    }

    /// `offset` as its layout counts: an empty text's one space is behind
    /// its caret.
    fn offset(&self, offset: usize) -> usize {
        match self.empty {
            true => 0,
            false => offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use koharu_scene::{Bounds, CharStyle, Direction, Frame, Text, TextStyle};
    use kurbo::{Point, Rect};

    use super::Lettering;
    use crate::{fonts::Library, text::layout};

    /// `content` in an installed sans-serif at 20 pixels, set as `frame` in
    /// `direction`, laid out as typing would see it.
    fn lettering(content: &str, frame: Frame, direction: Direction) -> Lettering {
        let font = [
            "Arial",
            "Segoe UI",
            "Noto Sans",
            "DejaVu Sans",
            "Liberation Sans",
        ]
        .into_iter()
        .find_map(|family| Library::new().family(family).ok())
        .expect("an installed sans-serif");
        let text = Text {
            content: content.into(),
            style: TextStyle {
                chars: CharStyle {
                    size: Some(20.0),
                    ..CharStyle::default()
                },
                ..TextStyle::default()
            },
            frame,
            direction,
            ..Text::default()
        };
        Lettering::of(&text, |text| {
            Ok(layout(text, |_, _| Ok(vec![font.clone()]))?.map(Arc::new))
        })
        .expect("a lettering")
    }

    #[test]
    fn a_caret_runs_along_a_line_and_a_hit_finds_it_again() {
        let laid = lettering("Hello world", Frame::Point, Direction::Horizontal);
        let carets: Vec<f64> = (0..=11).map(|at| laid.caret(at).p0.x).collect();
        assert!(
            carets.windows(2).all(|pair| pair[0] < pair[1]),
            "{carets:?}"
        );
        // Across its line, as tall as the line's box.
        let caret = laid.caret(5);
        assert_eq!(caret.p0.x, caret.p1.x);
        assert!(caret.p1.y - caret.p0.y > 20.0);
        assert_eq!(laid.hit(caret.p0.midpoint(caret.p1)), 5);
        // Past either end, a hit takes the nearer end.
        assert_eq!(laid.hit(Point::new(-50.0, 10.0)), 0);
        assert_eq!(laid.hit(Point::new(9999.0, 10.0)), 11);
        // A selection covers the characters between its carets.
        let [selected] = laid.selection(0..5)[..] else {
            panic!("one rectangle");
        };
        assert!((selected.x0 - carets[0]).abs() < 1e-3 && (selected.x1 - carets[5]).abs() < 1e-3);
    }

    #[test]
    fn a_wrapped_text_answers_each_line() {
        let frame = Frame::Box {
            bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 70.0,
                height: 400.0,
            },
            fit: Default::default(),
            anchor: Default::default(),
        };
        let laid = lettering("one two three four", frame, Direction::Horizontal);
        assert!(laid.lines() > 1);
        let last = laid.lines() - 1;
        let second = laid.line_text(1);
        assert_eq!(laid.line(second.start), 1);
        assert_eq!(laid.line(18), last);
        // A hit on the second line's height lands on it.
        let caret = laid.caret(second.start);
        assert!(
            second.contains(&laid.hit(Point::new(caret.p0.x + 1.0, caret.p0.midpoint(caret.p1).y)))
        );
        // A line break ends a line, and the caret after it starts the next.
        let broken = lettering("ab\ncd", Frame::Point, Direction::Horizontal);
        assert_eq!(broken.lines(), 2);
        assert_eq!(broken.line(3), 1);
        assert!(broken.caret(3).p0.y > broken.caret(2).p0.y);
    }

    #[test]
    fn a_vertical_caret_lies_across_its_column() {
        let laid = lettering("AV", Frame::Point, Direction::Vertical);
        let (first, second) = (laid.caret(0), laid.caret(1));
        assert_eq!(first.p0.y, first.p1.y);
        assert!(second.p0.y > first.p0.y);
        assert_eq!(laid.hit(second.p0.midpoint(second.p1)), 1);
    }

    #[test]
    fn an_empty_text_has_a_caret_a_line_tall() {
        let laid = lettering("", Frame::Point, Direction::Horizontal);
        let caret = laid.caret(0);
        assert_eq!(caret.p0.x, 0.0);
        assert!(caret.p1.y - caret.p0.y > 20.0);
        assert_eq!(laid.hit(Point::new(30.0, 10.0)), 0);
        assert!(laid.selection(0..0).is_empty());
        assert_eq!(laid.bounds(), Rect::from_points(caret.p0, caret.p1));
    }
}
