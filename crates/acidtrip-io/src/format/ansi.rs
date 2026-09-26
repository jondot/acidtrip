//! ANSI (CP437 or UTF-8) load via `icy_parser_core`, and minimal-SGR save.

use std::fmt::Write as _;

use acidtrip_core::color::{ANSI_TO_VGA, VGA_TO_ANSI, xterm256};
use acidtrip_core::model::{DEFAULT_FPS, FIRST_FRAME};
use acidtrip_core::{Cell, Color, DocKind, Document, Frame, FrameSet, Grid, cp437};
use icy_parser_core::{
    AnsiParser, Blink, CommandParser, CommandSink, DecMode, Direction, EraseInDisplayMode, EraseInLineMode, Intensity,
    SgrAttribute, TerminalCommand,
};

use super::sauce::{self, CHAR_ANSI};
use super::{SaveOptions, char_byte, classic_grid, export_grid, finish, want_sauce};

/// Hard cap on rows a (possibly hostile) file can create.
const MAX_ROWS: usize = 20_000;
/// Width used for UTF-8 files without SAUCE (lines are not wrapped).
const UNBOUNDED: usize = 2_000;

/// A virtual ANSI.SYS screen: immediate wrap at `width`, grows downwards.
pub(crate) struct Screen {
    pub width: usize,
    rows: Vec<Vec<Cell>>,
    pub x: usize,
    pub y: usize,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub blink: bool,
    inverse: bool,
    conceal: bool,
    saved: (usize, usize),
    utf8: bool,
    pub ice_hint: bool,
    /// Anything seen yet (a leading form feed clears, later ones are ♀ glyphs).
    started: bool,
}

impl Screen {
    pub fn new(width: usize, utf8: bool) -> Self {
        Screen {
            width: width.max(1),
            rows: Vec::new(),
            x: 0,
            y: 0,
            fg: Color::LIGHT_GRAY,
            bg: Color::BLACK,
            bold: false,
            blink: false,
            inverse: false,
            conceal: false,
            saved: (0, 0),
            utf8,
            ice_hint: false,
            started: false,
        }
    }

    pub fn reset_attr(&mut self) {
        self.fg = Color::LIGHT_GRAY;
        self.bg = Color::BLACK;
        self.bold = false;
        self.blink = false;
        self.inverse = false;
        self.conceal = false;
    }

    /// Set colors from a DOS attribute byte (bit 7 = bright background).
    pub fn set_attr(&mut self, attr: u8) {
        self.reset_attr();
        self.fg = Color::Pal(attr & 15);
        self.bg = Color::Pal(attr >> 4);
    }

    fn pen(&self) -> Cell {
        let mut fg = match self.fg {
            Color::Pal(i) if self.bold && i < 8 => Color::Pal(i + 8),
            c => c,
        };
        let mut bg = match self.bg {
            Color::Pal(i) if self.blink && i < 8 => Color::Pal(i + 8),
            c => c,
        };
        if self.inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if self.conceal {
            fg = bg;
        }
        Cell::new(' ', fg, bg)
    }

    pub fn cell(&self, x: usize, y: usize) -> Cell {
        self.rows.get(y).and_then(|r| r.get(x)).copied().unwrap_or(Cell::BLANK)
    }

    pub fn set(&mut self, x: usize, y: usize, c: Cell) {
        if x >= self.width || y >= MAX_ROWS {
            return;
        }
        if self.rows.len() <= y {
            self.rows.resize_with(y + 1, Vec::new);
        }
        let row = &mut self.rows[y];
        if row.len() <= x {
            row.resize(x + 1, Cell::BLANK);
        }
        row[x] = c;
    }

    pub fn put(&mut self, ch: char) {
        let c = Cell { ch, ..self.pen() };
        self.set(self.x, self.y, c);
        self.x += 1;
        if self.x >= self.width {
            self.x = 0;
            self.y = (self.y + 1).min(MAX_ROWS);
        }
    }

    pub fn fill(&mut self, x0: usize, x1: usize, y: usize) {
        let c = self.pen();
        for x in x0..x1.min(self.width) {
            if y < self.rows.len() || c != Cell::BLANK {
                self.set(x, y, c);
            }
        }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.x = 0;
        self.y = 0;
    }

