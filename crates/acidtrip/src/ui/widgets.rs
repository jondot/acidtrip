//! Theme and small reusable widgets: line input, filtered list, popups,
//! clickable buttons.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

pub mod theme {
    use ratatui::style::Color;

    pub const BG: Color = Color::Rgb(14, 14, 20);
    pub const PANEL: Color = Color::Rgb(24, 24, 34);
    pub const PANEL_HI: Color = Color::Rgb(40, 40, 58);
    pub const BORDER: Color = Color::Rgb(70, 70, 100);
    pub const TEXT: Color = Color::Rgb(210, 210, 220);
    pub const DIM: Color = Color::Rgb(120, 120, 145);
    pub const ACCENT: Color = Color::Rgb(255, 85, 255);
    pub const ACCENT2: Color = Color::Rgb(85, 255, 255);
    pub const OK: Color = Color::Rgb(85, 255, 85);
    pub const WARN: Color = Color::Rgb(255, 255, 85);
    pub const ERR: Color = Color::Rgb(255, 85, 85);
    pub const OUTSIDE: Color = Color::Rgb(9, 9, 13);
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let [r] = Layout::vertical([Constraint::Length(h)]).flex(Flex::Center).areas(area);
    let [r] = Layout::horizontal([Constraint::Length(w)]).flex(Flex::Center).areas(r);
    r
}

thread_local! {
    /// Everything popups covered since the last [`take_popup_area`].
    static POPUP_AREA: std::cell::Cell<Option<Rect>> = const { std::cell::Cell::new(None) };
}

/// The area the popups drawn since the last call cover (so a click outside
/// a dialog can be told from one inside it).
pub fn take_popup_area() -> Option<Rect> {
    POPUP_AREA.with(|a| a.take())
}

/// Draw a popup frame, returning the inner area.
pub fn popup(f: &mut Frame, area: Rect, title: &str, hint: &str) -> Rect {
    POPUP_AREA.with(|a| a.set(Some(a.get().map_or(area, |r| r.union(area)))));
    f.render_widget(Clear, area);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::ACCENT))
        .title(Span::styled(format!(" {title} "), Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)))
        .style(Style::new().bg(theme::PANEL).fg(theme::TEXT));
    // No hint, no gap in the border.
    if !hint.is_empty() {
        block = block
            .title_bottom(Line::from(Span::styled(format!(" {hint} "), Style::new().fg(theme::DIM))).right_aligned());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

/// Single-line text input with a cursor.
#[derive(Clone, Debug, Default)]
pub struct LineInput {
    pub text: String,
    /// Cursor position in chars.
    pub cursor: usize,
}

impl LineInput {
    pub fn new(text: &str) -> Self {
        LineInput { text: text.to_string(), cursor: text.chars().count() }
    }

    /// Returns true if the key was consumed.
    pub fn key(&mut self, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let len = self.text.chars().count();
        match k.code {
            KeyCode::Char('u') if ctrl => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = len,
            KeyCode::Char('w') if ctrl => {
                let chars: Vec<char> = self.text.chars().collect();
                let mut i = self.cursor;
                while i > 0 && chars[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && chars[i - 1] != ' ' {
                    i -= 1;
                }
                self.text = chars[..i].iter().chain(&chars[self.cursor..]).collect();
                self.cursor = i;
            }
            KeyCode::Char(c) if !ctrl => {
                let byte = self.byte_at(self.cursor);
                self.text.insert(byte, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let byte = self.byte_at(self.cursor - 1);
                self.text.remove(byte);
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < len => {
                let byte = self.byte_at(self.cursor);
                self.text.remove(byte);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            _ => return false,
        }
        true
    }

    pub fn paste(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            let byte = self.byte_at(self.cursor);
            self.text.insert(byte, c);
            self.cursor += 1;
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(self.text.len())
    }

    /// Render as one line; sets the terminal cursor when focused.
    pub fn render(&self, f: &mut Frame, area: Rect, label: &str, focused: bool) {
        let label_w = label.chars().count() as u16;
        let avail = area.width.saturating_sub(label_w + 1) as usize;
        let start = self.cursor.saturating_sub(avail.saturating_sub(1));
        let visible: String = self.text.chars().skip(start).take(avail).collect();
        let style =
            if focused { Style::new().fg(theme::TEXT).bg(theme::PANEL_HI) } else { Style::new().fg(theme::TEXT) };
        let line = Line::from(vec![
            Span::styled(label.to_string(), Style::new().fg(if focused { theme::ACCENT2 } else { theme::DIM })),
            Span::styled(format!("{visible:<avail$}"), style),
        ]);
        f.render_widget(Paragraph::new(line), area);
        if focused {
            f.set_cursor_position((area.x + label_w + (self.cursor - start) as u16, area.y));
        }
    }
}

/// Fuzzy score: every query char appears in order; bonus for word starts
/// and contiguous runs. None = no match.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut prev_match = None;
    for qc in query.to_lowercase().chars() {
        if qc == ' ' {
            continue;
        }
        let mut found = None;
        while ti < t.len() {
            if t[ti] == qc {
                found = Some(ti);
                break;
            }
            ti += 1;
        }
        let i = found?;
        score += 1;
        if i == 0 || !t[i - 1].is_alphanumeric() {
            score += 8;
        }
        if prev_match == Some(i.wrapping_sub(1)) {
            score += 5;
        }
        prev_match = Some(i);
        ti = i + 1;
    }
    // Contiguous substring matches beat scattered ones; word-start substrings
    // best. Words in the query match words in the text ("draw together").
    let q: String = query.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let hay: String = t.iter().collect();
    if let Some(pos) = hay.find(&q) {
        score += 1000;
        if pos == 0 || !hay[..pos].chars().last().is_some_and(char::is_alphanumeric) {
            score += 500;
        }
        // The very start (the title's first word) beats any later word.
        if pos == 0 {
            score += 200;
        }
    }
    Some(score * 100 - t.len() as i32)
}

/// A list with a selection cursor and scroll offset.
#[derive(Clone, Debug, Default)]
pub struct ListState {
    pub selected: usize,
    pub offset: usize,
}

impl ListState {
    pub fn key(&mut self, k: &KeyEvent, len: usize, page: usize) -> bool {
        if len == 0 {
            self.selected = 0;
            return matches!(k.code, KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown);
        }
        match k.code {
            KeyCode::Up => self.selected = self.selected.checked_sub(1).unwrap_or(len - 1),
            KeyCode::Down => self.selected = (self.selected + 1) % len,
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(page.max(1)),
            KeyCode::PageDown => self.selected = (self.selected + page.max(1)).min(len - 1),
            KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected = self.selected.checked_sub(1).unwrap_or(len - 1)
            }
            KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected = (self.selected + 1) % len
            }
            _ => return false,
        }
        true
    }

    /// Clamp and scroll for a viewport of `h` rows; returns visible range.
    pub fn visible(&mut self, len: usize, h: usize) -> std::ops::Range<usize> {
        if len == 0 {
            self.selected = 0;
            self.offset = 0;
            return 0..0;
        }
        self.selected = self.selected.min(len - 1);
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if h > 0 && self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
        self.offset..(self.offset + h).min(len)
    }
}

pub fn list_line<'a>(text: impl Into<String>, right: impl Into<String>, selected: bool, width: u16) -> Line<'a> {
    let text = text.into();
    let right = right.into();
    let w = width as usize;
    let rlen = right.chars().count();
    let tmax = w.saturating_sub(rlen + 2);
    let t: String = text.chars().take(tmax).collect();
    let pad = w.saturating_sub(t.chars().count() + rlen + 1);
    let style = if selected {
        Style::new().bg(theme::PANEL_HI).fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme::TEXT)
    };
    Line::from(vec![
        Span::styled(format!("{}{}", if selected { "▸" } else { " " }, t), style),
        Span::styled(" ".repeat(pad), style),
        Span::styled(right, if selected { style.fg(theme::ACCENT2) } else { Style::new().fg(theme::DIM) }),
    ])
}

/// RFC3339 timestamp → local "YYYY-MM-DD HH:MM".
pub fn local_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| ts.chars().take(16).collect::<String>().replace('T', " "))
}

