//! Gallery: browse scene art like a streaming service. The home screen is
//! rows of cards (recently opened, your collection, local folders, famous
//! groups, then every year on 16colo.rs); a card opens a pack, a group, an
//! artist or a folder as a grid, and a piece opens full screen. From there
//! it opens as a document, goes to the sourcing studio, or is saved to your
//! collection (a local copy that works offline).
//!
//! Everything loads in the background, newest request first, so what's on
//! screen fills in before what scrolled away. Posters are half-block
//! thumbnails, redrawn as real pixels on terminals with graphics.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use acidtrip_ai::gallery::{self, Artist, FEATURED_GROUPS, Group, Origin, Piece, Shelf};
use acidtrip_ai::harvest::PackInfo;
use acidtrip_core::model::conform_cell;
use acidtrip_core::{Cell, Clip, Color, DocMeta, Document, Grid, Palette};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::tab::Tab;
use crate::tools_ctl::FloatSource;
use crate::ui::canvas::rgb;
use crate::ui::minimap;
use crate::ui::widgets::{Btn, Buttons, LineInput, btn, popup, theme};

const CARD_W: u16 = 21;
const POSTER_H: u16 = 8;
/// Poster plus two caption lines.
const CARD_H: u16 = POSTER_H + 2;
const GAP: u16 = 2;
/// Background jobs at once.
const WORKERS: usize = 6;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Modem playback: 14400 baud ≈ 1440 bytes, about 1200 cells a second.
const PLAY_CELLS_PER_SEC: f32 = 1200.0;

/// What a row's cards come from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Feed {
    Recent,
    Saved,
    Folders,
    Featured,
    /// The years on 16colo.rs (the home screen grows a row per year).
    Years,
    Year(u32),
    Pack(String),
    Group(String),
    Artist(String),
    Folder(PathBuf),
    SearchPacks(String),
    SearchGroups(String),
    SearchArtists(String),
}

