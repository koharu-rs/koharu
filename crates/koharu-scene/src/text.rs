//! Text: styled spans in a frame. A text layer holds one, and so do a block's
//! source and translation.

use std::ops::Range;

use icu_locale_core::LanguageIdentifier;
use kurbo::{Affine, BezPath, Shape as _};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use specta::Type;

use crate::{Align, Direction, Frame, Rgba, Transform};

/// Character values; each is also a mark key on a span, and a mark wins for
/// its span.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CharStyle {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub weight: Option<u16>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub fill: Option<Rgba>,
    #[serde(default)]
    pub stroke: Option<Rgba>,
    #[serde(default)]
    pub stroke_width: Option<f64>,
    /// How the characters stand in a vertical column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    /// The mark set beside each character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<Emphasis>,
}

impl CharStyle {
    /// These values, with `base`'s where these have none.
    #[must_use]
    pub fn or(&self, base: &Self) -> Self {
        Self {
            family: self.family.clone().or_else(|| base.family.clone()),
            weight: self.weight.or(base.weight),
            italic: self.italic.or(base.italic),
            size: self.size.or(base.size),
            fill: self.fill.or(base.fill),
            stroke: self.stroke.or(base.stroke),
            stroke_width: self.stroke_width.or(base.stroke_width),
            orientation: self.orientation.or(base.orientation),
            emphasis: self.emphasis.or(base.emphasis),
        }
    }
}

// `mixed` turns Latin letters and digits sideways and stands the rest
// upright, short numbers and `!?` runs in one cell; `upright` stands every
// character one under another; `sideways` turns them all a quarter clockwise,
// as one line; and `tate_chu_yoko` sets them side by side in one upright cell:
// CSS's `text-orientation`, and the tate-chu-yoko of Photoshop, Clip Studio
// Paint, and CSS's `text-combine-upright: all`. The doc comments stay short, as
// agents' tools inline them wherever a character style goes.
/// How characters stand in a vertical column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Mixed,
    Upright,
    Sideways,
    TateChuYoko,
}

// CSS's `text-emphasis-style` shapes, each filled or open, as one list, as
// InDesign lists its kenten.
/// An emphasis mark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Emphasis {
    #[default]
    None,
    Dot,
    OpenDot,
    Circle,
    OpenCircle,
    DoubleCircle,
    OpenDoubleCircle,
    Triangle,
    OpenTriangle,
    Sesame,
    OpenSesame,
}

impl Emphasis {
    /// The character its mark is, as CSS names each; `None` for none.
    #[must_use]
    pub fn mark(self) -> Option<char> {
        Some(match self {
            Self::None => return None,
            Self::Dot => '•',
            Self::OpenDot => '◦',
            Self::Circle => '●',
            Self::OpenCircle => '○',
            Self::DoubleCircle => '◉',
            Self::OpenDoubleCircle => '◎',
            Self::Triangle => '▲',
            Self::OpenTriangle => '△',
            Self::Sesame => '﹅',
            Self::OpenSesame => '﹆',
        })
    }
}

/// A text's character and paragraph values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    #[serde(default)]
    pub chars: CharStyle,
    /// A paragraph key: style only, never a mark.
    #[serde(default)]
    pub align: Option<Align>,
    /// A paragraph key: a multiple of each line's largest size.
    #[serde(default)]
    pub line_height: Option<f64>,
    /// A text key: the kind of face it's lettered in, whose style sets its
    /// family, weight, and italic where `chars` and the marks don't. Agents
    /// never set it, translation and the type bar do, so their schema leaves
    /// out its nineteen documented values, which would crowd a small model's
    /// context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub kernel: Option<Kernel>,
}

/// The kind of face a text is lettered in, as its source's was: one of the
/// kernels Morisawa, Founder, and DynaComware classify their faces by, as
/// font detection reads them. Declared by group, each group's first naming
/// it.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Type,
    JsonSchema,
    strum::VariantArray,
)]
#[serde(rename_all = "snake_case")]
pub enum Kernel {
    /// アンチック: antique kana, manga's commonest dialogue face; a text's
    /// style when detection isn't sure of its source's, and what every other
    /// style leaves out.
    Antique,
    /// 明朝体, 宋体.
    Mincho,
    /// 仿宋体.
    Fangsong,
    /// 楷書体, 楷体.
    Kaisho,
    /// 教科書体.
    Textbook,
    /// ゴシック体, 黑体.
    Gothic,
    /// 丸ゴシック体, 圆体.
    Rounded,
    /// 手書き: hand-lettered faces.
    Handwriting,
    /// 筆書体: brush faces.
    Brush,
    /// 行書体: running script.
    Gyosho,
    /// 行楷体: running regular script.
    Gyokai,
    /// 草書体: cursive script.
    Sosho,
    /// 隷書体: clerical script.
    Reisho,
    /// 篆書体: seal script.
    Tensho,
    /// 古印体: the worn strokes of old seals.
    Kointai,
    /// 魏碑体: Wei stele script.
    Weibei,
    /// デザイン書体: design faces.
    Design,
    /// ポップ体.
    Pop,
    /// Decorative faces, such as horror's.
    Decorative,
}