    pub fn goto(&mut self, x: usize, y: usize) {
        self.x = x.min(self.width - 1);
        self.y = y.min(MAX_ROWS);
    }

    fn newline(&mut self) {
        self.x = 0;
        self.y = (self.y + 1).min(MAX_ROWS);
    }

    /// Final grid: width as configured (or the widest line for unbounded
    /// UTF-8), height = rows used but at least `min_h`.
    pub fn into_grid(self, min_h: usize) -> Grid {
        let w = if self.width == UNBOUNDED {
            self.rows.iter().map(Vec::len).max().unwrap_or(0).max(80)
        } else {
            self.width
        };
        let mut g = Grid::new(w, self.rows.len().max(min_h));
        for (y, row) in self.rows.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                g.set(x, y, c);
            }
        }
        g
    }

    /// What's on screen now, as tall as the rows drawn.
    fn snapshot(&self) -> Grid {
        let w = if self.width == UNBOUNDED {
            self.rows.iter().map(Vec::len).max().unwrap_or(0).max(80)
        } else {
            self.width
        };
        let mut g = Grid::new(w, self.rows.len().max(1));
        for (y, row) in self.rows.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                g.set(x, y, c);
            }
        }
        g
    }
}

fn color(c: icy_parser_core::Color, default: Color) -> Color {
    use icy_parser_core::Color as C;
    match c {
        C::Base(i) => Color::Pal(i & 15),
        C::Extended(n) if n < 16 => Color::Pal(ANSI_TO_VGA[(n & 7) as usize] | (n & 8)),
        C::Extended(n) => {
            let [r, g, b] = xterm256(n);
            Color::Rgb(r, g, b)
        }
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Default => default,
    }
}

impl CommandSink for Screen {
    fn print(&mut self, text: &[u8]) {
        self.started = true;
        if self.utf8 {
            for ch in String::from_utf8_lossy(text).chars() {
                self.put(ch);
            }
        } else {
            for &b in text {
                self.put(cp437::to_char(b));
            }
        }
    }

    fn emit(&mut self, cmd: TerminalCommand) {
        use TerminalCommand::*;
        let n1 = |n: u16| n.max(1) as usize;
        let started = std::mem::replace(&mut self.started, true);
        match cmd {
            CarriageReturn => self.x = 0,
            LineFeed | EscNextLine => self.newline(),
            EscIndex => self.y = (self.y + 1).min(MAX_ROWS),
            EscReverseIndex => self.y = self.y.saturating_sub(1),
            Backspace => self.x = self.x.saturating_sub(1),
            Tab => self.x = ((self.x / 8 + 1) * 8).min(self.width - 1),
            // Art files use these bytes as glyphs (♀ •); only a leading FF clears.
            FormFeed if !started => self.clear(),
            FormFeed => self.put(cp437::to_char(0x0C)),
            Bell => self.put(cp437::to_char(0x07)),
            Delete => self.put(cp437::to_char(0x7F)),
            CsiMoveCursor(dir, n, _) => match dir {
                Direction::Up => self.y = self.y.saturating_sub(n1(n)),
                Direction::Down => self.y = (self.y + n1(n)).min(MAX_ROWS),
                Direction::Left => self.x = self.x.saturating_sub(n1(n)),
                Direction::Right => self.x = (self.x + n1(n)).min(self.width - 1),
            },
            CsiCursorNextLine(n) => self.goto(0, self.y + n1(n)),
            CsiCursorPreviousLine(n) => self.goto(0, self.y.saturating_sub(n1(n))),
            CsiCursorHorizontalAbsolute(c) | CsiHorizontalPositionAbsolute(c) => self.goto(n1(c) - 1, self.y),
            CsiCharacterPositionForward(n) => self.goto(self.x + n1(n), self.y),
            CsiLinePositionAbsolute(r) => self.goto(self.x, n1(r) - 1),
            CsiLinePositionForward(n) => self.goto(self.x, self.y + n1(n)),
            CsiCursorPosition(r, c) => self.goto(n1(c) - 1, n1(r) - 1),
            CsiEraseInLine(mode) => match mode {
                EraseInLineMode::CursorToEnd => self.fill(self.x, self.width, self.y),
                EraseInLineMode::StartToCursor => self.fill(0, self.x + 1, self.y),
                EraseInLineMode::All => self.fill(0, self.width, self.y),
            },
            CsiEraseInDisplay(mode) => match mode {
                EraseInDisplayMode::All | EraseInDisplayMode::AllAndScrollback => self.clear(),
                EraseInDisplayMode::CursorToEnd => {
                    self.fill(self.x, self.width, self.y);
                    self.rows.truncate(self.y + 1);
                }
                EraseInDisplayMode::StartToCursor => {
                    for y in 0..self.y.min(self.rows.len()) {
                        self.fill(0, self.width, y);
                    }
                    self.fill(0, self.x + 1, self.y);
                }
            },
            CsiEraseCharacter(n) => self.fill(self.x, self.x + n1(n), self.y),
            CsiSaveCursorPosition | EscSaveCursor => self.saved = (self.x, self.y),
            CsiRestoreCursorPosition | EscRestoreCursor => (self.x, self.y) = self.saved,
            CsiDecSetMode(DecMode::IceColors, on) => self.ice_hint = on,
            EscReset => {
                self.reset_attr();
                self.clear();
            }
            CsiSelectGraphicRendition(sgr) => match sgr {
                SgrAttribute::Reset => self.reset_attr(),
                SgrAttribute::Intensity(i) => self.bold = i == Intensity::Bold,
                SgrAttribute::Blink(b) => self.blink = b != Blink::Off,
                SgrAttribute::Inverse(on) => self.inverse = on,
                SgrAttribute::Concealed(on) => self.conceal = on,
                SgrAttribute::Foreground(c) => self.fg = color(c, Color::LIGHT_GRAY),
                SgrAttribute::Background(c) => self.bg = color(c, Color::BLACK),
                _ => {}
            },
            _ => {}
        }
    }
}

