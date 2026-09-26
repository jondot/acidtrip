//! "Play at modem speed": reveal the piece in reading order at a baud
//! rate, like it scrolled onto a BBS screen.

use std::time::Instant;

use acidtrip_core::Cell;
use ratatui::buffer::Buffer;
use ratatui::style::Style;

use crate::tab::Tab;
use crate::ui::canvas::CanvasGeom;
use crate::ui::widgets::theme;

pub struct Playback {
    cells: Vec<(usize, usize, Cell)>,
    width: usize,
    start: Instant,
    /// Cells per second (baud / 10 bits per byte, ~1 byte per cell plus escapes).
    rate: f64,
    shown: usize,
    done_at: Option<Instant>,
}

impl Playback {
    pub fn new(tab: &Tab, baud: u32) -> Self {
        let g = tab.doc.flatten();
        let h = g.used_height().max(1);
        let cells = (0..h).flat_map(|y| (0..g.width).map(move |x| (x, y))).map(|(x, y)| (x, y, g.get(x, y))).collect();
        Playback {
            cells,
            width: g.width,
            start: Instant::now(),
            rate: baud as f64 / 10.0 / 1.6,
            shown: 0,
            done_at: None,
        }
    }

    /// Advance the clock; returns true when finished (after a short hold).
    pub fn advance(&mut self) -> bool {
        self.shown = ((self.start.elapsed().as_secs_f64() * self.rate) as usize).min(self.cells.len());
        if self.shown >= self.cells.len() {
            let at = *self.done_at.get_or_insert_with(Instant::now);
            return at.elapsed().as_secs_f64() > 2.0;
        }
        false
    }

    /// Canvas row currently being revealed (the view follows it).
    pub fn row(&self) -> usize {
        self.shown / self.width.max(1)
    }

    /// Everything already revealed, drawn over a black screen.
    pub fn overlay(&self) -> Vec<(usize, usize, Cell)> {
        self.cells[..self.shown].to_vec()
    }

    /// Blank the not-yet-revealed cells and draw the modem cursor.
    pub fn mask(&self, buf: &mut Buffer, g: &CanvasGeom) {
        let next = self.cells.get(self.shown).map(|&(x, y, _)| (x, y));
        for sy in 0..g.area.height {
            for sx in 0..g.area.width {
                let (cx, cy) = if g.zoom {
                    (g.scroll.0 + sx as usize / 2, g.scroll.1 + sy as usize / 2)
                } else {
                    (g.scroll.0 + sx as usize, g.scroll.1 + sy as usize)
                };
                if cx >= self.width {
                    continue;
                }
                let idx = cy * self.width + cx;
                if idx >= self.shown
                    && let Some(c) = buf.cell_mut((g.area.x + sx, g.area.y + sy))
                {
                    if Some((cx, cy)) == next {
                        c.set_char('▄').set_style(Style::new().fg(theme::TEXT).bg(ratatui::style::Color::Black));
                    } else {
                        c.set_char(' ').set_style(Style::new().bg(ratatui::style::Color::Black));
                    }
                }
            }
        }
    }
}