impl Feed {
    /// Comes from the network.
    fn remote(&self) -> bool {
        !matches!(self, Feed::Recent | Feed::Saved | Feed::Folders | Feed::Featured | Feed::Folder(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Card {
    Pack(PackInfo),
    Piece(Piece),
    Group(Group),
    Artist(Artist),
    Folder(PathBuf),
    AddFolder,
}

#[derive(Clone, Debug, PartialEq)]
enum Load {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

struct Row {
    title: String,
    feed: Feed,
    cards: Vec<Card>,
    load: Load,
    /// First card shown (rows scroll sideways).
    scroll: usize,
    /// Shown when there are no cards.
    empty: &'static str,
}

impl Row {
    fn new(title: impl Into<String>, feed: Feed, empty: &'static str) -> Row {
        Row { title: title.into(), feed, cards: vec![], load: Load::Idle, scroll: 0, empty }
    }
}

struct Page {
    title: String,
    rows: Vec<Row>,
    /// One row of cards wrapping into a grid (packs, groups, artists, folders).
    grid: bool,
    /// Selected row and card.
    sel: (usize, usize),
    /// First row (or grid line) shown.
    top: usize,
}

impl Page {
    fn grid(title: impl Into<String>, feed: Feed) -> Page {
        Page { title: title.into(), rows: vec![Row::new("", feed, "nothing here")], grid: true, sel: (0, 0), top: 0 }
    }

    fn card(&self) -> Option<&Card> {
        self.rows.get(self.sel.0)?.cards.get(self.sel.1)
    }
}

/// A loaded piece.
struct Art {
    doc: Document,
    grid: Grid,
    bytes: Vec<u8>,
}

enum Job {
    Feed(Feed),
    Art(Piece),
    PackPoster(String),
    GroupPoster(String),
}

enum Msg {
    Feed(Feed, Result<Vec<Card>, String>),
    Art(String, Result<Box<Art>, String>),
    PackPoster(String, Result<Piece, String>),
    GroupPoster(String, Result<Piece, String>),
}

/// The full-screen view of one piece.
struct Viewer {
    piece: Piece,
    top: usize,
    /// Modem playback started at.
    playing: Option<Instant>,
    /// Taking a part: drag a box over the art.
    take: Option<Take>,
}

/// What to do when taking a part.
const TAKE_HELP: &str = "drag a box, or arrows size one (shift moves it) · ⏎ takes it";

/// A box being dragged over the art, in art cells.
#[derive(Clone, Copy, Default)]
struct Take {
    anchor: Option<(usize, usize)>,
    head: (usize, usize),
}

impl Take {
    /// (x0, y0, x1, y1), inclusive.
    fn span(&self) -> Option<(usize, usize, usize, usize)> {
        let (ax, ay) = self.anchor?;
        let (hx, hy) = self.head;
        Some((ax.min(hx), ay.min(hy), ax.max(hx), ay.max(hy)))
    }
}

/// Cells `x0..=x1`, `y0..=y1` of a piece as a clip for a document with
/// `meta`: colors keep their palette index when the palettes match, and
/// otherwise go through RGB to what the document holds.
pub fn part_clip(g: &Grid, pal: &Palette, span: (usize, usize, usize, usize), meta: &DocMeta) -> Clip {
    let (x0, y0, x1, y1) = span;
    let (x1, y1) = (x1.min(g.width.saturating_sub(1)), y1.min(g.height.saturating_sub(1)));
    let (w, h) = (x1 + 1 - x0.min(x1), y1 + 1 - y0.min(y1));
    let same = meta.palette == *pal;
    let color = |c: Color| if same { c } else { let [r, g, b] = c.rgb(pal); Color::Rgb(r, g, b) };
    let mut clip = Clip::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let c = g.get(x0 + x, y0 + y);
            let ch = if c.ch.is_control() || c.ch == '\0' { ' ' } else { c.ch };
            let cell = Cell { ch, fg: color(c.fg), bg: color(c.bg) };
            clip.cells[y * w + x] = Some(conform_cell(meta, cell));
        }
    }
    clip
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    Open,
    Back,
    Studio,
    Keep,
    Search,
    AddFolder,
    Connect,
    Play,
    /// Take a part of the piece (drag a box) into your art.
    Take,
    Close,
    /// Scroll a row sideways (row, by).
    Scroll(usize, i32),
}

pub struct GalleryDialog {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Jobs waiting, the newest last (taken first).
    queue: Vec<Job>,
    running: usize,
    asked: HashSet<String>,
    cache: PathBuf,
    shelf: Shelf,
    shelf_path: PathBuf,
    collection: PathBuf,
    offline: bool,
    pages: Vec<Page>,
    viewer: Option<Viewer>,
    search: LineInput,
    search_focus: bool,
    folder: Option<LineInput>,
    arts: HashMap<String, Result<Box<Art>, String>>,
    /// A pack's poster piece (its first art file).
    pack_posters: HashMap<String, Result<Piece, String>>,
    /// A group's poster: the best piece of its newest pack.
    group_posters: HashMap<String, Result<Piece, String>>,
    status: Option<(String, Level)>,
    started: Instant,
    btns: Buttons<Act>,
    /// Cards on screen: (rect, row, card).
    hits: Vec<(Rect, usize, usize)>,
    /// Posters drawn this frame: (rect, art key) for the pixel pass.
    posters: Vec<(Rect, String)>,
    search_area: Rect,
    /// Cards per line in the last frame (grid Up/Down steps by it).
    per_row: usize,
    /// Where the viewer drew the art (for taking a part with the mouse).
    art_area: Rect,
    /// A source to open in the sourcing studio (the Library switches tabs).
    pub studio_request: Option<String>,
}

fn key_hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn ellipsize(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
    t.push('…');
    t
}

impl GalleryDialog {
    pub fn new(app: &App) -> Self {
        let (tx, rx) = mpsc::channel();
        let shelf_path = app.paths.state_dir.join("gallery.json");
        let mut d = GalleryDialog {
            tx,
            rx,
            queue: vec![],
            running: 0,
            asked: HashSet::new(),
            cache: app.paths.state_dir.join("harvest-cache"),
            shelf: Shelf::load(&shelf_path),
            shelf_path,
            collection: app.paths.data_dir.join("library").join("collection"),
            offline: std::env::var_os("ACIDTRIP_OFFLINE").is_some_and(|v| !v.is_empty()),
            pages: vec![],
            viewer: None,
            search: LineInput::default(),
            search_focus: false,
            folder: None,
            arts: HashMap::new(),
            pack_posters: HashMap::new(),
            group_posters: HashMap::new(),
            status: None,
            started: Instant::now(),
            btns: Buttons::default(),
            hits: vec![],
            posters: vec![],
            search_area: Rect::default(),
            per_row: 4,
            art_area: Rect::default(),
            studio_request: None,
        };
        d.pages.push(d.home());
        d
    }

    /// Open straight on one piece (from the sidebar); `take` starts taking
    /// a part of it.
    pub fn viewing(app: &App, piece: Piece, take: bool) -> Self {
        let mut d = GalleryDialog::new(app);
        d.shelf.opened(&piece);
        d.refresh_shelf_rows();
        d.want_art(&piece);
        let take = take.then(Take::default);
        d.viewer = Some(Viewer { piece, top: 0, playing: None, take });
        if take.is_some() {
            d.say(TAKE_HELP, Level::Info);
        }
        d
    }

    fn home(&self) -> Page {
        let mut rows = vec![
            Row::new("Continue", Feed::Recent, "pieces you open show up here"),
            Row::new("My collection", Feed::Saved, "save a piece (c) to keep a copy that works offline"),
            Row::new("Local folders", Feed::Folders, ""),
            Row::new("Famous groups", Feed::Featured, ""),
            Row::new("16colo.rs", Feed::Years, "offline — press connect to browse the archive"),
        ];
        for r in &mut rows {
            self.fill_local(r);
        }
        Page { title: "Home".into(), rows, grid: false, sel: (0, 0), top: 0 }
    }

    /// Cards that need no network.
    fn fill_local(&self, r: &mut Row) {
        let cards = match &r.feed {
            Feed::Recent => self.shelf.recent.iter().cloned().map(Card::Piece).collect(),
            Feed::Saved => self.shelf.saved.iter().cloned().map(Card::Piece).collect(),
            Feed::Folders => {
                let mut v: Vec<Card> = self.shelf.folders.iter().cloned().map(Card::Folder).collect();
                v.push(Card::AddFolder);
                v
            }
            Feed::Featured => FEATURED_GROUPS
                .iter()
                .map(|(tag, name)| {
                    Card::Group(Group { name: tag.to_string(), longname: name.to_string(), releases: 0 })
                })
                .collect(),
            Feed::Folder(p) => gallery::local_pieces(p).into_iter().map(Card::Piece).collect(),
            _ => return,
        };
        r.cards = cards;
        r.load = Load::Ready;
    }

    /// Refresh the home rows that come from the shelf.
    fn refresh_shelf_rows(&mut self) {
        let _ = self.shelf.save(&self.shelf_path);
        let mut pages = std::mem::take(&mut self.pages);
        for p in &mut pages {
            for r in &mut p.rows {
                if matches!(r.feed, Feed::Recent | Feed::Saved | Feed::Folders) {
                    self.fill_local(r);
                }
            }
            // Keep the selection on a card that exists.
            if let Some(r) = p.rows.get(p.sel.0) {
                p.sel.1 = p.sel.1.min(r.cards.len().saturating_sub(1));
            }
        }
        self.pages = pages;
    }

    fn page(&self) -> &Page {
        self.pages.last().expect("the home page is never popped")
    }

    fn page_mut(&mut self) -> &mut Page {
        self.pages.last_mut().expect("the home page is never popped")
    }

    fn say(&mut self, s: impl Into<String>, level: Level) {
        self.status = Some((s.into(), level));
    }

    fn spinner(&self) -> char {
        SPINNER[(self.started.elapsed().as_millis() / 90) as usize % SPINNER.len()]
    }

    // ------------------------------------------------------------ jobs

    fn ask(&mut self, id: String, job: Job) {
        if self.asked.insert(id) {
            self.queue.push(job);
        }
    }

    fn pump(&mut self) {
        while self.running < WORKERS {
            let Some(job) = self.queue.pop() else { break };
            self.running += 1;
            let (tx, cache) = (self.tx.clone(), self.cache.clone());
            std::thread::spawn(move || {
                let msg = match job {
                    Job::Feed(f) => {
                        let r = fetch_feed(&f, &cache).map_err(|e| format!("{e:#}"));
                        Msg::Feed(f, r)
                    }
                    Job::Art(p) => {
                        let r = gallery::load(&p, &cache)
                            .map(|(a, bytes)| Box::new(Art { grid: a.doc.flatten(), doc: a.doc, bytes }))
                            .map_err(|e| format!("{e:#}"));
                        Msg::Art(p.key(), r)
                    }
                    Job::PackPoster(pack) => {
                        let r = gallery::pack_pieces(&pack, &cache).map_err(|e| format!("{e:#}")).and_then(|v| {
                            gallery::poster(&v).cloned().ok_or_else(|| "no art in this pack".to_string())
                        });
                        Msg::PackPoster(pack, r)
                    }
                    Job::GroupPoster(group) => {
                        let r = gallery::group_packs(&group, &cache)
                            .map_err(|e| format!("{e:#}"))
                            .and_then(|packs| packs.into_iter().next().ok_or_else(|| "no packs".to_string()))
                            .and_then(|p| gallery::pack_pieces(&p.name, &cache).map_err(|e| format!("{e:#}")))
                            .and_then(|v| gallery::poster(&v).cloned().ok_or_else(|| "no art".to_string()));
                        Msg::GroupPoster(group, r)
                    }
                };
                let _ = tx.send(msg);
            });
        }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            self.running = self.running.saturating_sub(1);
            match m {
                Msg::Feed(feed, r) => self.on_feed(feed, r),
                Msg::Art(key, r) => {
                    if let Err(e) = &r
                        && self.viewer.as_ref().is_some_and(|v| v.piece.key() == key)
                    {
                        self.say(format!("can't load it: {e}"), Level::Error);
                    }
                    self.arts.insert(key, r);
                }
                Msg::PackPoster(pack, r) => {
                    self.pack_posters.insert(pack, r);
                }
                Msg::GroupPoster(group, r) => {
                    self.group_posters.insert(group, r);
                }
            }
        }
        self.pump();
    }

    fn on_feed(&mut self, feed: Feed, r: Result<Vec<Card>, String>) {
        if feed == Feed::Years
            && let Ok(cards) = &r
        {
            // One row per year, after the 16colo.rs header row.
            let home = &mut self.pages[0];
            let years: Vec<u32> =
                cards.iter().filter_map(|c| if let Card::Pack(p) = c { Some(p.year) } else { None }).collect();
            if let Some(i) = home.rows.iter().position(|r| r.feed == Feed::Years) {
                home.rows.remove(i);
                for (k, y) in years.iter().enumerate() {
                    home.rows.insert(i + k, Row::new(format!("{y} on 16colo.rs"), Feed::Year(*y), "no packs"));
                }
            }
            return;
        }
        for p in &mut self.pages {
            for row in p.rows.iter_mut().filter(|row| row.feed == feed) {
                match &r {
                    Ok(cards) => {
                        row.cards = cards.clone();
                        row.load = Load::Ready;
                    }
                    Err(e) => row.load = Load::Failed(e.clone()),
                }
            }
        }
        if let Err(e) = r {
            self.say(e, Level::Error);
        }
    }

    /// Ask for what a visible row needs.
    fn want_row(&mut self, page: usize, row: usize) {
        let r = &self.pages[page].rows[row];
        if r.load != Load::Idle || (r.feed.remote() && self.offline) {
            return;
        }
        let feed = r.feed.clone();
        self.pages[page].rows[row].load = Load::Loading;
        self.ask(format!("feed {feed:?}"), Job::Feed(feed));
    }

    fn want_art(&mut self, p: &Piece) {
        if self.arts.contains_key(&p.key()) || (self.offline && matches!(p.origin, Origin::Pack { .. })) {
            return;
        }
        self.ask(format!("art {}", p.key()), Job::Art(p.clone()));
    }

    fn want_pack_poster(&mut self, pack: &str) {
        if self.offline {
            return;
        }
        match self.pack_posters.get(pack) {
            Some(Ok(p)) => {
                let p = p.clone();
                self.want_art(&p);
            }
            Some(Err(_)) => {}
            None => self.ask(format!("pack {pack}"), Job::PackPoster(pack.to_string())),
        }
    }

    // ------------------------------------------------------------ navigation

    fn open(&mut self) -> Outcome {
        let Some(card) = self.page().card().cloned() else { return Outcome::Keep };
        match card {
            Card::Piece(p) => {
                self.shelf.opened(&p);
                self.refresh_shelf_rows();
                self.want_art(&p);
                self.viewer = Some(Viewer { piece: p, top: 0, playing: None, take: None });
            }
            Card::Pack(p) => {
                let title = format!("{} · {} · {}", p.name, p.year, p.groups.join(", "));
                self.pages.push(Page::grid(title, Feed::Pack(p.name)));
            }
            Card::Group(g) => {
                self.pages.push(Page::grid(format!("{} — packs, newest first", g.longname), Feed::Group(g.name)))
            }
            Card::Artist(a) => self.pages.push(Page::grid(format!("{} — pieces", a.name), Feed::Artist(a.name))),
            Card::Folder(dir) => {
                let mut page = Page::grid(dir.display().to_string(), Feed::Folder(dir));
                self.fill_local(&mut page.rows[0]);
                self.pages.push(page);
            }
            Card::AddFolder => self.folder = Some(LineInput::default()),
        }
        Outcome::Keep
    }

    fn back(&mut self) -> Outcome {
        if self.viewer.take().is_some() {
            return Outcome::Keep;
        }
        if self.pages.len() > 1 {
            self.pages.pop();
            return Outcome::Keep;
        }
        Outcome::Close
    }

    fn run_search(&mut self) {
        let q = self.search.text.trim().to_string();
        self.search_focus = false;
        if q.is_empty() {
            return;
        }
        let mut local: Vec<Card> = vec![];
        for p in self.shelf.saved.iter().chain(&self.shelf.recent) {
            let hay = format!("{} {} {}", p.title, p.byline(), p.file()).to_lowercase();
            if hay.contains(&q.to_lowercase()) && !local.iter().any(|c| c == &Card::Piece(p.clone())) {
                local.push(Card::Piece(p.clone()));
            }
        }
        let mut mine = Row::new("Yours", Feed::Recent, "nothing of yours matches");
        mine.cards = local;
        mine.load = Load::Ready;
        let rows = vec![
            mine,
            Row::new("Packs", Feed::SearchPacks(q.clone()), "no packs match"),
            Row::new("Groups", Feed::SearchGroups(q.clone()), "no groups match"),
            Row::new("Artists", Feed::SearchArtists(q.clone()), "no artists match"),
        ];
        self.viewer = None;
        self.pages.truncate(1);
        self.pages.push(Page { title: format!("Search: {q}"), rows, grid: false, sel: (0, 0), top: 0 });
    }

    fn selected_piece(&self) -> Option<Piece> {
        if let Some(v) = &self.viewer {
            return Some(v.piece.clone());
        }
        match self.page().card()? {
            Card::Piece(p) => Some(p.clone()),
            _ => None,
        }
    }

    /// Open the piece (viewer or selected card) as the document.
    fn open_document(&mut self) -> Outcome {
        let Some(p) = self.selected_piece() else { return Outcome::Keep };
        match self.arts.get(&p.key()) {
            Some(Ok(art)) => {
                let doc = art.doc.clone();
                let msg = format!("opened {} — {} (untitled: Ctrl-S saves a copy)", p.title, p.byline());
                self.shelf.opened(&p);
                let _ = self.shelf.save(&self.shelf_path);
                Outcome::Then(Box::new(move |app: &mut App| app.replace_doc(Tab::new(doc, None), msg)))
            }
            Some(Err(e)) => {
                self.say(format!("can't open it: {e}"), Level::Error);
                Outcome::Keep
            }
            None => {
                self.want_art(&p);
                self.say("still loading — try again in a moment", Level::Warn);
                Outcome::Keep
            }
        }
    }

    fn send_to_studio(&mut self) -> Outcome {
        let source = match (self.selected_piece(), self.page().card()) {
            (Some(p), _) => p.source(),
            (None, Some(Card::Pack(p))) => format!("16colo.rs:{}", p.name),
            (None, Some(Card::Folder(d))) => d.to_string_lossy().into_owned(),
            _ => return Outcome::Keep,
        };
        self.studio_request = Some(source);
        Outcome::Keep
    }

    /// The part in the box (with no box, what's shown), as a paste that
    /// follows the mouse.
    fn take_part(&mut self) -> Outcome {
        let Some(v) = &self.viewer else { return Outcome::Keep };
        let Some(Ok(art)) = self.arts.get(&v.piece.key()) else { return Outcome::Keep };
        let a = self.art_area;
        let span = match v.take.and_then(|t| t.span()) {
            Some(span) => span,
            None if art.grid.width > 0 && art.grid.height > v.top && a.width > 0 && a.height > 0 => (
                0,
                v.top,
                art.grid.width.min(a.width as usize) - 1,
                art.grid.height.min(v.top + a.height as usize) - 1,
            ),
            None => return Outcome::Keep,
        };
        let (grid, pal) = (art.grid.clone(), art.doc.meta.palette.clone());
        let credit = format!("{} — {}", v.piece.title, v.piece.byline());
        Outcome::Then(Box::new(move |app: &mut App| {
            let clip = part_clip(&grid, &pal, span, &app.tab().doc.meta);
            let (w, h) = (clip.width, clip.height);
            app.clipboard = Some(clip.clone());
            app.float(clip, FloatSource::Paste);
            app.flash(format!("took {w}x{h} from {credit} · click to stamp · Esc: done"), Level::Ok);
        }))
    }

    fn keep(&mut self) {
        let Some(p) = self.selected_piece() else { return };
        if let Some(saved) = self.shelf.saved.iter().find(|s| s.key() == p.key()).cloned() {
            // A collection piece: forget it (its copy stays on disk).
            self.shelf.forget(&saved);
            self.say(format!("removed {} from your collection", p.title), Level::Info);
            self.refresh_shelf_rows();
            return;
        }
        if self.shelf.is_saved(&p) {
            self.say(format!("{} is already in your collection", p.title), Level::Info);
            return;
        }
        match self.arts.get(&p.key()) {
            Some(Ok(art)) => match self.shelf.keep(&p, &art.bytes, &self.collection) {
                Ok(kept) => {
                    // The copy is the same art: reuse what's loaded.
                    if let Some(Ok(a)) = self.arts.get(&p.key()) {
                        let copy = Art { doc: a.doc.clone(), grid: a.grid.clone(), bytes: a.bytes.clone() };
                        self.arts.insert(kept.key(), Ok(Box::new(copy)));
                    }
                    self.say(format!("saved {} to your collection", p.title), Level::Ok);
                    self.refresh_shelf_rows();
                }
                Err(e) => self.say(format!("{e:#}"), Level::Error),
            },
            _ => {
                self.want_art(&p);
                self.say("still loading — try again in a moment", Level::Warn);
            }
        }
    }

    /// Add the folder typed in the folder field and close the field. A path
    /// that isn't a folder keeps the field open, to fix it.
    fn add_folder(&mut self) {
        let Some(input) = &self.folder else { return };
        let t = input.text.trim().to_string();
        if t.is_empty() {
            self.folder = None;
            return;
        }
        let p = super::forms::expand_tilde(&t);
        if !p.is_dir() {
            self.say(format!("{} is not a folder — fix the path, or Esc", p.display()), Level::Error);
            return;
        }
        self.folder = None;
        // Kept for later sessions, which may start in another directory.
        let p = p.canonicalize().unwrap_or(p);
        let n = gallery::local_pieces(&p).len();
        if !self.shelf.folders.contains(&p) {
            self.shelf.folders.push(p.clone());
        }
        self.say(format!("added {} ({n} pieces)", p.display()), Level::Ok);
        self.refresh_shelf_rows();
    }

    fn act(&mut self, a: Act) -> Outcome {
        match a {
            Act::Open if self.viewer.as_ref().is_some_and(|v| v.take.is_some()) => return self.take_part(),
            Act::Open if self.viewer.is_some() => return self.open_document(),
            Act::Open => return self.open(),
            Act::Back => return self.back(),
            Act::Studio => return self.send_to_studio(),
            Act::Keep => self.keep(),
            Act::Search => self.search_focus = true,
            Act::AddFolder => self.folder = Some(LineInput::default()),
            Act::Connect => {
                self.offline = false;
                self.asked.clear();
                for p in &mut self.pages {
                    for r in &mut p.rows {
                        if matches!(r.load, Load::Failed(_)) {
                            r.load = Load::Idle;
                        }
                    }
                }
            }
            Act::Play => {
                if let Some(v) = &mut self.viewer {
                    v.playing = if v.playing.is_some() { None } else { Some(Instant::now()) };
                    v.top = 0;
                }
            }
            Act::Take => {
                if let Some(v) = &mut self.viewer {
                    v.take = if v.take.is_some() { None } else { Some(Take::default()) };
                    if v.take.is_some() {
                        self.say(TAKE_HELP, Level::Info);
                    }
                }
            }
            Act::Close => return Outcome::Close,
            Act::Scroll(row, by) => {
                let page = self.page_mut();
                if let Some(r) = page.rows.get_mut(row) {
                    let n = r.cards.len();
                    let s = (r.scroll as i32 + by).clamp(0, n.saturating_sub(1) as i32) as usize;
                    r.scroll = s;
                    page.sel = (row, s);
                }
            }
        }
        Outcome::Keep
    }

    /// What `c` does to this piece, as `keep` does it: a collection copy
    /// comes out again, a piece already kept stays.
    fn keep_label(&self, p: &Piece) -> &'static str {
        if self.shelf.saved.iter().any(|s| s.key() == p.key()) {
            "remove from collection"
        } else if self.shelf.is_saved(p) {
            "✓ in your collection"
        } else {
            "save to collection"
        }
    }

    fn buttons(&self) -> Vec<Btn<'static, Act>> {
        if self.folder.is_some() {
            return vec![btn(Act::AddFolder, "⏎", "add folder").primary(), btn(Act::Back, "esc", "cancel")];
        }
        if let Some(v) = &self.viewer {
            if let Some(t) = v.take {
                return vec![
                    btn(Act::Open, "⏎", if t.anchor.is_some() { "take it" } else { "take what's shown" }).primary(),
                    btn(Act::Take, "esc", "cancel"),
                ];
            }
            return vec![
                btn(Act::Open, "⏎", "open as document").primary(),
                btn(Act::Take, "t", "✂ take a part"),
                btn(Act::Studio, "s", "sourcing studio"),
                btn(Act::Keep, "c", self.keep_label(&v.piece)),
                btn(Act::Play, "p", if v.playing.is_some() { "stop" } else { "play at 14400 baud" }),
                btn(Act::Back, "esc", "back"),
                btn(Act::Close, "^C", "close"),
            ];
        }
        let card = self.page().card();
        let open = match card {
            Some(Card::Piece(_)) => "view",
            Some(Card::AddFolder) => "add a folder",
            Some(_) => "open",
            None => "open",
        };
        let mut v = vec![btn(Act::Open, "⏎", open).primary().enabled(card.is_some())];
        if matches!(card, Some(Card::Piece(_) | Card::Pack(_) | Card::Folder(_))) {
            v.push(btn(Act::Studio, "s", "sourcing studio"));
        }
        if let Some(Card::Piece(p)) = card {
            v.push(btn(Act::Keep, "c", self.keep_label(p)));
        }
        v.push(btn(Act::Search, "/", "search"));
        v.push(btn(Act::AddFolder, "+", "add folder"));
        if self.offline {
            v.push(btn(Act::Connect, "", "connect to 16colo.rs"));
        }
        v.push(btn(Act::Back, "esc", if self.pages.len() > 1 { "back" } else { "close" }));
        v
    }

