//! End-to-end tests driving the real binary in a pseudo-terminal.
//! Screenshots land in target/shots/<test>/ for visual review.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use acidtrip_harness::{RunOptions, Session, run_script};

const BIN: &str = env!("CARGO_BIN_EXE_acidtrip");

/// These tests each drive a real app in a pseudo-terminal; running them in
/// parallel (debug builds) starves the apps and makes timing flaky. Serialize.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn shots(name: &str) -> PathBuf {
    let d = workspace().join("target/shots").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn spawn(home: &Path, args: &[&str]) -> Session {
    Session::spawn(
        Path::new(BIN),
        args,
        120,
        40,
        &[("ACIDTRIP_HOME", home.to_str().unwrap()), ("HOME", home.to_str().unwrap())],
        Some(home),
    )
    .unwrap()
}

fn t(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// Wait until the UI is up (the logo in the tab bar) and quiet.
fn ready(s: &mut Session) {
    s.wait_for_text("acidtrip", t(15)).unwrap();
    s.wait_idle(Duration::from_millis(200), t(5)).unwrap();
}

fn dismiss_welcome(s: &mut Session) {
    s.wait_for_text("Welcome to acidtrip", t(10)).unwrap();
    s.keys("esc").unwrap();
    s.wait_idle(Duration::from_millis(200), t(5)).unwrap();
}

/// Every scripted scenario in tests/e2e/*.at must pass.
#[test]
fn scripted_scenarios() {
    let _serial = serial();
    let dir = workspace().join("tests/e2e");
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "at"))
        .collect();
    scripts.sort();
    assert!(scripts.len() >= 10, "expected the scenario scripts in {}", dir.display());
    std::fs::remove_dir_all("/tmp/acidtrip-e2e").ok();
    std::fs::create_dir_all("/tmp/acidtrip-e2e").ok();
    let mut failures = vec![];
    for s in scripts {
        let name = s.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&s).unwrap();
        let mut opts = RunOptions::new(PathBuf::from(BIN), shots(&name));
        // Scripts use workspace-relative paths (corpus files, etc.).
        opts.cwd = Some(workspace());
        if let Err(e) = run_script(&text, &opts) {
            failures.push(format!("{name}: {e:#}"));
        }
    }
    assert!(failures.is_empty(), "scenario failures:\n{}", failures.join("\n\n"));
}

/// kill -9 with unsaved work → next start offers recovery → restored.
#[test]
fn crash_recovery() {
    let _serial = serial();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("config")).unwrap();
    std::fs::write(home.path().join("config/config.toml"), "autosave_seconds = 1\n").unwrap();
    let mut s = spawn(home.path(), &[]);
    ready(&mut s);
    s.keys("esc t").unwrap();
    s.type_text("UNSAVED MASTERPIECE").unwrap();
    s.keys("esc").unwrap();
    // Let an autosave happen (1s interval).
    std::thread::sleep(Duration::from_millis(2500));
    s.kill().unwrap();
    let rec = home.path().join("state/recovery");
    assert!(std::fs::read_dir(&rec).unwrap().count() >= 2, "recovery files written");

    let mut s = spawn(home.path(), &[]);
    s.wait_for_text("Recover unsaved work", t(10)).unwrap();
    s.screenshot(&shots("recovery").join("prompt.png")).unwrap();
    s.keys("enter").unwrap();
    s.wait_for_text("UNSAVED MASTERPIECE", t(5)).unwrap();
    s.wait_for_text("recovered", t(5)).unwrap();
    s.screenshot(&shots("recovery").join("restored.png")).unwrap();
}

