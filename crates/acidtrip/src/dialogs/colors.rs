//! Color dialog (ACiDDraw's quick palette): ↑↓ fg, ←→ bg, click, and hex
//! input for RGB colors in Modern documents.

use acidtrip_core::{Color, DocKind};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::sidebar::Slot;
use crate::ui::widgets::{LineInput, centered, popup, theme};

pub struct ColorDialog {
    fg: Color,
    bg: Color,
    hex: LineInput,
    hex_focus: bool,
    swatches: Rect,
    bg_limit: u8,
    modern: bool,
    /// Which color a plain click sets (as in the sidebar), and where the
    /// fg / bg labels that switch it are.
    slot: Slot,
    labels: [Rect; 2],
}

impl ColorDialog {
    pub fn new(app: &App) -> Self {
        let d = &app.tab().doc;
        let modern = d.meta.kind == DocKind::Modern;
        ColorDialog {
            fg: app.tools.brush.fg,
            bg: app.tools.brush.bg,
            hex: LineInput::default(),
            hex_focus: false,
            swatches: Rect::default(),
            bg_limit: if modern || d.meta.ice { 16 } else { 8 },
            modern,
            slot: app.color_slot,
            labels: [Rect::default(); 2],
        }
    }

    fn done(&self) -> Outcome {
        let (fg, bg, slot) = (self.fg, self.bg, self.slot);
        Outcome::Then(Box::new(move |app: &mut App| {
            app.tools.brush.fg = fg;
            app.tools.brush.bg = bg;
            app.color_slot = slot;
        }))
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok()?;
    (h.len() == 6).then_some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

impl Dialog for ColorDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let pal = &app.tab().doc.meta.palette;
        let r = centered(area, 64, if self.modern { 13 } else { 11 });
        let inner = popup(f, r, "Colors", "↑↓ fg · ←→ bg · click sets the ▸one◂ · Enter ok · Esc");
        self.swatches = Rect::new(inner.x + 2, inner.y + 1, 48, 4);
        for row in 0..2u8 {
            let mut spans = vec![];
            for col in 0..8u8 {
                let i = row * 8 + col;
                let [cr, cg, cb] = pal.get(i);
                let mark = match (self.fg == Color::Pal(i), self.bg == Color::Pal(i)) {
                    (true, true) => " FB ",
                    (true, false) => " F  ",
                    (false, true) => " B  ",
                    _ if i >= self.bg_limit => "  · ",
                    _ => "    ",
                };
                let lum = (cr as u32 * 3 + cg as u32 * 6 + cb as u32) / 10;
                let t = if lum > 120 { ratatui::style::Color::Black } else { ratatui::style::Color::White };
                spans.push(Span::styled(
                    format!("{mark}  "),
                    Style::new().bg(ratatui::style::Color::Rgb(cr, cg, cb)).fg(t),
                ));
            }
            f.render_widget(
                Paragraph::new(vec![Line::from(spans.clone()), Line::from(spans)]),
                Rect::new(self.swatches.x, self.swatches.y + row as u16 * 2, 48, 2),
            );
        }
        let (fgc, bgc) = (crate::ui::canvas::rgb(self.fg, pal), crate::ui::canvas::rgb(self.bg, pal));
        let row = Rect::new(inner.x + 2, inner.y + 6, inner.width.saturating_sub(4), 1);
        let sample = " ▓▒░ Aa ░▒▓ ";
        let mut spans = vec![Span::styled(sample, Style::new().fg(fgc).bg(bgc)), Span::raw("  ")];
        let mut x = row.x + sample.chars().count() as u16 + 2;
        for (i, (slot, name, c)) in [(Slot::Fg, "fg", self.fg), (Slot::Bg, "bg", self.bg)].into_iter().enumerate() {
            let on = self.slot == slot;
            let text = format!("{}{name} {}{}", if on { "▸" } else { " " }, crate::ui::sidebar::color_label(c), if on {
                "◂"
            } else {
                " "
            });
            let w = text.chars().count() as u16;
            self.labels[i] = Rect::new(x, row.y, w, 1);
            x += w + 1;
            let st = if on { Style::new().fg(theme::TEXT).bg(theme::PANEL_HI) } else { Style::new().fg(theme::DIM) };
            spans.push(Span::styled(text, st));
            spans.push(Span::raw(" "));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), row);
        if self.bg_limit == 8 {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "· = foreground only (turn on iCE for bright backgrounds)",
                    Style::new().fg(theme::DIM),
                )),
                Rect::new(inner.x + 2, inner.y + 7, inner.width.saturating_sub(4), 1),
            );
        }
        if self.modern {
            self.hex.render(
                f,
                Rect::new(inner.x + 2, inner.y + 9, inner.width.saturating_sub(4), 1),
                "RGB fg #",
                self.hex_focus,
            );
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        if self.hex_focus {
            match k.code {
                KeyCode::Enter => {
                    match parse_hex(&self.hex.text) {
                        Some(c) => self.fg = c,
                        None => {
                            return Outcome::KeepThen(Box::new(|app: &mut App| {
                                app.flash("hex color like ff8800", Level::Warn)
                            }));
                        }
                    }
                    self.hex_focus = false;
                }
                KeyCode::Esc | KeyCode::Tab => self.hex_focus = false,
                _ => {
                    self.hex.key(&k);
                }
            }
            return Outcome::Keep;
        }
        let fi = self.fg.index().unwrap_or(7);
        let bi = self.bg.index().unwrap_or(0) % self.bg_limit;
        match k.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => return self.done(),
            KeyCode::Up => self.fg = Color::Pal((fi + 15) % 16),
            KeyCode::Down => self.fg = Color::Pal((fi + 1) % 16),
            KeyCode::Left => self.bg = Color::Pal((bi + self.bg_limit - 1) % self.bg_limit),
            KeyCode::Right => self.bg = Color::Pal((bi + 1) % self.bg_limit),
            KeyCode::Tab | KeyCode::Char('#') if self.modern => self.hex_focus = true,
            _ => {}
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let s = self.swatches;
        let inside = |r: Rect| m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right();
        let MouseEventKind::Down(b) = m.kind else { return Outcome::Keep };
        if inside(s) {
            let i = ((m.row - s.y) / 2 * 8 + (m.column - s.x) / 6) as u8;
            // Right-click is only a shortcut for the background.
            if b == MouseButton::Right || self.slot == Slot::Bg {
                if i < self.bg_limit {
                    self.bg = Color::Pal(i);
                } else {
                    return Outcome::KeepThen(Box::new(|app: &mut App| {
                        app.flash("bright backgrounds need iCE colors (the iCE chip below)", Level::Warn)
                    }));
                }
            } else {
                self.fg = Color::Pal(i);
            }
        } else if inside(self.labels[0]) {
            self.slot = Slot::Fg;
        } else if inside(self.labels[1]) {
            self.slot = Slot::Bg;
        }
        Outcome::Keep
    }

    /// Clicking away keeps the colors picked (Esc is the way to cancel).
    fn click_outside(&mut self, _app: &App) -> Outcome {
        self.done()
    }

    fn paste(&mut self, s: &str) {
        if self.hex_focus {
            self.hex.paste(s);
        }
    }
}
