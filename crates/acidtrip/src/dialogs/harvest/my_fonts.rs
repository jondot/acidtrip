//! My fonts: every font harvested so far. Pick one to see which letters it
//! has (cut from art, or ✦ drawn by Claude) and which it's missing, look at
//! any letter up close, type a sample, delete bad letters, have Claude draw
//! the missing ones, or take it to the Font tool.

use std::path::{Path, PathBuf};
use std::time::Instant;

use acidtrip_ai::harvest::{self, HarvestedFont};
use acidtrip_core::{Clip, Palette};
use acidtrip_io::fonts::{FontLibrary, TextRenderOptions};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Act, Shared, credit_spans_of};
use crate::app::Level;
use crate::ui::canvas::draw_clip;
use crate::ui::widgets::{Btn, Buttons, LineInput, ListState, btn, list_line, theme};

const LIST_W: u16 = 30;
/// The coverage grid's rows.
const GRID: [&str; 3] = ["ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz", "0123456789"];

/// Where an action sends the studio.
pub(crate) enum Go {
    Stay,
    Back,
    /// To Find art (empty library).
    Find,
    /// Close and open the Font tool on this font.
    Use(PathBuf),
}

/// Buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Do {
    Use,
    Complete,
    DeleteLetter,
    DeleteFont,
    Undo,
    Back,
    Find,
}

#[derive(Default)]
pub(crate) struct FontsStage {
    /// Selected font by file (the list reloads under it).
    file: Option<PathBuf>,
    list: ListState,
    letter: char,
    sample: LineInput,
    sample_focus: bool,
    /// Delete font was clicked once: the next click deletes.
    armed: bool,
    /// The last deleted letter (font, char, glyph, its base row, drawn by
    /// Claude), for undo.
    deleted: Option<(PathBuf, char, Clip, Option<usize>, bool)>,
    /// Renderer for samples (reloaded when a font changes on disk).
    lib: Option<(Vec<(PathBuf, usize)>, FontLibrary)>,
    list_area: Rect,
    sample_area: Rect,
    /// Grid cells: (x, y, char).
    grid: Vec<(u16, u16, char)>,
}

impl FontsStage {
    /// Show `file` (or keep the selection).
    pub fn show(&mut self, file: Option<&Path>, sh: &Shared) {
        if let Some(f) = file {
            self.file = Some(f.to_path_buf());
        }
        if self.current(sh).is_none() {
            self.file = sh.fonts.first().map(|f| f.file.clone());
        }
        self.picked(sh);
    }

    fn index(&self, sh: &Shared) -> Option<usize> {
        let file = self.file.as_ref()?;
        sh.fonts.iter().position(|f| f.file == *file)
    }

