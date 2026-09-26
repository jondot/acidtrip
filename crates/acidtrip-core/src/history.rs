//! Linear undo/redo over transactions, with stroke coalescing and a
//! revision counter for dirty tracking / autosave.
//!
//! In a shared session (drawing together) the history also records every
//! change it makes, for sending to the others, and undo becomes per-user:
//! it only reverts cells that still hold what this user wrote.

use std::collections::HashSet;

use crate::model::{Document, Frame};
use crate::replay::{EditLog, LogOp};
use crate::tx::{CellChange, DocChange, Transaction};

pub const DEFAULT_CAPACITY: usize = 10_000;

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// While a group is open, commits merge into the top undo entry.
    group_open: bool,
    group_started: bool,
    capacity: usize,
    /// Bumped on every change (commit, undo, redo).
    revision: u64,
    /// The undo depth the saved state sits at, so undoing back to it
    /// reads as clean again (with `saved_lost`: no depth reaches it).
    saved_depth: usize,
    saved_lost: bool,
    /// Shared session: every change made through this history, in order,
    /// waiting to be sent (see [`History::take_changes`]).
    changes: Option<Vec<Transaction>>,
    /// Everything that happened, for replay (outlives `clear`).
    log: EditLog,
}

impl History {
    pub fn new() -> Self {
        History { capacity: DEFAULT_CAPACITY, ..Default::default() }
    }

    /// Apply `tx` to `doc` and record it. Empty transactions are ignored.
    pub fn commit(&mut self, doc: &mut Document, tx: Transaction) {
        if tx.is_empty() {
            return;
        }
        self.log.record(doc, self.log_op_for_commit(), tx.clone());
        doc.apply(&tx);
        if let Some(out) = &mut self.changes {
            out.push(tx.clone());
        }
        // Dropping redo steps drops the saved state if it was among them.
        if self.undo.len() < self.saved_depth {
            self.saved_lost = true;
        }
        self.redo.clear();
        self.revision += 1;
        if self.group_open
            && self.group_started
            && let Some(top) = self.undo.last_mut()
        {
            top.merge(tx);
            // The saved state was the top step, which just changed.
            if self.undo.len() == self.saved_depth {
                self.saved_lost = true;
            }
            return;
        }
        self.group_started = self.group_open;
        self.undo.push(tx);
        if self.undo.len() > self.capacity.max(1) {
            self.undo.remove(0);
            match self.saved_depth.checked_sub(1) {
                Some(d) => self.saved_depth = d,
                None => self.saved_lost = true,
            }
        }
    }

    /// Start coalescing subsequent commits into one undo step (a stroke).
    pub fn begin_group(&mut self) {
        self.group_open = true;
        self.group_started = false;
    }