/// Claude Code's view: `acidtrip mcp` attaches to the running editor and
/// its drawing appears live, undoable.
#[test]
fn mcp_live_attach() {
    let _serial = serial();
    let home = tempfile::tempdir().unwrap();
    let mut s = spawn(home.path(), &[]);
    dismiss_welcome(&mut s);

    let mut mcp = Command::new(BIN)
        .arg("mcp")
        .env("ACIDTRIP_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = mcp.stdin.take().unwrap();
    let mut out = BufReader::new(mcp.stdout.take().unwrap());
    let mut call = |id: u64, method: &str, params: serde_json::Value| -> serde_json::Value {
        let msg = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = call(
        1,
        "initialize",
        serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}}),
    );
    let instructions = init["result"]["instructions"].as_str().unwrap_or_default().to_lowercase();
    assert!(instructions.contains("live"), "attached to the live editor: {instructions}");
    let tools = call(2, "tools/list", serde_json::json!({}));
    assert!(tools["result"]["tools"].as_array().unwrap().len() >= 25);
    let r = call(
        3,
        "tools/call",
        serde_json::json!({"name": "draw_box", "arguments": {"x": 2, "y": 2, "w": 30, "h": 7, "style": "double", "fg": 14, "bg": 1, "filled": true}}),
    );
    assert_ne!(r["result"]["isError"], serde_json::json!(true), "{r}");
    let r = call(
        4,
        "tools/call",
        serde_json::json!({"name": "put_text", "arguments": {"x": 6, "y": 5, "text": "HELLO FROM MCP", "fg": 15, "bg": 1}}),
    );
    assert_ne!(r["result"]["isError"], serde_json::json!(true), "{r}");
    let r = call(5, "tools/call", serde_json::json!({"name": "render_png", "arguments": {}}));
    let has_image = r["result"]["content"].as_array().unwrap().iter().any(|c| c["type"] == "image");
    assert!(has_image, "render_png returns an image block");

    s.wait_for_text("HELLO FROM MCP", t(5)).unwrap();
    s.wait_for_text("╔", t(5)).unwrap();
    s.screenshot(&shots("mcp_live").join("drawn.png")).unwrap();
    // The AI edits went to an "AI" layer.
    assert!(s.screen_contains("AI"));
    // One undo removes the last AI step.
    s.keys("ctrl-z").unwrap();
    s.wait_for_text("undo: AI: put_text", t(5)).unwrap();
    assert!(!s.screen_contains("HELLO FROM MCP"));
    drop(stdin);
    let _ = mcp.wait();
}

/// Saving snapshots a version; the browser lists and restores it.
#[test]
fn versions_browser() {
    let _serial = serial();
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("v.acid");
    let f = file.to_str().unwrap();
    let mut s = spawn(home.path(), &[f]);
    ready(&mut s);
    s.keys("esc t").unwrap();
    s.type_text("FIRST").unwrap();
    s.keys("esc ctrl-s").unwrap();
    s.wait_for_text("saved", t(5)).unwrap();
    s.keys("t").unwrap();
    s.type_text(" SECOND").unwrap();
    s.keys("esc ctrl-s").unwrap();
    s.wait_idle(Duration::from_millis(300), t(5)).unwrap();
    s.keys("alt-v").unwrap();
    s.wait_for_text("Version history", t(5)).unwrap();
    s.screenshot(&shots("versions").join("browser.png")).unwrap();
    // Newest first: go to the older one and restore it.
    s.keys("down enter").unwrap();
    s.wait_for_text("restored version", t(5)).unwrap();
    assert!(!s.screen_contains("SECOND"));
    s.screenshot(&shots("versions").join("restored.png")).unwrap();
}

/// CLI: convert between formats and render.
#[test]
fn cli_convert_everything() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let src = workspace().join("crates/acidtrip-io/tests/corpus");
    let ans = std::fs::read_dir(&src)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ans")))
        .unwrap();
    for ext in [
        "xb", "bin", "adf", "idf", "tnd", "pcb", "avt", "asc", "utf8ans", "acid", "png", "gif", "svg", "html", "tsx",
        "c", "cast",
    ] {
        let out = dir.path().join(format!("out.{ext}"));
        let st = Command::new(BIN)
            .args(["convert", ans.to_str().unwrap(), out.to_str().unwrap()])
            .env("ACIDTRIP_HOME", dir.path())
            .output()
            .unwrap();
        assert!(st.status.success(), "{ext}: {}", String::from_utf8_lossy(&st.stderr));
        assert!(std::fs::metadata(&out).unwrap().len() > 0, "{ext} empty");
    }
    // Loadable ones come back.
    for ext in ["xb", "bin", "adf", "idf", "tnd", "acid"] {
        let out = dir.path().join(format!("out.{ext}"));
        let back = dir.path().join(format!("back-{ext}.ans"));
        let st = Command::new(BIN)
            .args(["convert", out.to_str().unwrap(), back.to_str().unwrap()])
            .env("ACIDTRIP_HOME", dir.path())
            .output()
            .unwrap();
        assert!(st.status.success(), "{ext} reload: {}", String::from_utf8_lossy(&st.stderr));
    }
}

