//! Brush studio: pick a brush preset, tune its parameters with sliders and
//! see a live stroke. Changes apply to the pen as you make them; S saves
//! the result as your own preset.

use acidtrip_core::tools::brush::{BrushSpec, Param};
use acidtrip_core::tools::pen::{self, PenStroke};
use acidtrip_core::tools::{Brush, Ctx};
use acidtrip_core::{Cell, Color, Document, TxBuilder};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::canvas;
use crate::ui::widgets::{Buttons, btn, centered, popup, theme};

const LIST_W: u16 = 22;
const LABEL_W: u16 = 13;
const SLIDER_W: u16 = 24;
const PREVIEW_H: u16 = 7;

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Save,
    Delete,
    Reset,
    Done,
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Presets,
    Params,
}

pub struct BrushStudio {
    focus: Focus,
    param: usize,
    list_offset: usize,
    presets: Vec<(Rect, usize)>,
    sliders: Vec<(Rect, Param)>,
    dragging: Option<Param>,
    preview: Option<(PreviewKey, Vec<Cell>)>,
    btns: Buttons<Act>,
}

#[derive(PartialEq)]
pub struct PreviewKey {
    spec: BrushSpec,
    tiles: Vec<char>,
    w: usize,
    h: usize,
    fg: Color,
    bg: Color,
}

impl BrushStudio {
    pub fn new(_app: &App) -> Self {
        BrushStudio {
            focus: Focus::Presets,
            param: 1,
            list_offset: 0,
            presets: vec![],
            sliders: vec![],
            dragging: None,
            preview: None,
            btns: Buttons::default(),
        }
    }
}

/// Apply a changed brush to the pen.
fn set(spec: BrushSpec) -> Outcome {
    Outcome::KeepThen(Box::new(move |app: &mut App| app.tools.pen_brush = spec))
}

fn edit(app: &App, f: impl FnOnce(&mut BrushSpec)) -> Outcome {
    let mut b = app.tools.pen_brush.clone();
    f(&mut b);
    set(b)
}

/// Ask for a name and save the pen's brush under it.
fn save_as(app: &App) -> Outcome {
    let name = app.tools.pen_brush.name.clone();
    Outcome::KeepThen(Box::new(move |app: &mut App| {
        app.dialogs.push(Box::new(super::prompt::PromptDialog::new(
            "Save brush as",
            &name,
            Box::new(|app: &mut App, name: String| save_brush(app, name)),
        )))
    }))
}

fn pick(i: usize) -> Outcome {
    Outcome::KeepThen(Box::new(move |app: &mut App| app.tools.select_brush(i)))
}

impl PreviewKey {
    pub fn new(ts: &crate::tools_ctl::ToolState, w: usize, h: usize) -> Self {
        PreviewKey {
            spec: ts.pen_spec(),
            tiles: ts.charsets[ts.charset % ts.charsets.len()].chars.to_vec(),
            w,
            h,
            fg: ts.brush.fg,
            bg: ts.brush.bg,
        }
    }
}

/// The preview for `key`, re-rendered only when the key changes.
pub fn cached_preview<'c>(
    cache: &'c mut Option<(PreviewKey, Vec<Cell>)>,
    key: PreviewKey,
    doc: &Document,
) -> &'c [Cell] {
    if cache.as_ref().is_none_or(|(k, _)| *k != key) {
        let cells = render_preview(&key, doc);
        *cache = Some((key, cells));
    }
    cache.as_ref().map(|(_, c)| c.as_slice()).unwrap_or_default()
}

/// A sample stroke with the brush: one wave across the preview.
fn render_preview(key: &PreviewKey, doc: &Document) -> Vec<Cell> {
    let mut d = Document::new(doc.meta.kind, key.w, key.h);
    d.meta.palette = doc.meta.palette.clone();
    let spec = &key.spec;
    let cands = spec.glyphs.candidates(&key.tiles);
    let ctx = Ctx { brush: Brush { ch: '█', fg: key.fg, bg: key.bg }, ..Ctx::default() };
    let (pw, ph) = ((key.w * pen::GW) as f32, (key.h * pen::GH) as f32);
    let margin = spec.size.max(4.0) + 4.0;
    let amp = (ph / 2.0 - margin).max(0.0);
    let mut b = TxBuilder::new(&d, "preview");
    let mut s = PenStroke::with_brush(spec);
    let n = (pw / 3.0) as usize;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = margin + t * (pw - 2.0 * margin);
        let y = ph / 2.0 - (t * std::f32::consts::TAU).sin() * amp;
        let changed = s.add_point(x, y);
        pen::apply(&mut b, &ctx, &s, changed, &cands);
    }
    let changed = s.finish();
    pen::apply(&mut b, &ctx, &s, changed, &cands);
    let tx = b.finish();
    d.apply(&tx);
    (0..key.h).flat_map(|y| (0..key.w).map(move |x| (x, y))).map(|(x, y)| d.canvas.composite(x, y)).collect()
}

