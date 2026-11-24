//! What the host sets on the canvas: the tool, the brush, the theme's
//! colours, and the Story panel's state. The host owns them; the canvas only
//! reads them.

use cursor_icon::CursorIcon;
use koharu_scene::{LayerId, Rgba};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    #[default]
    Select,
    DirectSelect,
    /// Photoshop's Rectangular Marquee: marks a rectangle of the page's
    /// pixels.
    Marquee,
    /// Photoshop's Lasso: marks a freehand shape of the page's pixels.
    Lasso,
    Text,
    /// Draws a text block, for a text detection missed.
    Block,
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
            Self::Select | Self::DirectSelect => Some(CursorIcon::Default),
            Self::Text => Some(CursorIcon::Text),
            Self::Draw | Self::Eraser | Self::Remove => None,
            Self::Marquee | Self::Lasso | Self::Block | Self::ColorPicker => {
                Some(CursorIcon::Crosshair)
            }
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

/// The Story panel as the canvas follows it: while it shows, every text's
/// frame and its number in the scene's reading order, and the frame of the
/// text whose entry the pointer is on drawn stronger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Story {
    pub shown: bool,
    pub hovered: Option<LayerId>,
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