/// The keyboard cursor blinks (ACiDDraw's flashing cursor) so a full block
/// under it stays visible, and the status bar names the cell under it.
#[test]
fn cursor_blinks_over_blocks() {
    let _serial = serial();
    let home = tempfile::tempdir().unwrap();
    let mut s = Session::spawn(
        Path::new(BIN),
        &[],
        120,
        40,
        &[("ACIDTRIP_HOME", home.path().to_str().unwrap()), ("ACIDTRIP_NO_BLINK", "0")],
        Some(home.path()),
    )
    .unwrap();
    s.wait_for_text("Welcome to acidtrip", t(10)).unwrap();
    s.keys("esc").unwrap();
    // Esc + a quick key reads as Alt-<key> in terminals; let the Esc land first.
    s.wait_idle(Duration::from_millis(200), t(5)).unwrap();
    // Place a full block, then step back onto it.
    s.keys("4 left").unwrap();
    s.wait_for_text("[█] 219 15/0", t(5)).unwrap();
    // Canvas cell (0,0) is screen (0,1). Sample it over ~1.5s: both phases appear.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..30 {
        let c = s.cell(0, 1).unwrap();
        seen.insert((c.fg, c.bg));
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(seen.len() >= 2, "cursor should alternate between inverted and the real cell: {seen:?}");
    s.screenshot(&shots("cursor").join("blink.png")).unwrap();
}

/// With a graphics-capable terminal the minimap and layer thumbnails are real
/// pixel images (iTerm2 inline image protocol here), not half blocks.
#[test]
fn pixel_previews_use_terminal_graphics() {
    let _serial = serial();
    let home = tempfile::tempdir().unwrap();
    let art = workspace().join("crates/acidtrip-io/tests/corpus/LDA-ANSIACADEMY.ANS");
    let mut s = Session::spawn(
        Path::new(BIN),
        &[art.to_str().unwrap()],
        160,
        40,
        &[("ACIDTRIP_HOME", home.path().to_str().unwrap()), ("ACIDTRIP_GRAPHICS", "iterm2")],
        Some(home.path()),
    )
    .unwrap();
    s.wait_for_text("LDA-ANSIACADEMY", t(10)).unwrap();
    s.keys("esc").unwrap();
    // Images are built after the first frame; poll for them.
    let deadline = std::time::Instant::now() + t(15);
    let (mut out, mut images) = (String::new(), 0);
    while std::time::Instant::now() < deadline {
        out = String::from_utf8_lossy(&s.raw_output()).into_owned();
        images = out.matches("\x1b]1337;File=").count();
        if images >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(images >= 2, "expected minimap + layer thumbnail images, got {images}");
    // Save the decoded images for review: ESC]1337;File=...:<base64> BEL
    use base64::Engine;
    for (k, part) in out.split("\x1b]1337;File=").skip(1).enumerate() {
        let Some(payload) = part.split_once(':').map(|(_, b)| b) else { continue };
        let b64: String = payload.chars().take_while(|c| *c != '\x07' && *c != '\x1b').collect();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
            std::fs::write(shots("pixel_previews").join(format!("image{k}.png")), bytes).unwrap();
        }
    }
    // A popup must not swap the pixel previews for the half-block fallback.
    let half_blocks = |s: &Session| {
        let (w, h) = s.size();
        (0..h)
            .flat_map(|y| (w - 30..w).map(move |x| (x, y)))
            .filter(|&(x, y)| s.cell(x, y).is_some_and(|c| c.ch == '▀'))
            .count()
    };
    s.wait_idle(Duration::from_millis(300), t(5)).unwrap();
    let before = half_blocks(&s);
    s.keys("?").unwrap();
    s.wait_for_text("any other key closes", t(5)).unwrap();
    s.wait_idle(Duration::from_millis(300), t(5)).unwrap();
    s.screenshot(&shots("pixel_previews").join("with_popup.png")).unwrap();
    assert_eq!(half_blocks(&s), before, "previews fell back to half blocks under a popup");
}