impl Dialog for BrushStudio {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App) {
        let ts = &app.tools;
        let spec = &ts.pen_brush;
        let r = centered(area, 88, Param::ALL.len() as u16 + PREVIEW_H + 6);
        let inner = popup(
            f,
            r,
            "Brush studio",
            "Tab pane · ↑↓ pick · ←→ adjust · S save as · D delete · R reset · Enter/Esc done",
        );
        let focus_style = |on: bool| if on { Style::new().fg(theme::ACCENT2) } else { Style::new().fg(theme::DIM) };

        // ---- presets
        let list_h = Param::ALL.len();
        f.render_widget(
            Paragraph::new(Span::styled(
                " BRUSHES",
                focus_style(self.focus == Focus::Presets).add_modifier(Modifier::BOLD),
            )),
            Rect::new(inner.x, inner.y, LIST_W, 1),
        );
        if ts.brush_idx < self.list_offset {
            self.list_offset = ts.brush_idx;
        } else if ts.brush_idx >= self.list_offset + list_h {
            self.list_offset = ts.brush_idx + 1 - list_h;
        }
        self.presets.clear();
        for (k, (i, b)) in ts.brushes.iter().enumerate().skip(self.list_offset).take(list_h).enumerate() {
            let row = Rect::new(inner.x, inner.y + 1 + k as u16, LIST_W, 1);
            let active = i == ts.brush_idx;
            let modified = active && b != spec;
            let yours = ts.user_brushes.contains(&b.name);
            let st = if active {
                Style::new().fg(theme::BG).bg(if self.focus == Focus::Presets { theme::ACCENT } else { theme::DIM })
            } else {
                Style::new().fg(theme::TEXT)
            };
            let mut name = format!(" {}{}", b.name, if modified { " •" } else { "" });
            name.truncate(LIST_W as usize - 2);
            let line = Line::from(vec![
                Span::styled(format!("{name:<w$}", w = LIST_W as usize - 2), st),
                Span::styled(if yours { "★" } else { " " }, Style::new().fg(theme::WARN)),
            ]);
            f.render_widget(Paragraph::new(line), row);
            self.presets.push((row, i));
        }