impl Kernel {
    /// The first kernel of this one's group, as Settings groups faces of one
    /// look to set at once: the mincho kin under mincho, the calligraphic
    /// hands under brush, and pop and decorative faces under design.
    #[must_use]
    pub fn group(self) -> Self {
        use Kernel::*;
        match self {
            Fangsong | Kaisho | Textbook => Mincho,
            Gyosho | Gyokai | Sosho | Reisho | Tensho | Kointai | Weibei => Brush,
            Pop | Decorative => Design,
            kernel => kernel,
        }
    }
}

/// The font a kernel's style sets for one script; each value it leaves out
/// is antique's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Face {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub weight: Option<u16>,
    #[serde(default)]
    pub italic: Option<bool>,
}

/// Text as spans in order, each with its marks, so no mark can fall outside the
/// text. Deserializes from a plain string too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(from = "RichTextInput")]
#[specta(type = Vec<Span>)]
pub struct RichText(pub Vec<Span>);

/// A string with its character marks and its ruby.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub insert: String,
    #[serde(default, skip_serializing_if = "unmarked")]
    pub attributes: CharStyle,
    /// The small text set beside `insert`, such as its reading. Neighbouring
    /// spans with the same ruby are one base, which it's set over once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruby: Option<String>,
}

fn unmarked(style: &CharStyle) -> bool {
    *style == CharStyle::default()
}

#[derive(Deserialize, Type, JsonSchema)]
#[serde(untagged)]
enum RichTextInput {
    Plain(String),
    Spans(Vec<Span>),
}

impl From<RichTextInput> for RichText {
    fn from(input: RichTextInput) -> Self {
        match input {
            RichTextInput::Plain(text) => Self::from(text.as_str()),
            RichTextInput::Spans(spans) => Self(spans),
        }
    }
}

impl RichText {
    /// The plain text.
    #[must_use]
    pub fn text(&self) -> String {
        self.0.iter().map(|span| span.insert.as_str()).collect()
    }

    /// Each ruby with its base, the UTF-8 offsets it's set over, in text
    /// order.
    #[must_use]
    pub fn ruby(&self) -> Vec<(Range<usize>, &str)> {
        let mut start = 0;
        let mut bases: Vec<(Range<usize>, &str)> = Vec::new();
        for span in &self.0 {
            let range = start..start + span.insert.len();
            start = range.end;
            let Some(ruby) = &span.ruby else { continue };
            match bases.last_mut() {
                Some((base, text)) if base.end == range.start && text == ruby => {
                    base.end = range.end;
                }
                _ => bases.push((range, ruby)),
            }
        }
        bases
    }

