//! Sourcing studio: find scene art (16colo.rs browser, URL, zip, file,
//! folder), pick logo candidates, and cut their letters into fonts and
//! stencils by hand: any shape, any size, selected freely on the whole piece.
//! Claude can read the letters to prefill the cutter and draw the letters a
//! font is missing, but nothing needs an API key.
//!
//! Tabs: [`source`] → [`candidates`] → [`cutter`] (a single art file skips
//! straight to the cutter), plus [`my_fonts`] for
//! the fonts harvested so far. Every action is a button in the bar at the
//! bottom (its key is on it). Network and AI work runs on background threads
//! that report through [`Msg`].

mod candidates;
mod cutter;
mod my_fonts;
mod source;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use acidtrip_ai::agent::AgentConfig;
use acidtrip_ai::harvest::{
    self, Attribution, Candidate, FontSpec, HarvestedFont, LetterReading, PackDetail, PackInfo, YearInfo,
};
use acidtrip_core::Grid;
use acidtrip_io::fonts::FontLibrary;
use acidtrip_io::library::Paths;
use acidtrip_io::stencils::StencilLibrary;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::ui::widgets::{Btn, BtnKind, Buttons, btn, popup, theme};

use candidates::CandStage;
use cutter::CutStage;
use my_fonts::FontsStage;
use source::SourceStage;

pub(super) const NOTE: &str = "harvested work is for personal use — credit the artists";
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Sources remembered in `<state>/harvest-recent`.
const RECENT_MAX: usize = 8;

/// Put Claude's letters into the font at `file` on disk (it may have
/// changed while Claude worked: only drawn letters it lacks are added).
/// Returns the font's title and how many letters were added.
fn merge_drawn(file: &Path, drawn: &FontSpec) -> Result<(String, usize), String> {
    let mut font = HarvestedFont::load(file).map_err(|e| format!("{e:#}"))?;
    let mut new = FontSpec { generated: drawn.generated.clone(), ..Default::default() };
    for ch in &drawn.generated {
        if !font.spec.glyphs.contains_key(ch)
            && let Some(g) = drawn.glyphs.get(ch)
        {
            new.insert(*ch, g.clone(), drawn.bases.get(ch).copied());
        }
    }
    let n = new.glyphs.len();
    font.merge(&new, &Default::default());
    font.save().map_err(|e| format!("saving {}: {e:#}", font.title()))?;
    Ok((font.title(), n))
}

/// Letters Claude is still drawing after the studio closed.
pub struct StudioDrawing(Receiver<Msg>);

impl StudioDrawing {
    /// What finished since last asked: the font's title and how many
    /// letters (already saved), or why it failed. `None` once nothing is
    /// left to hear.
    pub fn poll(&self) -> Option<Vec<Result<(String, usize), String>>> {
        let mut done = vec![];
        loop {
            match self.0.try_recv() {
                Ok(Msg::Completed(_, r)) => done.push(r),
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => return Some(done),
                Err(mpsc::TryRecvError::Disconnected) => return (!done.is_empty()).then_some(done),
            }
        }
    }
}

/// Closing the studio while Claude draws asks once.
const CLOSE_WHILE_DRAWING: &str = "Claude is still drawing — its letters will be saved to the font; close again to leave";

/// Results from background jobs.
pub(crate) enum Msg {
    Progress(String),
    Years(Result<Vec<YearInfo>, String>),
    Packs(u32, Result<Vec<PackInfo>, String>),
    Detail(String, Result<PackDetail, String>),
    Loaded(String, Result<(Vec<Piece>, Vec<Candidate>), String>),
    Reading(String, Result<LetterReading, String>),
    /// Claude drew missing letters for the font at this path; they are
    /// already saved into it (the font's title and how many were added).
    Completed(PathBuf, Result<(String, usize), String>),
}

/// One art file of a loaded source, flattened.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub grid: Grid,
    pub attribution: Attribution,
}

/// State every stage uses: job channel, status line, paths, AI config.
pub(crate) struct Shared {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Background jobs still running.
    pending: usize,
    pub status: Option<(String, Level)>,
    pub paths: Paths,
    pub cache: PathBuf,
    pub agent: Option<AgentConfig>,
    /// Claude's readings by candidate id.
    pub readings: HashMap<String, LetterReading>,
    /// Candidate ids with a read in flight.
    pub reading: Vec<String>,
    /// Fonts with Claude drawing letters, and when it started.
    pub completing: Vec<(PathBuf, Instant)>,
    /// The harvested fonts on disk.
    pub fonts: Vec<HarvestedFont>,
    /// Sources loaded before, newest first.
    pub recent: Vec<String>,
    /// Don't reach for the network until asked (tests).
    pub offline: bool,
    started: Instant,
    /// Anything written to the libraries (reload on close).
    pub wrote: bool,
}

