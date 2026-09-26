//! Stage 2: every logo candidate found in the source, as cards with live
//! previews. Save stencils, mark several, open the letter cutter, or ask
//! Claude to read the letters. The side panel shows which of your fonts the
//! logo's letters would go into, and what that font has so far.

use std::collections::{BTreeSet, HashSet};

use acidtrip_ai::harvest::{self, Candidate, LetterReading};
use acidtrip_core::Palette;
use acidtrip_io::stencils::StencilLibrary;
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Act, Shared, coverage_line, credit_spans, ellipsize};
use crate::app::Level;
use crate::ui::canvas::draw_clip;
use crate::ui::widgets::{Btn, Buttons, LineInput, btn, theme};

/// Where an action sends the studio.
pub(crate) enum Go {
    Stay,
    Back,
    Cut,
}

/// Buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Do {
    Cut,
    Stencil,
    Mark,
    Read,
    Back,
    /// Stencil name prompt.
    SaveNamed,
    CancelName,
}

#[derive(Default)]
pub(crate) struct CandStage {
    pub source: String,
    arts: usize,
    pub items: Vec<Candidate>,
    selected: usize,
    top: usize,
    marked: BTreeSet<usize>,
    /// Stencil name prompt and the candidates it saves.
    naming: Option<(LineInput, Vec<usize>)>,
    saved: HashSet<String>,
    /// Candidates cut into fonts.
    pub done: HashSet<String>,
    hits: Vec<(Rect, usize)>,
}

/// A stencil name for a candidate: SAUCE title, else the file name.
pub(crate) fn default_name(c: &Candidate, reading: Option<&LetterReading>) -> String {
    let a = &c.attribution;
    if let Some(r) = reading.filter(|r| !r.text.trim().is_empty()) {
        return r.text.trim().to_string();
    }
    if !a.title.trim().is_empty() {
        return a.title.trim().to_string();
    }
    let stem = a.file.rsplit_once('.').map_or(a.file.as_str(), |(s, _)| s);
    if stem.is_empty() { format!("{} logo", a.owner()) } else { stem.to_string() }
}

/// Keep the end of `s` (file names matter more than their folders).
fn ellipsize_left(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        return s.to_string();
    }
    let tail: String = s.chars().skip(len - n.saturating_sub(1)).collect();
    format!("…{tail}")
}

fn score_bar(s: f32) -> String {
    let n = (s * 5.0).round().clamp(0.0, 5.0) as usize;
    format!("{}{}", "▰".repeat(n), "▱".repeat(5 - n))
}

impl CandStage {
    pub fn new(source: String, arts: usize, items: Vec<Candidate>) -> Self {
        CandStage { source, arts, items, ..Default::default() }
    }

    pub fn current(&self) -> Option<&Candidate> {
        self.items.get(self.selected)
    }

