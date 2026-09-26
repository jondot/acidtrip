//! An open document: doc + history + file + view state.

use std::path::PathBuf;
use std::time::Instant;

use acidtrip_core::tools::Rect;
use acidtrip_core::tx::DocChange;
use acidtrip_core::{Document, History, Layer, Transaction, TxBuilder};
use acidtrip_io::format::Format;

pub struct Tab {
    pub doc: Document,
    pub history: History,
    pub file: Option<PathBuf>,
    /// Format to save back to (None = ask / .acid).
    pub format: Option<Format>,
    pub cursor: (usize, usize),
    /// Top-left canvas cell shown in the viewport.
    pub scroll: (usize, usize),
    pub selection: Option<Rect>,
    pub layer: usize,
    pub zoom: bool,
    pub autosaved_rev: u64,
    /// A recovery file for this document is on disk.
    pub recovery_written: bool,
    pub versioned_rev: u64,
    pub last_version_at: Instant,
    /// Text tool: column typing returns to on Enter.
    pub text_home_x: usize,
    /// Text tool insert mode (shift the line right when typing).
    pub insert_mode: bool,
    /// The FRAMES panel is open.
    pub frames_panel: bool,
    /// The EXPORT panel is open.
    pub export_panel: bool,
    /// EXPORT writes the whole piece even while something is selected.
    pub export_whole: bool,
    /// Onion skin: the previous / next frame shows dimmed through empty cells.
    pub onion: (bool, bool),
    /// Playing the frames: when the shown frame is due to change.
    pub playing: Option<Instant>,
    /// Text typed in one go, oldest first: where the text cursor began and
    /// ended, so undo puts the cursor back where the undone text began (and
    /// redo where it ended), and typing carries on from there.
    type_runs: Vec<TypeRun>,
    type_redo: Vec<TypeRun>,
}

type TypeRun = ((usize, usize), (usize, usize));

impl Tab {
    pub fn new(doc: Document, file: Option<PathBuf>) -> Tab {
        // Ctrl-S resaves only to a format that opens back as the same art: an
        // opened PNG is an import, and saving must not paint over it.
        let format = file.as_deref().and_then(Format::from_path).filter(|f| f.reopens());
        let top = doc.canvas.layers.len().saturating_sub(1);
        let animated = doc.is_animated();
        Tab {
            doc,
            history: History::new(),
            file,
            format,
            cursor: (0, 0),
            scroll: (0, 0),
            selection: None,
            layer: top,
            zoom: false,
            autosaved_rev: 0,
            recovery_written: false,
            versioned_rev: 0,
            last_version_at: Instant::now(),
            text_home_x: 0,
            insert_mode: false,
            frames_panel: animated,
            export_panel: false,
            export_whole: false,
            onion: (true, false),
            playing: None,
            type_runs: vec![],
            type_redo: vec![],
        }
    }

    /// Typing starts at the cursor (one undo step).
    pub fn type_run_start(&mut self) {
        self.type_redo.clear();
        if self.type_runs.len() >= 256 {
            self.type_runs.remove(0);
        }
        self.type_runs.push((self.cursor, self.cursor));
    }

    /// The text cursor moved while typing.
    pub fn type_run_moved(&mut self) {
        if let Some(r) = self.type_runs.last_mut() {
            r.1 = self.cursor;
        }
    }

    /// Carry on the edit log a loaded document came with (for replay).
    pub fn with_log(mut self, log: Option<acidtrip_core::replay::EditLog>) -> Tab {
        if let Some(l) = log {
            self.history.set_log(l);
        }
        self
    }

    pub fn title(&self) -> String {
        match &self.file {
            Some(p) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
            None => "untitled".into(),
        }
    }

    pub fn dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// Run `f` against a builder and commit the result as one undo step.
    pub fn edit(&mut self, label: &str, f: impl FnOnce(&mut TxBuilder)) {
        let tx = {
            let mut b = TxBuilder::new(&self.doc, label);
            f(&mut b);
            b.finish()
        };
        self.commit(tx);
    }

    pub fn commit(&mut self, tx: Transaction) {
        self.history.commit(&mut self.doc, tx);
        self.clamp();
    }

    /// Undo; the view moves to the frame the step changes first, so you see
    /// what was undone.
    pub fn undo(&mut self) -> Option<String> {
        let id = self.history.undo_frame();
        self.follow_frame(id);
        let layer = step_layer(self.history.undo_step(), self.doc.frame_id(), true, self.layer);
        let r = self.history.undo(&mut self.doc);
        if r.as_deref() == Some("Type")
            && let Some(run) = self.type_runs.pop()
        {
            self.cursor = run.0;
            self.type_redo.push(run);
        }
        self.layer = layer.unwrap_or(self.layer);
        self.clamp();
        // A frame the step brought back shows too.
        self.follow_frame(id);
        r
    }