impl Shared {
    fn new(paths: Paths, agent: Option<AgentConfig>) -> Self {
        let (tx, rx) = mpsc::channel();
        let recent = std::fs::read_to_string(recent_file(&paths))
            .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect())
            .unwrap_or_default();
        Shared {
            tx,
            rx,
            pending: 0,
            status: None,
            cache: paths.state_dir.join("harvest-cache"),
            fonts: HarvestedFont::list(&paths.fonts_dir()),
            paths,
            agent,
            readings: HashMap::new(),
            reading: vec![],
            completing: vec![],
            recent,
            offline: std::env::var_os("ACIDTRIP_OFFLINE").is_some_and(|v| !v.is_empty()),
            started: Instant::now(),
            wrote: false,
        }
    }

    /// Run `job` on a thread; it must send exactly one final (non-Progress) message.
    pub fn spawn(&mut self, job: impl FnOnce(Sender<Msg>) + Send + 'static) {
        self.pending += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || job(tx));
    }

    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    pub fn say(&mut self, s: impl Into<String>, level: Level) {
        self.status = Some((s.into(), level));
    }

    pub fn spinner(&self) -> char {
        SPINNER[(self.started.elapsed().as_millis() / 90) as usize % SPINNER.len()]
    }

    /// Ask Claude to read a candidate's letters in the background.
    pub fn read(&mut self, cand: &Candidate) {
        let Some(cfg) = self.agent.clone() else {
            self.say("no API key — select each letter and type what it is yourself", Level::Warn);
            return;
        };
        if self.reading.contains(&cand.id) {
            return;
        }
        self.reading.push(cand.id.clone());
        self.say(format!("asking Claude to read {}…", cand.id), Level::Info);
        let c = cand.clone();
        self.spawn(move |tx| {
            let r = harvest::read_letters(&cfg, &c).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Reading(c.id, r));
        });
    }

    /// Ask Claude to draw the A-Z/0-9 letters `font` is missing, in its style.
    pub fn complete(&mut self, font: &HarvestedFont) {
        let Some(cfg) = self.agent.clone() else {
            self.say("drawing letters needs Claude: set ANTHROPIC_API_KEY or ai.api_key in config.toml", Level::Warn);
            return;
        };
        if self.completing.iter().any(|(p, _)| *p == font.file) {
            return;
        }
        let missing: String = font.missing(harvest::COMPLETE_CHARS).into_iter().collect();
        if missing.is_empty() {
            self.say(format!("{} already has A–Z and 0–9", font.title()), Level::Ok);
            return;
        }
        self.completing.push((font.file.clone(), Instant::now()));
        self.say(format!("Claude is drawing {} letters for {}…", missing.chars().count(), font.title()), Level::Info);
        let (file, spec) = (font.file.clone(), font.spec.clone());
        self.spawn(move |tx| {
            // Saved here, not when the dialog hears back: closing the studio
            // while Claude draws must not throw the letters away.
            let r = harvest::complete_font(&cfg, &spec, Some(&missing))
                .map_err(|e| format!("{e:#}"))
                .and_then(|drawn| merge_drawn(&file, &drawn));
            let _ = tx.send(Msg::Completed(file, r));
        });
    }

    /// Put Claude's letters into the font on disk and say so.
    #[cfg(test)]
    pub fn drawn(&mut self, file: &Path, r: Result<FontSpec, String>) {
        let r = r.and_then(|drawn| merge_drawn(file, &drawn));
        self.drawn_saved(r);
    }

    /// Claude's letters are in the font (or drawing them failed): say so.
    pub fn drawn_saved(&mut self, r: Result<(String, usize), String>) {
        match r {
            Ok((title, n)) if n > 0 => {
                self.wrote = true;
                self.say(format!("Claude drew {n} letters for {title} — shown in cyan"), Level::Ok);
            }
            Ok(_) => self.say("Claude didn't draw any letters inside the boxes", Level::Warn),
            Err(e) => self.say(format!("drawing letters: {e}"), Level::Error),
        }
        self.reload_fonts();
    }

    pub fn reload_fonts(&mut self) {
        self.fonts = HarvestedFont::list(&self.paths.fonts_dir());
    }

    /// The harvested font an artist's letters go into with `style`.
    pub fn font_for(&self, a: &Attribution, style: &str) -> Option<&HarvestedFont> {
        let name = harvest::font_file_name(a, style);
        self.fonts.iter().find(|f| f.file.file_name().is_some_and(|n| n.to_string_lossy() == name))
    }

    /// Remember a source that loaded.
    fn remember(&mut self, source: &str) {
        self.recent.retain(|s| s != source);
        self.recent.insert(0, source.to_string());
        self.recent.truncate(RECENT_MAX);
        let _ = std::fs::write(recent_file(&self.paths), self.recent.join("\n") + "\n");
    }
}

fn recent_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("harvest-recent")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stage {
    Source,
    Candidates,
    Cutter,
    Fonts,
}

/// Everything a button can do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Act {
    Tab(Stage),
    Close,
    Source(source::Do),
    Cand(candidates::Do),
    Cut(cutter::Do),
    Fonts(my_fonts::Do),
}

