//! TUI view modules.

pub mod loops;
pub mod session;
pub mod tools;

/// Which top-level screen is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Session,
    Tools,
    Loops,
}

impl View {
    /// Cycles forward: Session -> Tools -> Loops -> Session.
    pub fn next(self) -> Self {
        match self {
            View::Session => View::Tools,
            View::Tools => View::Loops,
            View::Loops => View::Session,
        }
    }

    /// Cycles backward: Session -> Loops -> Tools -> Session.
    pub fn prev(self) -> Self {
        match self {
            View::Session => View::Loops,
            View::Loops => View::Tools,
            View::Tools => View::Session,
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
        assert_eq!(View::Loops.next(), View::Session);
    }

    #[test]
    fn prev_is_the_inverse_of_next() {
        for view in [View::Session, View::Tools, View::Loops] {
            assert_eq!(view.next().prev(), view);
        }
    }
}
