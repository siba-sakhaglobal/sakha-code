//! Tool approval / audit view: lists recent tool calls with status and risk
//! level, per spec "Tool approval" and `04-core-domain-model.md` `ToolCall`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::data::{ToolCallStatus, ToolCallSummary};

/// Renders pending and recent tool calls for approval/audit.
#[derive(Debug, Default)]
pub struct ToolsView {
    pub calls: Vec<ToolCallSummary>,
    pub selected: usize,
}

impl ToolsView {
    pub fn apply(&mut self, calls: Vec<ToolCallSummary>) {
        self.calls = calls;
        if self.calls.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.calls.len() {
            self.selected = self.calls.len() - 1;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.calls.is_empty() && self.selected + 1 < self.calls.len() {
            self.selected += 1;
        }
    }

    /// The currently highlighted tool call, if any.
    pub fn selected_call(&self) -> Option<&ToolCallSummary> {
        self.calls.get(self.selected)
    }

    fn status_color(status: ToolCallStatus) -> Color {
        match status {
            ToolCallStatus::Pending => Color::Yellow,
            ToolCallStatus::Running => Color::Cyan,
            ToolCallStatus::Succeeded => Color::Green,
            ToolCallStatus::Failed | ToolCallStatus::TimedOut => Color::Red,
            ToolCallStatus::Denied => Color::Magenta,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .calls
            .iter()
            .map(|c| {
                Line::from(vec![
                    Span::styled(format!("[{}] ", c.status.label()), Style::default().fg(Self::status_color(c.status))),
                    Span::raw(format!("{} ", c.tool_name)),
                    Span::styled(format!("({})", c.risk_level), Style::default().fg(Color::DarkGray)),
                    Span::raw(format!(" — {}", c.summary)),
                ])
            })
            .map(ListItem::new)
            .collect();
        let block = Block::default().title("Tools").borders(Borders::ALL);
        let mut state = ListState::default();
        if !self.calls.is_empty() {
            state.select(Some(self.selected));
        }
        let list = List::new(items).block(block).highlight_style(Style::default().fg(Color::Yellow));
        frame.render_stateful_widget(list, area, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_core::ToolCallId;

    fn call(name: &str, status: ToolCallStatus) -> ToolCallSummary {
        ToolCallSummary {
            id: ToolCallId::new(),
            tool_name: name.into(),
            status,
            risk_level: "low".into(),
            summary: "did a thing".into(),
        }
    }

    #[test]
    fn apply_clamps_selection() {
        let mut view = ToolsView::default();
        view.apply(vec![call("a", ToolCallStatus::Succeeded), call("b", ToolCallStatus::Succeeded)]);
        view.selected = 1;
        view.apply(vec![call("a", ToolCallStatus::Succeeded)]);
        assert_eq!(view.selected, 0);
    }

    #[test]
    fn selected_call_returns_highlighted_entry() {
        let mut view = ToolsView::default();
        view.apply(vec![call("a", ToolCallStatus::Pending), call("b", ToolCallStatus::Running)]);
        view.move_down();
        assert_eq!(view.selected_call().unwrap().tool_name, "b");
    }

    #[test]
    fn navigation_does_not_go_out_of_bounds() {
        let mut view = ToolsView::default();
        view.apply(vec![call("a", ToolCallStatus::Pending)]);
        view.move_up();
        assert_eq!(view.selected, 0);
        view.move_down();
        assert_eq!(view.selected, 0);
    }
}
