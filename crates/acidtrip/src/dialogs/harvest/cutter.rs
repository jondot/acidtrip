//! Stage 3: the cutter. The whole piece is shown; select a letter's cells
//! whichever way fits it (lasso around it, paint over it, click it with the
//! wand, or drag a box) and type the letter it is. There is no font format
//! to squeeze into: a letter is whatever you selected, any size and shape,
//! overlapping its neighbours or not. Enter puts the letters into
//! `<artist>-<style>.acidfont`; Claude's reading of a logo only prefills
//! letters you can then fix.

use std::path::PathBuf;

use acidtrip_ai::harvest::{self, Attribution, Candidate, FontSpec, HarvestResult, LetterReading};
use acidtrip_core::{Clip, Grid, Palette};
use acidtrip_io::fonts::{FontLibrary, TextRenderOptions};
use acidtrip_io::stencils::StencilLibrary;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Act, Piece, Shared, coverage_line, credit_spans_of, ellipsize};
use crate::app::Level;
use crate::ui::canvas::rgb;
use crate::ui::widgets::{Btn, BtnKind, Buttons, LineInput, btn, theme};

/// Width of the panel beside the art.
const PANEL_W: u16 = 34;
const UNDO_MAX: usize = 64;

/// How a drag selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tool {
    /// Draw around the letter, any shape; a click takes the shape under it.
    Lasso,
    /// Brush over the cells.
    Paint,
    /// Click: the connected shape under it.
    Wand,
    /// Drag a rectangle.
    Box,
}

impl Tool {
    const ALL: [Tool; 4] = [Tool::Lasso, Tool::Paint, Tool::Wand, Tool::Box];

    fn key(self) -> &'static str {
        match self {
            Tool::Lasso => "l",
            Tool::Paint => "p",
            Tool::Wand => "w",
            Tool::Box => "b",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tool::Lasso => "lasso",
            Tool::Paint => "paint",
            Tool::Wand => "wand",
            Tool::Box => "box",
        }
    }
}

/// What a selection gesture does to the selection there already is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    New,
    Add,
    Remove,
}

/// Where an action sends the studio.
pub(crate) enum Go {
    Stay,
    Back,
    Fonts,
}

/// Buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Do {
    Save,
    Stencil,
    Read,
    Undo,
    Tool(Tool),
    Mode(Mode),
    /// Select a letter to change its shape or what it is.
    Edit(usize),
    /// Delete the letter being edited.
    Drop,
    /// Deselect.
    Clear,
    Back,
    Fonts,
    /// Put the letters into one of the artist's fonts (by style).
    Style(usize),
}

/// A letter cut from the piece: its char and cells (indices into the grid).
#[derive(Clone, Debug, PartialEq)]
struct Letter {
    ch: char,
    cells: Vec<usize>,
}

/// A selection gesture in progress.
struct Drag {
    mode: Mode,
    /// The selection before the gesture.
    base: Vec<bool>,
    /// Grid cells the pointer went through (may lie outside the piece).
    path: Vec<(i32, i32)>,
}

struct Committed {
    file: PathBuf,
    glyphs: String,
    stencil: Option<String>,
    samples: Vec<(String, Result<Clip, String>)>,
}

type Snapshot = (Vec<bool>, Vec<Letter>, Option<usize>);

pub(crate) struct CutStage {
    pub piece: Piece,
    /// The logo it was opened on (saved as a stencil with the first cut).
    pub cand: Option<Candidate>,
    /// Cells that are art, not backdrop.
    ink: Vec<bool>,
    sel: Vec<bool>,
    letters: Vec<Letter>,
    /// The letter the selection is (changes to the selection change it).
    editing: Option<usize>,
    tool: Tool,
    mode: Mode,
    drag: Option<Drag>,
    style: LineInput,
    style_focus: bool,
    /// Style typed (or picked) by hand: Claude's reading doesn't replace it.
    style_set: bool,
    undo: Vec<Snapshot>,
    scroll: (usize, usize),
    /// Scroll this area into view on the next frame.
    want: Option<(usize, usize, usize, usize)>,
    /// Screen area of the art, the style field and the letter badges.
    view: Rect,
    style_area: Rect,
    badges: Vec<(u16, u16, usize)>,
    /// What Claude was asked to read.
    asked: Option<Candidate>,
    stencil_saved: bool,
    result: Option<Result<Committed, String>>,
    /// The letters as last saved (unchanged = nothing new to add).
    saved: Vec<Letter>,
    /// Unsaved letters you were warned about: leaving again drops them.
    warned: Vec<Letter>,
}

impl CutStage {
    pub fn new(piece: Piece, cand: Option<Candidate>, reading: Option<&LetterReading>, sh: &Shared) -> Self {
        // Keep adding to the artist's font when there's one.
        let style =
            artist_styles(&piece.attribution, sh).into_iter().next().map_or_else(|| "logo".to_string(), |(s, _)| s);
        let ink = harvest::backdrop(&piece.grid).into_iter().map(|b| !b).collect();
        let n = piece.grid.width * piece.grid.height;
        let want = cand.as_ref().map(|c| (c.x, c.y, c.w, c.h));
        let mut s = CutStage {
            piece,
            ink,
            sel: vec![false; n],
            letters: vec![],
            editing: None,
            tool: Tool::Lasso,
            mode: Mode::New,
            drag: None,
            style: LineInput::new(&style),
            style_focus: false,
            style_set: false,
            undo: vec![],
            scroll: (0, 0),
            want,
            view: Rect::default(),
            style_area: Rect::default(),
            badges: vec![],
            asked: cand.clone(),
            cand,
            stencil_saved: false,
            result: None,
            saved: vec![],
            warned: vec![],
        };
        if let (Some(r), Some(id)) = (reading, s.cand.as_ref().map(|c| c.id.clone())) {
            s.apply_reading(&id, r);
        }
        s
    }

    /// Letters cut since the last save that you haven't been warned about.
    pub fn unsaved(&self) -> usize {
        if self.letters == self.saved || self.letters == self.warned {
            return 0;
        }
        self.letters.iter().filter(|l| !self.saved.contains(l)).count()
    }

    /// Back in the cutter: leaving warns again.
    pub fn rearm(&mut self) {
        self.warned.clear();
    }

    /// Leaving with unsaved letters warns once (true); leaving again drops them.
    pub fn warn_unsaved(&mut self, sh: &mut Shared, again: &str) -> bool {
        let n = self.unsaved();
        if n == 0 {
            return false;
        }
        self.warned = self.letters.clone();
        let s = if n == 1 { "" } else { "s" };
        sh.say(format!("{n} letter{s} not saved — ⏎ saves the font, {again} drops them"), Level::Warn);
        true
    }