/// Load ANSI. `utf8`: force UTF-8 (Some(true)), force CP437 (Some(false)),
/// or detect (valid UTF-8 with multi-byte chars and no SAUCE).
pub fn load(data: &[u8], utf8: Option<bool>) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    let body = &body[..body.iter().position(|&b| b == 0x1A).unwrap_or(body.len())];
    let utf8 =
        utf8.unwrap_or_else(|| rec.is_none() && body.iter().any(|&b| b >= 0x80) && std::str::from_utf8(body).is_ok());
    let width = sauce::width(rec.as_ref()).unwrap_or(if utf8 { UNBOUNDED } else { 80 });
    let mut film = Film { scr: Screen::new(width, utf8), shots: Vec::new(), dirty: false, budget: SHOT_BUDGET };
    AnsiParser::new().parse(body, &mut film);
    film.shoot();
    let hint = film.scr.ice_hint;
    let shots = film.frames();
    if shots.len() < 2 {
        return Ok(finish(film.scr.into_grid(sauce::min_rows(rec.as_ref())), rec.as_ref(), None, utf8, hint));
    }
    // One grid, frames stacked, so colors are settled the same way for all.
    let h = shots.iter().map(|(g, _)| g.height).max().unwrap_or(1).max(1);
    let w = shots.iter().map(|(g, _)| g.width).max().unwrap_or(80);
    let mut tall = Grid::new(w, h * shots.len());
    for (i, (g, _)) in shots.iter().enumerate() {
        for y in 0..g.height {
            for x in 0..g.width {
                tall.set(x, i * h + y, g.get(x, y));
            }
        }
    }
    let d = finish(tall, rec.as_ref(), None, utf8, hint);
    let cells = &d.canvas.layers[0].cells;
    let mut set = FrameSet { frames: Vec::new(), current: 0, fps: DEFAULT_FPS };
    for (i, (_, hold)) in shots.iter().enumerate() {
        let part = Grid {
            width: w,
            height: h,
            cells: cells[i * h * w..(i + 1) * h * w].iter().map(|c| c.unwrap_or(Cell::BLANK)).collect(),
        };
        let id = if i == 0 { FIRST_FRAME } else { d.new_frame_id() };
        let canvas = Document::from_grid(d.meta.kind, &part).canvas;
        set.frames.push(Frame { id, hold: *hold, canvas: Some(canvas) });
    }
    let mut out = Document::new(d.meta.kind, w, h);
    out.meta = d.meta;
    out.set_frame_set(&set);
    Ok(out)
}

