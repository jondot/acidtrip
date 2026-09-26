//! Canvas size: width and height, one-click presets, "fit art", and a
//! preview of the new size over the art, so a crop shows before it happens.
//! The art stays at the top-left; growing adds blank space right and below.

use acidtrip_core::Document;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::widgets::{Btn, Buttons, LineInput, btn, centered, popup, theme};

const W: u16 = 64;
const PREVIEW_W: usize = 40;
const PREVIEW_H: usize = 8;
const PRESETS: [(usize, usize); 5] = [(80, 25), (80, 50), (80, 100), (160, 25), (160, 50)];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Do {
    Preset(usize, usize),
    Fit,
    MoreRows,
    Apply,
    Cancel,
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Width,
    Height,
}

pub struct CanvasSizeDialog {
    from: (usize, usize),
    width: LineInput,
    height: LineInput,
    focus: Field,
    /// Smallest size holding every inked cell (None: empty canvas).
    fit: Option<(usize, usize)>,
    /// Ink map of the canvas, sampled for the preview.
    ink: Vec<bool>,
    btns: Buttons<Do>,
    fields: [(Rect, Field); 2],
}

/// Inked cells (any layer) and the size that just holds them.
fn ink(doc: &Document) -> (Vec<bool>, Option<(usize, usize)>) {
    let (w, h) = (doc.width(), doc.height());
    let mut v = vec![false; w * h];
    let mut fit: Option<(usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if !doc.canvas.composite(x, y).is_blank() {
                v[y * w + x] = true;
                let (fw, fh) = fit.unwrap_or((0, 0));
                fit = Some((fw.max(x + 1), fh.max(y + 1)));
            }
        }
    }
    (v, fit)
}

impl CanvasSizeDialog {
    pub fn new(app: &App) -> Self {
        let d = &app.tab().doc;
        let (ink, fit) = ink(d);
        CanvasSizeDialog {
            from: (d.width(), d.height()),
            width: LineInput::new(&d.width().to_string()),
            height: LineInput::new(&d.height().to_string()),
            focus: Field::Width,
            fit,
            ink,
            btns: Buttons::default(),
            fields: [(Rect::default(), Field::Width), (Rect::default(), Field::Height)],
        }
    }

    /// The size typed in (unparsable = unchanged), clamped like Document properties.
    fn to(&self) -> (usize, usize) {
        let n = |i: &LineInput, d: usize| i.text.trim().parse::<usize>().unwrap_or(d);
        (n(&self.width, self.from.0).clamp(1, 4000), n(&self.height, self.from.1).clamp(1, 20000))
    }

    fn set(&mut self, (w, h): (usize, usize)) {
        self.width = LineInput::new(&w.to_string());
        self.height = LineInput::new(&h.to_string());
    }

    /// Inked cells outside the new size.
    fn cropped(&self) -> usize {
        let (w, h) = self.to();
        let fw = self.from.0;
        self.ink.iter().enumerate().filter(|&(i, &on)| on && (i % fw >= w || i / fw >= h)).count()
    }

    fn act(&mut self, d: Do) -> Outcome {
        match d {
            Do::Preset(w, h) => self.set((w, h)),
            Do::Fit => {
                if let Some(s) = self.fit {
                    self.set(s);
                }
            }
            Do::MoreRows => {
                let (w, h) = self.to();
                self.set((w, h + 25));
            }
            Do::Cancel => return Outcome::Close,
            Do::Apply => {
                let (w, h) = self.to();
                if (w, h) == self.from {
                    return Outcome::Close;
                }
                let cropped = self.cropped();
                return Outcome::Then(Box::new(move |app: &mut App| {
                    app.tab_mut().edit("Canvas size", |b| acidtrip_core::tools::resize(b, w, h));
                    let note = if cropped > 0 { format!(" — cropped {cropped} cells") } else { String::new() };
                    app.flash(format!("canvas {w}×{h}{note} (Ctrl-Z undoes)"), Level::Ok);
                }));
            }
        }
        Outcome::Keep
    }

    fn input(&mut self) -> &mut LineInput {
        match self.focus {
            Field::Width => &mut self.width,
            Field::Height => &mut self.height,
        }
    }