    pub fn committed(&self) -> bool {
        matches!(self.result, Some(Ok(_)))
    }

    /// The style the letters go in with (names the font file).
    pub fn style(&self) -> &str {
        let s = self.style.text.trim();
        if s.is_empty() { "logo" } else { s }
    }

    fn w(&self) -> usize {
        self.piece.grid.width
    }

    fn h(&self) -> usize {
        self.piece.grid.height
    }

    fn has_sel(&self) -> bool {
        self.sel.iter().any(|&s| s)
    }

    fn selected(&self) -> Vec<usize> {
        (0..self.sel.len()).filter(|&i| self.sel[i]).collect()
    }

    fn snapshot(&mut self) {
        self.undo.push((self.sel.clone(), self.letters.clone(), self.editing));
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
    }

    /// The selection changed: a letter being edited follows it.
    fn sel_changed(&mut self) {
        if let Some(i) = self.editing {
            self.letters[i].cells = self.selected();
        }
    }

    // ------------------------------------------------------------ actions

    pub fn buttons(&self, sh: &Shared) -> Vec<Btn<'static, Do>> {
        let unsaved = self.letters != self.saved;
        let mut v = vec![
            btn(Do::Save, "⏎", if self.committed() && !unsaved { "saved" } else { "save font" })
                .primary()
                .enabled(!self.letters.is_empty() && unsaved),
            btn(Do::Stencil, "^S", "→ stencil").enabled(self.has_sel() || self.cand.is_some()),
        ];
        if sh.agent.is_some() {
            v.push(btn(Do::Read, "^R", if self.has_sel() { "Claude reads selection" } else { "Claude reads it" }));
        }
        v.push(btn(Do::Undo, "^Z", "undo").enabled(!self.undo.is_empty()));
        if self.editing.is_some() {
            v.push(btn(Do::Drop, "del", "delete letter"));
        }
        if let Some(f) = sh.font_for(&self.piece.attribution, self.style()) {
            v.push(btn(Do::Fonts, "★", format!("font {}/62", alnum(&f.spec))));
        }
        v.push(btn(Do::Back, "esc", if self.has_sel() || self.editing.is_some() { "deselect" } else { "back" }));
        v
    }

    pub fn act(&mut self, d: Do, sh: &mut Shared) -> Go {
        match d {
            Do::Save => self.save(sh),
            Do::Stencil => self.stencil(sh),
            Do::Read => self.read(sh),
            Do::Undo => {
                if let Some((sel, letters, editing)) = self.undo.pop() {
                    (self.sel, self.letters, self.editing) = (sel, letters, editing);
                }
            }
            Do::Tool(t) => self.tool = t,
            Do::Mode(m) => self.mode = m,
            Do::Edit(i) => self.edit(i),
            Do::Drop => {
                if let Some(i) = self.editing.take() {
                    self.snapshot();
                    let l = self.letters.remove(i);
                    self.sel.fill(false);
                    sh.say(format!("deleted \u{201c}{}\u{201d}", l.ch), Level::Info);
                }
            }
            Do::Clear => {
                self.editing = None;
                self.sel.fill(false);
            }
            Do::Back if self.has_sel() || self.editing.is_some() => return self.act(Do::Clear, sh),
            Do::Back if self.warn_unsaved(sh, "Esc again") => {}
            Do::Back => return Go::Back,
            Do::Fonts => return Go::Fonts,
            Do::Style(i) => {
                if let Some((s, _)) = artist_styles(&self.piece.attribution, sh).into_iter().nth(i) {
                    self.style = LineInput::new(&s);
                    self.style_set = true;
                }
            }
        }
        Go::Stay
    }

    /// Select letter `i` to change it.
    fn edit(&mut self, i: usize) {
        let Some(l) = self.letters.get(i) else { return };
        self.sel.fill(false);
        for &c in &l.cells {
            self.sel[c] = true;
        }
        self.editing = Some(i);
        let (x, y, w, h) = bbox(&l.cells, self.w()).unwrap_or_default();
        self.want = Some((x, y, w, h));
    }

    /// The selection is `ch`: a new letter, or the one being edited renamed.
    /// A letter already cut as `ch` is replaced.
    fn assign(&mut self, ch: char, sh: &mut Shared) {
        if ch.is_whitespace() || ch.is_control() {
            return;
        }
        self.snapshot();
        let cells = self.selected();
        let keep = self.editing;
        let replaced = self.letters.iter().enumerate().any(|(i, l)| l.ch == ch && Some(i) != keep);
        let mut i = 0;
        self.letters.retain(|l| {
            let drop = l.ch == ch && Some(i) != keep;
            i += 1;
            !drop
        });
        match keep {
            Some(k) => {
                // Removing an earlier letter shifts the edited one down.
                let k = self.letters.iter().position(|l| l.cells == cells).unwrap_or(k.min(self.letters.len() - 1));
                self.letters[k] = Letter { ch, cells: cells.clone() };
            }
            None => self.letters.push(Letter { ch, cells: cells.clone() }),
        }
        self.editing = None;
        self.sel.fill(false);
        let (_, _, w, h) = bbox(&cells, self.w()).unwrap_or_default();
        let was = if replaced { " (replaces the one cut before)" } else { "" };
        sh.say(format!("\u{201c}{ch}\u{201d} is {w}×{h}{was} — select the next letter, or ⏎ to save the font"), Level::Ok);
    }

    /// Letters as glyphs, sitting on the baseline their neighbours share.
    fn glyphs(&self) -> Vec<(char, Clip, Option<usize>)> {
        glyphs(&self.piece.grid, &self.ink, &self.letters)
    }

    fn save(&mut self, sh: &mut Shared) {
        let glyphs = self.glyphs();
        if glyphs.is_empty() {
            sh.say("select a letter (lasso around it) and type what it is first", Level::Warn);
            return;
        }
        let a = self.piece.attribution.clone();
        let style = self.style().to_string();
        let mut spec = FontSpec { name: format!("{} {style}", a.owner()), style: style.clone(), spacing: 1, ..Default::default() };
        for (ch, clip, base) in glyphs {
            spec.insert(ch, clip, base);
        }
        let mut res = HarvestResult { fonts: vec![(spec, a)], ..Default::default() };
        if !self.stencil_saved
            && let Some(c) = &self.cand
        {
            let text = self.spelled((c.x, c.y, c.w, c.h));
            res.stencils.push(harvest::stencil_for(c, &reading(&text, &style)));
        }
        let paths = sh.paths.clone();
        let _ = std::fs::create_dir_all(paths.stencils_dir());
        let mut lib = StencilLibrary::load(&paths.stencils_dir());
        let rep = match harvest::write(&res, &paths.fonts_dir(), &mut lib, &paths.stencils_dir()) {
            Ok(r) => r,
            Err(e) => {
                self.result = Some(Err(format!("{e:#}")));
                sh.say("writing the font failed", Level::Error);
                return;
            }
        };
        sh.wrote = true;
        sh.reload_fonts();
        let stencil = res.stencils.first().map(|s| s.meta.name.clone());
        if stencil.is_some() {
            self.stencil_saved = true;
        }
        let file = rep.font_files.first().cloned().unwrap_or_default();
        let fonts = FontLibrary::load(Some(&paths.fonts_dir()));
        let info = fonts.list().into_iter().find(|i| i.path.as_deref() == Some(file.as_path()));
        let glyphs = info.as_ref().map(|i| i.charset.trim().to_string()).unwrap_or_default();
        // New words first: they prove the font types more than its source logo.
        let mut words = new_words(&glyphs);
        if "ACiD".chars().all(|c| glyphs.contains(c)) {
            words.push("ACiD".into());
        }
        words.push(self.letters.iter().map(|l| l.ch).collect());
        let samples = words
            .into_iter()
            .map(|w| {
                let r = match &info {
                    Some(i) => fonts.render(&i.id, &w, &TextRenderOptions::default()).map_err(|e| format!("{e:#}")),
                    None => Err("font not found after writing".into()),
                };
                (w, r)
            })
            .collect();
        sh.say(
            format!("font {} now has {} letters", file.file_name().unwrap_or_default().to_string_lossy(), glyphs.len()),
            Level::Ok,
        );
        self.result = Some(Ok(Committed { file, glyphs, stencil, samples }));
        self.saved = self.letters.clone();
    }

    /// The letters inside an area, left to right: what it spells.
    fn spelled(&self, (x, y, w, h): (usize, usize, usize, usize)) -> String {
        let mut v: Vec<(usize, char)> = self
            .letters
            .iter()
            .filter_map(|l| {
                let (lx, ly, lw, lh) = bbox(&l.cells, self.w())?;
                (lx < x + w && lx + lw > x && ly < y + h && ly + lh > y).then_some((lx, l.ch))
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// A candidate of the selected cells (or the logo the cutter opened on).
    fn selection_candidate(&self) -> Option<Candidate> {
        if !self.has_sel() {
            return self.cand.clone();
        }
        let cells = self.selected();
        let (x, y, w, h) = bbox(&cells, self.w())?;
        let mut clip = Clip::new(w, h);
        for &i in &cells {
            let (cx, cy) = (i % self.w(), i / self.w());
            clip.set(cx - x, cy - y, Some(self.piece.grid.get(cx, cy)));
        }
        let stem = if self.piece.attribution.file.is_empty() { "piece" } else { &self.piece.attribution.file };
        Some(Candidate {
            id: format!("{stem}@{x},{y}+{w}x{h}"),
            x,
            y,
            w,
            h,
            score: 1.0,
            clip,
            attribution: self.piece.attribution.clone(),
        })
    }

    /// The part of the piece in view, as a candidate (for Claude to read).
    fn view_candidate(&self) -> Option<Candidate> {
        let (x, y) = self.scroll;
        let w = (self.view.width as usize).min(self.w().saturating_sub(x));
        let h = (self.view.height as usize).min(self.h().saturating_sub(y));
        if w == 0 || h == 0 {
            return None;
        }
        let mut clip = Clip::new(w, h);
        for cy in 0..h {
            for cx in 0..w {
                if self.ink[(y + cy) * self.w() + x + cx] {
                    clip.set(cx, cy, Some(self.piece.grid.get(x + cx, y + cy)));
                }
            }
        }
        Some(Candidate {
            id: format!("{}@{x},{y}+{w}x{h}", self.piece.attribution.file),
            x,
            y,
            w,
            h,
            score: 1.0,
            clip,
            attribution: self.piece.attribution.clone(),
        })
    }

    fn stencil(&mut self, sh: &mut Shared) {
        let Some(c) = self.selection_candidate() else {
            return sh.say("select what the stencil is first", Level::Warn);
        };
        let text = self.spelled((c.x, c.y, c.w, c.h));
        let st = harvest::stencil_for(&c, &reading(&text, self.style()));
        let dir = sh.paths.stencils_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut lib = StencilLibrary::load(&dir);
        match lib.save(&dir, st) {
            Ok(m) => {
                sh.wrote = true;
                if self.cand.as_ref().is_some_and(|k| k.id == c.id) {
                    self.stencil_saved = true;
                }
                sh.say(format!("stencil \u{201c}{}\u{201d} saved ({}×{})", m.name, c.w, c.h), Level::Ok);
            }
            Err(e) => sh.say(format!("saving the stencil: {e:#}"), Level::Error),
        }
    }

    fn read(&mut self, sh: &mut Shared) {
        let Some(c) = self.selection_candidate().or_else(|| self.view_candidate()) else { return };
        sh.read(&c);
        self.asked = Some(c);
    }

    /// Whether Claude's reading `id` is for this cutter.
    pub fn reads(&self, id: &str) -> bool {
        self.asked.as_ref().is_some_and(|c| c.id == id)
    }

    /// Letters from Claude's reading of what it was asked: each span's ink
    /// becomes a letter (replacing one already cut as that char).
    pub fn apply_reading(&mut self, id: &str, r: &LetterReading) {
        let Some(c) = self.asked.clone().filter(|c| c.id == id) else { return };
        let r = harvest::validate_reading(r.clone(), c.w);
        if r.letters.is_empty() {
            return;
        }
        self.snapshot();
        for l in &r.letters {
            if l.ch.is_whitespace() || l.ch.is_control() {
                continue;
            }
            let mut cells = vec![];
            for y in 0..c.h {
                for x in l.x0..=l.x1.min(c.w - 1) {
                    if c.clip.get(x, y).is_some_and(|cell| !cell.is_blank()) {
                        let i = (c.y + y) * self.w() + c.x + x;
                        if self.ink[i] {
                            cells.push(i);
                        }
                    }
                }
            }
            if cells.is_empty() {
                continue;
            }
            cells.sort_unstable();
            self.letters.retain(|k| k.ch != l.ch);
            self.letters.push(Letter { ch: l.ch, cells });
        }
        if !self.style_set && !r.style.is_empty() && r.style != "not-text" {
            self.style = LineInput::new(&r.style);
        }
        self.editing = None;
        self.sel.fill(false);
    }

    // ------------------------------------------------------------ input

    /// Handle a key: typing names the selection, the rest become buttons.
    pub fn key(&mut self, k: &KeyEvent, sh: &mut Shared) -> Option<Do> {
        if self.style_focus {
            match k.code {
                KeyCode::Enter => return Some(Do::Save),
                KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.style_focus = false,
                _ => {
                    if self.style.key(k) {
                        self.style_set = true;
                    }
                }
            }
            return None;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let big = if k.modifiers.contains(KeyModifiers::SHIFT) { 8 } else { 1 };
        let page = (self.view.height as usize).max(2) - 1;
        match k.code {
            KeyCode::Char('s') if ctrl => return Some(Do::Stencil),
            KeyCode::Char('r') if ctrl => return Some(Do::Read),
            KeyCode::Char('z') if ctrl => return Some(Do::Undo),
            _ if ctrl => {}
            KeyCode::Enter => return Some(Do::Save),
            KeyCode::Tab => self.style_focus = true,
            KeyCode::Esc => return Some(Do::Back),
            KeyCode::Delete | KeyCode::Backspace if self.editing.is_some() => return Some(Do::Drop),
            KeyCode::Delete | KeyCode::Backspace => return Some(Do::Clear),
            KeyCode::Up => self.scroll_by(0, -big),
            KeyCode::Down => self.scroll_by(0, big),
            KeyCode::Left => self.scroll_by(-big, 0),
            KeyCode::Right => self.scroll_by(big, 0),
            KeyCode::PageUp => self.scroll_by(0, -(page as isize)),
            KeyCode::PageDown => self.scroll_by(0, page as isize),
            KeyCode::Home => self.scroll = (0, 0),
            KeyCode::End => self.scroll.1 = self.h(),
            KeyCode::Char(c) if self.has_sel() => self.assign(c, sh),
            KeyCode::Char(c) => {
                if let Some(t) = Tool::ALL.into_iter().find(|t| t.key().starts_with(c)) {
                    return Some(Do::Tool(t));
                }
                match c {
                    '=' => return Some(Do::Mode(Mode::New)),
                    '+' => return Some(Do::Mode(Mode::Add)),
                    '-' => return Some(Do::Mode(Mode::Remove)),
                    'a' => return Some(Do::Read),
                    's' => return Some(Do::Stencil),
                    'u' => return Some(Do::Undo),
                    _ => sh.say("select a letter first (lasso around it), then type what it is", Level::Info),
                }
            }
            _ => {}
        }
        None
    }

    pub fn paste(&mut self, s: &str) {
        if self.style_focus {
            self.style.paste(s.trim());
            self.style_set = true;
        }
    }

    fn scroll_by(&mut self, dx: isize, dy: isize) {
        self.scroll.0 = self.scroll.0.saturating_add_signed(dx);
        self.scroll.1 = self.scroll.1.saturating_add_signed(dy);
    }

    /// The grid cell under a screen position (outside the piece too).
    fn cell_at(&self, col: u16, row: u16) -> (i32, i32) {
        (
            col as i32 - self.view.x as i32 + self.scroll.0 as i32,
            row as i32 - self.view.y as i32 + self.scroll.1 as i32,
        )
    }

    fn in_view(&self, col: u16, row: u16) -> bool {
        col >= self.view.x && col < self.view.right() && row >= self.view.y && row < self.view.bottom()
    }

    pub fn mouse(&mut self, m: MouseEvent) {
        let shift = m.modifiers.contains(KeyModifiers::SHIFT);
        let alt = m.modifiers.contains(KeyModifiers::ALT);
        let on = |r: Rect| m.row == r.y && m.column >= r.x && m.column < r.right();
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if on(self.style_area) => self.style_focus = true,
            MouseEventKind::Down(MouseButton::Left)
                if let Some(&(_, _, i)) = self.badges.iter().find(|(x, y, _)| *x == m.column && *y == m.row) =>
            {
                self.style_focus = false;
                self.edit(i);
            }
            MouseEventKind::Down(b) if self.in_view(m.column, m.row) => {
                self.style_focus = false;
                // Right button, or Alt, takes away; Shift adds.
                let mode = if b == MouseButton::Right || alt {
                    Mode::Remove
                } else if shift {
                    Mode::Add
                } else {
                    self.mode
                };
                self.snapshot();
                let p = self.cell_at(m.column, m.row);
                let drag = Drag { mode, base: self.sel.clone(), path: vec![p] };
                if self.tool == Tool::Wand {
                    let region = self.shape_at(p);
                    self.apply(&drag, &region);
                } else {
                    self.drag = Some(drag);
                    self.drag_moved();
                }
            }
            MouseEventKind::Drag(_) if self.drag.is_some() => {
                let p = self.cell_at(m.column, m.row);
                if let Some(d) = &mut self.drag
                    && d.path.last() != Some(&p)
                {
                    let from = *d.path.last().unwrap_or(&p);
                    d.path.extend(line(from, p).into_iter().skip(1));
                    self.drag_moved();
                }
            }
            MouseEventKind::Up(_) => {
                if let Some(d) = self.drag.take() {
                    let region = match self.tool {
                        Tool::Lasso if d.path.iter().all(|&p| p == d.path[0]) => self.shape_at(d.path[0]),
                        Tool::Lasso => self.lasso(&d.path),
                        _ => self.region(&d),
                    };
                    self.apply(&d, &region);
                    if self.undo.last().is_some_and(|(s, _, _)| *s == self.sel) {
                        self.undo.pop();
                    }
                }
            }
            MouseEventKind::ScrollDown if shift => self.scroll_by(4, 0),
            MouseEventKind::ScrollUp if shift => self.scroll_by(-4, 0),
            MouseEventKind::ScrollDown => self.scroll_by(0, 3),
            MouseEventKind::ScrollUp => self.scroll_by(0, -3),
            MouseEventKind::ScrollRight => self.scroll_by(4, 0),
            MouseEventKind::ScrollLeft => self.scroll_by(-4, 0),
            _ => {}
        }
    }

    /// Paint and box show their result while dragging.
    fn drag_moved(&mut self) {
        if let Some(d) = self.drag.take() {
            if matches!(self.tool, Tool::Paint | Tool::Box) {
                let region = self.region(&d);
                self.apply(&d, &region);
            }
            self.drag = Some(d);
        }
    }

    /// Cells a paint or box gesture covers.
    fn region(&self, d: &Drag) -> Vec<usize> {
        let pts: Vec<(i32, i32)> = match self.tool {
            Tool::Box => {
                let (a, b) = (d.path[0], *d.path.last().unwrap_or(&d.path[0]));
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).collect()
            }
            _ => d.path.clone(),
        };
        pts.into_iter().filter_map(|p| self.index(p)).filter(|&i| self.ink[i]).collect()
    }

    /// Cells inside a lasso (their centers inside the drawn shape) and on it.
    fn lasso(&self, path: &[(i32, i32)]) -> Vec<usize> {
        let mut out: Vec<usize> = path.iter().filter_map(|&p| self.index(p)).collect();
        let x0 = path.iter().map(|p| p.0).min().unwrap_or(0).max(0) as usize;
        let x1 = path.iter().map(|p| p.0).max().unwrap_or(0).max(0) as usize;
        let y0 = path.iter().map(|p| p.1).min().unwrap_or(0).max(0) as usize;
        let y1 = path.iter().map(|p| p.1).max().unwrap_or(0).max(0) as usize;
        for y in y0..=y1.min(self.h().saturating_sub(1)) {
            for x in x0..=x1.min(self.w().saturating_sub(1)) {
                if inside(path, x as f32, y as f32) {
                    out.push(y * self.w() + x);
                }
            }
        }
        out.retain(|&i| self.ink[i]);
        out
    }

    /// The connected shape of art under a cell.
    fn shape_at(&self, p: (i32, i32)) -> Vec<usize> {
        let Some(start) = self.index(p).filter(|&i| self.ink[i]) else { return vec![] };
        let w = self.w();
        let mut seen = vec![false; self.ink.len()];
        let mut stack = vec![start];
        let mut out = vec![];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            out.push(i);
            let (x, y) = (i % w, i / w);
            let mut near = vec![];
            if x > 0 {
                near.push(i - 1);
            }
            if x + 1 < w {
                near.push(i + 1);
            }
            if y > 0 {
                near.push(i - w);
            }
            if y + 1 < self.h() {
                near.push(i + w);
            }
            for n in near {
                if self.ink[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        out
    }

    fn index(&self, (x, y): (i32, i32)) -> Option<usize> {
        (x >= 0 && y >= 0 && (x as usize) < self.w() && (y as usize) < self.h())
            .then(|| y as usize * self.w() + x as usize)
    }

    /// The selection becomes the gesture's base combined with `region`.
    fn apply(&mut self, d: &Drag, region: &[usize]) {
        self.sel = match d.mode {
            Mode::New => vec![false; d.base.len()],
            _ => d.base.clone(),
        };
        for &i in region {
            self.sel[i] = d.mode != Mode::Remove;
        }
        self.sel_changed();
    }

    /// Select the shape at a cell of the piece, as a wand click would.
    #[cfg(test)]
    pub fn pick(&mut self, x: usize, y: usize) {
        let region = self.shape_at((x as i32, y as i32));
        self.apply(&Drag { mode: Mode::New, base: self.sel.clone(), path: vec![] }, &region);
    }

    // ------------------------------------------------------------ drawing

    pub fn draw(&mut self, f: &mut Frame, area: Rect, sh: &Shared, btns: &mut Buttons<Act>) {
        let dim = Style::new().fg(theme::DIM);
        let a = &self.piece.attribution;
        let mut credit = vec![Span::styled("Cutting ", dim)];
        credit.extend(credit_spans_of(a));
        credit.push(Span::styled(format!("  {}×{}", self.w(), self.h()), dim));

        // Tools and modes.
        let ty = area.y + 1;
        let mut x = area.x;
        for t in Tool::ALL {
            let kind = if t == self.tool { BtnKind::On } else { BtnKind::Normal };
            x += btns.draw(f.buffer_mut(), x, ty, area.right(), &btn(Act::Cut(Do::Tool(t)), t.key(), t.label()).kind(kind))
                + 1;
        }
        x += 2;
        for (m, key, label) in [(Mode::New, "=", "new"), (Mode::Add, "+", "add"), (Mode::Remove, "-", "take away")] {
            let kind = if m == self.mode { BtnKind::On } else { BtnKind::Normal };
            x += btns.draw(f.buffer_mut(), x, ty, area.right(), &btn(Act::Cut(Do::Mode(m)), key, label).kind(kind)) + 1;
        }
        let hint = match (self.editing, self.has_sel()) {
            (Some(i), _) => format!(
                "editing \u{201c}{}\u{201d}: reshape, type to rename, del deletes",
                self.letters[i].ch
            ),
            (None, true) => "now type the letter it is".to_string(),
            (None, false) => match self.tool {
                Tool::Lasso => "draw around a letter, or click it",
                Tool::Paint => "paint over a letter's cells",
                Tool::Wand => "click a letter's shape",
                Tool::Box => "drag a box over a letter",
            }
            .to_string(),
        };
        let hint_st = if self.has_sel() { Style::new().fg(theme::WARN).add_modifier(Modifier::BOLD) } else { dim };
        if x + 4 < area.right() {
            f.render_widget(
                Paragraph::new(Span::styled(ellipsize(&hint, (area.right() - x - 1) as usize), hint_st)),
                Rect::new(x + 1, ty, area.right() - x - 1, 1),
            );
        }

        // The art on the left, the letters panel on the right.
        let body_y = ty + 2;
        let body_h = area.bottom().saturating_sub(body_y);
        let panel_w = if area.width >= PANEL_W + 40 { PANEL_W } else { 0 };
        let art_w = area.width - panel_w - u16::from(panel_w > 0);
        self.view = Rect::new(area.x, body_y, art_w.min(self.w() as u16), body_h.min(self.h() as u16));
        self.fit_scroll();
        self.draw_art(f.buffer_mut());
        // Where the view sits in a piece bigger than it, beside the credit.
        let top = Rect::new(area.x, area.y, area.width - panel_w, 1);
        let mut credit_w = area.width;
        if self.view.width < art_w || self.scroll != (0, 0) || self.h() > self.view.height as usize {
            let (sx, sy) = self.scroll;
            let (vw, vh) = (self.view.width as usize, self.view.height as usize);
            let (rows, cols) = ((sy + 1, (sy + vh).min(self.h()), self.h()), (sx + 1, (sx + vw).min(self.w()), self.w()));
            let mut at = format!("rows {}–{} of {}", rows.0, rows.1, rows.2);
            if self.w() > vw {
                at = format!("cols {}–{} of {} · {at}", cols.0, cols.1, cols.2);
            }
            // Short, when the full form would crowd out the credit.
            if (at.chars().count() as u16) + 30 > top.width {
                at = format!("↕{}–{}/{}", rows.0, rows.1, rows.2);
                if self.w() > vw {
                    at = format!("↔{}–{}/{} {at}", cols.0, cols.1, cols.2);
                }
            }
            let at_w = at.chars().count() as u16 + 1;
            f.render_widget(Paragraph::new(Span::styled(at, dim)).right_aligned(), top);
            credit_w = top.width.saturating_sub(at_w + 1);
        }
        f.render_widget(Paragraph::new(Line::from(credit)), Rect::new(area.x, area.y, credit_w, 1));
        if panel_w > 0 {
            let p = Rect::new(area.right() - panel_w, body_y - 1, panel_w, body_h + 1);
            self.draw_panel(f, p, sh, btns);
        }
    }

    /// Keep the scroll inside the piece and the wanted area in view.
    fn fit_scroll(&mut self) {
        let (vw, vh) = (self.view.width as usize, self.view.height as usize);
        if let Some((x, y, w, h)) = self.want.take() {
            if x < self.scroll.0 || x + w > self.scroll.0 + vw {
                self.scroll.0 = (x + w / 2).saturating_sub(vw / 2);
            }
            if y < self.scroll.1 || y + h > self.scroll.1 + vh {
                self.scroll.1 = (y + h / 2).saturating_sub(vh / 2);
            }
        }
        self.scroll.0 = self.scroll.0.min(self.w().saturating_sub(vw));
        self.scroll.1 = self.scroll.1.min(self.h().saturating_sub(vh));
    }

    fn draw_art(&mut self, buf: &mut Buffer) {
        let pal = Palette::default();
        let (sx, sy) = self.scroll;
        let v = self.view;
        let path: &[(i32, i32)] = match (&self.drag, self.tool) {
            (Some(d), Tool::Lasso) => &d.path,
            _ => &[],
        };
        for y in 0..v.height as usize {
            for x in 0..v.width as usize {
                let (gx, gy) = (sx + x, sy + y);
                let cell = self.piece.grid.get(gx, gy);
                let (mut fg, mut bg) = (rgb(cell.fg, &pal), rgb(cell.bg, &pal));
                let ch = if cell.ch.is_control() || cell.ch == '\u{0}' { ' ' } else { cell.ch };
                if self.sel[gy * self.w() + gx] {
                    (fg, bg) = (tint(fg), tint(bg));
                }
                if let Some(c) = buf.cell_mut((v.x + x as u16, v.y + y as u16)) {
                    c.set_char(ch).set_style(Style::new().fg(fg).bg(bg));
                }
            }
        }
        // The lasso being drawn.
        for &(px, py) in path {
            let (x, y) = (px - sx as i32, py - sy as i32);
            if x >= 0
                && y >= 0
                && (x as u16) < v.width
                && (y as u16) < v.height
                && let Some(c) = buf.cell_mut((v.x + x as u16, v.y + y as u16))
            {
                c.set_char('•').set_fg(theme::ACCENT2);
            }
        }
        // A badge at each letter's top-left, clickable to edit it.
        self.badges.clear();
        for (i, l) in self.letters.iter().enumerate() {
            if Some(i) == self.editing {
                continue;
            }
            let Some((lx, ly, _, _)) = bbox(&l.cells, self.w()) else { continue };
            let by = if ly > sy { ly - 1 } else { ly };
            if lx < sx || by < sy || lx >= sx + v.width as usize || by >= sy + v.height as usize {
                continue;
            }
            let (mut cx, cy) = (v.x + (lx - sx) as u16, v.y + (by - sy) as u16);
            // Letters starting at the same spot line their badges up.
            while self.badges.iter().any(|&(x, y, _)| (x, y) == (cx, cy)) && cx + 1 < v.right() {
                cx += 1;
            }
            if let Some(c) = buf.cell_mut((cx, cy)) {
                c.set_char(l.ch).set_style(Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD));
            }
            self.badges.push((cx, cy, i));
        }
    }

    fn draw_panel(&mut self, f: &mut Frame, p: Rect, sh: &Shared, btns: &mut Buttons<Act>) {
        let dim = Style::new().fg(theme::DIM);
        let head = |s: String| Line::from(Span::styled(s, dim.add_modifier(Modifier::BOLD)));
        let mut y = p.y;
        let x = p.x + 1;
        let w = p.width - 1;
        for yy in p.y..p.bottom() {
            if let Some(c) = f.buffer_mut().cell_mut((p.x, yy)) {
                c.set_char('│').set_fg(theme::BORDER);
            }
        }
        // Letters cut so far, each a button to edit it.
        f.render_widget(Paragraph::new(head(format!("LETTERS {}", self.letters.len()))), Rect::new(x, y, w, 1));
        y += 1;
        if self.letters.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled("none yet: select one, type it", dim)),
                Rect::new(x, y, w, 1),
            );
            y += 1;
        } else {
            let mut cx = x;
            let glyphs = self.glyphs();
            for (i, l) in self.letters.iter().enumerate() {
                let size = glyphs.iter().find(|g| g.0 == l.ch).map_or(String::new(), |g| format!("{}×{}", g.1.width, g.1.height));
                let kind = if Some(i) == self.editing { BtnKind::On } else { BtnKind::Normal };
                let b = btn(Act::Cut(Do::Edit(i)), "", format!("{} {size}", l.ch)).kind(kind);
                if cx > x && cx + b.width() > p.right() {
                    cx = x;
                    y += 1;
                }
                if y + 1 >= p.bottom() {
                    break;
                }
                cx += btns.draw(f.buffer_mut(), cx, y, p.right(), &b) + 1;
            }
            y += 1;
        }
        y += 1;

        // What's selected, as the glyph it would be.
        let preview = self.has_sel().then(|| {
            let cells = self.selected();
            glyphs(&self.piece.grid, &self.ink, &[Letter { ch: '?', cells }]).pop()
        });
        if let Some(Some((_, clip, _))) = preview
            && y + 2 < p.bottom()
        {
            let title = match self.editing {
                Some(i) => format!("EDITING \u{201c}{}\u{201d}  {}×{}", self.letters[i].ch, clip.width, clip.height),
                None => format!("SELECTED  {}×{}", clip.width, clip.height),
            };
            f.render_widget(Paragraph::new(head(title)), Rect::new(x, y, w, 1));
            y += 1;
            let ph = (clip.height as u16).min(10).min(p.bottom() - y);
            crate::ui::canvas::draw_clip(f.buffer_mut(), Rect::new(x, y, w, ph), &clip, &Palette::default());
            y += ph + 1;
        }

        // The font the letters go into: what it has, what saving adds.
        if y + 4 >= p.bottom() {
            return;
        }
        let font = sh.font_for(&self.piece.attribution, self.style());
        let name = font.map_or_else(
            || format!("{} (new)", harvest::font_file_name(&self.piece.attribution, self.style()).trim_end_matches(".acidfont")),
            |f| f.title(),
        );
        f.render_widget(Paragraph::new(head("INTO".into())), Rect::new(x, y, w, 1));
        f.render_widget(
            Paragraph::new(Span::styled(ellipsize(&name, w as usize), Style::new().fg(theme::ACCENT2))),
            Rect::new(x, y + 1, w, 1),
        );
        let adding: String = self
            .letters
            .iter()
            .filter(|l| !self.saved.contains(l))
            .map(|l| l.ch.to_ascii_uppercase())
            .collect();
        f.render_widget(Paragraph::new(coverage_line(font.map(|f| &f.spec), &adding, 0)), Rect::new(x, y + 2, w, 1));
        self.style_area = Rect::new(x, y + 3, w.min(24), 1);
        self.style.render(f, self.style_area, "Style › ", self.style_focus);
        y += 4;
        // The artist's other fonts: one click adds to one of them.
        let styles = artist_styles(&self.piece.attribution, sh);
        if styles.len() > 1 || styles.first().is_some_and(|(s, _)| !s.eq_ignore_ascii_case(self.style())) {
            let mut cx = x;
            for (i, (st, n)) in styles.iter().enumerate() {
                let kind = if st.eq_ignore_ascii_case(self.style()) { BtnKind::On } else { BtnKind::Normal };
                let dw = btns.draw(f.buffer_mut(), cx, y, p.right(), &btn(Act::Cut(Do::Style(i)), "", format!("{st} {n}")).kind(kind));
                if dw == 0 {
                    break;
                }
                cx += dw + 1;
            }
            y += 1;
        }
        y += 1;
        if y < p.bottom() {
            self.draw_result(f, Rect::new(x, y, w, p.bottom() - y));
        }
    }

    fn draw_result(&self, f: &mut Frame, r: Rect) {
        let dim = Style::new().fg(theme::DIM);
        match &self.result {
            None => {
                let text = "Any size, any shape: a letter is exactly what you select. Letters may overlap; \
                            each keeps only its own cells. Shift adds to a selection, alt or the right \
                            button takes away. Enter saves them into the font.";
                f.render_widget(Paragraph::new(Span::styled(text, dim)).wrap(Wrap { trim: false }), r);
            }
            Some(Err(e)) => {
                f.render_widget(
                    Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::ERR))).wrap(Wrap { trim: false }),
                    r,
                );
            }
            Some(Ok(done)) => {
                let mut lines = vec![Line::from(vec![
                    Span::styled("✓ ", Style::new().fg(theme::OK).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        done.file.file_name().unwrap_or_default().to_string_lossy().to_string(),
                        Style::new().fg(theme::OK).add_modifier(Modifier::BOLD),
                    ),
                ])];
                lines.push(Line::from(Span::styled(
                    format!("{} letters: {}", done.glyphs.chars().count(), ellipsize(&done.glyphs, 20)),
                    dim,
                )));
                if let Some(s) = &done.stencil {
                    lines.push(Line::from(Span::styled(format!("stencil \u{201c}{s}\u{201d}"), dim)));
                }
                let mut y = r.y + lines.len() as u16;
                f.render_widget(Paragraph::new(lines), r);
                for (word, clip) in &done.samples {
                    let Ok(clip) = clip else { continue };
                    if clip.width == 0 || y + 2 >= r.bottom() {
                        continue;
                    }
                    f.render_widget(
                        Paragraph::new(Span::styled(format!("\u{201c}{word}\u{201d}"), dim)),
                        Rect::new(r.x, y, r.width, 1),
                    );
                    let ph = (clip.height as u16).min(r.bottom() - y - 1);
                    crate::ui::canvas::draw_clip(f.buffer_mut(), Rect::new(r.x, y + 1, r.width, ph), clip, &Palette::default());
                    y += ph + 2;
                }
            }
        }
    }
}

