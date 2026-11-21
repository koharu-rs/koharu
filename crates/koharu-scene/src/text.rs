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
        }
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
}

/// Text as spans in order, each with its marks, so no mark can fall outside the
/// text. Deserializes from a plain string too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(from = "RichTextInput")]
#[specta(type = Vec<Span>)]
pub struct RichText(pub Vec<Span>);

/// A string with its character marks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub insert: String,
    #[serde(default, skip_serializing_if = "unmarked")]
    pub attributes: CharStyle,
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

    /// `new` carrying these marks on the characters that stayed. A new
    /// character takes the marks of the one before it, kept or replaced, or of
    /// the first character at the start, as typing does in a text editor.
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
        let mut spans = Vec::new();
        for change in TextDiff::from_chars(plain.as_str(), new).iter_all_changes() {
            if change.tag() != ChangeTag::Insert {
                last = marks[consumed];
                consumed += 1;
            }
            if change.tag() != ChangeTag::Delete {
                spans.push(Span {
                    insert: change.value().to_owned(),
                    attributes: last.clone(),
                });
            }
        }
        joined(spans)
    }

    /// `range`, UTF-8 offsets on character boundaries, replaced by `insert`
    /// carrying `marks`, as typing over a selection does; the other
    /// characters keep theirs.
    #[must_use]
    pub fn spliced(&self, range: Range<usize>, insert: &str, marks: &CharStyle) -> Self {
        let inserted = Span {
            insert: insert.to_owned(),
            attributes: marks.clone(),
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
                attributes: CharStyle::default(),
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

/// `spans` with empty ones dropped and equal neighbours joined.
pub(crate) fn joined(spans: impl IntoIterator<Item = Span>) -> RichText {
    let mut out: Vec<Span> = Vec::new();
    for span in spans {
        match out.last_mut() {
            _ if span.insert.is_empty() => {}
            Some(last) if last.attributes == span.attributes => last.insert.push_str(&span.insert),
            _ => out.push(span),
        }
    }
    RichText(out)
}