pub struct HarvestDialog {
    sh: Shared,
    stage: Stage,
    /// Where Esc on My fonts goes back to.
    before_fonts: Stage,
    source: SourceStage,
    cands: CandStage,
    /// The art files of the loaded source.
    pieces: Vec<Piece>,
    cut: Option<CutStage>,
    fonts: FontsStage,
    btns: Buttons<Act>,
}

impl HarvestDialog {
    pub fn new(app: &App) -> Self {
        let mut sh = Shared::new(app.paths.clone(), crate::cli_harvest::agent_config(&app.config));
        let mut source = SourceStage::new();
        source.start(&mut sh);
        HarvestDialog {
            sh,
            stage: Stage::Source,
            before_fonts: Stage::Source,
            source,
            cands: CandStage::default(),
            pieces: vec![],
            cut: None,
            fonts: FontsStage::default(),
            btns: Buttons::default(),
        }
    }

    /// Open on a source and start loading it (from the gallery).
    pub fn with_source(app: &App, source: &str) -> Self {
        let mut d = HarvestDialog::new(app);
        d.source.load_now(source, &mut d.sh);
        d
    }

    /// Open straight on My fonts (from the command palette).
    pub fn my_fonts(app: &App) -> Self {
        let mut d = HarvestDialog::new(app);
        d.go(Stage::Fonts);
        d
    }

    fn poll(&mut self) {
        while let Ok(m) = self.sh.rx.try_recv() {
            if !matches!(m, Msg::Progress(_)) {
                self.sh.pending = self.sh.pending.saturating_sub(1);
            }
            match m {
                Msg::Progress(s) => self.sh.status = Some((s, Level::Info)),
                Msg::Years(r) => self.source.on_years(r, &mut self.sh),
                Msg::Packs(y, r) => self.source.on_packs(y, r, &mut self.sh),
                Msg::Detail(p, r) => self.source.on_detail(p, r),
                Msg::Loaded(src, Ok((pieces, items))) => {
                    self.source.loading = None;
                    let (arts, n) = (pieces.len(), items.len());
                    if arts > 0 {
                        self.sh.remember(&src);
                    }
                    self.pieces = pieces;
                    self.cands = CandStage::new(src.clone(), arts, items);
                    if arts == 1 || (arts > 0 && n == 0) {
                        // One piece (a gallery piece, a file): cut it, no picking.
                        let piece = self.pieces[0].clone();
                        self.cut = Some(CutStage::new(piece, None, None, &self.sh));
                        self.stage = Stage::Cutter;
                        let why = if arts == 1 { String::new() } else { format!(" (no logos found in {arts} files)") };
                        self.sh.say(format!("{src}: lasso a letter, then type what it is{why}"), Level::Ok);
                    } else if n > 0 {
                        self.sh.say(format!("{src}: {arts} art files, {n} logo candidates"), Level::Ok);
                        self.stage = Stage::Candidates;
                    } else {
                        self.sh.say(format!("{src}: no art files in it"), Level::Warn);
                    }
                }
                Msg::Loaded(src, Err(e)) => {
                    self.source.loading = None;
                    self.sh.say(load_error(&src, &e), Level::Error);
                }
                Msg::Reading(id, r) => {
                    self.sh.reading.retain(|x| *x != id);
                    match r {
                        Ok(rd) if !rd.letters.is_empty() => {
                            self.sh.say(
                                format!("Claude reads {:?} ({}) — check its letters, then save", rd.text, rd.style),
                                Level::Ok,
                            );
                            self.sh.readings.insert(id.clone(), rd.clone());
                            match (&self.stage, &mut self.cut) {
                                (Stage::Cutter, Some(c)) if c.reads(&id) => c.apply_reading(&id, &rd),
                                (Stage::Candidates, _) if self.cands.current().is_some_and(|c| c.id == id) => {
                                    self.open_cutter();
                                }
                                _ => {}
                            }
                        }
                        Ok(_) => self.sh.say(format!("Claude sees no lettering in {id}"), Level::Warn),
                        Err(e) => self.sh.say(format!("reading {id}: {e}"), Level::Error),
                    }
                }
                Msg::Completed(file, r) => {
                    self.sh.completing.retain(|(p, _)| *p != file);
                    self.sh.drawn_saved(r);
                }
            }
        }
    }

    /// Cut the current logo: its whole piece, scrolled to it.
    fn open_cutter(&mut self) {
        let Some(c) = self.cands.current() else { return };
        let Some(piece) = self.pieces.iter().find(|p| p.attribution == c.attribution).cloned() else { return };
        let rd = self.sh.readings.get(&c.id).cloned();
        self.cut = Some(CutStage::new(piece, Some(c.clone()), rd.as_ref(), &self.sh));
        self.stage = Stage::Cutter;
    }

    fn available(&self, s: Stage) -> bool {
        match s {
            Stage::Source | Stage::Fonts => true,
            Stage::Candidates => !self.cands.items.is_empty(),
            Stage::Cutter => self.cut.is_some(),
        }
    }

