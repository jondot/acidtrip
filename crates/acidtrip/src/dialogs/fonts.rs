//! Font text stamp: pick a TheDraw/FIGlet font, type text, see it live,
//! Enter to carry it as a floating stamp.

use acidtrip_core::{Clip, Palette};
use acidtrip_io::fonts::{FontInfo, FontKind, TextRenderOptions};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::tools_ctl::FloatSource;
use crate::ui::canvas::draw_clip;
use crate::actions::Action;
use crate::ui::widgets::{Buttons, LineInput, ListState, btn, centered, fuzzy, list_line, popup, theme};

pub struct FontDialog {
    text: LineInput,
    filter: LineInput,
    filter_focus: bool,
    fonts: Vec<FontInfo>,
    filtered: Vec<usize>,
    list: ListState,
    outline: usize,
    preview: Option<Result<Clip, String>>,
    preview_key: (usize, String, usize),
    list_area: Rect,
    filter_area: Rect,
    text_area: Rect,
    preview_area: Rect,
    /// No TheDraw fonts installed yet (only the bundled FIGlet ones): offer
    /// the download under the preview.
    offer_download: bool,
    btns: Buttons<Get>,
}

/// The download button's id.
#[derive(Clone, Copy)]
struct Get;

impl FontDialog {
    pub fn new(app: &App) -> Self {
        let fonts = app.fonts.list();
        let offer_download = !fonts.iter().any(|f| matches!(f.kind, FontKind::TdfBlock | FontKind::TdfColor | FontKind::TdfOutline));
        let mut d = FontDialog {
            // Empty: the preview shows a sample until the user types, so
            // typing replaces it instead of appending to it.
            text: LineInput::default(),
            filter: LineInput::default(),
            filter_focus: false,
            filtered: (0..fonts.len()).collect(),
            fonts,
            list: ListState::default(),
            outline: 0,
            preview: None,
            preview_key: (usize::MAX, String::new(), 0),
            list_area: Rect::default(),
            filter_area: Rect::default(),
            text_area: Rect::default(),
            preview_area: Rect::default(),
            offer_download,
            btns: Buttons::default(),
        };
        d.refilter();
        d
    }

    fn refilter(&mut self) {
        let q = self.filter.text.clone();
        let mut s: Vec<(i32, usize)> =
            self.fonts.iter().enumerate().filter_map(|(i, f)| fuzzy(&q, &f.name).map(|x| (x, i))).collect();
        if !q.is_empty() {
            s.sort_by_key(|x| std::cmp::Reverse(x.0));
        }
        self.filtered = s.into_iter().map(|(_, i)| i).collect();
        self.list.selected = 0;
    }

    /// Select the font loaded from `path` (clearing the filter).
    pub fn select_path(&mut self, path: &std::path::Path) {
        self.filter = LineInput::default();
        self.refilter();
        if let Some(i) = self.filtered.iter().position(|&i| self.fonts[i].path.as_deref() == Some(path)) {
            self.list.selected = i;
        }
    }

    fn current(&self) -> Option<&FontInfo> {
        self.filtered.get(self.list.selected).map(|&i| &self.fonts[i])
    }

    /// What the preview shows: the typed text, or a sample until there is some.
    fn shown_text(&self) -> &str {
        if self.text.text.is_empty() { "ACiD" } else { &self.text.text }
    }

    fn render(&mut self, app: &App) {
        let Some(f) = self.current() else {
            self.preview = None;
            return;
        };
        let key = (self.filtered[self.list.selected], self.text.text.clone(), self.outline);
        if key == self.preview_key {
            return;
        }
        let opts = TextRenderOptions {
            outline_style: self.outline,
            fg: app.tools.brush.fg,
            bg: app.tools.brush.bg,
            spacing: 0,
        };
        let text = self.shown_text().to_string();
        self.preview = Some(app.fonts.render(&f.id, &text, &opts).map_err(|e| format!("{e:#}")));
        self.preview_key = key;
    }

