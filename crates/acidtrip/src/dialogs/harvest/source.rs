//! Stage 1: a source line (16colo.rs:pack, URL, zip, file, folder), the
//! sources loaded before, and a 16colo.rs browser: years → packs → load.
//! The browser fills itself when the studio opens.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use acidtrip_ai::harvest::{self, Candidate, PackDetail, PackInfo, YearInfo};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Act, Msg, Piece, Shared, ellipsize, wrap};
use crate::app::Level;
use crate::ui::widgets::{Btn, Buttons, LineInput, ListState, btn, list_line, theme};

/// A relative path that doesn't exist here is looked up in the parent
/// directories too (acidtrip started from a subfolder of a project).
fn resolve_local(source: &str) -> String {
    let p = std::path::Path::new(source);
    let looks_remote = source.contains("://") || source.starts_with("16colo.rs") || source.starts_with('~');
    if looks_remote || p.is_absolute() || p.exists() {
        return source.to_string();
    }
    let Ok(cwd) = std::env::current_dir() else { return source.to_string() };
    cwd.ancestors()
        .skip(1)
        .take(4)
        .map(|d| d.join(p))
        .find(|c| c.exists())
        .map_or_else(|| source.to_string(), |c| c.to_string_lossy().into_owned())
}

const DEBOUNCE: Duration = Duration::from_millis(350);
/// Candidates kept across a whole source.
const MAX_CANDIDATES: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Years,
    Packs,
}

/// Where a key sends the studio.
pub(crate) enum Go {
    Stay,
    Close,
    Candidates,
}

/// Buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Do {
    Load,
    Browse,
    LoadPack,
    Years,
    Connect,
    Recent(usize),
}

pub(crate) struct SourceStage {
    input: LineInput,
    focus: Focus,
    years: Option<Vec<YearInfo>>,
    years_list: ListState,
    years_loading: bool,
    years_err: Option<String>,
    packs: HashMap<u32, Vec<PackInfo>>,
    packs_loading: HashSet<u32>,
    /// Year whose packs are shown.
    shown_year: Option<u32>,
    packs_list: ListState,
    filter: String,
    filtered: Vec<usize>,
    details: HashMap<String, Result<PackDetail, String>>,
    detail_requested: HashSet<String>,
    want_detail: Option<(String, Instant)>,
    want_year: Option<(u32, Instant)>,
    /// Source being loaded.
    pub loading: Option<String>,
    years_area: Rect,
    packs_area: Rect,
    input_area: Rect,
}

impl SourceStage {
    pub fn new() -> Self {
        SourceStage {
            input: LineInput::new("16colo.rs:"),
            focus: Focus::Input,
            years: None,
            years_list: ListState::default(),
            years_loading: false,
            years_err: None,
            packs: HashMap::new(),
            packs_loading: HashSet::new(),
            shown_year: None,
            packs_list: ListState::default(),
            filter: String::new(),
            filtered: vec![],
            details: HashMap::new(),
            detail_requested: HashSet::new(),
            want_detail: None,
            want_year: None,
            loading: None,
            years_area: Rect::default(),
            packs_area: Rect::default(),
            input_area: Rect::default(),
        }
    }

    /// Fill the 16colo.rs browser (unless offline): years, then the
    /// selected year's packs.
    pub fn start(&mut self, sh: &mut Shared) {
        if !sh.offline {
            self.request_years(sh);
        }
    }