    pub fn redo(&mut self) -> Option<String> {
        let id = self.history.redo_frame();
        self.follow_frame(id);
        let layer = step_layer(self.history.redo_step(), self.doc.frame_id(), false, self.layer);
        let r = self.history.redo(&mut self.doc);
        if r.as_deref() == Some("Type")
            && let Some(run) = self.type_redo.pop()
        {
            self.cursor = run.1;
            self.type_runs.push(run);
        }
        self.layer = layer.unwrap_or(self.layer);
        self.clamp();
        // A frame the step brought back shows too.
        self.follow_frame(id);
        r
    }

    fn follow_frame(&mut self, id: Option<u64>) {
        if let Some(i) = id.and_then(|id| self.doc.frame_index(id)) {
            self.show_frame(i);
        }
    }

    /// Show (and edit) frame `i`. View state: not an edit.
    pub fn show_frame(&mut self, i: usize) -> bool {
        let moved = self.doc.show_frame(i);
        if moved {
            self.clamp();
        }
        moved
    }

    /// Frame `i`, wrapping around at both ends.
    pub fn step_frame(&mut self, by: isize) -> bool {
        let n = self.doc.frame_count() as isize;
        let i = (self.doc.current_frame() as isize + by).rem_euclid(n.max(1));
        self.show_frame(i as usize)
    }

    /// Advance playback when a frame's time is up; true when the view changed.
    pub fn play_tick(&mut self, now: Instant) -> bool {
        let Some(due) = self.playing else { return false };
        if self.doc.frame_count() < 2 {
            self.playing = None;
            return true;
        }
        if now < due {
            return false;
        }
        self.step_frame(1);
        let tick = std::time::Duration::from_secs_f64(1.0 / self.doc.fps() as f64);
        let next = due + tick * self.doc.hold(self.doc.current_frame());
        // Fell far behind (a slow terminal, a dialog): don't race to catch up.
        self.playing = Some(if next < now { now + tick } else { next });
        true
    }

    /// Keep cursor, layer and selection valid after structural changes.
    pub fn clamp(&mut self) {
        let (w, h) = (self.doc.width(), self.doc.height());
        self.cursor.0 = self.cursor.0.min(w.saturating_sub(1));
        self.cursor.1 = self.cursor.1.min(h.saturating_sub(1));
        self.layer = self.layer.min(self.doc.canvas.layers.len().saturating_sub(1));
        if let Some(s) = self.selection {
            if s.x >= w || s.y >= h {
                self.selection = None;
            } else {
                self.selection = Some(Rect::new(s.x, s.y, s.w.min(w - s.x), s.h.min(h - s.y)));
            }
        }
    }

    /// Scroll back as far as a viewport of `vw` x `vh` cells allows: a view
    /// that grew (a closed sidebar, a bigger terminal) shows no empty space
    /// past the canvas while there is art scrolled off the other side.
    pub fn clamp_scroll(&mut self, vw: usize, vh: usize) {
        let (w, h) = (self.doc.width(), self.doc.height());
        self.scroll.0 = self.scroll.0.min(w.saturating_sub(vw));
        self.scroll.1 = self.scroll.1.min(h.saturating_sub(vh));
    }

    /// Scroll so the cursor is visible in a viewport of `vw` x `vh` cells.
    pub fn follow_cursor(&mut self, vw: usize, vh: usize) {
        let (cx, cy) = self.cursor;
        if vw > 0 {
            if cx < self.scroll.0 {
                self.scroll.0 = cx;
            } else if cx >= self.scroll.0 + vw {
                self.scroll.0 = cx + 1 - vw;
            }
        }
        if vh > 0 {
            if cy < self.scroll.1 {
                self.scroll.1 = cy;
            } else if cy >= self.scroll.1 + vh {
                self.scroll.1 = cy + 1 - vh;
            }
        }
    }
}

/// When undoing (or redoing) `tx` on frame `frame` adds, removes or moves a
/// layer, the layer to work on afterwards: the one that came back (an undone
/// merge or removal), the one below the layer that went away, or the
/// `active` layer at its new place.
fn step_layer(tx: Option<&Transaction>, frame: u64, undo: bool, active: usize) -> Option<usize> {
    let canvases: Vec<_> = tx?
        .doc
        .iter()
        .filter_map(|c| match c {
            DocChange::Canvas { frame: f, before, after } if *f == frame => Some((before, after)),
            _ => None,
        })
        .collect();
    let (first, last) = (canvases.first()?, canvases.last()?);
    let (from, to) = if undo { (&last.1.layers, &first.0.layers) } else { (&first.0.layers, &last.1.layers) };
    changed_layer(from, to, active)
}