    /// Carry the preview as a floating stamp.
    fn stamp(&mut self, app: &App) -> Outcome {
        self.render(app);
        match &self.preview {
            Some(Ok(clip)) if clip.width > 0 => {
                let clip = clip.clone();
                Outcome::Then(Box::new(move |app: &mut App| app.float(clip, FloatSource::Font)))
            }
            _ => {
                let msg = match self.current().map(|f| missing(&f.charset, self.shown_text())) {
                    Some(m) if !m.is_empty() => format!("nothing to stamp — this font has no {m}"),
                    _ => "nothing to stamp".to_string(),
                };
                Outcome::KeepThen(Box::new(move |app: &mut App| app.flash(msg, Level::Warn)))
            }
        }
    }
}

/// Close, download the TheDraw pack, and open the dialog again on the new list.
fn get_fonts() -> Outcome {
    Outcome::Then(Box::new(|app: &mut App| {
        app.run(Action::GetFonts);
        app.run(Action::ToolFont);
    }))
}

/// The letters of `text` a font with `charset` can't draw (either case
/// counts), each once: "H, E, L".
fn missing(charset: &str, text: &str) -> String {
    let has = |c: char| {
        charset.contains(c) || c.to_uppercase().chain(c.to_lowercase()).any(|o| charset.contains(o))
    };
    let mut out: Vec<char> = vec![];
    for c in text.chars().filter(|c| !c.is_whitespace() && !has(*c)) {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out.iter().map(char::to_string).collect::<Vec<_>>().join(", ")
}

fn kind_name(k: FontKind) -> &'static str {
    match k {
        FontKind::TdfBlock => "TDF block",
        FontKind::TdfColor => "TDF color",
        FontKind::TdfOutline => "TDF outline",
        FontKind::Figlet => "FIGlet",
        FontKind::Cut => "cut from art",
    }
}

