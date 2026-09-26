//! Layers panel: large thumbnails and every layer action in one place.

use acidtrip_core::LayerKind;
use acidtrip_core::tools::{self, LayerProps};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::actions::Action;
use crate::app::App;
use crate::ui::minimap;
use crate::ui::widgets::{centered, popup, theme};

const ROW_H: u16 = 4;
const THUMB_W: u16 = 20;

/// A clickable layer flag (so one mouse button can lock, hide and mark
/// reference layers, not only the V/L/F keys).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flag {
    Visible,
    Locked,
    Reference,
}

pub struct LayersDialog {
    rows: Vec<(Rect, usize)>,
    chips: Vec<(Rect, usize, Flag)>,
    offset: usize,
}

impl LayersDialog {
    pub fn new(_app: &App) -> Self {
        LayersDialog { rows: vec![], chips: vec![], offset: 0 }
    }
}

fn props(f: impl FnOnce(&mut LayerProps, &acidtrip_core::Layer) + 'static) -> Outcome {
    Outcome::KeepThen(Box::new(move |app: &mut App| props_at(app, None, f)))
}

/// Change layer `at` (the active one if `None`).
fn props_at(app: &mut App, at: Option<usize>, f: impl FnOnce(&mut LayerProps, &acidtrip_core::Layer)) {
    let t = app.tab_mut();
    let l = at.unwrap_or(t.layer);
    let mut p = LayerProps::default();
    f(&mut p, &t.doc.canvas.layers[l]);
    t.edit("Layer properties", |b| tools::set_layer_props(b, l, &p));
}

fn toggle(p: &mut LayerProps, l: &acidtrip_core::Layer, flag: Flag) {
    match flag {
        Flag::Visible => p.visible = Some(!l.visible),
        Flag::Locked => p.locked = Some(!l.locked),
        Flag::Reference => p.reference = Some(l.kind != LayerKind::Reference),
    }
}

fn run(a: Action) -> Outcome {
    Outcome::KeepThen(Box::new(move |app: &mut App| app.run(a)))
}

