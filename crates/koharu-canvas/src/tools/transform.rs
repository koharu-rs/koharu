//! Moving, resizing, and turning selected layers: the drag, its preview, and
//! the ops it leaves. `transform_layer` moves each kind of layer by its own
//! values, and a text's frame resizes as the scene's own `Frame`.

use cursor_icon::CursorIcon;
use koharu_renderer::Frame;
use koharu_scene::{Bounds, Content, Edit, Frame as TextFrame, LayerId, Op, Path, Ref, Text};
use kurbo::{Affine, BezPath, Point, Rect};
use serde_json::{Value, json};

use crate::{
    Effect,
    control::{Control, Handle},
    draw::overlay::Overlay,
    gesture::Context,
    snapshot::Snapshot,
    view::Sample,
};

/// The shortest side a resize leaves, in a layer's own units: a page pixel
/// for every layer the canvas resizes but a scaled image.
const MINIMUM: f64 = 1.0;

/// What a drag on selected layers does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Move,
    Resize(Handle),
    Rotate,
}

/// Selected layers dragged, by their body or one's handle, from `start` to
/// `current` in page points.
pub(crate) struct Transform {
    mode: Mode,
    start: Point,
    current: Point,
    controls: Vec<Control>,
    /// What a move or a turn lifts out of the page, made once when it
    /// starts: the layers, the frame without them, and each one's own
    /// drawing beside the control it follows.
    lifted: Vec<LayerId>,
    under: Option<Frame>,
    drawn: Vec<(usize, Frame)>,
}

impl Transform {
    pub fn new(cx: &Context, mode: Mode, start: Point, controls: Vec<Control>) -> Self {
        let frame = cx.snapshot.frame();
        let drawn: Vec<(usize, Frame)> = match mode {
            Mode::Resize(_) => Vec::new(),
            Mode::Move | Mode::Rotate => controls
                .iter()
                .enumerate()
                .filter_map(|(index, control)| {
                    // A block draws only its translation, so one without has
                    // nothing to lift; a clean-up isn't in the layer tree.
                    let content = cx.snapshot.layer(control.layer).map(|layer| &layer.content);
                    let drawn = match content {
                        Some(Content::Block(_)) => frame.translation(control.layer),
                        _ => frame.layer(control.layer),
                    };
                    drawn.map(|drawn| (index, drawn.cropped()))
                })
                .collect(),
        };
        let lifted: Vec<LayerId> = drawn
            .iter()
            .map(|(index, _)| controls[*index].layer)
            .collect();
        let under = (!lifted.is_empty()).then(|| frame.without(&lifted));
        Self {
            mode,
            start,
            current: start,
            controls,
            lifted,
            under,
            drawn,
        }
    }

    pub fn current(&self) -> Point {
        self.current
    }

    pub fn lifted(&self) -> &[LayerId] {
        &self.lifted
    }

    pub fn under(&self) -> Option<&Frame> {
        self.under.as_ref()
    }

    pub fn moved(&mut self, samples: &[Sample]) {
        if let Some(last) = samples.last() {
            self.current = last.page;
        }
    }

    pub fn cursor(&self) -> CursorIcon {
        match self.mode {
            Mode::Move => CursorIcon::Default,
            Mode::Resize(handle) => handle.cursor(),
            Mode::Rotate => Handle::Rotate.cursor(),
        }
    }

    /// The edit the drag leaves, if it changed anything.
    pub fn finish(self, cx: &Context) -> Vec<Effect> {
        let (label, ops): (&str, Vec<Op>) = match self.mode {
            Mode::Move | Mode::Rotate => {
                let ops = self
                    .controls
                    .iter()
                    .filter_map(|control| {
                        let affine = self.turn(control);
                        (affine != Affine::IDENTITY).then(|| transformed(control, affine))
                    })
                    .collect();
                (
                    if self.mode == Mode::Move {
                        "move"
                    } else {
                        "rotate"
                    },
                    ops,
                )
            }
            Mode::Resize(handle) => {
                let ops = self
                    .controls
                    .iter()
                    .filter_map(|control| {
                        let local = control.resize_to(handle, self.current, MINIMUM);
                        (local != control.local())
                            .then(|| resized(cx.snapshot, control, local))
                            .flatten()
                    })
                    .collect();
                ("resize", ops)
            }
        };
        if ops.is_empty() {
            return Vec::new();
        }
        vec![Effect::Edit(Edit {
            label: label.to_owned(),
            ops,
        })]
    }

