//! AI prompt bar and AI/MCP setup info.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::widgets::{LineInput, ListState, centered, list_line, popup, theme};

const SUGGESTIONS: &[&str] = &[
    "make a logo that says ",
    "colorize the selection with a fire gradient",
    "shade the selection like chrome",
    "add a starfield in the background",
    "draw a small skull in the corner",
    "put a double-line frame around everything",
    "critique this piece and fix one thing",
];

pub struct AiPromptDialog {
    input: LineInput,
    list: ListState,
    area: Rect,
}

impl AiPromptDialog {
    pub fn new(_app: &App) -> Self {
        AiPromptDialog {
            input: LineInput::default(),
            list: ListState { selected: usize::MAX, offset: 0 },
            area: Rect::default(),
        }
    }

    fn submit(&self) -> Outcome {
        let prompt = self.input.text.trim().to_string();
        if prompt.is_empty() {
            return Outcome::Keep;
        }
        Outcome::Then(Box::new(move |app: &mut App| {
            if app.agent.is_some() {
                app.flash("the AI is still finishing its last run — try again in a moment", Level::Warn);
                return;
            }
            app.start_agent(prompt);
        }))
    }
}

impl Dialog for AiPromptDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let r = centered(area, 84, SUGGESTIONS.len() as u16 + 8);
        let inner = popup(f, r, "Ask AI", "Enter run · ↑↓ suggestions · Esc");
        self.input.render(f, Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1), "✦ ", true);
        let sel = app
            .tab()
            .selection
            .map(|s| format!("working on the selection {}x{} at {},{}", s.w, s.h, s.x, s.y))
            .unwrap_or_else(|| "working on the whole canvas (select an area to focus it)".into());
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{sel} · edits go to the AI layer · one Ctrl-Z undoes the run"),
                Style::new().fg(theme::DIM),
            )),
            Rect::new(inner.x + 1, inner.y + 2, inner.width.saturating_sub(2), 1),
        );
        self.area = Rect::new(inner.x + 1, inner.y + 4, inner.width.saturating_sub(2), SUGGESTIONS.len() as u16);
        let lines: Vec<_> = SUGGESTIONS
            .iter()
            .enumerate()
            .map(|(i, s)| list_line(*s, "", i == self.list.selected, self.area.width))
            .collect();
        f.render_widget(Paragraph::new(lines), self.area);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.submit(),
            KeyCode::Up | KeyCode::Down => {
                let n = SUGGESTIONS.len();
                self.list.selected = match (k.code, self.list.selected) {
                    (KeyCode::Down, usize::MAX) => 0,
                    (KeyCode::Up, usize::MAX) => n - 1,
                    (KeyCode::Down, i) => (i + 1) % n,
                    (_, i) => (i + n - 1) % n,
                };
                self.input = LineInput::new(SUGGESTIONS[self.list.selected]);
                Outcome::Keep
            }
            _ => {
                self.input.key(&k);
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let r = self.area;
        if let MouseEventKind::Down(_) = m.kind
            && m.row >= r.y
            && m.row < r.bottom()
            && m.column >= r.x
            && m.column < r.right()
        {
            let i = (m.row - r.y) as usize;
            self.list.selected = i;
            self.input = LineInput::new(SUGGESTIONS[i]);
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        self.input.paste(s);
    }
}

pub struct AiSetupDialog {
    lines: Vec<Line<'static>>,
}

impl AiSetupDialog {
    pub fn new(app: &App) -> Self {
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "acidtrip".into());
        let has_key = app.config.api_key().is_some();
        let live = app.live.as_ref().map(|l| l.path.display().to_string());
        let h = |s: &str| {
            Line::from(Span::styled(s.to_string(), Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)))
        };
        let t = |s: String| Line::from(Span::styled(s, Style::new().fg(theme::TEXT)));
        let d = |s: String| Line::from(Span::styled(s, Style::new().fg(theme::DIM)));
        let code = |s: String| Line::from(Span::styled(format!("  {s}"), Style::new().fg(theme::ACCENT2)));
        let mut lines = vec![h("1. Prompt bar (A or Ctrl-/)")];
        if has_key {
            lines.push(Line::from(Span::styled("  ✓ API key found".to_string(), Style::new().fg(theme::OK))));
            lines.push(d(format!("  model: {}", app.config.ai.model)));
        } else {
            lines.push(Line::from(Span::styled("  ✗ no API key".to_string(), Style::new().fg(theme::WARN))));
            lines.push(t("  Set it in your shell, then restart acidtrip:".into()));
            lines.push(code("export ANTHROPIC_API_KEY=sk-ant-…".into()));
            lines.push(t(format!("  or put it in {} under [ai] api_key", app.paths.config_file().display())));
        }
        lines.push(Line::from(""));
        lines.push(h("2. Claude Code draws in this editor (MCP, no key needed)"));
        lines.push(t("  Register once:".into()));
        lines.push(code(format!("claude mcp add acidtrip -- {exe} mcp")));
        lines.push(t("  Then ask Claude Code e.g. \"draw a neon logo in acidtrip\" —".into()));
        lines.push(t("  its edits appear here live, each one undoable.".into()));
        match live {
            Some(p) => lines.push(d(format!("  live socket: {p}"))),
            None => lines.push(Line::from(Span::styled(
                "  ✗ live socket not running".to_string(),
                Style::new().fg(theme::WARN),
            ))),
        }
        lines.push(Line::from(""));
        lines.push(h("3. Harvest fonts & stencils from scene art"));
        lines.push(t("  Palette › Harvest, or `acidtrip harvest <pack|url|dir>`".into()));
        AiSetupDialog { lines }
    }
}

impl Dialog for AiSetupDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, 92, self.lines.len() as u16 + 4);
        let inner = popup(f, r, "AI setup", "C copy MCP command · any key closes");
        f.render_widget(
            Paragraph::new(self.lines.clone()).wrap(Wrap { trim: false }),
            inner.inner(ratatui::layout::Margin::new(1, 1)),
        );
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        if matches!(k.code, KeyCode::Char('c') | KeyCode::Char('C')) {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "acidtrip".into());
            let cmd = format!("claude mcp add acidtrip -- {exe} mcp");
            return Outcome::Then(Box::new(move |app: &mut App| match crate::share::copy_text(&cmd) {
                Ok(()) => app.flash("MCP command copied — paste it in your shell", Level::Ok),
                Err(e) => app.flash(format!("copy failed: {e}"), Level::Error),
            }));
        }
        Outcome::Close
    }
}
