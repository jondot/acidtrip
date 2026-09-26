//! Terminal-capability probe used by the harness responder test. Behaves like
//! a crossterm app at startup: queries keyboard enhancement and the cursor
//! position, then echoes mouse events until a button release.

use std::io::Write;
use std::time::Instant;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::{cursor, execute, terminal};

fn main() -> std::io::Result<()> {
    let mut out = std::io::stdout();
    terminal::enable_raw_mode()?;
    let t0 = Instant::now();
    let enh = terminal::supports_keyboard_enhancement()?;
    let enh_ms = t0.elapsed().as_millis();
    write!(out, "\x1b[5;10H")?;
    out.flush()?;
    let t1 = Instant::now();
    let (col, row) = cursor::position()?;
    let pos_ms = t1.elapsed().as_millis();
    write!(out, "\x1b[1;1Henh={enh} ({enh_ms}ms)\r\npos={col},{row} ({pos_ms}ms)\r\n")?;
    execute!(out, EnableMouseCapture)?;
    write!(out, "READY\r\n")?;
    out.flush()?;
    loop {
        if let Event::Mouse(m) = event::read()? {
            write!(out, "mouse={:?} {},{}\r\n", m.kind, m.column, m.row)?;
            out.flush()?;
            if matches!(m.kind, event::MouseEventKind::Up(_)) {
                break;
            }
        }
    }
    execute!(out, DisableMouseCapture)?;
    out.flush()?;
    terminal::disable_raw_mode()?;
    Ok(())
}