    /// `new` carrying these marks on the characters that stayed. A new
    /// character takes the marks of the one before it, kept or replaced, or of
    /// the first character at the start, as typing does in a text editor. A
    /// base keeps its ruby when its characters all stay, together, or when the
    /// change is all inside it, and to no more than half of it, which takes it
    /// in; one reworded otherwise loses it, since it reads the old wording.
    #[must_use]
    pub fn replaced(&self, new: &str) -> Self {
        let plain = self.text();
        let marks: Vec<&CharStyle> = self
            .0
            .iter()
            .flat_map(|span| span.insert.chars().map(|_| &span.attributes))
            .collect();
        let unmarked = CharStyle::default();
        let mut last = marks.first().copied().unwrap_or(&unmarked);
        let mut consumed = 0;
        let (mut from, mut to) = (0, 0);
        // Each old offset whose character stayed, with its new one.
        let mut stayed: Vec<(usize, usize)> = Vec::new();
        let mut spans = Vec::new();
        for change in TextDiff::from_chars(plain.as_str(), new).iter_all_changes() {
            let length = change.value().len();
            if change.tag() == ChangeTag::Equal {
                stayed.push((from, to));
            }
            if change.tag() != ChangeTag::Insert {
                last = marks[consumed];
                consumed += 1;
                from += length;
            }
            if change.tag() != ChangeTag::Delete {
                spans.push(Span {
                    insert: change.value().to_owned(),
                    attributes: last.clone(),
                    ruby: None,
                });
                to += length;
            }
        }
        let changed = changed(&plain, new);
        let bases: Vec<(Range<usize>, &str)> = self
            .ruby()
            .into_iter()
            .filter_map(|(base, ruby)| {
                let inside = changed.as_ref().is_some_and(|(changed, _)| {
                    base.start <= changed.start
                        && changed.end <= base.end
                        && 2 * changed.len() <= base.len()
                        // Typed at either end, it's outside.
                        && !(changed.is_empty()
                            && (changed.start == base.start || changed.end == base.end))
                });
                if let (true, Some((changed, inserted))) = (inside, &changed) {
                    return Some((base.start..base.end - changed.len() + inserted, ruby));
                }
                let kept = &stayed[stayed.partition_point(|(at, _)| *at < base.start)
                    ..stayed.partition_point(|(at, _)| *at < base.end)];
                let start = kept.first()?.1;
                let whole = kept.len() == plain[base.clone()].chars().count()
                    && kept.iter().all(|(at, to)| at - base.start == to - start);
                whole.then(|| (start..start + base.len(), ruby))
            })
            .collect();
        let mut at = 0;
        for span in &mut spans {
            span.ruby = bases
                .iter()
                .find(|(base, _)| base.contains(&at))
                .map(|(_, ruby)| (*ruby).to_owned());
            at += span.insert.len();
        }
        joined(spans)
    }

    /// `range`, UTF-8 offsets on character boundaries, replaced by `insert`
    /// carrying `marks`, as typing over a selection does; the other
    /// characters keep theirs. What's typed inside a base is part of it, and
    /// at its edge isn't.
    #[must_use]
    pub fn spliced(&self, range: Range<usize>, insert: &str, marks: &CharStyle) -> Self {
        let inserted = Span {
            insert: insert.to_owned(),
            attributes: marks.clone(),
            ruby: self
                .ruby()
                .into_iter()
                .find(|(base, _)| base.start < range.start && range.end < base.end)
                .map(|(_, ruby)| ruby.to_owned()),
        };
        joined(
            self.slice(0..range.start)
                .chain(std::iter::once(inserted))
                .chain(self.slice(range.end..usize::MAX)),
        )
    }

    /// `range`'s characters with `patch`'s values over their marks.
    #[must_use]
    pub fn marked(&self, range: Range<usize>, patch: &CharStyle) -> Self {
        let restyled = self.slice(range.clone()).map(|span| Span {
            attributes: patch.or(&span.attributes),
            ..span
        });
        joined(
            self.slice(0..range.start)
                .chain(restyled)
                .chain(self.slice(range.end..usize::MAX)),
        )
    }

    /// `range`'s characters as one base with `ruby` over them, or with none.
    /// A base the range cuts loses its ruby on the rest, which it no longer
    /// reads.
    #[must_use]
    pub fn annotated(&self, range: Range<usize>, ruby: Option<&str>) -> Self {
        let touched = self
            .ruby()
            .into_iter()
            .filter(|(base, _)| base.start < range.end && range.start < base.end)
            .fold(range.clone(), |touched, (base, _)| {
                touched.start.min(base.start)..touched.end.max(base.end)
            });
        let with = |range: Range<usize>, ruby: Option<&str>| {
            let ruby = ruby.map(str::to_owned);
            self.slice(range).map(move |span| Span {
                ruby: ruby.clone(),
                ..span
            })
        };
        joined(
            self.slice(0..touched.start)
                .chain(with(touched.start..range.start, None))
                .chain(with(range.clone(), ruby))
                .chain(with(range.end..touched.end, None))
                .chain(self.slice(touched.end..usize::MAX)),
        )
    }

    /// The marks a character typed at `offset` takes: the one's before it,
    /// or the first character's at the start, as `replaced` gives them.
    #[must_use]
    pub fn marks_at(&self, offset: usize) -> CharStyle {
        let mut start = 0;
        for span in &self.0 {
            let end = start + span.insert.len();
            if offset <= end {
                return span.attributes.clone();
            }
            start = end;
        }
        self.0
            .last()
            .map(|span| span.attributes.clone())
            .unwrap_or_default()
    }

