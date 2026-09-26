//! Crash recovery prompt shown at startup when autosaves exist.

use acidtrip_io::recovery::{self, RecoveryEntry};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::tab::Tab;
use crate::ui::widgets::{Buttons, ListState, btn, centered, list_line, popup, theme};

pub struct RecoveryDialog {
    entries: Vec<RecoveryEntry>,
    list: ListState,
    list_area: Rect,
    btns: Buttons<Do>,
}

/// The buttons under the list, so one mouse button answers.
#[derive(Clone, Copy)]
enum Do {
    Restore,
    Discard,
    Later,
}

impl RecoveryDialog {
    pub fn new(entries: Vec<RecoveryEntry>) -> Self {
        RecoveryDialog { entries, list: ListState::default(), list_area: Rect::default(), btns: Buttons::default() }
    }

    fn act(&self, d: Do) -> Outcome {
        match d {
            Do::Restore => match self.entries.get(self.list.selected) {
                Some(e) => restore(e.clone()),
                None => Outcome::Close,
            },
            Do::Discard => {
                let entries = self.entries.clone();
                Outcome::Then(Box::new(move |app: &mut App| {
                    let dir = app.paths.recovery_dir();
                    for e in &entries {
                        recovery::clear(&dir, e.doc_id);
                    }
                    app.flash("discarded recovery files", Level::Info);
                }))
            }
            Do::Later => Outcome::Close,
        }
    }
}

/// One document at a time: restore the chosen one; others stay recoverable.
fn restore(e: RecoveryEntry) -> Outcome {
    Outcome::Then(Box::new(move |app: &mut App| match recovery::load(&e) {
        Ok(doc) => {
            let mut t = Tab::new(doc, e.file.clone());
            // Keep it dirty so the user saves it.
            t.history.mark_saved();
            t.edit("Recovered", |b| b.replace_meta(|_| {}));
            // The recovery file stays until the work is saved (or undone
            // back to the file), so a second crash doesn't lose it.
            t.recovery_written = true;
            app.tabs = vec![t];
            app.active = 0;
            app.flash("recovered — save it to keep it", Level::Ok);
        }
        Err(err) => app.flash(format!("recovery failed: {err:#}"), Level::Error),
    }))
}

impl Dialog for RecoveryDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, 76, self.entries.len().min(12) as u16 + 7);
        let inner = popup(f, r, "Recover unsaved work", "Enter or click restore · D discard all · Esc later");
        f.render_widget(
            Paragraph::new(Span::styled(
                "acidtrip didn't close cleanly — these documents had unsaved changes:",
                Style::new().fg(theme::WARN),
            )),
            Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
        );
        let lr = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(4));
        self.list_area = lr;
        let range = self.list.visible(self.entries.len(), lr.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let e = &self.entries[i];
                let name = e.file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "untitled".into());
                let when = crate::ui::widgets::local_time(&e.saved_at);
                list_line(name, when, i == self.list.selected, lr.width)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), lr);
        self.btns.clear();
        let btns = [
            btn(Do::Restore, "⏎", "restore").primary(),
            btn(Do::Discard, "D", "discard all"),
            btn(Do::Later, "esc", "later"),
        ];
        let bar = Rect::new(inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), 1);
        self.btns.row(f.buffer_mut(), bar, &btns);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Char('r') | KeyCode::Enter => self.act(Do::Restore),
            KeyCode::Char('d') | KeyCode::Char('D') => self.act(Do::Discard),
            KeyCode::Esc => self.act(Do::Later),
            _ => {
                self.list.key(&k, self.entries.len(), 5);
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        if let Some(d) = self.btns.mouse(&m) {
            return self.act(d);
        }
        let r = self.list_area;
        if let MouseEventKind::Down(_) = m.kind
            && m.row >= r.y
            && m.row < r.bottom()
            && m.column >= r.x
            && m.column < r.right()
        {
            let i = self.list.offset + (m.row - r.y) as usize;
            if i < self.entries.len() {
                self.list.selected = i;
            }
        }
        Outcome::Keep
    }
}