    pub fn buttons(&self, sh: &Shared) -> Vec<Btn<'static, Do>> {
        let loading = self.loading.is_some();
        let mut v = match self.focus {
            Focus::Input => vec![
                btn(Do::Load, "⏎", "load source").primary().enabled(!loading),
                btn(Do::Browse, "⇥", "browse 16colo.rs"),
            ],
            Focus::Years => vec![btn(Do::Browse, "⏎", "show packs").primary(), btn(Do::Load, "⇥", "source line")],
            Focus::Packs => vec![
                btn(Do::LoadPack, "⏎", "load pack").primary().enabled(!loading && self.current_pack().is_some()),
                btn(Do::Years, "←", "years"),
            ],
        };
        if self.years.is_none() && !self.years_loading {
            v.push(btn(Do::Connect, "", if sh.offline { "connect to 16colo.rs" } else { "↻ retry 16colo.rs" }));
        }
        v
    }

    pub fn act(&mut self, d: Do, sh: &mut Shared) {
        match d {
            Do::Load if self.focus == Focus::Input => self.load(self.input.text.clone(), sh),
            Do::Load => self.set_focus(Focus::Input, sh),
            Do::Browse if self.focus == Focus::Years => self.open_year(sh),
            Do::Browse => self.set_focus(Focus::Years, sh),
            Do::LoadPack => self.load_current_pack(sh),
            Do::Years => self.set_focus(Focus::Years, sh),
            Do::Connect => {
                sh.offline = false;
                self.years_err = None;
                self.request_years(sh);
            }
            Do::Recent(i) => {
                if let Some(src) = sh.recent.get(i).cloned() {
                    self.input = LineInput::new(&src);
                    self.focus = Focus::Input;
                    self.load(src, sh);
                }
            }
        }
    }

    /// Put `source` on the source line and load it.
    pub fn load_now(&mut self, source: &str, sh: &mut Shared) {
        self.input = LineInput::new(source);
        self.focus = Focus::Input;
        self.load(source.to_string(), sh);
    }

    fn open_year(&mut self, sh: &mut Shared) {
        if let Some(y) = self.current_year() {
            self.want_year = None;
            self.request_packs(y, sh);
            self.set_focus(Focus::Packs, sh);
        }
    }

    pub fn debouncing(&self) -> bool {
        self.want_detail.is_some() || self.want_year.is_some()
    }

    // ------------------------------------------------------------ jobs

    fn request_years(&mut self, sh: &mut Shared) {
        if self.years.is_some() || self.years_loading {
            return;
        }
        self.years_loading = true;
        sh.say("asking 16colo.rs for its years…", Level::Info);
        let cache = sh.cache.clone();
        sh.spawn(move |tx| {
            let _ = tx.send(Msg::Years(harvest::sixteen_colors_years(&cache).map_err(|e| format!("{e:#}"))));
        });
    }

    fn request_packs(&mut self, year: u32, sh: &mut Shared) {
        self.show_year(year);
        if self.packs.contains_key(&year) || !self.packs_loading.insert(year) {
            return;
        }
        sh.say(format!("listing {year} packs…"), Level::Info);
        let cache = sh.cache.clone();
        sh.spawn(move |tx| {
            let _ = tx.send(Msg::Packs(year, harvest::sixteen_colors_year(year, &cache).map_err(|e| format!("{e:#}"))));
        });
    }

    fn request_detail(&mut self, pack: String, sh: &mut Shared) {
        if !self.detail_requested.insert(pack.clone()) {
            return;
        }
        let cache = sh.cache.clone();
        sh.spawn(move |tx| {
            let r = harvest::sixteen_colors_pack(&pack, &cache).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Detail(pack, r));
        });
    }

    /// Fetch art and find candidates in the background.
    pub fn load(&mut self, source: String, sh: &mut Shared) {
        let source = source.trim().to_string();
        if source.is_empty() || source == "16colo.rs:" {
            sh.say("type a source, or pick a pack from 16colo.rs below", Level::Warn);
            return;
        }
        if self.loading.is_some() {
            return;
        }
        let source = resolve_local(&source);
        self.loading = Some(source.clone());
        sh.say(format!("loading {source}…"), Level::Info);
        let cache = sh.cache.clone();
        sh.spawn(move |tx| {
            let ptx = tx.clone();
            let r = harvest::fetch_with(&source, &cache, &mut |m| {
                let _ = ptx.send(Msg::Progress(m.to_string()));
            })
            .map(|arts| {
                let _ = ptx.send(Msg::Progress(format!("finding logos in {} files…", arts.len())));
                let mut all: Vec<Candidate> = arts.iter().flat_map(harvest::candidates).collect();
                all.sort_by(|a, b| b.score.total_cmp(&a.score));
                all.truncate(MAX_CANDIDATES);
                let pieces =
                    arts.into_iter().map(|a| Piece { grid: a.doc.flatten(), attribution: a.attribution }).collect();
                (pieces, all)
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Loaded(source, r));
        });
    }

    pub fn on_years(&mut self, r: Result<Vec<YearInfo>, String>, sh: &mut Shared) {
        self.years_loading = false;
        match r {
            Ok(y) => {
                let total: usize = y.iter().map(|y| y.packs).sum();
                sh.say(format!("16colo.rs: {} years, {total} packs", y.len()), Level::Ok);
                self.years_list.selected = y.iter().position(|y| y.year == 1996).unwrap_or(0);
                self.years = Some(y);
                // Show a year's packs right away: the browser is never empty.
                if let Some(y) = self.current_year() {
                    self.request_packs(y, sh);
                }
            }
            Err(e) => {
                sh.say(format!("16colo.rs: {e}"), Level::Error);
                self.years_err = Some(e);
            }
        }
    }

    pub fn on_packs(&mut self, year: u32, r: Result<Vec<PackInfo>, String>, sh: &mut Shared) {
        self.packs_loading.remove(&year);
        match r {
            Ok(p) => {
                sh.say(format!("{year}: {} packs", p.len()), Level::Ok);
                self.packs.insert(year, p);
                if self.shown_year == Some(year) {
                    self.refilter();
                }
            }
            Err(e) => sh.say(format!("{year}: {e}"), Level::Error),
        }
    }

    pub fn on_detail(&mut self, pack: String, r: Result<PackDetail, String>) {
        self.details.insert(pack, r);
    }

    // ------------------------------------------------------------ state

    fn show_year(&mut self, year: u32) {
        if self.shown_year != Some(year) {
            self.shown_year = Some(year);
            self.filter.clear();
            self.packs_list = ListState::default();
            self.refilter();
        }
    }

    fn year_packs(&self) -> &[PackInfo] {
        self.shown_year.and_then(|y| self.packs.get(&y)).map_or(&[], |v| v.as_slice())
    }

    fn refilter(&mut self) {
        let q = self.filter.to_lowercase();
        self.filtered = self
            .year_packs()
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                q.is_empty() || p.name.to_lowercase().contains(&q) || p.groups.iter().any(|g| g.contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        self.packs_list.selected = 0;
        self.packs_list.offset = 0;
    }

    fn current_pack(&self) -> Option<&PackInfo> {
        self.filtered.get(self.packs_list.selected).map(|&i| &self.year_packs()[i])
    }

    fn current_year(&self) -> Option<u32> {
        self.years.as_ref()?.get(self.years_list.selected).map(|y| y.year)
    }

    fn year_moved(&mut self) {
        if let Some(y) = self.current_year() {
            self.want_year = Some((y, Instant::now()));
        }
    }

    fn pack_moved(&mut self) {
        if let Some(p) = self.current_pack() {
            self.want_detail = Some((p.name.clone(), Instant::now()));
        }
    }

    /// Fire debounced requests whose delay has passed.
    fn tick(&mut self, sh: &mut Shared) {
        if let Some((y, t)) = self.want_year
            && t.elapsed() >= DEBOUNCE
        {
            self.want_year = None;
            self.request_packs(y, sh);
        }
        if let Some((p, t)) = self.want_detail.clone()
            && t.elapsed() >= DEBOUNCE
        {
            self.want_detail = None;
            self.request_detail(p, sh);
        }
    }

    fn set_focus(&mut self, f: Focus, sh: &mut Shared) {
        self.focus = f;
        if f != Focus::Input {
            self.request_years(sh);
        }
        if f == Focus::Packs
            && self.shown_year.is_none()
            && let Some(y) = self.current_year()
        {
            self.request_packs(y, sh);
        }
        if f == Focus::Packs {
            self.pack_moved();
        }
    }

    fn load_current_pack(&mut self, sh: &mut Shared) {
        if let Some(p) = self.current_pack() {
            let src = format!("16colo.rs:{}", p.name);
            self.input = LineInput::new(&src);
            self.load(src, sh);
        }
    }

    // ------------------------------------------------------------ input

    pub fn key(&mut self, k: &KeyEvent, sh: &mut Shared) -> Go {
        match (self.focus, k.code) {
            (Focus::Packs, KeyCode::Esc) if !self.filter.is_empty() => {
                self.filter.clear();
                self.refilter();
            }
            (_, KeyCode::Esc) => return Go::Close,
            (f, KeyCode::Tab) => {
                let next = match f {
                    Focus::Input => Focus::Years,
                    Focus::Years => Focus::Packs,
                    Focus::Packs => Focus::Input,
                };
                self.set_focus(next, sh);
            }
            (f, KeyCode::BackTab) => {
                let prev = match f {
                    Focus::Input => Focus::Packs,
                    Focus::Years => Focus::Input,
                    Focus::Packs => Focus::Years,
                };
                self.set_focus(prev, sh);
            }
            (Focus::Input, KeyCode::Enter) => self.load(self.input.text.clone(), sh),
            (Focus::Input, KeyCode::Down) => return Go::Candidates,
            (Focus::Input, _) => {
                self.input.key(k);
            }
            (Focus::Years, KeyCode::Enter | KeyCode::Right) => self.open_year(sh),
            (Focus::Years, _) => {
                let n = self.years.as_ref().map_or(0, Vec::len);
                if self.list_key(k, n, true) {
                    self.year_moved();
                }
            }
            (Focus::Packs, KeyCode::Enter) => self.load_current_pack(sh),
            (Focus::Packs, KeyCode::Left) => self.set_focus(Focus::Years, sh),
            (Focus::Packs, KeyCode::Backspace) => {
                self.filter.pop();
                self.refilter();
                self.pack_moved();
            }
            (Focus::Packs, KeyCode::Char(c)) if !c.is_control() => {
                self.filter.push(c);
                self.refilter();
                self.pack_moved();
            }
            (Focus::Packs, _) => {
                if self.list_key(k, self.filtered.len(), false) {
                    self.pack_moved();
                }
            }
        }
        Go::Stay
    }

    fn list_key(&mut self, k: &KeyEvent, len: usize, years: bool) -> bool {
        let l = if years { &mut self.years_list } else { &mut self.packs_list };
        match k.code {
            KeyCode::Home if len > 0 => l.selected = 0,
            KeyCode::End if len > 0 => l.selected = len - 1,
            _ => return l.key(k, len, 10),
        }
        true
    }

    pub fn mouse(&mut self, m: MouseEvent, sh: &mut Shared) {
        let inside = |r: Rect| m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom();
        match m.kind {
            MouseEventKind::Down(_) if inside(self.input_area) => self.set_focus(Focus::Input, sh),
            MouseEventKind::Down(_) if inside(self.years_area) => {
                let i = self.years_list.offset + (m.row - self.years_area.y) as usize;
                if i < self.years.as_ref().map_or(0, Vec::len) {
                    self.years_list.selected = i;
                    self.focus = Focus::Years;
                    if let Some(y) = self.current_year() {
                        self.request_packs(y, sh);
                    }
                } else {
                    self.set_focus(Focus::Years, sh);
                }
            }
            MouseEventKind::Down(_) if inside(self.packs_area) => {
                let i = self.packs_list.offset + (m.row - self.packs_area.y) as usize;
                if i < self.filtered.len() {
                    let again = self.focus == Focus::Packs && self.packs_list.selected == i;
                    self.packs_list.selected = i;
                    self.focus = Focus::Packs;
                    if again {
                        self.load_current_pack(sh);
                    } else {
                        self.pack_moved();
                    }
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = matches!(m.kind, MouseEventKind::ScrollDown);
                if inside(self.years_area) {
                    let n = self.years.as_ref().map_or(0, Vec::len);
                    if n > 0 {
                        let l = &mut self.years_list;
                        l.selected = if down { (l.selected + 1).min(n - 1) } else { l.selected.saturating_sub(1) };
                        self.year_moved();
                    }
                } else if inside(self.packs_area) && !self.filtered.is_empty() {
                    let n = self.filtered.len();
                    let l = &mut self.packs_list;
                    l.selected = if down { (l.selected + 1).min(n - 1) } else { l.selected.saturating_sub(1) };
                    self.pack_moved();
                }
            }
            _ => {}
        }
    }

    pub fn paste(&mut self, s: &str) {
        match self.focus {
            Focus::Input => self.input.paste(s.trim()),
            Focus::Packs => {
                self.filter.push_str(s.trim());
                self.refilter();
            }
            Focus::Years => {}
        }
    }

    // ------------------------------------------------------------ drawing

    pub fn draw(&mut self, f: &mut Frame, area: Rect, sh: &mut Shared, btns: &mut Buttons<Act>) {
        self.tick(sh);
        let dim = Style::new().fg(theme::DIM);
        let w = area.width;
        self.input_area = Rect::new(area.x, area.y, w, 1);
        let load = btn(Do::Load, "⏎", "load").primary().enabled(self.loading.is_none());
        let input_w = w.saturating_sub(load.width() + 1).min(90);
        self.input.render(f, Rect::new(area.x, area.y, input_w, 1), "Source › ", self.focus == Focus::Input);
        btns.draw(f.buffer_mut(), area.x + input_w + 1, area.y, area.right(), &wrap(load, Act::Source));
        let help = match &self.loading {
            Some(s) => Line::from(vec![
                Span::styled(format!("{} ", sh.spinner()), Style::new().fg(theme::ACCENT2)),
                Span::styled(format!("loading {s} — "), Style::new().fg(theme::TEXT)),
                Span::styled(sh.status.as_ref().map_or(String::new(), |s| s.0.clone()), dim),
            ]),
            None => Line::from(Span::styled(
                "16colo.rs:<pack> · an https:// URL to a file or .zip · a local .ans/.xb/.bin file, .zip or folder",
                dim,
            )),
        };
        f.render_widget(Paragraph::new(help), Rect::new(area.x, area.y + 1, w, 1));
        // Sources loaded before, one click away.
        let mut top = area.y + 3;
        if !sh.recent.is_empty() && area.height >= 12 {
            f.render_widget(
                Paragraph::new(Span::styled("Recent ", Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD))),
                Rect::new(area.x, area.y + 2, 7, 1),
            );
            let labels: Vec<String> = sh.recent.iter().map(|s| short_source(s)).collect();
            let chips: Vec<Btn<Act>> =
                labels.iter().enumerate().map(|(i, l)| btn(Act::Source(Do::Recent(i)), "", l.as_str())).collect();
            btns.row(f.buffer_mut(), Rect::new(area.x + 7, area.y + 2, w.saturating_sub(7), 1), &chips);
            top += 1;
        }
        if area.height < 8 {
            return;
        }
        let h = area.bottom().saturating_sub(top);
        let title = |f: &mut Frame, r: Rect, s: &str, on: bool| {
            let st = if on {
                Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::DIM).add_modifier(Modifier::BOLD)
            };
            f.render_widget(Paragraph::new(Span::styled(s.to_string(), st)), r);
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Browse 16colo.rs", Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD)),
                Span::styled("  — the scene's art archive: pick a year, then a pack", dim),
            ])),
            Rect::new(area.x, top, w, 1),
        );
        let lists_top = top + 2;
        let lh = h.saturating_sub(3);
        // Years column.
        let yw = 14u16;
        title(f, Rect::new(area.x, lists_top, yw, 1), "YEAR   PACKS", self.focus == Focus::Years);
        let yr = Rect::new(area.x, lists_top + 1, yw, lh);
        self.years_area = yr;
        match &self.years {
            Some(years) => {
                let range = self.years_list.visible(years.len(), yr.height as usize);
                let lines: Vec<Line> = range
                    .map(|i| {
                        let y = &years[i];
                        let sel = i == self.years_list.selected;
                        list_line(
                            y.year.to_string(),
                            y.packs.to_string(),
                            sel && (self.focus == Focus::Years || self.shown_year == Some(y.year)),
                            yr.width,
                        )
                    })
                    .collect();
                f.render_widget(Paragraph::new(lines), yr);
            }
            None => {
                let msg = if self.years_loading {
                    format!("{} loading…", sh.spinner())
                } else if self.years_err.is_some() {
                    "can't reach it".into()
                } else {
                    "not loaded".into()
                };
                f.render_widget(Paragraph::new(Span::styled(msg, dim)), yr);
            }
        }
        // Packs column.
        let px = area.x + yw + 2;
        let detail_w = if w >= 100 { (w - yw - 2) / 2 } else { 0 };
        let pw = w.saturating_sub(yw + 2 + detail_w + if detail_w > 0 { 2 } else { 0 });
        let head = if self.focus == Focus::Packs || !self.filter.is_empty() {
            format!("PACK · GROUP   filter: {}▏", self.filter)
        } else {
            "PACK · GROUP   (type to filter)".to_string()
        };
        title(f, Rect::new(px, lists_top, pw, 1), &head, self.focus == Focus::Packs);
        let pr = Rect::new(px, lists_top + 1, pw, lh);
        self.packs_area = pr;
        let year_loading = self.shown_year.is_some_and(|y| self.packs_loading.contains(&y));
        if self.filtered.is_empty() {
            let msg = if year_loading {
                format!("{} listing packs…", sh.spinner())
            } else if self.shown_year.is_some() {
                "no packs match".to_string()
            } else if self.years.is_none() {
                String::new()
            } else {
                "pick a year".to_string()
            };
            f.render_widget(Paragraph::new(Span::styled(msg, dim)), pr);
        } else {
            let range = self.packs_list.visible(self.filtered.len(), pr.height as usize);
            let packs = self.year_packs();
            let lines: Vec<Line> = range
                .map(|i| {
                    let p = &packs[self.filtered[i]];
                    list_line(
                        p.name.clone(),
                        ellipsize(&p.groups.join(", "), (pw as usize / 2).max(8)),
                        i == self.packs_list.selected && (self.focus == Focus::Packs || self.details_shown()),
                        pr.width,
                    )
                })
                .collect();
            f.render_widget(Paragraph::new(lines), pr);
        }
        // Pack details.
        if detail_w > 0 {
            let dr = Rect::new(px + pw + 2, lists_top, detail_w, lh + 1);
            self.draw_detail(f, dr, sh, btns);
        }
    }

    /// Details follow the pack list's selection once the packs are focused.
    fn details_shown(&self) -> bool {
        self.focus == Focus::Packs
    }

    fn draw_detail(&self, f: &mut Frame, r: Rect, sh: &Shared, btns: &mut Buttons<Act>) {
        let dim = Style::new().fg(theme::DIM);
        let pack = self.current_pack().filter(|_| self.details_shown());
        let Some(p) = pack else {
            let lines = vec![
                Line::from(Span::styled("How it works", Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled("1  find art: a pack, a file, a folder", dim)),
                Line::from(Span::styled("2  pick a logo among what's found in it", dim)),
                Line::from(Span::styled("3  lasso its letters, type each one", dim)),
                Line::from(Span::styled("   → a font in that artist's style", dim)),
                Line::from(Span::styled("★  My fonts: see and fix what you've cut", dim)),
                Line::from(""),
                Line::from(Span::styled(
                    if sh.agent.is_some() {
                        "Claude is on: it can read a logo's letters and draw the ones a font is missing."
                    } else {
                        "Everything works by hand. With an API key, Claude can also read letters and draw missing ones."
                    },
                    Style::new().fg(theme::TEXT),
                )),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), r);
            return;
        };
        let head = Line::from(vec![
            Span::styled(p.name.clone(), Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} · {}", p.year, p.groups.join(", ")), Style::new().fg(theme::TEXT)),
        ]);
        f.render_widget(Paragraph::new(head), Rect::new(r.x, r.y, r.width, 1));
        let load = btn(Act::Source(Do::LoadPack), "⏎", "load this pack").primary().enabled(self.loading.is_none());
        btns.draw(f.buffer_mut(), r.x, r.y + 1, r.right(), &load);
        let mut lines = vec![];
        match self.details.get(&p.name) {
            None => lines.push(Line::from(Span::styled(format!("{} reading pack…", sh.spinner()), dim))),
            Some(Err(e)) => lines.push(Line::from(Span::styled(e.clone(), Style::new().fg(theme::ERR)))),
            Some(Ok(d)) => {
                lines.push(Line::from(Span::styled(
                    format!("{} art files · {} files in the zip", d.art_files(), d.files.len()),
                    Style::new().fg(theme::TEXT),
                )));
                let artists = d.artists();
                if !artists.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("artists ", dim),
                        Span::styled(
                            ellipsize(&artists.join(", "), r.width as usize * 2 - 10),
                            Style::new().fg(theme::TEXT),
                        ),
                    ]));
                }
                lines.push(Line::from(""));
                for file in d.files.iter().filter(|f| harvest::is_art_file(&f.name)) {
                    let what = file.content.first().cloned().unwrap_or_default();
                    let by = file.artists.join(", ");
                    lines.push(Line::from(vec![
                        Span::styled(format!("{:<13}", ellipsize(&file.name, 13)), Style::new().fg(theme::TEXT)),
                        Span::styled(ellipsize(&format!("{what} — {by}"), r.width.saturating_sub(14) as usize), dim),
                    ]));
                }
            }
        }
        let rest = Rect::new(r.x, r.y + 3, r.width, r.height.saturating_sub(3));
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rest);
    }
}

/// A recent source as a chip: a pack name or a file name.
fn short_source(s: &str) -> String {
    let s = s.strip_prefix("16colo.rs:").unwrap_or(s);
    let name = s.trim_end_matches('/').rsplit('/').next().unwrap_or(s);
    ellipsize(name, 22)
}