    /// The spans' parts within `range`, UTF-8 offsets on character
    /// boundaries.
    fn slice(&self, range: Range<usize>) -> impl Iterator<Item = Span> + '_ {
        let mut start = 0;
        self.0.iter().filter_map(move |span| {
            let (from, to) = (start, start + span.insert.len());
            start = to;
            let (cut_from, cut_to) = (range.start.max(from), range.end.min(to));
            (cut_from < cut_to).then(|| Span {
                insert: span.insert[cut_from - from..cut_to - from].to_owned(),
                attributes: span.attributes.clone(),
                ruby: span.ruby.clone(),
            })
        })
    }
}

impl From<&str> for RichText {
    fn from(text: &str) -> Self {
        Self(match text.is_empty() {
            true => Vec::new(),
            false => vec![Span {
                insert: text.to_owned(),
                ..Span::default()
            }],
        })
    }
}

pub(crate) fn und() -> LanguageIdentifier {
    LanguageIdentifier::UNKNOWN
}

/// Styled text in a frame. `content` is set when its layer is added; after
/// that only the text ops change it, except a block's source, whose wording
/// a patch writes, as OCR does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Text {
    #[serde(default)]
    pub content: RichText,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default)]
    pub frame: Frame,
    #[serde(default)]
    pub direction: Direction,
    /// A BCP 47 tag; `und` when undetermined.
    #[serde(default = "und")]
    #[specta(type = String)]
    #[schemars(with = "String")]
    pub language: LanguageIdentifier,
    #[serde(default)]
    pub transform: Transform,
}

impl Text {
    /// Where its frame lies on the page, its transform applied; `None` for
    /// point text.
    #[must_use]
    pub fn bounds(&self) -> Option<kurbo::Rect> {
        let rect = match &self.frame {
            Frame::Point => return None,
            Frame::Box { bounds, .. } => kurbo::Rect::from(*bounds),
            Frame::Path { path, .. } => BezPath::from(path).bounding_box(),
        };
        Some(Affine::from(self.transform).transform_rect_bbox(rect))
    }

    /// Its place and style without its words or their language: what a block's
    /// translation starts as, from its source, whichever writer adds it, so
    /// new lettering lies where the original did.
    #[must_use]
    pub fn blank(&self) -> Self {
        Self {
            content: RichText::default(),
            language: und(),
            ..self.clone()
        }
    }
}

/// Empty point text in an undetermined language.
impl Default for Text {
    fn default() -> Self {
        Self {
            content: RichText::default(),
            style: TextStyle::default(),
            frame: Frame::default(),
            direction: Direction::default(),
            language: und(),
            transform: Transform::default(),
        }
    }
}

/// The one stretch of `old` that differs in `new`, and how long it is there;
/// `None` when they're the same.
fn changed(old: &str, new: &str) -> Option<(Range<usize>, usize)> {
    let start: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let (old, new) = (&old[start..], &new[start..]);
    let after: usize = old
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let changed = start..start + old.len() - after;
    let inserted = new.len() - after;
    (!changed.is_empty() || inserted > 0).then_some((changed, inserted))
}

/// `spans` with empty ones and empty ruby dropped and equal neighbours joined.
/// White space at a base's ends isn't read, so its ruby isn't over it: there it
/// would keep the line from breaking beside the word.
pub(crate) fn joined(spans: impl IntoIterator<Item = Span>) -> RichText {
    let text = merged(spans);
    let plain = text.text();
    let loose: Vec<Range<usize>> = text
        .ruby()
        .into_iter()
        .flat_map(|(base, _)| {
            let read = &plain[base.clone()];
            let start = base.end - read.trim_start().len();
            let end = (base.start + read.trim_end().len()).max(start);
            [base.start..start, end..base.end]
        })
        .filter(|space| !space.is_empty())
        .collect();
    loose.into_iter().fold(text, |text, space| {
        merged(
            text.slice(0..space.start)
                .chain(
                    text.slice(space.clone())
                        .map(|span| Span { ruby: None, ..span }),
                )
                .chain(text.slice(space.end..usize::MAX)),
        )
    })
}

fn merged(spans: impl IntoIterator<Item = Span>) -> RichText {
    let mut out: Vec<Span> = Vec::new();
    for mut span in spans {
        span.ruby = span.ruby.filter(|ruby| !ruby.is_empty());
        match out.last_mut() {
            _ if span.insert.is_empty() => {}
            Some(last) if last.attributes == span.attributes && last.ruby == span.ruby => {
                last.insert.push_str(&span.insert);
            }
            _ => out.push(span),
        }
    }
    RichText(out)
}
