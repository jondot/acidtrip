//! Native window mode: the same app, drawn into its own window instead of a
//! terminal. ratatui renders into [`GridBackend`]; the cells are painted with
//! the VGA 8x16 font from `acidtrip-core`; winit input is translated into the
//! crossterm events the app already understands.
//!
//! Threads: winit owns the main thread (macOS requires it). The app runs on
//! its own thread, reading events from a channel and publishing frames.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acidtrip_core::color::{VGA, xterm256};
use acidtrip_core::render::{self, CELL_H, CELL_W, RenderOptions};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton as CtButton, MouseEvent,
    MouseEventKind,
};
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color, Modifier};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Window, WindowId};

use crate::app::{self, Input};

/// Default colors, the same as the test harness's emulated terminal so the
/// two render identically.
pub const DEFAULT_FG: [u8; 3] = [200, 200, 200];
pub const DEFAULT_BG: [u8; 3] = [12, 12, 16];

// ------------------------------------------------------------------ frame

/// One published screen.
#[derive(Clone, Default)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
    /// Cursor cell when the cursor is shown.
    pub cursor: Option<(u16, u16)>,
    /// Pictures over the cells (thumbnails), in window pixels.
    pub images: Vec<crate::ui::thumbs::Overlay>,
}

/// xterm index → RGB, 0-15 in ANSI order mapped to the VGA palette.
fn index_rgb(i: u8) -> [u8; 3] {
    const ANSI_TO_VGA: [usize; 8] = [0, 4, 2, 6, 1, 5, 3, 7];
    match i {
        0..=7 => VGA[ANSI_TO_VGA[i as usize]],
        8..=15 => VGA[8 + ANSI_TO_VGA[i as usize - 8]],
        _ => xterm256(i),
    }
}

fn color_rgb(c: Color, default: [u8; 3]) -> [u8; 3] {
    match c {
        Color::Reset => default,
        Color::Black => index_rgb(0),
        Color::Red => index_rgb(1),
        Color::Green => index_rgb(2),
        Color::Yellow => index_rgb(3),
        Color::Blue => index_rgb(4),
        Color::Magenta => index_rgb(5),
        Color::Cyan => index_rgb(6),
        Color::Gray => index_rgb(7),
        Color::DarkGray => index_rgb(8),
        Color::LightRed => index_rgb(9),
        Color::LightGreen => index_rgb(10),
        Color::LightYellow => index_rgb(11),
        Color::LightBlue => index_rgb(12),
        Color::LightMagenta => index_rgb(13),
        Color::LightCyan => index_rgb(14),
        Color::White => index_rgb(15),
        Color::Indexed(i) => index_rgb(i),
        Color::Rgb(r, g, b) => [r, g, b],
    }
}

