//! Block menu (ACiDDraw Alt-B): operations on the current selection.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::actions::Action;
use crate::app::App;
use crate::ui::widgets::{ListState, list_line, popup};

const ITEMS: &[(char, Action)] = &[
    ('c', Action::Copy),
    ('x', Action::Cut),
    ('m', Action::MoveSelection),
    ('e', Action::DeleteSelection),
    ('f', Action::FillSelection),
    ('o', Action::OutlineSelection),
    ('h', Action::FlipX),
    ('v', Action::FlipY),
    ('r', Action::Rotate180),
    ('l', Action::JustifyLeft),
    ('n', Action::JustifyCenter),
    ('g', Action::JustifyRight),
    ('d', Action::DeleteBlock),
    ('k', Action::CropToSelection),
    ('s', Action::SaveStencil),
    ('p', Action::PatternFromSelection),
    ('a', Action::CopyAnsi),
    ('i', Action::AiPrompt),
];

pub struct BlockMenu {
    list: ListState,
    area: Rect,
}

impl BlockMenu {
    pub fn new(_app: &App) -> Self {
        BlockMenu { list: ListState::default(), area: Rect::default() }
    }

    fn run(i: usize) -> Outcome {
        let (_, a) = ITEMS[i];
        Outcome::Then(Box::new(move |app: &mut App| app.run(a)))
    }
}

fn label(key: char, a: Action) -> &'static str {
    match key {
        'm' => "Move (carry it)",
        'i' => "Ask AI about this block…",
        _ => a.title(),
    }
}

impl Dialog for BlockMenu {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let s = app.tab().selection.unwrap_or_default();
        let h = ITEMS.len() as u16 + 2;
        // Anchor near the selection when there is room.
        let x = (app.geom.area.x + s.right().saturating_sub(app.tab().scroll.0) as u16 + 2)
            .min(area.right().saturating_sub(40));
        let y = (app.geom.area.y + s.y.saturating_sub(app.tab().scroll.1) as u16).min(area.bottom().saturating_sub(h));
        let r = Rect::new(x, y, 40, h).intersection(area);
        let inner = popup(f, r, &format!("Block {}x{}", s.w, s.h), "key/Enter · Esc");
        self.area = inner;
        let lines: Vec<_> = ITEMS
            .iter()
            .enumerate()
            .map(|(i, (k, a))| list_line(label(*k, *a), k.to_string(), i == self.list.selected, inner.width))
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => Self::run(self.list.selected),
            KeyCode::Char(c) => match ITEMS.iter().position(|(key, _)| *key == c.to_ascii_lowercase()) {
                Some(i) => Self::run(i),
                None => Outcome::Keep,
            },
            _ => {
                self.list.key(&k, ITEMS.len(), 5);
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let r = self.area;
        match m.kind {
            MouseEventKind::Down(_)
                if m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right() =>
            {
                Self::run((m.row - r.y) as usize)
            }
            MouseEventKind::Down(_) => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}
