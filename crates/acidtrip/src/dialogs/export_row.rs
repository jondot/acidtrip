//! One EXPORT panel row, opened from its name or format: the file name
//! pattern, the format, the scale and the format's options. Enter keeps the
//! changes as one undo step; Esc drops them.

use acidtrip_core::ExportPreset;
use acidtrip_io::exports::{self, FORMATS};
use acidtrip_io::format::{Format, SaveOptions};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::export::{cycle_option, option_rows};
use super::{Dialog, Outcome};
use crate::app::App;
use crate::ui::widgets::{BtnKind, Buttons, LineInput, btn, centered, popup, theme};

/// Placeholders the name buttons insert.
const TOKENS: [&str; 5] = ["{name}", "{scale}", "{frame}", "{w}", "{h}"];

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Name,
    Format,
    Scale,
    Options,
}

#[derive(Clone, Copy, PartialEq)]
enum Do {
    Token(usize),
    Format(Format),
    Scale(u32),
    Opt(usize),
    Done,
    Cancel,
}

pub struct ExportRowDialog {
    index: usize,
    row: ExportPreset,
    opts: SaveOptions,
    /// Options were changed here (else the row keeps following the piece).
    touched: bool,
    name: LineInput,
    focus: Focus,
    opt_idx: usize,
    animated: bool,
    /// For the "writes …" preview.
    doc_name: String,
    size: (usize, usize),
    frames: usize,
    btns: Buttons<Do>,
    name_area: Rect,
}

impl ExportRowDialog {
    /// Edit row `index`; `format_first` puts the keyboard on the formats.
    pub fn new(app: &App, index: usize, format_first: bool) -> Self {
        let tab = app.tab();
        let row = crate::exporter::rows(tab).get(index).cloned().unwrap_or_default();
        let opts = exports::options(&row, &tab.doc);
        ExportRowDialog {
            index,
            name: LineInput::new(&row.name),
            touched: !row.options.is_null(),
            row,
            opts,
            focus: if format_first { Focus::Format } else { Focus::Name },
            opt_idx: 0,
            animated: tab.doc.is_animated(),
            doc_name: crate::exporter::name(tab),
            size: crate::exporter::size(tab),
            frames: tab.doc.frame_count(),
            btns: Buttons::default(),
            name_area: Rect::default(),
        }
    }

    fn format(&self) -> Format {
        exports::format_of(&self.row).unwrap_or(Format::Png)
    }

    /// The options shown here: scale has its own chips, and the identifier
    /// is typed in the Export as… dialog.
    fn options(&self) -> Vec<(&'static str, String)> {
        option_rows(&self.opts, self.format(), self.animated)
            .into_iter()
            .filter(|(l, _)| !matches!(*l, "Scale" | "Identifier"))
            .collect()
    }

    fn focus_order(&self) -> Vec<Focus> {
        let mut v = vec![Focus::Name, Focus::Format];
        if exports::has_scale(self.format()) {
            v.push(Focus::Scale);
        }
        if !self.options().is_empty() {
            v.push(Focus::Options);
        }
        v
    }

    fn step_focus(&mut self, back: bool) {
        let order = self.focus_order();
        let n = order.len();
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = order[if back { (i + n - 1) % n } else { (i + 1) % n }];
    }

    fn set_format(&mut self, f: Format) {
        self.row.format = exports::format_id(f);
        if !exports::has_scale(f) {
            self.sync_name();
            exports::set_scale(&mut self.row, 1);
            self.name = LineInput::new(&self.row.name);
        }
        self.opt_idx = self.opt_idx.min(self.options().len().saturating_sub(1));
    }

    fn set_scale(&mut self, s: u32) {
        self.sync_name();
        exports::set_scale(&mut self.row, s);
        self.name = LineInput::new(&self.row.name);
    }

    fn sync_name(&mut self) {
        self.row.name = self.name.text.clone();
    }

    fn cycle(&mut self, i: usize, dir: i32) {
        if let Some((label, _)) = self.options().get(i).cloned() {
            self.opt_idx = i;
            self.focus = Focus::Options;
            cycle_option(&mut self.opts, label, dir);
            self.touched = true;
        }
    }

