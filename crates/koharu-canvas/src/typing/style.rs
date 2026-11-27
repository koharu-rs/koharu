//! What typing tells the host for its type bar: the character style of the
//! selection, as Photoshop's Character panel shows it, a value the selected
//! characters don't share left empty; and the base its ruby would go over.

use koharu_scene::{CharStyle, LayerId};
use serde::{Deserialize, Serialize};
use specta::Type;

/// One of the values a character's style holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CharKey {
    Family,
    Weight,
    Italic,
    Size,
    Fill,
    Stroke,
    StrokeWidth,
    Orientation,
    Emphasis,
}

/// What the type bar's ruby field does to the text being typed.
#[derive(Clone, Debug, PartialEq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RubyEdit {
    /// Opens on the selected characters, or on the base the caret is in, which
    /// its ruby goes over until it closes.
    Open,
    /// Sets its ruby, or takes it off.
    Set { ruby: Option<String> },
    /// Closes, keeping the ruby.
    Keep,
    /// Closes, putting back the text as it opened on it.
    Cancel,
}

/// The characters a ruby goes over, as Clip Studio Paint shows a reading's
/// target string.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct RubyBase {
    pub text: String,
    pub ruby: Option<String>,
}

/// The text being typed into, as the host shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct Typed {
    /// The layer typed into; `None` for a new text the commit adds.
    pub layer: Option<LayerId>,
    /// Each value the selected characters share, the text's style under
    /// their marks; at a caret, the next character's.
    pub chars: CharStyle,
    /// The values the selected characters don't share, which `chars` leaves
    /// empty.
    pub mixed: Vec<CharKey>,
    /// The selected characters, or the base the caret is in; `None` at a caret
    /// in none.
    pub base: Option<RubyBase>,
    /// Whether the ruby field is open on `base`: anything else that changes
    /// the text or moves the caret closes it.
    pub annotating: bool,
}

/// What `styles`, each a character's whole style, share, and the values
/// they don't, which the shared style leaves empty.
pub(super) fn shared(mut styles: impl Iterator<Item = CharStyle>) -> (CharStyle, Vec<CharKey>) {
    let Some(mut chars) = styles.next() else {
        return (CharStyle::default(), Vec::new());
    };
    let mut mixed = Vec::new();
    for style in styles {
        let mut differs = |key: CharKey, same: bool| {
            if !same && !mixed.contains(&key) {
                mixed.push(key);
            }
        };
        differs(CharKey::Family, chars.family == style.family);
        differs(CharKey::Weight, chars.weight == style.weight);
        differs(CharKey::Italic, chars.italic == style.italic);
        differs(CharKey::Size, chars.size == style.size);
        differs(CharKey::Fill, chars.fill == style.fill);
        differs(CharKey::Stroke, chars.stroke == style.stroke);
        differs(
            CharKey::StrokeWidth,
            chars.stroke_width == style.stroke_width,
        );
        differs(CharKey::Orientation, chars.orientation == style.orientation);
        differs(CharKey::Emphasis, chars.emphasis == style.emphasis);
    }
    for key in &mixed {
        match key {
            CharKey::Family => chars.family = None,
            CharKey::Weight => chars.weight = None,
            CharKey::Italic => chars.italic = None,
            CharKey::Size => chars.size = None,
            CharKey::Fill => chars.fill = None,
            CharKey::Stroke => chars.stroke = None,
            CharKey::StrokeWidth => chars.stroke_width = None,
            CharKey::Orientation => chars.orientation = None,
            CharKey::Emphasis => chars.emphasis = None,
        }
    }
    (chars, mixed)
}
