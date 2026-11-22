//! Snapshot-based undo/redo with a memory budget.

use std::collections::VecDeque;

/// Approximate heap memory used by a value, so the history can keep its total in check.
pub trait HeapSize {
    /// Bytes this value holds on the heap (plus its own size), approximately.
    fn heap_size(&self) -> usize;
}

/// How much history to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    /// Never keep more undo steps than this.
    pub max_steps: usize,
    /// Drop the oldest steps once the undo snapshots together exceed this many bytes...
    pub max_bytes: usize,
    /// ...but always keep at least this many steps, however large the state.
    pub min_steps: usize,
}

/// Bounded undo/redo stacks of whole-state snapshots.
///
/// Pass [`History::record`] a snapshot taken *before* the state changed. Cloning the
/// whole state is far simpler than recording an inverse for every operation; the memory
/// budget in [`HistoryLimits`] keeps that affordable for very large states.
#[derive(Debug, Clone, PartialEq)]
pub struct History<T> {
    /// Snapshots with their sizes, oldest first.
    undo: VecDeque<(T, usize)>,
    redo: Vec<(T, usize)>,
    /// Sum of the sizes in `undo`.
    undo_bytes: usize,
    limits: HistoryLimits,
}

impl<T: HeapSize> History<T> {
    /// Create an empty history.
    #[must_use]
    pub fn new(limits: HistoryLimits) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            undo_bytes: 0,
            limits,
        }
    }

    /// Save `state` as an undo point and discard the redo stack, then drop the oldest
    /// snapshots while there are too many or they use too much memory.
    ///
    /// Takes ownership so the caller decides when (and whether) to clone. Dropping from
    /// the front is O(1) with a `VecDeque`.
    pub fn record(&mut self, state: T) {
        if self.limits.max_steps == 0 {
            return;
        }
        let size = state.heap_size();
        self.undo_bytes += size;
        self.undo.push_back((state, size));
        self.redo.clear();
        while self.undo.len() > self.limits.max_steps
            || (self.undo_bytes > self.limits.max_bytes && self.undo.len() > self.limits.min_steps)
        {
            let Some((_, dropped)) = self.undo.pop_front() else {
                break;
            };
            self.undo_bytes -= dropped;
        }
    }

    /// Restore the most recent undo point into `state`.
    ///
    /// # Returns
    ///
    /// `true` if something was undone.
    pub fn undo(&mut self, state: &mut T) -> bool {
        let Some((previous, size)) = self.undo.pop_back() else {
            return false;
        };
        self.undo_bytes -= size;
        // `mem::replace` swaps in the old value and hands back the current one without
        // cloning.
        let current = std::mem::replace(state, previous);
        let current_size = current.heap_size();
        self.redo.push((current, current_size));
        true
    }

    /// Re-apply the most recently undone change into `state`.
    ///
    /// # Returns
    ///
    /// `true` if something was redone.
    pub fn redo(&mut self, state: &mut T) -> bool {
        let Some((next, _)) = self.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(state, next);
        let current_size = current.heap_size();
        self.undo_bytes += current_size;
        self.undo.push_back((current, current_size));
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

    /// Number of undo steps currently kept.
    #[must_use]
    pub fn undo_steps(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test state that pretends to hold `.0` bytes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Sized(usize);

    impl HeapSize for Sized {
        fn heap_size(&self) -> usize {
            self.0
        }
    }

    const ROOMY: HistoryLimits = HistoryLimits {
        max_steps: 10,
        max_bytes: usize::MAX,
        min_steps: 1,
    };

    #[test]
    fn undo_and_redo_restore_states() {
        let mut history = History::new(ROOMY);
        let mut state = Sized(1);
        history.record(state);
        state = Sized(2);

        assert!(history.undo(&mut state));
        assert_eq!(state, Sized(1));
        assert!(history.redo(&mut state));
        assert_eq!(state, Sized(2));
        assert!(!history.can_redo());
    }

    #[test]
    fn empty_history_does_nothing() {
        let mut history: History<Sized> = History::new(ROOMY);
        let mut state = Sized(5);

        assert!(!history.undo(&mut state));
        assert!(!history.redo(&mut state));
        assert_eq!(state, Sized(5));
    }

    #[test]
    fn record_clears_redo() {
        let mut history = History::new(ROOMY);
        let mut state = Sized(1);
        history.record(state);
        state = Sized(2);
        history.undo(&mut state);

        history.record(state);

        assert!(!history.can_redo());
        assert!(history.can_undo());
    }

    #[test]
    fn step_limit_drops_oldest_snapshot() {
        let limits = HistoryLimits {
            max_steps: 2,
            ..ROOMY
        };
        let mut history = History::new(limits);
        let mut state = Sized(0);
        for next in 1..=3 {
            history.record(state);
            state = Sized(next);
        }

        assert!(history.undo(&mut state));
        assert!(history.undo(&mut state));
        assert!(!history.undo(&mut state));
        assert_eq!(state, Sized(1));
    }

    #[test]
    fn memory_budget_drops_oldest_but_keeps_minimum() {
        let limits = HistoryLimits {
            max_steps: 100,
            max_bytes: 250,
            min_steps: 2,
        };
        let mut history = History::new(limits);

        for _ in 0..10 {
            history.record(Sized(100));
        }
        // 2 x 100 bytes fit in the 250-byte budget; a third would not.
        assert_eq!(history.undo_steps(), 2);
        for _ in 0..3 {
            history.record(Sized(1_000));
        }

        // Each snapshot alone is over budget, but the minimum is still kept.
        assert_eq!(history.undo_steps(), 2);
    }

    #[test]
    fn zero_limit_records_nothing() {
        let limits = HistoryLimits {
            max_steps: 0,
            ..ROOMY
        };
        let mut history = History::new(limits);

        history.record(Sized(1));

        assert!(!history.can_undo());
    }
}
