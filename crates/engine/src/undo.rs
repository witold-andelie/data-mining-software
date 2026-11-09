//! Bounded transaction history. Each entry contains the edits that restore its target revision.
use dms_stream::{DocMeta, Edit};
use std::collections::VecDeque;

pub const MAX_TRANSACTIONS: usize = 500;

#[derive(Debug, Clone)]
pub(crate) struct Transaction {
    pub label: &'static str,
    pub edits: Vec<Edit>,
    pub revision: u64,
    pub selection: Vec<String>,
    pub meta: DocMeta,
}

#[derive(Debug, Default)]
pub struct UndoStack {
    pub(crate) undo: VecDeque<Transaction>,
    pub(crate) redo: VecDeque<Transaction>,
}

impl UndoStack {
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|t| t.label)
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.back().map(|t| t.label)
    }
    pub(crate) fn push(&mut self, transaction: Transaction) {
        self.redo.clear();
        self.undo.push_back(transaction);
        while self.undo.len() > MAX_TRANSACTIONS {
            self.undo.pop_front();
        }
    }
}
