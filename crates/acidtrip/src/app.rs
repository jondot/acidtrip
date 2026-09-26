//! Application state, event loop and action dispatch.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use acidtrip_ai::agent::{AgentConfig, AgentEvent};
use acidtrip_ai::exec::ExecState;
use acidtrip_ai::live::LiveGuard;
use acidtrip_ai::{ToolRequest, ToolResult};
use acidtrip_core::tools::{self, Justify, PaintMode, Rect as CellRect, Symmetry};
use acidtrip_core::{Cell, Clip, Color, DocKind, Document, TxBuilder};
use acidtrip_io::config::Config;
use acidtrip_io::fonts::FontLibrary;
use acidtrip_io::format::{self, Format, SaveOptions};
use acidtrip_io::library::Paths;
use acidtrip_io::stencils::StencilLibrary;
use acidtrip_io::{backup, recovery, versions};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::DefaultTerminal;
use ratatui::layout::Rect;

use crate::actions::Action;
use crate::dialogs::{self, Dialog, Outcome};
use crate::keymap::{Keymap, Preset};
use crate::tab::Tab;
use crate::tools_ctl::{self, Button, FloatSource, Floating, Tool, ToolState};
use crate::ui::canvas::CanvasGeom;
use crate::ui::sidebar::{Hit, LayerOp, Opt, Slot};

/// Messages the message history keeps.
pub const MSG_LOG_MAX: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

pub struct AgentRun {
    pub events: Receiver<AgentEvent>,
    /// This run's tool calls. Dropping the run drops it, so a run the user
    /// stopped fails its next call and makes no more API rounds, and never
    /// draws into a newer run.
    tools: Receiver<ToolRequest>,
    pub status: String,
    pub doc_id: uuid::Uuid,
    pub started: Instant,
}

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub tools: ToolState,
    pub keymap: Keymap,
    pub config: Config,
    pub paths: Paths,
    pub fonts: FontLibrary,
    pub stencils: StencilLibrary,
    pub dialogs: Vec<Box<dyn Dialog>>,
    pub msg: Option<(String, Level, Instant)>,
    /// Every message flashed this session, oldest first (capped): the
    /// status bar cuts long ones short, the message history shows them whole.
    pub msg_log: Vec<(String, Level, Instant)>,
    pub geom: CanvasGeom,
    pub sidebar_hits: Vec<(Rect, Hit)>,
    pub show_sidebar: bool,
    /// Under 100 columns the sidebar is hidden unless asked for (Ctrl-B, or
    /// a panel that lives in it): then it takes the right of the canvas.
    pub narrow_sidebar: bool,
    /// Terminal columns at the last draw.
    pub screen_w: u16,
    pub grid: bool,
    pub quit: bool,
    tool_rx: Receiver<ToolRequest>,
    pub agent: Option<AgentRun>,
    pub live: Option<LiveGuard>,
    mouse_button: Option<Button>,
    pub hover: Option<(usize, usize)>,
    pub enhanced_keys: bool,
    pub clipboard: Option<Clip>,
    typing: bool,
    last_autosave: Instant,
    pub playback: Option<crate::dialogs::playback::Playback>,
    /// Letters Claude is drawing for a font after the studio closed.
    pub studio_drawing: Option<crate::dialogs::harvest::StudioDrawing>,
    /// Replay of the edit log (see replay.rs); the live doc is untouched.
    pub replay: Option<crate::replay::ReplayView>,
    /// Graphics protocol picker for the pixel-exact preview (queried at start).
    pub picker: Option<ratatui_image::picker::Picker>,
    pub ai_own_layer: bool,
    /// Keyboard cursor shown (hidden while working with the mouse).
    pub key_cursor: bool,
    /// Blink phase origin: reset on every cursor move so the cursor shows at once.
    pub blink_start: Instant,
    /// Blinking cursor (ACiDDraw's "flashing cursor"), so a cursor over a full
    /// block doesn't hide what's there. Off with ACIDTRIP_NO_BLINK=1.
    pub blink: bool,
    /// Which color the palette sets on a plain click (Paint's primary/secondary).
    pub color_slot: Slot,
    pub show_minimap: bool,
    /// Where the top dialog drew its popup, with the dialog count then.
    pub dialog_area: Option<(usize, Rect)>,
    /// The screen was handed to another program (the settings editor):
    /// repaint every cell, not just what changed.
    pub repaint: bool,
    pub minimap_rect: Option<crate::ui::minimap::MiniGeom>,
    /// Sidebar control held down (minimap, sliders): drags keep feeding it.
    side_drag: Option<Hit>,
    /// Sidebar control under the mouse: highlighted, its tip in the status bar.
    pub side_hover: Option<Hit>,
    /// When the mouse reached `side_hover`: a message flashed after that
    /// (the click's result) wins over the tip.
    pub side_hover_at: Instant,
    /// The sidebar panel last opened from its folded title (short terminals).
    pub side_open: Option<crate::ui::sidebar::Fold>,
    /// Art mode: the left hand types glyphs from the board, the right moves.
    pub artboard: acidtrip_io::artboard::ArtBoard,
    /// Pieces recently viewed in the Gallery (the sidebar's strip).
    pub recent: crate::recent::Recent,
    /// The sidebar's pen brush preview, rendered when the brush changes.
    pub side_preview: Option<(dialogs::brushes::PreviewKey, Vec<acidtrip_core::Cell>)>,
    /// Pixel-image previews when the terminal supports graphics.
    pub thumbs: Option<crate::ui::thumbs::Thumbs>,
    /// Cell size in pixels when the terminal reports pixel-precise mouse
    /// positions (SGR-Pixels, mode 1016).
    pub cell_px: Option<(f32, f32)>,
    /// Position inside the cell (0..1, 0..1) of the last mouse event.
    pub mouse_frac: (f32, f32),
    /// Draw together: the shared session, if any, and its panel.
    pub together: crate::together::Together,
}