    pub fn buttons(&self, sh: &Shared) -> Vec<Btn<'static, Do>> {
        if self.naming.is_some() {
            return vec![btn(Do::SaveNamed, "⏎", "save stencil").primary(), btn(Do::CancelName, "esc", "cancel")];
        }
        let any = !self.items.is_empty();
        let marked = self.marked.contains(&self.selected);
        vec![
            btn(Do::Cut, "⏎", "cut letters → font").primary().enabled(any),
            btn(Do::Stencil, "s", if self.marked.len() > 1 { "save marked as stencils" } else { "save as stencil" })
                .enabled(any),
            btn(Do::Mark, "␣", if marked { "unmark" } else { "mark" }).enabled(any),
            btn(Do::Read, "a", "Claude reads it").enabled(any && sh.agent.is_some()),
            btn(Do::Back, "esc", "find art"),
        ]
    }

    fn card_h(c: &Candidate) -> usize {
        c.h + 2
    }

    fn save(&mut self, name: &str, idx: &[usize], sh: &mut Shared) {
        let dir = sh.paths.stencils_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut lib = StencilLibrary::load(&dir);
        let mut names = vec![];
        for &i in idx {
            let c = &self.items[i];
            let text = if idx.len() == 1 { name.to_string() } else { default_name(c, sh.readings.get(&c.id)) };
            let rd = LetterReading { text, letters: vec![], style: String::new() };
            for st in harvest::build(&[(c.clone(), rd)]).stencils {
                match lib.save(&dir, st) {
                    Ok(m) => {
                        names.push(m.name);
                        self.saved.insert(c.id.clone());
                        sh.wrote = true;
                    }
                    Err(e) => return sh.say(format!("saving stencil: {e:#}"), Level::Error),
                }
            }
        }
        self.marked.clear();
        let who = idx.first().map(|&i| self.items[i].attribution.owner()).unwrap_or_default();
        match names.as_slice() {
            [one] => sh.say(format!("saved stencil \u{201c}{one}\u{201d} — credited to {who}"), Level::Ok),
            many => sh.say(format!("saved {} stencils (credits kept)", many.len()), Level::Ok),
        }
    }

    /// Keys that move the selection are handled here; the rest become buttons.
    pub fn key(&mut self, k: &KeyEvent, _sh: &mut Shared) -> Option<Do> {
        if let Some((input, _)) = &mut self.naming {
            return match k.code {
                KeyCode::Esc => Some(Do::CancelName),
                KeyCode::Enter => Some(Do::SaveNamed),
                _ => {
                    input.key(k);
                    None
                }
            };
        }
        let n = self.items.len();
        match k.code {
            KeyCode::Esc | KeyCode::Backspace => return Some(Do::Back),
            KeyCode::Enter | KeyCode::Char('f') | KeyCode::Char('c') => return Some(Do::Cut),
            KeyCode::Char(' ') => return Some(Do::Mark),
            KeyCode::Char('s') => return Some(Do::Stencil),
            KeyCode::Char('a') => return Some(Do::Read),
            KeyCode::Up if n > 0 => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if n > 0 => self.selected = (self.selected + 1).min(n - 1),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(4),
            KeyCode::PageDown if n > 0 => self.selected = (self.selected + 4).min(n - 1),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = n.saturating_sub(1),
            _ => {}
        }
        None
    }

    pub fn act(&mut self, d: Do, sh: &mut Shared) -> Go {
        let n = self.items.len();
        match d {
            Do::Back => return Go::Back,
            Do::Cut if n > 0 => return Go::Cut,
            Do::Mark if n > 0 => {
                if !self.marked.remove(&self.selected) {
                    self.marked.insert(self.selected);
                }
                self.selected = (self.selected + 1).min(n - 1);
            }
            Do::Stencil if n > 0 => {
                if self.marked.len() > 1 {
                    let idx: Vec<usize> = self.marked.iter().copied().collect();
                    self.save("", &idx, sh);
                } else {
                    let i = self.marked.iter().next().copied().unwrap_or(self.selected);
                    let c = &self.items[i];
                    let name = default_name(c, sh.readings.get(&c.id));
                    self.naming = Some((LineInput::new(&name), vec![i]));
                }
            }
            Do::Read if n > 0 => {
                let c = self.items[self.selected].clone();
                sh.read(&c);
            }
            Do::SaveNamed => {
                if let Some((input, idx)) = self.naming.take() {
                    let name = input.text.trim().to_string();
                    if !name.is_empty() {
                        self.save(&name, &idx, sh);
                    }
                }
            }
            Do::CancelName => self.naming = None,
            _ => {}
        }
        Go::Stay
    }

    /// A click selects a card, a click on the selected one cuts it, a click
    /// on the ◇ marks it.
    pub fn mouse(&mut self, m: MouseEvent) -> Option<Do> {
        match m.kind {
            MouseEventKind::Down(_) => {
                let &(r, i) = self
                    .hits
                    .iter()
                    .find(|(r, _)| m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom())?;
                if m.row == r.y && m.column <= r.x + 2 {
                    if !self.marked.remove(&i) {
                        self.marked.insert(i);
                    }
                    return None;
                }
                if i == self.selected {
                    return Some(Do::Cut);
                }
                self.selected = i;
            }
            MouseEventKind::ScrollDown => self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1)),
            MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
            _ => {}
        }
        None
    }

    pub fn paste(&mut self, s: &str) {
        if let Some((input, _)) = &mut self.naming {
            input.paste(s);
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, sh: &Shared, btns: &mut Buttons<Act>) {
        let dim = Style::new().fg(theme::DIM);
        let side_w = if area.width >= 110 { 34 } else { 0 };
        let cards_w = area.width.saturating_sub(side_w + if side_w > 0 { 2 } else { 0 });
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{} ", ellipsize_left(&self.source, (cards_w as usize).saturating_sub(34).max(12))),
                    Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "· {} files · {} candidates{}",
                        self.arts,
                        self.items.len(),
                        if self.marked.is_empty() {
                            String::new()
                        } else {
                            format!(" · {} marked", self.marked.len())
                        }
                    ),
                    dim,
                ),
            ])),
            Rect::new(area.x, area.y, cards_w, 1),
        );
        let list = Rect::new(
            area.x,
            area.y + 2,
            cards_w,
            area.height.saturating_sub(if self.naming.is_some() { 4 } else { 2 }),
        );
        // Scroll so the selected card is fully visible.
        let avail = list.height as usize;
        self.top = self.top.min(self.selected);
        while self.top < self.selected
            && self.items[self.top..=self.selected].iter().map(Self::card_h).sum::<usize>() > avail
        {
            self.top += 1;
        }
        self.hits.clear();
        let mut y = list.y;
        for i in self.top..self.items.len() {
            if y >= list.bottom() {
                break;
            }
            let c = &self.items[i];
            let sel = i == self.selected;
            let mark = if self.marked.contains(&i) { "◆" } else { "◇" };
            let mut flags = String::new();
            if self.done.contains(&c.id) {
                flags.push_str(" ✓font");
            }
            if self.saved.contains(&c.id) {
                flags.push_str(" ✓stencil");
            }
            if sh.reading.contains(&c.id) {
                flags.push_str(&format!(" {} reading", sh.spinner()));
            } else if let Some(r) = sh.readings.get(&c.id) {
                flags.push_str(&format!(" \u{201c}{}\u{201d}", r.text));
            }
            let head_style = if sel {
                Style::new().bg(theme::PANEL_HI).fg(theme::TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::TEXT)
            };
            let mut spans = vec![
                Span::styled(if sel { "▌" } else { " " }, Style::new().fg(theme::ACCENT)),
                Span::styled(
                    format!("{mark} "),
                    Style::new().fg(if self.marked.contains(&i) { theme::ACCENT } else { theme::BORDER }),
                ),
                Span::styled(score_bar(c.score), Style::new().fg(if c.score > 0.7 { theme::OK } else { theme::DIM })),
                Span::styled(format!(" {:<18}", ellipsize(&c.attribution.file, 18)), head_style),
                Span::styled(format!("{:>3}x{:<3}", c.w, c.h), dim),
            ];
            spans.push(Span::styled(flags, Style::new().fg(theme::OK)));
            if cards_w >= 70 {
                spans.push(Span::raw("  "));
                spans.extend(credit_spans(c).into_iter().take(4));
            }
            let hr = Rect::new(list.x, y, list.width, 1);
            if sel {
                f.buffer_mut().set_style(hr, Style::new().bg(theme::PANEL_HI));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), hr);
            if sel {
                let cut = btn(Act::Cand(Do::Cut), "⏎", "cut letters").primary();
                let x = hr.right().saturating_sub(cut.width());
                btns.draw(f.buffer_mut(), x, y, hr.right(), &cut);
            }
            let ph = (c.h as u16).min(list.bottom().saturating_sub(y + 1));
            let pr = Rect::new(list.x + 3, y + 1, list.width.saturating_sub(3), ph);
            if sel {
                for yy in 0..ph {
                    f.render_widget(
                        Paragraph::new(Span::styled("▌", Style::new().fg(theme::ACCENT))),
                        Rect::new(list.x, y + 1 + yy, 1, 1),
                    );
                }
            }
            draw_clip(f.buffer_mut(), pr, &c.clip, &Palette::default());
            self.hits.push((Rect::new(list.x, y, list.width, ph + 1), i));
            y += Self::card_h(c) as u16;
        }
        if let Some((input, _)) = &self.naming {
            input.render(f, Rect::new(area.x, area.bottom() - 1, cards_w.min(70), 1), "Stencil name › ", true);
        }
        if side_w > 0
            && let Some(c) = self.current()
        {
            let r = Rect::new(area.right() - side_w, area.y, side_w, area.height);
            self.draw_side(f, r, c, sh);
        }
    }

    fn draw_side(&self, f: &mut Frame, r: Rect, c: &Candidate, sh: &Shared) {
        let dim = Style::new().fg(theme::DIM);
        let text = Style::new().fg(theme::TEXT);
        let a = &c.attribution;
        let mut lines = vec![
            Line::from(Span::styled("CREDIT", dim.add_modifier(Modifier::BOLD))),
            Line::from(Span::styled(
                if a.title.is_empty() { "(untitled)".to_string() } else { format!("\u{201c}{}\u{201d}", a.title) },
                Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("by ", dim),
                Span::styled(if a.author.is_empty() { "unknown artist".into() } else { a.author.clone() }, text),
            ]),
        ];
        if !a.group.is_empty() {
            lines.push(Line::from(vec![Span::styled("of ", dim), Span::styled(a.group.clone(), text)]));
        }
        let loc = [a.pack.as_str(), a.file.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>();
        lines.push(Line::from(Span::styled(loc.join("/"), dim)));
        lines.push(Line::from(Span::styled(
            format!("{}x{} cells at {},{} · score {:.2}", c.w, c.h, c.x, c.y, c.score),
            dim,
        )));
        lines.push(Line::from(""));
        if let Some(rd) = sh.readings.get(&c.id) {
            lines.push(Line::from(vec![
                Span::styled("Claude reads ", dim),
                Span::styled(format!("\u{201c}{}\u{201d}", rd.text), Style::new().fg(theme::OK)),
                Span::styled(format!(" ({})", rd.style), dim),
            ]));
            lines.push(Line::from(""));
        }
        // The font this logo feeds: fonts grow logo by logo, per artist and style.
        let style = sh.readings.get(&c.id).map_or("logo", |r| r.style.as_str());
        let font = sh.font_for(&c.attribution, style);
        lines.push(Line::from(Span::styled("LETTERS GO INTO", dim.add_modifier(Modifier::BOLD))));
        lines.push(Line::from(match font {
            Some(f) => Span::styled(f.title(), Style::new().fg(theme::ACCENT2)),
            None => Span::styled(
                format!("a new font: {}", harvest::font_file_name(&c.attribution, style).trim_end_matches(".acidfont")),
                Style::new().fg(theme::ACCENT2),
            ),
        }));
        let mut cov = coverage_line(font.map(|f| &f.spec), "", 26);
        cov.spans.truncate(26);
        lines.push(cov);
        lines.push(Line::from(Span::styled(
            match font {
                Some(f) => format!(
                    "{} of 62 letters so far",
                    f.spec.glyphs.keys().filter(|c| c.is_ascii_alphanumeric()).count()
                ),
                None => "no letters yet".to_string(),
            },
            dim,
        )));
        if sh.agent.is_none() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "No API key — you select each letter and type what it is.",
                dim,
            )));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), r);
    }
}