    fn go(&mut self, s: Stage) {
        if !self.available(s) || s == self.stage {
            return;
        }
        if s == Stage::Fonts {
            self.before_fonts = self.stage;
            self.sh.reload_fonts();
            let file =
                self.cut.as_ref().and_then(|c| self.sh.font_for(&c.piece.attribution, c.style()).map(|f| f.file.clone()));
            self.fonts.show(file.as_deref(), &self.sh);
        }
        if self.stage == Stage::Cutter
            && let Some(c) = &self.cut
            && c.committed()
            && let Some(k) = &c.cand
        {
            self.cands.done.insert(k.id.clone());
        }
        if s == Stage::Cutter
            && let Some(c) = &mut self.cut
        {
            c.rearm();
        }
        self.stage = s;
    }

    fn close(&mut self) -> Outcome {
        if let Some(c) = &mut self.cut
            && c.warn_unsaved(&mut self.sh, "close again")
        {
            self.stage = Stage::Cutter;
            return Outcome::Keep;
        }
        if !self.sh.completing.is_empty()
            && !self.sh.status.as_ref().is_some_and(|(m, _)| m == CLOSE_WHILE_DRAWING)
        {
            self.sh.say(CLOSE_WHILE_DRAWING, Level::Warn);
            return Outcome::Keep;
        }
        // Claude still drawing: the app hears when its letters are in.
        let drawing = (!self.sh.completing.is_empty()).then(|| {
            let (tx, rx) = mpsc::channel();
            self.sh.tx = tx;
            std::mem::replace(&mut self.sh.rx, rx)
        });
        if !self.sh.wrote && drawing.is_none() {
            return Outcome::Close;
        }
        let wrote = self.sh.wrote;
        Outcome::Then(Box::new(move |app: &mut App| {
            if wrote {
                app.fonts = FontLibrary::load(Some(&app.paths.fonts_dir()));
                app.stencils = StencilLibrary::load(&app.paths.stencils_dir());
                app.flash("libraries reloaded — F fonts, N stencils", Level::Ok);
            }
            app.studio_drawing = drawing.map(StudioDrawing);
        }))
    }

    fn act(&mut self, a: Act) -> Outcome {
        match a {
            Act::Tab(s) => self.go(s),
            Act::Close => return self.close(),
            Act::Source(d) => self.source.act(d, &mut self.sh),
            Act::Cand(d) => match self.cands.act(d, &mut self.sh) {
                candidates::Go::Back => self.go(Stage::Source),
                candidates::Go::Cut => self.open_cutter(),
                candidates::Go::Stay => {}
            },
            Act::Cut(d) => {
                let go = self.cut.as_mut().map(|c| c.act(d, &mut self.sh));
                match go {
                    Some(cutter::Go::Back) => {
                        self.go(if self.available(Stage::Candidates) { Stage::Candidates } else { Stage::Source })
                    }
                    Some(cutter::Go::Fonts) => self.go(Stage::Fonts),
                    _ => {}
                }
            }
            Act::Fonts(d) => match self.fonts.act(d, &mut self.sh) {
                my_fonts::Go::Back => {
                    let back = if self.available(self.before_fonts) { self.before_fonts } else { Stage::Source };
                    self.go(back)
                }
                my_fonts::Go::Find => self.go(Stage::Source),
                my_fonts::Go::Use(file) => return self.use_font(file),
                my_fonts::Go::Stay => {}
            },
        }
        Outcome::Keep
    }

    /// Close and open the Font tool on a harvested font.
    fn use_font(&self, file: PathBuf) -> Outcome {
        Outcome::Then(Box::new(move |app: &mut App| {
            app.fonts = FontLibrary::load(Some(&app.paths.fonts_dir()));
            app.stencils = StencilLibrary::load(&app.paths.stencils_dir());
            let mut d = crate::dialogs::fonts::FontDialog::new(app);
            d.select_path(&file);
            app.dialogs.push(Box::new(d));
        }))
    }

    fn tabs(&mut self, f: &mut Frame, r: Rect) {
        let n_fonts = format!("My fonts {}", self.sh.fonts.len());
        let tabs = [
            (Stage::Source, "1", "Find art"),
            (Stage::Candidates, "2", "Pick a logo"),
            (Stage::Cutter, "3", "Cut letters"),
            (Stage::Fonts, "^F", n_fonts.as_str()),
        ];
        let buf = f.buffer_mut();
        let mut x = r.x;
        for (i, &(s, key, label)) in tabs.iter().enumerate() {
            if i == 3 {
                x += 2;
            } else if i > 0 {
                buf.set_string(x, r.y, "›", Style::new().fg(theme::BORDER));
                x += 2;
            }
            let kind = if s == self.stage {
                BtnKind::On
            } else if self.available(s) {
                BtnKind::Normal
            } else {
                BtnKind::Off
            };
            x += self.btns.draw(buf, x, r.y, r.right(), &btn(Act::Tab(s), key, label).kind(kind)) + 1;
        }
        // Status on the row under the tabs, where it has the whole width.
        let row = Rect::new(r.x, r.y + 1, r.width, 1);
        let status = match &self.sh.status {
            Some((msg, level)) => {
                let color = match level {
                    Level::Info => theme::TEXT,
                    Level::Ok => theme::OK,
                    Level::Warn => theme::WARN,
                    Level::Error => theme::ERR,
                };
                let lead = if self.sh.busy() { format!("{} ", self.sh.spinner()) } else { String::new() };
                Some((ellipsize(&format!("{lead}{msg}"), row.width as usize), color))
            }
            None if self.sh.busy() => Some((self.sh.spinner().to_string(), theme::ACCENT2)),
            None => None,
        };
        if let Some((text, color)) = status {
            f.render_widget(Paragraph::new(Span::styled(text, Style::new().fg(color))).right_aligned(), row);
        }
    }

