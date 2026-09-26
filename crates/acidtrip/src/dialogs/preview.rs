//! Pixel-exact preview: the real VGA-font render shown through the
//! terminal's graphics protocol (kitty / iTerm2 / sixel), or half-blocks.

use acidtrip_core::render::{RenderOptions, render_document};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use super::{Dialog, Outcome};
use crate::app::App;
use crate::ui::widgets::{popup, theme};

pub struct PreviewDialog {
    proto: Option<StatefulProtocol>,
    protocol_name: String,
    rows: usize,
    scroll: usize,
    total: usize,
    doc: acidtrip_core::Document,
    picker: Picker,
}

impl PreviewDialog {
    pub fn new(app: &App) -> Self {
        let picker = app.picker.clone().unwrap_or_else(Picker::halfblocks);
        let doc = app.tab().doc.clone();
        let total = doc.canvas.used_height().max(1);
        let mut d = PreviewDialog {
            proto: None,
            protocol_name: format!("{:?}", picker.protocol_type()),
            rows: 0,
            scroll: 0,
            total,
            doc,
            picker,
        };
        d.rebuild(25);
        d
    }

    fn rebuild(&mut self, rows: usize) {
        self.rows = rows.max(1);
        let mut doc = self.doc.clone();
        // Render only the visible slice so large pieces stay fast.
        let h = self.rows.min(self.total.saturating_sub(self.scroll)).max(1);
        doc.canvas = crate::ui::minimap::crop_rows(&doc.canvas, self.scroll, h);
        let img = render_document(&doc, None, RenderOptions::default());
        self.proto = Some(self.picker.new_resize_protocol(image::DynamicImage::ImageRgba8(img)));
    }
}

impl Dialog for PreviewDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let inner = popup(f, area, "Pixel-exact preview", "↑↓ PgUp/PgDn scroll · any key closes");
        let [top, body] = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(1),
        ])
        .areas(inner);
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " VGA 8x16 · protocol: {} · rows {}-{} of {}",
                    self.protocol_name,
                    self.scroll + 1,
                    (self.scroll + self.rows).min(self.total),
                    self.total
                ),
                Style::new().fg(theme::DIM),
            )),
            top,
        );
        // Terminal cells are ~1:2, like VGA 8x16 cells, so one art row per text row.
        let want = body.height as usize;
        if want != self.rows {
            self.rebuild(want);
        }
        if let Some(p) = &mut self.proto {
            f.render_stateful_widget(StatefulImage::default(), body, p);
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let max = self.total.saturating_sub(self.rows);
        let new = match k.code {
            KeyCode::Down => (self.scroll + 1).min(max),
            KeyCode::Up => self.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => (self.scroll + self.rows).min(max),
            KeyCode::PageUp => self.scroll.saturating_sub(self.rows),
            _ => return Outcome::Close,
        };
        if new != self.scroll {
            self.scroll = new;
            self.rebuild(self.rows);
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, app: &App) -> Outcome {
        match m.kind {
            MouseEventKind::ScrollDown => self.key(KeyEvent::from(KeyCode::Down), app),
            MouseEventKind::ScrollUp => self.key(KeyEvent::from(KeyCode::Up), app),
            MouseEventKind::Down(_) => Outcome::Close,
            _ => Outcome::Keep,
        }
    }
}