impl Dialog for FontDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        self.render(app);
        let r = centered(area, area.width.saturating_sub(4).min(130), area.height.saturating_sub(2).min(34));
        let inner = popup(
            f,
            r,
            "Font text",
            "type text · ↑↓ font · Tab: filter · ←→ outline style · Enter or click the preview: stamp · Esc",
        );
        let [left, right] = Layout::horizontal([Constraint::Length(34), Constraint::Min(20)]).areas(inner);
        self.filter_area = Rect::new(left.x, left.y, left.width, 1);
        self.filter.render(f, self.filter_area, "Find ", self.filter_focus);
        let lr = Rect::new(left.x, left.y + 1, left.width.saturating_sub(1), left.height.saturating_sub(1));
        self.list_area = lr;
        let range = self.list.visible(self.filtered.len(), lr.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let fi = &self.fonts[self.filtered[i]];
                list_line(fi.name.clone(), kind_name(fi.kind), i == self.list.selected, lr.width)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), lr);

        self.text_area = Rect::new(right.x + 1, right.y, right.width.saturating_sub(2), 1);
        self.text.render(f, self.text_area, "Text ", !self.filter_focus);
        let mut info = format!("{} fonts", self.fonts.len());
        let mut info_style = Style::new().fg(theme::DIM);
        let gaps = self.current().filter(|fi| !fi.charset.is_empty()).map(|fi| missing(&fi.charset, self.shown_text()));
        if let Some(m) = gaps.filter(|m| !m.is_empty()) {
            info = format!("no {m} in this font — they're left out");
            info_style = Style::new().fg(theme::WARN);
        } else if let Some(fi) = self.current() {
            if fi.kind == FontKind::TdfOutline {
                info = format!("outline style {} (←→)", self.outline);
            } else if !fi.charset.is_empty() {
                info = format!(
                    "chars: {}",
                    fi.charset.chars().filter(|c| !c.is_whitespace()).take(60).collect::<String>()
                );
            }
        }
        f.render_widget(
            Paragraph::new(Span::styled(info, info_style)),
            Rect::new(right.x + 1, right.y + 1, right.width.saturating_sub(2), 1),
        );
        let offer_rows = if self.offer_download { 2 } else { 0 };
        let pr = Rect::new(
            right.x + 1,
            right.y + 3,
            right.width.saturating_sub(2),
            right.height.saturating_sub(3 + offer_rows),
        );
        self.btns.clear();
        if self.offer_download && right.height > 5 {
            let y = right.bottom() - 1;
            let w = self.btns.draw(
                f.buffer_mut(),
                right.x + 1,
                y,
                right.right().saturating_sub(1),
                &btn(Get, "^G", "Get TheDraw fonts").primary(),
            );
            let x = right.x + 1 + w + 1;
            f.render_widget(
                Paragraph::new(Span::styled(
                    "~1,200 fonts from the 90s ANSI scene (3.7 MB download)",
                    Style::new().fg(theme::DIM),
                )),
                Rect::new(x, y, right.right().saturating_sub(x + 1), 1),
            );
        }
        self.preview_area = pr;
        match &self.preview {
            Some(Ok(clip)) => draw_clip(f.buffer_mut(), pr, clip, &Palette::default()),
            Some(Err(e)) => {
                f.render_widget(Paragraph::new(Line::from(Span::styled(e.clone(), Style::new().fg(theme::ERR)))), pr)
            }
            None => {
                let msg = if self.fonts.is_empty() {
                    "No fonts yet — Get TheDraw fonts below".to_string()
                } else {
                    format!("no font matches \u{201c}{}\u{201d} — Tab to change the filter", self.filter.text)
                };
                f.render_widget(Paragraph::new(Span::styled(msg, Style::new().fg(theme::DIM))), pr)
            }
        }
        let _ = app;
    }

    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc if self.filter_focus => {
                self.filter_focus = false;
                Outcome::Keep
            }
            KeyCode::Esc => Outcome::Close,
            // '/' is text like any other key: Tab (or a click) gets to the filter.
            KeyCode::Tab | KeyCode::BackTab => {
                self.filter_focus = !self.filter_focus;
                Outcome::Keep
            }
            KeyCode::Enter if self.filter_focus => {
                self.filter_focus = false;
                Outcome::Keep
            }
            KeyCode::Enter => self.stamp(app),
            KeyCode::Char('g')
                if self.offer_download && k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                get_fonts()
            }
            KeyCode::Left
                if !self.filter_focus
                    && self.current().is_some_and(|f| f.kind == FontKind::TdfOutline)
                    && self.text.cursor == 0 =>
            {
                self.outline = (self.outline + 18) % 19;
                Outcome::Keep
            }
            KeyCode::Right
                if !self.filter_focus
                    && self.current().is_some_and(|f| f.kind == FontKind::TdfOutline)
                    && self.text.cursor == self.text.text.chars().count() =>
            {
                self.outline = (self.outline + 1) % 19;
                Outcome::Keep
            }
            _ => {
                if self.list.key(&k, self.filtered.len(), 10) {
                    return Outcome::Keep;
                }
                if self.filter_focus {
                    if self.filter.key(&k) {
                        self.refilter();
                    }
                } else {
                    self.text.key(&k);
                }
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, app: &App) -> Outcome {
        if self.btns.mouse(&m).is_some() {
            return get_fonts();
        }
        let r = self.list_area;
        let inside = |r: Rect| m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right();
        match m.kind {
            MouseEventKind::Down(_) if inside(r) => {
                let i = self.list.offset + (m.row - r.y) as usize;
                // A click on the selected font stamps it, as a click on the preview does.
                if i == self.list.selected && i < self.filtered.len() {
                    return self.stamp(app);
                }
                if i < self.filtered.len() {
                    self.list.selected = i;
                }
            }
            MouseEventKind::Down(_) if inside(self.preview_area) => return self.stamp(app),
            MouseEventKind::Down(_) if inside(self.filter_area) => self.filter_focus = true,
            MouseEventKind::Down(_) if inside(self.text_area) => self.filter_focus = false,
            MouseEventKind::ScrollDown => {
                self.list.selected = (self.list.selected + 1).min(self.filtered.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => self.list.selected = self.list.selected.saturating_sub(1),
            _ => {}
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        self.text.paste(s);
    }
}

#[cfg(test)]
mod tests {
    use super::missing;

    #[test]
    fn missing_letters_once_either_case() {
        assert_eq!(missing("ABCabc", "CAB cab"), "");
        assert_eq!(missing("ABC", "cab"), "");
        assert_eq!(missing("abc", "ABC"), "");
        assert_eq!(missing("ABC", "HELLO ABBA"), "H, E, L, O");
        assert_eq!(missing("", "hi"), "h, i");
    }
}
