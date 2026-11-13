//! The text tool: point text at a click, box text over a drag.

use koharu_scene::{
    Bounds, Content, Edit, Frame, LanguageIdentifier, RichText, Text, TextStyle, Transform,
};
use vello::kurbo::{Point, Rect};

use super::added;
use crate::{Effect, draw::overlay::Overlay, gesture::Context, view::Sample};

/// A text drag shorter than this on both sides, in device-independent
/// pixels, is a click, which makes point text, as the editor's was.
const CLICK: f64 = 4.0;

/// A text frame being dragged out, in page points.
pub(crate) struct Draft {
    start: Point,
    current: Point,
}

impl Draft {
    pub fn new(sample: Sample) -> Self {
        Self {
            start: sample.page,
            current: sample.page,
        }
    }

    pub fn rect(&self) -> Rect {
        Rect::from_points(self.start, self.current)
    }

    pub fn moved(&mut self, samples: &[Sample]) {
        if let Some(last) = samples.last() {
            self.current = last.page;
        }
    }

    /// Text where the drag went, at the top of the page, selected.
    pub fn finish(self, cx: &Context) -> Vec<Effect> {
        let rect = self.rect();
        let click = cx.view.page_length(CLICK);
        let (origin, frame) = match rect.width() < click && rect.height() < click {
            true => (self.start, Frame::Point),
            false => (
                rect.origin(),
                Frame::Box {
                    bounds: Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: rect.width(),
                        height: rect.height(),
                    },
                    fit: Default::default(),
                    anchor: Default::default(),
                },
            ),
        };
        let text = Text {
            content: RichText::default(),
            style: TextStyle::default(),
            frame,
            direction: Default::default(),
            language: LanguageIdentifier::UNKNOWN,
            transform: Transform([[1.0, 0.0, origin.x], [0.0, 1.0, origin.y]]),
        };
        let (id, add) = added(cx.snapshot.page(), Content::Text(Box::new(text)));
        vec![
            Effect::Edit(Edit {
                label: "add_text".to_owned(),
                ops: vec![add],
            }),
            Effect::Selection(vec![id]),
        ]
    }

    pub fn preview(&self, overlay: &mut Overlay) {
        overlay.dashed(self.rect());
    }
}
