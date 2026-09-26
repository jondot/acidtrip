use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use acidtrip_harness::{RunOptions, Session, run_script};

const SH: &str = "/bin/sh";

fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("acidtrip-harness-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn colored_text_and_screenshot() {
    let s = Session::spawn(
        Path::new(SH),
        &["-c", "printf '\\033[31mhello\\033[0m \\033[1;38;2;10;200;30mrgb\\033[0m'; sleep 5"],
        120,
        40,
        &[],
        None,
    )
    .unwrap();
    s.wait_for_text("hello", Duration::from_secs(5)).unwrap();
    s.wait_for_text("rgb", Duration::from_secs(5)).unwrap();
    let c = s.cell(0, 0).unwrap();
    assert_eq!(c.ch, 'h');
    assert_eq!(c.fg, [0xAA, 0, 0], "ANSI red");
    assert_eq!(c.bg, acidtrip_harness::DEFAULT_BG);
    let g = s.cell(6, 0).unwrap();
    assert_eq!((g.ch, g.fg, g.bold), ('r', [10, 200, 30], true));
    assert!(s.screen_text().starts_with("hello rgb\n"));

    let png = out_dir("color").join("hello.png");
    s.screenshot(&png).unwrap();
    let img = image::open(&png).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (120 * 8, 40 * 16));
    // Some red glyph pixels in the first cell, background elsewhere.
    let red = (0..8)
        .flat_map(|x| (0..16).map(move |y| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0 == [0xAA, 0, 0, 255])
        .count();
    assert!(red > 10, "red pixels: {red}");
    assert_eq!(img.get_pixel(500, 300).0, [12, 12, 16, 255]);
}

#[test]
fn cat_echo() {
    let mut s = Session::spawn(Path::new("/bin/cat"), &[], 80, 24, &[], None).unwrap();
    s.type_text("hello from the harness").unwrap();
    s.wait_for_text("hello from the harness", Duration::from_secs(5)).unwrap();
    assert!(s.is_alive());
    s.keys("enter ctrl-d").unwrap();
    assert!(s.wait_exit(Duration::from_secs(5)).unwrap().is_some());
    assert!(!s.is_alive());
}

#[test]
fn wait_for_text_error_includes_screen() {
    let s = Session::spawn(Path::new(SH), &["-c", "echo visible-thing; sleep 5"], 80, 24, &[], None).unwrap();
    s.wait_for_text("visible-thing", Duration::from_secs(5)).unwrap();
    let e = s.wait_for_text("absent", Duration::from_millis(100)).unwrap_err().to_string();
    assert!(e.contains("visible-thing"), "{e}");
}

#[test]
fn kill_and_resize() {
    let mut s = Session::spawn(
        Path::new(SH),
        &["-c", "trap 'echo resized' WINCH; while true; do sleep 0.05; done"],
        80,
        24,
        &[],
        None,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    s.resize(100, 30).unwrap();
    s.wait_for_text("resized", Duration::from_secs(5)).unwrap();
    assert_eq!(s.size(), (100, 30));
    s.kill().unwrap();
    assert!(!s.is_alive());
}

fn probe_bin() -> PathBuf {
    // Examples are built by `cargo test` into <target>/<profile>/examples.
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().and_then(Path::parent).unwrap();
    let p = profile_dir.join("examples").join(if cfg!(windows) { "probe.exe" } else { "probe" });
    if !p.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let st = std::process::Command::new(cargo)
            .args(["build", "-p", "acidtrip-harness", "--example", "probe"])
            .status()
            .unwrap();
        assert!(st.success());
    }
    assert!(p.exists(), "probe example not found at {}", p.display());
    p
}

#[test]
fn responder_unblocks_crossterm() {
    let t0 = Instant::now();
    let mut s = Session::spawn(&probe_bin(), &[], 120, 40, &[], None).unwrap();
    s.wait_for_text("READY", Duration::from_secs(3)).unwrap();
    let text = s.screen_text();
    assert!(text.contains("enh=false"), "{text}");
    assert!(text.contains("pos=9,4"), "{text}");
    // Queries must be answered promptly, not via crossterm's 2s timeout.
    let ms = |key: &str| -> u64 {
        let l = text.lines().find(|l| l.starts_with(key)).unwrap();
        l.split('(').nth(1).unwrap().trim_end_matches("ms)").parse().unwrap()
    };
    assert!(ms("enh=") < 1000, "{text}");
    assert!(ms("pos=") < 1000, "{text}");
    if let Err(e) = s.click(7, 3) {
        std::thread::sleep(Duration::from_millis(300));
        panic!(
            "click failed: {e:#}; alive={} exit={:?}\n{}",
            s.is_alive(),
            s.wait_exit(Duration::from_millis(10)),
            s.screen_text()
        );
    }
    s.wait_for_text("mouse=Up", Duration::from_secs(3)).unwrap();
    let text = s.screen_text();
    assert!(text.contains("mouse=Down(Left) 7,3\nmouse=Up(Left) 7,3"), "{text}");
    assert_eq!(s.wait_exit(Duration::from_secs(3)).unwrap(), Some(0));
    assert!(t0.elapsed() < Duration::from_secs(3), "took {:?}", t0.elapsed());
}

#[test]
fn script_runner_on_sh() {
    let out = out_dir("script");
    let script = r#"
# a tiny interactive shell loop
spawn -c "printf 'name? '; read n; printf '\033[32mhi %s\033[0m\n' \"$n\"; printf 'home=%s\n' \"$ACIDTRIP_HOME\"; read x"
wait "name?"
type bob
keys enter
wait "hi bob" 3s
expect "hi bob"
expect-not "hi alice"
wait idle
shot greeting
keys enter
expect-exit 0
"#;
    let mut opts = RunOptions::new(SH, &out);
    opts.cols = 80;
    opts.rows = 24;
    let r = run_script(script, &opts).unwrap();
    assert_eq!(r.shots, vec![out.join("greeting.png")]);
    let img = image::open(out.join("greeting.png")).unwrap();
    assert_eq!((img.width(), img.height()), (80 * 8, 24 * 16));
    let txt = std::fs::read_to_string(out.join("greeting.txt")).unwrap();
    assert!(txt.contains("hi bob"));
    let home = r.home.unwrap();
    assert!(txt.contains(&format!("home={}", home.display())), "{txt}");
}

#[test]
fn script_failure_writes_failure_shot() {
    let out = out_dir("fail");
    let script = "spawn -c \"echo something; sleep 5\"\nwait \"something\"\nexpect \"nothing\"\n";
    let e = run_script(script, &RunOptions::new(SH, &out)).unwrap_err().to_string();
    assert!(e.contains("line 3"), "{e}");
    assert!(e.contains("something"), "{e}");
    assert!(out.join("FAILURE.png").exists());
    assert!(std::fs::read_to_string(out.join("FAILURE.txt")).unwrap().contains("something"));
}
