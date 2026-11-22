//! Snapshot-based undo/redo.

use std::collections::VecDeque;

/// Bounded undo/redo stacks of whole-state snapshots.
///
/// Pass [`History::record`] a snapshot taken *before* the state changed. Mind maps are
/// small, so cloning the whole map is cheap and far simpler than recording an inverse for
/// every operation.
#[derive(Debug, Clone, PartialEq)]
pub struct History<T> {
    undo: VecDeque<T>,
    redo: Vec<T>,
    limit: usize,
}

impl<T> History<T> {
    /// Create an empty history that keeps at most `limit` undo steps.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::with_capacity(limit),
            redo: Vec::new(),
            limit,
        }
    }

    /// Save `state` as an undo point and discard the redo stack.
    ///
    /// Takes ownership so the caller decides when (and whether) to clone.
    /// When the limit is reached the oldest snapshot is dropped (O(1) with a `VecDeque`).
    pub fn record(&mut self, state: T) {
        if self.limit == 0 {
            return;
        }
        if self.undo.len() == self.limit {
            self.undo.pop_front();
        }
        self.undo.push_back(state);
        self.redo.clear();
    }

    /// Restore the most recent undo point into `state`.
    ///
    /// # Returns
    ///
    /// `true` if something was undone.
    pub fn undo(&mut self, state: &mut T) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        // `mem::replace` swaps in the old value and hands back the current one without
        // cloning.
        self.redo.push(std::mem::replace(state, previous));
        true
    }

    /// Re-apply the most recently undone change into `state`.
    ///
    /// # Returns
    ///
    /// `true` if something was redone.
    pub fn redo(&mut self, state: &mut T) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push_back(std::mem::replace(state, next));
        true
    }

    /// Whether [`History::undo`] would do anything.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether [`History::redo`] would do anything.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_restore_states() {
        let mut history = History::new(10);
        let mut state = 1;
        history.record(state);
        state = 2;

        assert!(history.undo(&mut state));
        assert_eq!(state, 1);
        assert!(history.redo(&mut state));
        assert_eq!(state, 2);
        assert!(!history.can_redo());
    }

    #[test]
    fn empty_history_does_nothing() {
        let mut history: History<i32> = History::new(10);
        let mut state = 5;

        assert!(!history.undo(&mut state));
        assert!(!history.redo(&mut state));
        assert_eq!(state, 5);
    }

    #[test]
    fn record_clears_redo() {
        let mut history = History::new(10);
        let mut state = 1;
        history.record(state);
        state = 2;
        history.undo(&mut state);

        history.record(state);

        assert!(!history.can_redo());
        assert!(history.can_undo());
    }

    #[test]
    fn limit_drops_oldest_snapshot() {
        let mut history = History::new(2);
        let mut state = 0;
        for next in 1..=3 {
            history.record(state);
            state = next;
        }

        assert!(history.undo(&mut state));
        assert!(history.undo(&mut state));
        assert!(!history.undo(&mut state));
        assert_eq!(state, 1);
    }

    #[test]
    fn zero_limit_records_nothing() {
        let mut history = History::new(0);

        history.record(1);

        assert!(!history.can_undo());
    }
}
