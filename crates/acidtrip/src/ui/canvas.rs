//! The canvas viewport: draws the document with truecolor from its palette
//! (never terminal color indices), overlays, selection and zoom.

use std::collections::HashMap;

use acidtrip_core::tools::Rect as CellRect;
use acidtrip_core::{Cell, Color as DocColor, Palette};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::widgets::theme;
use crate::tab::Tab;

pub struct CanvasView<'a> {
    pub tab: &'a Tab,
    pub overlay: &'a [(usize, usize, Cell)],
    pub pending_selection: Option<CellRect>,
    pub grid: bool,
    /// Show the keyboard cursor as an inverted cell (tool mode).
    pub show_cursor: bool,
    /// Onion skin the neighbouring frames (off while replaying).
    pub onion: bool,
}

/// How screen cells map to canvas cells for the last drawn frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct CanvasGeom {
    pub area: Rect,
    pub scroll: (usize, usize),
    pub zoom: bool,
    /// Visible size in canvas cells.
    pub cols: usize,
    pub rows: usize,
}

impl CanvasGeom {
    /// Screen position -> canvas cell (clamped to the canvas). In zoom mode
    /// also returns the half-block pixel row.
    pub fn to_cell(self, sx: u16, sy: u16, w: usize, h: usize) -> Option<(usize, usize, usize)> {
        if sx < self.area.x || sy < self.area.y || sx >= self.area.right() || sy >= self.area.bottom() {
            return None;
        }
        let (dx, dy) = ((sx - self.area.x) as usize, (sy - self.area.y) as usize);
        let (x, y, py) = if self.zoom {
            let x = self.scroll.0 + dx / 2;
            let y = self.scroll.1 + dy / 2;
            (x, y, self.scroll.1 * 2 + dy)
        } else {
            let y = self.scroll.1 + dy;
            (self.scroll.0 + dx, y, y * 2)
        };
        (x < w && y < h).then_some((x, y, py))
    }

    /// Like `to_cell` but clamps positions outside the canvas to its edge
    /// (dragging past the border).
    pub fn to_cell_clamped(self, sx: u16, sy: u16, w: usize, h: usize) -> (usize, usize, usize) {
        let cx = sx.clamp(self.area.x, self.area.right().saturating_sub(1));
        let cy = sy.clamp(self.area.y, self.area.bottom().saturating_sub(1));
        let (dx, dy) = ((cx - self.area.x) as usize, (cy - self.area.y) as usize);
        if self.zoom {
            let x = (self.scroll.0 + dx / 2).min(w.saturating_sub(1));
            let y = (self.scroll.1 + dy / 2).min(h.saturating_sub(1));
            (x, y, (self.scroll.1 * 2 + dy).min(h * 2 - 1))
        } else {
            let x = (self.scroll.0 + dx).min(w.saturating_sub(1));
            let y = (self.scroll.1 + dy).min(h.saturating_sub(1));
            (x, y, y * 2)
        }
    }
}

pub fn rgb(c: DocColor, pal: &Palette) -> Color {
    let [r, g, b] = c.rgb(pal);
    Color::Rgb(r, g, b)
}

fn dim(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(r / 3 + 10, g / 3 + 10, b / 3 + 14),
        o => o,
    }
}

/// Blend a color 45% toward mid gray (the cursor's translucent look).
fn veil(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let mix = |v: u8| ((v as u16 * 55 + 150 * 45) / 100) as u8;
            Color::Rgb(mix(r), mix(g), mix(b))
        }
        o => o,
    }
}

fn invert(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(255 - r, 255 - g, 255 - b),
        o => o,
    }
}