impl App {
    pub fn new(
        files: &[PathBuf],
        enhanced_keys: bool,
        picker: Option<ratatui_image::picker::Picker>,
    ) -> anyhow::Result<App> {
        let paths = Paths::resolve()?;
        let first_run = !paths.config_file().exists();
        let artboard = acidtrip_io::artboard::ArtBoard::load(&paths.artboard_file());
        let recent = crate::recent::Recent::new(&paths.state_dir);
        let (config, config_err) = match Config::load_or_create(&paths.config_file()) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{}, using defaults", config_error(&e)))),
        };
        let (keymap, key_errs) = Keymap::from_config(&config.keymap.preset, &config.keymap.bindings);
        let fonts = FontLibrary::load(Some(&paths.fonts_dir()));
        let stencils = StencilLibrary::load(&paths.stencils_dir());
        let (tool_tx, tool_rx) = mpsc::channel();
        let live = acidtrip_ai::live::serve(&paths.sockets_dir(), tool_tx).ok();
        let mut app = App {
            tabs: vec![],
            active: 0,
            tools: ToolState::default(),
            keymap,
            show_sidebar: config.ui.sidebar,
            narrow_sidebar: false,
            screen_w: 0,
            grid: config.ui.show_grid,
            config,
            paths,
            fonts,
            stencils,
            dialogs: vec![],
            msg: None,
            msg_log: vec![],
            geom: CanvasGeom::default(),
            sidebar_hits: vec![],
            quit: false,
            tool_rx,
            agent: None,
            live,
            mouse_button: None,
            hover: None,
            enhanced_keys,
            clipboard: None,
            typing: false,
            last_autosave: Instant::now(),
            playback: None,
            studio_drawing: None,
            replay: None,
            picker,
            ai_own_layer: true,
            key_cursor: false,
            blink_start: Instant::now(),
            blink: std::env::var("ACIDTRIP_NO_BLINK").map_or(true, |v| v != "1"),
            color_slot: Slot::Fg,
            show_minimap: true,
            dialog_area: None,
            repaint: false,
            minimap_rect: None,
            side_drag: None,
            side_hover: None,
            side_hover_at: Instant::now(),
            side_open: None,
            artboard,
            recent,
            side_preview: None,
            thumbs: None,
            cell_px: None,
            mouse_frac: (0.5, 0.5),
            together: Default::default(),
        };
        app.reload_brushes();
        app.reload_patterns();
        app.tools.select_brush(app.tools.brush_idx);
        app.thumbs = crate::ui::thumbs::Thumbs::new(app.picker.as_ref());
        if app.keymap.preset == Preset::Acid {
            app.tools.set_tool(Tool::Text);
        }
        // One document at a time: open the first file given. What happened to
        // it is said after the welcome line, which would otherwise cover it.
        let mut opening = None;
        if files.len() > 1 {
            opening = Some((format!("acidtrip edits one file at a time — opened {}", file_name(&files[0])), Level::Warn));
        }
        if let Some(f) = files.first() {
            match format::load_with_log(f) {
                Ok((doc, log)) => app.tabs.push(Tab::new(doc, Some(f.clone())).with_log(log)),
                Err(_) if !f.exists() => {
                    // A new file name: start a doc that saves there.
                    let mut t = Tab::new(app.new_doc(), Some(f.clone()));
                    t.history.mark_saved();
                    app.tabs.push(t);
                }
                Err(e) => opening = Some((open_error(f, &e), Level::Error)),
            }
        }
        if app.tabs.is_empty() {
            let d = app.new_doc();
            app.tabs.push(Tab::new(d, None));
        }
        app.active = app.tabs.len() - 1;
        let notes: Vec<String> = config_err.into_iter().chain(key_errs).collect();
        if let Some(n) = notes_line(&notes) {
            app.flash(n, Level::Warn);
        } else {
            app.flash("Ctrl-K commands · ? help · T type · draw with the mouse", Level::Info);
        }
        if let Some((text, level)) = opening {
            app.flash(text, level);
        }
        let rec = recovery::list(&app.paths.recovery_dir());
        if !rec.is_empty() {
            app.dialogs.push(Box::new(dialogs::recovery::RecoveryDialog::new(rec)));
        }
        if first_run {
            app.dialogs.push(Box::new(dialogs::help::HelpDialog::welcome()));
        }
        Ok(app)
    }

    pub fn new_doc(&self) -> Document {
        let c = &self.config.new_doc;
        let kind = if c.kind.eq_ignore_ascii_case("modern") { DocKind::Modern } else { DocKind::Classic };
        let mut d = Document::new(kind, c.width.clamp(1, 4000), c.height.clamp(1, 20000));
        d.meta.ice = c.ice;
        d.meta.sauce.author = self.config.ui.author.clone();
        d.meta.sauce.group = self.config.ui.group.clone();
        d.meta.sauce.attach = true;
        d
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Show the sidebar for a panel in it, on a narrow terminal too.
    pub fn reveal_sidebar(&mut self) {
        self.show_sidebar = true;
        self.narrow_sidebar = true;
    }

    pub fn flash(&mut self, text: impl Into<String>, level: Level) {
        let text = text.into();
        let now = Instant::now();
        match self.msg_log.last_mut() {
            // The same news again (a key held down) is one entry.
            Some(last) if last.0 == text && last.1 == level => last.2 = now,
            _ => self.msg_log.push((text.clone(), level, now)),
        }
        if self.msg_log.len() > MSG_LOG_MAX {
            self.msg_log.remove(0);
        }
        self.msg = Some((text, level, now));
    }

    /// No mouse button held (for hover previews).
    pub fn mouse_idle(&self) -> bool {
        self.mouse_button.is_none() && self.side_drag.is_none()
    }

    /// Half-block pixel row for a canvas cell row: exact in zoom view or with
    /// pixel-precise mouse reports, else the top half (tools paint whole cells).
    fn pixel_row(&self, y: usize, py_zoom: usize) -> usize {
        if self.tab().zoom {
            py_zoom
        } else if self.cell_px.is_some() {
            y * 2 + usize::from(self.mouse_frac.1 >= 0.5)
        } else {
            y * 2
        }
    }

    /// Mouse position in glyph pixels (8x16 per cell) for the pen: exact with
    /// pixel mouse reports or in zoom view, else the cell center.
    fn pen_point(&self, sx: u16, sy: u16, x: usize, y: usize) -> (f32, f32) {
        let (mut fx, mut fy) = if self.cell_px.is_some() { self.mouse_frac } else { (0.5, 0.5) };
        if self.geom.zoom {
            // Each cell is 2x2 terminal cells in zoom view.
            let (dx, dy) = (sx.saturating_sub(self.geom.area.x) % 2, sy.saturating_sub(self.geom.area.y) % 2);
            fx = (dx as f32 + fx) / 2.0;
            fy = (dy as f32 + fy) / 2.0;
        }
        (x as f32 * 8.0 + fx * 8.0, y as f32 * 16.0 + fy * 16.0)
    }

    /// Scroll so canvas cell (x, y) is centered in the viewport.
    fn center_on(&mut self, x: usize, y: usize) {
        let (vw, vh) = (self.geom.cols.max(1), self.geom.rows.max(1));
        let t = self.tab_mut();
        let (w, h) = (t.doc.width(), t.doc.height());
        t.scroll.0 = x.saturating_sub(vw / 2).min(w.saturating_sub(vw));
        t.scroll.1 = y.saturating_sub(vh / 2).min(h.saturating_sub(vh));
    }

    /// Show the keyboard cursor this frame (blink "on" phase).
    pub fn cursor_visible(&self) -> bool {
        self.key_cursor && (!self.blink || (self.blink_start.elapsed().as_millis() / 500).is_multiple_of(2))
    }

    fn cursor_blinking(&self) -> bool {
        self.blink && self.key_cursor && self.dialogs.is_empty() && !self.typing_mode()
    }

    pub fn typing_mode(&self) -> bool {
        self.tools.tool == Tool::Text && self.tools.floating.is_none()
    }

    // ------------------------------------------------------------ loop

    pub fn run_loop(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let mut dirty = true;
        let mut last_draw = Instant::now();
        while !self.quit {
            // Animations (AI spinner, modem playback) redraw on a timer; otherwise
            // only when something changed.
            let animating = self.agent.is_some()
                || self.playback.is_some()
                || self.tab().playing.is_some()
                || self.replay_animating()
                || self.cursor_blinking()
                || self.thumbs.as_ref().is_some_and(|t| t.stale)
                || self.dialogs.iter().any(|d| d.animating());
            if std::mem::take(&mut self.repaint) {
                terminal.clear()?;
                dirty = true;
            }
            if dirty || (animating && last_draw.elapsed() >= Duration::from_millis(80)) {
                terminal.draw(|f| crate::ui::draw(f, self))?;
                last_draw = Instant::now();
                dirty = false;
            }
            let timeout = if animating || self.together.active() { 30 } else { 100 };
            if event::poll(Duration::from_millis(timeout))? {
                self.handle_event(event::read()?);
                dirty = true;
                // Drain bursts (mouse drags) before redrawing.
                let burst = Instant::now();
                while burst.elapsed() < Duration::from_millis(12) && event::poll(Duration::ZERO)? {
                    self.handle_event(event::read()?);
                    if self.quit {
                        break;
                    }
                }
            }
            dirty |= self.tick();
            dirty |= self.together_tick();
        }
        self.together_leave();
        Ok(())
    }

    pub fn handle_event(&mut self, ev: Event) {
        // Debugging stray input: ACIDTRIP_LOG gets every key event.
        if let (Event::Key(k), Some(path)) = (&ev, std::env::var_os("ACIDTRIP_LOG")) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "key {k:?} · cursor {:?} tool {:?} dialogs {}", self.tab().cursor, self.tools.tool, self.dialogs.len());
            }
        }
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Paste(s) => self.on_paste(&s),
            Event::Resize(..) => {}
            _ => {}
        }
    }

    fn with_top_dialog(&mut self, f: impl FnOnce(&mut Box<dyn Dialog>, &App) -> Outcome) -> bool {
        let Some(mut d) = self.dialogs.pop() else {
            return false;
        };
        let out = f(&mut d, self);
        self.apply_outcome(d, out);
        true
    }

    fn apply_outcome(&mut self, d: Box<dyn Dialog>, out: Outcome) {
        if matches!(out, Outcome::Close | Outcome::Then(_)) {
            // The Gallery may have added to the recent pieces.
            self.recent.refresh();
        }
        match out {
            Outcome::Keep => self.dialogs.push(d),
            Outcome::Close => {}
            Outcome::Then(cb) => cb(self),
            Outcome::KeepThen(cb) => {
                self.dialogs.push(d);
                cb(self);
            }
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.playback.is_some() {
            self.playback = None;
            self.flash("playback stopped", Level::Info);
            return;
        }
        if !self.dialogs.is_empty() {
            self.with_top_dialog(|d, app| d.key(k, app));
            return;
        }
        if self.together.focus && self.together_key(k) {
            return;
        }
        if k.code == KeyCode::Esc && self.agent.is_some() {
            self.cancel_agent();
            return;
        }
        if self.mouse_button.is_some()
            && (self.art_mode() || !self.keymap.lookup(&k, self.typing_mode()).is_some_and(Action::keeps_drag))
        {
            // Esc only ends the drag; other keys end it and then do their thing.
            if self.end_drag(k.code == KeyCode::Esc) {
                return;
            }
        }
        if self.art_mode() && self.art_key(k) {
            return;
        }
        if self.replay_on() {
            if self.replay_key(k) {
                return;
            }
            // Typing goes to the live piece.
            if self.keymap.lookup(&k, self.typing_mode()).is_none() {
                self.replay = None;
            }
        }
        let typing = self.typing_mode();
        if let Some(a) = self.keymap.lookup(&k, typing) {
            if a != Action::Undo {
                self.end_typing();
            }
            self.run(a);
            return;
        }
        if typing {
            self.type_key(k);
        } else if k.code == KeyCode::Esc {
            self.run(Action::Deselect);
        }
    }

    /// End a mouse drag early because a key came in (see
    /// [`ToolState::end_drag`]). True when `cancel` (Esc) had a drag to end,
    /// so the key is used up.
    fn end_drag(&mut self, cancel: bool) -> bool {
        if self.mouse_button.take().is_none() {
            return false;
        }
        let mut tools = std::mem::take(&mut self.tools);
        let msg = tools.end_drag(&mut self.tabs[self.active], cancel);
        self.tools = tools;
        if let Some(s) = msg {
            self.flash(s, Level::Info);
        }
        cancel
    }

    /// The active layer can't take edits (locked, or hidden so the edit
    /// wouldn't show): says so and returns true.
    fn layer_blocked(&mut self) -> bool {
        let t = self.tab();
        let l = &t.doc.canvas.layers[t.layer];
        let (why, fix) = if l.locked {
            ("locked", "unlock")
        } else if !l.visible {
            ("hidden", "show")
        } else {
            return false;
        };
        let msg = format!("{} is {why}: {fix} it in LAYERS (≡ panel) to draw on it", l.name);
        self.flash(msg, Level::Warn);
        true
    }

    /// Whether any unlocked layer has art in `cells` (what an insert pushes
    /// off the edge; locked layers don't move).
    fn has_art(&self, mut cells: impl Iterator<Item = (usize, usize)>) -> bool {
        let c = &self.tab().doc.canvas;
        cells.any(|(x, y)| {
            (0..c.layers.len()).any(|l| !c.layers[l].locked && c.get(l, x, y).is_some_and(|cell| !cell.is_blank()))
        })
    }

    /// Insert or delete a line or column: every unlocked layer shifts,
    /// locked ones stay put (as they do for every other edit). Says what
    /// happened, and warns when art fell off `edge`.
    fn shift_edit(&mut self, label: &str, f: impl FnOnce(&mut TxBuilder), done: String, edge: Option<&str>) {
        let layers = &self.tab().doc.canvas.layers;
        if layers.iter().all(|l| l.locked) {
            self.flash("every layer is locked: unlock one in LAYERS (≡ panel) to shift it", Level::Warn);
            return;
        }
        let kept = layers.iter().any(|l| l.locked);
        self.tab_mut().edit(label, f);
        let done = if kept { format!("{done} (locked layers stay put)") } else { done };
        match edge {
            Some(edge) => self.flash(format!("{done}: {edge} fell off the canvas (undo brings it back)"), Level::Warn),
            None => self.flash(done, Level::Info),
        }
    }

    fn end_typing(&mut self) {
        if self.typing {
            self.typing = false;
            self.tab_mut().history.end_group();
        }
    }

    fn type_key(&mut self, k: KeyEvent) {
        let (w, h) = (self.tab().doc.width(), self.tab().doc.height());
        match k.code {
            KeyCode::Char(c) if !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => self.type_char(c),
            KeyCode::Enter => {
                self.end_typing();
                let t = self.tab_mut();
                let last = t.cursor.1 + 1 >= h;
                t.cursor = (t.text_home_x, (t.cursor.1 + 1).min(h - 1));
                if last {
                    self.flash(format!("last row: make the canvas taller with the {w}x{h} chip below"), Level::Warn);
                }
            }
            KeyCode::Backspace | KeyCode::Delete if self.layer_blocked() => {}
            KeyCode::Backspace => {
                let ctx = self.tools.ctx(self.tab(), Button::Left);
                let typing = self.typing;
                let t = self.tab_mut();
                if t.cursor.0 > 0 {
                    t.cursor.0 -= 1;
                    let (x, y) = t.cursor;
                    let layer = t.layer;
                    let blank = Cell::new(' ', ctx.brush.fg, ctx.brush.bg);
                    t.edit("Backspace", |b| b.set(layer, x, y, if layer == 0 { Some(blank) } else { None }));
                    if typing {
                        t.type_run_moved();
                    }
                }
            }
            KeyCode::Delete => {
                let t = self.tab_mut();
                let (x, y) = t.cursor;
                let layer = t.layer;
                t.edit("Delete", |b| tools::delete_block(b, layer, CellRect::new(x, y, 1, 1)));
            }
            KeyCode::Insert => {
                let t = self.tab_mut();
                t.insert_mode = !t.insert_mode;
                let m = if t.insert_mode { "insert" } else { "overwrite" };
                self.flash(format!("text: {m} mode"), Level::Info);
            }
            KeyCode::Esc => {
                let back = if self.tools.prev_tool == Tool::Text { Tool::Brush } else { self.tools.prev_tool };
                self.tools.set_tool(back);
                self.flash(format!("{} tool", back.name()), Level::Info);
            }
            _ => {}
        }
    }

    pub fn type_char(&mut self, c: char) {
        if self.layer_blocked() {
            return;
        }
        let ctx = self.tools.ctx(self.tab(), Button::Left);
        if !self.typing {
            self.typing = true;
            self.tab_mut().history.begin_group();
            self.tab_mut().type_run_start();
        }
        let t = self.tab_mut();
        let (x, y) = t.cursor;
        let w = t.doc.width();
        let insert = t.insert_mode;
        let layer = t.layer;
        t.edit("Type", |b| {
            if insert && x + 1 < w {
                let rest = tools::copy(b, Some(layer), CellRect::new(x, y, w - x - 1, 1));
                tools::stamp(b, layer, &rest, x + 1, y, tools::StampMode::Opaque);
            }
            tools::type_char(b, &ctx, x, y, c);
        });
        if x + 1 < w {
            t.cursor.0 += 1;
        }
        t.type_run_moved();
        if x + 1 >= w {
            // The cursor stays (as in ACiDDraw): say so, or the next letters
            // silently replace this one.
            self.flash(format!("end of the line ({w} columns): the next letter replaces this one"), Level::Warn);
        }
    }

    fn on_paste(&mut self, s: &str) {
        if !self.dialogs.is_empty() {
            if let Some(d) = self.dialogs.last_mut() {
                d.paste(s);
            }
            return;
        }
        if self.together_paste(s) {
            return;
        }
        if self.typing_mode() {
            for line in s.lines() {
                for c in line.chars() {
                    self.type_char(c);
                }
                self.type_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
            return;
        }
        let clip = text_clip(s, self.tools.brush.fg, self.tools.brush.bg);
        self.float(clip, FloatSource::Paste);
    }

    // ----------------------------------------------------------- mouse

    fn on_mouse(&mut self, mut m: MouseEvent) {
        // Pixel reports → cell + position within the cell.
        if let Some((cw, ch)) = self.cell_px {
            let (fx, fy) = (m.column as f32 / cw, m.row as f32 / ch);
            m.column = fx.floor() as u16;
            m.row = fy.floor() as u16;
            self.mouse_frac = (fx.fract(), fy.fract());
        }
        if self.playback.is_some() {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                self.playback = None;
            }
            return;
        }
        if !self.dialogs.is_empty() {
            // A click beside the dialog dismisses it, as Esc would: with one
            // mouse button that is the way out of a dialog without buttons.
            let outside = matches!(m.kind, MouseEventKind::Down(_))
                && self.dialog_area.is_some_and(|(n, r)| n == self.dialogs.len() && !contains(r, (m.column, m.row)));
            if outside {
                self.with_top_dialog(|d, app| d.click_outside(app));
            } else {
                self.with_top_dialog(|d, app| d.mouse(m, app));
            }
            return;
        }
        let (w, h) = (self.tab().doc.width(), self.tab().doc.height());
        let pos = (m.column, m.row);
        let button = |b: MouseButton| match b {
            MouseButton::Right => Some(Button::Right),
            MouseButton::Left => Some(Button::Left),
            MouseButton::Middle => None,
        };
        match m.kind {
            MouseEventKind::Down(b) => {
                self.end_typing();
                self.together.focus = false;
                if let Some(&(_, hit)) = self.sidebar_hits.iter().find(|(r, _)| contains(*r, pos)) {
                    // Right-click, or Ctrl/Alt-click for one-button trackpads.
                    let alt =
                        b == MouseButton::Right || m.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                    if hit.draggable() {
                        self.side_drag = Some(hit);
                    }
                    self.sidebar_click(hit, alt, pos);
                    return;
                }
                if self.replay_on() {
                    // The canvas shows the replay: a click plays / pauses it.
                    if self.geom.to_cell(m.column, m.row, w, h).is_some() {
                        self.toggle_replay_play();
                    }
                    return;
                }
                let Some(mut btn) = button(b) else { return };
                // One-button mice/trackpads: Option- or Ctrl-drag erases like a right-drag.
                if btn == Button::Left && m.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) {
                    btn = Button::Right;
                }
                self.key_cursor = false;
                if let Some((x, y, py)) = self.geom.to_cell(m.column, m.row, w, h) {
                    // A playing animation changes frame under the pointer: the
                    // click stops it on the frame shown, and paints nothing.
                    if self.tab_mut().playing.take().is_some() {
                        self.flash("stopped playing on this frame", Level::Info);
                        return;
                    }
                    // Dragging a selection from inside it moves the cells.
                    let moves = self.tools.tool == Tool::Select
                        && self.tab().selection.is_some_and(|s| s.contains(x, y));
                    let edits = self.tools.floating.is_some()
                        || moves
                        || !matches!(self.tools.tool, Tool::Select | Tool::Picker | Tool::Font | Tool::Stencil);
                    if edits && self.layer_blocked() {
                        return;
                    }
                    self.mouse_button = Some(btn);
                    let tool = self.tools.tool;
                    let py = self.pixel_row(y, py);
                    self.tools.pixel_full_cells = !self.tab().zoom && self.cell_px.is_none();
                    self.tools.pen_point = self.pen_point(m.column, m.row, x, y);
                    self.tools.pen_smooth = self.cell_px.is_none() && !self.tab().zoom;
                    let (cx, cy) = if tool == Tool::Pixel && self.tools.floating.is_none() { (x, py) } else { (x, y) };
                    if tool != Tool::Pixel {
                        self.tab_mut().cursor = (x, y);
                    }
                    let mut tools = std::mem::take(&mut self.tools);
                    let msg = tools.press(&mut self.tabs[self.active], cx, cy, btn);
                    self.tools = tools;
                    if let Some(s) = msg {
                        self.flash(s, Level::Info);
                    }
                    if matches!(self.tools.tool, Tool::Font | Tool::Stencil) && self.tools.floating.is_none() {
                        self.run(self.tools.tool.action());
                    }
                }
            }
            MouseEventKind::Drag(_) => {
                if let Some(hit) = self.side_drag {
                    self.sidebar_click(hit, false, pos);
                    return;
                }
                if self.mouse_button.is_none() {
                    return;
                }
                let (x, y, py) = self.geom.to_cell_clamped(m.column, m.row, w, h);
                self.hover = Some((x, y));
                self.tools.pen_point = self.pen_point(m.column, m.row, x, y);
                let (cx, cy) = if self.tools.tool == Tool::Pixel && self.tools.floating.is_none() {
                    (x, self.pixel_row(y, py))
                } else {
                    (x, y)
                };
                let mut tools = std::mem::take(&mut self.tools);
                tools.drag_to(&mut self.tabs[self.active], cx, cy);
                self.tools = tools;
            }
            MouseEventKind::Up(_) => {
                self.side_drag = None;
                if self.mouse_button.take().is_none() {
                    return;
                }
                let (x, y, py) = self.geom.to_cell_clamped(m.column, m.row, w, h);
                let (cx, cy) = if self.tools.tool == Tool::Pixel && self.tools.floating.is_none() {
                    (x, self.pixel_row(y, py))
                } else {
                    (x, y)
                };
                let mut tools = std::mem::take(&mut self.tools);
                let msg = tools.release(&mut self.tabs[self.active], cx, cy);
                self.tools = tools;
                if let Some(s) = msg {
                    self.flash(s, Level::Info);
                }
            }
            MouseEventKind::Moved => {
                let hover = self.sidebar_hits.iter().find(|(r, _)| contains(*r, pos)).map(|(_, h)| *h);
                if hover != self.side_hover {
                    self.side_hover = hover;
                    self.side_hover_at = Instant::now();
                }
                self.hover = self.geom.to_cell(m.column, m.row, w, h).map(|(x, y, _)| (x, y));
                if let Some((x, y)) = self.hover {
                    self.tools.hover(x, y);
                }
            }
            MouseEventKind::ScrollDown
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {
                let horizontal = m.modifiers.contains(KeyModifiers::SHIFT)
                    || matches!(m.kind, MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight);
                let fwd = matches!(m.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollRight);
                let (vw, vh) = (self.geom.cols, self.geom.rows);
                let t = self.tab_mut();
                if horizontal {
                    let max = w.saturating_sub(vw);
                    t.scroll.0 = if fwd { (t.scroll.0 + 4).min(max) } else { t.scroll.0.saturating_sub(4) };
                } else {
                    let max = h.saturating_sub(vh);
                    t.scroll.1 = if fwd { (t.scroll.1 + 3).min(max) } else { t.scroll.1.saturating_sub(3) };
                }
            }
        }
    }

    fn sidebar_click(&mut self, hit: Hit, alt: bool, pos: (u16, u16)) {
        // Replay is a view: anything but its own panel (and views) goes back to drawing.
        let view = matches!(hit, Hit::Replay(_) | Hit::Act(_) | Hit::Minimap | Hit::Unfold(_) | Hit::Message);
        if self.replay_on() && !view {
            self.replay = None;
        }
        match hit {
            Hit::Replay(h) => {
                let r = self.sidebar_hits.iter().find(|(_, x)| *x == hit).map(|&(r, _)| r).unwrap_or_default();
                self.replay_click(h, r.x, r.width, pos.0);
            }
            Hit::ArtKey(k) if k == acidtrip_io::artboard::ERASE_KEY => {
                self.flash(format!("{} erases (so does typing a glyph onto itself)", k.to_ascii_uppercase()), Level::Info)
            }
            Hit::ArtKey(k) => self.dialogs.push(Box::new(dialogs::chars::CharPicker::assign(self, k))),
            Hit::ArtSet(step) => self.cycle_art_set(step),
            Hit::Recent(i) if i == self.recent.sel => self.view_recent(false),
            Hit::Recent(i) => self.recent.select(i),
            Hit::RecentStep(by) => self.recent.step(by),
            Hit::TakePart => self.view_recent(true),
            Hit::RecentStudio => self.studio_recent(),
            Hit::PatternStep(step) => self.run(if step < 0 { Action::PatternPrev } else { Action::PatternNext }),
            Hit::Tool(t) => self.select_tool(t),
            Hit::Color(i) => {
                // Paint-style: plain click sets the active slot, right-click the other.
                let to_bg = (self.color_slot == Slot::Bg) != alt;
                if to_bg { self.set_bg(i) } else { self.tools.brush.fg = Color::Pal(i) }
            }
            Hit::Slot(slot) => {
                if self.color_slot == slot {
                    self.run(Action::ColorDialog);
                } else {
                    self.color_slot = slot;
                    let name = if slot == Slot::Fg { "foreground" } else { "background" };
                    self.flash(format!("palette clicks now set the {name}"), Level::Info);
                }
            }
            Hit::Swap => self.run(Action::SwapColors),
            Hit::Glyph(i) => {
                let ch = self.tools.glyph(i);
                self.pick_glyph(ch);
            }
            Hit::CharsetPrev => self.run(Action::CharsetPrev),
            Hit::CharsetNext => self.run(Action::CharsetNext),
            Hit::Layer(i) => self.tab_mut().layer = i,
            Hit::Frame(i) => {
                let t = self.tab_mut();
                t.playing = None;
                t.show_frame(i);
            }
            Hit::LayerEye(i) => {
                let t = self.tab_mut();
                let vis = t.doc.canvas.layers[i].visible;
                t.edit("Toggle layer", |b| {
                    tools::set_layer_props(b, i, &tools::LayerProps { visible: Some(!vis), ..Default::default() })
                });
            }
            Hit::LayerOp(op) => self.run(match op {
                LayerOp::Add => Action::LayerAdd,
                LayerOp::Duplicate => Action::LayerDuplicate,
                LayerOp::Remove => Action::LayerRemove,
                LayerOp::Up => Action::LayerMoveUp,
                LayerOp::Down => Action::LayerMoveDown,
                LayerOp::Merge => Action::LayerMerge,
                LayerOp::Rename => Action::LayerRename,
                LayerOp::Panel => Action::LayersPanel,
            }),
            Hit::Opt(o) => self.set_option(o, pos),
            // The Select panel's dimmed ⋯ chip: Enter's no-selection
            // fallback (start typing) would be a surprise from a click.
            Hit::Act(Action::BlockMenu) if self.tab().selection.is_none() && self.tools.floating.is_none() => {
                self.flash("select an area first (V, drag)", Level::Warn)
            }
            Hit::Act(a) => self.run(a),
            Hit::Unfold(f) => self.side_open = Some(f),
            Hit::Message => self.run(Action::Messages),
            Hit::Minimap => self.minimap_jump(pos),
            Hit::UsedColor(c) => {
                self.tools.fx.recolor.from = Some(c);
                let n = crate::ui::sidebar::color_label(c);
                self.flash(format!("Recolor: replace {n} · Enter applies"), Level::Info);
            }
            Hit::FxFrom => self.flash("Recolor: click a color on the canvas to replace it", Level::Info),
            Hit::Export(h) => self.export_click(h, alt),
        }
    }

    /// Where along a sidebar slider the mouse is, 0..=1 (the knob sits on
    /// cells 0..w-1, so the ends are easy to hit).
    fn slider_frac(&self, hit: Hit, pos: (u16, u16)) -> Option<f32> {
        let &(r, _) = self.sidebar_hits.iter().find(|(_, h)| *h == hit)?;
        let at = pos.0.saturating_sub(r.x) as f32 + self.mouse_frac.0 - 0.5;
        Some((at / r.width.saturating_sub(1).max(1) as f32).clamp(0.0, 1.0))
    }

    /// A tool option chip in the sidebar.
    fn set_option(&mut self, o: Opt, pos: (u16, u16)) {
        let classic = self.tab().doc.is_classic();
        let ts = &mut self.tools;
        let msg = match o {
            Opt::Paint(m) => {
                ts.opts.brush_mode = m;
                tools_ctl::paint_mode_name(m).to_string()
            }
            Opt::Match(m) => {
                ts.opts.fill_mode = m;
                m.name().to_string()
            }
            Opt::Shape(f) => {
                if ts.tool == Tool::Ellipse {
                    ts.opts.ellipse_fill = f
                } else {
                    ts.opts.rect_fill = f
                }
                tools_ctl::fill_name(f).to_string()
            }
            Opt::Look(ch) => {
                // The "brush glyph" look keeps a glyph that's already one.
                if !tools_ctl::look_of(ch).active(ts.brush.ch) {
                    ts.brush.ch = ch;
                }
                tools_ctl::look_of(ch).name.to_string()
            }
            Opt::Stamp(s) => {
                ts.opts.stamp_mode = s;
                tools_ctl::stamp_name(s).to_string()
            }
            Opt::PixelFill(f) => {
                ts.opts.pixel_fill = f;
                ts.option_summary()
            }
            Opt::Lighter(l) => {
                ts.opts.shade_lighter = l;
                ts.option_summary()
            }
            Opt::Insert(i) => {
                self.tab_mut().insert_mode = i;
                (if i { "insert" } else { "overwrite" }).to_string()
            }
            Opt::Mirror(s) => {
                ts.opts.symmetry = s;
                format!("mirror: {}", tools_ctl::symmetry_name(s))
            }
            Opt::Brush(d) => ts.cycle_brush(d),
            Opt::PatternMode(m) => {
                ts.opts.pattern_mode = m;
                ts.option_summary()
            }
            Opt::PatternSize(d) => ts.resize_pattern(d),
            Opt::PatternAnchor(a) => {
                ts.opts.pattern_anchor = a;
                match a {
                    tools_ctl::PatternAnchor::Canvas => "tiles line up on the canvas grid".into(),
                    tools_ctl::PatternAnchor::Start => "tiles start where you press".into(),
                }
            }
            Opt::PatternRecolor(r) => {
                ts.opts.pattern_recolor = r;
                (if r { "colors: brush" } else { "colors: its own" }).into()
            }
            Opt::Size => {
                let Some(&(r, _)) = self.sidebar_hits.iter().find(|(_, h)| *h == Hit::Opt(Opt::Size)) else {
                    return;
                };
                let frac = (pos.0.saturating_sub(r.x) as f32 + self.mouse_frac.0) / r.width.max(1) as f32;
                acidtrip_core::tools::brush::Param::Size.set_fraction(&mut self.tools.pen_brush, frac.clamp(0.0, 1.0));
                self.tools.option_summary()
            }
            Opt::Preset(d) => ts.fx.step_preset(d),
            Opt::Knob(i) => {
                let Some(frac) = self.slider_frac(Hit::Opt(o), pos) else { return };
                let k = acidtrip_core::filters::Knob::ALL[i % acidtrip_core::filters::Knob::ALL.len()];
                let (lo, hi) = k.range();
                let mut v = lo + (frac * (hi - lo) as f32).round() as i32;
                // Bipolar sliders snap to their middle.
                if lo < 0 && v.abs() <= 4 {
                    v = 0;
                }
                let f = &mut self.tools.fx.filter.filter;
                f.set(k, v);
                format!("{} {}", k.name(), f.get(k))
            }
            Opt::AllLayers(all) => {
                match ts.fx_kind() {
                    Some(crate::colorfx::Kind::Recolor) => ts.fx.recolor.all_layers = all,
                    _ => ts.fx.filter.all_layers = all,
                }
                (if all { "all layers" } else { "this layer" }).to_string()
            }
            Opt::Rerender(r) => {
                ts.fx.filter.rerender = r;
                (if r { "re-render glyphs" } else { "nearest colors" }).to_string()
            }
            Opt::Target(t) => {
                ts.fx.recolor.target = t;
                format!("replace in {}", t.name())
            }
            Opt::Tolerance => {
                let Some(frac) = self.slider_frac(Hit::Opt(o), pos) else { return };
                self.tools.fx.recolor.tolerance = (frac * 100.0).round() as u8;
                self.tools.option_summary()
            }
            Opt::UsedPage(d) => {
                let used = ts.fx.used_colors(&self.tabs[self.active]).len();
                let pages = used.div_ceil(crate::ui::sidebar::USED_PER_PAGE).max(1) as i64;
                let r = &mut ts.fx.recolor;
                r.page = (r.page as i64 + d as i64).rem_euclid(pages) as usize;
                format!("colors page {}/{pages}", ts.fx.recolor.page + 1)
            }
            Opt::GradShape(s) => {
                ts.opts.gradient.shape = s;
                ts.option_summary()
            }
            Opt::GradStyle(acidtrip_io::gradient::Style::Smooth) if classic => {
                self.flash("smooth needs a Modern document — Classic has 16 colors (try dither)", Level::Warn);
                return;
            }
            Opt::GradStyle(s) => {
                ts.opts.gradient.style = s;
                ts.option_summary()
            }
            Opt::GradRamp(r) => {
                ts.opts.gradient.ramp = r;
                ts.option_summary()
            }
            Opt::GradReverse => {
                ts.opts.gradient.reverse = !ts.opts.gradient.reverse;
                ts.option_summary()
            }
        };
        let name = self.tools.tool.name();
        self.flash(format!("{name}: {msg}"), Level::Info);
    }

    fn minimap_jump(&mut self, pos: (u16, u16)) {
        let Some(g) = self.minimap_rect else { return };
        let r = g.rect;
        let (w, h) = (self.tab().doc.width(), self.tab().doc.height());
        let p = (pos.0.clamp(r.x, r.right().saturating_sub(1)), pos.1.clamp(r.y, r.bottom().saturating_sub(1)));
        if let Some((x, y)) = crate::ui::minimap::click_to_cell(g, p.0, p.1, w, h) {
            self.center_on(x, y);
        }
    }

    /// Select a glyph for drawing (sidebar click, char picker): it becomes the
    /// brush, and a non-painting tool switches to the brush so the mouse
    /// paints it right away.
    pub fn pick_glyph(&mut self, ch: char) {
        self.tools.set_brush_char(ch);
        if !matches!(
            self.tools.tool,
            Tool::Brush | Tool::Fill | Tool::Line | Tool::Rect | Tool::Ellipse | Tool::Text | Tool::Art
        ) {
            self.tools.floating = None;
            self.tools.set_tool(Tool::Brush);
        }
        self.flash(format!("brush '{ch}' — drag to paint, or press 1-0 to place at the cursor"), Level::Info);
    }

    fn set_bg(&mut self, i: u8) {
        let t = self.tab();
        let limit = if t.doc.meta.kind == DocKind::Modern || t.doc.meta.ice { 16 } else { 8 };
        if i >= limit {
            self.flash("bright backgrounds need iCE colors (Alt-Z)", Level::Warn);
            return;
        }
        self.tools.brush.bg = Color::Pal(i);
    }

    fn select_tool(&mut self, t: Tool) {
        self.end_typing();
        if t == Tool::Art {
            self.set_art_mode(self.tools.tool != Tool::Art);
            return;
        }
        // The eraser toggles: E again goes back to what you were drawing with.
        if t == Tool::Erase && self.tools.tool == Tool::Erase {
            let back = if self.tools.prev_tool == Tool::Erase { Tool::Brush } else { self.tools.prev_tool };
            self.tools.set_tool(back);
            self.flash(format!("{} tool", back.name()), Level::Info);
            return;
        }
        // Font and Stencil are pickers: their key again reopens the chooser.
        if t == self.tools.tool && self.tools.floating.is_none() && !matches!(t, Tool::Font | Tool::Stencil) {
            let s = self.tools.option_summary();
            let key = self.keymap.key_for(Action::ToolOption).unwrap_or_default();
            self.flash(format!("{} tool · {s} ({key} to change)", t.name()), Level::Info);
            return;
        }
        self.tools.floating = None;
        self.tools.set_tool(t);
        match t {
            Tool::Font => {
                // Fonts saved since (Claude's letters land after the studio
                // closes; files copied in by hand) show without a restart.
                if self.fonts.changed_on_disk() {
                    self.fonts = self.fonts.reload();
                }
                self.dialogs.push(Box::new(dialogs::fonts::FontDialog::new(self)))
            }
            Tool::Stencil => self.dialogs.push(Box::new(dialogs::stencils::StencilDialog::new(self))),
            Tool::Pattern => {
                let s = self.tools.option_summary();
                let k = |a| self.keymap.key_for(a).unwrap_or_default();
                self.flash(
                    format!(
                        "Pattern · {s} · {} next · {} selection as pattern · right-drag erases",
                        k(Action::ToolOption),
                        k(Action::PatternFromSelection)
                    ),
                    Level::Info,
                );
            }
            Tool::Pixel => {
                let how = if self.cell_px.is_some() {
                    "half-cell precise"
                } else {
                    "whole cells — Z zooms for half cells"
                };
                self.flash(format!("Half-block pixels: {how} · Tab: pen/fill"), Level::Info);
            }
            Tool::Pen => {
                let s = self.tools.option_summary();
                let k = |a| self.keymap.key_for(a).unwrap_or_default();
                self.flash(
                    format!(
                        "Pen · {s} · {} next brush · {}/{} size · {} brush studio",
                        k(Action::ToolOption),
                        k(Action::BrushSmaller),
                        k(Action::BrushBigger),
                        k(Action::BrushStudio)
                    ),
                    Level::Info,
                );
            }
            Tool::Filters | Tool::Recolor => {
                let k = |a| self.keymap.key_for(a).unwrap_or_default();
                let what = if t == Tool::Filters {
                    format!("Filters · {} {}/{} presets", self.tools.option_summary(), k(Action::ToolStyle), k(Action::ToolOption))
                } else {
                    "Recolor · click the color to replace; the brush FG replaces it".into()
                };
                let scope = if self.tab().selection.is_some() { "the selection" } else { "the layer" };
                self.flash(format!("{what} · on {scope} · Enter applies"), Level::Info);
            }
            Tool::Text => {
                let t = self.tab_mut();
                t.text_home_x = t.cursor.0;
                self.flash("Text: type away · Esc back to tools · Insert toggles insert mode", Level::Info);
            }
            _ => {
                let s = self.tools.option_summary();
                let key = self.keymap.key_for(Action::ToolOption).unwrap_or_default();
                let hint = if s.is_empty() { String::new() } else { format!(" · {s} ({key} to change)") };
                self.flash(format!("{} tool{hint}", t.name()), Level::Info);
            }
        }
    }

    /// Built-in brushes merged with the user's (a user brush with a
    /// built-in's name replaces it); keeps the current pick by name.
    pub fn reload_brushes(&mut self) {
        let mut all = acidtrip_core::tools::brush::presets();
        let user = acidtrip_io::brushes::load(&self.paths.brushes_dir());
        self.tools.user_brushes = user.iter().map(|b| b.name.clone()).collect();
        for b in user {
            match all.iter_mut().find(|p| p.name == b.name) {
                Some(p) => *p = b,
                None => all.push(b),
            }
        }
        let cur = self.tools.brushes.get(self.tools.brush_idx).map(|b| b.name.clone());
        self.tools.brushes = all;
        let i = cur.and_then(|n| self.tools.brushes.iter().position(|b| b.name == n)).unwrap_or(0);
        self.tools.brush_idx = i.min(self.tools.brushes.len() - 1);
    }

    /// Built-in patterns, then the user's (a saved one with a built-in's
    /// name replaces it); keeps the current pick by name.
    pub fn reload_patterns(&mut self) {
        let mut all = acidtrip_core::tools::pattern::builtin();
        let user = acidtrip_io::patterns::load(&self.paths.patterns_dir());
        let ts = &mut self.tools;
        ts.user_patterns = user.iter().map(|p| p.name.clone()).collect();
        all.retain(|b| !ts.user_patterns.contains(&b.name));
        all.extend(user);
        ts.patterns = all;
        // A pattern from the list is found again by name; a fresh selection
        // (no index) stays as it is.
        if ts.pattern_idx.is_some() {
            match ts.patterns.iter().position(|p| p.name == ts.pattern.name) {
                Some(i) => ts.select_pattern(i),
                None => ts.pattern_idx = None,
            }
        }
    }

    /// The next / previous pattern (switching to the pattern tool).
    fn step_pattern(&mut self, dir: i32) {
        if self.tools.tool != Tool::Pattern {
            self.tools.floating = None;
            self.tools.set_tool(Tool::Pattern);
        }
        let classic = self.tab().doc.meta.kind == DocKind::Classic;
        let s = self.tools.cycle_pattern(dir, classic);
        self.flash(format!("Pattern: {s}"), Level::Info);
    }

    /// Paint with the selected block as a pattern.
    fn pattern_from_selection(&mut self) {
        let Some(r) = self.tab().selection else {
            let k = self.keymap.key_for(Action::ToolSelect).unwrap_or_default();
            self.flash(format!("select an area first ({k}, drag), then use it as a pattern"), Level::Warn);
            return;
        };
        let clip = crate::tools_ctl::copy_clip(self.tab(), None, r);
        let p = acidtrip_core::tools::pattern::Pattern::from_clip(&format!("selection {}x{}", r.w, r.h), &clip);
        if p.is_empty() {
            self.flash("the selection is empty: select some art to repeat", Level::Warn);
            return;
        }
        let cut = if r.w > p.width || r.h > p.height {
            format!(" (cut to {}x{})", p.width, p.height)
        } else {
            String::new()
        };
        // A brush at least as tall as the motif, so one stroke shows all of it.
        let tall = p.height.min(crate::tools_ctl::PATTERN_SIZE_MAX);
        self.tools.opts.pattern_size = self.tools.opts.pattern_size.max(tall);
        self.tools.pattern = p;
        self.tools.pattern_idx = None;
        self.tab_mut().selection = None;
        self.tools.floating = None;
        self.tools.set_tool(Tool::Pattern);
        let k = self.keymap.key_for(Action::PatternSave).unwrap_or_else(|| "★ save".into());
        self.flash(format!("Pattern: painting with the selection{cut} · drag to paint · {k} keeps it"), Level::Ok);
    }

    fn save_pattern(&mut self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.flash("a pattern needs a name", Level::Warn);
            return;
        }
        let mut p = self.tools.pattern.clone();
        p.name = name.clone();
        let same = |n: &String| n.to_lowercase() == name.to_lowercase();
        let note = if self.tools.user_patterns.iter().any(same) {
            " (replaces the one saved before)"
        } else if acidtrip_core::tools::pattern::builtin().iter().any(|b| same(&b.name)) {
            " (in place of the built-in; delete it to get that back)"
        } else {
            ""
        };
        match acidtrip_io::patterns::save(&self.paths.patterns_dir(), &p) {
            Ok(_) => {
                self.tools.pattern = p;
                // Now in the list: reloading picks it by name.
                self.tools.pattern_idx = Some(0);
                self.reload_patterns();
                self.flash(format!("saved pattern {name}{note}"), Level::Ok);
            }
            Err(e) => self.flash(format!("{e:#}"), Level::Error),
        }
    }

    /// Delete a saved pattern from the library.
    pub fn delete_pattern(&mut self, name: String) {
        match acidtrip_io::patterns::delete(&self.paths.patterns_dir(), &name) {
            Ok(()) => {
                self.reload_patterns();
                if self.tools.pattern_idx.is_none() && self.tools.pattern.name == name {
                    self.tools.select_pattern(0);
                }
                self.flash(format!("deleted pattern {name}"), Level::Ok);
            }
            Err(e) => self.flash(format!("{e:#}"), Level::Error),
        }
    }

    fn glyph(&mut self, i: usize) {
        let ch = self.tools.glyph(i);
        if self.typing_mode() {
            self.type_char(ch);
            return;
        }
        self.place_glyph(ch, true);
    }

    /// Art mode on or off.
    /// The art tool is on: the left hand types from the art board.
    pub fn art_mode(&self) -> bool {
        self.tools.tool == Tool::Art
    }

    /// Switch to the art tool, or back to the tool before it.
    pub fn set_art_mode(&mut self, on: bool) {
        if on == self.art_mode() {
            return;
        }
        self.end_typing();
        self.tools.floating = None;
        if on {
            self.tools.set_tool(Tool::Art);
            self.key_cursor = true;
            self.blink_start = Instant::now();
            let set = self.artboard.current(self.classic()).name;
            self.flash(format!("art: the left hand types {set} ([ ] other sets, R erases) · IJKL move · Esc leaves"), Level::Info);
        } else {
            let back = match self.tools.prev_tool {
                Tool::Art | Tool::Text => Tool::Brush,
                t => t,
            };
            self.tools.set_tool(back);
            self.flash(format!("art off · {} tool", back.name()), Level::Info);
        }
    }

    fn classic(&self) -> bool {
        self.tab().doc.meta.kind == acidtrip_core::DocKind::Classic
    }

    /// Rotate the letter keys to the next (or previous) glyph set.
    pub fn cycle_art_set(&mut self, step: i32) {
        let classic = self.classic();
        let name = self.artboard.cycle(step, classic);
        let _ = self.artboard.save(&self.paths.artboard_file());
        self.flash(format!("art keys: {name}"), Level::Info);
    }

    /// A key in art mode. Board keys place their glyph at the cursor (which
    /// stays); I J K L move, Shift draws a trail; U O cycle the foreground,
    /// M , the background. Other letters are swallowed so a stray key can't
    /// switch tools; chords and punctuation go on to the keymap.
    fn art_key(&mut self, k: KeyEvent) -> bool {
        use Action::*;
        if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return false;
        }
        let c = match k.code {
            KeyCode::Esc => {
                self.set_art_mode(false);
                return true;
            }
            KeyCode::Char(c) => c,
            _ => return false,
        };
        let shift = k.modifiers.contains(KeyModifiers::SHIFT) || c.is_ascii_uppercase();
        let lower = c.to_ascii_lowercase();
        let act = match lower {
            'i' => Some(if shift { DrawUp } else { Up }),
            'j' => Some(if shift { DrawLeft } else { Left }),
            'k' => Some(if shift { DrawDown } else { Down }),
            'l' => Some(if shift { DrawRight } else { Right }),
            'u' => Some(FgPrev),
            'o' => Some(FgNext),
            'm' => Some(BgPrev),
            ',' | '<' => Some(BgNext),
            'y' => Some(Undo),
            'h' => Some(Redo),
            _ => None,
        };
        match c {
            '[' | '{' => {
                self.cycle_art_set(-1);
                return true;
            }
            ']' | '}' => {
                self.cycle_art_set(1);
                return true;
            }
            _ => {}
        }
        if let Some(a) = act {
            self.key_cursor = true;
            self.blink_start = Instant::now();
            self.run(a);
            return true;
        }
        if lower == acidtrip_io::artboard::ERASE_KEY || c == ' ' {
            self.art_erase();
            return true;
        }
        if let Some(g) = self.artboard.glyph(lower, self.classic()) {
            // Typing a glyph onto itself takes it away again.
            let t = self.tab();
            let (x, y) = t.cursor;
            let here = t.doc.canvas.get(t.layer, x, y);
            let g_doc = t.doc.conform(Cell::new(g, self.tools.brush.fg, self.tools.brush.bg)).ch;
            if self.key_cursor && here.is_some_and(|h| h.ch == g_doc && !h.is_blank()) {
                self.tools.set_brush_char(g);
                self.art_erase();
            } else {
                self.place_glyph(g, false);
            }
            return true;
        }
        c.is_ascii_alphanumeric()
    }

    /// Erase the cell at the cursor, like the eraser (the cursor stays).
    fn art_erase(&mut self) {
        if !self.key_cursor
            && let Some(h) = self.canvas_hover()
        {
            self.tab_mut().cursor = h;
        }
        if self.layer_blocked() {
            return;
        }
        let mut ctx = self.tools.ctx(self.tab(), Button::Left);
        ctx.mode = PaintMode::Erase;
        ctx.symmetry = Symmetry::None;
        let t = self.tab_mut();
        let (x, y) = t.cursor;
        t.edit("Erase", |b| tools::paint(b, &ctx, x, y));
        self.key_cursor = true;
        self.blink_start = Instant::now();
    }

    /// Put `key` on the art board as `glyph` and remember it.
    pub fn set_art_key(&mut self, key: char, glyph: char) {
        let classic = self.classic();
        self.artboard.set(key, glyph, classic);
        match self.artboard.save(&self.paths.artboard_file()) {
            Ok(()) => self.flash(format!("{} now types {glyph}", key.to_ascii_uppercase()), Level::Ok),
            Err(e) => self.flash(format!("can't save the art keys: {e:#}"), Level::Error),
        }
    }

    /// Put `ch` at the cursor with the brush's colors (moving right when
    /// `advance`), making it the brush.
    fn place_glyph(&mut self, ch: char, advance: bool) {
        // ACiDDraw: the glyph lands at the cursor and the cursor advances. When
        // you've been using the mouse, "the cursor" is where the mouse is.
        if !self.key_cursor
            && let Some(h) = self.canvas_hover()
        {
            self.tab_mut().cursor = h;
        }
        self.tools.set_brush_char(ch);
        if !matches!(self.tools.tool, Tool::Brush | Tool::Fill | Tool::Line | Tool::Rect | Tool::Ellipse | Tool::Art) {
            self.tools.floating = None;
            self.tools.set_tool(Tool::Brush);
        }
        if self.layer_blocked() {
            return;
        }
        let brush = self.tools.brush;
        let t = self.tab_mut();
        let (x, y) = t.cursor;
        let layer = t.layer;
        t.edit("Glyph", |b| b.set(layer, x, y, Some(brush.cell())));
        if advance && x + 1 < t.doc.width() {
            t.cursor.0 += 1;
        }
        self.key_cursor = true;
        self.blink_start = Instant::now();
    }

    /// Open the Library on the selected recent piece: to look at it, or
    /// (`take`) to drag a box over a part of it.
    fn view_recent(&mut self, take: bool) {
        let Some(p) = self.recent.selected().cloned() else {
            self.flash("view a piece in the Gallery first", Level::Info);
            return;
        };
        let d = dialogs::library::LibraryDialog::piece(self, p, take);
        self.dialogs.push(Box::new(d));
    }

    /// The sourcing studio on the selected recent piece.
    fn studio_recent(&mut self) {
        let Some(p) = self.recent.selected() else { return self.run(Action::Harvest) };
        let studio = dialogs::harvest::HarvestDialog::with_source(self, &p.source());
        self.dialogs.push(Box::new(dialogs::library::LibraryDialog::studio(self, Some(studio))));
    }

    /// The canvas cell under the mouse, if it's still on the canvas. `hover`
    /// is set when the mouse moves, so after a crop, resize or undo it can
    /// point past the edge until the mouse moves again.
    pub fn canvas_hover(&self) -> Option<(usize, usize)> {
        let (w, h) = (self.tab().doc.width(), self.tab().doc.height());
        self.hover.filter(|&(x, y)| x < w && y < h)
    }

    pub fn float(&mut self, clip: Clip, source: FloatSource) {
        let (x, y) = self.canvas_hover().unwrap_or(self.tab().cursor);
        if !matches!(self.tools.tool, Tool::Select | Tool::Font | Tool::Stencil) {
            let t = match source {
                FloatSource::Font => Tool::Font,
                FloatSource::Stencil => Tool::Stencil,
                _ => Tool::Select,
            };
            self.tools.set_tool(t);
        }
        self.tools.floating = Some(Floating { clip, x, y, source, lift: None });
        self.flash("click or Space to stamp · Tab: stamp mode · Esc: done", Level::Info);
    }

    // --------------------------------------------------------- actions

    pub fn run(&mut self, a: Action) {
        use Action::*;
        self.replay_before(a);
        // A panel just asked for opens first on a short sidebar, ahead of
        // one opened from its folded title earlier (the options slot holds
        // Replay and Together, so those leave the rest to the default).
        match a {
            Export => self.side_open = Some(crate::ui::sidebar::Fold::Export),
            FramesPanel => self.side_open = Some(crate::ui::sidebar::Fold::Frames),
            Replay | TogetherPanel => self.side_open = None,
            _ => {}
        }
        let (w, h) = (self.tab().doc.width(), self.tab().doc.height());
        match a {
            New => self.dialogs.push(Box::new(dialogs::forms::new_doc_dialog(self))),
            Open => self.dialogs.push(Box::new(dialogs::files::OpenDialog::new())),
            Save => self.save(),
            SaveAs => self.dialogs.push(Box::new(dialogs::export::ExportDialog::save_as(self))),
            Export => self.toggle_export_panel(),
            ExportNow => self.export_now(),
            ExportAs => self.dialogs.push(Box::new(dialogs::export::ExportDialog::export(self))),
            Share => self.dialogs.push(Box::new(dialogs::share::ShareDialog::new())),
            Quit => self.request_quit(),
            Versions => match dialogs::versions::VersionsDialog::new(self) {
                Ok(d) => self.dialogs.push(Box::new(d)),
                Err(e) => self.flash(format!("versions: {e:#}"), Level::Error),
            },
            SnapshotVersion => self.dialogs.push(Box::new(dialogs::prompt::PromptDialog::new(
                "Name this version",
                "",
                Box::new(|app: &mut App, name: String| app.snapshot(&name)),
            ))),
            ImportImage => self.dialogs.push(Box::new(dialogs::import::ImportDialog::new(self, false))),
            ReferenceImage => self.dialogs.push(Box::new(dialogs::import::ImportDialog::new(self, true))),
            Undo => {
                self.end_typing();
                let r = self.tab_mut().undo();
                self.flash(r.map(|l| format!("undo: {l}")).unwrap_or_else(|| "nothing to undo".into()), Level::Info);
            }
            Redo => {
                let r = self.tab_mut().redo();
                self.flash(r.map(|l| format!("redo: {l}")).unwrap_or_else(|| "nothing to redo".into()), Level::Info);
            }
            Copy | Cut | CopyAnsi => self.copy(a),
            MoveSelection => self.lift_selection(),
            Paste => self.paste(),
            SelectAll => {
                self.tools.set_tool(Tool::Select);
                self.tab_mut().selection = Some(CellRect::new(0, 0, w, h));
            }
            Deselect => {
                // Esc first stops a playing animation: it's what's moving.
                if self.tab_mut().playing.take().is_some() {
                    self.flash("stopped playing", Level::Info);
                } else if let Some(f) = self.tools.floating.take() {
                    match f.lift {
                        Some(l) => {
                            self.tab_mut().selection = Some(l.rect);
                            self.flash("move cancelled: the block stays where it was", Level::Info);
                        }
                        None => self.flash("done stamping", Level::Info),
                    }
                } else if self.tools.anchor.take().is_some() {
                    self.flash(format!("{} cancelled", self.tools.tool.name()), Level::Info);
                } else if self.tools.tool == Tool::Filters
                    && self.tools.fx.filter.filter != acidtrip_core::filters::Filter::default()
                {
                    // Esc drops the live preview; nothing was applied.
                    self.tools.fx.filter.filter = acidtrip_core::filters::Filter::default();
                    self.flash("filter preview off: back to Original, nothing applied", Level::Info);
                } else {
                    self.tab_mut().selection = None;
                }
            }
            DeleteSelection => self.with_selection("Erase", tools::erase),
            BlockMenu if self.tools.fx_kind().is_some() && self.tools.floating.is_none() => self.run(ApplyFx),
            ApplyFx => {
                if self.tools.fx_kind().is_none() {
                    self.select_tool(Tool::Filters);
                }
                let mut tools = std::mem::take(&mut self.tools);
                let msg = tools.apply_fx(&mut self.tabs[self.active]);
                self.tools = tools;
                if let Some(s) = msg {
                    self.flash(s, Level::Ok);
                }
            }
            ResetFilter => {
                self.tools.fx.filter.filter = acidtrip_core::filters::Filter::default();
                self.flash("filter reset: Original, sliders at 0", Level::Info);
            }
            BlockMenu => {
                if self.tools.floating.is_some() {
                    // Enter stamps a carried stencil/paste/font, like Space.
                    self.run(Apply);
                } else if self.tab().selection.is_some() {
                    self.dialogs.push(Box::new(dialogs::block::BlockMenu::new(self)));
                } else if self.keymap.preset == Preset::Acid {
                    self.tools.set_tool(Tool::Select);
                    self.tools.anchor = Some(self.tab().cursor);
                    self.flash("block: move the cursor, Space to mark the corner", Level::Info);
                } else {
                    self.select_tool(Tool::Text);
                }
            }
            FlipX => self.transform_selection("Flip X", "flipped {block} left to right", |c| tools::flip_x(c, true)),
            FlipY => self.transform_selection("Flip Y", "flipped {block} upside down", |c| tools::flip_y(c, true)),
            Rotate180 => self.transform_selection("Rotate", "rotated {block} 180°", tools::rotate_180),
            FillSelection => {
                let ctx = self.tools.ctx(self.tab(), Button::Left);
                self.with_selection("Fill", |b, _, r| tools::fill_rect(b, &ctx, r, tools::FillWhat::All));
            }
            OutlineSelection => {
                let ctx = self.tools.ctx(self.tab(), Button::Left);
                let style = self.tools.opts.box_style;
                self.with_selection("Outline", |b, _, r| tools::outline(b, &ctx, r, style));
            }
            JustifyLeft => self.with_selection("Justify", |b, l, r| tools::justify(b, l, r, Justify::Left)),
            JustifyCenter => self.with_selection("Justify", |b, l, r| tools::justify(b, l, r, Justify::Center)),
            JustifyRight => self.with_selection("Justify", |b, l, r| tools::justify(b, l, r, Justify::Right)),
            DeleteBlock => self.with_selection("Delete block", tools::delete_block),
            CropToSelection => {
                if let Some(r) = self.tab().selection {
                    self.tab_mut().edit("Crop", |b| tools::crop(b, r));
                    self.tab_mut().selection = None;
                    self.flash(format!("cropped to {}x{} (Ctrl-Z to undo)", r.w, r.h), Level::Info);
                } else {
                    self.flash("select an area first (V, drag)", Level::Warn);
                }
            }
            SaveStencil => {
                if let Some(r) = self.tab().selection {
                    let clip = crate::tools_ctl::copy_clip(self.tab(), None, r);
                    self.dialogs.push(Box::new(dialogs::prompt::PromptDialog::new(
                        "Stencil name",
                        "",
                        Box::new(move |app: &mut App, name: String| app.save_stencil(clip, name)),
                    )));
                } else {
                    self.flash("select an area first (V, drag)", Level::Warn);
                }
            }
            InsertLine => {
                let y = self.tab().cursor.1;
                let lost = self.has_art((0..w).map(|x| (x, h - 1)));
                let done = format!("inserted a line at row {y}");
                self.shift_edit("Insert line", |b| tools::insert_line(b, y), done, lost.then_some("the bottom row"));
            }
            DeleteLine => {
                let y = self.tab().cursor.1;
                self.shift_edit("Delete line", |b| tools::delete_line(b, y), format!("deleted row {y}"), None);
            }
            InsertColumn => {
                let x = self.tab().cursor.0;
                let lost = self.has_art((0..h).map(|y| (w - 1, y)));
                let done = format!("inserted a column at {x}");
                let edge = lost.then_some("the right column");
                self.shift_edit("Insert column", |b| tools::insert_column(b, x), done, edge);
            }
            DeleteColumn => {
                let x = self.tab().cursor.0;
                self.shift_edit("Delete column", |b| tools::delete_column(b, x), format!("deleted column {x}"), None);
            }
            ClearCanvas => {
                self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                    "Clear the current layer?",
                    Box::new(move |app: &mut App| {
                        let t = app.tab_mut();
                        let layer = t.layer;
                        t.edit("Clear", |b| tools::erase(b, layer, CellRect::new(0, 0, w, h)));
                    }),
                )));
            }
            ToolSelect | ToolText | ToolBrush | ToolPen | ToolPixel | ToolLine | ToolRect | ToolEllipse | ToolFill
            | ToolGradient | ToolPicker | ToolFont | ToolStencil | ToolShade | ToolColorize | ToolErase | ToolPattern
            | ToolFilters | ToolRecolor => {
                if let Some(t) = Tool::from_action(a) {
                    self.select_tool(t)
                }
            }
            ToolOption | ToolStyle if self.tools.tool == Tool::Pattern => {
                self.step_pattern(if a == ToolOption { 1 } else { -1 })
            }
            PatternNext | PatternPrev => self.step_pattern(if a == PatternNext { 1 } else { -1 }),
            PatternFromSelection => self.pattern_from_selection(),
            PatternBrowse => {
                if self.tools.tool != Tool::Pattern {
                    self.tools.floating = None;
                    self.tools.set_tool(Tool::Pattern);
                }
                self.dialogs.push(Box::new(dialogs::patterns::PatternDialog::new(self)));
            }
            PatternSave => {
                let p = &self.tools.pattern;
                let name = if p.name.starts_with("selection ") { String::new() } else { p.name.clone() };
                self.dialogs.push(Box::new(dialogs::prompt::PromptDialog::new(
                    "Pattern name",
                    &name,
                    Box::new(move |app: &mut App, name: String| app.save_pattern(name)),
                )));
            }
            PatternDelete => {
                if !self.tools.pattern_saved() {
                    self.flash("only saved patterns can be deleted (built-ins stay)", Level::Warn);
                } else {
                    let name = self.tools.pattern.name.clone();
                    self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                        &format!("Delete the pattern \"{name}\"?"),
                        Box::new(move |app: &mut App| app.delete_pattern(name)),
                    )));
                }
            }
            ToolOption => {
                let mut s = self.tools.cycle_option();
                // Classic has no truecolor: Tab skips the smooth style.
                if self.tools.tool == Tool::Gradient
                    && self.tools.opts.gradient.style == acidtrip_io::gradient::Style::Smooth
                    && self.tab().doc.is_classic()
                {
                    s = self.tools.cycle_option();
                }
                self.flash(format!("{}: {s}", self.tools.tool.name()), Level::Info);
            }
            ToolStyle if self.tools.tool == Tool::Pen => {
                let s = self.tools.cycle_brush(-1);
                self.flash(format!("Pen: {s}"), Level::Info);
            }
            ToolStyle => {
                let classic = self.tab().doc.meta.kind == DocKind::Classic;
                let s = self.tools.cycle_style(classic);
                let what = if self.tools.tool.is_shape() { self.tools.tool.name() } else { "box style" };
                self.flash(format!("{what}: {s}"), Level::Info);
            }
            BrushStudio => {
                if self.tools.tool != Tool::Pen {
                    self.tools.floating = None;
                    self.tools.set_tool(Tool::Pen);
                }
                self.dialogs.push(Box::new(dialogs::brushes::BrushStudio::new(self)));
            }
            BrushBigger | BrushSmaller if self.tools.tool == Tool::Pattern => {
                self.tools.opts.pattern_mode = tools_ctl::PatternMode::Brush;
                let s = self.tools.resize_pattern(if a == BrushBigger { 1 } else { -1 });
                self.flash(format!("Pattern: {s}"), Level::Info);
            }
            BrushBigger | BrushSmaller => {
                if self.tools.tool != Tool::Pen {
                    self.select_tool(Tool::Pen);
                }
                let s = self.tools.resize_brush(if a == BrushBigger { 1 } else { -1 });
                self.flash(format!("Pen: {s}"), Level::Info);
            }
            Mirror => {
                let s = self.tools.cycle_mirror();
                self.flash(s, Level::Info);
            }
            FgNext | FgPrev => {
                let i = self.tools.brush.fg.index().unwrap_or(7);
                self.tools.brush.fg = Color::Pal(if a == FgNext { (i + 1) % 16 } else { (i + 15) % 16 });
            }
            BgNext | BgPrev => {
                let limit = if self.tab().doc.meta.kind == DocKind::Modern || self.tab().doc.meta.ice { 16 } else { 8 };
                let i = self.tools.brush.bg.index().unwrap_or(0) % limit;
                self.tools.brush.bg = Color::Pal(if a == BgNext { (i + 1) % limit } else { (i + limit - 1) % limit });
            }
            SwapColors => {
                let b = &mut self.tools.brush;
                std::mem::swap(&mut b.fg, &mut b.bg);
                let t = &self.tabs[self.active];
                if t.doc.meta.kind == DocKind::Classic
                    && !t.doc.meta.ice
                    && let Color::Pal(i) = self.tools.brush.bg
                    && i >= 8
                {
                    self.tools.brush.bg = Color::Pal(i - 8);
                }
            }
            PickUnderCursor => {
                let (x, y) = self.tab().cursor;
                let c = self.tab().doc.canvas.composite(x, y);
                self.tools.brush.fg = c.fg;
                self.tools.brush.bg = c.bg;
                self.flash("picked colors under cursor", Level::Info);
            }
            ColorDialog => self.dialogs.push(Box::new(dialogs::colors::ColorDialog::new(self))),
            CharPicker => self.dialogs.push(Box::new(dialogs::chars::CharPicker::new(self))),
            CharsetNext | CharsetPrev => {
                let n = self.tools.charsets.len();
                self.tools.charset =
                    if a == CharsetNext { (self.tools.charset + 1) % n } else { (self.tools.charset + n - 1) % n };
                let s = &self.tools.charsets[self.tools.charset];
                let chars: String = s.chars.iter().collect();
                self.flash(format!("charset {}: {} {chars}", self.tools.charset + 1, s.name), Level::Info);
            }
            Glyph1 | Glyph2 | Glyph3 | Glyph4 | Glyph5 | Glyph6 | Glyph7 | Glyph8 | Glyph9 | Glyph10 => {
                let i = [Glyph1, Glyph2, Glyph3, Glyph4, Glyph5, Glyph6, Glyph7, Glyph8, Glyph9, Glyph10]
                    .iter()
                    .position(|g| *g == a)
                    .unwrap_or(0);
                self.glyph(i);
            }
            DocProperties => self.dialogs.push(Box::new(dialogs::forms::doc_props_dialog(self))),
            CanvasSize => self.dialogs.push(Box::new(dialogs::canvas_size::CanvasSizeDialog::new(self))),
            Sauce => self.dialogs.push(Box::new(dialogs::forms::sauce_dialog(self))),
            ToggleIce => {
                let on = !self.tab().doc.meta.ice;
                self.tab_mut().edit("iCE colors", |b| tools::set_ice(b, on));
                self.flash(
                    if on { "iCE colors on: 16 backgrounds" } else { "iCE colors off: 8 backgrounds + blink" },
                    Level::Info,
                );
            }
            ConvertModern => {
                self.tab_mut().edit("Convert to Modern", |b| tools::set_kind(b, DocKind::Modern));
                self.flash("Modern document: any Unicode char, 24-bit color (Ctrl-Z undoes)", Level::Ok);
            }
            ConvertClassic => {
                self.tab_mut().edit("Convert to Classic", |b| tools::set_kind(b, DocKind::Classic));
                self.flash("Classic document: CP437 + 16 colors, downsampled (Ctrl-Z undoes)", Level::Ok);
            }
            LayerAdd => {
                let t = self.tab_mut();
                let at = t.layer + 1;
                let n = t.doc.canvas.layers.len() + 1;
                let mut idx = at;
                t.edit("Add layer", |b| idx = tools::add_layer(b, &format!("Layer {n}"), at));
                t.layer = idx;
            }
            LayerDuplicate => {
                let t = self.tab_mut();
                let l = t.layer;
                let mut idx = l;
                t.edit("Duplicate layer", |b| idx = tools::duplicate_layer(b, l));
                t.layer = idx;
            }
            LayersPanel => self.dialogs.push(Box::new(dialogs::layers::LayersDialog::new(self))),
            Minimap => {
                self.show_minimap = !self.show_minimap;
                let on = self.show_minimap;
                self.flash(if on { "minimap on (sidebar)" } else { "minimap off" }, Level::Info);
            }
            LayerRemove => {
                if self.tab().doc.canvas.layers.len() > 1 {
                    let t = self.tab_mut();
                    let l = t.layer;
                    t.edit("Remove layer", |b| tools::remove_layer(b, l));
                } else {
                    self.flash("can't remove the only layer", Level::Warn);
                }
            }
            LayerUp => {
                let t = self.tab_mut();
                t.layer = (t.layer + 1).min(t.doc.canvas.layers.len() - 1);
            }
            LayerDown => {
                let t = self.tab_mut();
                t.layer = t.layer.saturating_sub(1);
            }
            LayerMoveUp | LayerMoveDown => {
                let t = self.tab_mut();
                let n = t.doc.canvas.layers.len();
                let from = t.layer;
                let to = if a == LayerMoveUp { (from + 1).min(n - 1) } else { from.saturating_sub(1) };
                if from != to {
                    t.edit("Move layer", |b| tools::move_layer(b, from, to));
                    t.layer = to;
                } else {
                    let msg = if n == 1 { "only one layer" } else if to == 0 { "already the bottom layer" } else { "already the top layer" };
                    self.flash(msg, Level::Info);
                }
            }
            LayerToggle => {
                let t = self.tab_mut();
                let l = t.layer;
                let vis = t.doc.canvas.layers[l].visible;
                t.edit("Toggle layer", |b| {
                    tools::set_layer_props(b, l, &tools::LayerProps { visible: Some(!vis), ..Default::default() })
                });
            }
            LayerMerge => {
                let t = self.tab_mut();
                let l = t.layer;
                let layers = &t.doc.canvas.layers;
                if l == 0 {
                    self.flash("the bottom layer has nothing below to merge into", Level::Info);
                } else if layers[l - 1].locked {
                    let msg = format!("{} is locked: unlock it to merge into it", layers[l - 1].name);
                    self.flash(msg, Level::Warn);
                } else {
                    t.edit("Merge down", |b| tools::merge_down(b, l));
                    t.layer = l - 1;
                }
            }
            LayerRename => {
                let cur = self.tab().doc.canvas.layers[self.tab().layer].name.clone();
                self.dialogs.push(Box::new(dialogs::prompt::PromptDialog::new(
                    "Layer name",
                    &cur,
                    Box::new(|app: &mut App, name: String| {
                        let t = app.tab_mut();
                        let l = t.layer;
                        t.edit("Rename layer", |b| {
                            tools::set_layer_props(b, l, &tools::LayerProps { name: Some(name), ..Default::default() })
                        });
                    }),
                )));
            }
            FramesPanel => {
                let t = self.tab_mut();
                t.frames_panel = !t.frames_panel;
                if !t.frames_panel {
                    t.playing = None;
                }
            }
            FrameAdd | FrameDuplicate => {
                let t = self.tab_mut();
                t.playing = None;
                let at = t.doc.current_frame() + 1;
                let (canvas, label) = if a == FrameAdd {
                    (t.doc.blank_frame_canvas(), "Add frame")
                } else {
                    (t.doc.canvas.clone(), "Duplicate frame")
                };
                let hold = if a == FrameAdd { 1 } else { t.doc.hold(at - 1) };
                t.edit(label, |b| {
                    b.insert_frame(at, canvas, hold);
                });
                t.show_frame(at);
                t.frames_panel = true;
                let n = t.doc.frame_count();
                self.flash(format!("frame {}/{n}", at + 1), Level::Info);
            }
            FrameRemove => {
                if self.tab().doc.frame_count() > 1 {
                    let t = self.tab_mut();
                    t.playing = None;
                    let cur = t.doc.current_frame();
                    t.edit("Delete frame", |b| b.remove_frame(cur));
                } else {
                    self.flash("can't delete the only frame", Level::Warn);
                }
            }
            FrameNext | FramePrev => {
                let t = self.tab_mut();
                if t.doc.frame_count() < 2 {
                    self.flash("one frame so far: Alt-F adds another", Level::Info);
                } else {
                    t.playing = None;
                    t.step_frame(if a == FrameNext { 1 } else { -1 });
                }
            }
            FrameMoveLeft | FrameMoveRight => {
                let t = self.tab_mut();
                let n = t.doc.frame_count();
                let from = t.doc.current_frame();
                let to = if a == FrameMoveRight { (from + 1).min(n - 1) } else { from.saturating_sub(1) };
                if from != to {
                    t.playing = None;
                    t.edit("Move frame", |b| b.move_frame(from, to));
                    t.show_frame(to);
                }
            }
            FramePlay => {
                let t = self.tab_mut();
                if t.playing.take().is_none() {
                    if t.doc.frame_count() < 2 {
                        self.flash("add a frame first: nothing to play", Level::Info);
                    } else {
                        t.frames_panel = true;
                        t.playing = Some(Instant::now());
                    }
                }
            }
            OnionPrev | OnionNext => {
                let t = self.tab_mut();
                let on = if a == OnionPrev {
                    t.onion.0 = !t.onion.0;
                    t.onion.0
                } else {
                    t.onion.1 = !t.onion.1;
                    t.onion.1
                };
                let which = if a == OnionPrev { "previous" } else { "next" };
                self.flash(format!("onion skin ({which} frame) {}", if on { "on" } else { "off" }), Level::Info);
            }
            FpsUp | FpsDown => {
                let t = self.tab_mut();
                let fps = t.doc.fps();
                let to = if a == FpsUp { fps + 1 } else { fps.saturating_sub(1) }.clamp(1, 60);
                t.edit("Frame rate", |b| b.set_fps(to));
                self.flash(format!("{to} fps"), Level::Info);
            }
            HoldMore | HoldLess => {
                let t = self.tab_mut();
                let cur = t.doc.current_frame();
                let h = t.doc.hold(cur);
                let to = if a == HoldMore { h + 1 } else { h.saturating_sub(1) }.clamp(1, 99);
                t.edit("Frame hold", |b| b.set_hold(cur, to));
                self.flash(format!("frame {} holds {to} tick{}", cur + 1, if to == 1 { "" } else { "s" }), Level::Info);
            }
            Zoom => {
                let t = self.tab_mut();
                t.zoom = !t.zoom;
                let z = t.zoom;
                self.flash(if z { "zoom on: 2x2 per cell, pixel-accurate" } else { "zoom off" }, Level::Info);
            }
            Sidebar => {
                if self.screen_w >= crate::ui::WIDE {
                    self.show_sidebar = !self.show_sidebar;
                } else if self.show_sidebar && self.narrow_sidebar {
                    self.narrow_sidebar = false;
                } else if self.screen_w >= crate::ui::NARROW_SIDEBAR_MIN {
                    (self.show_sidebar, self.narrow_sidebar) = (true, true);
                } else {
                    let (need, w) = (crate::ui::NARROW_SIDEBAR_MIN, self.screen_w);
                    self.flash(format!("the sidebar needs {need} columns; this terminal has {w}"), Level::Warn);
                }
            }
            Grid => self.grid = !self.grid,
            Preview => self.dialogs.push(Box::new(dialogs::preview::PreviewDialog::new(self))),
            PlayBaud => self.playback = Some(dialogs::playback::Playback::new(self.tab(), 14400)),
            Replay => self.set_replay(!self.replay_on()),
            ArtMode => self.set_art_mode(!self.art_mode()),
            DrawUp | DrawDown | DrawLeft | DrawRight => {
                // Keyboard pen (TheDraw's draw mode): paint here, step, paint there.
                let step = match a {
                    DrawUp => Up,
                    DrawDown => Down,
                    DrawLeft => Left,
                    _ => Right,
                };
                if !matches!(self.tools.tool, Tool::Brush | Tool::Erase | Tool::Shade | Tool::Colorize | Tool::Art) {
                    self.tools.floating = None;
                    self.tools.set_tool(Tool::Brush);
                }
                let ctx = self.tools.ctx(self.tab(), Button::Left);
                let from = self.tab().cursor;
                self.move_cursor(step);
                if self.layer_blocked() {
                    return;
                }
                let to = self.tab().cursor;
                self.tab_mut().edit("Draw", |b| {
                    tools::paint(b, &ctx, from.0, from.1);
                    tools::paint(b, &ctx, to.0, to.1);
                });
            }
            Up | Down | Left | Right | PageUp | PageDown | LineStart | LineEnd | FirstChar | LastChar | TabStop => {
                self.move_cursor(a)
            }
            Apply => {
                let edits = self.tools.floating.is_some()
                    || !matches!(self.tools.tool, Tool::Select | Tool::Picker | Tool::Font | Tool::Stencil);
                if edits && self.layer_blocked() {
                    return;
                }
                let mut tools = std::mem::take(&mut self.tools);
                let msg = tools.apply_at_cursor(&mut self.tabs[self.active]);
                self.tools = tools;
                if let Some(s) = msg {
                    self.flash(s, Level::Info);
                }
            }
            AiPrompt => {
                if self.agent.is_some() {
                    // One run at a time; offer the way out instead of a prompt
                    // that can't start (its warning would hide under the spinner).
                    self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                        "The AI is still working. Stop it?",
                        Box::new(|app: &mut App| app.cancel_agent()),
                    )));
                } else if self.config.api_key().is_none() {
                    self.dialogs.push(Box::new(dialogs::ai::AiSetupDialog::new(self)));
                } else {
                    self.dialogs.push(Box::new(dialogs::ai::AiPromptDialog::new(self)));
                }
            }
            AiSetup => self.dialogs.push(Box::new(dialogs::ai::AiSetupDialog::new(self))),
            AiStop => match &self.agent {
                Some(_) => self.cancel_agent(),
                None => self.flash("the AI isn't running", Level::Info),
            },
            Harvest => self.dialogs.push(Box::new(dialogs::library::LibraryDialog::studio(self, None))),
            Gallery => self.dialogs.push(Box::new(dialogs::library::LibraryDialog::gallery(self))),
            HarvestedFonts => {
                let d = dialogs::harvest::HarvestDialog::my_fonts(self);
                self.dialogs.push(Box::new(dialogs::library::LibraryDialog::studio(self, Some(d))))
            }
            GetFonts => self.get_fonts(),
            TogetherPanel | TogetherHost | TogetherJoin | TogetherPasteTicket | TogetherCopyTicket | TogetherLeave => {
                self.together_run(a)
            }
            CommandPalette => self.dialogs.push(Box::new(dialogs::palette::CommandPalette::new(self))),
            Help => self.dialogs.push(Box::new(dialogs::help::HelpDialog::new())),
            Messages => self.dialogs.push(Box::new(dialogs::messages::MessagesDialog::new())),
            Settings => self.open_settings(),
            ReloadConfig => self.reload_config(),
        }
        self.after_action();
    }

    fn after_action(&mut self) {
        let (vw, vh) = (self.geom.cols, self.geom.rows);
        self.tab_mut().follow_cursor(vw, vh);
    }

    fn move_cursor(&mut self, a: Action) {
        use Action::*;
        self.key_cursor = true;
        self.blink_start = Instant::now();
        let t = self.tab_mut();
        let (w, h) = (t.doc.width(), t.doc.height());
        let (x, y) = t.cursor;
        t.cursor = match a {
            Up => (x, y.saturating_sub(1)),
            Down => (x, (y + 1).min(h - 1)),
            Left => (x.saturating_sub(1), y),
            Right => ((x + 1).min(w - 1), y),
            PageUp => (x, y.saturating_sub(20)),
            PageDown => (x, (y + 20).min(h - 1)),
            LineStart => (0, y),
            LineEnd => (w - 1, y),
            FirstChar | LastChar => {
                let row: Vec<Cell> = (0..w).map(|cx| t.doc.canvas.composite(cx, y)).collect();
                let pos = if a == FirstChar {
                    row.iter().position(|c| !c.is_blank())
                } else {
                    row.iter().rposition(|c| !c.is_blank())
                };
                (pos.unwrap_or(0), y)
            }
            TabStop => {
                let next = t.doc.meta.tab_stops.iter().map(|&s| s as usize).find(|&s| s > x).unwrap_or(w - 1);
                (next.min(w - 1), y)
            }
            _ => (x, y),
        };
        if self.tools.tool == Tool::Text && matches!(a, Up | Down | Left | Right | LineStart | LineEnd | FirstChar) {
            self.end_typing();
            let t = self.tab_mut();
            t.text_home_x =
                if matches!(a, Left | Right | LineStart | LineEnd | FirstChar) { t.cursor.0 } else { t.text_home_x };
        }
        let c = self.tab().cursor;
        self.tools.hover(c.0, c.1);
    }

    fn with_selection(&mut self, label: &str, f: impl FnOnce(&mut TxBuilder, usize, CellRect)) {
        let Some(r) = self.tab().selection else {
            self.flash("select an area first (V, drag)", Level::Warn);
            return;
        };
        if self.layer_blocked() {
            return;
        }
        let t = self.tab_mut();
        let layer = t.layer;
        t.edit(label, |b| f(b, layer, r));
    }

    /// Flip or rotate the selected block in place, and say so.
    fn transform_selection(&mut self, label: &str, done: &str, f: impl FnOnce(&Clip) -> Clip) {
        let Some(r) = self.tab().selection else {
            self.flash("select an area first (V, drag)", Level::Warn);
            return;
        };
        if self.layer_blocked() {
            return;
        }
        let t = self.tab_mut();
        let layer = t.layer;
        let clip = crate::tools_ctl::copy_clip(t, Some(layer), r);
        let out = f(&clip);
        t.edit(label, |b| {
            tools::erase(b, layer, r);
            tools::stamp(b, layer, &out, r.x, r.y, tools::StampMode::Opaque);
        });
        let block = format!("the {}x{} block", r.w, r.h);
        self.flash(format!("{} (Ctrl-Z to undo)", done.replace("{block}", &block)), Level::Info);
    }

    fn copy(&mut self, a: Action) {
        let Some(r) = self.tab().selection else {
            self.flash("select an area first (V, drag)", Level::Warn);
            return;
        };
        // Copy takes what you see; Cut takes the active layer's cells only
        // (the ones it erases), so a move never drags lower layers along.
        let cut = a == Action::Cut;
        if cut && self.layer_blocked() {
            return;
        }
        let from = cut.then_some(self.tab().layer);
        let clip = crate::tools_ctl::copy_clip(self.tab(), from, r);
        let doc = Document::from_grid(self.tab().doc.meta.kind, &clip.to_grid());
        let text = if a == Action::CopyAnsi {
            format::save_bytes(&doc, Format::Utf8Ansi, &SaveOptions::default())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default()
        } else {
            (0..clip.height)
                .map(|y| {
                    (0..clip.width)
                        .map(|x| clip.get(x, y).map(|c| c.ch).unwrap_or(' '))
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let sys = crate::share::copy_text(&text);
        self.clipboard = Some(clip);
        if cut {
            let t = self.tab_mut();
            let layer = t.layer;
            t.edit("Cut", |b| tools::erase(b, layer, r));
        }
        let what = if a == Action::CopyAnsi { "as ANSI" } else { "" };
        let did = if cut { "cut" } else { "copied" };
        match sys {
            Ok(()) => self.flash(format!("{did} {}x{} {what}", r.w, r.h), Level::Ok),
            Err(e) => self.flash(format!("{did} internally (system clipboard: {e})"), Level::Warn),
        }
    }

    /// Pick the selection up to carry it. Nothing changes until it's put
    /// down: then the erase and the stamp are one "Move" step, and Esc
    /// leaves the block where it was.
    fn lift_selection(&mut self) {
        let Some(r) = self.tab().selection else {
            self.flash("select an area first (V, drag)", Level::Warn);
            return;
        };
        if self.layer_blocked() {
            return;
        }
        let (doc, layer) = (self.tab().doc.meta.id, self.tab().layer);
        let clip = crate::tools_ctl::copy_clip(self.tab(), Some(layer), r);
        if !matches!(self.tools.tool, Tool::Select | Tool::Font | Tool::Stencil) {
            self.tools.set_tool(Tool::Select);
        }
        let lift = Some(crate::tools_ctl::Lift { doc, layer, rect: r });
        self.tools.floating = Some(Floating { clip, x: r.x, y: r.y, source: FloatSource::Move, lift });
        self.tab_mut().selection = None;
        self.flash(
            format!("carrying the {}x{} block: click or Space to put it down · Esc: leave it", r.w, r.h),
            Level::Info,
        );
    }

    fn paste(&mut self) {
        let clip = match (&self.clipboard, crate::share::paste_text()) {
            (Some(c), _) => Some(c.clone()),
            (None, Some(t)) => Some(text_clip(&t, self.tools.brush.fg, self.tools.brush.bg)),
            _ => None,
        };
        match clip {
            Some(c) => self.float(c, FloatSource::Paste),
            None => self.flash("clipboard is empty", Level::Warn),
        }
    }

    fn save_stencil(&mut self, clip: Clip, name: String) {
        let meta = acidtrip_io::stencils::StencilMeta {
            name: if name.trim().is_empty() { "untitled".into() } else { name },
            author: self.tab().doc.meta.sauce.author.clone(),
            group: self.tab().doc.meta.sauce.group.clone(),
            source: self.tab().file.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "drawn".into()),
            license: "own work".into(),
            ..Default::default()
        };
        let dir = self.paths.stencils_dir();
        match self.stencils.save(&dir, acidtrip_io::stencils::Stencil { meta, clip }) {
            Ok(m) => self.flash(format!("saved stencil \"{}\" (N to stamp)", m.name), Level::Ok),
            Err(e) => self.flash(format!("stencil: {e:#}"), Level::Error),
        }
    }

    // ------------------------------------------------------------- files

    /// Open a document already read from `p` (asking first about unsaved work).
    pub fn open_loaded(&mut self, p: &Path, doc: Document, log: Option<acidtrip_core::replay::EditLog>) {
        if self.tab().file.as_deref() == Some(p) {
            self.flash(format!("{} is already open", p.display()), Level::Info);
            return;
        }
        let (w, h) = (doc.width(), doc.height());
        let msg = format!("opened {} ({w}x{h})", p.display());
        self.replace_doc(Tab::new(doc, Some(p.to_path_buf())).with_log(log), msg);
    }

    /// Replace the open document (asking first if it has unsaved changes).
    pub fn replace_doc(&mut self, tab: Tab, msg: String) {
        let swap = move |app: &mut App| {
            let old = app.tab().doc.meta.id;
            recovery::clear(&app.paths.recovery_dir(), old);
            app.tabs = vec![tab];
            app.active = 0;
            app.tools.floating = None;
            app.tools.anchor = None;
            app.flash(msg, Level::Ok);
        };
        if self.tab().dirty() {
            let name = self.tab().title();
            self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                &format!("{name} has unsaved changes. Discard them? (N, then Ctrl-S to save first)"),
                Box::new(swap),
            )));
        } else {
            swap(self);
        }
    }

    pub fn save(&mut self) {
        match (self.tab().file.clone(), self.tab().format) {
            (Some(p), Some(f)) if f.can_save() => {
                let warnings = format::loss_warnings(&self.tab().doc, f);
                if warnings.is_empty() {
                    self.save_to(&p, f, &SaveOptions::default());
                } else {
                    let msg = format!(
                        "Saving as {} will lose: {}.\nSave anyway? (.acid keeps everything)",
                        f.name(),
                        warnings.join("; ")
                    );
                    self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                        &msg,
                        Box::new(move |app: &mut App| app.save_to(&p, f, &SaveOptions::default())),
                    )));
                }
            }
            _ => self.run(Action::SaveAs),
        }
    }

    pub fn save_to(&mut self, path: &Path, fmt: Format, opts: &SaveOptions) {
        if let Err(e) = backup::backup_before_save(path, self.config.backup) {
            self.flash(format!("backup failed: {e:#}"), Level::Warn);
        }
        let mut doc = self.tab().doc.clone();
        if doc.meta.sauce.date.is_empty() {
            doc.meta.sauce.date = chrono::Local::now().format("%Y%m%d").to_string();
        }
        match format::save_with_log(&doc, Some(self.tab().history.log()), path, fmt, opts) {
            Ok(()) => {
                let t = self.tab_mut();
                let is_doc_format = fmt.reopens();
                if is_doc_format {
                    t.file = Some(path.to_path_buf());
                    t.format = Some(fmt);
                    t.history.mark_saved();
                }
                let id = t.doc.meta.id;
                if is_doc_format {
                    t.recovery_written = false;
                    recovery::clear(&self.paths.recovery_dir(), id);
                }
                self.snapshot("save");
                self.flash(format!("saved {}", path.display()), Level::Ok);
            }
            Err(e) => self.flash(format!("save failed: {e:#}"), Level::Error),
        }
    }

    pub fn snapshot(&mut self, label: &str) {
        let dir = self.paths.versions_dir();
        let t = &mut self.tabs[self.active];
        match versions::VersionStore::open(&dir, versions::store_id(&t.doc, t.file.as_deref()))
            .and_then(|mut s| s.snapshot(&t.doc, label, t.file.as_deref()))
        {
            Ok(_) => {
                t.versioned_rev = t.history.revision();
                t.last_version_at = Instant::now();
                if label != "save" && label != "auto" {
                    self.flash(format!("version \"{label}\" saved"), Level::Ok);
                }
            }
            Err(e) => self.flash(format!("version snapshot failed: {e:#}"), Level::Warn),
        }
    }

    fn request_quit(&mut self) {
        let dirty: Vec<String> = self.tabs.iter().filter(|t| t.dirty()).map(|t| t.title()).collect();
        if dirty.is_empty() {
            // Clears recovery files left from before an undo back to clean.
            self.autosave(true);
            self.quit = true;
        } else {
            self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                &format!("Unsaved: {}. Quit anyway? (recoverable next start)", dirty.join(", ")),
                Box::new(|app: &mut App| {
                    app.autosave(true);
                    app.quit = true;
                }),
            )));
        }
    }

    fn open_settings(&mut self) {
        let path = self.paths.config_file();
        let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
        let status =
            crate::term::suspend(self.enhanced_keys, || std::process::Command::new(&editor).arg(&path).status());
        self.repaint = true;
        match status {
            Ok(_) => self.reload_config(),
            Err(e) => self.flash(format!("can't run {editor}: {e} — settings file: {}", path.display()), Level::Warn),
        }
    }

    fn reload_config(&mut self) {
        match Config::load_or_create(&self.paths.config_file()) {
            Ok(c) => {
                let (km, errs) = Keymap::from_config(&c.keymap.preset, &c.keymap.bindings);
                self.keymap = km;
                self.config = c;
                match notes_line(&errs) {
                    Some(e) => self.flash(e, Level::Warn),
                    None => self.flash("settings reloaded", Level::Ok),
                }
            }
            Err(e) => self.flash(format!("{}, settings unchanged", config_error(&e)), Level::Error),
        }
    }

    fn get_fonts(&mut self) {
        let dir = self.paths.fonts_dir();
        if std::env::var_os("ACIDTRIP_OFFLINE").is_some_and(|v| !v.is_empty()) {
            let msg = format!("offline: no downloads — `acidtrip fonts install FILE` or copy fonts into {}", dir.display());
            self.flash(msg, Level::Warn);
            return;
        }
        self.flash("downloading TheDraw fonts…", Level::Info);
        match acidtrip_io::fonts::download_packs(&dir) {
            Ok(n) => {
                self.fonts = FontLibrary::load(Some(&dir));
                self.flash(
                    format!("installed {n} font files ({} fonts total) — F to use", self.fonts.list().len()),
                    Level::Ok,
                );
            }
            Err(e) => self.flash(format!("font download failed: {e:#}"), Level::Error),
        }
    }

    // --------------------------------------------------------------- AI

    /// Index of the layer AI edits go to (created on demand).
    fn ai_layer(&mut self, tab: usize) -> usize {
        if !self.ai_own_layer {
            return self.tabs[tab].layer;
        }
        let t = &mut self.tabs[tab];
        if let Some(i) = t.doc.canvas.layers.iter().position(|l| l.name == "AI") {
            return i;
        }
        let at = t.doc.canvas.layers.len();
        let mut idx = at;
        t.edit("Add AI layer", |b| idx = tools::add_layer(b, "AI", at));
        idx
    }

    pub fn start_agent(&mut self, prompt: String) {
        let Some(key) = self.config.api_key() else {
            self.dialogs.push(Box::new(dialogs::ai::AiSetupDialog::new(self)));
            return;
        };
        let t = self.tab();
        let sel = t
            .selection
            .map(|r| {
                format!(
                    "The user's selection is x={} y={} w={} h={}; keep edits inside it unless asked otherwise.",
                    r.x, r.y, r.w, r.h
                )
            })
            .unwrap_or_default();
        let context = format!(
            "Document: {}x{} {:?}, iCE {}. Brush fg {:?} bg {:?}. Cursor at {:?}. {sel}",
            t.doc.width(),
            t.doc.height(),
            t.doc.meta.kind,
            if t.doc.meta.ice { "on" } else { "off" },
            self.tools.brush.fg,
            self.tools.brush.bg,
            t.cursor
        );
        let cfg = AgentConfig {
            api_key: key,
            model: self.config.ai.model.clone(),
            max_rounds: self.config.ai.max_tool_rounds,
        };
        let (etx, erx) = mpsc::channel();
        let (ttx, trx) = mpsc::channel();
        let doc_id = t.doc.meta.id;
        acidtrip_ai::agent::spawn(cfg, prompt.clone(), context, ttx, etx);
        self.tab_mut().history.begin_group();
        self.agent = Some(AgentRun {
            events: erx,
            tools: trx,
            status: format!("AI: {prompt}"),
            doc_id,
            started: Instant::now(),
        });
    }

    /// Stop the AI run now, even mid-request: the run ends here, and its
    /// thread gives up at its next tool call (its channel is gone).
    pub fn cancel_agent(&mut self) {
        if self.agent.is_some() {
            self.end_agent(true, None);
        }
    }

    fn end_agent(&mut self, stopped: bool, err: Option<String>) {
        let a = self.agent.take();
        if let Some(i) = a.and_then(|a| self.tabs.iter().position(|t| t.doc.meta.id == a.doc_id)) {
            self.tabs[i].history.end_group();
        }
        match err {
            _ if stopped => self.flash("AI stopped — Ctrl-Z undoes what it drew", Level::Warn),
            None => self.flash("AI done — Ctrl-Z undoes the whole run", Level::Ok),
            Some(e) => self.flash(format!("AI error: {e}"), Level::Error),
        }
    }

    fn handle_tool_request(&mut self, req: ToolRequest) {
        if req.origin == "ai" && self.agent.is_none() {
            let _ = req.reply.send(ToolResult::cancelled());
            return;
        }
        // AI runs edit the doc they started on; MCP edits the active tab.
        let tab_idx = match (&self.agent, req.origin.as_str()) {
            (Some(a), "ai") => self.tabs.iter().position(|t| t.doc.meta.id == a.doc_id).unwrap_or(self.active),
            _ => self.active,
        };
        let layer = self.ai_layer(tab_idx);
        let dir = self.paths.clone();
        let t = &mut self.tabs[tab_idx];
        let mut file = t.file.clone();
        let result = {
            let mut st = ExecState {
                doc: &mut t.doc,
                history: &mut t.history,
                layer,
                fonts: &self.fonts,
                stencils: &mut self.stencils,
                paths: &dir,
                file: &mut file,
            };
            acidtrip_ai::exec::execute(&mut st, &req.call)
        };
        t.file = file;
        t.clamp();
        if !result.is_error
            && req.call.name != "render_png"
            && req.call.name != "get_canvas"
            && req.call.name != "get_info"
        {
            let msg = format!("{}: {}", req.origin, req.call.name);
            if let Some(a) = &mut self.agent {
                a.status = format!("AI working… {}", req.call.name);
            }
            self.flash(msg, Level::Info);
        }
        let _ = req.reply.send(result);
    }

    // ------------------------------------------------------------ ticks

    /// Background work; returns true if the screen needs a redraw.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(req) = self.tool_rx.try_recv() {
            self.handle_tool_request(req);
            changed = true;
        }
        let ai_calls: Vec<_> = self.agent.iter().flat_map(|a| a.tools.try_iter()).collect();
        for req in ai_calls {
            self.handle_tool_request(req);
            changed = true;
        }
        let mut finished = None;
        if let Some(a) = &mut self.agent {
            while let Ok(ev) = a.events.try_recv() {
                changed = true;
                match ev {
                    AgentEvent::Status(s) => a.status = s,
                    AgentEvent::Text(t) => {
                        a.status =
                            format!("AI: {}", t.lines().last().unwrap_or("").chars().take(120).collect::<String>())
                    }
                    AgentEvent::Done => finished = Some(None),
                    AgentEvent::Error(e) => finished = Some(Some(e)),
                }
            }
        }
        if let Some(err) = finished {
            self.end_agent(false, err);
        }
        if let Some(d) = &self.studio_drawing {
            match d.poll() {
                None => self.studio_drawing = None,
                Some(done) => {
                    for r in done {
                        changed = true;
                        match r {
                            Ok((title, n)) => {
                                self.fonts = self.fonts.reload();
                                self.flash(format!("Claude drew {n} letters for {title} — F to use them"), Level::Ok);
                            }
                            Err(e) => self.flash(format!("drawing letters: {e}"), Level::Error),
                        }
                    }
                }
            }
        }
        if let Some(p) = &self.playback {
            // Follow the reveal on long pieces.
            let (row, vh) = (p.row(), self.geom.rows.max(1));
            let t = &mut self.tabs[self.active];
            if row >= t.scroll.1 + vh {
                t.scroll.1 = row + 1 - vh;
            }
        }
        if let Some(p) = &mut self.playback
            && p.advance()
        {
            self.playback = None;
            changed = true;
        }
        changed |= self.replay_tick();
        changed |= self.tabs[self.active].play_tick(Instant::now());
        if self.last_autosave.elapsed() >= Duration::from_secs(self.config.autosave_seconds.max(1))
            && self.config.autosave_seconds > 0
        {
            self.autosave(false);
        }
        let every = Duration::from_secs(self.config.version_every_minutes * 60);
        if self.config.version_every_minutes > 0 {
            let due: Vec<usize> = (0..self.tabs.len())
                .filter(|&i| {
                    let t = &self.tabs[i];
                    t.history.revision() != t.versioned_rev && t.last_version_at.elapsed() >= every
                })
                .collect();
            let cur = self.active;
            for i in due {
                self.active = i;
                self.snapshot("auto");
            }
            self.active = cur;
        }
        // A message under the mouse (being read whole) stays.
        if let Some((_, _, at)) = &self.msg
            && at.elapsed() > Duration::from_secs(8)
            && self.side_hover != Some(Hit::Message)
        {
            self.msg = None;
            changed = true;
        }
        changed
    }

    pub fn autosave(&mut self, force: bool) {
        self.last_autosave = Instant::now();
        let dir = self.paths.recovery_dir();
        for t in &mut self.tabs {
            if !t.dirty() {
                // Undone back to the saved state: the recovery file is stale.
                if t.recovery_written {
                    recovery::clear(&dir, t.doc.meta.id);
                    t.recovery_written = false;
                }
            } else if (force || t.history.revision() != t.autosaved_rev)
                && recovery::write(&dir, &t.doc, t.file.as_deref()).is_ok()
            {
                t.autosaved_rev = t.history.revision();
                t.recovery_written = true;
            }
        }
    }
}