/// Comparing from the top down, where two layer stacks of different sizes
/// first differ, as an index into `to`. Stacks of one size (layers moved):
/// where the `active` layer went.
fn changed_layer(from: &[Layer], to: &[Layer], active: usize) -> Option<usize> {
    if to.is_empty() {
        return None;
    }
    if from.len() == to.len() {
        let l = from.get(active)?;
        if to.get(active) == Some(l) {
            return None;
        }
        return to.iter().position(|t| t == l);
    }
    let same = from.iter().rev().zip(to.iter().rev()).take_while(|(a, b)| a == b).count();
    Some(to.len().saturating_sub(1 + same))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::tools;

    fn typed(t: &mut Tab, s: &str) {
        t.history.begin_group();
        t.type_run_start();
        for c in s.chars() {
            let (x, y) = t.cursor;
            t.edit("Type", |b| b.set(0, x, y, Some(acidtrip_core::Cell::new(c, Default::default(), Default::default()))));
            t.cursor.0 += 1;
            t.type_run_moved();
        }
        t.history.end_group();
    }

    #[test]
    fn undoing_typing_puts_the_cursor_back() {
        let mut t = Tab::new(Document::new(acidtrip_core::DocKind::Classic, 80, 25), None);
        t.cursor = (4, 2);
        typed(&mut t, "hello");
        t.cursor = (10, 5);
        typed(&mut t, "yo");
        assert_eq!(t.cursor, (12, 5));
        assert_eq!(t.undo().as_deref(), Some("Type"));
        assert_eq!(t.cursor, (10, 5));
        assert_eq!(t.undo().as_deref(), Some("Type"));
        assert_eq!(t.cursor, (4, 2));
        assert_eq!(t.redo().as_deref(), Some("Type"));
        assert_eq!(t.cursor, (9, 2));
        // New typing drops what could be redone.
        typed(&mut t, "!");
        assert_eq!(t.redo(), None);
        assert_eq!(t.undo().as_deref(), Some("Type"));
        assert_eq!(t.cursor, (9, 2));
    }

    fn layer(name: &str) -> Layer {
        Layer::new(name, 4, 2)
    }

    fn names(t: &Tab) -> Vec<String> {
        t.doc.canvas.layers.iter().map(|l| l.name.clone()).collect()
    }

    #[test]
    fn changed_layer_picks_the_layer_that_came_or_went() {
        let (a, b, c) = (layer("A"), layer("B"), layer("C"));
        // undo of adding B on top of A: back to A
        assert_eq!(changed_layer(&[a.clone(), b.clone()], std::slice::from_ref(&a), 1), Some(0));
        // undo of removing B from the middle: B again
        assert_eq!(changed_layer(&[a.clone(), c.clone()], &[a.clone(), b.clone(), c.clone()], 0), Some(1));
        // undo of adding B between A and C: A, the layer B was added above
        assert_eq!(changed_layer(&[a.clone(), b.clone(), c.clone()], &[a.clone(), c.clone()], 1), Some(0));
        // a layer moved: the active one is followed to its new place
        assert_eq!(changed_layer(&[a.clone(), b.clone()], &[b.clone(), a.clone()], 1), Some(0));
        // the active layer stayed put (or changed in place): nothing to say
        assert_eq!(changed_layer(&[a.clone(), b.clone(), c.clone()], &[b.clone(), a.clone(), c.clone()], 2), None);
        assert_eq!(changed_layer(&[a.clone(), b.clone()], &[a, c], 1), None);
    }

    #[test]
    fn undoing_a_merge_goes_back_to_the_merged_layer() {
        let mut t = Tab::new(Document::new(acidtrip_core::DocKind::Classic, 4, 2), None);
        t.edit("Add layer", |b| {
            tools::add_layer(b, "Layer 2", 1);
        });
        t.edit("Add layer", |b| {
            tools::add_layer(b, "Layer 3", 2);
        });
        t.layer = 2;
        t.edit("Merge down", |b| tools::merge_down(b, 2));
        t.layer = 1;
        assert_eq!(names(&t).len(), 2);
        t.undo();
        assert_eq!(names(&t).len(), 3);
        assert_eq!(t.layer, 2, "the layer that was merged is active again");
        t.redo();
        assert_eq!(t.layer, 1, "the layer merged into");
        t.layer = 0;
        t.edit("Remove layer", |b| tools::remove_layer(b, 0));
        t.undo();
        assert_eq!(t.layer, 0);
        assert_eq!(names(&t)[0], "Background");
    }
}