    /// The action bar: this stage's buttons, then Close.
    fn bar_buttons(&self) -> Vec<Btn<'static, Act>> {
        let mut btns: Vec<Btn<Act>> = match self.stage {
            Stage::Source => self.source.buttons(&self.sh).into_iter().map(|b| wrap(b, Act::Source)).collect(),
            Stage::Candidates => self.cands.buttons(&self.sh).into_iter().map(|b| wrap(b, Act::Cand)).collect(),
            Stage::Cutter => self
                .cut
                .as_ref()
                .map_or(vec![], |c| c.buttons(&self.sh).into_iter().map(|b| wrap(b, Act::Cut)).collect()),
            Stage::Fonts => self.fonts.buttons(&self.sh).into_iter().map(|b| wrap(b, Act::Fonts)).collect(),
        };
        btns.push(btn(Act::Close, "^C", "close"));
        btns
    }
}

fn wrap<'a, D>(b: Btn<'a, D>, f: fn(D) -> Act) -> Btn<'a, Act> {
    Btn { id: f(b.id), key: b.key, label: b.label, kind: b.kind }
}

impl Dialog for HarvestDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        self.poll();
        self.btns.clear();
        let r = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
        let inner = popup(f, r, "Sourcing studio — fonts & stencils from scene art", NOTE);
        if inner.height < 4 || inner.width < 20 {
            return;
        }
        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);
        self.tabs(f, Rect::new(x, inner.y, w, 2));
        // The bar takes two rows when its buttons don't fit on one.
        let bar_btns = self.bar_buttons();
        let bar_h = Buttons::rows_needed(&bar_btns, w).min(if inner.height >= 16 { 2 } else { 1 });
        let bar = Rect::new(x, inner.bottom() - bar_h, w, bar_h);
        let body = Rect::new(x, inner.y + 2, w, inner.height.saturating_sub(3 + bar_h));
        match self.stage {
            Stage::Source => self.source.draw(f, body, &mut self.sh, &mut self.btns),
            Stage::Candidates => self.cands.draw(f, body, &self.sh, &mut self.btns),
            Stage::Cutter => {
                if let Some(c) = &mut self.cut {
                    c.draw(f, body, &self.sh, &mut self.btns)
                }
            }
            Stage::Fonts => self.fonts.draw(f, body, &self.sh, &mut self.btns),
        }
        self.btns.row(f.buffer_mut(), bar, &bar_btns);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        self.poll();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return self.close();
        }
        if ctrl && k.code == KeyCode::Char('f') {
            self.go(Stage::Fonts);
            return Outcome::Keep;
        }
        match self.stage {
            Stage::Source => match self.source.key(&k, &mut self.sh) {
                source::Go::Close => return self.close(),
                source::Go::Candidates => self.go(Stage::Candidates),
                source::Go::Stay => {}
            },
            Stage::Candidates => {
                if let Some(d) = self.cands.key(&k, &mut self.sh) {
                    return self.act(Act::Cand(d));
                }
            }
            Stage::Cutter => {
                if let Some(d) = self.cut.as_mut().and_then(|c| c.key(&k, &mut self.sh)) {
                    return self.act(Act::Cut(d));
                }
            }
            Stage::Fonts => {
                if let Some(d) = self.fonts.key(&k, &mut self.sh) {
                    return self.act(Act::Fonts(d));
                }
            }
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        self.poll();
        if let Some(a) = self.btns.mouse(&m) {
            return self.act(a);
        }
        if matches!(m.kind, MouseEventKind::Moved) {
            return Outcome::Keep;
        }
        match self.stage {
            Stage::Source => self.source.mouse(m, &mut self.sh),
            Stage::Candidates => {
                if let Some(d) = self.cands.mouse(m) {
                    return self.act(Act::Cand(d));
                }
            }
            Stage::Cutter => {
                if let Some(c) = &mut self.cut {
                    c.mouse(m)
                }
            }
            Stage::Fonts => self.fonts.mouse(m, &self.sh),
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        match self.stage {
            Stage::Source => self.source.paste(s),
            Stage::Candidates => self.cands.paste(s),
            Stage::Cutter => {
                if let Some(c) = &mut self.cut {
                    c.paste(s)
                }
            }
            Stage::Fonts => self.fonts.paste(s),
        }
    }

    fn animating(&self) -> bool {
        self.sh.busy() || self.source.debouncing()
    }
}

