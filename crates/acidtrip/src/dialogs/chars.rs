//! Character picker: the full CP437 grid (Classic) plus a Unicode block
//! browser for Modern docs.

use acidtrip_core::{DocKind, cp437};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::App;
use crate::ui::widgets::{centered, popup, theme};

const UNICODE_EXTRA: &str =
    "╭╮╰╯━┃┏┓┗┛┣┫┳┻╋▁▂▃▅▆▇▉▊▋▍▎▏▔▕▖▗▘▙▚▛▜▝▞▟◢◣◤◥◆◇○●◐◑◒◓★☆✦✧✶✹❖❤♪♫☀☁☂☃☄⚡⌁⌂⎔⏣⏢▰▱▲△▼▽◀▶⠁⠃⠇⡇⣇⣧⣷⣿🬀🬁🬂🬃🬄🬅🬆🬇🬈🬉🬊🬋";

pub struct CharPicker {
    chars: Vec<char>,
    sel: usize,
    grid: Rect,
    cols: usize,
    /// Choosing the glyph for this art-mode key rather than the brush.
    assign: Option<char>,
}

impl CharPicker {
    pub fn new(app: &App) -> Self {
        let mut chars: Vec<char> = (0..=255u8).map(cp437::to_char).map(|c| if c == '\0' { ' ' } else { c }).collect();
        if app.tab().doc.meta.kind == DocKind::Modern {
            chars.extend(UNICODE_EXTRA.chars());
        }
        let sel = chars.iter().position(|&c| c == app.tools.brush.ch).unwrap_or(219);
        CharPicker { chars, sel, grid: Rect::default(), cols: 32, assign: None }
    }

    /// Pick the glyph an art-mode key types.
    pub fn assign(app: &App, key: char) -> Self {
        let mut p = CharPicker::new(app);
        if let Some(g) = app.artboard.glyph(key, app.tab().doc.meta.kind == DocKind::Classic) {
            p.sel = p.chars.iter().position(|&c| c == g).unwrap_or(p.sel);
        }
        p.assign = Some(key);
        p
    }

    fn pick(&self) -> Outcome {
        let ch = self.chars[self.sel];
        if let Some(key) = self.assign {
            return Outcome::Then(Box::new(move |app: &mut App| app.set_art_key(key, ch)));
        }
        Outcome::Then(Box::new(move |app: &mut App| {
            if app.typing_mode() {
                app.type_char(ch);
            } else {
                app.pick_glyph(ch);
            }
        }))
    }
}

impl Dialog for CharPicker {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let rows = self.chars.len().div_ceil(self.cols) as u16;
        let r = centered(area, self.cols as u16 * 2 + 4, rows + 6);
        let title = match self.assign {
            Some(k) => format!("Art mode: what {} types", k.to_ascii_uppercase()),
            None => "Characters".into(),
        };
        let inner = popup(f, r, &title, "arrows · Enter pick · click · Esc");
        self.grid = Rect::new(inner.x + 1, inner.y + 1, self.cols as u16 * 2, rows);
        let b = app.tools.brush;
        let pal = &app.tab().doc.meta.palette;
        let (fg, bg) = (crate::ui::canvas::rgb(b.fg, pal), crate::ui::canvas::rgb(b.bg, pal));
        let mut lines = vec![];
        for row in 0..rows as usize {
            let mut spans = vec![];
            for col in 0..self.cols {
                let i = row * self.cols + col;
                let Some(&c) = self.chars.get(i) else { break };
                let st = if i == self.sel {
                    Style::new().fg(theme::BG).bg(theme::ACCENT2)
                } else {
                    Style::new().fg(fg).bg(bg)
                };
                spans.push(Span::styled(format!("{c} "), st));
            }
            lines.push(Line::from(spans));
        }
        f.render_widget(Paragraph::new(lines), self.grid);
        let c = self.chars[self.sel];
        let code =
            cp437::from_char(c).map(|b| format!("CP437 {b} (0x{b:02X})")).unwrap_or_else(|| "not in CP437".into());
        f.render_widget(
            Paragraph::new(Span::styled(format!("'{c}'  U+{:04X}  {code}", c as u32), Style::new().fg(theme::DIM))),
            Rect::new(inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), 1),
        );
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let n = self.chars.len();
        match k.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter | KeyCode::Char(' ') => return self.pick(),
            KeyCode::Left => self.sel = (self.sel + n - 1) % n,
            KeyCode::Right => self.sel = (self.sel + 1) % n,
            KeyCode::Up => self.sel = (self.sel + n - self.cols.min(n)) % n,
            KeyCode::Down => self.sel = (self.sel + self.cols) % n,
            KeyCode::Char(c) => {
                if let Some(i) = self.chars.iter().position(|&x| x == c) {
                    self.sel = i;
                }
            }
            _ => {}
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let g = self.grid;
        if let MouseEventKind::Down(_) = m.kind
            && m.row >= g.y
            && m.row < g.bottom()
            && m.column >= g.x
            && m.column < g.right()
        {
            let i = (m.row - g.y) as usize * self.cols + (m.column - g.x) as usize / 2;
            if i < self.chars.len() {
                self.sel = i;
                return self.pick();
            }
        }
        Outcome::Keep
    }
}