/// A selected cell's color: pulled toward the accent.
fn tint(c: Color) -> Color {
    let Color::Rgb(r, g, b) = c else { return c };
    let Color::Rgb(ar, ag, ab) = theme::ACCENT else { return c };
    let mix = |v: u8, a: u8| ((v as u16 * 45 + a as u16 * 55) / 100) as u8;
    Color::Rgb(mix(r, ar), mix(g, ag), mix(b, ab))
}

/// The bounding box (x, y, w, h) of grid cells.
fn bbox(cells: &[usize], w: usize) -> Option<(usize, usize, usize, usize)> {
    let xs = cells.iter().map(|i| i % w);
    let ys = cells.iter().map(|i| i / w);
    let (x0, x1) = (xs.clone().min()?, xs.max()?);
    let (y0, y1) = (ys.clone().min()?, ys.max()?);
    Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// Cells on the straight line from `a` to `b` (Bresenham).
fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    let mut out = vec![(x, y)];
    while (x, y) != b {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        out.push((x, y));
    }
    out
}

/// Whether cell (x, y)'s center is inside the closed path (even-odd).
fn inside(path: &[(i32, i32)], x: f32, y: f32) -> bool {
    let mut odd = false;
    let n = path.len();
    for i in 0..n {
        let (xi, yi) = (path[i].0 as f32, path[i].1 as f32);
        let (xj, yj) = (path[(i + n - 1) % n].0 as f32, path[(i + n - 1) % n].1 as f32);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            odd = !odd;
        }
    }
    odd
}