/// One coverage cell per char of `chars`: what a font has (✓ cut from art,
/// ✦ drawn by Claude), what this cut adds, and what's still missing.
pub(crate) fn coverage_spans(font: Option<&FontSpec>, adding: &str, chars: &str) -> Vec<Span<'static>> {
    chars
        .chars()
        .map(|c| {
            let has = font.is_some_and(|f| f.glyphs.contains_key(&c));
            let drawn = font.is_some_and(|f| f.generated.contains(&c));
            let st = if adding.contains(c) {
                Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD)
            } else if drawn {
                Style::new().fg(theme::ACCENT2)
            } else if has {
                Style::new().fg(theme::OK).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::BORDER)
            };
            Span::styled(c.to_string(), st)
        })
        .collect()
}

/// "A-Z 0-9" coverage with a count, fitted to `width`.
pub(crate) fn coverage_line(font: Option<&FontSpec>, adding: &str, width: u16) -> Line<'static> {
    let have = font.map_or(0, |f| harvest::ALPHABET.chars().filter(|c| f.glyphs.contains_key(c)).count());
    let mut spans = coverage_spans(font, adding, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    if width >= 60 {
        spans.push(Span::raw(" "));
        spans.extend(coverage_spans(font, adding, "0123456789"));
    }
    spans.push(Span::styled(format!("  {have}/{}", harvest::ALPHABET.chars().count()), Style::new().fg(theme::DIM)));
    Line::from(spans)
}

/// Clip `s` to `n` chars with an ellipsis.
pub(crate) fn ellipsize(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// "Title" by Artist / Group — the credit line shown for a candidate.
pub(crate) fn credit_spans(c: &Candidate) -> Vec<Span<'static>> {
    credit_spans_of(&c.attribution)
}

pub(crate) fn credit_spans_of(a: &harvest::Attribution) -> Vec<Span<'static>> {
    let mut v = vec![];
    if !a.title.is_empty() {
        v.push(Span::styled(format!("\u{201c}{}\u{201d} ", a.title), Style::new().fg(theme::ACCENT2)));
    }
    v.push(Span::styled("by ", Style::new().fg(theme::DIM)));
    v.push(Span::styled(
        if a.author.is_empty() { "unknown artist".to_string() } else { a.author.clone() },
        Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD),
    ));
    if !a.group.is_empty() {
        v.push(Span::styled(format!(" / {}", a.group), Style::new().fg(theme::TEXT)));
    }
    let loc = [a.pack.as_str(), a.file.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>();
    if !loc.is_empty() {
        v.push(Span::styled(format!("  {}", loc.join("/")), Style::new().fg(theme::DIM)));
    }
    v
}