    fn current<'s>(&self, sh: &'s Shared) -> Option<&'s HarvestedFont> {
        self.index(sh).map(|i| &sh.fonts[i])
    }

    /// A font was picked: first letter it has, a sample it can spell.
    fn picked(&mut self, sh: &Shared) {
        self.armed = false;
        let Some(f) = self.current(sh) else { return };
        self.letter = f.spec.glyphs.keys().copied().find(char::is_ascii_alphanumeric).unwrap_or('A');
        let have: String = f.spec.glyphs.keys().collect();
        let sample = super::cutter::new_words(&have)
            .into_iter()
            .next()
            .unwrap_or_else(|| f.spec.glyphs.keys().filter(|c| c.is_ascii_uppercase()).take(10).collect());
        self.sample = LineInput::new(&sample);
    }

    fn select(&mut self, i: usize, sh: &Shared) {
        if let Some(f) = sh.fonts.get(i) {
            self.file = Some(f.file.clone());
            self.list.selected = i;
            self.picked(sh);
        }
    }

    fn drawing(&self, sh: &Shared) -> Option<Instant> {
        let file = self.file.as_ref()?;
        sh.completing.iter().find(|(p, _)| p == file).map(|(_, t)| *t)
    }

    pub fn buttons(&self, sh: &Shared) -> Vec<Btn<'static, Do>> {
        let Some(f) = self.current(sh) else {
            return vec![btn(Do::Find, "1", "find art to cut").primary(), btn(Do::Back, "esc", "back")];
        };
        let missing = f.missing(harvest::COMPLETE_CHARS).len();
        let complete = match self.drawing(sh) {
            Some(t) => btn(Do::Complete, "✦", format!("Claude is drawing… {}s", t.elapsed().as_secs())).enabled(false),
            None if missing == 0 => btn(Do::Complete, "a", "A–Z 0–9 complete").enabled(false),
            None => btn(Do::Complete, "a", format!("Claude draws {missing} missing")).enabled(sh.agent.is_some()),
        };
        let has = f.spec.glyphs.contains_key(&self.letter);
        let mut v = vec![
            btn(Do::Use, "⏎", "use in Font tool").primary(),
            complete,
            btn(Do::DeleteLetter, "x", format!("delete \u{201c}{}\u{201d}", self.letter)).enabled(has),
        ];
        if let Some((_, c, _, _, _)) = &self.deleted {
            v.push(btn(Do::Undo, "u", format!("undo delete \u{201c}{c}\u{201d}")));
        }
        v.push(btn(Do::DeleteFont, "D", if self.armed { "click again: delete font" } else { "delete font" }));
        v.push(btn(Do::Back, "esc", "back"));
        v
    }

    pub fn act(&mut self, d: Do, sh: &mut Shared) -> Go {
        if d != Do::DeleteFont {
            self.armed = false;
        }
        match d {
            Do::Back => return Go::Back,
            Do::Find => return Go::Find,
            Do::Use => {
                if let Some(f) = self.current(sh) {
                    return Go::Use(f.file.clone());
                }
            }
            Do::Complete => {
                if let Some(f) = self.current(sh).cloned() {
                    sh.complete(&f);
                }
            }
            Do::DeleteLetter => {
                let Some(mut f) = self.current(sh).cloned() else { return Go::Stay };
                if let Some(g) = f.spec.glyphs.get(&self.letter).cloned() {
                    let drawn = f.spec.generated.contains(&self.letter);
                    let base = f.spec.bases.get(&self.letter).copied();
                    self.deleted = Some((f.file.clone(), self.letter, g, base, drawn));
                    f.remove(self.letter);
                    self.save(f, format!("deleted \u{201c}{}\u{201d}", self.letter), sh);
                }
            }
            Do::Undo => {
                let Some((file, c, g, base, drawn)) = self.deleted.take() else { return Go::Stay };
                let Some(mut f) = sh.fonts.iter().find(|f| f.file == file).cloned() else { return Go::Stay };
                f.spec.insert(c, g, base);
                if drawn {
                    f.spec.generated.insert(c);
                }
                self.letter = c;
                self.save(f, format!("put \u{201c}{c}\u{201d} back"), sh);
            }
            Do::DeleteFont if !self.armed => self.armed = true,
            Do::DeleteFont => {
                self.armed = false;
                let Some(f) = self.current(sh).cloned() else { return Go::Stay };
                match f.delete() {
                    Ok(()) => {
                        sh.wrote = true;
                        sh.say(format!("deleted font {}", f.title()), Level::Ok);
                        sh.reload_fonts();
                        let i = self.list.selected.min(sh.fonts.len().saturating_sub(1));
                        self.file = None;
                        self.select(i, sh);
                    }
                    Err(e) => sh.say(format!("{e:#}"), Level::Error),
                }
            }
        }
        Go::Stay
    }

    fn save(&mut self, f: HarvestedFont, msg: String, sh: &mut Shared) {
        match f.save() {
            Ok(()) => {
                sh.wrote = true;
                sh.say(format!("{msg} from {}", f.title()), Level::Ok);
                sh.reload_fonts();
            }
            Err(e) => sh.say(format!("{e:#}"), Level::Error),
        }
    }

    /// Letters in grid order plus any others the font has.
    fn letters(&self, sh: &Shared) -> Vec<char> {
        let mut v: Vec<char> = GRID.concat().chars().collect();
        if let Some(f) = self.current(sh) {
            v.extend(f.spec.glyphs.keys().filter(|c| !c.is_ascii_alphanumeric()));
        }
        v
    }

    pub fn key(&mut self, k: &KeyEvent, sh: &mut Shared) -> Option<Do> {
        if self.sample_focus {
            match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Tab => self.sample_focus = false,
                _ => {
                    self.sample.key(k);
                }
            }
            return None;
        }
        let n = sh.fonts.len();
        match k.code {
            KeyCode::Esc => return Some(Do::Back),
            KeyCode::Enter => return Some(Do::Use),
            KeyCode::Char('a') => return Some(Do::Complete),
            KeyCode::Char('x') | KeyCode::Delete => return Some(Do::DeleteLetter),
            KeyCode::Char('u') => return Some(Do::Undo),
            KeyCode::Char('D') => return Some(Do::DeleteFont),
            KeyCode::Char('1') if n == 0 => return Some(Do::Find),
            KeyCode::Tab => self.sample_focus = true,
            KeyCode::Up | KeyCode::Down if n > 0 => {
                let i = self.index(sh).unwrap_or(0);
                let j = if k.code == KeyCode::Up { i.saturating_sub(1) } else { (i + 1).min(n - 1) };
                self.select(j, sh);
            }
            KeyCode::Left | KeyCode::Right => {
                let all = self.letters(sh);
                let i = all.iter().position(|&c| c == self.letter).unwrap_or(0);
                let j = if k.code == KeyCode::Left { i.saturating_sub(1) } else { (i + 1).min(all.len() - 1) };
                self.letter = all[j];
                self.armed = false;
            }
            _ => {}
        }
        None
    }

    pub fn mouse(&mut self, m: MouseEvent, sh: &Shared) {
        let MouseEventKind::Down(MouseButton::Left) = m.kind else {
            if let (MouseEventKind::ScrollDown | MouseEventKind::ScrollUp, Some(i)) = (m.kind, self.index(sh)) {
                let j = if m.kind == MouseEventKind::ScrollUp { i.saturating_sub(1) } else { i + 1 };
                self.select(j.min(sh.fonts.len().saturating_sub(1)), sh);
            }
            return;
        };
        let r = self.list_area;
        if m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right() {
            self.select(self.list.offset + (m.row - r.y) as usize, sh);
            return;
        }
        self.sample_focus =
            m.row == self.sample_area.y && m.column >= self.sample_area.x && m.column < self.sample_area.right();
        if let Some(&(_, _, c)) = self.grid.iter().find(|(x, y, _)| m.row == *y && m.column == *x) {
            self.letter = c;
            self.armed = false;
        }
    }

    pub fn paste(&mut self, s: &str) {
        if self.sample_focus {
            self.sample.paste(s);
        }
    }

    /// Render `text` the way the Font tool will.
    fn render(&mut self, f: &HarvestedFont, text: &str, sh: &Shared) -> Option<Clip> {
        let stamp: Vec<(PathBuf, usize)> = sh.fonts.iter().map(|f| (f.file.clone(), f.spec.glyphs.len())).collect();
        if self.lib.as_ref().is_none_or(|(s, _)| *s != stamp) {
            let dir = HarvestedFont::dir(&sh.paths.fonts_dir());
            self.lib = Some((stamp, FontLibrary::load(Some(&dir))));
        }
        let lib = &self.lib.as_ref()?.1;
        let info = lib.list().into_iter().find(|i| i.path.as_deref() == Some(f.file.as_path()))?;
        lib.render(&info.id, text, &TextRenderOptions::default()).ok()
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, sh: &Shared, _btns: &mut Buttons<Act>) {
        let dim = Style::new().fg(theme::DIM);
        if self.current(sh).is_none() && !sh.fonts.is_empty() {
            self.file = sh.fonts.first().map(|f| f.file.clone());
        }
        if sh.fonts.is_empty() {
            let lines = vec![
                Line::from(Span::styled("No harvested fonts yet.", Style::new().fg(theme::TEXT))),
                Line::from(""),
                Line::from(Span::styled(
                    "Find a logo in scene art and cut its letters: they become a font in that artist's style.",
                    dim,
                )),
            ];
            f.render_widget(Paragraph::new(lines), area);
            return;
        }
        // The list.
        let lw = LIST_W.min(area.width / 3);
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("MY FONTS  {}", sh.fonts.len()),
                Style::new().fg(theme::DIM).add_modifier(Modifier::BOLD),
            )),
            Rect::new(area.x, area.y, lw, 1),
        );
        let lr = Rect::new(area.x, area.y + 1, lw, area.height.saturating_sub(1));
        self.list_area = lr;
        let sel = self.index(sh).unwrap_or(0);
        self.list.selected = sel;
        let range = self.list.visible(sh.fonts.len(), lr.height as usize);
        let lines: Vec<Line> = range
            .map(|i| {
                let hf = &sh.fonts[i];
                let busy = sh.completing.iter().any(|(p, _)| *p == hf.file);
                let right =
                    if busy { format!("{} ✦", sh.spinner()) } else { super::cutter::alnum(&hf.spec).to_string() };
                list_line(hf.title(), right, i == sel, lr.width)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), lr);

        // The selected font.
        let Some(font) = self.current(sh).cloned() else { return };
        let x = area.x + lw + 2;
        let w = area.right().saturating_sub(x);
        let mut y = area.y;
        let drawn = font.spec.generated.iter().filter(|c| c.is_ascii_alphanumeric()).count();
        let mut head = vec![Span::styled(font.title(), Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD))];
        head.push(Span::styled(
            format!(
                "  {} of 62 letters{}",
                super::cutter::alnum(&font.spec),
                if drawn > 0 { format!(" · {drawn} drawn by Claude") } else { String::new() }
            ),
            dim,
        ));
        f.render_widget(Paragraph::new(Line::from(head)), Rect::new(x, y, w, 1));
        y += 1;
        for a in font.attribution.iter().take(2) {
            let mut spans = vec![Span::styled("from ", dim)];
            spans.extend(credit_spans_of(a));
            f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(x, y, w, 1));
            y += 1;
        }
        if font.attribution.len() > 2 {
            let more = format!("and {} more sources", font.attribution.len() - 2);
            f.render_widget(Paragraph::new(Span::styled(more, dim)), Rect::new(x, y, w, 1));
            y += 1;
        }
        y += 1;

        // Coverage grid: click a letter to see it.
        self.grid.clear();
        let extras: String = font.spec.glyphs.keys().filter(|c| !c.is_ascii_alphanumeric()).collect();
        let rows = [GRID[0], GRID[1], GRID[2], extras.as_str()];
        let buf = f.buffer_mut();
        for row in rows.iter().filter(|r| !r.is_empty()) {
            if y >= area.bottom() {
                return;
            }
            let mut cx = x;
            for c in row.chars() {
                if cx + 1 >= area.right() {
                    break;
                }
                let has = font.spec.glyphs.contains_key(&c);
                let ai = font.spec.generated.contains(&c);
                let mut st = if ai {
                    Style::new().fg(theme::ACCENT2)
                } else if has {
                    Style::new().fg(theme::OK).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(theme::BORDER)
                };
                if c == self.letter {
                    st = st.bg(theme::PANEL_HI).add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
                }
                buf.set_string(cx, y, c.to_string(), st);
                self.grid.push((cx, y, c));
                cx += 2;
            }
            y += 1;
        }
        let legend = Line::from(vec![
            Span::styled("A", Style::new().fg(theme::OK).add_modifier(Modifier::BOLD)),
            Span::styled(" cut from art   ", dim),
            Span::styled("A", Style::new().fg(theme::ACCENT2)),
            Span::styled(" drawn by Claude   ", dim),
            Span::styled("A", Style::new().fg(theme::BORDER)),
            Span::styled(" missing   click a letter to see it", dim),
        ]);
        f.render_widget(Paragraph::new(legend), Rect::new(x, y, w, 1));
        y += 2;

        // The letter up close.
        let c = self.letter;
        let about = match font.spec.glyphs.get(&c) {
            Some(g) if font.spec.generated.contains(&c) => format!("drawn by Claude · {}x{}", g.width, g.height),
            Some(g) => format!("cut from art · {}x{}", g.width, g.height),
            None if sh.agent.is_some() => "missing — Claude can draw it (a)".to_string(),
            None => "missing — cut it from another logo by this artist".to_string(),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("\u{201c}{c}\u{201d} "),
                    Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(about, dim),
            ])),
            Rect::new(x, y, w, 1),
        );
        y += 1;
        let gh = font.spec.height().max(1) as u16;
        if let Some(g) = font.spec.glyphs.get(&c) {
            let r = Rect::new(x, y, (g.width as u16).min(w), (g.height as u16).min(area.bottom().saturating_sub(y)));
            draw_clip(f.buffer_mut(), r, g, &Palette::default());
        } else if let Some(t) = self.drawing(sh) {
            let msg = format!("{} Claude is drawing the missing letters… {}s", sh.spinner(), t.elapsed().as_secs());
            f.render_widget(Paragraph::new(Span::styled(msg, Style::new().fg(theme::ACCENT2))), Rect::new(x, y, w, 1));
        }
        y += gh + 1;

        // A sample line in the font.
        if y >= area.bottom() {
            return;
        }
        self.sample_area = Rect::new(x, y, w.min(50), 1);
        self.sample.render(f, self.sample_area, "Sample › ", self.sample_focus);
        y += 1;
        let text = self.sample.text.clone();
        if let Some(clip) = self.render(&font, &text, sh)
            && y < area.bottom()
        {
            let r = Rect::new(x, y, (clip.width as u16).min(w), (clip.height as u16).min(area.bottom() - y));
            draw_clip(f.buffer_mut(), r, &clip, &Palette::default());
            if clip.width as u16 > w {
                f.render_widget(Paragraph::new(Span::styled("…", dim)), Rect::new(area.right() - 1, y, 1, 1));
            }
        }
    }
}