pub fn key_hint(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {key} "), Style::new().fg(theme::BG).bg(theme::ACCENT2)),
        Span::styled(format!(" {label}  "), Style::new().fg(theme::DIM)),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BtnKind {
    Normal,
    /// The main thing to do next.
    Primary,
    /// The current choice in a set (tabs, toggles).
    On,
    /// Shown but not clickable right now.
    Off,
}

/// One clickable button: a key chip (optional) and a label.
pub struct Btn<'a, T> {
    pub id: T,
    pub key: &'a str,
    pub label: std::borrow::Cow<'a, str>,
    pub kind: BtnKind,
}

pub fn btn<'a, T>(id: T, key: &'a str, label: impl Into<std::borrow::Cow<'a, str>>) -> Btn<'a, T> {
    Btn { id, key, label: label.into(), kind: BtnKind::Normal }
}

impl<T> Btn<'_, T> {
    pub fn kind(mut self, kind: BtnKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(BtnKind::Primary)
    }

    /// Disabled unless `enabled`.
    pub fn enabled(self, enabled: bool) -> Self {
        if enabled { self } else { self.kind(BtnKind::Off) }
    }

    pub fn width(&self) -> u16 {
        let key = if self.key.is_empty() { 0 } else { self.key.chars().count() + 2 };
        (key + self.label.chars().count() + 2) as u16
    }
}

/// Buttons drawn this frame and where they landed, so clicks find them and
/// the one under the mouse lights up.
pub struct Buttons<T> {
    hits: Vec<(Rect, T)>,
    hover: Option<(u16, u16)>,
}

impl<T> Default for Buttons<T> {
    fn default() -> Self {
        Buttons { hits: vec![], hover: None }
    }
}

impl<T: Copy> Buttons<T> {
    /// Forget last frame's buttons (call before drawing).
    pub fn clear(&mut self) {
        self.hits.clear();
    }

