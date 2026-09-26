//! Stencil library browser: search, preview, stamp, delete.

use acidtrip_core::Palette;
use acidtrip_io::stencils::StencilMeta;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::tools_ctl::FloatSource;
use crate::ui::canvas::draw_clip;
use crate::ui::widgets::{Buttons, LineInput, ListState, btn, centered, list_line, popup, theme};

pub struct StencilDialog {
    query: LineInput,
    items: Vec<StencilMeta>,
    list: ListState,
    list_area: Rect,
    btns: Buttons<Act>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Stamp,
    Delete,
    Close,
}

impl StencilDialog {
    pub fn new(app: &App) -> Self {
        StencilDialog {
            query: LineInput::default(),
            items: app.stencils.list(),
            list: ListState::default(),
            list_area: Rect::default(),
            btns: Buttons::default(),
        }
    }

    /// The dialog again after a delete: same search, the row that took the
    /// deleted one's place selected.
    fn after_delete(app: &App, query: &str, selected: usize) -> Self {
        let mut d = StencilDialog::new(app);
        d.query = LineInput::new(query);
        d.refilter(app);
        d.list.selected = selected.min(d.items.len().saturating_sub(1));
        d
    }

    fn selected_builtin(&self, app: &App) -> bool {
        self.items.get(self.list.selected).is_some_and(|m| app.stencils.is_builtin(&m.id))
    }

    fn delete(&self) -> Outcome {
        let Some(m) = self.items.get(self.list.selected).cloned() else {
            return Outcome::Keep;
        };
        let (query, selected) = (self.query.text.clone(), self.list.selected);
        Outcome::KeepThen(Box::new(move |app: &mut App| {
            let dir = app.paths.stencils_dir();
            match app.stencils.delete(&dir, &m.id) {
                Ok(()) => app.flash(format!("deleted stencil {}", m.name), Level::Ok),
                Err(e) => app.flash(format!("{e:#}"), Level::Warn),
            }
            // Refresh the open dialog's list.
            if app.dialogs.pop().is_some() {
                let d = StencilDialog::after_delete(app, &query, selected);
                app.dialogs.push(Box::new(d));
            }
        }))
    }

    fn refilter(&mut self, app: &App) {
        self.items = app.stencils.search(&self.query.text);
        self.list.selected = 0;
    }

    fn stamp(&self) -> Outcome {
        match self.items.get(self.list.selected) {
            Some(m) => {
                let id = m.id.clone();
                Outcome::Then(Box::new(move |app: &mut App| match app.stencils.get(&id) {
                    Some(s) => app.float(s.clip, FloatSource::Stencil),
                    None => app.flash("stencil vanished", Level::Error),
                }))
            }
            None => Outcome::Keep,
        }
    }
}

impl Dialog for StencilDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let r = centered(area, area.width.saturating_sub(4).min(120), area.height.saturating_sub(2).min(32));
        let inner = popup(f, r, "Stencils", "type to search · ↑↓ · Enter or click again: stamp · Ctrl-D delete · Esc");
        let [left, right] = Layout::horizontal([Constraint::Length(40), Constraint::Min(20)]).areas(inner);
        self.query.render(f, Rect::new(left.x, left.y, left.width, 1), "› ", true);
        // Buttons under the list, so one mouse button does it all.
        self.btns.clear();
        let any = !self.items.is_empty();
        let bar = [
            btn(Act::Stamp, "⏎", "stamp").primary().enabled(any),
            btn(Act::Delete, "^D", "delete").enabled(any && !self.selected_builtin(app)),
            btn(Act::Close, "esc", "close"),
        ];
        let bar_h = Buttons::rows_needed(&bar, left.width).min(2);
        self.btns.row(f.buffer_mut(), Rect::new(left.x, left.bottom().saturating_sub(bar_h), left.width, bar_h), &bar);
        let lr = Rect::new(
            left.x,
            left.y + 2,
            left.width.saturating_sub(1),
            left.height.saturating_sub(3 + bar_h),
        );
        self.list_area = lr;
        let range = self.list.visible(self.items.len(), lr.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let m = &self.items[i];
                let tag = if m.generated {
                    "ai"
                } else if m.source == "built-in" {
                    "built-in"
                } else {
                    ""
                };
                list_line(m.name.clone(), tag, i == self.list.selected, lr.width)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), lr);
        if let Some(m) = self.items.get(self.list.selected) {
            let mut meta = vec![Line::from(Span::styled(m.name.clone(), Style::new().fg(theme::ACCENT2)))];
            let by = [m.author.as_str(), m.group.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" / ");
            if !by.is_empty() {
                meta.push(Line::from(Span::styled(format!("by {by}"), Style::new().fg(theme::TEXT))));
            }
            if !m.tags.is_empty() {
                meta.push(Line::from(Span::styled(
                    format!("tags: {}", m.tags.join(", ")),
                    Style::new().fg(theme::DIM),
                )));
            }
            meta.push(Line::from(Span::styled(
                format!("source: {} · {}", m.source, m.license),
                Style::new().fg(theme::DIM),
            )));
            let mh = meta.len() as u16 + 1;
            f.render_widget(
                Paragraph::new(meta).wrap(Wrap { trim: true }),
                Rect::new(right.x + 1, right.y, right.width.saturating_sub(2), mh),
            );
            if let Some(s) = app.stencils.get(&m.id) {
                let pr = Rect::new(
                    right.x + 1,
                    right.y + mh,
                    right.width.saturating_sub(2),
                    right.height.saturating_sub(mh),
                );
                draw_clip(f.buffer_mut(), pr, &s.clip, &Palette::default());
            }
        }
    }

    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.stamp(),
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => self.delete(),
            _ => {
                if !self.list.key(&k, self.items.len(), 10) && self.query.key(&k) {
                    self.refilter(app);
                }
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match self.btns.mouse(&m) {
            Some(Act::Stamp) => return self.stamp(),
            Some(Act::Delete) => return self.delete(),
            Some(Act::Close) => return Outcome::Close,
            None => {}
        }
        let r = self.list_area;
        match m.kind {
            MouseEventKind::Down(_)
                if m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right() =>
            {
                let i = self.list.offset + (m.row - r.y) as usize;
                if i < self.items.len() {
                    if self.list.selected == i {
                        return self.stamp();
                    }
                    self.list.selected = i;
                }
            }
            MouseEventKind::ScrollDown => {
                self.list.selected = (self.list.selected + 1).min(self.items.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => self.list.selected = self.list.selected.saturating_sub(1),
            _ => {}
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        self.query.paste(s);
    }
}
