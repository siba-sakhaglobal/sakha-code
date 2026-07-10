//! TUI view modules.

pub mod loops;
pub mod session;
pub mod tools;

/// Which top-level screen is currently active. Names and ordering mirror
/// spec `modules/13-ui-cli-tui-web-desktop.md` "TUI Screens" ("Chat" is
/// covered by `Session`, which renders the same turn timeline; "Tool
/// approval" is covered by `Tools`; "Loop dashboard" is covered by `Loops`).
/// The remaining six screens (`Diff`, `Provider`/cost, `Memory`, `Research`,
/// `Compression`) are declared here so the app-level view cycle and keymap
/// have a stable, complete surface to route to; their `render`/data-backed
/// bodies land alongside the crates that own their data (`sakha-tools`
/// diffs, `sakha-observability` cost, `sakha-research` evidence,
/// `sakha-compression` stats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Session,
    Tools,
    Loops,
    Diff,
    Provider,
    Memory,
    Research,
    Compression,
}

/// Every `View` variant, in the fixed order `next`/`prev` cycle through.
const ALL_VIEWS: [View; 8] =
    [View::Session, View::Tools, View::Loops, View::Diff, View::Provider, View::Memory, View::Research, View::Compression];

impl View {
    fn index(self) -> usize {
        ALL_VIEWS.iter().position(|v| *v == self).expect("View::index: every variant is listed in ALL_VIEWS")
    }

    /// Cycles forward through every screen, wrapping after the last.
    pub fn next(self) -> Self {
        ALL_VIEWS[(self.index() + 1) % ALL_VIEWS.len()]
    }

    /// Cycles backward through every screen, wrapping before the first.
    pub fn prev(self) -> Self {
        ALL_VIEWS[(self.index() + ALL_VIEWS.len() - 1) % ALL_VIEWS.len()]
    }

    /// A short, human-readable label for tab bars/status lines.
    pub fn label(self) -> &'static str {
        match self {
            View::Session => "Session",
            View::Tools => "Tools",
            View::Loops => "Loops",
            View::Diff => "Diff",
            View::Provider => "Provider",
            View::Memory => "Memory",
            View::Research => "Research",
            View::Compression => "Compression",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_cycles_through_all_views_and_back() {
        assert_eq!(View::Session.next(), View::Tools);
        assert_eq!(View::Tools.next(), View::Loops);
        assert_eq!(View::Loops.next(), View::Diff);
        assert_eq!(View::Compression.next(), View::Session);
    }

    #[test]
    fn prev_is_the_inverse_of_next() {
        for view in ALL_VIEWS {
            assert_eq!(view.next().prev(), view);
        }
    }

    #[test]
    fn every_view_has_a_distinct_label() {
        let labels: std::collections::HashSet<&str> = ALL_VIEWS.iter().map(|v| v.label()).collect();
        assert_eq!(labels.len(), ALL_VIEWS.len());
    }
}
