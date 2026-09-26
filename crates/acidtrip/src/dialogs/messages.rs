//! Message history: every status-bar message of the session, whole. The
//! status bar cuts a long one short; this is where it can be read.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::widgets::{centered, popup, theme, wrap_words};

/// Columns the age takes in front of each message.
const AGE_W: usize = 5;

pub struct MessagesDialog {
    /// Rows scrolled off the top; starts at the newest (the end).
    scroll: usize,
    view: usize,
    max_scroll: usize,
}

impl MessagesDialog {
    pub fn new() -> Self {
        MessagesDialog { scroll: usize::MAX, view: 0, max_scroll: 0 }
    }

    fn scroll_by(&mut self, d: i64) {
        self.scroll = (self.scroll.min(self.max_scroll) as i64 + d).clamp(0, self.max_scroll as i64) as usize;
    }
}

/// How long ago, short: "4s", "12m", "3h".
fn age(at: Instant) -> String {
    let s = at.elapsed().as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

/// The messages, oldest first, each wrapped to `width` under its age.
fn rows(log: &[(String, Level, Instant)], width: usize) -> Vec<Line<'static>> {
    let text_w = width.saturating_sub(AGE_W + 1).max(8);
    let mut out = vec![];
    for (text, level, at) in log {
        let c = match level {
            Level::Info => theme::TEXT,
            Level::Ok => theme::OK,
            Level::Warn => theme::WARN,
            Level::Error => theme::ERR,
        };
        for (i, row) in wrap_words(text, text_w).into_iter().enumerate() {
            let head = if i == 0 { format!("{:>AGE_W$} ", age(*at)) } else { " ".repeat(AGE_W + 1) };
            out.push(Line::from(vec![
                Span::styled(head, Style::new().fg(theme::DIM)),
                Span::styled(row, Style::new().fg(c)),
            ]));
        }
    }
    out
}

impl Dialog for MessagesDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let r = centered(area, area.width.saturating_sub(2).min(90), area.height.saturating_sub(4).clamp(8, 30));
        let inner = popup(f, r, "Messages", "↑↓ scroll · Esc close");
        let body = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
        if app.msg_log.is_empty() {
            let none = Span::styled("No messages yet.", Style::new().fg(theme::DIM));
            f.render_widget(Paragraph::new(none), body);
            return;
        }
        let lines = rows(&app.msg_log, body.width as usize);
        self.view = body.height as usize;
        self.max_scroll = lines.len().saturating_sub(self.view);
        self.scroll = self.scroll.min(self.max_scroll);
        let shown: Vec<Line> = lines.into_iter().skip(self.scroll).take(self.view).collect();
        f.render_widget(Paragraph::new(shown), body);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let page = self.view.saturating_sub(1).max(1) as i64;
        match k.code {
            KeyCode::Up => self.scroll_by(-1),
            KeyCode::Down => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::PageDown => self.scroll_by(page),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => return Outcome::Close,
            _ => {}
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match m.kind {
            MouseEventKind::ScrollDown => self.scroll_by(2),
            MouseEventKind::ScrollUp => self.scroll_by(-2),
            _ => {}
        }
        Outcome::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_wrap_under_their_age() {
        let now = Instant::now();
        let log = vec![
            ("saved".to_string(), Level::Ok, now),
            ("a message far too long for the status bar, read here whole".to_string(), Level::Warn, now),
        ];
        let lines = rows(&log, 30);
        let text: Vec<String> =
            lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect();
        assert_eq!(text[0], "   0s saved");
        assert!(text.len() > 3, "{text:?}");
        assert!(text[2].starts_with("      ") && !text[2].trim().is_empty());
        let whole: Vec<&str> = text[1..].iter().map(|t| t[AGE_W + 1..].trim()).collect();
        assert_eq!(whole.join(" "), log[1].0);
        assert!(text.iter().all(|t| t.chars().count() <= 30));
    }

    #[test]
    fn scroll_starts_at_the_newest_and_stays_in_range() {
        let mut d = MessagesDialog::new();
        d.max_scroll = 5;
        d.scroll_by(1);
        assert_eq!(d.scroll, 5);
        d.scroll_by(-9);
        assert_eq!(d.scroll, 0);
    }
}