    // ------------------------------------------------------------ drawing

    fn draw_card(&mut self, f: &mut Frame, r: Rect, card: &Card, selected: bool) {
        let dim = Style::new().fg(theme::DIM);
        let poster = Rect::new(r.x, r.y, r.width, POSTER_H.min(r.height));
        f.render_widget(Paragraph::new("").style(Style::new().bg(theme::BG)), poster);
        let (title, sub) = match card {
            Card::Piece(p) => {
                self.draw_piece_poster(f, poster, p);
                (p.title.clone(), p.byline())
            }
            Card::Pack(p) => {
                self.want_pack_poster(&p.name);
                match self.pack_posters.get(&p.name).cloned() {
                    Some(Ok(piece)) => self.draw_piece_poster(f, poster, &piece),
                    Some(Err(_)) => self.text_poster(f, poster, &p.name, "no preview"),
                    None if self.offline => self.text_poster(f, poster, &p.name, "offline"),
                    None => self.text_poster(f, poster, &p.name, &format!("{} loading", self.spinner())),
                }
                (p.name.clone(), format!("{} · {}", p.year, p.groups.join(", ")))
            }
            Card::Group(g) => {
                let sub = if g.releases > 0 { releases(g.releases) } else { "group".into() };
                if !self.offline && !self.group_posters.contains_key(&g.name) {
                    self.ask(format!("group {}", g.name), Job::GroupPoster(g.name.clone()));
                }
                match self.group_posters.get(&g.name).cloned() {
                    Some(Ok(piece)) => self.draw_piece_poster(f, poster, &piece),
                    _ => self.text_poster(f, poster, &g.longname, &sub),
                }
                (g.longname.clone(), sub)
            }
            Card::Artist(a) => {
                self.text_poster(f, poster, &a.name, &releases(a.releases));
                (a.name.clone(), "artist".into())
            }
            Card::Folder(d) => {
                let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.text_poster(f, poster, &format!("▤ {name}"), "folder");
                (name, d.display().to_string())
            }
            Card::AddFolder => {
                self.text_poster(f, poster, "+", "add a folder of art");
                ("Add folder".into(), "your own art files".into())
            }
        };
        if selected {
            // A bar on the poster's left edge.
            for y in poster.y..poster.bottom() {
                if let Some(c) = f.buffer_mut().cell_mut((r.x.saturating_sub(1), y)) {
                    c.set_char('▌').set_style(Style::new().fg(theme::ACCENT));
                }
            }
        }
        // Captions only where the card has room (the last row can be cut).
        if r.height <= POSTER_H {
            return;
        }
        let cap = Rect::new(r.x, r.y + POSTER_H, r.width, 1);
        let title_st = if selected {
            Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{:<w$}", ellipsize(&title, r.width as usize), w = r.width as usize),
                title_st,
            )),
            cap,
        );
        if r.height > POSTER_H + 1 {
            f.render_widget(
                Paragraph::new(Span::styled(ellipsize(&sub, r.width as usize), dim)),
                Rect::new(r.x, r.y + POSTER_H + 1, r.width, 1),
            );
        }
    }

    fn draw_piece_poster(&mut self, f: &mut Frame, poster: Rect, p: &Piece) {
        self.want_art(p);
        match self.arts.get(&p.key()) {
            Some(Ok(art)) => {
                minimap::draw_thumb(f.buffer_mut(), poster, &art.doc.canvas, &art.doc.meta.palette);
                self.posters.push((poster, p.key()));
            }
            Some(Err(_)) => self.text_poster(f, poster, &p.file(), "can't read it"),
            None if self.offline && matches!(p.origin, Origin::Pack { .. }) => {
                self.text_poster(f, poster, &p.file(), "offline")
            }
            None => {
                let s = format!("{} loading", self.spinner());
                self.text_poster(f, poster, &p.file(), &s)
            }
        }
    }

    fn text_poster(&self, f: &mut Frame, r: Rect, big: &str, small: &str) {
        f.render_widget(Paragraph::new("").style(Style::new().bg(theme::PANEL_HI)), r);
        let mid = r.y + r.height / 2;
        let w = r.width as usize;
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{:^w$}", ellipsize(big, w)),
                Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI).add_modifier(Modifier::BOLD),
            )),
            Rect::new(r.x, mid.saturating_sub(1), r.width, 1),
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{:^w$}", ellipsize(small, w)),
                Style::new().fg(theme::DIM).bg(theme::PANEL_HI),
            )),
            Rect::new(r.x, mid, r.width, 1),
        );
    }

    /// Land on a row with cards (the first rows can be empty, and taking
    /// the last piece out of My collection empties its row). Done before the
    /// buttons are laid out, so they speak for the card that is selected.
    fn land_on_cards(&mut self) {
        let page = self.page_mut();
        if page.grid {
            return;
        }
        if page.rows.get(page.sel.0).is_none_or(|r| r.cards.is_empty())
            && let Some(i) = page.rows.iter().position(|r| !r.cards.is_empty())
        {
            page.sel = (i, 0);
        }
    }

    fn draw_rows(&mut self, f: &mut Frame, body: Rect) {
        let per_row = ((body.width + GAP) / (CARD_W + GAP)).max(1) as usize;
        self.per_row = per_row;
        let pi = self.pages.len() - 1;
        {
            // Rows with cards are tall, empty ones a title and a note.
            let page = &mut self.pages[pi];
            let h = |r: &Row| if r.cards.is_empty() { 3 } else { CARD_H + 2 };
            page.top = page.top.min(page.sel.0);
            while page.top < page.sel.0 && page.rows[page.top..=page.sel.0].iter().map(h).sum::<u16>() > body.height {
                page.top += 1;
            }
        }
        let (top, sel, n_rows) = (self.pages[pi].top, self.pages[pi].sel, self.pages[pi].rows.len());
        let mut y = body.y;
        for ri in top..n_rows {
            if y + 3 > body.bottom() {
                break;
            }
            self.want_row(pi, ri);
            // Keep the selected card in view.
            {
                let row = &mut self.pages[pi].rows[ri];
                if ri == sel.0 {
                    if sel.1 < row.scroll {
                        row.scroll = sel.1;
                    }
                    if sel.1 >= row.scroll + per_row {
                        row.scroll = sel.1 + 1 - per_row;
                    }
                }
            }
            let row = &self.pages[pi].rows[ri];
            let (title, n, scroll, load, empty) =
                (row.title.clone(), row.cards.len(), row.scroll, row.load.clone(), row.empty);
            let mut head = vec![Span::styled(
                format!("{title}  "),
                Style::new().fg(if ri == sel.0 { theme::ACCENT2 } else { theme::TEXT }).add_modifier(Modifier::BOLD),
            )];
            match &load {
                Load::Loading => {
                    head.push(Span::styled(format!("{} loading", self.spinner()), Style::new().fg(theme::DIM)))
                }
                Load::Failed(e) => head.push(Span::styled(ellipsize(e, 60), Style::new().fg(theme::ERR))),
                _ if n > per_row => head.push(Span::styled(
                    format!("{}–{} of {n}", scroll + 1, (scroll + per_row).min(n)),
                    Style::new().fg(theme::DIM),
                )),
                _ => {}
            }
            f.render_widget(Paragraph::new(Line::from(head)), Rect::new(body.x, y, body.width, 1));
            if n > per_row {
                let buf = f.buffer_mut();
                let r = body.right();
                let right = btn(Act::Scroll(ri, per_row as i32), "", "›");
                let left = btn(Act::Scroll(ri, -(per_row as i32)), "", "‹");
                let w = right.width();
                self.btns.draw(buf, r - w, y, r, &right);
                self.btns.draw(buf, r - 2 * w - 1, y, r, &left);
            }
            y += 1;
            if n == 0 {
                let msg = match (&load, &row_feed_offline(&self.pages[pi].rows[ri].feed, self.offline)) {
                    (_, true) => "offline — press connect to browse 16colo.rs",
                    (Load::Ready, _) => empty,
                    _ => "",
                };
                f.render_widget(
                    Paragraph::new(Span::styled(msg, Style::new().fg(theme::DIM))),
                    Rect::new(body.x, y, body.width, 1),
                );
                y += 2;
                continue;
            }
            let cards: Vec<Card> = self.pages[pi].rows[ri].cards.iter().skip(scroll).take(per_row).cloned().collect();
            for (k, card) in cards.iter().enumerate() {
                let x = body.x + 1 + k as u16 * (CARD_W + GAP);
                let r = Rect::new(x, y, CARD_W, CARD_H.min(body.bottom().saturating_sub(y)));
                if r.height < 3 {
                    break;
                }
                let selected = sel == (ri, scroll + k);
                self.draw_card(f, r, card, selected);
                self.hits.push((r, ri, scroll + k));
            }
            y += CARD_H + 1;
        }
    }

    fn draw_grid(&mut self, f: &mut Frame, body: Rect) {
        let per_row = ((body.width.saturating_sub(1) + GAP) / (CARD_W + GAP)).max(1) as usize;
        self.per_row = per_row;
        let line_h = CARD_H + 1;
        let fit = (body.height / line_h).max(1) as usize;
        let pi = self.pages.len() - 1;
        self.want_row(pi, 0);
        let (sel, load, n) = {
            let p = &self.pages[pi];
            (p.sel.1, p.rows[0].load.clone(), p.rows[0].cards.len())
        };
        let line = sel / per_row;
        {
            let p = &mut self.pages[pi];
            if line < p.top {
                p.top = line;
            }
            if line >= p.top + fit {
                p.top = line + 1 - fit;
            }
        }
        let top = self.pages[pi].top;
        match (&load, n) {
            (Load::Loading, _) => {
                let s = format!("{} loading", self.spinner());
                f.render_widget(Paragraph::new(Span::styled(s, Style::new().fg(theme::DIM))), body);
                return;
            }
            (Load::Failed(e), _) => {
                f.render_widget(Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::ERR))), body);
                return;
            }
            (_, 0) => {
                let msg = if self.offline { "offline — press connect to browse 16colo.rs" } else { "nothing here" };
                f.render_widget(Paragraph::new(Span::styled(msg, Style::new().fg(theme::DIM))), body);
                return;
            }
            _ => {}
        }
        let cards: Vec<Card> =
            self.pages[pi].rows[0].cards.iter().skip(top * per_row).take(fit * per_row).cloned().collect();
        for (k, card) in cards.iter().enumerate() {
            let i = top * per_row + k;
            let x = body.x + 1 + (k % per_row) as u16 * (CARD_W + GAP);
            let y = body.y + (k / per_row) as u16 * line_h;
            let r = Rect::new(x, y, CARD_W, CARD_H.min(body.bottom().saturating_sub(y)));
            if r.height < 3 {
                break;
            }
            self.draw_card(f, r, card, i == sel);
            self.hits.push((r, 0, i));
        }
    }

    fn draw_viewer(&mut self, f: &mut Frame, body: Rect) {
        let Some(v) = &self.viewer else { return };
        let piece = v.piece.clone();
        self.want_art(&piece);
        let dim = Style::new().fg(theme::DIM);
        let side_w = if body.width >= 110 { 30 } else { 0 };
        let art_area =
            Rect::new(body.x, body.y, body.width.saturating_sub(side_w + if side_w > 0 { 2 } else { 0 }), body.height);
        self.art_area = art_area;
        match self.arts.get(&piece.key()) {
            Some(Ok(art)) => {
                let g = &art.grid;
                let pal = art.doc.meta.palette.clone();
                let v = self.viewer.as_mut().expect("viewer");
                // Playback reveals cell by cell and follows the reveal.
                let shown = v.playing.map(|t| (t.elapsed().as_secs_f32() * PLAY_CELLS_PER_SEC) as usize);
                if let Some(n) = shown {
                    let row = n / g.width.max(1);
                    if row >= v.top + art_area.height as usize {
                        v.top = row + 1 - art_area.height as usize;
                    }
                    if n >= g.width * g.height {
                        v.playing = None;
                    }
                }
                v.top = v.top.min(g.height.saturating_sub(art_area.height as usize));
                draw_grid_window(f.buffer_mut(), art_area, g, &pal, v.top, shown);
                if let Some((x0, y0, x1, y1)) = v.take.and_then(|t| t.span()) {
                    for y in y0.max(v.top)..=y1.min(v.top + art_area.height as usize - 1) {
                        for x in x0..=x1.min(art_area.width as usize - 1) {
                            let pos = (art_area.x + x as u16, art_area.y + (y - v.top) as u16);
                            if let Some(c) = f.buffer_mut().cell_mut(pos) {
                                c.modifier.insert(Modifier::REVERSED);
                            }
                        }
                    }
                }
                if g.height > art_area.height as usize {
                    let pos = format!(
                        " rows {}–{} of {} ",
                        v.top + 1,
                        (v.top + art_area.height as usize).min(g.height),
                        g.height
                    );
                    let w = pos.chars().count() as u16;
                    f.render_widget(
                        Paragraph::new(Span::styled(pos, Style::new().fg(theme::TEXT).bg(theme::PANEL_HI))),
                        Rect::new(art_area.right().saturating_sub(w), art_area.bottom().saturating_sub(1), w, 1),
                    );
                }
            }
            Some(Err(e)) => {
                f.render_widget(Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::ERR))), art_area)
            }
            None => {
                let s = format!("{} loading {}", self.spinner(), piece.file());
                f.render_widget(Paragraph::new(Span::styled(s, dim)), art_area)
            }
        }
        if side_w > 0 {
            let x = body.right() - side_w;
            let mut lines = vec![
                Line::from(Span::styled(
                    piece.title.clone(),
                    Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(piece.byline(), Style::new().fg(theme::TEXT))),
                Line::from(""),
            ];
            match &piece.origin {
                Origin::Pack { pack, file } => {
                    lines.push(Line::from(vec![
                        Span::styled("pack ", dim),
                        Span::styled(pack.clone(), Style::new().fg(theme::TEXT)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("file ", dim),
                        Span::styled(file.clone(), Style::new().fg(theme::TEXT)),
                    ]));
                }
                Origin::Local(p) => lines.push(Line::from(Span::styled(p.display().to_string(), dim))),
            }
            if let Some(Ok(art)) = self.arts.get(&piece.key()) {
                let d = &art.doc;
                lines.push(Line::from(Span::styled(format!("{}x{} cells", d.width(), d.height()), dim)));
                let s = &d.meta.sauce;
                if !s.title.trim().is_empty() || !s.author.trim().is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled("SAUCE", dim.add_modifier(Modifier::BOLD))));
                    for (k, val) in [("title", &s.title), ("author", &s.author), ("group", &s.group)] {
                        if !val.trim().is_empty() {
                            lines.push(Line::from(vec![
                                Span::styled(format!("{k} "), dim),
                                Span::styled(val.trim().to_string(), Style::new().fg(theme::TEXT)),
                            ]));
                        }
                    }
                }
            }
            if self.shelf.saved.iter().any(|s| s.key() == piece.key()) || self.shelf.is_saved(&piece) {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("♥ in your collection", Style::new().fg(theme::OK))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("↑↓ PgUp PgDn scroll · wheel too", dim)));
            f.render_widget(
                Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
                Rect::new(x, body.y, side_w, body.height),
            );
        }
    }

    /// Dragging the box when taking a part; None when not taking.
    fn take_mouse(&mut self, m: &MouseEvent) -> Option<Outcome> {
        let a = self.art_area;
        let v = self.viewer.as_mut()?;
        let t = v.take.as_mut()?;
        let (g_w, g_h) = match self.arts.get(&v.piece.key()) {
            Some(Ok(art)) => (art.grid.width, art.grid.height),
            _ => return Some(Outcome::Keep),
        };
        let inside = m.column >= a.x && m.column < a.right() && m.row >= a.y && m.row < a.bottom();
        let cell = |m: &MouseEvent| {
            let x = (m.column.clamp(a.x, a.right().saturating_sub(1)) - a.x) as usize;
            let y = v.top + (m.row.clamp(a.y, a.bottom().saturating_sub(1)) - a.y) as usize;
            (x.min(g_w.saturating_sub(1)), y.min(g_h.saturating_sub(1)))
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if inside => {
                let c = cell(m);
                *t = Take { anchor: Some(c), head: c };
            }
            MouseEventKind::Drag(MouseButton::Left) if t.anchor.is_some() => t.head = cell(m),
            MouseEventKind::Up(MouseButton::Left) if t.anchor.is_some() => {
                t.head = cell(m);
                return Some(self.take_part());
            }
            _ => return None,
        }
        Some(Outcome::Keep)
    }

    fn move_sel(&mut self, dx: i32, dy: i32, per_row: usize) {
        let page = self.page_mut();
        if page.grid {
            let n = page.rows[0].cards.len();
            if n == 0 {
                return;
            }
            let i = page.sel.1 as i32 + dx + dy * per_row as i32;
            page.sel.1 = i.clamp(0, n as i32 - 1) as usize;
            return;
        }
        let (mut r, mut c) = (page.sel.0 as i32, page.sel.1 as i32);
        if dy != 0 {
            // Skip empty rows.
            let mut nr = r + dy;
            while nr >= 0 && (nr as usize) < page.rows.len() && page.rows[nr as usize].cards.is_empty() {
                nr += dy;
            }
            if nr >= 0 && (nr as usize) < page.rows.len() {
                r = nr;
                c = c.min(page.rows[r as usize].cards.len() as i32 - 1).max(0);
            }
        }
        if dx != 0 {
            let n = page.rows[r as usize].cards.len() as i32;
            c = (c + dx).clamp(0, (n - 1).max(0));
        }
        page.sel = (r as usize, c as usize);
    }
}