/// Screen snapshots a file may take (in cells), so a hostile file can't
/// blow up memory.
const SHOT_BUDGET: usize = 8_000_000;
/// Most frames an ANSI animation loads as.
const MAX_FRAMES: usize = 1000;

/// The screen, plus a snapshot each time drawing goes back to the top
/// left (cursor home or clear screen) after something was printed. That's
/// how ansimations play, frame by frame.
struct Film {
    scr: Screen,
    shots: Vec<Grid>,
    dirty: bool,
    budget: usize,
}

impl Film {
    fn shoot(&mut self) {
        if !std::mem::take(&mut self.dirty) || self.budget == 0 {
            return;
        }
        let g = self.scr.snapshot();
        self.budget = self.budget.saturating_sub(g.cells.len());
        if self.budget == 0 {
            self.shots.clear();
        } else {
            self.shots.push(g);
        }
    }

    /// Distinct frames with how many snapshots each was held for; empty
    /// unless it's really an animation.
    fn frames(&self) -> Vec<(Grid, u32)> {
        if self.budget == 0 {
            return Vec::new();
        }
        let mut out: Vec<(Grid, u32)> = Vec::new();
        for g in &self.shots {
            match out.last_mut() {
                Some((last, n)) if last == g => *n = (*n + 1).min(99),
                _ => out.push((g.clone(), 1)),
            }
        }
        if out.len() > MAX_FRAMES { Vec::new() } else { out }
    }
}

impl CommandSink for Film {
    fn print(&mut self, text: &[u8]) {
        self.dirty = true;
        self.scr.print(text);
    }

    fn emit(&mut self, cmd: TerminalCommand) {
        let home = match cmd {
            TerminalCommand::CsiCursorPosition(r, c) => r <= 1 && c <= 1,
            TerminalCommand::CsiEraseInDisplay(m) => {
                matches!(m, EraseInDisplayMode::All | EraseInDisplayMode::AllAndScrollback)
            }
            _ => false,
        };
        if home {
            self.shoot();
        }
        self.scr.emit(cmd);
    }
}

/// Bytes that the ANSI parser would interpret instead of print.
pub fn unprintable(b: u8) -> bool {
    matches!(b, 0x00 | 0x08..=0x0A | 0x0D | 0x1A | 0x1B)
}

/// Visually close printable replacement for an unprintable byte.
pub fn printable(b: u8) -> u8 {
    match b {
        0x00 => b' ',
        0x08 | 0x0A => 0xDB,
        0x09 => b'o',
        0x0D => 0x0E,
        0x1A => b'>',
        0x1B => b'<',
        b => b,
    }
}

/// A cell that can be skipped with a cursor move (looks like the empty screen).
fn skippable(c: &Cell) -> bool {
    matches!(c.ch, ' ' | '\0') && c.bg == Color::BLACK
}

/// Current SGR state as the loader would track it.
#[derive(Clone, Copy, PartialEq)]
struct Pen {
    fg: Color,
    bg: Color,
    bold: bool,
    blink: bool,
}

const RESET: Pen = Pen { fg: Color::LIGHT_GRAY, bg: Color::BLACK, bold: false, blink: false };

impl Pen {
    /// Append the minimal SGR (and PabloDraw 24-bit) codes to reach `c`.
    fn to(&mut self, c: &Cell, out: &mut Vec<u8>) {
        let (fg, want_bold) = match c.fg {
            Color::Pal(i) => (Color::Pal(i & 7), Some(i >= 8)),
            rgb => (rgb, None),
        };
        let (bg, want_blink) = match c.bg {
            Color::Pal(i) => (Color::Pal(i & 7), Some(i >= 8)),
            rgb => (rgb, None),
        };
        let mut codes: Vec<u8> = Vec::new();
        if (self.bold && want_bold == Some(false)) || (self.blink && want_blink == Some(false)) {
            *self = RESET;
            codes.push(0);
        }
        if want_bold == Some(true) && !self.bold {
            codes.push(1);
            self.bold = true;
        }
        if want_blink == Some(true) && !self.blink {
            codes.push(5);
            self.blink = true;
        }
        if let Color::Pal(i) = fg
            && self.fg != fg
        {
            codes.push(30 + VGA_TO_ANSI[i as usize]);
            self.fg = fg;
        }
        if let Color::Pal(i) = bg
            && self.bg != bg
        {
            codes.push(40 + VGA_TO_ANSI[i as usize]);
            self.bg = bg;
        }
        if !codes.is_empty() {
            let s: Vec<String> = codes.iter().map(u8::to_string).collect();
            out.extend(format!("\x1b[{}m", s.join(";")).bytes());
        }
        if let Color::Rgb(r, g, b) = fg
            && self.fg != fg
        {
            out.extend(format!("\x1b[1;{r};{g};{b}t").bytes());
            self.fg = fg;
        }
        if let Color::Rgb(r, g, b) = bg
            && self.bg != bg
        {
            out.extend(format!("\x1b[0;{r};{g};{b}t").bytes());
            self.bg = bg;
        }
    }

