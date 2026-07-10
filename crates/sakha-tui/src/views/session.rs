//! Session timeline / chat view: renders turns (input/output) plus the
//! session's accumulated cost, per spec "Session timeline" and
//! `04-core-domain-model.md` `Turn`/`Session`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use sakha_memory::{SessionRecord, TurnRecord};

use crate::data::CostSnapshot;

/// Renders the session/chat timeline view: session status header, a
/// scrollable list of turns, and a cost/token footer.
#[derive(Debug, Default)]
pub struct SessionView {
    pub session: Option<SessionRecord>,
    pub turns: Vec<TurnRecord>,
    pub cost: CostSnapshot,
    pub selected: usize,
}

impl SessionView {
    /// Applies a fresh data snapshot, clamping the current selection so it
    /// stays in bounds as the turn list grows or shrinks.
    pub fn apply(&mut self, session: Option<SessionRecord>, turns: Vec<TurnRecord>, cost: CostSnapshot) {
        self.session = session;
        self.turns = turns;
        self.cost = cost;
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        if self.turns.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.turns.len() {
            self.selected = self.turns.len() - 1;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.turns.is_empty() && self.selected + 1 < self.turns.len() {
            self.selected += 1;
        }
    }

    fn header_text(&self) -> String {
        match &self.session {
            Some(s) => format!("Session {} — {:?}", s.id, s.status),
            None => "No active session".to_string(),
        }
    }

    fn footer_text(&self) -> String {
        format!(
            "cost: ${:.4}  in: {}  out: {}  requests: {}",
            self.cost.cost_usd(),
            self.cost.input_tokens,
            self.cost.output_tokens,
            self.cost.request_count
        )
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        frame.render_widget(Paragraph::new(self.header_text()), chunks[0]);

        let items: Vec<ListItem> = self
            .turns
            .iter()
            .map(|t| {
                let out = t.output_text.as_deref().unwrap_or("(pending)");
                ListItem::new(Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Cyan)),
                    Span::raw(t.input_text.clone()),
                    Span::raw("  ->  "),
                    Span::raw(out.to_string()),
                ]))
            })
            .collect();
        let block = Block::default().title("Session Timeline").borders(Borders::ALL);
        let mut state = ListState::default();
        if !self.turns.is_empty() {
            state.select(Some(self.selected));
        }
        let list = List::new(items).block(block).highlight_style(Style::default().fg(Color::Yellow));
        frame.render_stateful_widget(list, chunks[1], &mut state);

        frame.render_widget(Paragraph::new(self.footer_text()), chunks[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_core::{SessionId, TurnId, WorkspaceId};
    use sakha_memory::SessionStatus;

    fn turn(input: &str) -> TurnRecord {
        TurnRecord {
            id: TurnId::new(),
            session_id: SessionId::new(),
            input_text: input.into(),
            output_text: Some("ok".into()),
            created_at: sakha_core::time::now_utc(),
        }
    }

    #[test]
    fn apply_clamps_selection_when_turns_shrink() {
        let mut view = SessionView::default();
        view.apply(None, vec![turn("a"), turn("b"), turn("c")], CostSnapshot::default());
        view.selected = 2;
        view.apply(None, vec![turn("a")], CostSnapshot::default());
        assert_eq!(view.selected, 0);
    }

    #[test]
    fn move_down_stops_at_last_turn() {
        let mut view = SessionView::default();
        view.apply(None, vec![turn("a"), turn("b")], CostSnapshot::default());
        view.move_down();
        assert_eq!(view.selected, 1);
        view.move_down();
        assert_eq!(view.selected, 1, "should not move past the last item");
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut view = SessionView::default();
        view.apply(None, vec![turn("a")], CostSnapshot::default());
        view.move_up();
        assert_eq!(view.selected, 0);
    }

    #[test]
    fn header_reflects_session_status() {
        let mut view = SessionView::default();
        let session = SessionRecord {
            id: SessionId::new(),
            workspace_id: WorkspaceId::new(),
            goal_id: None,
            status: SessionStatus::Active,
            provider_profile: None,
            created_at: sakha_core::time::now_utc(),
            updated_at: sakha_core::time::now_utc(),
        };
        view.apply(Some(session), vec![], CostSnapshot::default());
        assert!(view.header_text().contains("Active"));
    }
}
