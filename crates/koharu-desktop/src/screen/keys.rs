//! The keyboard and input method, as CEF takes them: key events identified by
//! Windows virtual-key codes, text as character events, and the input
//! method's composition through CEF's IME calls.

use tauri_runtime_cef::cef::{
    BrowserHost, CefString, CompositionUnderline, CompositionUnderlineStyle, ImplBrowserHost as _,
    KeyEvent as CefKeyEvent, KeyEventType, Range, sys::cef_event_flags_t,
};
use winit::{
    event::{ElementState, Ime, KeyEvent},
    keyboard::{Key, KeyCode, ModifiersState, PhysicalKey},
};

// `c_uint` everywhere but Windows, where bindgen makes it a `c_int`.
#[allow(clippy::unnecessary_cast)]
const SHIFT_DOWN: u32 = cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32;
#[allow(clippy::unnecessary_cast)]
const CONTROL_DOWN: u32 = cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32;
#[allow(clippy::unnecessary_cast)]
const ALT_DOWN: u32 = cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32;
#[allow(clippy::unnecessary_cast)]
const COMMAND_DOWN: u32 = cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0 as u32;
#[allow(clippy::unnecessary_cast)]
pub const LEFT_BUTTON: u32 = cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON.0 as u32;
#[allow(clippy::unnecessary_cast)]
pub const MIDDLE_BUTTON: u32 = cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0 as u32;
#[allow(clippy::unnecessary_cast)]
pub const RIGHT_BUTTON: u32 = cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON.0 as u32;

/// CEF's event flags for the modifiers and mouse buttons held.
pub fn flags(modifiers: ModifiersState, buttons: u32) -> u32 {
    let mut flags = buttons;
    for (held, flag) in [
        (modifiers.shift_key(), SHIFT_DOWN),
        (modifiers.control_key(), CONTROL_DOWN),
        (modifiers.alt_key(), ALT_DOWN),
        (modifiers.meta_key(), COMMAND_DOWN),
    ] {
        if held {
            flags |= flag;
        }
    }
    flags
}

/// A key press or release, and the text a press types.
pub fn send_key(host: &BrowserHost, event: &KeyEvent, modifiers: ModifiersState) {
    let flags = flags(modifiers, 0);
    let pressed = event.state == ElementState::Pressed;
    host.send_key_event(Some(&CefKeyEvent {
        type_: if pressed {
            KeyEventType::RAWKEYDOWN
        } else {
            KeyEventType::KEYUP
        },
        modifiers: flags,
        windows_key_code: windows_key_code(event),
        // Windows sends Alt's chords as system keys, which aren't text.
        is_system_key: i32::from(modifiers.alt_key() && !modifiers.control_key()),
        ..Default::default()
    }));
    // A shortcut isn't text; AltGr, which Windows reports as Control and
    // Alt together, types it.
    let shortcut = (modifiers.control_key() && !modifiers.alt_key()) || modifiers.meta_key();
    if pressed
        && !shortcut
        && let Some(text) = &event.text
    {
        for unit in text.encode_utf16() {
            host.send_key_event(Some(&CefKeyEvent {
                type_: KeyEventType::CHAR,
                modifiers: flags,
                windows_key_code: i32::from(unit),
                character: unit,
                unmodified_character: unit,
                ..Default::default()
            }));
        }
    }
}

/// The input method's composition and what it commits. A commit replaces
/// the composition, which character events would leave in place.
pub fn send_ime(host: &BrowserHost, ime: &Ime) {
    // No range: the composition replaces the selection.
    let none = Range {
        from: u32::MAX,
        to: u32::MAX,
    };
    match ime {
        Ime::Preedit(text, _) if text.is_empty() => host.ime_cancel_composition(),
        Ime::Preedit(text, cursor) => {
            let utf16 = |byte: usize| text[..byte].encode_utf16().count() as u32;
            let length = utf16(text.len());
            let selection = cursor.map_or(
                Range {
                    from: length,
                    to: length,
                },
                |(start, end)| Range {
                    from: utf16(start),
                    to: utf16(end),
                },
            );
            let underline = CompositionUnderline {
                range: Range {
                    from: 0,
                    to: length,
                },
                color: 0xff00_0000,
                style: CompositionUnderlineStyle::SOLID,
                ..Default::default()
            };
            host.ime_set_composition(
                Some(&CefString::from(text.as_str())),
                Some(&[underline]),
                Some(&none),
                Some(&selection),
            );
        }
        Ime::Commit(text) => {
            host.ime_commit_text(Some(&CefString::from(text.as_str())), Some(&none), 0);
        }
        _ => {}
    }
}