    fn buttons(&self) -> Vec<Btn<'static, Do>> {
        let to = self.to();
        let mut v: Vec<Btn<Do>> = PRESETS
            .iter()
            .map(|&(w, h)| {
                let b = btn(Do::Preset(w, h), "", format!("{w}×{h}"));
                if (w, h) == to { b.kind(crate::ui::widgets::BtnKind::On) } else { b }
            })
            .collect();
        v.push(match self.fit {
            Some((w, h)) => btn(Do::Fit, "", format!("fit art {w}×{h}")),
            None => btn(Do::Fit, "", "fit art").enabled(false),
        });
        v.push(btn(Do::MoreRows, "", "+25 rows"));
        v
    }

    /// Old canvas (its art), new canvas outline, and art that would be cut, scaled down.
    fn preview(&self, f: &mut Frame, r: Rect) {
        let (fw, fh) = self.from;
        let (tw, th) = self.to();
        let (mw, mh) = (fw.max(tw), fh.max(th));
        // One preview cell covers s×s canvas cells (same shape, so the aspect holds).
        let s = (mw as f32 / PREVIEW_W as f32).max(mh as f32 / PREVIEW_H as f32).max(1.0);
        let (sx, sy) = (s, s);
        let cols = ((mw as f32 / sx).ceil() as usize).min(PREVIEW_W);
        let rows = ((mh as f32 / sy).ceil() as usize).min(PREVIEW_H);
        let buf = f.buffer_mut();
        for py in 0..rows {
            for px in 0..cols {
                let (x0, y0) = ((px as f32 * sx) as usize, (py as f32 * sy) as usize);
                let (x1, y1) = (((px + 1) as f32 * sx) as usize, ((py + 1) as f32 * sy) as usize);
                let inside_old = x0 < fw && y0 < fh;
                let inside_new = x0 < tw && y0 < th;
                let inked = (y0..y1.min(fh)).any(|y| (x0..x1.min(fw)).any(|x| self.ink[y * fw + x]));
                let (ch, st) = match (inside_old, inside_new, inked) {
                    (true, false, true) => ('▓', Style::new().fg(theme::ERR)),
                    (true, false, false) => ('░', Style::new().fg(theme::BORDER)),
                    (true, true, true) => ('▓', Style::new().fg(theme::TEXT)),
                    (true, true, false) => ('·', Style::new().fg(theme::BORDER)),
                    (false, true, _) => ('░', Style::new().fg(theme::ACCENT2)),
                    (false, false, _) => (' ', Style::new()),
                };
                if let Some(c) = buf.cell_mut((r.x + px as u16, r.y + py as u16)) {
                    c.set_char(ch).set_style(st);
                }
            }
        }
    }
}

