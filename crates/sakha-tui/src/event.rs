//! Terminal input events and key mapping. See spec
//! `modules/13-ui-cli-tui-web-desktop.md` "TUI Screens".

use serde::{Deserialize, Serialize};

use crate::data::RefreshSnapshot;

/// A normalized input event the TUI event loop reduces state on.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    Resize { width: u16, height: u16 },
    /// A completed data-source refresh, applied to state in one step.
    Refreshed(RefreshSnapshot),
    Quit,
}

/// A single key press, decoupled from `crossterm::event::KeyEvent` so the
/// reducer stays testable without a real terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub ctrl: bool,
}

impl KeyEvent {
    pub fn new(code: KeyCode) -> Self {
        Self { code, ctrl: false }
    }

    pub fn ctrl(code: KeyCode) -> Self {
        Self { code, ctrl: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Tab,
    Backspace,
}

/// Maps raw key events to application actions/keybindings.
#[derive(Debug, Default)]
pub struct KeyMap;

/// A high-level action a key press maps to. Kept separate from `KeyCode` so
/// the reducer matches on intent rather than raw keys, and so the keymap can
/// be remapped later without touching `TuiApp::on_event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextView,
    PrevView,
    SelectSession,
    SelectTools,
    SelectLoops,
    SelectDiff,
    SelectProvider,
    SelectMemory,
    SelectResearch,
    SelectCompression,
    MoveUp,
    MoveDown,
    Refresh,
    Noop,
}

impl KeyMap {
    pub fn new() -> Self {
        Self
    }

    /// Returns true if this key event should quit the application.
    pub fn is_quit(&self, event: &KeyEvent) -> bool {
        matches!(event.code, KeyCode::Char('q')) || (event.ctrl && matches!(event.code, KeyCode::Char('c')))
    }

    /// Resolves a raw key event to the `Action` it triggers.
    pub fn resolve(&self, event: &KeyEvent) -> Action {
        if self.is_quit(event) {
            return Action::Quit;
        }
        match event.code {
            KeyCode::Tab | KeyCode::Right => Action::NextView,
            KeyCode::Left => Action::PrevView,
            KeyCode::Char('1') => Action::SelectSession,
            KeyCode::Char('2') => Action::SelectTools,
            KeyCode::Char('3') => Action::SelectLoops,
            KeyCode::Char('4') => Action::SelectDiff,
            KeyCode::Char('5') => Action::SelectProvider,
            KeyCode::Char('6') => Action::SelectMemory,
            KeyCode::Char('7') => Action::SelectResearch,
            KeyCode::Char('8') => Action::SelectCompression,
            KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
            KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::Noop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_key_resolves_to_quit_action() {
        let keymap = KeyMap::new();
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn ctrl_c_resolves_to_quit_action() {
        let keymap = KeyMap::new();
        assert_eq!(keymap.resolve(&KeyEvent::ctrl(KeyCode::Char('c'))), Action::Quit);
    }

    #[test]
    fn digit_keys_select_views() {
        let keymap = KeyMap::new();
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Char('1'))), Action::SelectSession);
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Char('2'))), Action::SelectTools);
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Char('3'))), Action::SelectLoops);
    }

    #[test]
    fn arrow_and_vim_keys_move_selection() {
        let keymap = KeyMap::new();
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Up)), Action::MoveUp);
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Char('k'))), Action::MoveUp);
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Down)), Action::MoveDown);
        assert_eq!(keymap.resolve(&KeyEvent::new(KeyCode::Char('j'))), Action::MoveDown);
    }
}