impl Dialog for LayersDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let t = app.tab();
        let layers = &t.doc.canvas.layers;
        let h = (layers.len() as u16 * ROW_H + 5).min(area.height.saturating_sub(2)).max(12);
        let r = centered(area, 78, h);
        let inner = popup(f, r, "Layers", "↑↓ select · Shift-↑↓ move · N new · D duplicate · X delete · Esc close");
        let keys = Line::from(Span::styled(
            " V show/hide · L lock · F reference (not exported) · R rename · M merge down",
            Style::new().fg(theme::DIM),
        ));
        f.render_widget(Paragraph::new(keys), Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1));
        let list_h = inner.height.saturating_sub(1);
        let per_page = (list_h / ROW_H).max(1) as usize;
        // Top layer first; keep the active one visible.
        let order: Vec<usize> = (0..layers.len()).rev().collect();
        let pos = order.iter().position(|&i| i == t.layer).unwrap_or(0);
        if pos < self.offset {
            self.offset = pos;
        } else if pos >= self.offset + per_page {
            self.offset = pos + 1 - per_page;
        }
        self.rows.clear();
        self.chips.clear();
        for (k, &i) in order.iter().skip(self.offset).take(per_page).enumerate() {
            let l = &layers[i];
            let y = inner.y + k as u16 * ROW_H;
            let row = Rect::new(inner.x, y, inner.width, ROW_H - 1);
            self.rows.push((row, i));
            let active = i == t.layer;
            if active {
                for yy in row.y..row.bottom() {
                    f.render_widget(
                        Paragraph::new("").style(Style::new().bg(theme::PANEL_HI)),
                        Rect::new(row.x, yy, row.width, 1),
                    );
                }
            }
            minimap::draw_layer(
                f.buffer_mut(),
                Rect::new(row.x + 1, y, THUMB_W, ROW_H - 1),
                &t.doc.canvas,
                i,
                &t.doc.meta.palette,
                t.scroll.1,
            );
            let tx = row.x + THUMB_W + 3;
            let name_style = if active {
                Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::TEXT)
            };
            let reference = l.kind == LayerKind::Reference;
            let flags = [
                (if l.visible { "◉ visible" } else { "○ hidden" }, l.visible, Flag::Visible),
                (if l.locked { "■ locked" } else { "□ lock" }, l.locked, Flag::Locked),
                (if reference { "■ reference" } else { "□ reference" }, reference, Flag::Reference),
            ];
            let mut spans = vec![Span::raw("  ")];
            let mut x = tx + 2;
            for (label, on, flag) in flags {
                let w = label.chars().count() as u16;
                self.chips.push((Rect::new(x, y + 1, w, 1), i, flag));
                spans.push(Span::styled(label, Style::new().fg(if on { theme::TEXT } else { theme::DIM })));
                spans.push(Span::raw("  "));
                x += w + 2;
            }
            if i == 0 {
                spans.push(Span::styled("background", Style::new().fg(theme::DIM)));
            }
            let used = l.cells.iter().filter(|c| c.is_some_and(|c| !c.is_blank())).count();
            let lines = vec![
                Line::from(Span::styled(format!("{}{}", if active { "▸ " } else { "  " }, l.name), name_style)),
                Line::from(spans),
                Line::from(Span::styled(
                    format!("  {used} cells drawn · layer {}", i + 1),
                    Style::new().fg(theme::DIM),
                )),
            ];
            f.render_widget(Paragraph::new(lines), Rect::new(tx, y, row.width.saturating_sub(THUMB_W + 4), ROW_H - 1));
        }
    }

    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Esc | KeyCode::Enter => Outcome::Close,
            KeyCode::Up if shift => run(Action::LayerMoveUp),
            KeyCode::Down if shift => run(Action::LayerMoveDown),
            KeyCode::Up => run(Action::LayerUp),
            KeyCode::Down => run(Action::LayerDown),
            KeyCode::Char(c) => match c.to_ascii_lowercase() {
                'n' | '+' => run(Action::LayerAdd),
                'd' => run(Action::LayerDuplicate),
                'x' => run(Action::LayerRemove),
                'm' => run(Action::LayerMerge),
                'r' => {
                    let cur = app.tab().doc.canvas.layers[app.tab().layer].name.clone();
                    Outcome::KeepThen(Box::new(move |app: &mut App| {
                        app.dialogs.push(Box::new(super::prompt::PromptDialog::new(
                            "Layer name",
                            &cur,
                            Box::new(|app: &mut App, name: String| {
                                let t = app.tab_mut();
                                let l = t.layer;
                                t.edit("Rename layer", |b| {
                                    tools::set_layer_props(b, l, &LayerProps { name: Some(name), ..Default::default() })
                                });
                            }),
                        )))
                    }))
                }
                'v' => props(|p, l| toggle(p, l, Flag::Visible)),
                'l' => props(|p, l| toggle(p, l, Flag::Locked)),
                'f' => props(|p, l| toggle(p, l, Flag::Reference)),
                '[' => run(Action::LayerMoveDown),
                ']' => run(Action::LayerMoveUp),
                _ => Outcome::Keep,
            },
            KeyCode::Delete | KeyCode::Backspace => run(Action::LayerRemove),
            _ => Outcome::Keep,
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match m.kind {
            MouseEventKind::Down(_) => {
                let hit = |r: &Rect| m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right();
                if let Some(&(_, i, flag)) = self.chips.iter().find(|(r, ..)| hit(r)) {
                    return Outcome::KeepThen(Box::new(move |app: &mut App| {
                        app.tab_mut().layer = i;
                        props_at(app, Some(i), |p, l| toggle(p, l, flag));
                    }));
                }
                match self
                    .rows
                    .iter()
                    .find(|(r, _)| m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right())
                {
                    Some(&(_, i)) => Outcome::KeepThen(Box::new(move |app: &mut App| app.tab_mut().layer = i)),
                    None => Outcome::Keep,
                }
            }
            MouseEventKind::ScrollUp => run(Action::LayerUp),
            MouseEventKind::ScrollDown => run(Action::LayerDown),
            _ => Outcome::Keep,
        }
    }
}