/// Top/bottom half-block pixel colors of a cell, if it is pixel-like.
fn halves(c: &Cell) -> Option<(DocColor, DocColor)> {
    match c.ch {
        '▀' => Some((c.fg, c.bg)),
        '▄' => Some((c.bg, c.fg)),
        '█' => Some((c.fg, c.fg)),
        ' ' | '\u{0}' | '\u{A0}' => Some((c.bg, c.bg)),
        _ => None,
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, v: &CanvasView) -> CanvasGeom {
    let tab = v.tab;
    let doc = &tab.doc;
    let pal = &doc.meta.palette;
    let zoom = tab.zoom;
    let cols = if zoom { area.width as usize / 2 } else { area.width as usize };
    let rows = if zoom { area.height as usize / 2 } else { area.height as usize };
    let geom = CanvasGeom { area, scroll: tab.scroll, zoom, cols, rows };
    let overlay: HashMap<(usize, usize), Cell> = v.overlay.iter().map(|&(x, y, c)| ((x, y), c)).collect();
    let sel = v.pending_selection.or(tab.selection);
    let (w, h) = (doc.width(), doc.height());
    // Onion skin: the neighbouring frames, dimmed, where this one is empty.
    let cur = doc.current_frame();
    let onion: Vec<&acidtrip_core::Canvas> = if v.onion && tab.playing.is_none() && doc.is_animated() {
        [(tab.onion.0 && cur > 0).then(|| cur - 1), (tab.onion.1 && cur + 1 < doc.frame_count()).then_some(cur + 1)]
            .into_iter()
            .flatten()
            .map(|i| doc.frame_canvas(i))
            .collect()
    } else {
        vec![]
    };

    for sy in 0..area.height {
        for sx in 0..area.width {
            let (cx, cy, sub_y) = if zoom {
                (tab.scroll.0 + sx as usize / 2, tab.scroll.1 + sy as usize / 2, sy as usize % 2)
            } else {
                (tab.scroll.0 + sx as usize, tab.scroll.1 + sy as usize, 0)
            };
            let Some(bcell) = buf.cell_mut((area.x + sx, area.y + sy)) else {
                continue;
            };
            if cx >= w || cy >= h {
                bcell.set_char(' ').set_style(Style::new().bg(theme::OUTSIDE));
                if cx == w && cy < h {
                    bcell.set_char('▏').set_fg(theme::BORDER);
                } else if cy == h && cx < w {
                    bcell.set_char('▔').set_fg(theme::BORDER);
                }
                continue;
            }
            let (mut cell, mut reference) = match overlay.get(&(cx, cy)) {
                Some(c) => (*c, false),
                None => doc.canvas.composite_with_reference(cx, cy),
            };
            if cell.is_blank()
                && !reference
                && !overlay.contains_key(&(cx, cy))
                && let Some(ghost) = onion.iter().map(|c| c.composite(cx, cy)).find(|c| !c.is_blank())
            {
                // Drawn like a reference layer: dimmed, not part of the frame.
                (cell, reference) = (ghost, true);
            }
            let (mut ch, mut fg, mut bg) = (cell.ch, rgb(cell.fg, pal), rgb(cell.bg, pal));
            if zoom {
                match halves(&cell) {
                    Some((top, bottom)) => {
                        let c = if sub_y == 0 { top } else { bottom };
                        ch = ' ';
                        bg = rgb(c, pal);
                    }
                    None => {
                        // Non-pixel glyph: show it top-left, background elsewhere.
                        if sx % 2 != 0 || sub_y != 0 {
                            ch = ' ';
                        }
                    }
                }
            }
            if ch == '\u{0}' || ch.is_control() {
                ch = ' ';
            }
            if reference {
                fg = dim(fg);
                bg = dim(bg);
            }
            if v.grid && cell.is_blank() && !overlay.contains_key(&(cx, cy)) && (cx % 10 == 0 && cy % 5 == 0) {
                ch = '·';
                fg = theme::BORDER;
            }
            let mut style = Style::new().fg(fg).bg(bg);
            if let Some(s) = sel {
                let on_edge = s.contains(cx, cy) && (cx == s.x || cx == s.right() || cy == s.y || cy == s.bottom());
                if on_edge && (cx + cy) % 2 == 0 {
                    style = Style::new().fg(invert(fg)).bg(invert(bg));
                } else if s.contains(cx, cy) {
                    style = style.add_modifier(Modifier::BOLD);
                }
            }
            if v.show_cursor && (cx, cy) == tab.cursor && (!zoom || sx % 2 == 0) {
                // A see-through gray veil: the glyph and its colors stay readable.
                style = Style::new().fg(veil(fg)).bg(veil(bg));
            }
            bcell.set_char(ch).set_style(style);
        }
    }
    geom
}

/// Draw a clip (transparent cells show the panel background) into `area`,
/// clipped. Used by font/stencil/version previews.
pub fn draw_clip(buf: &mut Buffer, area: Rect, clip: &acidtrip_core::Clip, pal: &Palette) {
    for y in 0..clip.height.min(area.height as usize) {
        for x in 0..clip.width.min(area.width as usize) {
            if let Some(c) = clip.get(x, y)
                && let Some(b) = buf.cell_mut((area.x + x as u16, area.y + y as u16))
            {
                let ch = if c.ch.is_control() { ' ' } else { c.ch };
                b.set_char(ch).set_style(Style::new().fg(rgb(c.fg, pal)).bg(rgb(c.bg, pal)));
            }
        }
    }
}
