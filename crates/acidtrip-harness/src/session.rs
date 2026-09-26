//! A child process running in a pseudo-terminal, with its output fed through
//! a `vt100` emulator by a background thread.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use acidtrip_core::color::{VGA, xterm256};
use acidtrip_core::render::{self, CELL_H, CELL_W, RenderOptions};
use anyhow::{Context, Result, anyhow, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::keys;
use crate::responder::{self, ReplyCtx, Scanner};

/// Default foreground of the emulated terminal (also reported to OSC 10).
pub const DEFAULT_FG: [u8; 3] = [200, 200, 200];
/// Default background of the emulated terminal (also reported to OSC 11).
pub const DEFAULT_BG: [u8; 3] = [12, 12, 16];

const POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press(MouseButton),
    Release(MouseButton),
    /// Motion with a button held.
    Drag(MouseButton),
    /// Motion with no button held.
    Move,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

/// A mouse event at 0-based cell coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEv {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    pub mods: keys::Mods,
}

impl MouseEv {
    pub fn new(kind: MouseKind, x: u16, y: u16) -> Self {
        MouseEv { kind, x, y, mods: keys::Mods::default() }
    }

    /// SGR (1006) encoding.
    pub fn encode(&self) -> Vec<u8> {
        let btn = |b: MouseButton| match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        };
        let (code, release) = match self.kind {
            MouseKind::Press(b) => (btn(b), false),
            MouseKind::Release(b) => (btn(b), true),
            MouseKind::Drag(b) => (btn(b) + 32, false),
            MouseKind::Move => (35, false),
            MouseKind::ScrollUp => (64, false),
            MouseKind::ScrollDown => (65, false),
            MouseKind::ScrollLeft => (66, false),
            MouseKind::ScrollRight => (67, false),
        };
        let code = code + 4 * self.mods.shift as u32 + 8 * self.mods.alt as u32 + 16 * self.mods.ctrl as u32;
        format!("\x1b[<{};{};{}{}", code, self.x as u32 + 1, self.y as u32 + 1, if release { 'm' } else { 'M' })
            .into_bytes()
    }
}

/// A resolved screen cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellInfo {
    /// First char of the cell contents (' ' when empty).
    pub ch: char,
    /// Full cell contents (may hold combining chars; empty when blank).
    pub text: String,
    /// Foreground as set by the app (before `inverse` is applied).
    pub fg: [u8; 3],
    /// Background as set by the app (before `inverse` is applied).
    pub bg: [u8; 3],
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
}

/// Map an xterm color index to RGB. 0-15 use the VGA-like palette in ANSI
/// order (1 = red), 16-255 the xterm cube and grays.
pub fn index_rgb(i: u8) -> [u8; 3] {
    const ANSI_TO_VGA: [usize; 8] = [0, 4, 2, 6, 1, 5, 3, 7];
    match i {
        0..=7 => VGA[ANSI_TO_VGA[i as usize]],
        8..=15 => VGA[8 + ANSI_TO_VGA[i as usize - 8]],
        _ => xterm256(i),
    }
}

fn color_rgb(c: vt100::Color, default: [u8; 3]) -> [u8; 3] {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => index_rgb(i),
        vt100::Color::Rgb(r, g, b) => [r, g, b],
    }
}

const RAW_CAP: usize = 32 << 20;

struct Shared {
    parser: Mutex<vt100::Parser>,
    last_output: Mutex<Instant>,
    /// When input was last sent: quiet counts from after it, so a wait right
    /// after typing can't pass on the silence from before the keys.
    last_input: Mutex<Instant>,
    total_bytes: Mutex<u64>,
    /// Raw output (capped) for asserting on escape sequences like graphics.
    raw: Mutex<Vec<u8>>,
    eof: AtomicBool,
}

pub struct Session {
    shared: Arc<Shared>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pid: Option<u32>,
    reader: Option<JoinHandle<()>>,
    exit_code: Option<i32>,
}