    fn looks_black(&self) -> bool {
        self.bg == Color::BLACK && !self.blink
    }
}

/// Classic ANSI body (no SAUCE/EOF). Modern docs are reduced to CP437 + 16
/// colors; Classic docs keep RGB cells as PabloDraw 24-bit codes.
pub fn encode(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    if opts.animate && doc.is_animated() {
        return encode_animation(doc, opts);
    }
    let g = if doc.meta.kind == DocKind::Modern { classic_grid(doc, opts) } else { export_grid(doc, opts) };
    let uses_ice = g.cells.iter().any(|c| matches!(c.bg, Color::Pal(8..=15)));
    let mut out = Vec::new();
    if opts.clear_screen {
        out.extend(b"\x1b[2J");
    }
    if opts.ice_hint && uses_ice {
        out.extend(b"\x1b[?33h");
    }
    out.extend(b"\x1b[0m");
    let mut pen = RESET;
    body(&g, &mut out, &mut pen, opts.line_length.filter(|&n| n > 0), false);
    // Blank rows at the bottom write nothing, so a reader would stop at the
    // last row with art. Keeping the whole canvas: a blank on the last row
    // makes it (and every row above) come back.
    if !opts.trim_height && g.height > 0 && g.row(g.height - 1).iter().all(skippable) {
        pen.to(&Cell::BLANK, &mut out);
        out.push(b' ');
    }
    out.extend(b"\x1b[0m");
    out
}

/// The classic ansimation: clear, then every frame drawn over the last from
/// the home position (a frame held for n ticks is sent n times), so it plays
/// with `cat` or at modem speed.
fn encode_animation(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    let mut frames = super::frame_grids(doc, opts);
    if doc.meta.kind == DocKind::Modern {
        for (g, _) in &mut frames {
            super::to_classic(g, &doc.meta.palette);
        }
    }
    let uses_ice = frames.iter().any(|(g, _)| g.cells.iter().any(|c| matches!(c.bg, Color::Pal(8..=15))));
    let mut out = Vec::new();
    out.extend(b"\x1b[0m\x1b[2J\x1b[H");
    if opts.ice_hint && uses_ice {
        out.extend(b"\x1b[?33h");
    }
    let mut pen = RESET;
    for (g, hold) in &frames {
        for _ in 0..*hold {
            out.extend(b"\x1b[H");
            body(g, &mut out, &mut pen, None, true);
        }
    }
    out.extend(b"\x1b[0m");
    out
}

/// Rows of `g` as ANSI. `overwrite`: every cell is written and line ends
/// are erased, so the frame fully covers whatever was on screen.
fn body(g: &Grid, out: &mut Vec<u8>, pen: &mut Pen, limit: Option<usize>, overwrite: bool) {
    let mut line_start = out.len();
    let mut tok = Vec::new();
    for y in 0..g.height {
        let row = g.row(y);
        let last = row.iter().rposition(|c| !skippable(c));
        let mut x = 0;
        while let Some(last) = last.filter(|&l| x <= l) {
            tok.clear();
            let step = if skippable(&row[x]) && !overwrite {
                let run = row[x..=last].iter().take_while(|c| skippable(c)).count();
                if run >= 4 || !pen.looks_black() {
                    if run == 1 { tok.extend(b"\x1b[C") } else { tok.extend(format!("\x1b[{run}C").bytes()) }
                } else {
                    tok.resize(run, b' ');
                }
                run
            } else {
                pen.to(&row[x], &mut tok);
                tok.push(printable(char_byte(row[x].ch)));
                1
            };
            if let Some(n) = limit
                && out.len() - line_start > 0
                && out.len() - line_start + tok.len() > n
            {
                out.extend(b"\r\n\x1b[A");
                line_start = out.len() - 3;
                if x > 0 {
                    out.extend(format!("\x1b[{x}C").bytes());
                }
            }
            out.extend(&tok);
            x += step;
        }
        let wrapped = last.is_some_and(|l| l + 1 >= g.width);
        if overwrite && !wrapped {
            pen.to(&Cell::BLANK, out);
            out.extend(b"\x1b[K");
        }
        if !wrapped && y + 1 < g.height {
            out.extend(b"\r\n");
            line_start = out.len();
        }
    }
}