    /// The controls, where the drag puts them.
    pub fn dragged(&self) -> Vec<Control> {
        self.controls
            .iter()
            .map(|control| match self.mode {
                Mode::Resize(handle) => {
                    control.with_local(control.resize_to(handle, self.current, MINIMUM))
                }
                Mode::Move | Mode::Rotate => control.moved(self.turn(control)),
            })
            .collect()
    }

    /// Each lifted layer's own drawing, where the drag takes it.
    pub fn preview(&self, overlay: &mut Overlay) -> anyhow::Result<()> {
        for (index, own) in &self.drawn {
            overlay.frame(own, self.turn(&self.controls[*index]))?;
        }
        Ok(())
    }

    /// What a move or a turn does to `control` so far, in page space.
    fn turn(&self, control: &Control) -> Affine {
        match self.mode {
            Mode::Move => Affine::translate(self.current - self.start),
            Mode::Rotate => control.turn(self.start, self.current),
            Mode::Resize(_) => Affine::IDENTITY,
        }
    }
}

/// The op that carries `control`'s layer along `affine`, a page-space move
/// or turn. A picture, which can't turn, moves by whole pixels, as the scene
/// takes it.
pub(crate) fn transformed(control: &Control, affine: Affine) -> Op {
    let affine = match control.rotates {
        true => affine,
        false => Affine::translate(affine.translation().round()),
    };
    Op::TransformLayer {
        layer: Ref::from(control.layer),
        transform: affine.into(),
    }
}

/// The op that gives `control` the frame `local` in its own coordinates,
/// for a resize; `None` for what doesn't resize. Text resizes its frame
/// without scaling its type, as Photoshop resizes a paragraph's box; an
/// image or a shape is stretched.
pub(crate) fn resized(snapshot: &Snapshot, control: &Control, local: Rect) -> Option<Op> {
    let stretch = map(control.local(), local);
    let layer = Ref::from(control.layer);
    let content = &snapshot.layer(control.layer)?.content;
    if let Content::Image(_) | Content::Shape(_) = content {
        let page = control.transform() * stretch * control.transform().inverse();
        return Some(Op::TransformLayer {
            layer,
            transform: page.into(),
        });
    }
    let (lettering, text) = Lettering::of(content)?;
    Some(Op::UpdateLayer {
        layer,
        patch: lettering.patch(frame(text, local, stretch)?),
    })
}

/// Where a layer's drawn text lives among its values: a text layer's own, a
/// block's translation, or a block's source while it has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lettering {
    Text,
    Translation,
    Source,
}

impl Lettering {
    pub fn of(content: &Content) -> Option<(Self, &Text)> {
        match content {
            Content::Text(text) => Some((Self::Text, text)),
            Content::Block(block) => Some(match &block.translation {
                Some(text) => (Self::Translation, text),
                None => (Self::Source, &block.source),
            }),
            _ => None,
        }
    }

    /// The layer's patch giving that text `frame`.
    pub fn patch(self, frame: Value) -> Value {
        match self {
            Self::Text => json!({"frame": frame}),
            Self::Translation => json!({"translation": {"frame": frame}}),
            Self::Source => json!({"source": {"frame": frame}}),
        }
    }
}

/// `text`'s frame resized to `local`: a box takes it as its bounds, and a
/// path is stretched onto it.
fn frame(text: &Text, local: Rect, stretch: Affine) -> Option<Value> {
    let mut frame = text.frame.clone();
    match &mut frame {
        TextFrame::Box { bounds, .. } => {
            *bounds = Bounds {
                x: local.x0,
                y: local.y0,
                width: local.width(),
                height: local.height(),
            };
        }
        TextFrame::Path { path, .. } => {
            let mut outline = BezPath::from_svg(path.as_str()).ok()?;
            outline.apply_affine(stretch);
            *path = Path::try_from(outline.to_svg()).ok()?;
        }
        TextFrame::Point => return None,
    }
    serde_json::to_value(frame).ok()
}

/// The affine taking `from` onto `to`.
fn map(from: Rect, to: Rect) -> Affine {
    Affine::translate(to.origin().to_vec2())
        * Affine::scale_non_uniform(to.width() / from.width(), to.height() / from.height())
        * Affine::translate(-from.origin().to_vec2())
}