impl Dialog for CanvasSizeDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, W, 21);
        let inner = popup(f, r, "Canvas size", "art stays top-left · Enter resize · Esc cancel");
        if inner.height < 12 {
            return;
        }
        self.btns.clear();
        let dim = Style::new().fg(theme::DIM);
        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);
        let mut y = inner.y + 1;
        let wr = Rect::new(x, y, 20, 1);
        let hr = Rect::new(x + 24, y, 20, 1);
        self.width.render(f, wr, "Width  › ", self.focus == Field::Width);
        self.height.render(f, hr, "Height › ", self.focus == Field::Height);
        self.fields = [(wr, Field::Width), (hr, Field::Height)];
        y += 2;
        f.render_widget(Paragraph::new(Span::styled("Presets", dim)), Rect::new(x, y, 8, 1));
        y += self.btns.row(f.buffer_mut(), Rect::new(x + 9, y, w.saturating_sub(9), 2), &self.buttons()) + 1;

        // What changes.
        let (fw, fh) = self.from;
        let (tw, th) = self.to();
        let mut what = vec![];
        if tw != fw {
            what.push(format!("{} {} columns on the right", if tw > fw { "adds" } else { "removes" }, tw.abs_diff(fw)));
        }
        if th != fh {
            what.push(format!("{} {} rows at the bottom", if th > fh { "adds" } else { "removes" }, th.abs_diff(fh)));
        }
        let head = Line::from(vec![
            Span::styled(format!("{fw}×{fh} → "), dim),
            Span::styled(format!("{tw}×{th}"), Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD)),
            Span::styled(if what.is_empty() { "  (no change)".into() } else { format!("  {}", what.join(", ")) }, dim),
        ]);
        f.render_widget(Paragraph::new(head), Rect::new(x, y, w, 1));
        y += 1;
        let cropped = self.cropped();
        let warn = if cropped > 0 {
            Span::styled(format!("⚠ cuts off {cropped} cells of art (red below)"), Style::new().fg(theme::ERR))
        } else {
            Span::styled("keeps all the art", Style::new().fg(theme::OK))
        };
        f.render_widget(Paragraph::new(warn), Rect::new(x, y, w, 1));
        y += 2;
        self.preview(f, Rect::new(x + 2, y, PREVIEW_W as u16, PREVIEW_H as u16));
        let legend = [("▓", theme::TEXT, "art"), ("░", theme::ACCENT2, "new space"), ("▓", theme::ERR, "cut off")];
        for (i, (g, c, l)) in legend.iter().enumerate() {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(*g, Style::new().fg(*c)),
                    Span::styled(format!(" {l}"), dim),
                ])),
                Rect::new(x + 4 + PREVIEW_W as u16, y + i as u16, 14, 1),
            );
        }
        let by = inner.bottom() - 1;
        let bar = [btn(Do::Apply, "⏎", "resize").primary(), btn(Do::Cancel, "esc", "cancel")];
        self.btns.row(f.buffer_mut(), Rect::new(x, by, w, 1), &bar);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let big = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter => self.act(Do::Apply),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Field::Width { Field::Height } else { Field::Width };
                Outcome::Keep
            }
            // Up/Down nudge the focused number (Shift: by 10).
            KeyCode::Up | KeyCode::Down => {
                let (w, h) = self.to();
                let step = if big { 10 } else { 1 };
                let v = if self.focus == Field::Width { w } else { h };
                let v = if k.code == KeyCode::Up { v + step } else { v.saturating_sub(step).max(1) };
                *self.input() = LineInput::new(&v.to_string());
                Outcome::Keep
            }
            KeyCode::Char(c) if !c.is_ascii_digit() && k.modifiers.is_empty() => Outcome::Keep,
            _ => {
                self.input().key(&k);
                Outcome::Keep
            }
        }
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        if let Some(d) = self.btns.mouse(&m) {
            return self.act(d);
        }
        if let MouseEventKind::Down(MouseButton::Left) = m.kind
            && let Some(&(_, fld)) =
                self.fields.iter().find(|(r, _)| m.row == r.y && m.column >= r.x && m.column < r.right())
        {
            self.focus = fld;
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        let digits: String = s.chars().filter(char::is_ascii_digit).collect();
        self.input().paste(&digits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{Cell, Color, DocKind};

    fn dialog(doc: &Document) -> CanvasSizeDialog {
        let (ink, fit) = ink(doc);
        CanvasSizeDialog {
            from: (doc.width(), doc.height()),
            width: LineInput::new(&doc.width().to_string()),
            height: LineInput::new(&doc.height().to_string()),
            focus: Field::Width,
            fit,
            ink,
            btns: Buttons::default(),
            fields: [(Rect::default(), Field::Width), (Rect::default(), Field::Height)],
        }
    }

    #[test]
    fn fit_and_crop_count_the_art() {
        let mut d = Document::new(DocKind::Classic, 80, 25);
        for (x, y) in [(3, 2), (40, 10), (70, 20)] {
            d.canvas.layers[0].cells[y * 80 + x] = Some(Cell::new('█', Color::WHITE, Color::BLACK));
        }
        let mut dlg = dialog(&d);
        assert_eq!(dlg.fit, Some((71, 21)));
        dlg.act(Do::Fit);
        assert_eq!(dlg.to(), (71, 21));
        assert_eq!(dlg.cropped(), 0);
        dlg.act(Do::Preset(40, 25));
        assert_eq!(dlg.cropped(), 2, "x=40 and x=70 fall outside 40 columns");
        dlg.act(Do::MoreRows);
        assert_eq!(dlg.to(), (40, 50));
    }
}
