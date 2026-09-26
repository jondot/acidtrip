//! Share menu: one keypress to get your art somewhere.

use acidtrip_io::format::{self, Format, SaveOptions};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::share;
use crate::ui::widgets::{ListState, centered, list_line, popup, theme};

#[derive(Clone, Copy)]
enum Item {
    CopyPng,
    CopyAnsi,
    CopyText,
    Gist,
    PasteHost,
    ExportFiles,
}

const ITEMS: [(Item, &str, &str); 6] = [
    (Item::CopyPng, "Copy as PNG image", "1"),
    (Item::CopyAnsi, "Copy as ANSI (UTF-8, for terminals/chat)", "2"),
    (Item::CopyText, "Copy as plain text", "3"),
    (Item::Gist, "Publish a GitHub gist (.ans + .utf8ans)", "4"),
    (Item::PasteHost, "Upload PNG to paste host, copy link", "5"),
    (Item::ExportFiles, "Export to file… (PNG, GIF, SVG, HTML, React…)", "6"),
];

pub struct ShareDialog {
    list: ListState,
    area: Rect,
}

impl ShareDialog {
    pub fn new() -> Self {
        ShareDialog { list: ListState::default(), area: Rect::default() }
    }

    fn run(&self, i: usize) -> Outcome {
        let item = ITEMS[i].0;
        Outcome::Then(Box::new(move |app: &mut App| do_share(app, item)))
    }
}

/// Over SSH text goes to the local clipboard through the terminal (OSC 52),
/// which some terminals ignore: say so.
fn via() -> &'static str {
    if share::over_ssh() { " through your terminal (OSC 52)" } else { "" }
}

fn do_share(app: &mut App, item: Item) {
    let doc = app.tab().doc.clone();
    let scale = app.config.share.png_scale.max(1);
    let title = if doc.meta.sauce.title.is_empty() { app.tab().title() } else { doc.meta.sauce.title.clone() };
    let stem: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase();
    let stem = if stem.is_empty() { "art".to_string() } else { stem };
    let png = || format::save_bytes(&doc, Format::Png, &SaveOptions { scale, ..Default::default() });
    let result: anyhow::Result<String> = (|| match item {
        Item::CopyPng => {
            share::copy_image(&png()?)?;
            Ok("PNG copied — paste it anywhere".into())
        }
        Item::CopyAnsi => {
            let b = format::save_bytes(&doc, Format::Utf8Ansi, &SaveOptions::default())?;
            share::copy_text(&String::from_utf8_lossy(&b))?;
            Ok(format!("ANSI copied{} — paste into a terminal or chat code block", via()))
        }
        Item::CopyText => {
            let b = format::save_bytes(&doc, Format::Ascii, &SaveOptions::default())?;
            let text: String = b.iter().map(|&c| acidtrip_core::cp437::to_char(c)).collect();
            share::copy_text(&text)?;
            Ok(format!("text copied{}", via()))
        }
        Item::Gist => {
            let ans = share::temp_file(
                &format!("{stem}.ans"),
                &format::save_bytes(&doc, Format::Ansi, &SaveOptions::default())?,
            )?;
            let utf = share::temp_file(
                &format!("{stem}.utf8ans"),
                &format::save_bytes(&doc, Format::Utf8Ansi, &SaveOptions::default())?,
            )?;
            let url = share::gist(&[&ans, &utf], &format!("{title} — made with acidtrip"))?;
            let _ = share::copy_text(&url);
            Ok(format!("gist: {url} (link copied)"))
        }
        Item::PasteHost => {
            let p = share::temp_file(&format!("{stem}.png"), &png()?)?;
            let url = share::paste_host(&p, &app.config.share.paste_url)?;
            let _ = share::copy_text(&url);
            Ok(format!("uploaded: {url} (link copied)"))
        }
        Item::ExportFiles => Ok(String::new()),
    })();
    match (item, result) {
        (Item::ExportFiles, _) => app.run(crate::actions::Action::ExportAs),
        (_, Ok(msg)) => app.flash(msg, Level::Ok),
        (_, Err(e)) => app.flash(format!("share failed: {e:#}"), Level::Error),
    }
}

impl Dialog for ShareDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let r = centered(area, 62, ITEMS.len() as u16 + 6);
        let inner = popup(f, r, "Share", "1-6 or Enter · Esc");
        self.area = Rect::new(inner.x, inner.y + 1, inner.width, ITEMS.len() as u16);
        let lines: Vec<_> = ITEMS
            .iter()
            .enumerate()
            .map(|(i, (_, label, key))| list_line(*label, *key, i == self.list.selected, inner.width))
            .collect();
        f.render_widget(Paragraph::new(lines), self.area);
        let note = if share::over_ssh() {
            "over SSH: copies go through your terminal; images can't".to_string()
        } else {
            format!("paste host: {} · gist needs `gh auth login`", app.config.share.paste_url)
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(note, Style::new().fg(theme::DIM)))),
            Rect::new(inner.x + 1, inner.bottom().saturating_sub(1), inner.width.saturating_sub(2), 1),
        );
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.run(self.list.selected),
            KeyCode::Char(c @ '1'..='6') => self.run(c as usize - '1' as usize),
            _ => {
                self.list.key(&k, ITEMS.len(), 3);
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
            return self.run((m.row - r.y) as usize);
        }
        Outcome::Keep
    }
}
