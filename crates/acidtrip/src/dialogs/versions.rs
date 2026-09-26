//! Version history browser: list snapshots, preview, restore or open.

use acidtrip_core::Clip;
use acidtrip_io::versions::{VersionInfo, VersionStore};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::tab::Tab;
use crate::ui::canvas::draw_clip;
use crate::ui::widgets::{Buttons, ListState, btn, centered, list_line, popup, theme};

pub struct VersionsDialog {
    store: VersionStore,
    items: Vec<VersionInfo>,
    list: ListState,
    cache: Option<(String, Clip, acidtrip_core::Palette)>,
    list_area: Rect,
    btns: Buttons<Do>,
}

/// The buttons under the list, so one mouse button restores or opens.
#[derive(Clone, Copy)]
enum Do {
    Restore,
    Copy,
    Close,
}

impl VersionsDialog {
    pub fn new(app: &App) -> anyhow::Result<Self> {
        let id = acidtrip_io::versions::store_id(&app.tab().doc, app.tab().file.as_deref());
        let store = VersionStore::open(&app.paths.versions_dir(), id)?;
        let items = store.list();
        Ok(VersionsDialog {
            store,
            items,
            list: ListState::default(),
            cache: None,
            list_area: Rect::default(),
            btns: Buttons::default(),
        })
    }

    fn load_preview(&mut self) {
        let Some(v) = self.items.get(self.list.selected) else {
            return;
        };
        if self.cache.as_ref().is_some_and(|(h, ..)| *h == v.hash) {
            return;
        }
        if let Ok(doc) = self.store.load(&v.hash) {
            let clip = Clip::from_grid(&doc.flatten());
            self.cache = Some((v.hash.clone(), clip, doc.meta.palette.clone()));
        }
    }

    fn choose(&self, as_new_tab: bool) -> Outcome {
        let Some(v) = self.items.get(self.list.selected) else {
            return Outcome::Keep;
        };
        let doc = match self.store.load(&v.hash) {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("{e:#}");
                return Outcome::Then(Box::new(move |app: &mut App| app.flash(msg, Level::Error)));
            }
        };
        let when = crate::ui::widgets::local_time(&v.timestamp);
        Outcome::Then(Box::new(move |app: &mut App| {
            if as_new_tab {
                let mut d = doc;
                d.meta.id = uuid::Uuid::new_v4();
                app.replace_doc(Tab::new(d, None), format!("opened version from {when} as an untitled copy"));
            } else {
                let t = app.tab_mut();
                let animated = doc.is_animated() || t.doc.is_animated();
                let frames = doc.frame_set();
                let (canvas, meta) = (doc.canvas, doc.meta);
                t.edit("Restore version", |b| {
                    if animated {
                        b.replace_frames(frames);
                    } else {
                        b.replace_canvas(|_| canvas);
                    }
                    b.replace_meta(|m| {
                        let id = m.id;
                        *m = meta;
                        m.id = id;
                    });
                });
                app.flash(format!("restored version from {when} (Ctrl-Z to undo)"), Level::Ok);
            }
        }))
    }
}

impl Dialog for VersionsDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        self.load_preview();
        let r = centered(area, area.width.saturating_sub(4).min(130), area.height.saturating_sub(2).min(34));
        let inner = popup(f, r, "Version history", "↑↓ · Enter restore · O open as a copy · Esc");
        let [side, right] = Layout::horizontal([Constraint::Length(42), Constraint::Min(20)]).areas(inner);
        let [left, _, bar] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)]).areas(side);
        self.list_area = left;
        self.btns.clear();
        if self.items.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    " No versions yet — they're saved on every save and every few minutes of work.",
                    Style::new().fg(theme::DIM),
                )),
                inner,
            );
            return;
        }
        let range = self.list.visible(self.items.len(), left.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let v = &self.items[i];
                let when = crate::ui::widgets::local_time(&v.timestamp);
                list_line(when, v.label.clone(), i == self.list.selected, left.width.saturating_sub(1))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), left);
        let btns = [
            btn(Do::Restore, "⏎", "restore").primary(),
            btn(Do::Copy, "O", "open copy"),
            btn(Do::Close, "esc", "close"),
        ];
        self.btns.row(f.buffer_mut(), bar, &btns);
        if let Some((_, clip, pal)) = &self.cache {
            draw_clip(
                f.buffer_mut(),
                Rect::new(right.x + 1, right.y, right.width.saturating_sub(1), right.height),
                clip,
                pal,
            );
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.choose(false),
            KeyCode::Char('o') | KeyCode::Char('O') => self.choose(true),
            _ => {
                self.list.key(&k, self.items.len(), 10);
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match self.btns.mouse(&m) {
            Some(Do::Restore) => return self.choose(false),
            Some(Do::Copy) => return self.choose(true),
            Some(Do::Close) => return Outcome::Close,
            None => {}
        }
        let r = self.list_area;
        match m.kind {
            MouseEventKind::Down(_)
                if m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right() =>
            {
                let i = self.list.offset + (m.row - r.y) as usize;
                if i < self.items.len() {
                    self.list.selected = i;
                }
            }
            MouseEventKind::ScrollDown => {
                self.list.selected = (self.list.selected + 1).min(self.items.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => self.list.selected = self.list.selected.saturating_sub(1),
            _ => {}
        }
        Outcome::Keep
    }
}