fn releases(n: usize) -> String {
    if n == 1 { "1 release".into() } else { format!("{n} releases") }
}

fn row_feed_offline(feed: &Feed, offline: bool) -> bool {
    offline && feed.remote()
}

/// What a feed's cards are (runs on a worker thread).
fn fetch_feed(feed: &Feed, cache: &std::path::Path) -> anyhow::Result<Vec<Card>> {
    Ok(match feed {
        Feed::Years => gallery::years(cache)?
            .into_iter()
            .map(|y| Card::Pack(PackInfo { name: String::new(), year: y, groups: vec![] }))
            .collect(),
        Feed::Year(y) => {
            // Newest packs of the year first (names sort roughly by date).
            let mut v = gallery::year_packs(*y, cache)?;
            v.reverse();
            v.into_iter().map(Card::Pack).collect()
        }
        Feed::Pack(p) => gallery::pack_pieces(p, cache)?.into_iter().map(Card::Piece).collect(),
        Feed::Group(g) => gallery::group_packs(g, cache)?.into_iter().map(Card::Pack).collect(),
        Feed::Artist(a) => gallery::artist_pieces(a, cache)?.into_iter().map(Card::Piece).collect(),
        Feed::SearchPacks(q) => gallery::search_packs(q, cache)?.into_iter().map(Card::Pack).collect(),
        Feed::SearchGroups(q) => gallery::groups(q, cache)?.into_iter().map(Card::Group).collect(),
        Feed::SearchArtists(q) => gallery::artists(q, cache)?.into_iter().map(Card::Artist).collect(),
        Feed::Recent | Feed::Saved | Feed::Folders | Feed::Featured | Feed::Folder(_) => vec![],
    })
}