    /// Draw `b` at (x, y) if it fits before `right`; returns its width (0 if
    /// it didn't fit).
    pub fn draw(&mut self, buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, right: u16, b: &Btn<T>) -> u16 {
        let w = b.width();
        if x + w > right {
            return 0;
        }
        let r = Rect::new(x, y, w, 1);
        let hot = b.kind != BtnKind::Off && self.hover.is_some_and(|(c, row)| row == y && c >= x && c < x + w);
        let (key_st, label_st) = match b.kind {
            BtnKind::Normal => (
                Style::new().fg(theme::BG).bg(theme::ACCENT2),
                Style::new().fg(theme::TEXT).bg(if hot { theme::BORDER } else { theme::PANEL_HI }),
            ),
            BtnKind::Primary => (
                Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD),
                Style::new()
                    .fg(Color::White)
                    .bg(if hot { theme::BORDER } else { theme::PANEL_HI })
                    .add_modifier(Modifier::BOLD),
            ),
            BtnKind::On => (
                Style::new().fg(theme::BG).bg(theme::ACCENT),
                Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ),
            BtnKind::Off => {
                (Style::new().fg(theme::DIM).bg(theme::PANEL_HI), Style::new().fg(theme::DIM).bg(theme::PANEL))
            }
        };
        let mut spans = vec![];
        if !b.key.is_empty() {
            spans.push(Span::styled(format!(" {} ", b.key), key_st));
        }
        spans.push(Span::styled(format!(" {} ", b.label), label_st));
        buf.set_line(x, y, &Line::from(spans), w);
        if b.kind != BtnKind::Off {
            self.hits.push((r, b.id));
        }
        w
    }

    /// Rows `btns` take laid out in `width` columns.
    pub fn rows_needed(btns: &[Btn<T>], width: u16) -> u16 {
        let (mut x, mut rows) = (0u16, 1u16);
        for b in btns {
            if x > 0 && x + b.width() > width {
                (x, rows) = (0, rows + 1);
            }
            x += b.width() + 1;
        }
        rows
    }

    /// Lay `btns` out left to right from `area`'s top-left, wrapping onto
    /// the next rows while `area` has them. Returns the rows used.
    pub fn row(&mut self, buf: &mut ratatui::buffer::Buffer, area: Rect, btns: &[Btn<T>]) -> u16 {
        let (mut x, mut y) = (area.x, area.y);
        for b in btns {
            if x > area.x && x + b.width() > area.right() {
                if y + 1 >= area.bottom() {
                    break;
                }
                (x, y) = (area.x, y + 1);
            }
            x += self.draw(buf, x, y, area.right(), b) + 1;
        }
        if area.height == 0 { 0 } else { y - area.y + 1 }
    }

    pub fn at(&self, col: u16, row: u16) -> Option<T> {
        self.hits.iter().find(|(r, _)| row == r.y && col >= r.x && col < r.right()).map(|(_, id)| *id)
    }

    /// Track the hover position; a left click returns the button under it.
    pub fn mouse(&mut self, m: &MouseEvent) -> Option<T> {
        match m.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                self.hover = Some((m.column, m.row));
                None
            }
            MouseEventKind::Down(MouseButton::Left) => self.at(m.column, m.row),
            _ => None,
        }
    }
}

/// Word-wrap `text` to `width` columns (a word longer than the width is cut).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = vec![];
    let mut line = String::new();
    for word in text.split_whitespace() {
        let wl = word.chars().count();
        let ll = line.chars().count();
        if ll > 0 && ll + 1 + wl > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word.chars().take(width).collect::<String>());
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_substrings() {
        let a = fuzzy("ice", "Toggle iCE colors Document").unwrap();
        let b = fuzzy("ice", "Insert column Edit").unwrap();
        assert!(a > b);
    }

    #[test]
    fn fuzzy_multi_word_queries_match_as_phrases() {
        let a = fuzzy("draw together", "Draw together: live drawing with others… Together").unwrap();
        let b = fuzzy("draw together", "Host: share this drawing live Together").unwrap();
        assert!(a > b, "{a} vs {b}");
        let c = fuzzy("add  layer", "Add layer Layers").unwrap();
        let d = fuzzy("add layer", "Add reference image layer… File").unwrap();
        assert!(c > d, "{c} vs {d}");
    }

    #[test]
    fn fuzzy_prefers_word_starts() {
        let a = fuzzy("sv", "Save").unwrap();
        let b = fuzzy("sv", "Version history").unwrap_or(i32::MIN);
        assert!(a > b);
        assert!(fuzzy("xyz", "Save").is_none());
    }

    #[test]
    fn fuzzy_prefers_the_first_word() {
        let a = fuzzy("harvest", "Harvest fonts & stencils from art… AI").unwrap();
        let b = fuzzy("harvest", "My harvested fonts… AI").unwrap();
        assert!(a > b);
    }

    #[test]
    fn line_input_edits() {
        let mut i = LineInput::new("helo");
        i.key(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        i.key(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert_eq!(i.text, "hello");
        i.key(&KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(i.text, "");
    }
}
