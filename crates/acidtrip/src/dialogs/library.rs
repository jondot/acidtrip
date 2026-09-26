//! The Library: the Gallery and the sourcing studio as tabs of one window,
//! since they work on the same art. "Sourcing studio" in the Gallery just
//! switches tabs with the piece loaded. Ctrl-T or a click on a tab switches.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use acidtrip_ai::gallery::Piece;

use super::gallery::GalleryDialog;
use super::harvest::HarvestDialog;
use super::{Dialog, Outcome};
use crate::app::App;
use crate::ui::widgets::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibTab {
    Gallery,
    Studio,
}

pub struct LibraryDialog {
    pub tab: LibTab,
    gallery: GalleryDialog,
    /// Made the first time the Studio tab opens (it starts loading a source).
    studio: Option<HarvestDialog>,
    tabs: Vec<(Rect, LibTab)>,
}

impl LibraryDialog {
    pub fn gallery(app: &App) -> Self {
        LibraryDialog { tab: LibTab::Gallery, gallery: GalleryDialog::new(app), studio: None, tabs: vec![] }
    }

    /// The Gallery open on one piece; `take` starts taking a part of it.
    pub fn piece(app: &App, piece: Piece, take: bool) -> Self {
        LibraryDialog {
            tab: LibTab::Gallery,
            gallery: GalleryDialog::viewing(app, piece, take),
            studio: None,
            tabs: vec![],
        }
    }

    /// The Studio tab, on `studio` or a fresh one.
    pub fn studio(app: &App, studio: Option<HarvestDialog>) -> Self {
        LibraryDialog {
            tab: LibTab::Studio,
            gallery: GalleryDialog::new(app),
            studio: Some(studio.unwrap_or_else(|| HarvestDialog::new(app))),
            tabs: vec![],
        }
    }

    fn switch(&mut self, tab: LibTab, app: &App) {
        if tab == LibTab::Studio && self.studio.is_none() {
            self.studio = Some(HarvestDialog::new(app));
        }
        self.tab = tab;
    }

    /// After the Gallery handled an event: its "sourcing studio" opens the
    /// Studio tab on that source.
    fn follow(&mut self, out: Outcome, app: &App) -> Outcome {
        if let Some(src) = self.gallery.studio_request.take() {
            self.studio = Some(HarvestDialog::with_source(app, &src));
            self.tab = LibTab::Studio;
        }
        out
    }

    fn active(&mut self) -> &mut dyn Dialog {
        match (self.tab, &mut self.studio) {
            (LibTab::Studio, Some(s)) => s,
            _ => &mut self.gallery,
        }
    }

    /// Tabs on the window's top border, at the right.
    fn draw_tabs(&mut self, f: &mut Frame, area: Rect) {
        self.tabs.clear();
        let y = area.y + 1;
        let items = [(LibTab::Gallery, " ▦ Gallery "), (LibTab::Studio, " ⚒ Studio ")];
        let hint = " ^T ";
        let w: u16 = items.iter().map(|(_, l)| l.chars().count() as u16).sum::<u16>() + hint.len() as u16;
        if area.width < w + 40 {
            return;
        }
        let mut x = area.right() - 2 - w;
        for (tab, label) in items {
            let lw = label.chars().count() as u16;
            let st = if tab == self.tab {
                Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::TEXT).bg(theme::PANEL_HI)
            };
            let r = Rect::new(x, y, lw, 1);
            f.render_widget(Paragraph::new(Span::styled(label, st)), r);
            self.tabs.push((r, tab));
            x += lw;
        }
        f.render_widget(
            Paragraph::new(Span::styled(hint, Style::new().fg(theme::DIM).bg(theme::PANEL))),
            Rect::new(x, y, hint.len() as u16, 1),
        );
    }
}

impl Dialog for LibraryDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        self.active().draw(f, area, app);
        self.draw_tabs(f, area);
    }

    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome {
        if k.code == KeyCode::Char('t') && k.modifiers.contains(KeyModifiers::CONTROL) {
            let next = if self.tab == LibTab::Gallery { LibTab::Studio } else { LibTab::Gallery };
            self.switch(next, app);
            return Outcome::Keep;
        }
        let out = self.active().key(k, app);
        self.follow(out, app)
    }

    fn mouse(&mut self, m: MouseEvent, app: &App) -> Outcome {
        if let MouseEventKind::Down(MouseButton::Left) = m.kind
            && let Some(&(_, tab)) =
                self.tabs.iter().find(|(r, _)| m.row == r.y && m.column >= r.x && m.column < r.right())
        {
            self.switch(tab, app);
            return Outcome::Keep;
        }
        let out = self.active().mouse(m, app);
        self.follow(out, app)
    }

    fn paste(&mut self, s: &str) {
        self.active().paste(s);
    }

    fn animating(&self) -> bool {
        self.gallery.animating() || self.studio.as_ref().is_some_and(|s| s.animating())
    }

    fn pixels(&mut self, f: &mut Frame, thumbs: &mut crate::ui::thumbs::Thumbs) {
        if self.tab == LibTab::Gallery {
            self.gallery.pixels(f, thumbs);
        }
    }
}