/// Import image: pick a file from the list, see the preview, switch preset,
/// import as a layer; then the same file as a reference layer.
#[test]
fn import_image_dialog() {
    let _serial = serial();
    let home = tempfile::tempdir().unwrap();
    // A small "photo": a gradient sky with a red disc.
    let img = image::RgbImage::from_fn(160, 80, |x, y| {
        let (dx, dy) = (x as i32 - 80, y as i32 - 40);
        if dx * dx + dy * dy < 20 * 20 {
            image::Rgb([220, 30, 30])
        } else {
            image::Rgb([(x * 255 / 160) as u8, 120, (y * 255 / 80) as u8])
        }
    });
    img.save(home.path().join("sunset.png")).unwrap();
    let mut s = spawn(home.path(), &[]);
    dismiss_welcome(&mut s);
    let dir = shots("import_image");
    s.keys("ctrl-k").unwrap();
    s.wait_for_text("Commands", t(5)).unwrap();
    s.type_text("Import image").unwrap();
    s.keys("enter").unwrap();
    s.wait_for_text("sunset.png", t(5)).unwrap();
    s.screenshot(&dir.join("picker.png")).unwrap();
    s.keys("enter").unwrap();
    s.wait_for_text("PRESET", t(5)).unwrap();
    s.wait_for_text("160×80", t(5)).unwrap();
    assert!(s.screen_contains("best-fit blocks"), "photo preset by default:\n{}", s.screen_text());
    s.keys("3").unwrap();
    s.wait_for_text("half blocks", t(5)).unwrap();
    // Cel keeps outlines: its "Keep lines" row is on.
    s.keys("4").unwrap();
    s.wait_for_text("Keep lines", t(5)).unwrap();
    s.wait_for_text("80%", t(5)).unwrap();
    s.screenshot(&dir.join("cel.png")).unwrap();
    s.keys("1").unwrap();
    s.wait_for_text("best-fit blocks", t(5)).unwrap();
    // First row is "Into": stamp → new layer.
    s.keys("right").unwrap();
    s.wait_for_text("new layer", t(5)).unwrap();
    s.wait_idle(Duration::from_millis(300), t(10)).unwrap();
    s.screenshot(&dir.join("options.png")).unwrap();
    s.keys("enter").unwrap();
    s.wait_for_text("imported sunset as a layer", t(10)).unwrap();
    s.screenshot(&dir.join("imported.png")).unwrap();

    s.keys("ctrl-k").unwrap();
    s.wait_for_text("Commands", t(5)).unwrap();
    s.type_text("reference image").unwrap();
    s.keys("enter").unwrap();
    s.wait_for_text("Reference image layer", t(5)).unwrap();
    s.type_text("sunset").unwrap();
    s.keys("enter").unwrap();
    s.wait_for_text("Add layer", t(5)).unwrap();
    assert!(!s.screen_contains("stamp (place it)"), "reference always makes a layer");
    s.keys("enter").unwrap();
    s.wait_for_text("Drawing", t(10)).unwrap();
    s.screenshot(&dir.join("reference.png")).unwrap();
}

