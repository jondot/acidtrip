//! Help / keys cheat sheet, generated from the live keymap. Also the
//! first-run welcome screen.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Dialog, Outcome};
use crate::actions::Action;
use crate::app::App;
use crate::ui::widgets::{centered, hint_fits_border, popup, theme};

const HINT: &str = "↑↓ PgDn scroll · any other key closes";
const SHORT_HINT: &str = "↑↓ scroll · Esc closes";

pub struct HelpDialog {
    welcome: bool,
    scroll: u16,
    /// Rows on screen and the furthest scroll that still shows content,
    /// both from the last draw.
    view: u16,
    max_scroll: u16,
}

impl HelpDialog {
    pub fn new() -> Self {
        HelpDialog { welcome: false, scroll: 0, view: 0, max_scroll: u16::MAX }
    }

    pub fn welcome() -> Self {
        HelpDialog { welcome: true, ..Self::new() }
    }

    fn scroll_by(&mut self, d: i32) {
        self.scroll = (self.scroll as i32 + d).clamp(0, self.max_scroll as i32) as u16;
    }
}

/// Rows `lines` take when word-wrapped to `width`.
fn height(lines: &[Line], width: u16) -> u16 {
    let w = width.max(1) as usize;
    let rows = |l: &Line| {
        let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        let (mut rows, mut cur, mut first) = (1u16, 0usize, true);
        for word in text.split(' ') {
            let n = word.chars().count();
            if !first && cur + 1 + n > w {
                rows += 1;
                cur = 0;
                first = true;
            }
            cur += if first { n } else { 1 + n };
            first = false;
            while cur > w {
                rows += 1;
                cur -= w;
            }
        }
        rows
    };
    lines.iter().map(rows).sum()
}

fn row(key: String, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:>13} "), Style::new().fg(theme::ACCENT2)),
        Span::styled(label.to_string(), Style::new().fg(theme::TEXT)),
    ])
}

/// Wraps key rows that are too wide under their description, not under the
/// key. Other lines are left to the paragraph's own wrapping.
fn fit(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    const KEY: usize = 14;
    let room = (width as usize).saturating_sub(KEY).max(10);
    let mut out = vec![];
    for line in lines {
        let [key, label] = &line.spans[..] else {
            out.push(line);
            continue;
        };
        if key.content.chars().count() != KEY || label.content.chars().count() <= room {
            out.push(line);
            continue;
        }
        let mut rows: Vec<String> = vec![String::new()];
        for word in label.content.split(' ') {
            let cur = rows.last_mut().unwrap();
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > room {
                rows.push(word.to_string());
            } else {
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(word);
            }
        }
        for (i, text) in rows.into_iter().enumerate() {
            let k = if i == 0 { key.clone() } else { Span::raw(" ".repeat(KEY)) };
            out.push(Line::from(vec![k, Span::styled(text, label.style)]));
        }
    }
    out
}

fn head(s: &str) -> Line<'static> {
    Line::from(Span::styled(s.to_string(), Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)))
}

