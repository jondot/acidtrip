//! Window-mode parity: drive the same steps through the terminal build (in
//! the pty harness) and the native window (real X input via xdotool), and
//! compare the two screens pixel for pixel.
//!
//! Needs an X server and xdotool, so it is ignored by default:
//!
//!     xvfb-run -s "-screen 0 1600x1000x24" cargo test -p acidtrip --test window_parity -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use acidtrip_harness::{MouseButton, MouseEv, MouseKind, Session};

const BIN: &str = env!("CARGO_BIN_EXE_acidtrip");
const COLS: u16 = 120;
const ROWS: u16 = 40;

#[derive(Clone, Copy)]
enum Step<'a> {
    /// Harness key spec, and the xdotool key names for the same keys.
    Keys(&'a str, &'a str),
    Type(&'a str),
    /// Hover then click a cell.
    Click(u16, u16),
    Drag((u16, u16), (u16, u16)),
    /// Compare the two screens.
    Check(&'a str),
}

fn out_dir() -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/window_parity");
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Win {
    child: Child,
    wid: String,
    snap: PathBuf,
    last_input: Instant,
}

impl Win {
    fn spawn(home: &Path, snap: PathBuf) -> Win {
        let child = Command::new(BIN)
            .arg("--window")
            .env("ACIDTRIP_HOME", home)
            .env("HOME", home)
            .env("ACIDTRIP_WINDOW_SIZE", format!("{COLS}x{ROWS}"))
            .env("ACIDTRIP_WINDOW_SCALE", "1")
            .env("ACIDTRIP_PIXEL_MOUSE", "0")
            .env("ACIDTRIP_WINDOW_SNAP", &snap)
            .env("ACIDTRIP_NO_BLINK", "1")
            .env("ACIDTRIP_OFFLINE", "1")
            // The harness terminal has no graphics: half-block previews on both.
            .env("ACIDTRIP_GRAPHICS", "off")
            .env_remove("ANTHROPIC_API_KEY")
            .current_dir(home)
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn window");
        let out = Command::new("xdotool")
            .args(["search", "--sync", "--name", "^acidtrip$"])
            // (no timeout flag in this xdotool; the window must start)
            .output()
            .expect("xdotool (install it, and run under xvfb-run)");
        let wid = String::from_utf8_lossy(&out.stdout).lines().next().expect("window id").to_string();
        xdo(&["windowmove", "--sync", &wid, "0", "0"]);
        xdo(&["windowfocus", "--sync", &wid]);
        Win { child, wid, snap, last_input: Instant::now() }
    }

    fn xdo(&mut self, args: &[&str]) {
        xdo(args);
        self.last_input = Instant::now();
    }

    /// Pointer to the center of a cell.
    fn at(&self, x: u16, y: u16) -> [String; 5] {
        let (px, py) = (x as u32 * 8 + 4, y as u32 * 16 + 8);
        ["mousemove".into(), "--window".into(), self.wid.clone(), px.to_string(), py.to_string()]
    }

    fn step(&mut self, s: Step) {
        match s {
            Step::Keys(_, xk) => {
                let mut a = vec!["key", "--delay", "30"];
                a.extend(xk.split_whitespace());
                self.xdo(&a);
            }
            Step::Type(t) => self.xdo(&["type", "--delay", "30", t]),
            Step::Click(x, y) => {
                let mv = self.at(x, y);
                let mut a: Vec<&str> = mv.iter().map(String::as_str).collect();
                a.extend(["sleep", "0.05", "click", "1"]);
                self.xdo(&a);
            }
            Step::Drag(from, to) => {
                let mut cmds: Vec<String> = self.at(from.0, from.1).to_vec();
                cmds.extend(["mousedown".into(), "1".into()]);
                for (x, y) in line(from, to) {
                    cmds.extend(self.at(x, y));
                }
                cmds.extend(["mouseup".into(), "1".into()]);
                let a: Vec<&str> = cmds.iter().map(String::as_str).collect();
                self.xdo(&a);
            }
            Step::Check(_) => {}
        }
    }

    fn wait_idle(&self, quiet: Duration) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let m = std::fs::metadata(&self.snap).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let since_frame = m.elapsed().unwrap_or_default();
            if since_frame >= quiet && self.last_input.elapsed() >= quiet && self.snap.exists() {
                return;
            }
            assert!(Instant::now() < deadline, "window never went idle");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn frame(&self) -> image::RgbaImage {
        image::open(&self.snap).expect("window snapshot").to_rgba8()
    }

    /// What X actually shows in the window.
    fn screen(&self, path: &Path) -> image::RgbaImage {
        let st = Command::new("import").args(["-window", &self.wid]).arg(path).status().unwrap();
        assert!(st.success());
        image::open(path).unwrap().to_rgba8()
    }
}

fn xdo(args: &[&str]) {
    let st = Command::new("xdotool").args(args).status().expect("xdotool");
    assert!(st.success(), "xdotool {args:?}");
}

/// Intermediate cells of a straight line, like the harness's drag.
fn line(from: (u16, u16), to: (u16, u16)) -> Vec<(u16, u16)> {
    let (dx, dy) = (to.0 as i32 - from.0 as i32, to.1 as i32 - from.1 as i32);
    let steps = dx.abs().max(dy.abs());
    (1..=steps).map(|i| ((from.0 as i32 + dx * i / steps) as u16, (from.1 as i32 + dy * i / steps) as u16)).collect()
}

fn term_step(s: &mut Session, step: Step) {
    match step {
        Step::Keys(k, _) => s.keys(k).unwrap(),
        Step::Type(t) => {
            for c in t.chars() {
                s.type_text(&c.to_string()).unwrap();
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        Step::Click(x, y) => {
            // A real terminal reports motion too (any-event tracking).
            s.mouse(MouseEv::new(MouseKind::Move, x, y)).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            s.click(x, y).unwrap();
        }
        Step::Drag(a, b) => {
            s.mouse(MouseEv::new(MouseKind::Move, a.0, a.1)).unwrap();
            s.drag_with(MouseButton::Left, a, b).unwrap();
        }
        Step::Check(_) => {}
    }
}

/// Cells (8x16 blocks) that differ.
fn diff_cells(a: &image::RgbaImage, b: &image::RgbaImage) -> Vec<(u32, u32)> {
    assert_eq!(a.dimensions(), b.dimensions(), "frame sizes differ");
    let mut out = vec![];
    for cy in 0..a.height() / 16 {
        for cx in 0..a.width() / 8 {
            let differs = (0..16).any(|py| {
                (0..8).any(|px| a.get_pixel(cx * 8 + px, cy * 16 + py) != b.get_pixel(cx * 8 + px, cy * 16 + py))
            });
            if differs {
                out.push((cx, cy));
            }
        }
    }
    out
}

/// Side-by-side: terminal | window | diff mask.
fn save_diff(path: &Path, a: &image::RgbaImage, b: &image::RgbaImage, cells: &[(u32, u32)]) {
    let (w, h) = a.dimensions();
    let mut img = image::RgbaImage::new(w * 3, h);
    image::imageops::overlay(&mut img, a, 0, 0);
    image::imageops::overlay(&mut img, b, w as i64, 0);
    for &(cx, cy) in cells {
        for py in 0..16 {
            for px in 0..8 {
                img.put_pixel(2 * w + cx * 8 + px, cy * 16 + py, image::Rgba([255, 0, 80, 255]));
            }
        }
    }
    img.save(path).unwrap();
}

#[test]
#[ignore = "needs an X server and xdotool"]
fn window_matches_terminal() {
    let steps = [
        Step::Check("01-welcome"),
        Step::Keys("esc", "Escape"),
        Step::Check("02-editor"),
        Step::Drag((40, 8), (70, 18)),
        Step::Check("03-stroke"),
        Step::Keys("ctrl-k", "ctrl+k"),
        Step::Type("grad"),
        Step::Check("04-palette"),
        Step::Keys("esc", "Escape"),
        Step::Keys("?", "question"),
        Step::Check("05-help"),
        Step::Keys("esc", "Escape"),
        Step::Keys("ctrl-z", "ctrl+z"),
        Step::Check("06-undo"),
        Step::Keys("right right down 1 2 3 4", "Right Right Down 1 2 3 4"),
        Step::Check("07-tiles"),
        Step::Click(100, 10),
        Step::Check("08-sidebar-click"),
        Step::Drag((20, 20), (50, 30)),
        Step::Check("09-second-tool"),
    ];
    let out = out_dir();
    let (th, wh) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut term = Session::spawn(
        Path::new(BIN),
        &[],
        COLS,
        ROWS,
        &[("ACIDTRIP_HOME", th.path().to_str().unwrap()), ("HOME", th.path().to_str().unwrap())],
        Some(th.path()),
    )
    .unwrap();
    let mut win = Win::spawn(wh.path(), out.join("window-live.png"));
    term.wait_for_text("Welcome", Duration::from_secs(10)).unwrap();

    let mut failures = vec![];
    for step in steps {
        term_step(&mut term, step);
        win.step(step);
        let Step::Check(name) = step else { continue };
        // Both sides idle, then compare; retry briefly for animation/timers.
        let deadline = Instant::now() + Duration::from_secs(3);
        let (a, b, cells) = loop {
            term.wait_idle(Duration::from_millis(400), Duration::from_secs(15)).unwrap();
            win.wait_idle(Duration::from_millis(400));
            let (a, b) = (term.render(), win.frame());
            let cells = diff_cells(&a, &b);
            if cells.is_empty() || Instant::now() > deadline {
                break (a, b, cells);
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        a.save(out.join(format!("{name}-term.png"))).unwrap();
        b.save(out.join(format!("{name}-window.png"))).unwrap();
        // The pixels on screen must be the frame we rendered.
        let shown = win.screen(&out.join(format!("{name}-xscreen.png")));
        let shown_cells = diff_cells(&b, &shown);
        println!(
            "{name}: {} cells differ (terminal vs window), {} (frame vs X screen)",
            cells.len(),
            shown_cells.len()
        );
        if !cells.is_empty() {
            save_diff(&out.join(format!("{name}-DIFF.png")), &a, &b, &cells);
            failures.push(format!("{name}: {} cells, first {:?}", cells.len(), &cells[..cells.len().min(8)]));
        }
        if !shown_cells.is_empty() {
            failures.push(format!("{name}: on-screen pixels differ from frame in {} cells", shown_cells.len()));
        }
    }
    let _ = term.kill();
    let _ = win.child.kill();
    assert!(failures.is_empty(), "window differs from terminal:\n{}", failures.join("\n"));
}
