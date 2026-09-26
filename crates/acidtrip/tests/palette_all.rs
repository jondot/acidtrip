//! Every command in the palette, one after another, from a fresh document:
//! picked the way a person would (Ctrl-K, arrows to it, Enter), then Esc
//! until nothing is open. The app must never crash or get stuck, Esc must
//! always get back to the canvas, and the canvas must still take a stroke.
//!
//! The command list comes from `acidtrip keys`, which prints the palette's
//! commands in palette order. Shots land in target/shots/palette_all/.

use std::path::{Path, PathBuf};

use acidtrip_harness::{RunOptions, run_script};

const BIN: &str = env!("CARGO_BIN_EXE_acidtrip");

/// Commands this walk doesn't run: Quit ends it, and the font download
/// goes to the network.
const SKIP: &[&str] = &["quit", "get_fonts"];

/// (id, title) of every palette command, in palette order.
fn commands(home: &Path) -> Vec<(String, String)> {
    let out = std::process::Command::new(BIN).arg("keys").env("ACIDTRIP_HOME", home).output().unwrap();
    assert!(out.status.success(), "acidtrip keys failed");
    let text = String::from_utf8(out.stdout).unwrap();
    let mut cat = String::new();
    let mut cmds = vec![];
    for line in text.lines() {
        if let Some(c) = line.strip_prefix("# ") {
            cat = c.to_string();
            continue;
        }
        let Some((id, rest)) = line.split_once(' ') else { continue };
        if id.is_empty() || cat == "Cursor" || cat.starts_with("preset") {
            continue;
        }
        // The title runs up to the key column (two or more spaces).
        let title = rest.trim_start().split("  ").next().unwrap_or("").trim().to_string();
        cmds.push((id.to_string(), title));
    }
    cmds
}

#[test]
fn every_palette_command_returns_to_the_canvas() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let cmds = commands(home.path());
    assert!(cmds.len() > 100, "only {} commands parsed", cmds.len());
    let n = cmds.len();
    let mut s = String::from(
        "size 120 40\nenv EDITOR true\nenv VISUAL true\nenv ACIDTRIP_NET_LOCAL 1\nspawn\nwait \"Welcome\" 10s\nkeys esc\nwait idle\n",
    );
    // script line -> command id, to name the command a failure happened in
    let mut owner = vec![String::new(); s.lines().count()];
    for (i, (id, title)) in cmds.iter().enumerate() {
        if SKIP.contains(&id.as_str()) {
            continue;
        }
        let moves = if i <= n - i { "down ".repeat(i) } else { "up ".repeat(n - i) };
        // Long titles are cut to fit the palette; the start is enough.
        let head: String = title.chars().take(18).collect();
        let step = format!(
            "keys ctrl-k\nwait \"Commands\"\n{}wait \"▸{head}\"\nkeys enter\n\
             keys esc esc esc\nwait idle\nexpect-not \"╭\"\n",
            if moves.is_empty() { String::new() } else { format!("keys {moves}\n") },
        );
        owner.extend(std::iter::repeat_n(id.clone(), step.lines().count()));
        s.push_str(&step);
    }
    // A new document (discarding all that), the brush: a stroke still paints.
    s.push_str(
        "keys ctrl-n\nwait \"New document\"\nkeys enter\nwait \"Confirm\"\nkeys y\nwait \"new 80x25\"\n\
         keys ctrl-k\nwait \"Commands\"\ntype brush\nwait \"▸Brush\"\nkeys enter\nwait idle\n\
         expect \"TOOLS\"\nexpect \"acidtrip\"\nkeys 4\nwait idle\ndrag 3 5 12 5\nwait idle\nexpect \"██████████\"\nshot end\n",
    );
    let shots = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/palette_all");
    std::fs::create_dir_all(&shots).unwrap();
    let mut opts = RunOptions::new(PathBuf::from(BIN), shots);
    opts.home = Some(home.path().to_path_buf());
    opts.cwd = Some(cwd.path().to_path_buf());
    if let Err(e) = run_script(&s, &opts) {
        let msg = format!("{e:#}");
        let at = msg
            .split("line ")
            .nth(1)
            .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|l| owner.get(l.saturating_sub(1)))
            .filter(|id| !id.is_empty())
            .map(|id| format!(" (while running `{id}`)"))
            .unwrap_or_default();
        panic!("{msg}{at}");
    }
}
