//! The window's pointer, routed to the canvas where CEF painted nothing over
//! its viewport and to the web interface everywhere else, as each side's
//! own events.

use std::{
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use koharu_canvas::{Point, Rect};
use serde::Deserialize;
use specta::Type;
use tauri_runtime_cef::cef::{ImplBrowserHost as _, MouseButtonType, MouseEvent};
use ui_events::{
    ScrollDelta,
    keyboard::Modifiers,
    pointer::{
        PointerButton, PointerButtonEvent, PointerButtons, PointerEvent, PointerId, PointerInfo,
        PointerScrollEvent, PointerState, PointerType, PointerUpdate,
    },
};
use winit::{
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::ModifiersState,
    window::Window,
};

use super::{Screen, keys, set_cursor};
use crate::app::App;

/// A native pointer event, in the window's physical pixels: as the window
/// delivers it, and as the debug build's `debug_pointer` feeds it.
#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Pointer {
    Moved {
        x: f64,
        y: f64,
    },
    Pressed {
        x: f64,
        y: f64,
        button: Button,
    },
    Released {
        x: f64,
        y: f64,
        button: Button,
    },
    /// At the pointer's last position: notches when `lines`, pixels
    /// otherwise, with winit's sign.
    Wheel {
        dx: f32,
        dy: f32,
        lines: bool,
    },
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
}

impl Button {
    /// CEF's flag of the button held.
    fn flag(self) -> u32 {
        match self {
            Self::Left => keys::LEFT_BUTTON,
            Self::Middle => keys::MIDDLE_BUTTON,
            Self::Right => keys::RIGHT_BUTTON,
        }
    }

    fn ui(self) -> PointerButton {
        match self {
            Self::Left => PointerButton::Primary,
            Self::Middle => PointerButton::Auxiliary,
            Self::Right => PointerButton::Secondary,
        }
    }

    fn cef(self) -> MouseButtonType {
        match self {
            Self::Left => MouseButtonType::LEFT,
            Self::Middle => MouseButtonType::MIDDLE,
            Self::Right => MouseButtonType::RIGHT,
        }
    }
}