        // ---- parameters
        let px = inner.x + LIST_W + 2;
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{}{}", spec.name, if ts.brushes.get(ts.brush_idx) != Some(spec) { " (modified)" } else { "" }),
                focus_style(self.focus == Focus::Params).add_modifier(Modifier::BOLD),
            )),
            Rect::new(px, inner.y, inner.right().saturating_sub(px), 1),
        );
        self.sliders.clear();
        for (k, p) in Param::ALL.iter().enumerate() {
            let y = inner.y + 1 + k as u16;
            let sel = self.focus == Focus::Params && k == self.param;
            let label_st =
                if sel { Style::new().fg(theme::BG).bg(theme::ACCENT) } else { Style::new().fg(theme::TEXT) };
            let slider = Rect::new(px + LABEL_W, y, SLIDER_W, 1);
            let mut spans =
                vec![Span::styled(format!(" {:<w$}", p.label(), w = LABEL_W as usize - 2), label_st), Span::raw(" ")];
            match p.fraction(spec) {
                Some(t) => {
                    let knob = ((SLIDER_W - 1) as f32 * t).round() as usize;
                    let fill = Style::new().fg(if sel { theme::ACCENT } else { theme::ACCENT2 });
                    spans.push(Span::styled("━".repeat(knob), fill));
                    spans.push(Span::styled("●", fill.add_modifier(Modifier::BOLD)));
                    spans.push(Span::styled("─".repeat(SLIDER_W as usize - 1 - knob), Style::new().fg(theme::BORDER)));
                }
                None => {
                    let text = format!("◂ {} ▸", p.display(spec));
                    spans.push(Span::styled(
                        format!("{text:^w$}", w = SLIDER_W as usize),
                        Style::new().fg(theme::ACCENT2),
                    ));
                }
            }
            if p.fraction(spec).is_some() {
                spans.push(Span::styled(format!("  {}", p.display(spec)), Style::new().fg(theme::TEXT)));
            }
            f.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(px - 1, y, inner.right().saturating_sub(px - 1), 1),
            );
            self.sliders.push((slider, *p));
        }
        // What the selected parameter does.
        let help = match Param::ALL[self.param] {
            Param::Glyphs => "what the ink is drawn with — tile set follows [ ]",
            Param::Size => "tip radius in glyph pixels (a cell is 8x16)",
            Param::Hardness => "crisp edge ↔ edge fades out into shades",
            Param::Opacity => "most ink a stroke lays down; below 100% never solid",
            Param::Flow => "ink per dab; low flow builds up like an airbrush",
            Param::Spacing => "distance between dabs, % of the size",
            Param::Roundness => "flatten the tip into a calligraphy nib",
            Param::Angle => "angle of a flattened nib",
            Param::Square => "square tip, like a marker",
            Param::Scatter => "throw dabs around the path (spray)",
            Param::Count => "dabs per step",
            Param::Grain => "paper texture: breaks the ink like chalk",
            Param::Taper => "thin start and end of a stroke",
            Param::Velocity => "fast strokes come out thinner",
            Param::Streamline => "the pen trails the mouse for smooth curves",
        };
        let hy = inner.y + 1 + Param::ALL.len() as u16;
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {help}"), Style::new().fg(theme::DIM))),
            Rect::new(px - 1, hy, inner.right().saturating_sub(px - 1), 1),
        );

        // Buttons on the last row, so one mouse button does it all.
        self.btns.clear();
        let yours = ts.brushes.get(ts.brush_idx).is_some_and(|b| ts.user_brushes.contains(&b.name));
        let modified = ts.brushes.get(ts.brush_idx) != Some(spec);
        let bar = [
            btn(Act::Save, "S", "save as"),
            btn(Act::Delete, "D", "delete").enabled(yours),
            btn(Act::Reset, "R", "reset").enabled(modified),
            btn(Act::Done, "⏎", "done").primary(),
        ];
        let by = inner.bottom().saturating_sub(1);
        self.btns.row(f.buffer_mut(), Rect::new(inner.x + 1, by, inner.width.saturating_sub(2), 1), &bar);

        // ---- live preview
        let py = hy + 1;
        let prev = Rect::new(inner.x + 1, py, inner.width.saturating_sub(2), PREVIEW_H.min(by.saturating_sub(py)));
        if prev.height == 0 {
            return;
        }
        let t = app.tab();
        cached_preview(&mut self.preview, PreviewKey::new(ts, prev.width as usize, prev.height as usize), &t.doc);
        let pal = &t.doc.meta.palette;
        let Some((key, cells)) = &self.preview else { return };
        let buf = f.buffer_mut();
        for y in 0..key.h {
            for x in 0..key.w {
                let c = cells[y * key.w + x];
                let (fg, bg) = (canvas::rgb(c.fg, pal), canvas::rgb(if c.is_blank() { key.bg } else { c.bg }, pal));
                if let Some(bc) = buf.cell_mut((prev.x + x as u16, prev.y + y as u16)) {
                    bc.set_char(if c.ch == '\0' { ' ' } else { c.ch }).set_style(Style::new().fg(fg).bg(bg));
                }
            }
        }
    }

    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome {
        let n = app.tools.brushes.len();
        let fast = if k.modifiers.contains(KeyModifiers::SHIFT) { 5 } else { 1 };
        match k.code {
            KeyCode::Esc | KeyCode::Enter => Outcome::Close,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Presets { Focus::Params } else { Focus::Presets };
                Outcome::Keep
            }
            KeyCode::Up | KeyCode::Down => {
                let d = if k.code == KeyCode::Up { -1 } else { 1 };
                match self.focus {
                    Focus::Presets => pick((app.tools.brush_idx as i32 + d).rem_euclid(n.max(1) as i32) as usize),
                    Focus::Params => {
                        self.param = (self.param as i32 + d).rem_euclid(Param::ALL.len() as i32) as usize;
                        Outcome::Keep
                    }
                }
            }
            KeyCode::Left | KeyCode::Right => {
                let d = if k.code == KeyCode::Left { -fast } else { fast };
                match self.focus {
                    Focus::Params => {
                        let p = Param::ALL[self.param];
                        edit(app, |b| p.nudge(b, d))
                    }
                    Focus::Presets => {
                        self.focus = Focus::Params;
                        Outcome::Keep
                    }
                }
            }
            KeyCode::Char('-') => edit(app, |b| Param::Size.nudge(b, -fast)),
            KeyCode::Char('=') | KeyCode::Char('+') => edit(app, |b| Param::Size.nudge(b, fast)),
            KeyCode::Char(c) => match c.to_ascii_lowercase() {
                'r' => pick(app.tools.brush_idx),
                's' => save_as(app),
                'd' => delete_brush(app),
                _ => Outcome::Keep,
            },
            KeyCode::Delete | KeyCode::Backspace => delete_brush(app),
            _ => Outcome::Keep,
        }
    }

    fn mouse(&mut self, m: MouseEvent, app: &App) -> Outcome {
        let inside = |r: &Rect| m.row == r.y && m.column >= r.x && m.column < r.right();
        let slide = |p: Param, r: Rect| {
            let t = (m.column.saturating_sub(r.x)) as f32 / (r.width - 1).max(1) as f32;
            edit(app, |b| p.set_fraction(b, t))
        };
        match self.btns.mouse(&m) {
            Some(Act::Save) => return save_as(app),
            Some(Act::Delete) => return delete_brush(app),
            Some(Act::Reset) => return pick(app.tools.brush_idx),
            Some(Act::Done) => return Outcome::Close,
            None => {}
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(&(_, i)) = self.presets.iter().find(|(r, _)| inside(r)) {
                    self.focus = Focus::Presets;
                    return pick(i);
                }
                // A click on the row's label selects it; on the slider it sets it.
                let row = self.sliders.iter().enumerate().find(|(_, (r, _))| {
                    m.row == r.y && m.column >= r.x.saturating_sub(LABEL_W) && m.column < r.right() + 12
                });
                if let Some((k, &(r, p))) = row {
                    self.focus = Focus::Params;
                    self.param = k;
                    if m.column < r.x {
                        return Outcome::Keep;
                    }
                    return match p.fraction(&app.tools.pen_brush) {
                        Some(_) => {
                            self.dragging = Some(p);
                            slide(p, r)
                        }
                        // Choices: left half steps back, right half forward.
                        None => edit(app, |b| p.nudge(b, if m.column < r.x + r.width / 2 { -1 } else { 1 })),
                    };
                }
                Outcome::Keep
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.dragging {
                Some(p) => {
                    let r = self.sliders.iter().find(|(_, q)| *q == p).map(|(r, _)| *r).unwrap_or_default();
                    slide(p, r)
                }
                None => Outcome::Keep,
            },
            MouseEventKind::Up(_) => {
                self.dragging = None;
                Outcome::Keep
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let d = if m.kind == MouseEventKind::ScrollUp { 1 } else { -1 };
                if let Some(&(_, p)) = self.sliders.iter().find(|(r, _)| m.row == r.y) {
                    return edit(app, |b| p.nudge(b, d));
                }
                Outcome::Keep
            }
            _ => Outcome::Keep,
        }
    }
}