    /// Close the group. A group that ended up changing nothing (a block
    /// lifted and put back where it was) leaves no undo step behind.
    pub fn end_group(&mut self) {
        let noop = |tx: &Transaction| tx.doc.is_empty() && tx.cells.iter().all(|c| c.before == c.after);
        if self.group_started && self.undo.last().is_some_and(noop) {
            self.undo.pop();
        }
        self.group_open = false;
        self.group_started = false;
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        self.end_group();
        let tx = self.undo.pop()?;
        if self.changes.is_some() {
            let step = guarded(doc, &tx, false);
            if !step.is_empty() {
                // Logged as an undo whose reversal is exactly this step.
                self.log.record(doc, LogOp::Undo, inverted(&step));
            }
            self.apply_shared(doc, step);
        } else {
            self.log.record(doc, LogOp::Undo, tx.clone());
            doc.revert(&tx);
        }
        let label = tx.label.clone();
        self.redo.push(tx);
        self.revision += 1;
        Some(label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        self.end_group();
        let tx = self.redo.pop()?;
        if self.changes.is_some() {
            let step = guarded(doc, &tx, true);
            if !step.is_empty() {
                self.log.record(doc, LogOp::Redo, step.clone());
            }
            self.apply_shared(doc, step);
        } else {
            self.log.record(doc, LogOp::Redo, tx.clone());
            doc.apply(&tx);
        }
        let label = tx.label.clone();
        self.undo.push(tx);
        self.revision += 1;
        Some(label)
    }

    fn apply_shared(&mut self, doc: &mut Document, step: Transaction) {
        if step.is_empty() {
            return;
        }
        doc.apply(&step);
        if let Some(out) = &mut self.changes {
            out.push(step);
        }
    }

    /// Start or stop a shared session. While shared, changes are recorded
    /// for [`History::take_changes`] and undo/redo leave alone cells someone
    /// else changed since.
    pub fn set_shared(&mut self, on: bool) {
        self.changes = on.then(Vec::new);
    }

    pub fn is_shared(&self) -> bool {
        self.changes.is_some()
    }

    /// The changes made since the last call (shared sessions only).
    pub fn take_changes(&mut self) -> Vec<Transaction> {
        self.changes.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Apply a change someone else made: it isn't undoable here, but it
    /// counts as a change for saving and goes into the edit log, so a
    /// session replays in full.
    pub fn apply_remote(&mut self, doc: &mut Document, tx: &Transaction) {
        if tx.is_empty() {
            return;
        }
        self.log.record(doc, LogOp::Remote, tx.clone());
        doc.apply(tx);
        self.revision += 1;
        self.saved_lost = true;
    }

    /// Apply a change of ours coming back from the host, in its order: the
    /// log has it already.
    pub fn apply_echo(&mut self, doc: &mut Document, tx: &Transaction) {
        if tx.is_empty() {
            return;
        }
        doc.apply(tx);
        self.revision += 1;
        self.saved_lost = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|t| t.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|t| t.label.as_str())
    }

    /// The frame the next undo changes (the view goes there first).
    pub fn undo_frame(&self) -> Option<u64> {
        self.undo.last().and_then(Transaction::frame)
    }

    /// The step the next undo reverts.
    pub fn undo_step(&self) -> Option<&Transaction> {
        self.undo.last()
    }

    /// The step the next redo applies.
    pub fn redo_step(&self) -> Option<&Transaction> {
        self.redo.last()
    }

    /// The frame the next redo changes.
    pub fn redo_frame(&self) -> Option<u64> {
        self.redo.last().and_then(Transaction::frame)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_depth = self.undo.len();
        self.saved_lost = false;
    }

    /// Whether the document differs from its last save: undoing back to
    /// the saved state makes it clean again.
    pub fn is_dirty(&self) -> bool {
        self.saved_lost || self.undo.len() != self.saved_depth
    }

    pub fn clear(&mut self) {
        let dirty = self.is_dirty();
        self.saved_depth = 0;
        self.saved_lost = dirty;
        self.undo.clear();
        self.redo.clear();
        self.end_group();
    }

    /// Whether a commit now starts a new step or joins the open group.
    fn log_op_for_commit(&self) -> LogOp {
        if self.group_open && self.group_started && !self.undo.is_empty() { LogOp::Merge } else { LogOp::Commit }
    }

    /// The edit log (for replay and saving).
    pub fn log(&self) -> &EditLog {
        &self.log
    }

    /// Continue the log of a loaded document.
    pub fn set_log(&mut self, log: EditLog) {
        self.log = log;
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }
}

/// The transaction whose revert does what `tx` does (no canvas change
/// together with cell changes on the same frame, as [`guarded`] makes undo steps).
fn inverted(tx: &Transaction) -> Transaction {
    let cells = tx.cells.iter().rev().map(|c| CellChange { before: c.after, after: c.before, ..*c }).collect();
    let doc = tx.doc.iter().rev().map(DocChange::inverse).collect();
    Transaction { label: tx.label.clone(), doc, cells }
}

/// What undoing (`forward` false) or redoing `tx` changes when others may
/// have edited since: only the parts that still look as `tx` left them (or,
/// redoing, as it found them). The result carries real before/after values,
/// so it can be sent on as an ordinary transaction.
fn guarded(doc: &Document, tx: &Transaction, forward: bool) -> Transaction {
    let mut out = Transaction { label: tx.label.clone(), ..Default::default() };
    // Frames whose whole canvas this step puts back.
    let mut whole: HashSet<u64> = HashSet::new();
    let steps: Vec<DocChange> =
        if forward { tx.doc.clone() } else { tx.doc.iter().rev().map(DocChange::inverse).collect() };
    for step in steps {
        match step {
            DocChange::Canvas { frame, before, after } => {
                // Undo: the canvas as the transaction left it (its new canvas plus its cell edits).
                let mut expect = *before;
                if !forward {
                    for ch in tx.cells.iter().filter(|c| c.frame == frame) {
                        if let (Some(i), Some(l)) = (expect.idx(ch.x, ch.y), expect.layers.get_mut(ch.layer)) {
                            l.cells[i] = ch.after;
                        }
                    }
                }
                if doc.canvas_of(frame) == Some(&expect) {
                    whole.insert(frame);
                    out.doc.push(DocChange::Canvas { frame, before: Box::new(expect), after });
                }
            }
            DocChange::Meta { before, after } => {
                if doc.meta == *before {
                    out.doc.push(DocChange::Meta { before, after });
                }
            }
            DocChange::FrameInsert { index, frame } => {
                if doc.frame_index(frame.id).is_none() {
                    out.doc.push(DocChange::FrameInsert { index, frame });
                }
            }
            DocChange::FrameRemove { frame, .. } => {
                // Only a frame nobody has drawn on since.
                if let Some(i) = doc.frame_index(frame.id)
                    && doc.frame_canvas(i) == frame.canvas.as_ref().unwrap_or(&doc.canvas)
                {
                    let now = Frame { canvas: Some(doc.frame_canvas(i).clone()), ..doc.frames.list[i].clone() };
                    out.doc.push(DocChange::FrameRemove { index: i, frame: Box::new(now) });
                }
            }
            DocChange::FrameMove { id, to, .. } => {
                if let Some(i) = doc.frame_index(id) {
                    out.doc.push(DocChange::FrameMove { id, from: i, to });
                }
            }
            DocChange::FrameHold { id, before, after } => {
                if doc.frame_index(id).is_some_and(|i| doc.frames.list[i].hold == before) {
                    out.doc.push(DocChange::FrameHold { id, before, after });
                }
            }
            DocChange::Fps { before, after } => {
                if doc.frames.fps == before {
                    out.doc.push(DocChange::Fps { before, after });
                }
            }
            DocChange::Frames { before, after } => {
                let now = doc.frame_set();
                if now.frames == before.frames && now.fps == before.fps {
                    out.doc.push(DocChange::Frames { before: Box::new(now), after });
                }
            }
        }
    }
    // Metadata first, as the builder orders it.
    out.doc.sort_by_key(|d| !matches!(d, DocChange::Meta { .. }));
    let cells: Box<dyn Iterator<Item = &CellChange>> =
        if forward { Box::new(tx.cells.iter()) } else { Box::new(tx.cells.iter().rev()) };
    for c in cells {
        let (from, to) = if forward { (c.before, c.after) } else { (c.after, c.before) };
        let keep = if whole.contains(&c.frame) {
            // Undoing, the old canvas already holds the cells as they were.
            forward
        } else {
            doc.canvas_of(c.frame).is_some_and(|cv| cv.get(c.layer, c.x, c.y) == from)
        };
        if keep {
            out.cells.push(CellChange { before: from, after: to, ..*c });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::model::{Cell, DocKind};
    use crate::tx::TxBuilder;

    fn put(doc: &Document, x: usize, ch: char) -> Transaction {
        let mut b = TxBuilder::new(doc, "put");
        b.set(0, x, 0, Some(Cell::new(ch, Color::WHITE, Color::BLACK)));
        b.finish()
    }

    #[test]
    fn undo_redo() {
        let mut d = Document::new(DocKind::Classic, 5, 1);
        let mut h = History::new();
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        let t = put(&d, 1, 'B');
        h.commit(&mut d, t);
        assert!(h.is_dirty());
        h.undo(&mut d);
        assert_eq!(d.canvas.composite(1, 0), Cell::BLANK);
        h.redo(&mut d);
        assert_eq!(d.canvas.composite(1, 0).ch, 'B');
    }

    #[test]
    fn a_group_that_changes_nothing_leaves_no_step() {
        let mut d = Document::new(DocKind::Classic, 5, 1);
        let mut h = History::new();
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        h.begin_group();
        let mut b = TxBuilder::new(&d, "Move");
        b.set(0, 0, 0, None);
        let t = b.finish();
        h.commit(&mut d, t);
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        h.end_group();
        assert_eq!(h.len(), 1, "lifting and putting back is not a step");
        assert_eq!(h.undo_label(), Some("put"));
        assert_eq!(d.canvas.composite(0, 0).ch, 'A');
    }

    #[test]
    fn stroke_groups_into_one_step() {
        let mut d = Document::new(DocKind::Classic, 5, 1);
        let mut h = History::new();
        h.begin_group();
        for x in 0..5 {
            let t = put(&d, x, '#');
            h.commit(&mut d, t);
        }
        h.end_group();
        assert_eq!(h.len(), 1);
        h.undo(&mut d);
        assert!((0..5).all(|x| d.canvas.composite(x, 0) == Cell::BLANK));
    }

    #[test]
    fn logs_every_change() {
        use crate::replay::{Timeline, TimelineOptions};
        let mut d = Document::new(DocKind::Classic, 5, 1);
        let mut h = History::new();
        h.begin_group();
        for x in 0..3 {
            let t = put(&d, x, '#');
            h.commit(&mut d, t);
        }
        h.end_group();
        h.undo(&mut d);
        h.redo(&mut d);
        let ops: Vec<LogOp> = h.log().entries().iter().map(|e| e.op).collect();
        assert_eq!(ops, [LogOp::Commit, LogOp::Merge, LogOp::Merge, LogOp::Undo, LogOp::Redo]);
        let mut tl = Timeline::new(h.log(), TimelineOptions::default()).unwrap();
        tl.seek(3);
        assert_eq!(tl.doc(), &d);
        tl.seek(4);
        assert_eq!(tl.doc().canvas.composite(0, 0), Cell::BLANK);
    }

    #[test]
    fn saved_marker() {
        let mut d = Document::new(DocKind::Classic, 2, 1);
        let mut h = History::new();
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        h.mark_saved();
        assert!(!h.is_dirty());
        h.undo(&mut d);
        assert!(h.is_dirty());
        // Back to the saved state: clean again.
        h.redo(&mut d);
        assert!(!h.is_dirty());
    }

    #[test]
    fn undo_to_the_saved_state_is_clean() {
        let mut d = Document::new(DocKind::Classic, 3, 1);
        let mut h = History::new();
        assert!(!h.is_dirty());
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        assert!(h.is_dirty());
        h.undo(&mut d);
        assert!(!h.is_dirty(), "undoing every change of a fresh document leaves it clean");
        // A new change after undoing past the save loses the saved state.
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        h.mark_saved();
        h.undo(&mut d);
        let t = put(&d, 1, 'B');
        h.commit(&mut d, t);
        assert!(h.is_dirty());
        h.undo(&mut d);
        assert!(h.is_dirty(), "the saved state is gone from the history");
    }

    #[test]
    fn merging_into_the_saved_step_is_dirty() {
        let mut d = Document::new(DocKind::Classic, 3, 1);
        let mut h = History::new();
        h.begin_group();
        let t = put(&d, 0, 'A');
        h.commit(&mut d, t);
        h.mark_saved();
        let t = put(&d, 1, 'B');
        h.commit(&mut d, t);
        h.end_group();
        assert!(h.is_dirty());
    }

    #[test]
    fn remote_changes_stay_dirty() {
        let mut d = Document::new(DocKind::Classic, 3, 1);
        let mut h = History::new();
        let t = put(&d, 0, 'A');
        h.apply_remote(&mut d, &t);
        assert!(h.is_dirty());
        h.clear();
        assert!(h.is_dirty());
        h.mark_saved();
        h.clear();
        assert!(!h.is_dirty());
    }

    #[test]
    fn shared_undo_skips_cells_others_changed() {
        let mut d = Document::new(DocKind::Classic, 5, 1);
        let mut h = History::new();
        h.set_shared(true);
        // I write A at 0 and 1; someone else then writes Z over cell 1.
        let mut b = TxBuilder::new(&d, "mine");
        b.set(0, 0, 0, Some(Cell::new('A', Color::WHITE, Color::BLACK)));
        b.set(0, 1, 0, Some(Cell::new('A', Color::WHITE, Color::BLACK)));
        let t = b.finish();
        h.commit(&mut d, t);
        let theirs = put(&d, 1, 'Z');
        h.apply_remote(&mut d, &theirs);
        assert_eq!(h.take_changes().len(), 1, "remote edits aren't sent back");
        h.undo(&mut d);
        assert_eq!(d.canvas.composite(0, 0), Cell::BLANK);
        assert_eq!(d.canvas.composite(1, 0).ch, 'Z', "their cell survives my undo");
        // The undo goes out as an ordinary change of just my cell.
        let sent = h.take_changes();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].cells.len(), 1);
        assert_eq!(sent[0].cells[0].x, 0);
        h.redo(&mut d);
        assert_eq!(d.canvas.composite(0, 0).ch, 'A');
        assert_eq!(d.canvas.composite(1, 0).ch, 'Z');
    }

    #[test]
    fn shared_session_replays_in_full() {
        let mut d = Document::new(DocKind::Classic, 5, 1);
        let mut h = History::new();
        h.set_shared(true);
        let mut b = TxBuilder::new(&d, "mine");
        b.set(0, 0, 0, Some(Cell::new('A', Color::WHITE, Color::BLACK)));
        b.set(0, 1, 0, Some(Cell::new('A', Color::WHITE, Color::BLACK)));
        let t = b.finish();
        h.commit(&mut d, t);
        let theirs = put(&d, 1, 'Z');
        h.apply_remote(&mut d, &theirs);
        // My own edit coming back from the host isn't logged twice.
        let mine = put(&d, 3, 'M');
        h.commit(&mut d, mine.clone());
        h.apply_echo(&mut d, &mine);
        h.undo(&mut d);
        h.redo(&mut d);
        h.undo(&mut d);
        let entries = h.log().entries();
        let ops: Vec<LogOp> = entries.iter().map(|e| e.op).collect();
        assert_eq!(ops, [LogOp::Commit, LogOp::Remote, LogOp::Commit, LogOp::Undo, LogOp::Redo, LogOp::Undo]);
        // Played forward, the log ends where the document is; played back, where it began.
        let mut r = h.log().base_doc().unwrap();
        let start = r.clone();
        for e in &entries {
            e.forward(&mut r);
        }
        assert_eq!(r, d);
        for e in entries.iter().rev() {
            e.backward(&mut r);
        }
        assert_eq!(r, start);
        // The guarded undo of my first edit keeps their Z, in the replay too.
        h.undo(&mut d);
        let mut r = h.log().base_doc().unwrap();
        for e in &h.log().entries() {
            e.forward(&mut r);
        }
        assert_eq!(r, d);
        assert_eq!(d.canvas.composite(1, 0).ch, 'Z');
        assert_eq!(d.canvas.composite(0, 0), Cell::BLANK);
    }

    #[test]
    fn shared_undo_of_canvas_change() {
        let mut d = Document::new(DocKind::Classic, 4, 2);
        let orig = d.clone();
        let mut h = History::new();
        h.set_shared(true);
        let mut b = TxBuilder::new(&d, "resize");
        b.replace_canvas(|c| c.resized(8, 2));
        b.set(0, 6, 0, Some(Cell::new('R', Color::WHITE, Color::BLACK)));
        let t = b.finish();
        h.commit(&mut d, t);
        h.undo(&mut d);
        assert_eq!(d, orig);
        let sent = h.take_changes();
        assert_eq!(sent.len(), 2);
        // Replaying what was sent on a copy gets the same document.
        let mut other = orig.clone();
        for t in &sent {
            other.apply(t);
        }
        assert_eq!(other, d);
    }
}
