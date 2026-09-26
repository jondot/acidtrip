//! Pattern browser: every pattern as a swatch, built-in and saved. Click one
//! to pick it, click again (or Enter) to paint with it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::sidebar::draw_pattern;
use crate::ui::widgets::{centered, popup, theme};
use acidtrip_core::DocKind;

/// A card: the swatch and its name under it.
const CARD_W: u16 = 16;
const SWATCH_H: u16 = 3;
const CARD_H: u16 = SWATCH_H + 2;

pub struct PatternDialog {
    /// Index into the visible patterns.
    sel: usize,
    /// First visible row of cards.
    top: usize,
    cols: usize,
    cards: Vec<(Rect, usize)>,
    use_btn: Rect,
    delete_btn: Option<Rect>,
}

/// The patterns this document can use (indices into `app.tools.patterns`).
fn visible(app: &App) -> Vec<usize> {
    app.tools.visible_patterns(app.tab().doc.meta.kind == DocKind::Classic)
}

impl PatternDialog {
    pub fn new(app: &App) -> Self {
        let sel = app.tools.pattern_idx.and_then(|i| visible(app).iter().position(|&v| v == i)).unwrap_or(0);
        PatternDialog { sel, top: 0, cols: 1, cards: vec![], use_btn: Rect::default(), delete_btn: None }
    }

    fn pick(&self, app: &App) -> Outcome {
        let Some(&i) = visible(app).get(self.sel) else { return Outcome::Keep };
        Outcome::Then(Box::new(move |app: &mut App| {
            app.tools.select_pattern(i);
            let s = app.tools.option_summary();
            app.flash(format!("Pattern: {s}"), Level::Info);
        }))
    }

    fn delete(&self, app: &App) -> Outcome {
        let Some(&i) = visible(app).get(self.sel) else { return Outcome::Keep };
        let name = app.tools.patterns[i].name.clone();
        if !app.tools.user_patterns.contains(&name) {
            return Outcome::KeepThen(Box::new(|app: &mut App| {
                app.flash("built-in patterns can't be deleted", Level::Warn)
            }));
        }
        Outcome::KeepThen(Box::new(move |app: &mut App| {
            app.dialogs.push(Box::new(super::prompt::ConfirmDialog::new(
                &format!("Delete the pattern \"{name}\"?"),
                Box::new(move |app: &mut App| app.delete_pattern(name)),
            )));
        }))
    }
}

impl Dialog for PatternDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let vis = visible(app);
        self.sel = self.sel.min(vis.len().saturating_sub(1));
        let r = centered(area, area.width.saturating_sub(4).min(CARD_W * 5 + 4), area.height.saturating_sub(2).min(24));
        let inner = popup(f, r, "Patterns", "click · ←↑↓→ · Enter use · Ctrl-D delete · Esc");
        self.cols = ((inner.width.saturating_sub(1)) / CARD_W).max(1) as usize;
        let rows = (inner.height.saturating_sub(2) / CARD_H).max(1) as usize;
        let row = self.sel / self.cols;
        if row < self.top {
            self.top = row;
        }
        if row >= self.top + rows {
            self.top = row + 1 - rows;
        }
        let ts = &app.tools;
        let (fg, bg) = (ts.brush.fg, ts.brush.bg);
        self.cards.clear();
        for (n, &i) in vis.iter().enumerate().skip(self.top * self.cols).take(rows * self.cols) {
            let (cx, cy) = ((n % self.cols) as u16, (n / self.cols - self.top) as u16);
            let card = Rect::new(inner.x + 1 + cx * CARD_W, inner.y + cy * CARD_H, CARD_W - 2, CARD_H - 1);
            let p = &ts.patterns[i];
            draw_pattern(
                f.buffer_mut(),
                Rect::new(card.x, card.y, card.width, SWATCH_H),
                p,
                app.tab(),
                fg,
                bg,
                ts.opts.pattern_recolor,
            );
            let saved = ts.user_patterns.contains(&p.name);
            let label: String = format!("{}{}", if saved { "★ " } else { "" }, p.name).chars().take(card.width as usize).collect();
            let st = if n == self.sel {
                Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD)
            } else if ts.pattern_idx == Some(i) {
                Style::new().fg(theme::ACCENT2)
            } else {
                Style::new().fg(theme::TEXT)
            };
            f.render_widget(
                Paragraph::new(format!("{label:^w$}", w = card.width as usize)).style(st),
                Rect::new(card.x, card.y + SWATCH_H, card.width, 1),
            );
            self.cards.push((card, n));
        }
        // The pick, with what can be done to it.
        let y = inner.bottom().saturating_sub(1);
        let Some(&i) = vis.get(self.sel) else { return };
        let p = &ts.patterns[i];
        let saved = ts.user_patterns.contains(&p.name);
        let what = format!(
            " {} · {}x{} · {}{}",
            p.name,
            p.width,
            p.height,
            if saved { "saved" } else { "built-in" },
            if p.has_colors() { " · own colors" } else { "" }
        );
        let btn = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
        let del = Style::new().fg(theme::TEXT).bg(theme::PANEL_HI);
        let use_w = 7;
        let del_w = if saved { 10 } else { 0 };
        self.use_btn = Rect::new(inner.right().saturating_sub(use_w + del_w + 1), y, use_w, 1);
        self.delete_btn = saved.then(|| Rect::new(self.use_btn.right() + 1, y, del_w - 1, 1));
        let tw = self.use_btn.x.saturating_sub(inner.x);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(what, Style::new().fg(theme::DIM)))),
            Rect::new(inner.x, y, tw, 1),
        );
        f.render_widget(Paragraph::new(" ✓ use ").style(btn), self.use_btn);
        if let Some(d) = self.delete_btn {
            f.render_widget(Paragraph::new(" ✕ delete").style(del), d);
        }
    }

    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome {
        let n = visible(app).len();
        if n == 0 {
            return if k.code == KeyCode::Esc { Outcome::Close } else { Outcome::Keep };
        }
        let cols = self.cols.max(1);
        match k.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => return self.pick(app),
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => return self.delete(app),
            KeyCode::Delete => return self.delete(app),
            KeyCode::Left => self.sel = self.sel.checked_sub(1).unwrap_or(n - 1),
            KeyCode::Right => self.sel = (self.sel + 1) % n,
            KeyCode::Up if self.sel >= cols => self.sel -= cols,
            KeyCode::Down if self.sel + cols < n => self.sel += cols,
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.sel = n - 1,
            _ => {}
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, app: &App) -> Outcome {
        let inside = |r: Rect| m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom();
        match m.kind {
            MouseEventKind::Down(_) if inside(self.use_btn) => return self.pick(app),
            MouseEventKind::Down(_) if self.delete_btn.is_some_and(inside) => return self.delete(app),
            MouseEventKind::Down(_) => {
                if let Some(&(_, n)) = self.cards.iter().find(|(r, _)| inside(*r)) {
                    if n == self.sel {
                        return self.pick(app);
                    }
                    self.sel = n;
                }
            }
            MouseEventKind::ScrollDown => {
                let n = visible(app).len();
                self.sel = (self.sel + self.cols).min(n.saturating_sub(1));
            }
            MouseEventKind::ScrollUp => self.sel = self.sel.saturating_sub(self.cols),
            _ => {}
        }
        Outcome::Keep
    }
}