fn save_brush(app: &mut App, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let spec = BrushSpec { name: name.clone(), ..app.tools.pen_brush.clone() };
    let note = if app.tools.user_brushes.contains(&name) {
        " (replaces the one saved before)"
    } else if app.tools.brushes.iter().any(|b| b.name == name) {
        " (in place of the built-in; delete it to get that back)"
    } else {
        ""
    };
    match acidtrip_io::brushes::save(&app.paths.brushes_dir(), &spec) {
        Ok(_) => {
            app.reload_brushes();
            if let Some(i) = app.tools.brushes.iter().position(|b| b.name == name) {
                app.tools.select_brush(i);
            }
            app.flash(format!("saved brush \"{name}\"{note}"), Level::Ok);
        }
        Err(e) => app.flash(format!("can't save brush: {e:#}"), Level::Error),
    }
}

fn delete_brush(app: &App) -> Outcome {
    let Some(name) = app.tools.brushes.get(app.tools.brush_idx).map(|b| b.name.clone()) else { return Outcome::Keep };
    if !app.tools.user_brushes.contains(&name) {
        return Outcome::KeepThen(Box::new(|app: &mut App| {
            app.flash("built-in brushes can't be deleted (R resets one)", Level::Warn)
        }));
    }
    Outcome::KeepThen(Box::new(move |app: &mut App| {
        app.dialogs.push(Box::new(super::prompt::ConfirmDialog::new(
            &format!("Delete the brush \"{name}\"?"),
            Box::new(move |app: &mut App| {
                let at = app.tools.brush_idx;
                match acidtrip_io::brushes::delete(&app.paths.brushes_dir(), &name) {
                    Ok(()) => {
                        app.reload_brushes();
                        // Stay put in the list (or on the built-in it hid).
                        let i = app.tools.brushes.iter().position(|b| b.name == name).unwrap_or(at);
                        app.tools.select_brush(i.min(app.tools.brushes.len().saturating_sub(1)));
                        app.flash(format!("deleted brush \"{name}\""), Level::Ok);
                    }
                    Err(e) => app.flash(format!("can't delete brush: {e:#}"), Level::Error),
                }
            }),
        )));
    }))
}
