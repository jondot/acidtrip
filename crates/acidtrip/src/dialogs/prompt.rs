//! Simple text prompt and yes/no confirm.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::{Dialog, Outcome};
use crate::app::App;
use crate::ui::widgets::{Buttons, LineInput, btn, centered, key_hint, popup, theme, wrap};

pub type StrCallback = Box<dyn FnOnce(&mut App, String)>;

pub struct PromptDialog {
    title: String,
    input: LineInput,
    on_ok: Option<StrCallback>,
    /// "⏎ ok" / "esc cancel", so a mouse alone can answer.
    btns: Buttons<bool>,
}

impl PromptDialog {
    pub fn new(title: &str, initial: &str, on_ok: StrCallback) -> Self {
        PromptDialog { title: title.into(), input: LineInput::new(initial), on_ok: Some(on_ok), btns: Buttons::default() }
    }

    fn ok(&mut self) -> Outcome {
        let text = self.input.text.clone();
        match self.on_ok.take() {
            Some(cb) => Outcome::Then(Box::new(move |app: &mut App| cb(app, text))),
            None => Outcome::Close,
        }
    }
}

impl Dialog for PromptDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, 60, 6);
        let inner = popup(f, r, &self.title, "Enter ok · Esc cancel");
        self.input.render(f, Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1), "› ", true);
        self.btns.clear();
        if inner.height >= 4 {
            let bar = [btn(true, "⏎", "ok").primary(), btn(false, "esc", "cancel")];
            let row = Rect::new(inner.x + 1, inner.bottom() - 1, inner.width.saturating_sub(2), 1);
            self.btns.row(f.buffer_mut(), row, &bar);
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.ok(),
            _ => {
                self.input.key(&k);
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match self.btns.mouse(&m) {
            Some(true) => self.ok(),
            Some(false) => Outcome::Close,
            None => Outcome::Keep,
        }
    }

    fn paste(&mut self, s: &str) {
        self.input.paste(s);
    }
}

pub struct ConfirmDialog {
    text: String,
    on_yes: Option<super::Callback>,
    /// Where "Y yes" and "N/Esc no" landed, so a click answers.
    yes: Rect,
    no: Rect,
}

impl ConfirmDialog {
    pub fn new(text: &str, on_yes: super::Callback) -> Self {
        ConfirmDialog { text: text.into(), on_yes: Some(on_yes), yes: Rect::default(), no: Rect::default() }
    }
}

impl Dialog for ConfirmDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        // the wrapped text, a gap, the answers, inside the border
        let h = wrap(&self.text, 58).len() as u16 + 4;
        let r = centered(area, 62, h);
        let inner = popup(f, r, "Confirm", "");
        let [body, keys] = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Min(1),
            ratatui::layout::Constraint::Length(1),
        ])
        .areas(inner);
        f.render_widget(
            Paragraph::new(self.text.clone()).wrap(Wrap { trim: true }).style(Style::new().fg(theme::TEXT)),
            body.inner(ratatui::layout::Margin::new(1, 0)),
        );
        let mut spans = key_hint("Y", "yes");
        let yes_w = spans.iter().map(|s| s.width() as u16).sum::<u16>();
        let no = key_hint("N/Esc", "no");
        let no_w = no.iter().map(|s| s.width() as u16).sum::<u16>();
        spans.extend(no);
        self.yes = Rect::new(keys.x, keys.y, yes_w, 1);
        self.no = Rect::new(keys.x + yes_w, keys.y, no_w, 1);
        f.render_widget(Paragraph::new(Line::from(spans)), keys);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => match self.on_yes.take() {
                Some(cb) => Outcome::Then(cb),
                None => Outcome::Close,
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Outcome::Close,
            _ => Outcome::Keep,
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let inside = |r: Rect| m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom();
        match m.kind {
            MouseEventKind::Down(_) if inside(self.yes) => match self.on_yes.take() {
                Some(cb) => Outcome::Then(cb),
                None => Outcome::Close,
            },
            MouseEventKind::Down(_) if inside(self.no) => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}
