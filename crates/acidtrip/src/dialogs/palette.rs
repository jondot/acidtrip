//! Ctrl-K command palette: fuzzy search over every action.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::actions::Action;
use crate::app::App;
use crate::ui::widgets::{LineInput, ListState, centered, fuzzy, list_line, popup, theme};

pub struct CommandPalette {
    input: LineInput,
    list: ListState,
    items: Vec<(Action, String)>,
    filtered: Vec<usize>,
    list_area: Rect,
}

impl CommandPalette {
    pub fn new(app: &App) -> Self {
        let items: Vec<(Action, String)> = Action::ALL
            .iter()
            .filter(|a| !matches!(a.category(), "Cursor"))
            .map(|a| (*a, app.keymap.key_for(*a).unwrap_or_default()))
            .collect();
        let mut p = CommandPalette {
            input: LineInput::default(),
            list: ListState::default(),
            filtered: vec![],
            items,
            list_area: Rect::default(),
        };
        p.refilter();
        p
    }

    fn refilter(&mut self) {
        self.filtered = search(&self.items, self.input.text.trim());
        self.list.selected = 0;
    }

    fn chosen(&self) -> Option<Action> {
        self.filtered.get(self.list.selected).map(|&i| self.items[i].0)
    }
}

/// Indices of `items` matching `q`, best first. A command is found by its
/// title, its category, or its config name (`snapshot`, `save_as`), which is
/// what `acidtrip keys` and config.toml call it.
fn search(items: &[(Action, String)], q: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, (a, _))| {
            let hay = format!("{} {}", a.title(), a.category());
            let by_id = || fuzzy(q, &a.id().replace('_', " ")).map(|s| s - 1);
            fuzzy(q, &hay).or_else(by_id).map(|s| (s, i))
        })
        .collect();
    if !q.is_empty() {
        scored.sort_by_key(|x| std::cmp::Reverse(x.0));
    }
    scored.into_iter().map(|(_, i)| i).collect()
}

impl Dialog for CommandPalette {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, 64, 22);
        let inner = popup(f, r, "Commands", "↑↓ select · Enter run · Esc close");
        self.input.render(f, Rect::new(inner.x, inner.y, inner.width, 1), "› ", true);
        let lr = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(2));
        self.list_area = lr;
        let range = self.list.visible(self.filtered.len(), lr.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let (a, key) = &self.items[self.filtered[i]];
                list_line(format!("{}  ·{}", a.title(), a.category()), key.clone(), i == self.list.selected, lr.width)
            })
            .collect();
        if self.filtered.is_empty() {
            let msg = format!("no command matches \"{}\" · Ctrl-U clears", self.input.text.trim());
            f.render_widget(Paragraph::new(Span::styled(msg, Style::new().fg(theme::DIM))), lr);
        } else {
            f.render_widget(Paragraph::new(lines), lr);
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => match self.chosen() {
                Some(a) => Outcome::Then(Box::new(move |app: &mut App| app.run(a))),
                None => Outcome::Keep,
            },
            _ => {
                if self.list.key(&k, self.filtered.len(), 10) {
                    return Outcome::Keep;
                }
                if self.input.key(&k) {
                    self.refilter();
                }
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let r = self.list_area;
        match m.kind {
            MouseEventKind::Down(_)
                if m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom() =>
            {
                let i = self.list.offset + (m.row - r.y) as usize;
                if i < self.filtered.len() {
                    self.list.selected = i;
                    if let Some(a) = self.chosen() {
                        return Outcome::Then(Box::new(move |app: &mut App| app.run(a)));
                    }
                }
                Outcome::Keep
            }
            MouseEventKind::ScrollDown => {
                self.list.selected = (self.list.selected + 1).min(self.filtered.len().saturating_sub(1));
                Outcome::Keep
            }
            MouseEventKind::ScrollUp => {
                self.list.selected = self.list.selected.saturating_sub(1);
                Outcome::Keep
            }
            // a click inside the box but off the list (the search line, the
            // empty rows) keeps it open; outside clicks close it upstream
            _ => Outcome::Keep,
        }
    }

    fn paste(&mut self, s: &str) {
        self.input.paste(s);
        self.refilter();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_found_by_config_name() {
        let items: Vec<(Action, String)> = Action::ALL.iter().map(|a| (*a, String::new())).collect();
        let hits = search(&items, "snapshot");
        assert_eq!(hits.first().map(|&i| items[i].0), Some(Action::SnapshotVersion));
        // a title match still wins over a config-name match
        let hits = search(&items, "save as");
        assert_eq!(hits.first().map(|&i| items[i].0), Some(Action::SaveAs));
        assert!(search(&items, "qqqzzzxx").is_empty());
    }
}
