//! Generic form dialog (text / number / toggle / choice fields) and the
//! forms built on it: new document, document properties, SAUCE.

use std::collections::HashMap;
use std::path::PathBuf;

use acidtrip_core::tools;
use acidtrip_core::{DocKind, Document, SauceMeta};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::tab::Tab;
use crate::ui::widgets::{Buttons, LineInput, btn, centered, popup, theme, wrap};

pub enum FieldKind {
    Text(LineInput),
    Toggle(bool),
    Choice { options: Vec<String>, idx: usize },
}

pub struct Field {
    pub key: &'static str,
    pub label: String,
    pub kind: FieldKind,
}

impl Field {
    pub fn text(key: &'static str, label: &str, v: &str) -> Field {
        Field { key, label: label.into(), kind: FieldKind::Text(LineInput::new(v)) }
    }

    pub fn toggle(key: &'static str, label: &str, v: bool) -> Field {
        Field { key, label: label.into(), kind: FieldKind::Toggle(v) }
    }

    pub fn choice(key: &'static str, label: &str, options: &[&str], idx: usize) -> Field {
        Field {
            key,
            label: label.into(),
            kind: FieldKind::Choice { options: options.iter().map(|s| s.to_string()).collect(), idx },
        }
    }
}

#[derive(Default)]
pub struct Values(HashMap<&'static str, String>);

impl Values {
    pub fn str(&self, k: &str) -> String {
        self.0.get(k).cloned().unwrap_or_default()
    }

    pub fn bool(&self, k: &str) -> bool {
        self.0.get(k).is_some_and(|v| v == "true")
    }

    pub fn usize(&self, k: &str, default: usize) -> usize {
        self.0.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Ok,
    Cancel,
}

pub type FormCallback = Box<dyn FnOnce(&mut App, Values)>;

pub struct FormDialog {
    title: String,
    pub fields: Vec<Field>,
    focus: usize,
    note: Vec<String>,
    on_ok: Option<FormCallback>,
    rows: Vec<Rect>,
    width: u16,
    btns: Buttons<Do>,
}

impl FormDialog {
    pub fn new(title: &str, fields: Vec<Field>, note: Vec<String>, on_ok: FormCallback) -> Self {
        FormDialog {
            title: title.into(),
            fields,
            focus: 0,
            note,
            on_ok: Some(on_ok),
            rows: vec![],
            width: 64,
            btns: Buttons::default(),
        }
    }

    fn values(&self) -> Values {
        Values(
            self.fields
                .iter()
                .map(|f| {
                    let v = match &f.kind {
                        FieldKind::Text(i) => i.text.clone(),
                        FieldKind::Toggle(b) => b.to_string(),
                        FieldKind::Choice { options, idx } => options[*idx].clone(),
                    };
                    (f.key, v)
                })
                .collect(),
        )
    }

    fn submit(&mut self) -> Outcome {
        let v = self.values();
        match self.on_ok.take() {
            Some(cb) => Outcome::Then(Box::new(move |app: &mut App| cb(app, v))),
            None => Outcome::Close,
        }
    }
}

impl Dialog for FormDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let text_w = self.width.saturating_sub(4) as usize;
        let notes: Vec<String> = self.note.iter().flat_map(|n| wrap(n, text_w)).collect();
        // fields, a gap, the notes, a gap, the buttons, inside the border
        let h = self.fields.len() as u16 + notes.len() as u16 + 6;
        let r = centered(area, self.width, h);
        let inner = popup(f, r, &self.title, "↑↓ move · Space/←→ change · Enter ok · Esc");
        self.rows.clear();
        self.btns.clear();
        let label_w = self.fields.iter().map(|f| f.label.chars().count()).max().unwrap_or(8) as u16 + 2;
        for (i, fld) in self.fields.iter().enumerate() {
            let row = Rect::new(inner.x + 1, inner.y + 1 + i as u16, inner.width.saturating_sub(2), 1);
            self.rows.push(row);
            let focused = i == self.focus;
            let lab = format!("{:>w$} ", fld.label, w = label_w as usize - 1);
            match &fld.kind {
                FieldKind::Text(inp) => inp.render(f, row, &lab, focused),
                FieldKind::Toggle(b) => {
                    let st = if focused {
                        Style::new().fg(theme::BG).bg(theme::ACCENT2)
                    } else {
                        Style::new().fg(theme::TEXT)
                    };
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(lab, Style::new().fg(if focused { theme::ACCENT2 } else { theme::DIM })),
                            Span::styled(if *b { "[x] on " } else { "[ ] off" }, st),
                        ])),
                        row,
                    );
                }
                FieldKind::Choice { options, idx } => {
                    let st = if focused {
                        Style::new().fg(theme::BG).bg(theme::ACCENT2)
                    } else {
                        Style::new().fg(theme::TEXT)
                    };
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(lab, Style::new().fg(if focused { theme::ACCENT2 } else { theme::DIM })),
                            Span::styled(format!("‹ {} ›", options[*idx]), st),
                        ])),
                        row,
                    );
                }
            }
        }
        for (i, n) in notes.into_iter().enumerate() {
            let y = inner.y + 2 + self.fields.len() as u16 + i as u16;
            if y + 1 < inner.bottom() {
                f.render_widget(
                    Paragraph::new(Span::styled(n, Style::new().fg(theme::DIM))),
                    Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
                );
            }
        }
        if inner.height > 0 {
            let by = inner.bottom() - 1;
            let bar = [btn(Do::Ok, "⏎", "ok").primary(), btn(Do::Cancel, "esc", "cancel")];
            self.btns.row(f.buffer_mut(), Rect::new(inner.x + 1, by, inner.width.saturating_sub(2), 1), &bar);
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let n = self.fields.len();
        match k.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => return self.submit(),
            KeyCode::Tab | KeyCode::Down => {
                self.focus = (self.focus + 1) % n;
                return Outcome::Keep;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + n - 1) % n;
                return Outcome::Keep;
            }
            _ => {}
        }
        let fld = &mut self.fields[self.focus];
        match &mut fld.kind {
            FieldKind::Text(i) => {
                i.key(&k);
            }
            FieldKind::Toggle(b) => {
                if matches!(k.code, KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right) {
                    *b = !*b;
                }
            }
            FieldKind::Choice { options, idx } => match k.code {
                KeyCode::Left => *idx = (*idx + options.len() - 1) % options.len(),
                KeyCode::Right | KeyCode::Char(' ') => *idx = (*idx + 1) % options.len(),
                _ => {}
            },
        }
        let _ = KeyModifiers::NONE;
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match self.btns.mouse(&m) {
            Some(Do::Ok) => return self.submit(),
            Some(Do::Cancel) => return Outcome::Close,
            None => {}
        }
        let back = matches!(m.kind, MouseEventKind::Down(MouseButton::Right));
        if let MouseEventKind::Down(_) = m.kind
            && let Some(i) = self.rows.iter().position(|r| m.row == r.y && m.column >= r.x && m.column < r.right())
        {
            self.focus = i;
            match &mut self.fields[i].kind {
                FieldKind::Toggle(b) => *b = !*b,
                // Right-click steps back (a shortcut; a left click cycles round).
                FieldKind::Choice { options, idx } => {
                    *idx = if back { (*idx + options.len() - 1) % options.len() } else { (*idx + 1) % options.len() }
                }
                FieldKind::Text(_) => {}
            }
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        if let FieldKind::Text(i) = &mut self.fields[self.focus].kind {
            i.paste(s);
        }
    }
}