impl Pointer {
    pub fn from_window_event(event: &WindowEvent) -> Option<Self> {
        Some(match event {
            WindowEvent::PointerMoved { position, .. } => Self::Moved {
                x: position.x,
                y: position.y,
            },
            WindowEvent::PointerButton {
                state,
                position,
                button,
                ..
            } => {
                let button = match button.clone().mouse_button()? {
                    MouseButton::Left => Button::Left,
                    MouseButton::Middle => Button::Middle,
                    MouseButton::Right => Button::Right,
                    _ => return None,
                };
                let (x, y) = (position.x, position.y);
                match state {
                    ElementState::Pressed => Self::Pressed { x, y, button },
                    ElementState::Released => Self::Released { x, y, button },
                }
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                MouseScrollDelta::LineDelta(dx, dy) => Self::Wheel {
                    dx: *dx,
                    dy: *dy,
                    lines: true,
                },
                MouseScrollDelta::PixelDelta(delta) => Self::Wheel {
                    dx: delta.x as f32,
                    dy: delta.y as f32,
                    lines: false,
                },
                _ => return None,
            },
            WindowEvent::PointerLeft { .. } => Self::Left,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    Canvas,
    #[default]
    Web,
}

/// The side under a pointer at `at`, in window pixels: the canvas inside
/// its viewport where CEF's paint is fully transparent, and the interface
/// wherever CEF painted anything, a panel over the canvas included.
pub fn route(at: Point, viewport: Rect, painted_alpha: u8) -> Side {
    if painted_alpha == 0 && viewport.contains(at) {
        Side::Canvas
    } else {
        Side::Web
    }
}

/// A drag: the side its first press went to keeps every pointer event until
/// all of its buttons are up, wherever the pointer goes.
#[derive(Default)]
pub struct Drag {
    side: Option<Side>,
    /// The buttons held, as bit flags.
    buttons: u32,
}

impl Drag {
    /// The side a drag holds; `None` between drags.
    pub fn side(&self) -> Option<Side> {
        self.side
    }

    /// A press of `button`, `under` the pointer: it starts a drag there, or
    /// joins the one going.
    pub fn press(&mut self, button: u32, under: Side) -> Side {
        self.buttons |= button;
        *self.side.get_or_insert(under)
    }

    /// A release of `button`, which goes where its drag went; the drag ends
    /// with its last button. A release no press began goes to the interface.
    pub fn release(&mut self, button: u32) -> Side {
        self.buttons &= !button;
        let side = self.side.unwrap_or(Side::Web);
        if self.buttons == 0 {
            self.side = None;
        }
        side
    }
}

/// The pointer and keyboard state both sides' events carry.
#[derive(Default)]
pub struct Input {
    /// The pointer, in window pixels.
    position: Point,
    pub modifiers: ModifiersState,
    /// CEF's flags of the mouse buttons held.
    buttons: u32,
    last_press: Option<Press>,
    drag: Drag,
    /// The side the pointer was last over.
    hover: Side,
}

/// A press, to count the clicks of a double or triple click.
struct Press {
    button: u32,
    at: Instant,
    position: Point,
    count: u8,
}

/// Windows' defaults for a double click: within 500 ms and 4 pixels.
const CLICK_TIME: Duration = Duration::from_millis(500);
const CLICK_DISTANCE: f64 = 4.0;

/// When pointer times count from.
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

impl Input {
    pub fn over_web(&self) -> bool {
        self.hover == Side::Web
    }

    /// Records a press of `flag`, and answers how many clicks it makes.
    fn click(&mut self, flag: u32, scale: f64) -> u8 {
        self.buttons |= flag;
        let (now, position) = (Instant::now(), self.position);
        let count = match &self.last_press {
            Some(last)
                if last.button == flag
                    && now.duration_since(last.at) <= CLICK_TIME
                    && (last.position - position).hypot() <= CLICK_DISTANCE * scale =>
            {
                last.count.saturating_add(1)
            }
            _ => 1,
        };
        self.last_press = Some(Press {
            button: flag,
            at: now,
            position,
            count,
        });
        count
    }
}

/// Routes one pointer event to its side.
pub fn pointer(app: &Arc<App>, window: &dyn Window, pointer: Pointer) {
    let screen = &app.screen;
    let scale = window.scale_factor();
    match pointer {
        Pointer::Moved { x, y } => {
            let at = Point::new(x, y);
            let held = {
                let mut input = screen.input.lock();
                input.position = at;
                input.drag.side()
            };
            let side = held.unwrap_or_else(|| screen.under(at));
            hover(app, window, side);
            match side {
                Side::Web => {
                    let event = screen.mouse_event(scale);
                    if let Some(host) = screen.host() {
                        host.send_mouse_move_event(Some(&event), 0);
                    }
                }
                Side::Canvas => {
                    let update = PointerUpdate {
                        pointer: mouse(),
                        current: screen.pointer_state(scale, 0),
                        coalesced: Vec::new(),
                        predicted: Vec::new(),
                    };
                    canvas(app, window, PointerEvent::Move(update));
                }
            }
        }
        Pointer::Pressed { x, y, button } => {
            let at = Point::new(x, y);
            screen.input.lock().position = at;
            let under = screen.under(at);
            let (side, count, first) = {
                let mut input = screen.input.lock();
                let first = input.drag.side().is_none();
                let side = input.drag.press(button.flag(), under);
                (side, input.click(button.flag(), scale), first)
            };
            hover(app, window, side);
            match side {
                Side::Web => {
                    let event = screen.mouse_event(scale);
                    if let Some(host) = screen.host() {
                        host.send_mouse_click_event(
                            Some(&event),
                            button.cef(),
                            0,
                            i32::from(count),
                        );
                    }
                }
                Side::Canvas => {
                    if first {
                        screen.blur();
                    }
                    let down = PointerButtonEvent {
                        button: Some(button.ui()),
                        pointer: mouse(),
                        state: screen.pointer_state(scale, count),
                    };
                    canvas(app, window, PointerEvent::Down(down));
                }
            }
        }
        Pointer::Released { x, y, button } => {
            let (side, count) = {
                let mut input = screen.input.lock();
                input.position = Point::new(x, y);
                input.buttons &= !button.flag();
                let count = input.last_press.as_ref().map_or(1, |press| press.count);
                (input.drag.release(button.flag()), count)
            };
            match side {
                Side::Web => {
                    let event = screen.mouse_event(scale);
                    if let Some(host) = screen.host() {
                        host.send_mouse_click_event(
                            Some(&event),
                            button.cef(),
                            1,
                            i32::from(count),
                        );
                    }
                }
                Side::Canvas => {
                    let up = PointerButtonEvent {
                        button: Some(button.ui()),
                        pointer: mouse(),
                        state: screen.pointer_state(scale, count),
                    };
                    canvas(app, window, PointerEvent::Up(up));
                }
            }
            // The drag over, the pointer belongs to the side it's over.
            let (held, at) = {
                let input = screen.input.lock();
                (input.drag.side(), input.position)
            };
            if held.is_none() {
                hover(app, window, screen.under(at));
            }
        }
        Pointer::Wheel { dx, dy, lines } => {
            let (held, at) = {
                let input = screen.input.lock();
                (input.drag.side(), input.position)
            };
            match held.unwrap_or_else(|| screen.under(at)) {
                Side::Web => {
                    // Chromium scrolls by pixels, and a notch is 120 of
                    // them, Windows' convention, which CEF follows
                    // everywhere; its sign is winit's.
                    let (dx, dy) = if lines {
                        (dx * 120.0, dy * 120.0)
                    } else {
                        (dx, dy)
                    };
                    let event = screen.mouse_event(scale);
                    if let Some(host) = screen.host() {
                        host.send_mouse_wheel_event(Some(&event), dx as i32, dy as i32);
                    }
                }
                Side::Canvas => {
                    // The canvas takes a browser's sign, where a positive
                    // delta moves the view down, against winit's.
                    let delta = if lines {
                        ScrollDelta::LineDelta(-dx, -dy)
                    } else {
                        ScrollDelta::PixelDelta((-f64::from(dx), -f64::from(dy)).into())
                    };
                    let scroll = PointerScrollEvent {
                        pointer: mouse(),
                        delta,
                        state: screen.pointer_state(scale, 0),
                    };
                    canvas(app, window, PointerEvent::Scroll(scroll));
                }
            }
        }
        Pointer::Left => {
            if screen.input.lock().drag.side().is_none() {
                hover(app, window, Side::Web);
                let event = screen.mouse_event(scale);
                if let Some(host) = screen.host() {
                    host.send_mouse_move_event(Some(&event), 1);
                }
            }
        }
    }
}

/// Moves the pointer to `side`: the side it leaves hears that it left, and
/// the cursor becomes the new side's.
fn hover(app: &Arc<App>, window: &dyn Window, side: Side) {
    let screen = &app.screen;
    let left = std::mem::replace(&mut screen.input.lock().hover, side);
    if left == side {
        return;
    }
    match side {
        Side::Canvas => {
            let event = screen.mouse_event(window.scale_factor());
            if let Some(host) = screen.host() {
                host.send_mouse_move_event(Some(&event), 1);
            }
        }
        Side::Web => {
            canvas(app, window, PointerEvent::Leave(mouse()));
            set_cursor(window, *screen.web_cursor.lock());
        }
    }
}

/// Gives the canvas `event`, carries out what it answers, and shows its
/// cursor while the pointer is over it.
fn canvas(app: &Arc<App>, window: &dyn Window, event: PointerEvent) {
    let left = matches!(event, PointerEvent::Leave(_));
    let answer = app.screen.with_canvas(|canvas| {
        canvas
            .input(event)
            .map(|effects| (effects, canvas.cursor()))
    });
    match answer {
        Some(Ok((effects, cursor))) => {
            if !left {
                set_cursor(window, cursor);
            }
            app.handle(effects);
            app.screen.request_redraw();
        }
        Some(Err(error)) => tracing::error!("the canvas failed a pointer event: {error:#}"),
        None => {}
    }
}

impl Screen {
    /// The side under a pointer at `at`.
    fn under(&self, at: Point) -> Side {
        if self.canvas.lock().is_none() {
            return Side::Web;
        }
        let alpha = self
            .painted
            .lock()
            .alpha(at.x.max(0.0) as u32, at.y.max(0.0) as u32);
        route(at, self.physical_viewport(), alpha)
    }

    /// A CEF mouse event at the pointer, in the view's DIPs.
    fn mouse_event(&self, scale: f64) -> MouseEvent {
        let input = self.input.lock();
        MouseEvent {
            x: (input.position.x / scale).round() as i32,
            y: (input.position.y / scale).round() as i32,
            modifiers: keys::flags(input.modifiers, input.buttons),
        }
    }

    /// The pointer as the canvas takes it.
    fn pointer_state(&self, scale: f64, count: u8) -> PointerState {
        let input = self.input.lock();
        let mut buttons = PointerButtons::default();
        for button in [Button::Left, Button::Middle, Button::Right] {
            if input.buttons & button.flag() != 0 {
                buttons.insert(button.ui());
            }
        }
        let mut modifiers = Modifiers::empty();
        for (held, modifier) in [
            (input.modifiers.shift_key(), Modifiers::SHIFT),
            (input.modifiers.control_key(), Modifiers::CONTROL),
            (input.modifiers.alt_key(), Modifiers::ALT),
            (input.modifiers.meta_key(), Modifiers::META),
        ] {
            if held {
                modifiers |= modifier;
            }
        }
        PointerState {
            time: START.elapsed().as_nanos() as u64,
            position: (input.position.x, input.position.y).into(),
            // A mouse has no pressure: half while a button is down.
            pressure: if input.buttons == 0 { 0.0 } else { 0.5 },
            buttons,
            modifiers,
            count,
            scale_factor: scale,
            ..PointerState::default()
        }
    }
}

fn mouse() -> PointerInfo {
    PointerInfo {
        pointer_id: Some(PointerId::PRIMARY),
        persistent_device_id: None,
        pointer_type: PointerType::Mouse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Rect = Rect::new(100.0, 50.0, 900.0, 650.0);

    #[test]
    fn a_press_over_a_transparent_canvas_pixel_goes_to_the_canvas() {
        assert_eq!(route(Point::new(400.0, 300.0), VIEWPORT, 0), Side::Canvas);
    }

    #[test]
    fn a_press_over_painted_ui_goes_to_the_web() {
        // A panel or a toolbar over the canvas.
        assert_eq!(route(Point::new(400.0, 300.0), VIEWPORT, 255), Side::Web);
        assert_eq!(route(Point::new(400.0, 300.0), VIEWPORT, 1), Side::Web);
        // Transparent, but outside the viewport.
        assert_eq!(route(Point::new(20.0, 300.0), VIEWPORT, 0), Side::Web);
    }

    #[test]
    fn a_drag_keeps_the_side_it_began_on() {
        let mut drag = Drag::default();
        assert_eq!(drag.side(), None);
        assert_eq!(drag.press(LEFT, Side::Canvas), Side::Canvas);
        assert_eq!(drag.side(), Some(Side::Canvas));
        // A second button pressed over the interface stays with the drag.
        assert_eq!(drag.press(RIGHT, Side::Web), Side::Canvas);
        assert_eq!(drag.release(RIGHT), Side::Canvas);
        assert_eq!(drag.side(), Some(Side::Canvas));
        assert_eq!(drag.release(LEFT), Side::Canvas);
        assert_eq!(drag.side(), None);

        assert_eq!(drag.press(LEFT, Side::Web), Side::Web);
        assert_eq!(drag.release(LEFT), Side::Web);
        assert_eq!(drag.side(), None);
    }

    const LEFT: u32 = 1;
    const RIGHT: u32 = 4;
}