pub fn save(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let mut out = encode(doc, opts);
    let rows = if opts.animate && doc.is_animated() {
        super::frame_grids(doc, opts).first().map_or(1, |(g, _)| g.height)
    } else {
        super::export_rows(doc, opts)
    };
    // Without SAUCE a reader opens at least 25 rows: a shorter canvas needs it.
    let short = !opts.trim_height && rows < 25;
    if want_sauce(doc, opts, doc.width() != 80 || short) {
        sauce::append(&mut out, doc, sauce::Kind::Character(CHAR_ANSI), doc.width(), rows)?;
    } else if opts.eof_char {
        out.push(0x1A);
    }
    Ok(out)
}

/// UTF-8 ANSI: truecolor SGR on change, `ESC[0m\n` line ends.
pub fn utf8_string(doc: &Document, opts: &SaveOptions) -> String {
    utf8_grid(&export_grid(doc, opts), &doc.meta.palette)
}

/// A grid as UTF-8 ANSI (see [`utf8_string`]).
pub fn utf8_grid(g: &Grid, pal: &acidtrip_core::Palette) -> String {
    let mut s = String::new();
    for y in 0..g.height {
        let (mut fg, mut bg): (Option<[u8; 3]>, Option<[u8; 3]>) = (None, None);
        for c in g.row(y) {
            let (f, b) = (c.fg.rgb(pal), c.bg.rgb(pal));
            let mut codes = Vec::new();
            if fg != Some(f) {
                codes.push(format!("38;2;{};{};{}", f[0], f[1], f[2]));
                fg = Some(f);
            }
            if bg != Some(b) {
                codes.push(format!("48;2;{};{};{}", b[0], b[1], b[2]));
                bg = Some(b);
            }
            if !codes.is_empty() {
                let _ = write!(s, "\x1b[{}m", codes.join(";"));
            }
            s.push(match c.ch {
                '\0' => ' ',
                ch if ch.is_control() => cp437::to_char(char_byte(ch)),
                ch => ch,
            });
        }
        s.push_str("\x1b[0m\n");
    }
    s
}

pub fn save_utf8(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    utf8_string(doc, opts).into_bytes()
}

/// Incremental replay of an ANSI stream (for reveal animations).
pub struct Replay {
    parser: AnsiParser,
    pub screen: Screen,
}