    fn step_format(&mut self, dir: i32) {
        let n = FORMATS.len() as i32;
        let i = FORMATS.iter().position(|f| *f == self.format()).unwrap_or(0) as i32;
        self.set_format(FORMATS[(i + dir).rem_euclid(n) as usize]);
    }

    fn done(&mut self) -> Outcome {
        self.sync_name();
        let mut row = self.row.clone();
        if row.name.trim().is_empty() {
            row.name = exports::auto_name(row.scale);
        }
        if self.touched {
            self.opts.scale = row.scale;
            exports::set_options(&mut row, &self.opts);
        }
        let i = self.index;
        Outcome::Then(Box::new(move |app: &mut App| {
            app.edit_export_rows("Export settings", |rows| {
                if let Some(r) = rows.get_mut(i) {
                    *r = row;
                }
            });
        }))
    }

    /// What the row writes, as the name reads now.
    fn preview(&self) -> String {
        let pad = self.frames.to_string().len();
        let file = |frame: Option<&str>| {
            let v =
                exports::Vars { name: &self.doc_name, scale: self.row.scale, w: self.size.0, h: self.size.1, frame };
            exports::file_name(&exports::expand(&self.name.text, &v), self.format())
        };
        if self.name.text.contains("{frame}") && self.frames > 1 {
            let (first, last) = (format!("{:0pad$}", 1), self.frames.to_string());
            format!("{} … {} ({} files)", file(Some(&first)), file(Some(&last)), self.frames)
        } else {
            file(self.name.text.contains("{frame}").then_some("1"))
        }
    }
}