impl Dialog for HelpDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        // As tall as the screen allows; the width stays clear of the sidebar.
        let r = centered(area, 100, area.height.saturating_sub(4).clamp(34, 50));
        let title = if self.welcome { "Welcome to acidtrip" } else { "Help" };
        // The short hint on a narrow terminal, so it keeps to the border.
        let hint = [HINT, SHORT_HINT].into_iter().find(|h| hint_fits_border(h, r.width)).unwrap_or(SHORT_HINT);
        let inner = popup(f, r, title, hint);
        let km = &app.keymap;
        let k = |a: Action| km.key_for(a).unwrap_or_default();
        let mut left = vec![];
        if self.welcome {
            left.push(Line::from(Span::styled(
                "ANSI art, the ACiDDraw way — with a mouse.",
                Style::new().fg(theme::TEXT),
            )));
            left.push(Line::from(""));
            left.push(Line::from(Span::styled("Just start: drag on the canvas to paint.", Style::new().fg(theme::OK))));
            left.push(Line::from(Span::styled(
                format!(
                    "{} toggles the eraser. {}-{} type tiles, Shift+arrows draw.",
                    k(Action::ToolErase),
                    k(Action::Glyph1),
                    k(Action::Glyph10)
                ),
                Style::new().fg(theme::DIM),
            )));
            left.push(Line::from(Span::styled(
                format!("Need inspiration? {} opens the Gallery of scene art.", k(Action::Gallery)),
                Style::new().fg(theme::DIM),
            )));
            left.push(Line::from(""));
        }
        left.push(head("DRAW"));
        for a in [
            Action::ToolBrush,
            Action::ToolText,
            Action::ToolPen,
            Action::ToolPixel,
            Action::ToolLine,
            Action::ToolRect,
            Action::ToolEllipse,
            Action::ToolFill,
            Action::ToolGradient,
            Action::ToolPattern,
            Action::ToolShade,
            Action::ToolColorize,
            Action::ToolPicker,
            Action::ToolSelect,
            Action::ToolFont,
            Action::ToolStencil,
            Action::ToolFilters,
            Action::ToolRecolor,
        ] {
            left.push(row(k(a), a.title()));
        }
        left.push(row(k(Action::ToolErase), "eraser on/off (or Option/right-drag)"));
        left.push(row(k(Action::ToolOption), "cycle tool option (pen: next brush)"));
        left.push(row(k(Action::BrushStudio), "brush studio: presets + sliders"));
        left.push(row(format!("{} {}", k(Action::BrushSmaller), k(Action::BrushBigger)), "pen brush size"));
        left.push(row(k(Action::ToolStyle), "shape look (pen: prev brush)"));
        left.push(row(k(Action::Mirror), "mirror / symmetry"));
        left.push(row(k(Action::Apply), "apply at cursor (arrows)"));
        left.push(row("Shift-arrows".into(), "draw a trail with the tile"));
        left.push(Line::from(""));
        left.push(head("COLOR & CHARACTERS"));
        left.push(row("click".into(), "FG/BG box picks what the palette sets"));
        left.push(row(format!("{} {}", k(Action::FgPrev), k(Action::FgNext)), "foreground"));
        left.push(row(format!("{} {}", k(Action::BgPrev), k(Action::BgNext)), "background"));
        left.push(row(k(Action::SwapColors), "swap fg/bg"));
        left.push(row(
            format!("{}-{}", k(Action::Glyph1), k(Action::Glyph10)),
            "place glyph at cursor (ACiDDraw bar)",
        ));
        left.push(row(format!("{} {}", k(Action::CharsetPrev), k(Action::CharsetNext)), "switch character set"));
        left.push(row(k(Action::CharPicker), "all characters"));
        left.push(row(k(Action::ArtMode), "Art tool: keys type blocks, [ ] sets, R erases"));
        left.push(row(k(Action::ToggleIce), "iCE colors (16 backgrounds)"));

        let mut right = vec![head("FILE & SHARE")];
        for a in [
            Action::New,
            Action::Open,
            Action::Gallery,
            Action::Save,
            Action::SaveAs,
            Action::Export,
            Action::ExportNow,
            Action::ExportAs,
            Action::Share,
            Action::Versions,
            Action::Quit,
        ] {
            right.push(row(k(a), a.title()));
        }
        right.push(row("Ctrl-T".into(), "Gallery ⇄ Studio tab (in the Gallery)"));
        right.push(Line::from(""));
        right.push(head("EDIT"));
        for a in [
            Action::Undo,
            Action::Redo,
            Action::Copy,
            Action::Cut,
            Action::Paste,
            Action::SelectAll,
            Action::BlockMenu,
            Action::InsertLine,
            Action::DeleteLine,
            Action::InsertColumn,
            Action::DeleteColumn,
        ] {
            right.push(row(k(a), a.title()));
        }
        right.push(Line::from(""));
        right.push(head("VIEW & MORE"));
        for a in [
            Action::Zoom,
            Action::Preview,
            Action::PlayBaud,
            Action::Replay,
            Action::TogetherPanel,
            Action::Sidebar,
            Action::LayerUp,
            Action::LayerAdd,
            Action::FrameAdd,
            Action::FrameNext,
            Action::FramePlay,
            Action::AiPrompt,
            Action::CommandPalette,
        ] {
            right.push(row(k(a), a.title()));
        }
        right.push(Line::from(""));
        right.push(Line::from(Span::styled(
            format!("Keymap preset: {:?} · change in settings (palette › Open settings file)", app.keymap.preset),
            Style::new().fg(theme::DIM),
        )));
        if !app.enhanced_keys {
            right.push(Line::from(Span::styled(
                "Tip: Alt keys need \"Option as Meta/Esc+\" in your terminal profile.",
                Style::new().fg(theme::DIM),
            )));
        }
        let body = inner.inner(ratatui::layout::Margin::new(1, 1));
        let [l, r2] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(2).areas(body);
        let wrap = Wrap { trim: false };
        let (left, right) = (fit(left, l.width), fit(right, r2.width));
        self.view = body.height;
        self.max_scroll = height(&left, l.width).max(height(&right, r2.width)).saturating_sub(body.height);
        self.scroll = self.scroll.min(self.max_scroll);
        f.render_widget(Paragraph::new(left).wrap(wrap).scroll((self.scroll, 0)), l);
        f.render_widget(Paragraph::new(right).wrap(wrap).scroll((self.scroll, 0)), r2);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Down => {
                self.scroll_by(3);
                Outcome::Keep
            }
            KeyCode::Up => {
                self.scroll_by(-3);
                Outcome::Keep
            }
            KeyCode::PageDown | KeyCode::PageUp => {
                let page = self.view.saturating_sub(2).max(3) as i32;
                self.scroll_by(if k.code == KeyCode::PageDown { page } else { -page });
                Outcome::Keep
            }
            KeyCode::Home => {
                self.scroll = 0;
                Outcome::Keep
            }
            KeyCode::End => {
                self.scroll = self.max_scroll;
                Outcome::Keep
            }
            _ => Outcome::Close,
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match m.kind {
            MouseEventKind::ScrollDown => {
                self.scroll_by(2);
                Outcome::Keep
            }
            MouseEventKind::ScrollUp => {
                self.scroll_by(-2);
                Outcome::Keep
            }
            MouseEventKind::Down(_) => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_stops_at_the_last_line() {
        let lines = vec![Line::from("x".repeat(25)), Line::from(""), Line::from("short")];
        assert_eq!(height(&lines, 10), 3 + 1 + 1);
        let mut h = HelpDialog::new();
        h.max_scroll = 4;
        h.scroll_by(3);
        h.scroll_by(3);
        assert_eq!(h.scroll, 4);
        h.scroll_by(-9);
        assert_eq!(h.scroll, 0);
    }
}