/// The Windows virtual-key code Chromium identifies a key by, which it also
/// derives `KeyboardEvent.code` from. A letter is the layout's, as Windows
/// reports it; every other key is its position's on a US layout.
fn windows_key_code(event: &KeyEvent) -> i32 {
    if let Key::Character(text) = &event.logical_key
        && let Some(letter) = text.chars().next().filter(char::is_ascii_alphabetic)
    {
        return letter.to_ascii_uppercase() as i32;
    }
    let PhysicalKey::Code(code) = event.physical_key else {
        return 0;
    };
    match code {
        KeyCode::Backspace => 0x08,
        KeyCode::Tab => 0x09,
        KeyCode::Enter | KeyCode::NumpadEnter => 0x0d,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => 0x10,
        KeyCode::ControlLeft | KeyCode::ControlRight => 0x11,
        KeyCode::AltLeft | KeyCode::AltRight => 0x12,
        KeyCode::Pause => 0x13,
        KeyCode::CapsLock => 0x14,
        KeyCode::KanaMode | KeyCode::Lang1 => 0x15,
        KeyCode::Lang2 => 0x19,
        KeyCode::Convert => 0x1c,
        KeyCode::NonConvert => 0x1d,
        KeyCode::Escape => 0x1b,
        KeyCode::Space => 0x20,
        KeyCode::PageUp => 0x21,
        KeyCode::PageDown => 0x22,
        KeyCode::End => 0x23,
        KeyCode::Home => 0x24,
        KeyCode::ArrowLeft => 0x25,
        KeyCode::ArrowUp => 0x26,
        KeyCode::ArrowRight => 0x27,
        KeyCode::ArrowDown => 0x28,
        KeyCode::PrintScreen => 0x2c,
        KeyCode::Insert => 0x2d,
        KeyCode::Delete => 0x2e,
        KeyCode::Help => 0x2f,
        KeyCode::Digit0 => 0x30,
        KeyCode::Digit1 => 0x31,
        KeyCode::Digit2 => 0x32,
        KeyCode::Digit3 => 0x33,
        KeyCode::Digit4 => 0x34,
        KeyCode::Digit5 => 0x35,
        KeyCode::Digit6 => 0x36,
        KeyCode::Digit7 => 0x37,
        KeyCode::Digit8 => 0x38,
        KeyCode::Digit9 => 0x39,
        KeyCode::KeyA => 0x41,
        KeyCode::KeyB => 0x42,
        KeyCode::KeyC => 0x43,
        KeyCode::KeyD => 0x44,
        KeyCode::KeyE => 0x45,
        KeyCode::KeyF => 0x46,
        KeyCode::KeyG => 0x47,
        KeyCode::KeyH => 0x48,
        KeyCode::KeyI => 0x49,
        KeyCode::KeyJ => 0x4a,
        KeyCode::KeyK => 0x4b,
        KeyCode::KeyL => 0x4c,
        KeyCode::KeyM => 0x4d,
        KeyCode::KeyN => 0x4e,
        KeyCode::KeyO => 0x4f,
        KeyCode::KeyP => 0x50,
        KeyCode::KeyQ => 0x51,
        KeyCode::KeyR => 0x52,
        KeyCode::KeyS => 0x53,
        KeyCode::KeyT => 0x54,
        KeyCode::KeyU => 0x55,
        KeyCode::KeyV => 0x56,
        KeyCode::KeyW => 0x57,
        KeyCode::KeyX => 0x58,
        KeyCode::KeyY => 0x59,
        KeyCode::KeyZ => 0x5a,
        KeyCode::MetaLeft => 0x5b,
        KeyCode::MetaRight => 0x5c,
        KeyCode::ContextMenu => 0x5d,
        KeyCode::Numpad0 => 0x60,
        KeyCode::Numpad1 => 0x61,
        KeyCode::Numpad2 => 0x62,
        KeyCode::Numpad3 => 0x63,
        KeyCode::Numpad4 => 0x64,
        KeyCode::Numpad5 => 0x65,
        KeyCode::Numpad6 => 0x66,
        KeyCode::Numpad7 => 0x67,
        KeyCode::Numpad8 => 0x68,
        KeyCode::Numpad9 => 0x69,
        KeyCode::NumpadMultiply => 0x6a,
        KeyCode::NumpadAdd => 0x6b,
        KeyCode::NumpadSubtract => 0x6d,
        KeyCode::NumpadDecimal => 0x6e,
        KeyCode::NumpadDivide => 0x6f,
        KeyCode::F1 => 0x70,
        KeyCode::F2 => 0x71,
        KeyCode::F3 => 0x72,
        KeyCode::F4 => 0x73,
        KeyCode::F5 => 0x74,
        KeyCode::F6 => 0x75,
        KeyCode::F7 => 0x76,
        KeyCode::F8 => 0x77,
        KeyCode::F9 => 0x78,
        KeyCode::F10 => 0x79,
        KeyCode::F11 => 0x7a,
        KeyCode::F12 => 0x7b,
        KeyCode::NumLock => 0x90,
        KeyCode::ScrollLock => 0x91,
        KeyCode::Semicolon => 0xba,
        KeyCode::Equal => 0xbb,
        KeyCode::Comma => 0xbc,
        KeyCode::Minus => 0xbd,
        KeyCode::Period => 0xbe,
        KeyCode::Slash => 0xbf,
        KeyCode::Backquote => 0xc0,
        KeyCode::IntlRo => 0xc1,
        KeyCode::BracketLeft => 0xdb,
        KeyCode::Backslash | KeyCode::IntlYen => 0xdc,
        KeyCode::BracketRight => 0xdd,
        KeyCode::Quote => 0xde,
        KeyCode::IntlBackslash => 0xe2,
        _ => 0,
    }
}