/// Draw together across processes: the test hosts a session in-process (as
/// the host app would: apply, then pass every edit on), and two real apps
/// join it by pasting the ticket and clicking Join. What anyone draws shows
/// up for everyone, the others' cursors carry their names, undo takes back
/// only your own stroke, and the host ending it tells the guests.
#[test]
fn draw_together_two_apps() {
    use acidtrip_core::{Cell, Color, DocKind, Document, TxBuilder};
    use acidtrip_harness::{MouseEv, MouseKind};
    use acidtrip_net::{Event, Options, Session as Net};
    use std::sync::{Arc, Mutex, mpsc};

    let _serial = serial();
    let dir = shots("draw_together");
    let doc = Arc::new(Mutex::new(Document::new(DocKind::Classic, 80, 25)));
    let (ticket_tx, ticket_rx) = mpsc::channel();
    let (draw_tx, draw_rx) = mpsc::channel::<Option<(usize, usize, String)>>();
    let host = {
        let doc = doc.clone();
        std::thread::spawn(move || {
            let net = Net::host(Options { name: "ada".into(), localhost: true }, doc.lock().unwrap().clone());
            loop {
                while let Some(e) = net.poll() {
                    match e {
                        Event::Ticket(t) => ticket_tx.send(t).unwrap(),
                        Event::Edit(tx) => {
                            doc.lock().unwrap().apply(&tx);
                            net.edit(tx);
                        }
                        _ => {}
                    }
                }
                match draw_rx.try_recv() {
                    Ok(Some((x, y, text))) => {
                        let mut d = doc.lock().unwrap();
                        let mut b = TxBuilder::new(&d, "type");
                        for (i, ch) in text.chars().enumerate() {
                            b.set(0, x + i, y, Some(Cell::new(ch, Color::Pal(14), Color::BLACK)));
                        }
                        let tx = b.finish();
                        d.apply(&tx);
                        net.edit(tx);
                    }
                    Ok(None) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            net.leave();
        })
    };
    let ticket: String = ticket_rx.recv_timeout(t(20)).unwrap();

    let homes = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let join = |home: &Path, name: &str| {
        let env = [("ACIDTRIP_HOME", home.to_str().unwrap()), ("ACIDTRIP_NET_LOCAL", "1"), ("USER", name)];
        let mut s = Session::spawn(Path::new(BIN), &[], 120, 40, &env, Some(home)).unwrap();
        dismiss_welcome(&mut s);
        // Pasting a ticket anywhere opens the panel with it filled in.
        s.send_raw(format!("\x1b[200~{ticket}\x1b[201~").as_bytes()).unwrap();
        s.wait_for_text("TOGETHER", t(5)).unwrap();
        s.wait_idle(Duration::from_millis(200), t(5)).unwrap();
        s.click(115, 7).unwrap();
        s.wait_for_text("joined ada's drawing", t(20)).unwrap();
        s
    };
    let at = |x: usize, y: usize| doc.lock().unwrap().canvas.get(0, x, y).is_some_and(|c| c.ch != ' ');
    let until = |what: &str, ok: &dyn Fn() -> bool| {
        let start = std::time::Instant::now();
        while !ok() {
            assert!(start.elapsed() < t(15), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    let mut bob = join(homes[0].path(), "bob");
    bob.wait_for_text("● ada", t(5)).unwrap();
    bob.wait_for_text("bob (you)", t(5)).unwrap();
    // The host's drawing reaches bob (canvas row y is screen row y + 1).
    draw_tx.send(Some((2, 2, "HELLO FROM ADA".into()))).unwrap();
    bob.wait_for_text("HELLO FROM ADA", t(10)).unwrap();
    // Bob's stroke reaches the host.
    bob.drag((5, 11), (25, 11)).unwrap();
    until("bob's stroke at the host", &|| at(5, 10) && at(25, 10));

    let mut cy = join(homes[1].path(), "cy");
    cy.wait_for_text("HELLO FROM ADA", t(5)).unwrap();
    cy.wait_for_text("● bob", t(5)).unwrap();
    // Bob's cursor shows on cy's canvas with his name beside it.
    bob.mouse(MouseEv::new(MouseKind::Move, 40, 20)).unwrap();
    let row = |s: &Session, y: usize| {
        s.screen_text().lines().nth(y).unwrap_or("").chars().skip(41).take(5).collect::<String>()
    };
    let start = std::time::Instant::now();
    while row(&cy, 20) != " bob " {
        assert!(start.elapsed() < t(10), "bob's cursor on cy's screen:\n{}", cy.screen_text());
        std::thread::sleep(Duration::from_millis(50));
    }
    cy.screenshot(&dir.join("cy_sees_bob.png")).unwrap();

    // Cy writes over part of bob's stroke (in ▀); bob's undo takes back
    // only the cells that still hold what he wrote.
    cy.keys("5").unwrap();
    cy.drag((20, 11), (30, 11)).unwrap();
    until("cy's stroke at the host", &|| at(30, 10));
    bob.keys("ctrl-z").unwrap();
    until("bob's undo", &|| !at(5, 10) && !at(19, 10));
    assert!(at(20, 10) && at(30, 10), "cy's cells survive bob's undo");
    assert!(at(2, 2), "ada's text survives bob's undo");
    cy.wait_idle(Duration::from_millis(300), t(10)).unwrap();
    cy.screenshot(&dir.join("after_undo.png")).unwrap();

    // Bob replays the session so far while ada keeps drawing: the replay
    // stays up, and the live piece has her new text when he's back.
    bob.keys("alt-shift-r").unwrap();
    bob.wait_for_text("REPLAY", t(5)).unwrap();
    draw_tx.send(Some((2, 6, "LIVE FROM ADA".into()))).unwrap();
    cy.wait_for_text("LIVE FROM ADA", t(10)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(bob.screen_contains("REPLAY"), "replay stays up:\n{}", bob.screen_text());
    bob.screenshot(&dir.join("bob_replays.png")).unwrap();
    bob.keys("esc").unwrap();
    bob.wait_for_text("LIVE FROM ADA", t(5)).unwrap();
    assert!(!bob.screen_contains("REPLAY"));

    // The host ends it and the guests hear why.
    draw_tx.send(None).unwrap();
    host.join().unwrap();
    bob.wait_for_text("ada ended the session", t(15)).unwrap();
    cy.wait_for_text("ada ended the session", t(15)).unwrap();
    bob.screenshot(&dir.join("ended.png")).unwrap();
}
