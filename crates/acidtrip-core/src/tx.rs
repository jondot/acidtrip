//! Transactions: the single way documents change. Undo/redo, versioning and
//! AI edits all go through this.
//!
//! Edits name the frame they belong to by id, so they land on the right
//! frame whichever one is being shown (switching frames is view state, not
//! an edit) and wherever it has been moved since.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{Canvas, Cell, DocMeta, Document, FIRST_FRAME, Frame, FrameSet};

fn is_first(id: &u64) -> bool {
    *id == FIRST_FRAME
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellChange {
    /// Frame id (absent in files from before animation: the first frame).
    #[serde(default, skip_serializing_if = "is_first")]
    pub frame: u64,
    pub layer: usize,
    pub x: usize,
    pub y: usize,
    pub before: Option<Cell>,
    pub after: Option<Cell>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DocChange {
    /// A frame's whole canvas replaced (resize, line/column insert/delete, layer ops, crop).
    Canvas {
        #[serde(default, skip_serializing_if = "is_first")]
        frame: u64,
        before: Box<Canvas>,
        after: Box<Canvas>,
    },
    /// Metadata replaced (palette, iCE, kind, SAUCE, font, tabs).
    Meta { before: Box<DocMeta>, after: Box<DocMeta> },
    /// A frame (with its canvas) added at `index`.
    FrameInsert { index: usize, frame: Box<Frame> },
    /// The frame at `index` (as it was) removed.
    FrameRemove { index: usize, frame: Box<Frame> },
    /// A frame moved from one position to another.
    FrameMove { id: u64, from: usize, to: usize },
    /// How long a frame stays up.
    FrameHold { id: u64, before: u32, after: u32 },
    /// Playback speed.
    Fps { before: u32, after: u32 },
    /// Every frame replaced at once (version restore).
    Frames { before: Box<FrameSet>, after: Box<FrameSet> },
}

impl DocChange {
    /// The change that undoes this one.
    pub fn inverse(&self) -> DocChange {
        match self {
            DocChange::Canvas { frame, before, after } => {
                DocChange::Canvas { frame: *frame, before: after.clone(), after: before.clone() }
            }
            DocChange::Meta { before, after } => DocChange::Meta { before: after.clone(), after: before.clone() },
            DocChange::FrameInsert { index, frame } => DocChange::FrameRemove { index: *index, frame: frame.clone() },
            DocChange::FrameRemove { index, frame } => DocChange::FrameInsert { index: *index, frame: frame.clone() },
            DocChange::FrameMove { id, from, to } => DocChange::FrameMove { id: *id, from: *to, to: *from },
            DocChange::FrameHold { id, before, after } => {
                DocChange::FrameHold { id: *id, before: *after, after: *before }
            }
            DocChange::Fps { before, after } => DocChange::Fps { before: *after, after: *before },
            DocChange::Frames { before, after } => DocChange::Frames { before: after.clone(), after: before.clone() },
        }
    }

    /// The frame this change is about, if it is about one.
    pub fn frame(&self) -> Option<u64> {
        match self {
            DocChange::Canvas { frame, .. } => Some(*frame),
            DocChange::FrameInsert { frame, .. } | DocChange::FrameRemove { frame, .. } => Some(frame.id),
            DocChange::FrameMove { id, .. } | DocChange::FrameHold { id, .. } => Some(*id),
            DocChange::Meta { .. } | DocChange::Fps { .. } | DocChange::Frames { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Transaction {
    pub label: String,
    /// Applied in order: doc changes first, then cell changes.
    pub doc: Vec<DocChange>,
    pub cells: Vec<CellChange>,
}

impl Transaction {
    pub fn is_empty(&self) -> bool {
        self.doc.is_empty() && self.cells.is_empty()
    }

    /// The frame this transaction edits (undo shows it again).
    pub fn frame(&self) -> Option<u64> {
        self.cells.first().map(|c| c.frame).or_else(|| self.doc.iter().find_map(DocChange::frame))
    }

    /// Merge `later` into `self` (used to coalesce brush strokes). Cell
    /// changes to the same position keep the earliest `before`.
    pub fn merge(&mut self, later: Transaction) {
        if !later.doc.is_empty() {
            self.doc.extend(later.doc);
            self.cells.extend(later.cells);
            return;
        }
        let mut index: HashMap<(u64, usize, usize, usize), usize> =
            self.cells.iter().enumerate().map(|(i, c)| ((c.frame, c.layer, c.x, c.y), i)).collect();
        for c in later.cells {
            let key = (c.frame, c.layer, c.x, c.y);
            match index.get(&key) {
                Some(&i) if self.doc.is_empty() => self.cells[i].after = c.after,
                _ => {
                    index.insert(key, self.cells.len());
                    self.cells.push(c);
                }
            }
        }
    }
}

impl Document {
    pub fn apply(&mut self, tx: &Transaction) {
        for d in &tx.doc {
            self.apply_change(d);
        }
        let cur = self.frame_id();
        for c in &tx.cells {
            self.set_cell(cur, c, c.after);
        }
    }

    pub fn revert(&mut self, tx: &Transaction) {
        let cur = self.frame_id();
        for c in tx.cells.iter().rev() {
            self.set_cell(cur, c, c.before);
        }
        for d in tx.doc.iter().rev() {
            self.apply_change(&d.inverse());
        }
    }

    fn set_cell(&mut self, cur: u64, c: &CellChange, v: Option<Cell>) {
        let canvas = if c.frame == cur {
            &mut self.canvas
        } else {
            match self.canvas_of_mut(c.frame) {
                Some(canvas) => canvas,
                None => return,
            }
        };
        if let (Some(i), Some(l)) = (canvas.idx(c.x, c.y), canvas.layers.get_mut(c.layer)) {
            l.cells[i] = v;
        }
    }

    /// Frame changes go by id, so they apply the same on every copy of a
    /// shared document, whatever frame each one shows; one that no longer
    /// fits (the frame is gone, or already there) does nothing.
    fn apply_change(&mut self, d: &DocChange) {
        match d {
            DocChange::Canvas { frame, after, .. } => {
                if let Some(c) = self.canvas_of_mut(*frame) {
                    *c = (**after).clone();
                }
            }
            DocChange::Meta { after, .. } => self.meta = (**after).clone(),
            DocChange::FrameInsert { index, frame } => {
                if self.frame_index(frame.id).is_some() {
                    return;
                }
                let (w, h) = (self.width(), self.height());
                let canvas = match &frame.canvas {
                    Some(c) if c.width == w && c.height == h => c.clone(),
                    Some(c) => c.resized(w, h),
                    None => self.blank_frame_canvas(),
                };
                let f = Frame { id: frame.id, hold: frame.hold.max(1), canvas: Some(canvas) };
                let at = (*index).min(self.frames.list.len());
                self.frames.list.insert(at, f);
            }
            DocChange::FrameRemove { frame, .. } => {
                let Some(i) = self.frame_index(frame.id) else { return };
                let n = self.frames.list.len();
                if n <= 1 {
                    return;
                }
                if i == self.current_frame() {
                    self.show_frame(if i + 1 < n { i + 1 } else { i - 1 });
                }
                self.frames.list.remove(i);
            }
            DocChange::FrameMove { id, to, .. } => {
                let Some(i) = self.frame_index(*id) else { return };
                let f = self.frames.list.remove(i);
                let at = (*to).min(self.frames.list.len());
                self.frames.list.insert(at, f);
            }
            DocChange::FrameHold { id, after, .. } => {
                if let Some(i) = self.frame_index(*id) {
                    self.frames.list[i].hold = (*after).max(1);
                }
            }
            DocChange::Fps { after, .. } => self.frames.fps = (*after).clamp(1, 60),
            DocChange::Frames { after, .. } => self.set_frame_set(after),
        }
    }
}

/// Builds a transaction against a document. Reads see pending writes, so
/// tools can compose (e.g. fill reading cells a previous step wrote).
///
/// Cell and canvas edits go to the frame being shown; frame operations
/// (insert, remove, move, hold, fps) are recorded as they are and don't
/// change what the builder reads.
pub struct TxBuilder<'a> {
    doc: &'a Document,
    label: String,
    doc_changes: Vec<DocChange>,
    /// Working copy when a doc-level change was staged (cell edits then apply to it).
    staged_canvas: Option<Canvas>,
    staged_meta: Option<DocMeta>,
    cells: Vec<CellChange>,
    index: HashMap<(usize, usize, usize), usize>,
    frame: u64,
}

impl<'a> TxBuilder<'a> {
    pub fn new(doc: &'a Document, label: impl Into<String>) -> Self {
        TxBuilder {
            doc,
            label: label.into(),
            doc_changes: vec![],
            staged_canvas: None,
            staged_meta: None,
            cells: vec![],
            index: HashMap::new(),
            frame: doc.frame_id(),
        }
    }

    pub fn doc(&self) -> &Document {
        self.doc
    }

    pub fn canvas(&self) -> &Canvas {
        self.staged_canvas.as_ref().unwrap_or(&self.doc.canvas)
    }

    pub fn meta(&self) -> &DocMeta {
        self.staged_meta.as_ref().unwrap_or(&self.doc.meta)
    }

    pub fn width(&self) -> usize {
        self.canvas().width
    }

    pub fn height(&self) -> usize {
        self.canvas().height
    }

    /// Current (pending) value of a layer cell.
    pub fn get(&self, layer: usize, x: usize, y: usize) -> Option<Cell> {
        if let Some(&i) = self.index.get(&(layer, x, y)) {
            return self.cells[i].after;
        }
        self.canvas().get(layer, x, y)
    }

    /// Current (pending) composite value.
    pub fn composite(&self, x: usize, y: usize) -> Cell {
        let c = self.canvas();
        for (li, l) in c.layers.iter().enumerate().rev() {
            if l.visible
                && l.kind == crate::model::LayerKind::Normal
                && let Some(cell) = self.get(li, x, y)
            {
                return cell;
            }
        }
        Cell::BLANK
    }

    /// Set a cell (conformed to the doc kind). Out of bounds and locked
    /// layers are ignored.
    pub fn set(&mut self, layer: usize, x: usize, y: usize, cell: Option<Cell>) {
        let canvas = self.canvas();
        if x >= canvas.width || y >= canvas.height {
            return;
        }
        let Some(l) = canvas.layers.get(layer) else {
            return;
        };
        if l.locked {
            return;
        }
        let cell = cell.map(|c| crate::model::conform_cell(self.meta(), c));
        match self.index.get(&(layer, x, y)) {
            Some(&i) => self.cells[i].after = cell,
            None => {
                let before = canvas.get(layer, x, y);
                if before == cell {
                    return;
                }
                self.index.insert((layer, x, y), self.cells.len());
                self.cells.push(CellChange { frame: self.frame, layer, x, y, before, after: cell });
            }
        }
    }

    /// Stage a whole-canvas replacement of the frame being shown. Pending
    /// cell edits are folded into the "before" state first so ordering
    /// stays correct.
    pub fn replace_canvas(&mut self, f: impl FnOnce(&Canvas) -> Canvas) {
        let mut current = self.canvas().clone();
        for c in &self.cells {
            if let (Some(i), Some(l)) = (current.idx(c.x, c.y), current.layers.get_mut(c.layer)) {
                l.cells[i] = c.after;
            }
        }
        let original = self.doc.canvas.clone();
        let next = f(&current);
        self.cells.clear();
        self.index.clear();
        let frame = self.frame;
        self.doc_changes.retain(|d| !matches!(d, DocChange::Canvas { frame: f, .. } if *f == frame));
        self.doc_changes.push(DocChange::Canvas { frame, before: Box::new(original), after: Box::new(next.clone()) });
        self.staged_canvas = Some(next);
    }

    /// Replace the canvas of every frame with `f` of it (resize, crop,
    /// converting colors): frames always share a size.
    pub fn replace_all_canvases(&mut self, f: impl Fn(&Canvas) -> Canvas) {
        self.replace_canvas(&f);
        for (i, fr) in self.doc.frames.list.iter().enumerate() {
            if fr.id == self.frame {
                continue;
            }
            let staged = self.doc_changes.iter().position(|d| matches!(d, DocChange::Canvas { frame, .. } if *frame == fr.id));
            let (before, input) = match staged.map(|p| self.doc_changes.remove(p)) {
                Some(DocChange::Canvas { before, after, .. }) => (before, *after),
                _ => (Box::new(self.doc.frame_canvas(i).clone()), self.doc.frame_canvas(i).clone()),
            };
            self.doc_changes.push(DocChange::Canvas { frame: fr.id, before, after: Box::new(f(&input)) });
        }
    }

    pub fn replace_meta(&mut self, f: impl FnOnce(&mut DocMeta)) {
        let mut m = self.meta().clone();
        f(&mut m);
        self.doc_changes.retain(|d| !matches!(d, DocChange::Meta { .. }));
        self.doc_changes.push(DocChange::Meta { before: Box::new(self.doc.meta.clone()), after: Box::new(m.clone()) });
        self.staged_meta = Some(m);
    }

    /// Add a frame holding `canvas` at `index`; returns its id.
    pub fn insert_frame(&mut self, index: usize, canvas: Canvas, hold: u32) -> u64 {
        let id = self.doc.new_frame_id();
        let index = index.min(self.doc.frame_count());
        self.doc_changes.push(DocChange::FrameInsert {
            index,
            frame: Box::new(Frame { id, hold: hold.max(1), canvas: Some(canvas) }),
        });
        id
    }

    /// Remove frame `index` (never the last one left).
    pub fn remove_frame(&mut self, index: usize) {
        if self.doc.frame_count() <= 1 || index >= self.doc.frame_count() {
            return;
        }
        let f = &self.doc.frames.list[index];
        let frame = Frame { id: f.id, hold: f.hold, canvas: Some(self.doc.frame_canvas(index).clone()) };
        self.doc_changes.push(DocChange::FrameRemove { index, frame: Box::new(frame) });
    }

    /// Remove every frame but the one being shown.
    pub fn keep_only_current_frame(&mut self) {
        let cur = self.doc.current_frame();
        for i in (0..self.doc.frame_count()).rev().filter(|&i| i != cur) {
            self.remove_frame(i);
        }
    }

    pub fn move_frame(&mut self, from: usize, to: usize) {
        let n = self.doc.frame_count();
        let to = to.min(n.saturating_sub(1));
        if from >= n || from == to {
            return;
        }
        self.doc_changes.push(DocChange::FrameMove { id: self.doc.frames.list[from].id, from, to });
    }

    pub fn set_hold(&mut self, index: usize, hold: u32) {
        let Some(f) = self.doc.frames.list.get(index) else { return };
        let hold = hold.clamp(1, 99);
        if f.hold != hold {
            self.doc_changes.push(DocChange::FrameHold { id: f.id, before: f.hold, after: hold });
        }
    }

    pub fn set_fps(&mut self, fps: u32) {
        let fps = fps.clamp(1, 60);
        if self.doc.frames.fps != fps {
            self.doc_changes.push(DocChange::Fps { before: self.doc.frames.fps, after: fps });
        }
    }

    /// Replace every frame at once.
    pub fn replace_frames(&mut self, set: FrameSet) {
        let before = self.doc.frame_set();
        if before != set {
            self.doc_changes.push(DocChange::Frames { before: Box::new(before), after: Box::new(set) });
        }
    }

    pub fn finish(self) -> Transaction {
        // Meta first so a kind change applies before cells conform.
        let mut doc = self.doc_changes;
        doc.sort_by_key(|d| !matches!(d, DocChange::Meta { .. }));
        Transaction { label: self.label, doc, cells: self.cells.into_iter().filter(|c| c.before != c.after).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::model::DocKind;

    fn cell(ch: char) -> Cell {
        Cell::new(ch, Color::WHITE, Color::BLACK)
    }

    #[test]
    fn apply_and_revert_roundtrip() {
        let mut d = Document::new(DocKind::Classic, 10, 5);
        let orig = d.clone();
        let mut b = TxBuilder::new(&d, "t");
        b.set(0, 1, 1, Some(cell('A')));
        b.set(0, 2, 1, Some(cell('B')));
        b.set(0, 1, 1, Some(cell('C')));
        let tx = b.finish();
        assert_eq!(tx.cells.len(), 2);
        d.apply(&tx);
        assert_eq!(d.canvas.get(0, 1, 1).unwrap().ch, 'C');
        d.revert(&tx);
        assert_eq!(d, orig);
    }

    #[test]
    fn canvas_change_then_cells() {
        let mut d = Document::new(DocKind::Classic, 4, 4);
        let orig = d.clone();
        let mut b = TxBuilder::new(&d, "resize+draw");
        b.set(0, 0, 0, Some(cell('A')));
        b.replace_canvas(|c| c.resized(8, 8));
        b.set(0, 7, 7, Some(cell('Z')));
        let tx = b.finish();
        d.apply(&tx);
        assert_eq!(d.width(), 8);
        assert_eq!(d.canvas.get(0, 0, 0).unwrap().ch, 'A');
        assert_eq!(d.canvas.get(0, 7, 7).unwrap().ch, 'Z');
        d.revert(&tx);
        assert_eq!(d, orig);
    }

    #[test]
    fn merge_keeps_first_before() {
        let d = Document::new(DocKind::Classic, 4, 4);
        let mut b = TxBuilder::new(&d, "a");
        b.set(0, 0, 0, Some(cell('A')));
        let mut t1 = b.finish();
        let mut d2 = d.clone();
        d2.apply(&t1);
        let mut b = TxBuilder::new(&d2, "b");
        b.set(0, 0, 0, Some(cell('B')));
        t1.merge(b.finish());
        assert_eq!(t1.cells.len(), 1);
        assert_eq!(t1.cells[0].before, Some(Cell::BLANK));
        assert_eq!(t1.cells[0].after.unwrap().ch, 'B');
    }

    #[test]
    fn classic_conforms_on_set() {
        let d = Document::new(DocKind::Classic, 2, 2);
        let mut b = TxBuilder::new(&d, "x");
        b.set(0, 0, 0, Some(Cell::new('╭', Color::Rgb(255, 255, 255), Color::BLACK)));
        let tx = b.finish();
        assert_eq!(tx.cells[0].after.unwrap().ch, '┌');
        assert_eq!(tx.cells[0].after.unwrap().fg, Color::Pal(15));
    }
}
