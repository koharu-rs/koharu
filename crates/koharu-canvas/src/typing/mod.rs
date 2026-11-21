//! Typing into a text on the canvas, as Photoshop's Type tool does: the
//! session holds the rich text as typed, its caret and selection, the style
//! held for what's typed next, and the input method's composition. The
//! renderer lays it out on every change, the canvas draws it in place of the
//! layer, and leaving commits it as one step, as Photoshop's "Edit Type
//! Layer". Offsets are UTF-8 offsets into its characters.

mod keys;
mod style;

use std::{ops::Range, time::Instant};

use icu_segmenter::{GraphemeClusterSegmenter, WordSegmenter, options::WordBreakInvariantOptions};
use koharu_renderer::{Lettering, Renderer};
use koharu_scene::{CharStyle, Content, Edit, LayerId, Op, Page, RichText, Text, Uuid};
use kurbo::{Affine, Line, Point, Rect};
use serde_json::json;

pub(crate) use self::keys::Keyed;
pub use self::style::{CharKey, Typed};
use crate::{Effect, tools::added};

/// How long the caret shows, and then hides, as Windows blinks one.
const BLINK: u128 = 530;

/// What a session types into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A text layer, or a block's translation.
    Layer(LayerId),
    /// A block without a translation, which the commit gives one in its
    /// source's place.
    Untranslated(LayerId),
    /// A new text, which the commit adds.
    New,
}

/// The input method's text while it composes.
struct Composition {
    text: String,
    /// The part it has selected, in its own offsets.
    selected: Option<Range<usize>>,
}

pub(crate) struct Typing {
    renderer: Renderer,
    target: Target,
    /// The text typed into: its frame, transform, and style. Its content is
    /// `content`.
    base: Text,
    /// The content as the session found it, so leaving it unchanged
    /// commits nothing.
    original: RichText,
    content: RichText,
    caret: usize,
    anchor: usize,
    /// The marks the next character typed takes, set at the caret.
    held: Option<CharStyle>,
    composition: Option<Composition>,
    /// Where moving up and down keeps the caret along the lines.
    goal: Option<f64>,
    /// When the caret last moved, which it blinks from.
    since: Instant,
    /// Bumped by every change it shows.
    version: u64,
    lettering: Lettering,
    /// The pointer dragging a selection across it.
    pub dragging: Option<u64>,
}

impl Typing {
    /// Typing into `layer`'s `text`; with `untranslated`, `text` is a block's
    /// source standing in for the translation it doesn't have yet.
    pub fn layer(
        renderer: &Renderer,
        layer: LayerId,
        text: &Text,
        untranslated: bool,
    ) -> anyhow::Result<Self> {
        let (target, content) = match untranslated {
            true => (Target::Untranslated(layer), RichText::default()),
            false => (Target::Layer(layer), text.content.clone()),
        };
        Self::start(renderer, target, text.clone(), content)
    }

    /// Typing a new text, `text` giving its frame and place.
    pub fn new_text(renderer: &Renderer, text: Text) -> anyhow::Result<Self> {
        Self::start(renderer, Target::New, text, RichText::default())
    }

    fn start(
        renderer: &Renderer,
        target: Target,
        base: Text,
        content: RichText,
    ) -> anyhow::Result<Self> {
        let lettering = renderer.lettering(
            LayerId(Uuid::nil()),
            &Text {
                content: content.clone(),
                ..base.clone()
            },
        )?;
        Ok(Self {
            renderer: renderer.clone(),
            target,
            base,
            original: content.clone(),
            content,
            caret: 0,
            anchor: 0,
            held: None,
            composition: None,
            goal: None,
            since: Instant::now(),
            version: 0,
            lettering,
            dragging: None,
        })
    }

    /// The layer typed into; `None` for a new text.
    pub fn target(&self) -> Option<LayerId> {
        match self.target {
            Target::Layer(layer) | Target::Untranslated(layer) => Some(layer),
            Target::New => None,
        }
    }