/// The art at 1:1 from row `top`; with `shown`, only that many cells (playback).
fn draw_grid_window(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    g: &Grid,
    pal: &Palette,
    top: usize,
    shown: Option<usize>,
) {
    for sy in 0..area.height as usize {
        let y = top + sy;
        if y >= g.height {
            break;
        }
        for x in 0..g.width.min(area.width as usize) {
            if shown.is_some_and(|n| y * g.width + x >= n) {
                return;
            }
            let c = g.get(x, y);
            let ch = if c.ch.is_control() || c.ch == '\0' { ' ' } else { c.ch };
            if let Some(cell) = buf.cell_mut((area.x + x as u16, area.y + sy as u16)) {
                cell.set_char(ch).set_style(Style::new().fg(rgb(c.fg, pal)).bg(rgb(c.bg, pal)));
            }
        }
    }
}

impl Dialog for GalleryDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        self.poll();
        self.btns.clear();
        self.hits.clear();
        self.posters.clear();
        let r = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
        let hint = "scene art from 16colo.rs and your folders — credit the artists";
        let inner = popup(f, r, "Gallery", hint);
        if inner.height < 8 || inner.width < 30 {
            return;
        }
        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);
        // Where we are, and the search box.
        let crumbs: Vec<String> = self.pages.iter().map(|p| p.title.clone()).collect();
        let mut path = crumbs.join(" › ");
        if let Some(v) = &self.viewer {
            path = format!("{path} › {}", v.piece.title);
        }
        let search_w = 34.min(w / 2);
        f.render_widget(
            Paragraph::new(Span::styled(
                ellipsize(&path, (w - search_w - 2) as usize),
                Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD),
            )),
            Rect::new(x, inner.y, w - search_w - 2, 1),
        );
        self.search_area = Rect::new(x + w - search_w, inner.y, search_w, 1);
        self.search.render(f, self.search_area, "Search › ", self.search_focus);
        if let Some((msg, level)) = &self.status {
            let color = match level {
                Level::Info => theme::TEXT,
                Level::Ok => theme::OK,
                Level::Warn => theme::WARN,
                Level::Error => theme::ERR,
            };
            f.render_widget(
                Paragraph::new(Span::styled(ellipsize(msg, w as usize), Style::new().fg(color))).right_aligned(),
                Rect::new(x, inner.y + 1, w, 1),
            );
        }
        self.land_on_cards();
        let bar_btns = self.buttons();
        let bar_h = Buttons::rows_needed(&bar_btns, w).min(2);
        let body = Rect::new(x, inner.y + 2, w, inner.height.saturating_sub(3 + bar_h));
        if self.viewer.is_some() {
            self.draw_viewer(f, body);
        } else if self.page().grid {
            self.draw_grid(f, body);
        } else {
            self.draw_rows(f, body);
        }
        if let Some(input) = &self.folder {
            let pr = Rect::new(x, inner.bottom() - bar_h - 1, w.min(80), 1);
            f.render_widget(Paragraph::new("").style(Style::new().bg(theme::PANEL)), Rect::new(x, pr.y, w, 1));
            input.render(f, pr, "Folder › ", true);
        }
        self.btns.row(f.buffer_mut(), Rect::new(x, inner.bottom() - bar_h, w, bar_h), &bar_btns);
        self.pump();
    }

    fn pixels(&mut self, f: &mut Frame, thumbs: &mut crate::ui::thumbs::Thumbs) {
        let keys: Vec<u64> = self.posters.iter().map(|(_, k)| key_hash(k)).collect();
        thumbs.retain_art(&keys);
        for (r, k) in &self.posters {
            if let Some(Ok(art)) = self.arts.get(k) {
                thumbs.art(f, *r, key_hash(k), &art.doc.canvas, &art.doc.meta.palette);
            }
        }
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        self.poll();
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Close;
        }
        if let Some(input) = &mut self.folder {
            match k.code {
                KeyCode::Esc => self.folder = None,
                KeyCode::Enter => self.add_folder(),
                _ => {
                    input.key(&k);
                }
            }
            return Outcome::Keep;
        }
        if self.search_focus {
            match k.code {
                KeyCode::Esc => self.search_focus = false,
                KeyCode::Enter => self.run_search(),
                _ => {
                    self.search.key(&k);
                }
            }
            return Outcome::Keep;
        }
        if let Some(v) = &mut self.viewer {
            let page = 20;
            if v.take.is_some() && k.code == KeyCode::Esc {
                v.take = None;
                self.status = None;
                return Outcome::Keep;
            }
            // The box by keyboard: arrows size it from the top-left of the
            // view, Shift+arrows move it.
            let arrow = match k.code {
                KeyCode::Left => Some((-1, 0)),
                KeyCode::Right => Some((1, 0)),
                KeyCode::Up => Some((0, -1)),
                KeyCode::Down => Some((0, 1)),
                _ => None,
            };
            if let (Some(t), Some((dx, dy)), Some(Ok(art))) = (v.take.as_mut(), arrow, self.arts.get(&v.piece.key())) {
                let (gw, gh) = (art.grid.width as i64, art.grid.height as i64);
                if gw == 0 || gh == 0 {
                    return Outcome::Keep;
                }
                if t.anchor.is_none() {
                    let corner = (0, v.top.min(gh as usize - 1));
                    *t = Take { anchor: Some(corner), head: corner };
                }
                let anchor = t.anchor.expect("anchored");
                let at = |(x, y): (usize, usize)| {
                    ((x as i64 + dx).clamp(0, gw - 1) as usize, (y as i64 + dy).clamp(0, gh - 1) as usize)
                };
                if k.modifiers.contains(KeyModifiers::SHIFT) {
                    let (a2, h2) = (at(anchor), at(t.head));
                    // Only move when both corners can (the box keeps its size).
                    if (a2.0 as i64 - anchor.0 as i64, a2.1 as i64 - anchor.1 as i64)
                        == (h2.0 as i64 - t.head.0 as i64, h2.1 as i64 - t.head.1 as i64)
                    {
                        t.anchor = Some(a2);
                        t.head = h2;
                    }
                } else {
                    t.head = at(t.head);
                }
                // Keep the moving corner in view.
                let rows = self.art_area.height.max(1) as usize;
                if t.head.1 < v.top {
                    v.top = t.head.1;
                } else if t.head.1 >= v.top + rows {
                    v.top = t.head.1 + 1 - rows;
                }
                let (x0, y0, x1, y1) = t.span().expect("anchored");
                let msg = format!("{}x{} · ⏎ take it", x1 - x0 + 1, y1 - y0 + 1);
                self.say(msg, Level::Info);
                return Outcome::Keep;
            }
            match k.code {
                KeyCode::Up => v.top = v.top.saturating_sub(1),
                KeyCode::Down => v.top += 1,
                KeyCode::PageUp => v.top = v.top.saturating_sub(page),
                KeyCode::PageDown | KeyCode::Char(' ') => v.top += page,
                KeyCode::Home => v.top = 0,
                KeyCode::End => v.top = usize::MAX / 2,
                KeyCode::Enter => return self.act(Act::Open),
                KeyCode::Esc | KeyCode::Backspace => return self.act(Act::Back),
                KeyCode::Char('s') => return self.act(Act::Studio),
                KeyCode::Char('c') => return self.act(Act::Keep),
                KeyCode::Char('p') => return self.act(Act::Play),
                KeyCode::Char('t') => return self.act(Act::Take),
                _ => {}
            }
            return Outcome::Keep;
        }
        let per_row = self.per_row;
        match k.code {
            KeyCode::Left => self.move_sel(-1, 0, per_row),
            KeyCode::Right => self.move_sel(1, 0, per_row),
            KeyCode::Up => self.move_sel(0, -1, per_row),
            KeyCode::Down => self.move_sel(0, 1, per_row),
            KeyCode::Enter => return self.act(Act::Open),
            KeyCode::Esc | KeyCode::Backspace => return self.act(Act::Back),
            KeyCode::Char('/') => self.search_focus = true,
            KeyCode::Char('s') => return self.act(Act::Studio),
            KeyCode::Char('c') => return self.act(Act::Keep),
            KeyCode::Char('+') => return self.act(Act::AddFolder),
            _ => {}
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        self.poll();
        if let Some(a) = self.btns.mouse(&m) {
            if self.folder.is_some() {
                // The folder field's own buttons: add, or cancel just the field.
                match a {
                    Act::AddFolder => self.add_folder(),
                    _ => self.folder = None,
                }
                return Outcome::Keep;
            }
            return self.act(a);
        }
        if let Some(out) = self.take_mouse(&m) {
            return out;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let s = self.search_area;
                self.search_focus = m.row == s.y && m.column >= s.x && m.column < s.right();
                if self.viewer.is_some() {
                    return Outcome::Keep;
                }
                if let Some(&(_, row, card)) = self
                    .hits
                    .iter()
                    .find(|(r, _, _)| m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom())
                {
                    // Click selects; a click on the selected card opens it.
                    if self.page().sel == (row, card) {
                        return self.act(Act::Open);
                    }
                    self.page_mut().sel = (row, card);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
                if let Some(v) = &mut self.viewer {
                    v.top = (v.top as i32 + d * 3).max(0) as usize;
                } else {
                    let per_row = self.per_row;
                    self.move_sel(0, d, per_row);
                }
            }
            _ => {}
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        if let Some(input) = &mut self.folder {
            input.paste(s.trim());
        } else {
            self.search_focus = true;
            self.search.paste(s.trim());
        }
    }

    fn animating(&self) -> bool {
        self.running > 0 || !self.queue.is_empty() || self.viewer.as_ref().is_some_and(|v| v.playing.is_some())
    }
}
