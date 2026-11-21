//! What typing tells the host for its type bar: the character style of the
//! selection, as Photoshop's Character panel shows it, a value the selected
//! characters don't share left empty.

use koharu_scene::{CharStyle, LayerId};
use serde::Serialize;
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
        }
    }
    (chars, mixed)
}
