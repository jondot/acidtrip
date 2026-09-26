//! Terminal setup/teardown. Restores the terminal on panic too.

use std::io::{self, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange,
    EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::DefaultTerminal;

pub struct TermGuard {
    pub enhanced_keys: bool,
}

pub fn setup() -> anyhow::Result<(DefaultTerminal, TermGuard)> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste, EnableFocusChange)?;
    let enhanced_keys = matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true));
    if enhanced_keys {
        execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )?;
    }
    install_panic_hook(enhanced_keys);
    let terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    Ok((terminal, TermGuard { enhanced_keys }))
}

fn restore(enhanced: bool) {
    disable_pixel_mouse();
    let mut out = io::stdout();
    if enhanced {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableFocusChange,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
    let _ = out.flush();
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        restore(self.enhanced_keys);
    }
}

fn install_panic_hook(enhanced: bool) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore(enhanced);
        prev(info);
    }));
}

/// Temporarily leave the TUI (e.g. to run $EDITOR), then come back.
pub fn suspend<T>(enhanced: bool, f: impl FnOnce() -> T) -> T {
    restore(enhanced);
    let r = f();
    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste, EnableFocusChange);
    if enhanced {
        let _ = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        );
    }
    r
}

/// Ask the terminal (DECRQM) whether it supports SGR-Pixels mouse reports
/// (mode 1016) and turn it on if so. Must run before crossterm starts
/// reading input. Returns true when mouse coordinates will be pixels.
#[cfg(unix)]
pub fn enable_pixel_mouse() -> bool {
    use std::io::Read;
    if std::env::var("ACIDTRIP_PIXEL_MOUSE").is_ok_and(|v| v == "0") {
        return false;
    }
    let mut out = io::stdout();
    if out.write_all(b"\x1b[?1016$p").and_then(|_| out.flush()).is_err() {
        return false;
    }
    // Read the reply `ESC [ ? 1016 ; Ps $ y` (Ps 1/2 = known, 0/4 = not) with a timeout.
    let mut reply = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    let mut stdin = io::stdin();
    while std::time::Instant::now() < deadline && !reply.ends_with(b"$y") {
        let mut fds = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as i32;
        // SAFETY: one valid pollfd for stdin.
        if unsafe { libc::poll(&mut fds, 1, left.max(1)) } <= 0 {
            break;
        }
        let mut b = [0u8; 64];
        match stdin.read(&mut b) {
            Ok(n) if n > 0 => reply.extend_from_slice(&b[..n]),
            _ => break,
        }
    }
    let text = String::from_utf8_lossy(&reply);
    let supported = text.contains("[?1016;1$y") || text.contains("[?1016;2$y");
    if supported {
        let _ = out.write_all(b"\x1b[?1016h");
        let _ = out.flush();
    }
    supported
}

#[cfg(not(unix))]
pub fn enable_pixel_mouse() -> bool {
    false
}

pub fn disable_pixel_mouse() {
    let mut out = io::stdout();
    let _ = out.write_all(b"\x1b[?1016l");
    let _ = out.flush();
}