/// Chars that don't sit on the baseline: they don't vote for where it is.
const OFF_BASELINE: &str = "gjpqyQ,;_-~^'\"`*°";

/// Each letter as a glyph: the bounding box of its cells (other cells in it
/// stay transparent), sitting on the baseline shared by the letters beside
/// it (the most common bottom row among the letters its rows overlap, the
/// lowest on a tie; descenders and marks don't count). A letter reaching
/// below it keeps that row as its base (a descender); one ending above it
/// gets blank rows down to it.
fn glyphs(grid: &Grid, ink: &[bool], letters: &[Letter]) -> Vec<(char, Clip, Option<usize>)> {
    let w = grid.width;
    let boxes: Vec<Option<(usize, usize, usize, usize)>> = letters
        .iter()
        .map(|l| {
            let cells: Vec<usize> = l.cells.iter().copied().filter(|&i| ink[i]).collect();
            bbox(&cells, w)
        })
        .collect();
    let mut out = vec![];
    for (l, b) in letters.iter().zip(&boxes) {
        let Some((x0, y0, bw, bh)) = *b else { continue };
        let bottom = y0 + bh - 1;
        let mut bottoms: Vec<usize> = letters
            .iter()
            .zip(&boxes)
            .filter(|(o, _)| !OFF_BASELINE.contains(o.ch))
            .filter_map(|(_, b)| *b)
            .filter(|(_, oy, _, oh)| *oy <= bottom && oy + oh > y0)
            .map(|(_, oy, _, oh)| oy + oh - 1)
            .collect();
        bottoms.sort_unstable();
        let mut base_row = bottom;
        let mut best = 0;
        for chunk in bottoms.chunk_by(|a, b| a == b) {
            if chunk.len() >= best {
                (best, base_row) = (chunk.len(), chunk[0]);
            }
        }
        let height = bottom.max(base_row) - y0 + 1;
        let mut clip = Clip::new(bw, height);
        for &i in &l.cells {
            if ink[i] {
                clip.set(i % w - x0, i / w - y0, Some(grid.get(i % w, i / w)));
            }
        }
        let base = (bottom > base_row).then(|| base_row - y0);
        out.push((l.ch, clip, base));
    }
    out
}