// ------------------------------------------------------------ builders

pub fn new_doc_dialog(app: &App) -> FormDialog {
    let c = &app.config.new_doc;
    FormDialog::new(
        "New document",
        vec![
            Field::text("width", "Width", &c.width.to_string()),
            Field::text("height", "Height", &c.height.to_string()),
            Field::choice("kind", "Kind", &["classic", "modern"], usize::from(c.kind.eq_ignore_ascii_case("modern"))),
            Field::toggle("ice", "iCE colors", c.ice),
        ],
        vec![
            "classic: CP437 + 16 colors, saves losslessly to .ans/.xb/.bin".into(),
            "modern: any Unicode char + 24-bit color (SVG/HTML/React/UTF-8)".into(),
        ],
        Box::new(|app: &mut App, v: Values| {
            let mut d = app.new_doc();
            let kind = if v.str("kind") == "modern" { DocKind::Modern } else { DocKind::Classic };
            let w = v.usize("width", 80).clamp(1, 4000);
            let h = v.usize("height", 25).clamp(1, 20000);
            let mut nd = Document::new(kind, w, h);
            nd.meta.sauce = std::mem::take(&mut d.meta.sauce);
            nd.meta.ice = v.bool("ice");
            app.replace_doc(Tab::new(nd, None), format!("new {w}x{h} {kind:?} document"));
        }),
    )
}

