//! The keys while typing, as Photoshop's Type tool takes them: text, line
//! breaks, deleting, moving and selecting by grapheme, word, line, and text,
//! and the clipboard. A chord typing doesn't use goes to the host.

use koharu_scene::Direction;
use ui_events::keyboard::{Code, Key, KeyState, KeyboardEvent, NamedKey};

use super::Typing;
use crate::{Effect, tools::command};

/// What a key does to a typing session.
pub(crate) enum Keyed {
    /// Typing took it, answering these.
    Taken(Vec<Effect>),
    /// Ctrl+Enter, or Enter on the number pad: keep what's typed and stop.
    Commit,
    /// Escape: drop what's typed and stop.
    Cancel,
    /// Not typing's: the host's.
    Pass,
}

impl Typing {
    /// What `event` does to the session. A release is the host's, as typing
    /// acts on presses and their repeats.
    pub fn key(&mut self, event: &KeyboardEvent) -> anyhow::Result<Keyed> {
        if event.state != KeyState::Down {
            return Ok(Keyed::Pass);
        }
        let modifiers = event.modifiers;
        let (shift, word) = (modifiers.shift(), command(modifiers));
        // AltGr, which Windows reports as Ctrl and Alt together, types.
        let chord = (word && !modifiers.alt()) || modifiers.meta();
        let vertical = self.base.direction == Direction::Vertical;
        let taken = Keyed::Taken(Vec::new());
        Ok(match &event.key {
            Key::Named(NamedKey::Escape) => Keyed::Cancel,
            Key::Named(NamedKey::Enter) if word || event.code == Code::NumpadEnter => Keyed::Commit,
            Key::Named(NamedKey::Enter) => {
                self.insert("\n")?;
                taken
            }
            Key::Named(NamedKey::Backspace) => {
                self.delete(false, word)?;
                taken
            }
            Key::Named(NamedKey::Delete) => {
                self.delete(true, word)?;
                taken
            }
            // Along a column the caret runs down; across, columns run right
            // to left.
            Key::Named(arrow @ (NamedKey::ArrowLeft | NamedKey::ArrowRight)) => {
                let right = *arrow == NamedKey::ArrowRight;
                match vertical {
                    true => self.line(!right, shift),
                    false => self.step(right, word, shift),
                }
                taken
            }
            Key::Named(arrow @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) => {
                let down = *arrow == NamedKey::ArrowDown;
                match vertical {
                    true => self.step(down, word, shift),
                    false => self.line(down, shift),
                }
                taken
            }
            Key::Named(edge @ (NamedKey::Home | NamedKey::End)) => {
                self.edge(*edge == NamedKey::End, word, shift);
                taken
            }
            Key::Character(text) if chord => match text.to_lowercase().as_str() {
                "a" => {
                    self.select_all();
                    taken
                }
                "c" => Keyed::Taken(self.copied()),
                "x" => {
                    let copied = self.copied();
                    // A caret has nothing to cut; `delete` there would take
                    // the grapheme before it.
                    if !copied.is_empty() {
                        self.delete(false, false)?;
                    }
                    Keyed::Taken(copied)
                }
                "v" => Keyed::Taken(vec![Effect::Paste]),
                // Undo and redo inside a session aren't kept; the history's
                // would change the text under it.
                "z" | "y" => taken,
                _ => Keyed::Pass,
            },
            // Alt alone opens a menu by its letter.
            Key::Character(_) if modifiers.alt() && !word => Keyed::Pass,
            Key::Character(text) => {
                self.insert(text)?;
                taken
            }
            // The input method's own presses, which compose through its
            // events, and keys that type nothing.
            Key::Named(
                NamedKey::Process
                | NamedKey::Unidentified
                | NamedKey::Dead
                | NamedKey::Compose
                | NamedKey::Tab,
            ) => taken,
            Key::Named(_) => Keyed::Pass,
        })
    }

    /// The selection for the host's clipboard; nothing when none.
    fn copied(&self) -> Vec<Effect> {
        let text = self.selected_text();
        match text.is_empty() {
            true => Vec::new(),
            false => vec![Effect::Copy(text)],
        }
    }
}