impl Replay {
    pub fn new(width: usize) -> Self {
        Replay { parser: AnsiParser::new(), screen: Screen::new(width, false) }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.parse(bytes, &mut self.screen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(doc: &Document) -> Grid {
        doc.flatten()
    }

    #[test]
    fn sgr_bold_blink_and_wrap() {
        let d = load(b"\x1b[1;31mA\x1b[0;5;44mB\x1b[0m\r\nC", None).unwrap();
        let g = grid(&d);
        assert_eq!(g.get(0, 0), Cell::new('A', Color::Pal(12), Color::BLACK));
        assert_eq!(g.get(1, 0), Cell::new('B', Color::LIGHT_GRAY, Color::Pal(9)));
        assert_eq!(g.get(0, 1).ch, 'C');
        assert!(d.meta.ice);
        assert_eq!(d.height(), 25);
        // immediate wrap at 80
        let mut long = vec![b'x'; 81];
        long.extend(b"\r\ny");
        let g = grid(&load(&long, None).unwrap());
        assert_eq!(g.get(0, 1).ch, 'x');
        assert_eq!(g.get(0, 2).ch, 'y');
    }

    #[test]
    fn cursor_moves_and_save_restore() {
        let g = grid(&load(b"\x1b[5;10HX\x1b[sY\x1b[3;1HZ\x1b[uW\x1b[2CQ\x1b[20GR", None).unwrap());
        assert_eq!(g.get(9, 4).ch, 'X');
        assert_eq!(g.get(10, 4).ch, 'W');
        assert_eq!(g.get(13, 4).ch, 'Q');
        assert_eq!(g.get(19, 4).ch, 'R');
        assert_eq!(g.get(0, 2).ch, 'Z');
    }

    #[test]
    fn truecolor_and_pablo() {
        let d = load(b"\x1b[38;2;1;2;3mA\x1b[1;170;0;0tB\x1b[48;5;196mC\x1b[38;5;9mD", None).unwrap();
        let g = grid(&d);
        assert_eq!(g.get(0, 0).fg, Color::Rgb(1, 2, 3));
        assert_eq!(g.get(1, 0).fg, Color::Pal(4), "exact palette RGB canonicalizes");
        assert_eq!(g.get(2, 0).bg, Color::Rgb(255, 0, 0));
        assert_eq!(g.get(3, 0).fg, Color::Pal(12));
        assert_eq!(d.meta.kind, DocKind::Modern);
    }

    #[test]
    fn utf8_detection() {
        let d = load("\x1b[31m╔═╗ 😀".as_bytes(), None).unwrap();
        assert_eq!(d.meta.kind, DocKind::Modern);
        assert_eq!(grid(&d).get(4, 0).ch, '😀');
        let d = load(b"\xc9\xcd\xbb", None).unwrap();
        assert_eq!(grid(&d).get(0, 0).ch, '╔');
        assert_eq!(d.meta.kind, DocKind::Classic);
    }

    #[test]
    fn save_is_minimal() {
        let mut d = Document::new(DocKind::Classic, 80, 1);
        let red = Cell::new('A', Color::Pal(12), Color::BLACK);
        d.canvas.layers[0].cells[0] = Some(red);
        d.canvas.layers[0].cells[1] = Some(red);
        d.canvas.layers[0].cells[10] = Some(Cell::new('B', Color::Pal(4), Color::Pal(9)));
        let out = encode(&d, &SaveOptions::default());
        assert_eq!(String::from_utf8_lossy(&out), "\x1b[0m\x1b[1;31mAA\x1b[8C\x1b[0;5;31;44mB\x1b[0m");
    }

    #[test]
    fn keeps_blank_rows_at_the_bottom() {
        let keep = SaveOptions { trim_height: false, ..SaveOptions::default() };
        for (h, attach) in [(40, false), (40, true), (10, false), (10, true)] {
            let mut d = Document::new(DocKind::Classic, 80, h);
            d.meta.sauce.attach = attach;
            d.canvas.layers[0].cells[0] = Some(Cell::new('A', Color::Pal(12), Color::Pal(1)));
            let back = load(&save(&d, &keep).unwrap(), None).unwrap();
            assert_eq!(back.height(), h, "{h} rows, SAUCE {attach}");
            assert_eq!(back.flatten().row(0)[0], d.flatten().row(0)[0]);
            assert!(back.flatten().row(h - 1).iter().all(Cell::is_blank));
        }
        // Trimmed (an export), the file says it's the rows with art.
        let mut d = Document::new(DocKind::Classic, 80, 40);
        d.meta.sauce.attach = true;
        d.canvas.layers[0].cells[0] = Some(Cell::new('A', Color::LIGHT_GRAY, Color::BLACK));
        assert_eq!(load(&save(&d, &SaveOptions::default()).unwrap(), None).unwrap().height(), 1);
    }

    #[test]
    fn line_length_breaks_resume() {
        let mut d = Document::new(DocKind::Classic, 80, 2);
        for (i, c) in d.canvas.layers[0].cells.iter_mut().enumerate() {
            *c = Some(Cell::new('#', Color::Pal((i % 16) as u8), Color::Pal((i % 7) as u8)));
        }
        let opts = SaveOptions { line_length: Some(79), ..SaveOptions::default() };
        let out = encode(&d, &opts);
        assert!(out.split(|&b| b == b'\n').all(|l| l.len() <= 79 + 1));
        assert_eq!(grid(&load(&out, None).unwrap()).row(0), d.flatten().row(0));
    }
}