impl Frame {
    /// Paint the frame (the same rules as the harness's terminal render).
    pub fn render(&self, scale: u32) -> image::RgbaImage {
        let cols = self.cols as usize;
        let mut resolved = Vec::with_capacity(self.cells.len());
        for (i, c) in self.cells.iter().enumerate() {
            let (x, y) = ((i % cols) as u16, (i / cols) as u16);
            let ch = c.symbol().chars().next().unwrap_or(' ');
            let mut fg = color_rgb(c.fg, DEFAULT_FG);
            let mut bg = color_rgb(c.bg, DEFAULT_BG);
            if c.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if c.modifier.contains(Modifier::DIM) {
                fg = [0, 1, 2].map(|i| ((fg[i] as u16 + bg[i] as u16) / 2) as u8);
            }
            if self.cursor == Some((x, y)) {
                std::mem::swap(&mut fg, &mut bg);
            }
            resolved.push((ch, fg, bg, c.modifier.contains(Modifier::UNDERLINED)));
        }
        let opts = RenderOptions { scale, ..RenderOptions::default() };
        let mut img = render::render_cells(cols, self.rows as usize, opts, |x, y| {
            let c = resolved[y * cols + x];
            (c.0, c.1, c.2)
        });
        for (i, c) in resolved.iter().enumerate() {
            if c.3 {
                let (x, y) = ((i % cols) as u32, (i / cols) as u32);
                for px in 0..CELL_W * scale {
                    for sy in 0..scale {
                        img.put_pixel(
                            x * CELL_W * scale + px,
                            (y * CELL_H + CELL_H - 2) * scale + sy,
                            image::Rgba([c.1[0], c.1[1], c.1[2], 255]),
                        );
                    }
                }
            }
        }
        for (r, pic) in &self.images {
            let (w, h) = (r.width as u32 * CELL_W * scale, r.height as u32 * CELL_H * scale);
            let (x, y) = (r.x as i64 * (CELL_W * scale) as i64, r.y as i64 * (CELL_H * scale) as i64);
            if pic.dimensions() == (w, h) {
                image::imageops::replace(&mut img, &**pic, x, y);
            } else {
                // Snapshots are 1x while pictures are made at window scale.
                let fit = image::imageops::resize(&**pic, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
                image::imageops::replace(&mut img, &fit, x, y);
            }
        }
        img
    }
}

// ---------------------------------------------------------------- backend

#[derive(Debug)]
pub enum UserEvent {
    /// A new frame is ready.
    Redraw,
    /// The app thread finished.
    Exit,
}

struct Shared {
    frame: Mutex<Frame>,
    /// Current size in cells.
    size: Mutex<(u16, u16)>,
    /// Window pixels per VGA pixel, fixed when the window opens.
    scale: AtomicU32,
}

impl Shared {
    fn scale(&self) -> u32 {
        self.scale.load(Ordering::Relaxed).max(1)
    }
}

/// A ratatui backend that keeps the screen in memory and hands finished
/// frames to the window.
pub struct GridBackend {
    buf: Buffer,
    cursor: Position,
    cursor_shown: bool,
    shared: Arc<Shared>,
    proxy: EventLoopProxy<UserEvent>,
    /// `ACIDTRIP_WINDOW_SNAP`: write each frame here as a 1x PNG (test rig).
    snap: Option<PathBuf>,
}

impl GridBackend {
    fn sync_size(&mut self) {
        let (cols, rows) = *self.shared.size.lock().unwrap();
        let area = Rect::new(0, 0, cols, rows);
        if self.buf.area != area {
            self.buf.resize(area);
        }
    }
}

impl Backend for GridBackend {
    type Error = std::io::Error;

