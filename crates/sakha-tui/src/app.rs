//! `TuiApp`: top-level application state and event-reduction loop.

use crate::data::RefreshSnapshot;
use crate::event::{Action, AppEvent, KeyMap};
use crate::views::loops::LoopsView;
use crate::views::session::SessionView;
use crate::views::tools::ToolsView;
use crate::views::View;

/// The full TUI application state.
pub struct TuiApp {
    pub active_view: View,
    pub session_view: SessionView,
    pub tools_view: ToolsView,
    pub loops_view: LoopsView,
    pub keymap: KeyMap,
    pub should_quit: bool,
    /// Set by `Action::Refresh` (or the periodic tick, at the driver's
    /// discretion) to signal the terminal-loop driver that it should call
    /// `DataSource::refresh` and feed the result back in as
    /// `AppEvent::Refreshed`. The reducer itself never awaits I/O.
    pub refresh_requested: bool,
    /// Last known terminal size, updated on `AppEvent::Resize`. Available
    /// for views that want to adapt layout beyond what `ratatui`'s
    /// constraint solver already handles per-frame.
    pub terminal_size: (u16, u16),
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            active_view: View::Session,
            session_view: SessionView::default(),
            tools_view: ToolsView::default(),
            loops_view: LoopsView::default(),
            keymap: KeyMap::new(),
            should_quit: false,
            refresh_requested: false,
            terminal_size: (0, 0),
        }
    }

    /// Reduces one `AppEvent` into updated state. Pure function of
    /// (state, event) -> state (aside from the `refresh_requested` I/O
    /// signal flag), so it's testable without a real terminal.
    pub fn on_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_action(self.keymap.resolve(&key)),
            AppEvent::Quit => self.should_quit = true,
            AppEvent::Tick => {}
            AppEvent::Resize { width, height } => self.terminal_size = (width, height),
            AppEvent::Refreshed(snapshot) => self.apply_snapshot(snapshot),
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::NextView => self.active_view = self.active_view.next(),
            Action::PrevView => self.active_view = self.active_view.prev(),
            Action::SelectSession => self.active_view = View::Session,
            Action::SelectTools => self.active_view = View::Tools,
            Action::SelectLoops => self.active_view = View::Loops,
            Action::SelectDiff => self.active_view = View::Diff,
            Action::SelectProvider => self.active_view = View::Provider,
            Action::SelectMemory => self.active_view = View::Memory,
            Action::SelectResearch => self.active_view = View::Research,
            Action::SelectCompression => self.active_view = View::Compression,
            Action::MoveUp => self.move_selection_up(),
            Action::MoveDown => self.move_selection_down(),
            Action::Refresh => self.refresh_requested = true,
            Action::Noop => {}
        }
    }

    fn move_selection_up(&mut self) {
        match self.active_view {
            View::Session => self.session_view.move_up(),
            View::Tools => self.tools_view.move_up(),
            View::Loops => self.loops_view.move_up(),
            // The remaining screens (Diff/Provider/Memory/Research/
            // Compression) don't yet have a selectable list of their own;
            // navigation is a no-op until their data-backed views land.
            View::Diff | View::Provider | View::Memory | View::Research | View::Compression => {}
        }
    }

    fn move_selection_down(&mut self) {
        match self.active_view {
            View::Session => self.session_view.move_down(),
            View::Tools => self.tools_view.move_down(),
            View::Loops => self.loops_view.move_down(),
            View::Diff | View::Provider | View::Memory | View::Research | View::Compression => {}
        }
    }

    /// Applies a full data refresh to all views at once, so a single
    /// `DataSource::refresh` call keeps every screen consistent.
    fn apply_snapshot(&mut self, snapshot: RefreshSnapshot) {
        self.session_view.apply(snapshot.session, snapshot.turns, snapshot.cost);
        self.tools_view.apply(snapshot.tool_calls);
        self.loops_view.apply(snapshot.loops);
        self.refresh_requested = false;
    }
}