/// A reading of `text` (for naming stencils).
fn reading(text: &str, style: &str) -> LetterReading {
    LetterReading { text: text.to_string(), letters: vec![], style: style.to_string() }
}

/// The styles of the artist's harvested fonts with their letter counts,
/// biggest first.
fn artist_styles(a: &Attribution, sh: &Shared) -> Vec<(String, usize)> {
    let owner = a.owner();
    let mut v: Vec<(String, usize)> = sh
        .fonts
        .iter()
        .filter(|f| f.artists().first() == Some(&owner) && !f.spec.style.is_empty())
        .map(|f| (f.spec.style.clone(), alnum(&f.spec)))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Letters and digits a font has.
pub(crate) fn alnum(spec: &FontSpec) -> usize {
    spec.glyphs.keys().filter(|c| c.is_ascii_alphanumeric()).count()
}

/// Words from a small scene vocabulary that the cut glyphs can spell.
pub(crate) fn new_words(glyphs: &str) -> Vec<String> {
    const WORDS: [&str; 40] = [
        "scene", "ansi", "acid", "bbs", "elite", "demo", "warez", "code", "rad", "dark", "star", "bar", "bars", "arts",
        "sax", "abs", "crash", "brass", "sabre", "bass", "rax", "xs", "ice", "fire", "blade", "cyber", "dream", "zero",
        "night", "storm", "tribe", "neon", "glow", "wild", "void", "chaos", "lord", "pixel", "retro", "grab",
    ];
    let have = |w: &str| w.chars().all(|c| glyphs.contains(c) || glyphs.contains(c.to_ascii_uppercase()));
    WORDS.iter().filter(|w| have(w)).take(2).map(|w| w.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{Cell, Color as DocColor};

    fn grid(rows: &[&str]) -> Grid {
        let mut g = Grid::new(rows[0].chars().count(), rows.len());
        for (y, r) in rows.iter().enumerate() {
            for (x, c) in r.chars().enumerate() {
                if c != '.' {
                    g.set(x, y, Cell::new('█', DocColor::Pal(12), DocColor::BLACK));
                }
            }
        }
        g
    }

    #[test]
    fn lasso_takes_what_it_encloses_and_a_line_is_contiguous() {
        let l = line((0, 0), (5, 2));
        assert_eq!(l.first(), Some(&(0, 0)));
        assert_eq!(l.last(), Some(&(5, 2)));
        assert!(l.windows(2).all(|p| (p[0].0 - p[1].0).abs() <= 1 && (p[0].1 - p[1].1).abs() <= 1));
        let square = [(0, 0), (4, 0), (4, 4), (0, 4)];
        assert!(inside(&square, 2.0, 2.0));
        assert!(!inside(&square, 6.0, 2.0));
    }

    #[test]
    fn glyphs_share_a_baseline_and_keep_only_their_cells() {
        // "A" and a "g" whose tail hangs two rows under it, a "-" floating
        // mid-height, and an overlapping neighbour in the A's box.
        let g = grid(&[
            "###.......", //
            "#.#.##....", //
            "###.##.##.", //
            "#.#.##....", //
            "#.#.##....", //
            ".....#....", //
            "....##....", //
        ]);
        let ink: Vec<bool> = g.cells.iter().map(|c| !c.is_blank()).collect();
        let w = g.width;
        let cells = |pts: &[(usize, usize)]| pts.iter().map(|&(x, y)| y * w + x).collect::<Vec<_>>();
        let a: Vec<(usize, usize)> =
            (0..5).flat_map(|y| (0..3).map(move |x| (x, y))).filter(|&(x, y)| ink[y * w + x]).collect();
        let gl: Vec<(usize, usize)> =
            (1..7).flat_map(|y| (4..6).map(move |x| (x, y))).filter(|&(x, y)| ink[y * w + x]).collect();
        let letters = vec![
            Letter { ch: 'A', cells: cells(&a) },
            Letter { ch: 'g', cells: cells(&gl) },
            Letter { ch: '-', cells: cells(&[(7, 2), (8, 2)]) },
            Letter { ch: 'i', cells: cells(&[(1, 0), (1, 1)]) },
        ];
        let out = glyphs(&g, &ink, &letters);
        let get = |c: char| out.iter().find(|o| o.0 == c).unwrap();
        // A: 3×5 on the baseline (row 4).
        assert_eq!((get('A').1.width, get('A').1.height, get('A').2), (3, 5, None));
        // g: its base is row 4, three rows above its bottom.
        assert_eq!((get('g').1.height, get('g').2), (6, Some(3)));
        // "-" gets blank rows down to the baseline.
        assert_eq!((get('-').1.width, get('-').1.height, get('-').2), (2, 3, None));
        assert!(get('-').1.get(0, 2).is_none());
        // "i" took the A's top middle cell only; the A's hole stays a hole.
        assert!(get('A').1.get(1, 1).is_none());
        assert_eq!(get('i').1.width, 1);
    }
}