    fn draw<'a, I>(&mut self, content: I) -> std::io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.sync_size();
        for (x, y, c) in content {
            if let Some(cell) = self.buf.cell_mut((x, y)) {
                *cell = c.clone();
            }
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> std::io::Result<()> {
        self.cursor_shown = false;
        Ok(())
    }

    fn show_cursor(&mut self) -> std::io::Result<()> {
        self.cursor_shown = true;
        Ok(())
    }

    fn get_cursor_position(&mut self) -> std::io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> std::io::Result<()> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> std::io::Result<()> {
        self.sync_size();
        self.buf.reset();
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> std::io::Result<()> {
        self.clear()
    }

    fn size(&self) -> std::io::Result<Size> {
        let (w, h) = *self.shared.size.lock().unwrap();
        Ok(Size::new(w, h))
    }

    fn window_size(&mut self) -> std::io::Result<WindowSize> {
        let size = self.size()?;
        let s = self.shared.scale();
        Ok(WindowSize {
            columns_rows: size,
            pixels: Size::new(size.width * (CELL_W * s) as u16, size.height * (CELL_H * s) as u16),
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.sync_size();
        let frame = Frame {
            cols: self.buf.area.width,
            rows: self.buf.area.height,
            cells: self.buf.content.clone(),
            cursor: self.cursor_shown.then_some((self.cursor.x, self.cursor.y)),
            images: crate::ui::thumbs::take_overlays(),
        };
        if let Some(path) = &self.snap {
            let tmp = path.with_extension("tmp.png");
            if frame.render(1).save_with_format(&tmp, image::ImageFormat::Png).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
        *self.shared.frame.lock().unwrap() = frame;
        let _ = self.proxy.send_event(UserEvent::Redraw);
        Ok(())
    }
}

// ------------------------------------------------------------------ input

enum Msg {
    Ev(Event),
    Close,
}

/// Events from the window, via a channel.
pub struct WindowInput {
    rx: Receiver<Msg>,
    pending: Option<Event>,
    closed: bool,
}

impl WindowInput {
    fn take(&mut self, m: Msg) -> bool {
        match m {
            Msg::Ev(e) => {
                self.pending = Some(e);
                true
            }
            Msg::Close => {
                self.closed = true;
                false
            }
        }
    }
}

impl Input for WindowInput {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<bool> {
        if self.pending.is_some() {
            return Ok(true);
        }
        if self.closed {
            return Ok(false);
        }
        let m = if timeout.is_zero() {
            match self.rx.try_recv() {
                Ok(m) => m,
                Err(TryRecvError::Empty) => return Ok(false),
                Err(TryRecvError::Disconnected) => Msg::Close,
            }
        } else {
            match self.rx.recv_timeout(timeout) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => return Ok(false),
                Err(RecvTimeoutError::Disconnected) => Msg::Close,
            }
        };
        Ok(self.take(m))
    }

    fn read(&mut self) -> std::io::Result<Event> {
        loop {
            if let Some(e) = self.pending.take() {
                return Ok(e);
            }
            if self.closed {
                return Err(std::io::Error::other("window closed"));
            }
            let m = self.rx.recv().unwrap_or(Msg::Close);
            self.take(m);
        }
    }

    fn closed(&self) -> bool {
        self.closed
    }
}

fn mods(m: ModifiersState) -> KeyModifiers {
    let mut k = KeyModifiers::NONE;
    if m.shift_key() {
        k |= KeyModifiers::SHIFT;
    }
    if m.control_key() {
        k |= KeyModifiers::CONTROL;
    }
    if m.alt_key() {
        k |= KeyModifiers::ALT;
    }
    if m.super_key() {
        k |= KeyModifiers::SUPER;
    }
    k
}

/// winit key → crossterm key, the way a kitty-protocol terminal reports it.
fn key_event(ev: &winit::event::KeyEvent, m: ModifiersState) -> Option<KeyEvent> {
    let mut modifiers = mods(m);
    // With Ctrl/Alt held, use the unmodified key (Alt-x on macOS types ≈).
    let key = if m.control_key() || m.alt_key() { ev.key_without_modifiers() } else { ev.logical_key.clone() };
    let code = match &key {
        Key::Named(n) => match n {
            NamedKey::Enter => KeyCode::Enter,
            NamedKey::Tab if m.shift_key() => KeyCode::BackTab,
            NamedKey::Tab => KeyCode::Tab,
            NamedKey::Backspace => KeyCode::Backspace,
            NamedKey::Escape => KeyCode::Esc,
            NamedKey::ArrowLeft => KeyCode::Left,
            NamedKey::ArrowRight => KeyCode::Right,
            NamedKey::ArrowUp => KeyCode::Up,
            NamedKey::ArrowDown => KeyCode::Down,
            NamedKey::Home => KeyCode::Home,
            NamedKey::End => KeyCode::End,
            NamedKey::PageUp => KeyCode::PageUp,
            NamedKey::PageDown => KeyCode::PageDown,
            NamedKey::Insert => KeyCode::Insert,
            NamedKey::Delete => KeyCode::Delete,
            NamedKey::Space => KeyCode::Char(' '),
            NamedKey::F1 => KeyCode::F(1),
            NamedKey::F2 => KeyCode::F(2),
            NamedKey::F3 => KeyCode::F(3),
            NamedKey::F4 => KeyCode::F(4),
            NamedKey::F5 => KeyCode::F(5),
            NamedKey::F6 => KeyCode::F(6),
            NamedKey::F7 => KeyCode::F(7),
            NamedKey::F8 => KeyCode::F(8),
            NamedKey::F9 => KeyCode::F(9),
            NamedKey::F10 => KeyCode::F(10),
            NamedKey::F11 => KeyCode::F(11),
            NamedKey::F12 => KeyCode::F(12),
            _ => return None,
        },
        Key::Character(s) => {
            let mut c = s.chars().next()?;
            if m.shift_key() && (m.control_key() || m.alt_key()) {
                c = c.to_ascii_uppercase();
            }
            // Shifted printable keys arrive as their shifted char; the
            // shift is already in the char, as crossterm reports it.
            if !c.is_ascii_alphabetic() && !m.control_key() && !m.alt_key() {
                modifiers.remove(KeyModifiers::SHIFT);
            }
            KeyCode::Char(c)
        }
        _ => return None,
    };
    if code == KeyCode::BackTab {
        modifiers.insert(KeyModifiers::SHIFT);
    }
    let kind = if ev.repeat { KeyEventKind::Repeat } else { KeyEventKind::Press };
    Some(KeyEvent { code, modifiers, kind, state: KeyEventState::NONE })
}

// ----------------------------------------------------------------- window

/// The app icon, for window managers that show one (X11, Wayland via the
/// .desktop file; macOS takes it from the app bundle).
fn icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../../../packaging/icon/acidtrip-64.png")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// Runs the app on its thread: backend, input, and the window's scale.
type StartApp = Box<dyn FnOnce(GridBackend, WindowInput, u32) + Send>;

struct WinApp {
    shared: Arc<Shared>,
    proxy: EventLoopProxy<UserEvent>,
    tx: Sender<Msg>,
    /// Handed to the app thread when the window exists.
    start: Option<StartApp>,
    /// `ACIDTRIP_WINDOW_SCALE`, else the monitor's scale factor.
    scale_override: Option<u32>,
    rx: Option<Receiver<Msg>>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    init_cells: (u16, u16),
    modifiers: ModifiersState,
    cursor_px: PhysicalPosition<f64>,
    buttons: Vec<CtButton>,
    pixel_mouse: bool,
    scroll_acc: f64,
    snap: Option<PathBuf>,
}

impl WinApp {
    fn send(&self, e: Event) {
        let _ = self.tx.send(Msg::Ev(e));
    }

    fn mouse(&self, kind: MouseEventKind) {
        let s = self.shared.scale() as f64;
        let (cw, ch) = (CELL_W as f64 * s, CELL_H as f64 * s);
        let (x, y) = (self.cursor_px.x.max(0.0), self.cursor_px.y.max(0.0));
        let (column, row) = if self.pixel_mouse { (x as u16, y as u16) } else { ((x / cw) as u16, (y / ch) as u16) };
        self.send(Event::Mouse(MouseEvent { kind, column, row, modifiers: mods(self.modifiers) }));
    }

    fn resized(&mut self, size: PhysicalSize<u32>) {
        let s = self.shared.scale();
        let cols = (size.width / (CELL_W * s)).max(1) as u16;
        let rows = (size.height / (CELL_H * s)).max(1) as u16;
        let mut cur = self.shared.size.lock().unwrap();
        if *cur != (cols, rows) {
            *cur = (cols, rows);
            drop(cur);
            self.send(Event::Resize(cols, rows));
        }
        if let (Some(surface), Some(w), Some(h)) =
            (self.surface.as_mut(), NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            let _ = surface.resize(w, h);
        }
    }

    fn present(&mut self) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let _ = surface.resize(w, h);
        let frame = self.shared.frame.lock().unwrap().clone();
        let img = frame.render(self.shared.scale());
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let bg = u32::from_be_bytes([0, DEFAULT_BG[0], DEFAULT_BG[1], DEFAULT_BG[2]]);
        let (iw, ih) = (img.width(), img.height());
        for y in 0..size.height {
            let row = &mut buf[(y * size.width) as usize..((y + 1) * size.width) as usize];
            for (x, px) in row.iter_mut().enumerate() {
                let x = x as u32;
                *px = if x < iw && y < ih {
                    let p = img.get_pixel(x, y).0;
                    u32::from_be_bytes([0, p[0], p[1], p[2]])
                } else {
                    bg
                };
            }
        }
        let _ = buf.present();
    }
}

impl ApplicationHandler<UserEvent> for WinApp {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (cols, rows) = self.init_cells;
        let mut attrs = Window::default_attributes().with_title("acidtrip").with_window_icon(icon());
        attrs = match self.scale_override {
            Some(s) => attrs.with_inner_size(PhysicalSize::new(cols as u32 * CELL_W * s, rows as u32 * CELL_H * s)),
            // 8x16 points per cell: crisp at whole-number scale factors (Retina = 2).
            None => attrs.with_inner_size(LogicalSize::new(cols as u32 * CELL_W, rows as u32 * CELL_H)),
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // Matches the .desktop file, so docks group the window under its launcher.
            use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
            attrs = WindowAttributesExtX11::with_name(attrs, "acidtrip", "acidtrip");
            attrs = WindowAttributesExtWayland::with_name(attrs, "acidtrip", "acidtrip");
        }
        let window = Rc::new(el.create_window(attrs).expect("create window"));
        let scale = self.scale_override.unwrap_or_else(|| (window.scale_factor().round() as u32).max(1));
        self.shared.scale.store(scale, Ordering::Relaxed);
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
        self.window = Some(window.clone());
        self.surface = Some(surface);
        self.resized(window.inner_size());
        if let (Some(start), Some(rx)) = (self.start.take(), self.rx.take()) {
            let backend = GridBackend {
                buf: Buffer::empty(Rect::new(0, 0, 1, 1)),
                cursor: Position::default(),
                cursor_shown: false,
                shared: self.shared.clone(),
                proxy: self.proxy.clone(),
                snap: self.snap.clone(),
            };
            let input = WindowInput { rx, pending: None, closed: false };
            std::thread::Builder::new()
                .name("acidtrip-app".into())
                .spawn(move || start(backend, input, scale))
                .expect("spawn app thread");
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Redraw => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::Exit => el.exit(),
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                let _ = self.tx.send(Msg::Close);
            }
            WindowEvent::Resized(size) => {
                self.resized(size);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.present(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(k) = key_event(&event, self.modifiers) {
                    self.send(Event::Key(k));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_px = position;
                match self.buttons.last() {
                    Some(&b) => self.mouse(MouseEventKind::Drag(b)),
                    None => self.mouse(MouseEventKind::Moved),
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => CtButton::Left,
                    MouseButton::Right => CtButton::Right,
                    MouseButton::Middle => CtButton::Middle,
                    _ => return,
                };
                match state {
                    ElementState::Pressed => {
                        self.buttons.push(b);
                        self.mouse(MouseEventKind::Down(b));
                    }
                    ElementState::Released => {
                        self.buttons.retain(|&x| x != b);
                        self.mouse(MouseEventKind::Up(b));
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / (CELL_H * self.shared.scale()) as f64,
                };
                self.scroll_acc += lines;
                while self.scroll_acc >= 1.0 {
                    self.scroll_acc -= 1.0;
                    self.mouse(MouseEventKind::ScrollUp);
                }
                while self.scroll_acc <= -1.0 {
                    self.scroll_acc += 1.0;
                    self.mouse(MouseEventKind::ScrollDown);
                }
            }
            WindowEvent::Focused(f) => self.send(if f { Event::FocusGained } else { Event::FocusLost }),
            _ => {}
        }
    }
}

// ------------------------------------------------------------------- entry

fn env_size() -> Option<(u16, u16)> {
    let v = std::env::var("ACIDTRIP_WINDOW_SIZE").ok()?;
    let (c, r) = v.split_once('x')?;
    Some((c.trim().parse().ok()?, r.trim().parse().ok()?))
}

/// Run the editor in a native window.
pub fn run(files: Vec<PathBuf>) -> anyhow::Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    // Whole-number scales keep the VGA font crisp.
    let scale_override = std::env::var("ACIDTRIP_WINDOW_SCALE").ok().and_then(|v| v.parse::<u32>().ok()).map(|s| s.max(1));
    let pixel_mouse = std::env::var("ACIDTRIP_PIXEL_MOUSE").map_or(true, |v| v != "0");
    let init_cells = env_size().unwrap_or((160, 50));
    let shared = Arc::new(Shared { frame: Mutex::new(Frame::default()), size: Mutex::new(init_cells), scale: AtomicU32::new(1) });
    let (tx, rx) = mpsc::channel();
    let result: Arc<Mutex<Option<anyhow::Result<()>>>> = Arc::new(Mutex::new(None));
    let result2 = result.clone();
    let exit_proxy = proxy.clone();
    let start = Box::new(move |backend: GridBackend, mut input: WindowInput, scale: u32| {
        let r = (|| -> anyhow::Result<()> {
            let mut terminal = ratatui::Terminal::new(backend)?;
            let mut app = app::App::new(&files, true, None)?;
            if pixel_mouse {
                let s = scale as f32;
                app.cell_px = Some((CELL_W as f32 * s, CELL_H as f32 * s));
            }
            app.thumbs = crate::ui::thumbs::Thumbs::native((CELL_W * scale, CELL_H * scale));
            crate::run_start_actions(&mut app);
            let r = app.run_loop(&mut terminal, &mut input);
            app.autosave(false);
            r
        })();
        *result2.lock().unwrap() = Some(r);
        let _ = exit_proxy.send_event(UserEvent::Exit);
    });
    let mut win = WinApp {
        shared,
        proxy,
        tx,
        start: Some(start),
        scale_override,
        rx: Some(rx),
        window: None,
        surface: None,
        init_cells,
        modifiers: ModifiersState::empty(),
        cursor_px: PhysicalPosition::new(0.0, 0.0),
        buttons: vec![],
        pixel_mouse,
        scroll_acc: 0.0,
        snap: std::env::var_os("ACIDTRIP_WINDOW_SNAP").map(PathBuf::from),
    };
    event_loop.run_app(&mut win)?;
    result.lock().unwrap().take().unwrap_or(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_paint_over_their_cells_at_any_scale() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        let red = image::Rgba([255, 0, 0, 255]);
        // Made at window scale 2: 2x1 cells = 32x32 px.
        let pic = Arc::new(image::RgbaImage::from_pixel(32, 32, red));
        crate::ui::thumbs::overlay(&mut buf, Rect::new(1, 1, 2, 1), pic);
        let frame = Frame {
            cols: 4,
            rows: 2,
            cells: buf.content.clone(),
            cursor: None,
            images: crate::ui::thumbs::take_overlays(),
        };
        for scale in [1, 2] {
            let img = frame.render(scale);
            let (cw, ch) = (CELL_W * scale, CELL_H * scale);
            assert_eq!(*img.get_pixel(cw, ch), red, "scale {scale}: picture's corner");
            assert_eq!(*img.get_pixel(3 * cw - 1, 2 * ch - 1), red, "scale {scale}: picture's far corner");
            assert_ne!(*img.get_pixel(cw - 1, ch), red, "scale {scale}: left of the picture");
            assert_ne!(*img.get_pixel(3 * cw, ch), red, "scale {scale}: right of the picture");
        }
        assert!(crate::ui::thumbs::take_overlays().is_empty(), "taken once per frame");
    }
}