impl Session {
    /// Spawn `program args` in a pty of `cols` x `rows`. The environment is
    /// inherited, with `TERM=xterm-256color`, `COLORTERM=truecolor` and then
    /// `env` applied on top.
    pub fn spawn(
        program: &Path,
        args: &[&str],
        cols: u16,
        rows: u16,
        env: &[(&str, &str)],
        cwd: Option<&Path>,
    ) -> Result<Session> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows, cols, pixel_width: cols * CELL_W as u16, pixel_height: rows * CELL_H as u16 })
            .map_err(|e| anyhow!("openpty: {e}"))?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Don't inherit the host terminal's identity (e.g. iTerm2 when tests run
        // inside iTerm2): apps would assume graphics the emulator can't show.
        for v in [
            "TERM_PROGRAM",
            "TERM_PROGRAM_VERSION",
            "LC_TERMINAL",
            "LC_TERMINAL_VERSION",
            "ITERM_SESSION_ID",
            "KITTY_WINDOW_ID",
            "WEZTERM_EXECUTABLE",
            "TMUX",
        ] {
            cmd.env_remove(v);
        }
        // Blinking cursors never let the screen go idle; tests want stable frames.
        cmd.env("ACIDTRIP_NO_BLINK", "1");
        // No network unless a script asks (`env ACIDTRIP_OFFLINE ""`).
        cmd.env("ACIDTRIP_OFFLINE", "1");
        for k in ["NO_COLOR", "TMUX", "STY", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "KITTY_WINDOW_ID", "WEZTERM_PANE"]
        {
            cmd.env_remove(k);
        }
        // A developer's real key must never reach an e2e run; scripts opt in
        // through `env` / `fake-claude`.
        for k in ["ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"] {
            cmd.env_remove(k);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        match cwd {
            Some(d) => cmd.cwd(d),
            None => cmd.cwd(std::env::current_dir()?),
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| anyhow!("spawn {}: {e}", program.display()))?;
        // Close our copy of the slave so reads hit EOF when the child exits.
        drop(pair.slave);
        let pid = child.process_id();

        let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow!("pty reader: {e}"))?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| anyhow!("pty writer: {e}"))?));
        let shared = Arc::new(Shared {
            parser: Mutex::new(vt100::Parser::new(rows, cols, 0)),
            last_output: Mutex::new(Instant::now()),
            last_input: Mutex::new(Instant::now()),
            total_bytes: Mutex::new(0),
            raw: Mutex::new(Vec::new()),
            eof: AtomicBool::new(false),
        });

        let sh = shared.clone();
        let wr = writer.clone();
        let handle = std::thread::Builder::new().name("acidtrip-harness-pty".into()).spawn(move || {
            let mut scanner = Scanner::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let chunk = &buf[..n];
                let queries = scanner.scan(chunk);
                let mut replies = Vec::new();
                {
                    let mut p = lock(&sh.parser);
                    let mut fed = 0;
                    for (q, end) in &queries {
                        let end = (*end).clamp(fed, n);
                        p.process(&chunk[fed..end]);
                        fed = end;
                        let s = p.screen();
                        let (rows, cols) = s.size();
                        let ctx = ReplyCtx { cursor: s.cursor_position(), rows, cols, fg: DEFAULT_FG, bg: DEFAULT_BG };
                        replies.extend(responder::reply(q, &ctx));
                    }
                    p.process(&chunk[fed..]);
                }
                *lock(&sh.total_bytes) += n as u64;
                {
                    let mut raw = lock(&sh.raw);
                    let room = RAW_CAP.saturating_sub(raw.len());
                    raw.extend_from_slice(&chunk[..n.min(room)]);
                }
                *lock(&sh.last_output) = Instant::now();
                if !replies.is_empty() {
                    let mut w = lock(&wr);
                    let _ = w.write_all(&replies);
                    let _ = w.flush();
                }
            }
            sh.eof.store(true, Ordering::SeqCst);
        })?;

        Ok(Session { shared, writer, master: pair.master, child, pid, reader: Some(handle), exit_code: None })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn send_raw(&mut self, bytes: &[u8]) -> Result<()> {
        let mut w = lock(&self.writer);
        w.write_all(bytes).context("write to pty")?;
        w.flush().context("flush pty")?;
        *lock(&self.shared.last_input) = Instant::now();
        Ok(())
    }

    /// Send space-separated key specs (see [`crate::keys`]).
    pub fn keys(&mut self, spec: &str) -> Result<()> {
        let app_cursor = lock(&self.shared.parser).screen().application_cursor();
        // One write per key so apps see them as separate events.
        for k in spec.split_whitespace() {
            let bytes = keys::encode_key(k, app_cursor)?;
            let before = self.output_bytes();
            self.send_raw(&bytes)?;
            // Separate reads, so e.g. `esc a` is not decoded as `alt-a`. A lone
            // Esc is only "Esc" if nothing follows it quickly: wait until the
            // app has reacted to it (redrew), however loaded the machine is.
            if bytes == [0x1b] {
                let deadline = Instant::now() + Duration::from_millis(1500);
                while self.output_bytes() == before && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::thread::sleep(Duration::from_millis(40));
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }

    /// Type literal text (UTF-8), sent as plain keystroke bytes.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        self.send_raw(text.as_bytes())
    }

    pub fn mouse(&mut self, ev: MouseEv) -> Result<()> {
        self.send_raw(&ev.encode())?;
        std::thread::sleep(Duration::from_millis(5));
        Ok(())
    }

    pub fn click(&mut self, x: u16, y: u16) -> Result<()> {
        self.click_with(MouseButton::Left, x, y)
    }

    /// Press and release. A click that makes the app exit (Quit from a menu)
    /// can close the pty before the release is written: that is not an error.
    pub fn click_with(&mut self, b: MouseButton, x: u16, y: u16) -> Result<()> {
        self.mouse(MouseEv::new(MouseKind::Press(b), x, y))?;
        let release = self.mouse(MouseEv::new(MouseKind::Release(b), x, y));
        if release.is_err() {
            std::thread::sleep(Duration::from_millis(100));
            if self.poll_exit().is_some() {
                return Ok(());
            }
        }
        release
    }

    /// Left-button drag: press at `from`, drag through each intermediate cell
    /// of a straight line, release at `to`.
    pub fn drag(&mut self, from: (u16, u16), to: (u16, u16)) -> Result<()> {
        self.drag_with(MouseButton::Left, from, to)
    }

    pub fn drag_with(&mut self, button: MouseButton, from: (u16, u16), to: (u16, u16)) -> Result<()> {
        self.mouse(MouseEv::new(MouseKind::Press(button), from.0, from.1))?;
        let (dx, dy) = (to.0 as i32 - from.0 as i32, to.1 as i32 - from.1 as i32);
        let steps = dx.abs().max(dy.abs());
        for i in 1..=steps {
            let x = from.0 as i32 + dx * i / steps;
            let y = from.1 as i32 + dy * i / steps;
            self.mouse(MouseEv::new(MouseKind::Drag(button), x as u16, y as u16))?;
        }
        self.mouse(MouseEv::new(MouseKind::Release(button), to.0, to.1))
    }

    /// Wait until the screen contains `needle`. The error includes the screen.
    pub fn wait_for_text(&self, needle: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.screen_contains(needle) {
                return Ok(());
            }
            let text = self.screen_text();
            if Instant::now() >= deadline {
                bail!("timed out after {timeout:?} waiting for {needle:?}; screen:\n{}", framed(&text));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Wait until there has been no output for `quiet` (or the child closed
    /// the pty).
    pub fn wait_idle(&self, quiet: Duration, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let last = (*lock(&self.shared.last_output)).max(*lock(&self.shared.last_input));
            if last.elapsed() >= quiet || self.shared.eof.load(Ordering::SeqCst) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out after {timeout:?} waiting for {quiet:?} of quiet; screen:\n{}",
                    framed(&self.screen_text())
                );
            }
            std::thread::sleep(POLL);
        }
    }

    /// Raw bytes the child has written (the first 32 MB).
    pub fn raw_output(&self) -> Vec<u8> {
        lock(&self.shared.raw).clone()
    }

    /// Total bytes the child has written so far.
    pub fn output_bytes(&self) -> u64 {
        *lock(&self.shared.total_bytes)
    }

    /// Screen cell (column, row) at the middle of the `nth` (1-based)
    /// occurrence of `needle`, scanning rows top to bottom.
    pub fn find_text(&self, needle: &str, nth: usize) -> Option<(u16, u16)> {
        let p = lock(&self.shared.parser);
        let s = p.screen();
        let (rows, cols) = s.size();
        let want: Vec<char> = needle.chars().collect();
        let mut seen = 0;
        for y in 0..rows {
            // one entry per char, with the column it starts at
            let mut chars: Vec<(char, u16)> = vec![];
            for x in 0..cols {
                match s.cell(y, x) {
                    Some(c) if c.is_wide_continuation() => {}
                    Some(c) if c.has_contents() => chars.extend(c.contents().chars().map(|ch| (ch, x))),
                    _ => chars.push((' ', x)),
                }
            }
            if want.is_empty() || want.len() > chars.len() {
                continue;
            }
            for i in 0..=chars.len() - want.len() {
                if chars[i..i + want.len()].iter().map(|(c, _)| *c).eq(want.iter().copied()) {
                    seen += 1;
                    if seen == nth {
                        let (a, b) = (chars[i].1, chars[i + want.len() - 1].1);
                        return Some(((a + b) / 2, y));
                    }
                }
            }
        }
        None
    }

    /// Screen rows joined by `\n`, trailing spaces trimmed.
    pub fn screen_text(&self) -> String {
        self.rows_text(true)
    }

    fn rows_text(&self, trim: bool) -> String {
        let p = lock(&self.shared.parser);
        let s = p.screen();
        let (rows, cols) = s.size();
        let mut out = Vec::with_capacity(rows as usize);
        for y in 0..rows {
            let mut line = String::new();
            for x in 0..cols {
                match s.cell(y, x) {
                    Some(c) if c.is_wide_continuation() => {}
                    Some(c) if c.has_contents() => line.push_str(c.contents()),
                    _ => line.push(' '),
                }
            }
            out.push(if trim { line.trim_end().to_string() } else { line });
        }
        out.join("\n")
    }

    /// Does the screen contain `needle`? Matches against both the trimmed
    /// text and the full-width rows, so needles with trailing spaces work.
    pub fn screen_contains(&self, needle: &str) -> bool {
        self.screen_text().contains(needle) || self.rows_text(false).contains(needle)
    }

    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = lock(&self.shared.parser).screen().size();
        (cols, rows)
    }

    /// 0-based (x, y) cursor position.
    pub fn cursor(&self) -> (u16, u16) {
        let (r, c) = lock(&self.shared.parser).screen().cursor_position();
        (c, r)
    }

    pub fn cell(&self, x: u16, y: u16) -> Option<CellInfo> {
        let p = lock(&self.shared.parser);
        let c = p.screen().cell(y, x)?;
        let text = c.contents().to_string();
        Some(CellInfo {
            ch: text.chars().next().unwrap_or(' '),
            text,
            fg: color_rgb(c.fgcolor(), DEFAULT_FG),
            bg: color_rgb(c.bgcolor(), DEFAULT_BG),
            bold: c.bold(),
            dim: c.dim(),
            italic: c.italic(),
            underline: c.underline(),
            inverse: c.inverse(),
            wide: c.is_wide(),
        })
    }

    /// Render the screen to an RGBA image (8x16 pixels per cell).
    pub fn render(&self) -> image::RgbaImage {
        let p = lock(&self.shared.parser);
        let s = p.screen();
        let (rows, cols) = s.size();
        let (cr, cc) = s.cursor_position();
        let show_cursor = !s.hide_cursor();
        let mut cells = Vec::with_capacity(rows as usize * cols as usize);
        for y in 0..rows {
            for x in 0..cols {
                let (ch, mut fg, mut bg, underline) = match s.cell(y, x) {
                    Some(c) => {
                        let ch =
                            if c.is_wide_continuation() { ' ' } else { c.contents().chars().next().unwrap_or(' ') };
                        let mut fg = color_rgb(c.fgcolor(), DEFAULT_FG);
                        let mut bg = color_rgb(c.bgcolor(), DEFAULT_BG);
                        if c.inverse() {
                            std::mem::swap(&mut fg, &mut bg);
                        }
                        if c.dim() {
                            fg = [0, 1, 2].map(|i| ((fg[i] as u16 + bg[i] as u16) / 2) as u8);
                        }
                        (ch, fg, bg, c.underline())
                    }
                    None => (' ', DEFAULT_FG, DEFAULT_BG, false),
                };
                if show_cursor && y == cr && x == cc {
                    std::mem::swap(&mut fg, &mut bg);
                }
                cells.push((ch, fg, bg, underline));
            }
        }
        drop(p);
        let cols_u = cols as usize;
        let mut img = render::render_cells(cols_u, rows as usize, RenderOptions::default(), |x, y| {
            let c = cells[y * cols_u + x];
            (c.0, c.1, c.2)
        });
        for (i, c) in cells.iter().enumerate() {
            if c.3 {
                let (x, y) = ((i % cols_u) as u32, (i / cols_u) as u32);
                for px in 0..CELL_W {
                    img.put_pixel(x * CELL_W + px, y * CELL_H + CELL_H - 2, image::Rgba([c.1[0], c.1[1], c.1[2], 255]));
                }
            }
        }
        img
    }

    /// Save a PNG of the screen.
    pub fn screenshot(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        self.render()
            .save_with_format(path, image::ImageFormat::Png)
            .with_context(|| format!("save {}", path.display()))
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        // Counts as input: `wait idle` must give the app time to redraw.
        *lock(&self.shared.last_input) = Instant::now();
        lock(&self.shared.parser).screen_mut().set_size(rows, cols);
        self.master
            .resize(PtySize { rows, cols, pixel_width: cols * CELL_W as u16, pixel_height: rows * CELL_H as u16 })
            .map_err(|e| anyhow!("resize: {e}"))
    }

    pub fn is_alive(&mut self) -> bool {
        self.poll_exit().is_none()
    }

    fn poll_exit(&mut self) -> Option<i32> {
        if self.exit_code.is_none()
            && let Ok(Some(st)) = self.child.try_wait()
        {
            self.exit_code = Some(st.exit_code() as i32);
        }
        self.exit_code
    }

    /// SIGKILL the child (simulates a crash) and reap it.
    pub fn kill(&mut self) -> Result<()> {
        if self.poll_exit().is_some() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            let pid = self.pid.ok_or_else(|| anyhow!("no child pid"))?;
            // SAFETY: plain syscall on the child's pid.
            if unsafe { libc::kill(pid as i32, libc::SIGKILL) } != 0 {
                let err = std::io::Error::last_os_error();
                if self.poll_exit().is_none() {
                    return Err(err).context("kill -9");
                }
            }
        }
        #[cfg(not(unix))]
        self.child.kill()?;
        self.wait_exit(Duration::from_secs(5))?.ok_or_else(|| anyhow!("child did not die after SIGKILL"))?;
        Ok(())
    }

    /// Wait for the child to exit. `Ok(Some(code))` when it exited (a signal
    /// death reports a non-zero code), `Ok(None)` if still running at timeout.
    pub fn wait_exit(&mut self, timeout: Duration) -> Result<Option<i32>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(c) = self.poll_exit() {
                // Let the reader drain the remaining output.
                let drain = Instant::now() + Duration::from_millis(500);
                while !self.shared.eof.load(Ordering::SeqCst) && Instant::now() < drain {
                    std::thread::sleep(POLL);
                }
                return Ok(Some(c));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.poll_exit().is_none() {
            let _ = self.kill();
        }
        // The reader ends at EOF once the child is gone; don't block on it
        // if some grandchild still holds the pty open.
        if let Some(h) = self.reader.take()
            && self.shared.eof.load(Ordering::SeqCst)
        {
            let _ = h.join();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn framed(text: &str) -> String {
    let mut out = String::from("+----- screen -----\n");
    for l in text.lines() {
        out.push('|');
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("+------------------");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_encoding() {
        assert_eq!(MouseEv::new(MouseKind::Press(MouseButton::Left), 0, 0).encode(), b"\x1b[<0;1;1M");
        assert_eq!(MouseEv::new(MouseKind::Release(MouseButton::Right), 9, 4).encode(), b"\x1b[<2;10;5m");
        assert_eq!(MouseEv::new(MouseKind::Drag(MouseButton::Left), 1, 1).encode(), b"\x1b[<32;2;2M");
        assert_eq!(MouseEv::new(MouseKind::ScrollDown, 1, 1).encode(), b"\x1b[<65;2;2M");
        let mut e = MouseEv::new(MouseKind::Press(MouseButton::Left), 1, 1);
        e.mods.ctrl = true;
        assert_eq!(e.encode(), b"\x1b[<16;2;2M");
    }

    #[test]
    fn palette_is_ansi_ordered() {
        assert_eq!(index_rgb(1), [0xAA, 0, 0]);
        assert_eq!(index_rgb(4), [0, 0, 0xAA]);
        assert_eq!(index_rgb(9), [0xFF, 0x55, 0x55]);
        assert_eq!(index_rgb(196), [255, 0, 0]);
    }
}