impl Dialog for ExportRowDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let fmt = self.format();
        let opts = self.options();
        let scaled = exports::has_scale(fmt);
        let r = centered(area, 66, 19 + u16::from(opts.len() > 4));
        let inner = popup(f, r, &format!("Export row {}", self.index + 1), "Enter keep · Esc cancel · Tab next");
        self.btns.clear();
        let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
        let dim = Style::new().fg(theme::DIM);
        let head =
            |on: bool| Style::new().fg(if on { theme::ACCENT2 } else { theme::TEXT }).add_modifier(Modifier::BOLD);
        let mut y = inner.y;
        let buf_line = |f: &mut Frame, y: u16, l: Line| f.render_widget(Paragraph::new(l), Rect::new(x, y, w, 1));

        // Name.
        buf_line(f, y, Line::from(Span::styled("File name", head(self.focus == Focus::Name))));
        y += 1;
        self.name_area = Rect::new(x, y, w, 1);
        self.name.render(f, self.name_area, "› ", self.focus == Focus::Name);
        y += 1;
        buf_line(
            f,
            y,
            Line::from(vec![Span::styled("writes ", dim), Span::styled(self.preview(), Style::new().fg(theme::OK))]),
        );
        y += 1;
        let mut bx = x;
        let lw = "insert ".len() as u16;
        buf_line(f, y, Line::from(Span::styled("insert ", dim)));
        bx += lw;
        for (i, t) in TOKENS.iter().enumerate() {
            bx += self.btns.draw(f.buffer_mut(), bx, y, x + w, &btn(Do::Token(i), "", *t)) + 1;
        }
        y += 2;

        // Format.
        buf_line(
            f,
            y,
            Line::from(vec![
                Span::styled("Format  ", head(self.focus == Focus::Format)),
                Span::styled(fmt.name(), Style::new().fg(theme::TEXT)),
            ]),
        );
        y += 1;
        let chips: Vec<_> = FORMATS
            .iter()
            .map(|&g| {
                let b = btn(Do::Format(g), "", exports::short_label(g));
                if g == fmt { b.kind(BtnKind::On) } else { b }
            })
            .collect();
        y += self.btns.row(f.buffer_mut(), Rect::new(x, y, w, 3), &chips) + 1;

        // Scale.
        let scale_line = if scaled {
            vec![Span::styled("Scale   ", head(self.focus == Focus::Scale))]
        } else {
            vec![Span::styled("Scale   ", dim), Span::styled(format!("{} has no pixel scale", fmt.name()), dim)]
        };
        buf_line(f, y, Line::from(scale_line));
        if scaled {
            let mut sx = x + 8;
            for s in exports::SCALES {
                let b = btn(Do::Scale(s), "", format!("{s}x"));
                let b = if s == self.row.scale { b.kind(BtnKind::On) } else { b };
                sx += self.btns.draw(f.buffer_mut(), sx, y, x + w, &b) + 1;
            }
        }
        y += 2;

        // Options.
        buf_line(f, y, Line::from(Span::styled("Options", head(self.focus == Focus::Options))));
        y += 1;
        if opts.is_empty() {
            buf_line(f, y, Line::from(Span::styled("none for this format", dim)));
            y += 1;
        } else {
            let labels: Vec<String> = opts.iter().map(|(l, v)| format!("{l}: {v}")).collect();
            let chips: Vec<_> = labels
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let b = btn(Do::Opt(i), "", l.as_str());
                    if self.focus == Focus::Options && i == self.opt_idx { b.kind(BtnKind::On) } else { b }
                })
                .collect();
            y += self.btns.row(f.buffer_mut(), Rect::new(x, y, w, 3), &chips);
        }
        if !self.touched {
            buf_line(f, y, Line::from(Span::styled("(click an option to change it; right-click steps back)", dim)));
        }
        let bar = [btn(Do::Done, "⏎", "keep").primary(), btn(Do::Cancel, "esc", "cancel")];
        let by = inner.bottom().saturating_sub(1);
        self.btns.row(f.buffer_mut(), Rect::new(x, by, w, 1), &bar);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => return self.done(),
            KeyCode::Tab if shift => self.step_focus(true),
            KeyCode::BackTab => self.step_focus(true),
            KeyCode::Tab => self.step_focus(false),
            _ => match self.focus {
                Focus::Name => {
                    self.name.key(&k);
                }
                Focus::Format => match k.code {
                    KeyCode::Left | KeyCode::Up => self.step_format(-1),
                    KeyCode::Right | KeyCode::Down => self.step_format(1),
                    _ => {}
                },
                Focus::Scale => match k.code {
                    KeyCode::Left | KeyCode::Down => self.set_scale(exports::step_scale(self.row.scale, -1)),
                    KeyCode::Right | KeyCode::Up => self.set_scale(exports::step_scale(self.row.scale, 1)),
                    _ => {}
                },
                Focus::Options => {
                    let n = self.options().len().max(1);
                    match k.code {
                        KeyCode::Up => self.opt_idx = (self.opt_idx + n - 1) % n,
                        KeyCode::Down => self.opt_idx = (self.opt_idx + 1) % n,
                        KeyCode::Left => self.cycle(self.opt_idx, -1),
                        KeyCode::Right | KeyCode::Char(' ') => self.cycle(self.opt_idx, 1),
                        _ => {}
                    }
                }
            },
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        let right = matches!(m.kind, MouseEventKind::Down(MouseButton::Right));
        let hit = if right { self.btns.at(m.column, m.row) } else { self.btns.mouse(&m) };
        let a = self.name_area;
        if hit.is_none()
            && matches!(m.kind, MouseEventKind::Down(_))
            && m.row == a.y
            && m.column >= a.x
            && m.column < a.right()
        {
            self.focus = Focus::Name;
            return Outcome::Keep;
        }
        match hit {
            Some(Do::Token(i)) => {
                self.focus = Focus::Name;
                self.name.paste(TOKENS[i]);
            }
            Some(Do::Format(g)) => {
                self.focus = Focus::Format;
                self.set_format(g);
            }
            Some(Do::Scale(s)) => {
                self.focus = Focus::Scale;
                self.set_scale(s);
            }
            Some(Do::Opt(i)) => self.cycle(i, if right { -1 } else { 1 }),
            Some(Do::Done) => return self.done(),
            Some(Do::Cancel) => return Outcome::Close,
            None => {}
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        if self.focus == Focus::Name {
            self.name.paste(s);
        }
    }
}
