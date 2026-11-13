//! Text: styled spans in a frame. A text layer holds one, and so do a block's
//! source and translation.

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
            Frame::Box { bounds, .. } => kurbo::Rect::new(
                bounds.x,
                bounds.y,
                bounds.x + bounds.width,
                bounds.y + bounds.height,
            ),
            Frame::Path { path, .. } => BezPath::from_svg(path.as_str()).ok()?.bounding_box(),
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

/// `new` carrying `old`'s marks on the characters that stayed. A new character
/// takes the marks of the one before it, kept or replaced, or of the first
/// character at the start, as typing does in a text editor.
pub(crate) fn replace_keeping_marks(old: &RichText, new: &str) -> RichText {
    let plain = old.text();
    let marks: Vec<&CharStyle> = old
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