/// A source that didn't load: its name, once (the error may start with it).
fn load_error(src: &str, e: &str) -> String {
    if e.starts_with(src) { e.to_string() } else { format!("{src}: {e}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_ai::harvest::{Attribution, SourceArt};
    use acidtrip_core::{Cell, Color, DocKind, Document, Grid};

    /// "AB" as two 5x5 blocks with a 1-column gap and a bar after them,
    /// credited to Tester / ACiD: the piece, and the logo found in it.
    fn piece() -> (Piece, Candidate) {
        let mut g = Grid::new(40, 12);
        for (i, fg) in [12u8, 14].into_iter().enumerate() {
            for y in 2..7 {
                for x in 4 + i * 6..9 + i * 6 {
                    g.set(x, y, Cell::new('█', Color::Pal(fg), Color::BLACK));
                }
            }
        }
        for x in 16..24 {
            g.set(x, 4, Cell::new('▓', Color::Pal(10), Color::Pal(2)));
        }
        let art = SourceArt {
            doc: Document::from_grid(DocKind::Classic, &g),
            attribution: Attribution { author: "Tester".into(), group: "ACiD".into(), ..Default::default() },
        };
        let cand = harvest::candidates(&art).remove(0);
        (Piece { grid: g, attribution: art.attribution }, cand)
    }

    fn shared(dir: &std::path::Path) -> Shared {
        let paths = Paths { config_dir: dir.join("config"), data_dir: dir.join("data"), state_dir: dir.join("state") };
        Shared::new(paths, None)
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    /// A key in the cutter, doing what its button does.
    fn press(cut: &mut CutStage, sh: &mut Shared, c: KeyCode) {
        if let Some(d) = cut.key(&key(c), sh) {
            cut.act(d, sh);
        }
    }

    /// Select the shape at each (x, y) and type its letter.
    fn cut_letters(cut: &mut CutStage, sh: &mut Shared, letters: &[(usize, usize, char)]) {
        for &(x, y, ch) in letters {
            cut.pick(x, y);
            press(cut, sh, KeyCode::Char(ch));
        }
    }

    #[test]
    fn one_art_file_opens_straight_in_the_cutter() {
        let dir = tempfile::tempdir().unwrap();
        let sh = shared(dir.path());
        let (piece, cand) = piece();
        let mut d = HarvestDialog {
            sh,
            stage: Stage::Source,
            before_fonts: Stage::Source,
            source: SourceStage::new(),
            cands: CandStage::default(),
            pieces: vec![],
            cut: None,
            fonts: FontsStage::default(),
            btns: Buttons::default(),
        };
        d.sh.pending = 1;
        d.sh.tx.send(Msg::Loaded("gallery.ans".into(), Ok((vec![piece.clone()], vec![cand.clone()])))).unwrap();
        d.poll();
        assert_eq!(d.stage, Stage::Cutter, "{:?}", d.sh.status);
        let cut = d.cut.as_ref().unwrap();
        assert!(cut.cand.is_none());
        assert_eq!(cut.piece.grid, piece.grid, "the whole piece, not a logo cut out of it");
        // Its logo is still there to pick, and picking it cuts the same piece.
        d.go(Stage::Candidates);
        assert_eq!(d.stage, Stage::Candidates);
        d.open_cutter();
        assert_eq!(d.cut.as_ref().unwrap().cand.as_ref().map(|c| &c.id), Some(&cand.id));
        // Several files: pick a logo first.
        d.sh.pending = 1;
        d.sh.tx.send(Msg::Loaded("pack.zip".into(), Ok((vec![piece.clone(), piece], vec![cand])))).unwrap();
        d.poll();
        assert_eq!(d.stage, Stage::Candidates);
    }

    #[test]
    fn load_errors_name_the_source_once() {
        assert_eq!(load_error("/x.ans", "/x.ans: no such file"), "/x.ans: no such file");
        assert_eq!(load_error("pack.zip", "bad zip"), "pack.zip: bad zip");
    }

    #[test]
    fn leaving_with_unsaved_letters_warns_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut sh = shared(dir.path());
        let (piece, cand) = piece();
        let mut cut = CutStage::new(piece, Some(cand), None, &sh);
        // Nothing cut: Esc leaves at once.
        assert!(matches!(cut.act(cutter::Do::Back, &mut sh), cutter::Go::Back));
        cut_letters(&mut cut, &mut sh, &[(5, 3, 'a')]);
        // A second "a" replaces the first, and says so.
        cut_letters(&mut cut, &mut sh, &[(11, 3, 'a')]);
        assert!(sh.status.as_ref().is_some_and(|(s, _)| s.contains("replaces")), "{:?}", sh.status);
        assert_eq!(cut.unsaved(), 1);
        // Esc warns and stays; Esc again leaves.
        assert!(matches!(cut.act(cutter::Do::Back, &mut sh), cutter::Go::Stay));
        assert!(sh.status.as_ref().is_some_and(|(s, _)| s.contains("1 letter not saved")), "{:?}", sh.status);
        assert!(matches!(cut.act(cutter::Do::Back, &mut sh), cutter::Go::Back));
        // A new letter warns again; saving clears it.
        cut_letters(&mut cut, &mut sh, &[(17, 4, 'z')]);
        assert_eq!(cut.unsaved(), 2);
        press(&mut cut, &mut sh, KeyCode::Enter);
        assert_eq!(cut.unsaved(), 0);
        assert!(matches!(cut.act(cutter::Do::Back, &mut sh), cutter::Go::Back));
    }

    #[test]
    fn cutter_cuts_a_font_by_hand() {
        let dir = tempfile::tempdir().unwrap();
        let mut sh = shared(dir.path());
        let (piece, cand) = piece();
        let mut cut = CutStage::new(piece.clone(), Some(cand), None, &sh);
        // Keys don't type letters until something is selected.
        press(&mut cut, &mut sh, KeyCode::Char('q'));
        cut_letters(&mut cut, &mut sh, &[(5, 3, 'a'), (11, 3, 'b'), (17, 4, 'z')]);
        press(&mut cut, &mut sh, KeyCode::Enter);
        assert!(cut.committed(), "{:?}", sh.status);
        assert!(sh.wrote);
        assert_eq!(sh.fonts.len(), 1, "the studio's font list is fresh");
        assert!(sh.fonts[0].file.to_string_lossy().ends_with("tester-logo.acidfont"));
        let fonts = FontLibrary::load(Some(&sh.paths.fonts_dir()));
        let f = fonts.list().into_iter().find(|f| f.name.starts_with("Tester")).expect("harvested font");
        assert!(f.charset.contains("ABZabz"), "{}", f.charset);
        // The bar is 8 wide and 1 tall: no format says a letter can't be.
        let clip = fonts.render(&f.id, "zab", &Default::default()).unwrap();
        assert_eq!(clip.width, 8 + 1 + 5 + 1 + 5);
        assert_eq!(clip.height, 5);
        assert_eq!(StencilLibrary::load(&sh.paths.stencils_dir()).search("abz").len(), 1);
        // Recutting a letter and saving again updates the font but doesn't
        // duplicate the stencil.
        cut_letters(&mut cut, &mut sh, &[(11, 3, 'c')]);
        press(&mut cut, &mut sh, KeyCode::Enter);
        assert_eq!(StencilLibrary::load(&sh.paths.stencils_dir()).search("abz").len(), 1);
        assert!(sh.fonts[0].spec.glyphs.contains_key(&'c'));
        // The next piece by the same artist goes into the same font.
        let next = CutStage::new(piece, None, None, &sh);
        assert_eq!(next.style(), "logo");
        assert!(sh.font_for(&next.piece.attribution, next.style()).is_some());
    }

    #[test]
    fn cutter_prefills_from_a_reading() {
        let sh = shared(tempfile::tempdir().unwrap().path());
        let (piece, cand) = piece();
        let rd = LetterReading {
            text: "A B".into(),
            style: "Blocky".into(),
            letters: vec![
                harvest::LetterSpan { ch: 'A', x0: 0, x1: 4 },
                harvest::LetterSpan { ch: 'B', x0: 6, x1: 10 },
            ],
        };
        let cut = CutStage::new(piece, Some(cand), Some(&rd), &sh);
        assert_eq!(cut.style(), "Blocky");
        let save = cut.buttons(&sh).into_iter().find(|b| b.label == "save font").unwrap();
        assert_ne!(save.kind, BtnKind::Off, "Claude's letters are ready to save");
    }

    #[test]
    fn claudes_letters_are_saved_even_with_the_studio_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut sh = shared(dir.path());
        let mut cut = CutStage::new(piece().0, None, None, &sh);
        cut_letters(&mut cut, &mut sh, &[(5, 3, 'a')]);
        press(&mut cut, &mut sh, KeyCode::Enter);
        let font = sh.fonts[0].clone();
        let mut drawn = font.spec.clone();
        let g = drawn.glyphs[&'A'].clone();
        drawn.glyphs.insert('Q', g);
        drawn.generated.insert('Q');
        // What the worker thread does, with no dialog to hear back.
        let (_, n) = merge_drawn(&font.file, &drawn).unwrap();
        assert_eq!(n, 1);
        let on_disk = HarvestedFont::load(&font.file).unwrap();
        assert!(on_disk.spec.glyphs.contains_key(&'Q'));
    }

    #[test]
    fn claudes_letters_join_the_font_without_replacing_cut_ones() {
        let dir = tempfile::tempdir().unwrap();
        let mut sh = shared(dir.path());
        let mut cut = CutStage::new(piece().0, None, None, &sh);
        cut_letters(&mut cut, &mut sh, &[(5, 3, 'a'), (11, 3, 'b')]);
        press(&mut cut, &mut sh, KeyCode::Enter);
        let font = sh.fonts[0].clone();
        // Claude drew C, and (wrongly) an A over the real one.
        let mut drawn = font.spec.clone();
        let g = drawn.glyphs[&'B'].clone();
        for ch in ['C', 'A'] {
            drawn.glyphs.insert(ch, g.clone());
            drawn.generated.insert(ch);
        }
        sh.drawn(&font.file, Ok(drawn));
        let after = &sh.fonts[0];
        assert_eq!(after.spec.glyphs[&'A'], font.spec.glyphs[&'A'], "cut letters stay");
        assert!(after.spec.glyphs.contains_key(&'c'), "other case filled in");
        assert_eq!(after.spec.generated.iter().collect::<String>(), "Cc");
        assert!(matches!(&sh.status, Some((m, Level::Ok)) if m.contains("drew 1 letters")), "{:?}", sh.status);
    }

    #[test]
    fn my_fonts_deletes_a_letter_and_the_font_after_confirming() {
        let dir = tempfile::tempdir().unwrap();
        let mut sh = shared(dir.path());
        let mut cut = CutStage::new(piece().0, None, None, &sh);
        cut_letters(&mut cut, &mut sh, &[(5, 3, 'a'), (11, 3, 'b')]);
        press(&mut cut, &mut sh, KeyCode::Enter);
        let mut st = FontsStage::default();
        st.show(None, &sh);
        let first = sh.fonts[0].spec.glyphs.keys().next().copied().unwrap();
        st.act(my_fonts::Do::DeleteLetter, &mut sh);
        assert!(!sh.fonts[0].spec.glyphs.contains_key(&first));
        st.act(my_fonts::Do::Undo, &mut sh);
        assert!(sh.fonts[0].spec.glyphs.contains_key(&first), "undo puts it back");
        st.act(my_fonts::Do::DeleteLetter, &mut sh);
        st.act(my_fonts::Do::DeleteFont, &mut sh);
        assert_eq!(sh.fonts.len(), 1, "the first click only arms it");
        st.act(my_fonts::Do::DeleteFont, &mut sh);
        assert!(sh.fonts.is_empty());
        assert!(matches!(st.act(my_fonts::Do::Use, &mut sh), my_fonts::Go::Stay));
    }
}
