//! Loop dashboard view: lists running/paused/completed loops with progress,
//! per spec "Loop dashboard" and `04-core-domain-model.md` `LoopSpec`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::data::{LoopState, LoopSummary};

/// Renders the running/paused/completed loop dashboard.
#[derive(Debug, Default)]
pub struct LoopsView {
    pub loops: Vec<LoopSummary>,
    pub selected: usize,
}

impl LoopsView {
    pub fn apply(&mut self, loops: Vec<LoopSummary>) {
        self.loops = loops;
        if self.loops.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.loops.len() {
            self.selected = self.loops.len() - 1;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.loops.is_empty() && self.selected + 1 < self.loops.len() {
            self.selected += 1;
        }
    }

    pub fn selected_loop(&self) -> Option<&LoopSummary> {
        self.loops.get(self.selected)
    }

    fn state_color(state: LoopState) -> Color {
        match state {
            LoopState::Created => Color::DarkGray,
            LoopState::Running => Color::Green,
            LoopState::Paused => Color::Yellow,
            LoopState::Stopped => Color::Red,
            LoopState::Completed => Color::Blue,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .loops
            .iter()
            .map(|l| {
                Line::from(vec![
                    Span::styled(format!("[{}] ", l.state.label()), Style::default().fg(Self::state_color(l.state))),
                    Span::raw(format!("{} ", l.objective)),
                    Span::styled(format!("({})", l.kind), Style::default().fg(Color::DarkGray)),
                    Span::raw(format!(" — {}/{} iterations", l.iterations_completed, l.max_iterations)),
                ])
            })
            .map(ListItem::new)
            .collect();
        let block = Block::default().title("Loops").borders(Borders::ALL);
        let mut state = ListState::default();
        if !self.loops.is_empty() {
            state.select(Some(self.selected));
        }
        let list = List::new(items).block(block).highlight_style(Style::default().fg(Color::Yellow));
        frame.render_stateful_widget(list, area, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_core::LoopId;

    fn summary(objective: &str, state: LoopState) -> LoopSummary {
        LoopSummary {
            id: LoopId::new(),
            objective: objective.into(),
            kind: "agent".into(),
            state,
            iterations_completed: 1,
            max_iterations: 10,
        }
    }

    #[test]
    fn apply_clamps_selection() {
        let mut view = LoopsView::default();
        view.apply(vec![summary("a", LoopState::Running), summary("b", LoopState::Paused)]);
        view.selected = 1;
        view.apply(vec![summary("a", LoopState::Running)]);
        assert_eq!(view.selected, 0);
    }

    #[test]
    fn selected_loop_tracks_navigation() {
        let mut view = LoopsView::default();
        view.apply(vec![summary("a", LoopState::Running), summary("b", LoopState::Paused)]);
        view.move_down();
        assert_eq!(view.selected_loop().unwrap().objective, "b");
        view.move_up();
        assert_eq!(view.selected_loop().unwrap().objective, "a");
    }
}
