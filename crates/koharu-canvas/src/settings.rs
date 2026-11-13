//! What the host sets on the canvas: the tool, the brush, and the theme's
//! colours. The host owns them; the canvas only reads them.

use cursor_icon::CursorIcon;
use koharu_scene::Rgba;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    #[default]
    Select,
    Text,
    Draw,
    Eraser,
    ColorPicker,
    Remove,
    Pan,
}

impl Tool {
    /// Whether it strokes with the brush.
    pub(crate) fn is_brush(self) -> bool {
        matches!(self, Self::Draw | Self::Eraser | Self::Remove)
    }

    /// The pointer's shape over the canvas with nothing under it, in CSS's
    /// names; `None` hides it, as the brush tools draw their own ring.
    pub(crate) fn cursor(self) -> Option<CursorIcon> {
        match self {
            Self::Select => Some(CursorIcon::Default),
            Self::Text => Some(CursorIcon::Text),
            Self::Draw | Self::Eraser | Self::Remove => None,
            Self::ColorPicker => Some(CursorIcon::Crosshair),
            Self::Pan => Some(CursorIcon::Grab),
        }
    }
}

/// The brush the draw, eraser, and remove tools stroke with.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Brush {
    /// In page pixels.
    pub diameter: f64,
    pub color: Rgba,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            diameter: 16.0,
            color: Rgba([0.0, 0.0, 0.0, 1.0]),
        }
    }
}

/// The app's theme colours the canvas draws with.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct Colors {
    /// Around the page.
    pub workspace: Rgba,
    /// Outlines, handles, and the marquee.
    pub selection: Rgba,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            workspace: Rgba([0.15, 0.15, 0.16, 1.0]),
            selection: Rgba([0.05, 0.6, 1.0, 1.0]),
        }
    }
}