fn contains(r: Rect, (x, y): (u16, u16)) -> bool {
    x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

/// Plain text → clip with the given colors (spaces transparent).
pub fn text_clip(s: &str, fg: Color, bg: Color) -> Clip {
    let lines: Vec<Vec<char>> = s.lines().map(|l| l.chars().filter(|c| !c.is_control()).collect()).collect();
    let w = lines.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let h = lines.len().max(1);
    let mut c = Clip::new(w, h);
    for (y, l) in lines.iter().enumerate() {
        for (x, ch) in l.iter().enumerate() {
            if *ch != ' ' {
                c.set(x, y, Some(Cell::new(*ch, fg, bg)));
            }
        }
    }
    c
}

/// A file's name, for status lines too short for the whole path.
fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Why a file didn't open, short enough that the reason fits the status line:
/// the outer context only repeats the path and the innermost is often a
/// codec's jargon, so say the reason just under the path.
pub(crate) fn open_error(p: &Path, e: &anyhow::Error) -> String {
    let why = e.chain().nth(1).unwrap_or(e.root_cause());
    format!("can't open {}: {why}", file_name(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_error_names_the_file_and_the_reason() {
        let p = Path::new("/a/long/folder/name/broken.acid");
        let e = anyhow::anyhow!("Unknown frame descriptor")
            .context("not an .acid file, or it is damaged")
            .context(format!("loading {} as acidtrip", p.display()));
        assert_eq!(open_error(p, &e), "can't open broken.acid: not an .acid file, or it is damaged");
        let e = anyhow::anyhow!("No such file").context(format!("reading {}", p.display()));
        assert_eq!(open_error(p, &e), "can't open broken.acid: No such file");
    }
}

/// Config problems as one status line: the first, and how many more.
fn notes_line(notes: &[String]) -> Option<String> {
    let first = notes.first()?;
    Some(match notes.len() {
        1 => first.clone(),
        n => format!("{first} (+{} more)", n - 1),
    })
}

/// A config load error in one short line: where in config.toml it went
/// wrong (the full path and the TOML excerpt don't fit a status bar).
fn config_error(e: &anyhow::Error) -> String {
    let cause = e.root_cause().to_string();
    let first = cause.lines().next().unwrap_or("").trim().trim_start_matches("TOML parse error ");
    format!("config.toml error {first}")
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn config_errors_fit_the_status_bar() {
        let e = Config::parse("[keymap\npreset = ").unwrap_err().context("/some/long/path/config.toml");
        let m = config_error(&e);
        assert!(m.starts_with("config.toml error at line 1"), "{m}");
        assert!(!m.contains('\n') && !m.contains("/some/long"), "{m}");
        assert_eq!(notes_line(&["a".into(), "b".into(), "c".into()]).as_deref(), Some("a (+2 more)"));
        assert_eq!(notes_line(&[]), None);
    }
}
