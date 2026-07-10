//! Transaction-based undo/redo history.

use serde::{Deserialize, Serialize};

use crate::action::Action;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub label: String,
    /// Applied actions (for redo), in order.
    pub actions: Vec<Action>,
    /// Inverses (for undo), aligned 1:1 with `actions`.
    pub inverses: Vec<Action>,
    /// When the entry was (last) touched — coalescing window only.
    #[serde(skip)]
    pub stamp: Option<std::time::Instant>,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        History { undo: vec![], redo: vec![], cap }
    }

    pub fn push(&mut self, entry: HistoryEntry) {
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > self.cap {
            let overflow = self.undo.len() - self.cap;
            self.undo.drain(0..overflow);
        }
    }

    /// Merges `entry` into the previous one when it continues the same
    /// gesture (same label): a slider drag emits dozens of dispatches and
    /// must undo as ONE step. The merged entry keeps the FIRST inverses
    /// (state before the gesture) and the LATEST actions (for redo).
    pub fn push_coalesced(&mut self, mut entry: HistoryEntry) {
        const WINDOW: std::time::Duration = std::time::Duration::from_millis(1500);
        self.redo.clear();
        entry.stamp = Some(std::time::Instant::now());
        if let Some(last) = self.undo.last_mut() {
            let recent = last.stamp.is_some_and(|s| s.elapsed() < WINDOW);
            if last.label == entry.label && recent {
                last.actions = entry.actions;
                last.stamp = entry.stamp;
                return;
            }
        }
        self.push(entry);
    }

    pub fn pop_undo(&mut self) -> Option<HistoryEntry> {
        self.undo.pop()
    }
    pub fn push_redo(&mut self, e: HistoryEntry) {
        self.redo.push(e);
    }
    pub fn pop_redo(&mut self) -> Option<HistoryEntry> {
        self.redo.pop()
    }
    pub fn push_undo_from_redo(&mut self, e: HistoryEntry) {
        // returning from a redo: does not clear the remaining redo stack
        self.undo.push(e);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo.iter().map(|e| e.label.as_str()).collect()
    }
}