impl Default for TuiApp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{CostSnapshot, LoopState, LoopSummary, ToolCallStatus, ToolCallSummary};
    use crate::event::{KeyCode, KeyEvent};
    use sakha_core::{LoopId, ToolCallId};

    #[test]
    fn quit_key_sets_should_quit() {
        let mut app = TuiApp::new();
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('q'))));
        assert!(app.should_quit);
    }

    #[test]
    fn tick_event_does_not_quit() {
        let mut app = TuiApp::new();
        app.on_event(AppEvent::Tick);
        assert!(!app.should_quit);
    }

    /// `AppEvent::Quit` is a direct (non-key-driven) quit signal, reserved
    /// for programmatic shutdown (e.g. a supervising process or a future
    /// "quit" IPC command) as opposed to `Action::Quit`, which is derived
    /// from a key press via the keymap.
    #[test]
    fn quit_event_sets_should_quit_directly() {
        let mut app = TuiApp::new();
        app.on_event(AppEvent::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn resize_event_updates_terminal_size() {
        let mut app = TuiApp::new();
        assert_eq!(app.terminal_size, (0, 0));
        app.on_event(AppEvent::Resize { width: 120, height: 40 });
        assert_eq!(app.terminal_size, (120, 40));
    }

    #[test]
    fn tab_key_cycles_active_view() {
        let mut app = TuiApp::new();
        assert_eq!(app.active_view, View::Session);
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Tab)));
        assert_eq!(app.active_view, View::Tools);
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Tab)));
        assert_eq!(app.active_view, View::Loops);
        // Tab continues through the remaining screens (per spec "TUI
        // Screens") before wrapping back to Session.
        for expected in [View::Diff, View::Provider, View::Memory, View::Research, View::Compression, View::Session] {
            app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Tab)));
            assert_eq!(app.active_view, expected);
        }
    }

    #[test]
    fn digit_keys_jump_directly_to_a_view() {
        let mut app = TuiApp::new();
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('3'))));
        assert_eq!(app.active_view, View::Loops);
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('2'))));
        assert_eq!(app.active_view, View::Tools);
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('1'))));
        assert_eq!(app.active_view, View::Session);
    }

    #[test]
    fn refresh_key_sets_refresh_requested_flag() {
        let mut app = TuiApp::new();
        assert!(!app.refresh_requested);
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('r'))));
        assert!(app.refresh_requested);
    }

    #[test]
    fn refreshed_event_populates_all_views_and_clears_flag() {
        let mut app = TuiApp::new();
        app.refresh_requested = true;

        let snapshot = RefreshSnapshot {
            session: None,
            turns: vec![],
            tool_calls: vec![ToolCallSummary {
                id: ToolCallId::new(),
                tool_name: "file.read".into(),
                status: ToolCallStatus::Succeeded,
                risk_level: "low".into(),
                summary: "read a file".into(),
            }],
            loops: vec![LoopSummary {
                id: LoopId::new(),
                objective: "ship it".into(),
                kind: "agent".into(),
                state: LoopState::Running,
                iterations_completed: 2,
                max_iterations: 10,
            }],
            cost: CostSnapshot { cost_micros: 1000, input_tokens: 10, output_tokens: 5, request_count: 1 },
        };
        app.on_event(AppEvent::Refreshed(snapshot));

        assert!(!app.refresh_requested);
        assert_eq!(app.tools_view.calls.len(), 1);
        assert_eq!(app.loops_view.loops.len(), 1);
        assert_eq!(app.session_view.cost.cost_micros, 1000);
    }

    #[test]
    fn move_down_only_affects_the_active_view() {
        let mut app = TuiApp::new();
        app.tools_view.apply(vec![
            ToolCallSummary {
                id: ToolCallId::new(),
                tool_name: "a".into(),
                status: ToolCallStatus::Pending,
                risk_level: "low".into(),
                summary: String::new(),
            },
            ToolCallSummary {
                id: ToolCallId::new(),
                tool_name: "b".into(),
                status: ToolCallStatus::Pending,
                risk_level: "low".into(),
                summary: String::new(),
            },
        ]);
        // Active view is Session, so Down should not move the tools selection.
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Down)));
        assert_eq!(app.tools_view.selected, 0);

        app.active_view = View::Tools;
        app.on_event(AppEvent::Key(KeyEvent::new(KeyCode::Down)));
        assert_eq!(app.tools_view.selected, 1);
    }

    #[test]
    fn ctrl_c_quits_regardless_of_active_view() {
        let mut app = TuiApp::new();
        app.active_view = View::Loops;
        app.on_event(AppEvent::Key(KeyEvent::ctrl(KeyCode::Char('c'))));
        assert!(app.should_quit);
    }
}