    /// Takes the text's frame, place, and style again from `text`, as
    /// another change left them, keeping what's typed.
    pub fn rebase(&mut self, text: &Text) -> anyhow::Result<()> {
        self.base = text.clone();
        self.relay()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn vertical(&self) -> bool {
        self.base.direction == koharu_scene::Direction::Vertical
    }

    /// From the text's own coordinates to the page.
    pub fn transform(&self) -> Affine {
        Affine::from(self.base.transform)
    }

    pub fn lettering(&self) -> &Lettering {
        &self.lettering
    }

    /// Whether the caret shows at `now`: it blinks, and shows for a while
    /// after each change.
    pub fn caret_shows(&self, now: Instant) -> bool {
        (now.duration_since(self.since).as_millis() / BLINK).is_multiple_of(2)
    }

    /// The caret, in the text's own coordinates: after the composition's
    /// selected part while one composes.
    pub fn caret(&self) -> Line {
        let within = self.composition.as_ref().map_or(0, |composition| {
            composition
                .selected
                .as_ref()
                .map_or(composition.text.len(), |selected| selected.end)
        });
        self.lettering.caret(self.caret + within)
    }

    /// The selection's rectangles, in the text's own coordinates; none
    /// while the input method composes.
    pub fn highlight(&self) -> Vec<Rect> {
        match self.composition {
            Some(_) => Vec::new(),
            None => self.lettering.selection(self.selected()),
        }
    }

    /// The composition's rectangles, each with whether it's the part the
    /// input method has selected, in the text's own coordinates.
    pub fn composing(&self) -> Vec<(Rect, bool)> {
        let Some(composition) = &self.composition else {
            return Vec::new();
        };
        let at = self.caret;
        let mut rects: Vec<(Rect, bool)> = self
            .lettering
            .selection(at..at + composition.text.len())
            .into_iter()
            .map(|rect| (rect, false))
            .collect();
        if let Some(selected) = &composition.selected {
            rects.extend(
                self.lettering
                    .selection(at + selected.start..at + selected.end)
                    .into_iter()
                    .map(|rect| (rect, true)),
            );
        }
        rects
    }

    /// Whether `page` lies on the text, within `slack` page pixels of what
    /// it covers.
    pub fn covers(&self, page: Point, slack: f64) -> bool {
        let point = self.transform().inverse() * page;
        self.lettering
            .bounds()
            .inflate(slack, slack)
            .contains(point)
    }

    /// The selected range, in order.
    fn selected(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// The text as it shows: the content with the composition at the caret.
    fn shown(&self) -> Text {
        let content = match &self.composition {
            Some(composition) => {
                self.content
                    .spliced(self.caret..self.caret, &composition.text, &self.marks())
            }
            None => self.content.clone(),
        };
        Text {
            content,
            ..self.base.clone()
        }
    }

    /// Lays the shown text out again after a change.
    fn relay(&mut self) -> anyhow::Result<()> {
        self.lettering = self
            .renderer
            .lettering(LayerId(Uuid::nil()), &self.shown())?;
        self.touch();
        Ok(())
    }

    /// Marks a change the caret shows from.
    fn touch(&mut self) {
        self.version += 1;
        self.since = Instant::now();
    }

    /// The marks a character typed now takes: what the caret holds, the
    /// first selected character's, or the one's before the caret.
    fn marks(&self) -> CharStyle {
        if let Some(held) = &self.held {
            return held.clone();
        }
        let range = self.selected();
        let plain = self.content.text();
        let at = match range.is_empty() {
            true => range.start,
            false => next_grapheme(&plain, range.start),
        };
        self.content.marks_at(at)
    }

    /// Types `text` over the selection.
    pub fn insert(&mut self, text: &str) -> anyhow::Result<()> {
        let range = self.selected();
        let marks = self.marks();
        self.content = self.content.spliced(range.clone(), text, &marks);
        self.caret = range.start + text.len();
        self.anchor = self.caret;
        self.held = None;
        self.goal = None;
        self.relay()
    }

    /// Deletes the selection, or the grapheme or word beside the caret,
    /// before it unless `forward`.
    pub fn delete(&mut self, forward: bool, word: bool) -> anyhow::Result<()> {
        let mut range = self.selected();
        if range.is_empty() {
            let plain = self.content.text();
            range = match (forward, word) {
                (false, false) => previous_grapheme(&plain, self.caret)..self.caret,
                (true, false) => self.caret..next_grapheme(&plain, self.caret),
                (false, true) => previous_word(&plain, self.caret)..self.caret,
                (true, true) => self.caret..next_word(&plain, self.caret),
            };
        }
        if range.is_empty() {
            return Ok(());
        }
        self.content = self
            .content
            .spliced(range.clone(), "", &CharStyle::default());
        self.caret = range.start;
        self.anchor = range.start;
        self.goal = None;
        self.relay()
    }

    /// Puts the caret at `offset`, moving only the caret when `extend`, so
    /// the selection runs from where it was anchored.
    fn place(&mut self, offset: usize, extend: bool) {
        self.caret = offset;
        if !extend {
            self.anchor = offset;
        }
        self.held = None;
        self.touch();
    }

    /// The caret a grapheme, or a word, on through the text, or back.
    pub fn step(&mut self, forward: bool, word: bool, extend: bool) {
        let plain = self.content.text();
        let selection = self.selected();
        let to = match (forward, word) {
            // Without Shift, an arrow first collapses a selection to its
            // side.
            _ if !extend && !selection.is_empty() => match forward {
                true => selection.end,
                false => selection.start,
            },
            (true, false) => next_grapheme(&plain, self.caret),
            (false, false) => previous_grapheme(&plain, self.caret),
            (true, true) => next_word(&plain, self.caret),
            (false, true) => previous_word(&plain, self.caret),
        };
        self.goal = None;
        self.place(to, extend);
    }

    /// The caret a line on, or back, keeping its place along the lines; at
    /// the last line or the first, to the text's end or start.
    pub fn line(&mut self, forward: bool, extend: bool) {
        let along = *self
            .goal
            .get_or_insert_with(|| self.lettering.along(self.caret));
        let line = self.lettering.line(self.caret);
        let to = match forward {
            true if line + 1 < self.lettering.lines() => self.lettering.nearest(line + 1, along),
            true => self.content.text().len(),
            false if line > 0 => self.lettering.nearest(line - 1, along),
            false => 0,
        };
        self.place(to, extend);
    }

    /// The caret to its line's end or start, or the text's when `whole`.
    pub fn edge(&mut self, end: bool, whole: bool, extend: bool) {
        let line = self.lettering.line_text(self.lettering.line(self.caret));
        let to = match (end, whole) {
            (true, true) => self.content.text().len(),
            (false, true) => 0,
            (true, false) => line.end,
            (false, false) => line.start,
        };
        self.goal = None;
        self.place(to, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.content.text().len();
        self.held = None;
        self.goal = None;
        self.touch();
    }

    pub fn selected_text(&self) -> String {
        self.content.text()[self.selected()].to_owned()
    }

    /// A press at `page`: the caret there, the selection out to it when
    /// `extend`, or on a double click the word there.
    pub fn press(&mut self, page: Point, extend: bool, clicks: u8) {
        self.settle();
        let offset = self.hit(page);
        self.goal = None;
        if clicks >= 2 {
            let word = word_at(&self.content.text(), offset);
            self.anchor = word.start;
            self.place(word.end, true);
        } else {
            self.place(offset, extend);
        }
    }

    /// A drag across the text to `page`, the selection following it.
    pub fn drag(&mut self, page: Point) {
        let offset = self.hit(page);
        if offset != self.caret {
            self.place(offset, true);
        }
    }

    fn hit(&self, page: Point) -> usize {
        self.lettering.hit(self.transform().inverse() * page)
    }

    /// Styles the selection with `patch`'s values; at a caret, holds them
    /// for what's typed next, as Photoshop's Character panel does.
    pub fn style(&mut self, patch: &CharStyle) -> anyhow::Result<()> {
        let range = self.selected();
        if range.is_empty() {
            self.held = Some(patch.or(&self.marks()));
            self.touch();
            return Ok(());
        }
        self.content = self.content.marked(range, patch);
        self.relay()
    }

    /// The input method's text as it composes, empty when it stops; it
    /// replaces the selection.
    pub fn compose(&mut self, text: &str, selected: Option<Range<usize>>) -> anyhow::Result<()> {
        if self.composition.is_none() && !self.selected().is_empty() && !text.is_empty() {
            self.delete(false, false)?;
        }
        self.composition = (!text.is_empty()).then(|| Composition {
            text: text.to_owned(),
            selected,
        });
        self.relay()
    }

    /// Types what the input method composed, as a press or leaving it does.
    fn settle(&mut self) {
        if let Some(composition) = self.composition.take() {
            let marks = self.marks();
            self.content = self
                .content
                .spliced(self.caret..self.caret, &composition.text, &marks);
            self.caret += composition.text.len();
            self.anchor = self.caret;
        }
    }

    /// What the host shows for it.
    pub fn typed(&self) -> Typed {
        let base = &self.base.style.chars;
        let range = self.selected();
        let (chars, mixed) = match range.is_empty() {
            true => style::shared(std::iter::once(self.marks().or(base))),
            false => {
                let mut start = 0;
                style::shared(self.content.0.iter().filter_map(|span| {
                    let (from, to) = (start, start + span.insert.len());
                    start = to;
                    (from < range.end && range.start < to).then(|| span.attributes.or(base))
                }))
            }
        };
        Typed {
            layer: self.target(),
            chars,
            mixed,
        }
    }

    /// The step that keeps what was typed into `page`: none when nothing
    /// changed, or a new text was left empty.
    pub fn commit(mut self, page: &Page) -> Vec<Effect> {
        self.settle();
        let edit = |label: &str, ops: Vec<Op>| {
            Effect::Edit(Edit {
                label: label.to_owned(),
                ops,
            })
        };
        let spans = |layer: LayerId, content: RichText| Op::SetRichText {
            layer: layer.into(),
            spans: content.0,
        };
        match self.target {
            Target::Layer(layer) if self.content != self.original => {
                vec![edit("set_text", vec![spans(layer, self.content)])]
            }
            Target::Untranslated(layer) if !self.content.0.is_empty() => {
                // The translation takes its source's place and style; its
                // characters come only through the text ops.
                let mut placed = serde_json::to_value(&self.base).expect("a text serializes");
                if let Some(fields) = placed.as_object_mut() {
                    fields.remove("content");
                }
                let place = Op::UpdateLayer {
                    layer: layer.into(),
                    patch: json!({ "translation": placed }),
                };
                vec![edit("set_text", vec![place, spans(layer, self.content)])]
            }
            Target::New if !self.content.0.is_empty() => {
                let text = Text {
                    content: self.content,
                    ..self.base
                };
                let (id, add) = added(page, Content::Text(Box::new(text)));
                vec![edit("add_text", vec![add]), Effect::Selection(vec![id])]
            }
            _ => Vec::new(),
        }
    }
}

/// The grapheme boundary after `at`, or the text's end.
fn next_grapheme(text: &str, at: usize) -> usize {
    GraphemeClusterSegmenter::new()
        .segment_str(text)
        .find(|&boundary| boundary > at)
        .unwrap_or(text.len())
}

/// The grapheme boundary before `at`, or the start.
fn previous_grapheme(text: &str, at: usize) -> usize {
    GraphemeClusterSegmenter::new()
        .segment_str(text)
        .take_while(|&boundary| boundary < at)
        .last()
        .unwrap_or(0)
}

/// `text`'s segments between word boundaries, each with whether it's a word.
fn words(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut start = 0;
    WordSegmenter::new_auto(WordBreakInvariantOptions::default())
        .segment_str(text)
        .iter_with_word_type()
        .filter(|&(boundary, _)| boundary > 0)
        .map(|(end, kind)| {
            let segment = start..end;
            start = end;
            (segment, kind.is_word_like())
        })
        .collect()
}

/// The end of the next word after `at`, or the text's end.
fn next_word(text: &str, at: usize) -> usize {
    words(text)
        .into_iter()
        .find(|(segment, word)| *word && segment.end > at)
        .map_or(text.len(), |(segment, _)| segment.end)
}

/// The start of the word before `at`, or the start.
fn previous_word(text: &str, at: usize) -> usize {
    words(text)
        .into_iter()
        .rev()
        .find(|(segment, word)| *word && segment.start < at)
        .map_or(0, |(segment, _)| segment.start)
}

/// The segment a double click at `at` selects: the one it's in, or the one
/// it ends at the text's end.
fn word_at(text: &str, at: usize) -> Range<usize> {
    words(text)
        .into_iter()
        .map(|(segment, _)| segment)
        .find(|segment| segment.contains(&at) || segment.end == text.len() && at == text.len())
        .unwrap_or(at..at)
}