pub fn doc_props_dialog(app: &App) -> FormDialog {
    let d = &app.tab().doc;
    FormDialog::new(
        "Document properties",
        vec![
            Field::text("width", "Width", &d.width().to_string()),
            Field::text("height", "Height", &d.height().to_string()),
            Field::choice("kind", "Kind", &["classic", "modern"], usize::from(d.meta.kind == DocKind::Modern)),
            Field::toggle("ice", "iCE colors", d.meta.ice),
            Field::toggle("nine", "9px letter spacing", d.meta.letter_spacing_9px),
            Field::text("font", "Font name", &d.meta.font_name),
            Field::text("tabs", "Tab stop every", &d.meta.tab_stops.first().copied().unwrap_or(8).to_string()),
        ],
        vec!["Width 160 = ACiDDraw wide mode. Changing kind to classic downsamples.".into()],
        Box::new(|app: &mut App, v: Values| {
            let t = app.tab_mut();
            let (w0, h0) = (t.doc.width(), t.doc.height());
            let w = v.usize("width", w0).clamp(1, 4000);
            let h = v.usize("height", h0).clamp(1, 20000);
            let kind = if v.str("kind") == "modern" { DocKind::Modern } else { DocKind::Classic };
            let ice = v.bool("ice");
            let every = v.usize("tabs", 8).clamp(1, 80) as u16;
            let (nine, font) = (v.bool("nine"), v.str("font"));
            t.edit("Document properties", |b| {
                if (w, h) != (w0, h0) {
                    tools::resize(b, w, h);
                }
                if kind != b.meta().kind {
                    tools::set_kind(b, kind);
                }
                if ice != b.meta().ice {
                    tools::set_ice(b, ice);
                }
                b.replace_meta(|m| {
                    m.letter_spacing_9px = nine;
                    m.font_name = font;
                    m.tab_stops = (1..(w as u16 / every + 1)).map(|i| i * every).collect();
                });
            });
            app.flash("document updated", Level::Ok);
        }),
    )
}

pub fn sauce_dialog(app: &App) -> FormDialog {
    let s = &app.tab().doc.meta.sauce;
    FormDialog::new(
        "SAUCE",
        vec![
            Field::text("title", "Title", &s.title),
            Field::text("author", "Author", &s.author),
            Field::text("group", "Group", &s.group),
            Field::text("date", "Date (YYYYMMDD)", &s.date),
            Field::text("comments", "Comments", &s.comments.join(" | ")),
            Field::toggle("attach", "Attach on save", s.attach),
        ],
        vec![
            "Separate comment lines with |. Size and date fill in on save.".into(),
            "Filling it in turns attach on.".into(),
        ],
        Box::new(|app: &mut App, v: Values| {
            let t = app.tab_mut();
            t.edit("SAUCE", |b| {
                b.replace_meta(|m| {
                    let before = m.sauce.clone();
                    m.sauce.title = v.str("title").chars().take(35).collect();
                    m.sauce.author = v.str("author").chars().take(20).collect();
                    m.sauce.group = v.str("group").chars().take(20).collect();
                    m.sauce.date = v.str("date").chars().filter(char::is_ascii_digit).take(8).collect();
                    m.sauce.comments = v
                        .str("comments")
                        .split('|')
                        .map(|c| c.trim().chars().take(64).collect())
                        .filter(|c: &String| !c.is_empty())
                        .collect();
                    m.sauce.attach = sauce_attach(&before, &m.sauce, v.bool("attach"));
                })
            });
            app.flash("SAUCE updated", Level::Ok);
        }),
    )
}

/// Whether to attach SAUCE after an edit: as the toggle says, except that
/// filling in a record that wasn't attached attaches it (a file opened
/// without SAUCE has the toggle off, and typed-in fields would otherwise be
/// dropped on save without a word).
fn sauce_attach(before: &SauceMeta, after: &SauceMeta, toggle: bool) -> bool {
    let typed = before.title != after.title
        || before.author != after.author
        || before.group != after.group
        || before.date != after.date
        || before.comments != after.comments;
    toggle || (!before.attach && typed)
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filling_in_sauce_attaches_it() {
        let none = SauceMeta::default();
        let titled = SauceMeta { title: "Night Drive".into(), ..SauceMeta::default() };
        // A file without SAUCE: typing a title attaches it.
        assert!(sauce_attach(&none, &titled, false));
        // Nothing typed: stays off.
        assert!(!sauce_attach(&none, &none, false));
        // Already attached, turned off: off, even with an edit.
        let attached = SauceMeta { attach: true, ..titled.clone() };
        let retitled = SauceMeta { title: "Other".into(), ..titled.clone() };
        assert!(!sauce_attach(&attached, &retitled, false));
        // Unattached and left alone the second time: the toggle decides.
        assert!(!sauce_attach(&titled, &titled, false));
        assert!(sauce_attach(&titled, &titled, true));
    }

    #[test]
    fn notes_wrap_to_the_dialog() {
        let n = "Width 160 = ACiDDraw wide mode. Changing kind to classic downsamples.";
        let lines = wrap(n, 60);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.chars().count() <= 60));
        assert_eq!(lines.join(" "), n);
        assert_eq!(wrap("", 10), vec![String::new()]);
        assert_eq!(wrap("abcdefghijkl", 5), vec!["abcde".to_string()]);
    }
}
