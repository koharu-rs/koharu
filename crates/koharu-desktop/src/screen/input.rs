//! The window's pointer, routed to the canvas where CEF painted nothing over
//! its viewport and to the web interface everywhere else, as each side's
//! own events, and its keys, routed to the side a press last landed on.

use std::{
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use koharu_canvas::{Canvas, Effect, Point, Rect};
use serde::Deserialize;
use specta::Type;
use tauri_runtime_cef::cef::{BrowserHost, ImplBrowserHost as _, MouseButtonType, MouseEvent};
use ui_events::{
    ScrollDelta,
    keyboard::{Code, Key, KeyState, KeyboardEvent, Modifiers, NamedKey},
    pointer::{
        PointerButton, PointerButtonEvent, PointerButtons, PointerEvent, PointerId, PointerInfo,
        PointerScrollEvent, PointerState, PointerType, PointerUpdate,
    },
};
use winit::{
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::{Key as WinitKey, ModifiersState, PhysicalKey},
    window::Window,
};

use super::{Main, blur, change, keys, set_cursor, with};
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
                MouseScrollDelta::LineDelta(dx, dy) => {
                    // winit's GTK 4 backend, which tauri-runtime-cef runs on
                    // Linux, passes GTK's deltas on as they are, positive
                    // scrolling down or right, against winit's sign and its
                    // X11 backend's (tauri-winit-gtk4 0.31.0-beta.3,
                    // `window/pointers.rs`).
                    let sign = if cfg!(target_os = "linux") { -1.0 } else { 1.0 };
                    Self::Wheel {
                        dx: dx * sign,
                        dy: dy * sign,
                        lines: true,
                    }
                }
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
    /// The side holding the keyboard: the one a press last landed on.
    keyboard: Side,
    /// A field of the page's has the focus, as CEF tells when it would
    /// show an on-screen keyboard.
    pub(super) field: bool,
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

    /// The modifiers held, as both sides' events carry them.
    fn modifiers(&self) -> Modifiers {
        let mut modifiers = Modifiers::empty();
        for (held, modifier) in [
            (self.modifiers.shift_key(), Modifiers::SHIFT),
            (self.modifiers.control_key(), Modifiers::CONTROL),
            (self.modifiers.alt_key(), Modifiers::ALT),
            (self.modifiers.meta_key(), Modifiers::META),
        ] {
            if held {
                modifiers |= modifier;
            }
        }
        modifiers
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
    let scale = window.scale_factor();
    match pointer {
        Pointer::Moved { x, y } => {
            let at = Point::new(x, y);
            let Some(side) = with(|main| {
                main.input.position = at;
                main.input.drag.side().unwrap_or_else(|| main.under(at))
            }) else {
                return;
            };
            hover(app, window, side);
            match side {
                Side::Web => web(scale, |host, event| {
                    host.send_mouse_move_event(Some(event), 0)
                }),
                Side::Canvas => {
                    let Some(current) = with(|main| main.pointer_state(scale, 0)) else {
                        return;
                    };
                    let update = PointerUpdate {
                        pointer: mouse(),
                        current,
                        coalesced: Vec::new(),
                        predicted: Vec::new(),
                    };
                    canvas(app, window, PointerEvent::Move(update));
                }
            }
        }
        Pointer::Pressed { x, y, button } => {
            let at = Point::new(x, y);
            let Some((side, count, first)) = with(|main| {
                main.input.position = at;
                let under = main.under(at);
                let first = main.input.drag.side().is_none();
                let side = main.input.drag.press(button.flag(), under);
                if first {
                    main.input.keyboard = side;
                }
                (side, main.input.click(button.flag(), scale), first)
            }) else {
                return;
            };
            hover(app, window, side);
            match side {
                Side::Web => web(scale, |host, event| {
                    host.send_mouse_click_event(Some(event), button.cef(), 0, i32::from(count));
                }),
                Side::Canvas => {
                    if first {
                        blur();
                    }
                    let Some(state) = with(|main| main.pointer_state(scale, count)) else {
                        return;
                    };
                    let down = PointerButtonEvent {
                        button: Some(button.ui()),
                        pointer: mouse(),
                        state,
                    };
                    canvas(app, window, PointerEvent::Down(down));
                }
            }
        }
        Pointer::Released { x, y, button } => {
            let Some((side, count)) = with(|main| {
                let input = &mut main.input;
                input.position = Point::new(x, y);
                input.buttons &= !button.flag();
                let count = input.last_press.as_ref().map_or(1, |press| press.count);
                (input.drag.release(button.flag()), count)
            }) else {
                return;
            };
            match side {
                Side::Web => web(scale, |host, event| {
                    host.send_mouse_click_event(Some(event), button.cef(), 1, i32::from(count));
                }),
                Side::Canvas => {
                    let Some(state) = with(|main| main.pointer_state(scale, count)) else {
                        return;
                    };
                    let up = PointerButtonEvent {
                        button: Some(button.ui()),
                        pointer: mouse(),
                        state,
                    };
                    canvas(app, window, PointerEvent::Up(up));
                }
            }
            // The drag over, the pointer belongs to the side it's over.
            let over = with(|main| {
                let input = &main.input;
                input
                    .drag
                    .side()
                    .is_none()
                    .then(|| main.under(input.position))
            });
            if let Some(side) = over.flatten() {
                hover(app, window, side);
            }
        }
        Pointer::Wheel { dx, dy, lines } => {
            let Some(side) = with(|main| {
                let input = &main.input;
                input
                    .drag
                    .side()
                    .unwrap_or_else(|| main.under(input.position))
            }) else {
                return;
            };
            match side {
                Side::Web => {
                    // Its sign is winit's. On Windows a line delta counts
                    // lines, winit giving a notch the system's lines per
                    // notch, three by default, and CEF scrolls 100 px for a
                    // 120 delta, so 40 a line scrolls 100/3 px a line, as
                    // Chrome does (`web_input_event_builders_win.cc`).
                    // Elsewhere a line delta stays a 120 notch.
                    //
                    // TODO: `LINE` is fitted to CEF 154's
                    // `send_mouse_wheel_event`, which takes the raw wheel
                    // delta and knows neither the lines setting nor a page a
                    // notch. CEF's API 15800 adds
                    // `CefBrowserHost::SendWheelEvent`, which takes pixels
                    // with their units and the notches (cefclient's
                    // `SetOsrWindowsWheelDelta` in `osr_input_util.h`). Once
                    // tauri-runtime-cef's CEF has it, send winit's lines at
                    // 100/3 px each, or a page for `WHEEL_PAGESCROLL`, and
                    // drop `LINE`.
                    const LINE: f32 = if cfg!(windows) { 40.0 } else { 120.0 };
                    let (dx, dy) = if lines {
                        (dx * LINE, dy * LINE)
                    } else {
                        (dx, dy)
                    };
                    web(scale, |host, event| {
                        host.send_mouse_wheel_event(Some(event), dx as i32, dy as i32);
                    });
                }
                Side::Canvas => {
                    // The canvas takes a browser's sign, where a positive
                    // delta moves the view down, against winit's, and a
                    // line as far as the interface scrolls one above:
                    // Chrome's 100/3 px on Windows, where winit counts the
                    // system's lines a notch, and a notch's 100 px
                    // elsewhere.
                    const LINE: f64 = if cfg!(windows) { 100.0 / 3.0 } else { 100.0 };
                    let line = if lines { LINE * scale } else { 1.0 };
                    let delta = ScrollDelta::PixelDelta(
                        (-f64::from(dx) * line, -f64::from(dy) * line).into(),
                    );
                    let Some(state) = with(|main| main.pointer_state(scale, 0)) else {
                        return;
                    };
                    let scroll = PointerScrollEvent {
                        pointer: mouse(),
                        delta,
                        state,
                    };
                    canvas(app, window, PointerEvent::Scroll(scroll));
                }
            }
        }
        Pointer::Left => {
            if with(|main| main.input.drag.side().is_none()) == Some(true) {
                hover(app, window, Side::Web);
                web(scale, |host, event| {
                    host.send_mouse_move_event(Some(event), 1)
                });
            }
        }
    }
}

/// Routes a key: to the canvas first while it holds the keyboard, after a
/// press on it, as Photoshop's document window takes the keys once clicked,
/// and on to the page when the canvas leaves the key. Tab passed on moves the
/// page's focus, so the page holds the keyboard after it.
pub fn key(app: &Arc<App>, event: &KeyEvent) {
    let taken = with(|main| {
        // While the canvas types, it takes the keys unless a field of the
        // page's has them, as Photoshop's type takes them back after its
        // options bar.
        let typing = !main.input.field && main.canvas.as_ref().is_some_and(Canvas::is_typing);
        if main.input.keyboard != Side::Canvas && !typing {
            return None;
        }
        let key = keyboard_event(event, main.input.modifiers());
        main.canvas.as_mut()?.key(&key).transpose()
    })
    .flatten();
    match taken {
        Some(Ok(effects)) => return carry(app, effects),
        Some(Err(error)) => {
            tracing::error!("the canvas failed a key: {error:#}");
            return;
        }
        None => {}
    }
    let held = with(|main| {
        if event.state == ElementState::Pressed
            && event.logical_key == WinitKey::Named(NamedKey::Tab)
        {
            main.input.keyboard = Side::Web;
        }
        Some((main.host()?, main.input.modifiers, main.input.field))
    })
    .flatten();
    if let Some((host, modifiers, field)) = held {
        keys::send_key(&host, event, modifiers, field);
    }
}

/// Has the app carry out what the canvas answered, once the window is free
/// again, and draws again, showing the page as its edits left it.
pub(super) fn carry(app: &Arc<App>, effects: Vec<Effect>) {
    let edited = app.handle(effects);
    change(|main| {
        if let (Some(snapshot), Some(canvas)) = (edited, &mut main.canvas) {
            canvas.show(snapshot);
        }
    });
}

/// `event` as the canvas takes it, a W3C keyboard event; winit's named keys,
/// codes, and locations are `keyboard-types`' own.
fn keyboard_event(event: &KeyEvent, modifiers: Modifiers) -> KeyboardEvent {
    KeyboardEvent {
        state: match event.state {
            ElementState::Pressed => KeyState::Down,
            ElementState::Released => KeyState::Up,
        },
        key: match &event.logical_key {
            WinitKey::Named(named) => Key::Named(*named),
            WinitKey::Character(text) => Key::Character(text.to_string()),
            _ => Key::Named(NamedKey::Unidentified),
        },
        code: match event.physical_key {
            PhysicalKey::Code(code) => code,
            PhysicalKey::Unidentified(_) => Code::Unidentified,
        },
        location: event.location,
        modifiers,
        repeat: event.repeat,
        is_composing: false,
    }
}

/// Moves the pointer to `side`: the side it leaves hears that it left, and
/// the cursor becomes the new side's.
fn hover(app: &Arc<App>, window: &dyn Window, side: Side) {
    let Some(left) = with(|main| std::mem::replace(&mut main.input.hover, side)) else {
        return;
    };
    if left == side {
        return;
    }
    match side {
        Side::Canvas => web(window.scale_factor(), |host, event| {
            host.send_mouse_move_event(Some(event), 1);
        }),
        Side::Web => {
            canvas(app, window, PointerEvent::Leave(mouse()));
            if let Some(cursor) = with(|main| main.web_cursor) {
                set_cursor(window, cursor);
            }
        }
    }
}

/// Sends CEF a mouse event at the pointer, through the browser's host,
/// once the window's borrow is over.
fn web(scale: f64, send: impl FnOnce(&BrowserHost, &MouseEvent)) {
    let sent = with(|main| Some((main.host()?, main.mouse_event(scale))));
    if let Some((host, event)) = sent.flatten() {
        send(&host, &event);
    }
}

/// Gives the canvas `event`, carries out what it answers, shows the page as
/// its edit left it in the same redraw, and shows its cursor while the
/// pointer is over it.
fn canvas(app: &Arc<App>, window: &dyn Window, event: PointerEvent) {
    let left = matches!(event, PointerEvent::Leave(_));
    let answer = with(|main| {
        let canvas = main.canvas.as_mut()?;
        Some(
            canvas
                .input(event)
                .map(|effects| (effects, canvas.cursor())),
        )
    });
    match answer.flatten() {
        Some(Ok((effects, cursor))) => {
            if !left {
                set_cursor(window, cursor);
            }
            carry(app, effects);
        }
        Some(Err(error)) => tracing::error!("the canvas failed a pointer event: {error:#}"),
        None => {}
    }
}

impl Main {
    /// The side under a pointer at `at`.
    fn under(&self, at: Point) -> Side {
        if self.canvas.is_none() {
            return Side::Web;
        }
        let alpha = self
            .painted
            .alpha(at.x.max(0.0) as u32, at.y.max(0.0) as u32);
        route(at, self.physical_viewport(), alpha)
    }

    /// A CEF mouse event at the pointer, in the view's DIPs.
    fn mouse_event(&self, scale: f64) -> MouseEvent {
        let input = &self.input;
        MouseEvent {
            x: (input.position.x / scale).round() as i32,
            y: (input.position.y / scale).round() as i32,
            modifiers: keys::flags(input.modifiers, input.buttons),
        }
    }

    /// The pointer as the canvas takes it.
    fn pointer_state(&self, scale: f64, count: u8) -> PointerState {
        let input = &self.input;
        let mut buttons = PointerButtons::default();
        for button in [Button::Left, Button::Middle, Button::Right] {
            if input.buttons & button.flag() != 0 {
                buttons.insert(button.ui());
            }
        }
        let modifiers = input.modifiers();
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
