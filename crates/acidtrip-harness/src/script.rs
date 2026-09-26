//! Line-based script runner.
//!
//! ```text
//! # comment
//! spawn [args...]         # spawn the app; ACIDTRIP_HOME = --home or a fresh tempdir
//! size 120 40             # before spawn: pty size; after: resize
//! keys ctrl-k esc
//! type hello world        # or: type "quoted \t text"
//! mouse down left 10 5 [ctrl] [alt] [shift]
//! mouse drag left 12 6 | mouse up left 12 6 | mouse move 3 3 | mouse scroll up 10 5
//! click 10 5 [right|middle]
//! click-text "text" [N]   # left-click the middle of the Nth (default 1st) "text" on screen
//! drag 10 5 20 8 [right|middle]
//! wait 200ms | wait idle [quiet] | wait "text" [timeout]
//! expect "text"
//! expect-not "text"
//! shot NAME               # DIR/NAME.png + DIR/NAME.txt
//! kill                    # SIGKILL
//! expect-exit [code] [timeout]
//! env NAME VALUE          # environment for later spawns (overrides earlier values)
//! fake-claude FILE        # serve FILE's canned responses as the Claude API (see fake_claude)
//! expect-claude-calls N [timeout]  # the fake got at least N requests
//! expect-file PATH [PATH...]      # each file exists and isn't empty
//! expect-no-file PATH [PATH...]   # none of them exists
//! expect-file-contains PATH "text" # the file's bytes contain the text
//! expect-file-not-contains PATH "text" # the file does not
//! write-file PATH "text"          # create (or replace) a file, dirs included
//! chmod 555 PATH                  # set permissions (unix); leave nothing that blocks cleanup
//! wait-file PATH [timeout]         # until PATH is a non-empty file; `dir/*.json` any match
//! home-file REL "text"    # write a file under the app's home (before `spawn`: the next one's)
//! restart [args...]       # stop the app and spawn it again with the same home
//! session NAME            # switch to (or start) another app slot, each with its own home;
//!                         # the first slot is "main"
//! paste "text"            # a bracketed paste, as a terminal sends it
//! paste-clipboard [NAME]  # paste what session NAME (default: this one) last copied via OSC 52
//! expect-cell X Y "g" [fg=C] [bg=C]  # screen cell glyph ("*" = any); C = VGA index 0-15 or #rrggbb
//! ```
//!
//! A `PATH` starting with `~/` is under the current slot's home.
//!
//! Relative paths (`fake-claude FILE`, spawn args) resolve against the app's
//! working directory, `RunOptions::cwd` (default: the current dir).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::fake_claude::FakeClaude;
use crate::keys::Mods;
use crate::session::{MouseButton, MouseEv, MouseKind, Session, framed};

#[derive(Clone, Debug)]
pub struct RunOptions {
    /// Where `shot` and failure screenshots go.
    pub out_dir: PathBuf,
    /// The program `spawn` launches.
    pub bin: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// ACIDTRIP_HOME for the app. `None` = a fresh tempdir per run.
    pub home: Option<PathBuf>,
    /// Working directory of the app (default: the current dir).
    pub cwd: Option<PathBuf>,
    /// Extra environment for the app.
    pub env: Vec<(String, String)>,
    /// Timeout for `wait "text"`, `wait idle`, `expect-exit`.
    pub default_timeout: Duration,
    /// Print each step to stderr.
    pub verbose: bool,
}

impl RunOptions {
    pub fn new(bin: impl Into<PathBuf>, out_dir: impl Into<PathBuf>) -> Self {
        RunOptions {
            out_dir: out_dir.into(),
            bin: bin.into(),
            cols: 120,
            rows: 40,
            home: None,
            cwd: None,
            env: Vec::new(),
            default_timeout: Duration::from_secs(5),
            verbose: false,
        }
    }
}

#[derive(Debug)]
pub struct RunReport {
    /// PNGs written by `shot`.
    pub shots: Vec<PathBuf>,
    /// Number of commands executed.
    pub steps: usize,
    /// The ACIDTRIP_HOME used by the last spawn.
    pub home: Option<PathBuf>,
    /// Final screen text (empty if nothing was spawned).
    pub final_screen: String,
    /// Keeps a temporary home alive until the report is dropped.
    pub temp_home: Option<tempfile::TempDir>,
}

/// Workspace root, derived from this crate's location.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|_| PathBuf::from("."))
}

/// Default app binary: `$CARGO_TARGET_DIR/debug/acidtrip` or
/// `<workspace>/target/debug/acidtrip`.
pub fn default_app_bin() -> PathBuf {
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| workspace_root().join("target"));
    let target = if target.is_relative() { workspace_root().join(target) } else { target };
    target.join("debug").join(if cfg!(windows) { "acidtrip.exe" } else { "acidtrip" })
}

/// Run a script. On failure, writes `FAILURE.png` / `FAILURE.txt` to the
/// out dir (when a session exists) and returns an error naming the line.
pub fn run_script(script: &str, opts: &RunOptions) -> Result<RunReport> {
    let mut r = Runner {
        opts,
        cols: opts.cols,
        rows: opts.rows,
        session: None,
        home: None,
        spawned: false,
        slot: "main".to_string(),
        parked: HashMap::new(),
        homes: Vec::new(),
        env: Vec::new(),
        fake: None,
        report: RunReport { shots: vec![], steps: 0, home: None, final_screen: String::new(), temp_home: None },
    };
    for (idx, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if opts.verbose {
            eprintln!("[{:>3}] {line}", idx + 1);
        }
        if let Err(e) = r.exec(line) {
            let mut msg = format!("line {}: `{line}` failed: {e:#}", idx + 1);
            if let Some(s) = &r.session {
                let _ = std::fs::create_dir_all(&opts.out_dir);
                let png = opts.out_dir.join("FAILURE.png");
                let text = s.screen_text();
                let _ = s.screenshot(&png);
                let _ = std::fs::write(opts.out_dir.join("FAILURE.txt"), &text);
                msg.push_str(&format!("\nfailure screenshot: {}", png.display()));
            }
            return Err(anyhow!(msg));
        }
        r.report.steps += 1;
    }
    if let Some(s) = &r.session {
        r.report.final_screen = s.screen_text();
    }
    Ok(r.report)
}

struct Runner<'a> {
    opts: &'a RunOptions,
    cols: u16,
    rows: u16,
    session: Option<Session>,
    /// This slot's home, once `home-file` or `spawn` made one.
    home: Option<PathBuf>,
    /// Whether an app already ran in `home` (a later `spawn` starts afresh).
    spawned: bool,
    /// The current slot's name, and the others (session, home, spawned).
    slot: String,
    parked: HashMap<String, Slot>,
    /// Temporary homes, kept until the run ends.
    homes: Vec<tempfile::TempDir>,
    /// Set by `env` / `fake-claude`; applied after `RunOptions::env`.
    env: Vec<(String, String)>,
    /// Dropped after `session`, so the app never outlives its fake API.
    fake: Option<FakeClaude>,
    report: RunReport,
}

type Slot = (Option<Session>, Option<PathBuf>, bool);

impl Runner<'_> {
    /// The home for the next spawn: a fixed one, this slot's (prepared by
    /// `home-file`, or kept by `restart`), or a fresh tempdir.
    fn home_for_spawn(&mut self, reuse: bool) -> Result<PathBuf> {
        if let Some(h) = &self.opts.home {
            std::fs::create_dir_all(h)?;
            return Ok(h.clone());
        }
        if let Some(h) = &self.home
            && (reuse || !self.spawned)
        {
            return Ok(h.clone());
        }
        self.new_home()
    }

    fn new_home(&mut self) -> Result<PathBuf> {
        let td = tempfile::tempdir()?;
        let p = td.path().to_path_buf();
        // The report keeps the latest; earlier ones live until the run ends.
        if let Some(old) = self.report.temp_home.replace(td) {
            self.homes.push(old);
        }
        self.home = Some(p.clone());
        self.spawned = false;
        Ok(p)
    }

    fn spawn(&mut self, words: &[String], reuse: bool) -> Result<()> {
        self.session = None;
        let home = self.home_for_spawn(reuse)?;
        let home_s = home.to_string_lossy().to_string();
        let mut env: Vec<(&str, &str)> = vec![("ACIDTRIP_HOME", home_s.as_str()), ("HOME", home_s.as_str())];
        env.extend(self.opts.env.iter().chain(&self.env).map(|(k, v)| (k.as_str(), v.as_str())));
        let args: Vec<&str> = words.iter().map(String::as_str).collect();
        let s = Session::spawn(&self.opts.bin, &args, self.cols, self.rows, &env, self.opts.cwd.as_deref())?;
        self.home = Some(home.clone());
        self.spawned = true;
        self.report.home = Some(home);
        self.session = Some(s);
        Ok(())
    }

    /// `~/x` under this slot's home, else relative to the app's cwd.
    fn path(&self, w: &str) -> Result<PathBuf> {
        if let Some(rel) = w.strip_prefix("~/") {
            let h = self.home.as_ref().ok_or_else(|| anyhow!("no home yet for {w:?}"))?;
            return Ok(h.join(rel));
        }
        Ok(match &self.opts.cwd {
            Some(d) => d.join(w),
            None => PathBuf::from(w),
        })
    }

    fn sess(&mut self) -> Result<&mut Session> {
        self.session.as_mut().ok_or_else(|| anyhow!("no running session (missing `spawn`?)"))
    }

    fn set_env(&mut self, name: &str, value: &str) {
        self.env.retain(|(k, _)| k != name);
        self.env.push((name.to_string(), value.to_string()));
    }

    fn exec(&mut self, line: &str) -> Result<()> {
        let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let words = split_words(rest)?;
        let t = self.opts.default_timeout;
        match cmd {
            "spawn" => self.spawn(&words, false)?,
            "restart" => {
                if self.home.is_none() {
                    bail!("restart needs an earlier `spawn`");
                }
                self.spawn(&words, true)?;
            }
            "home-file" => {
                let [rel, text] = words.as_slice() else { bail!("usage: home-file REL \"text\"") };
                let home = match &self.home {
                    Some(h) => h.clone(),
                    None => self.new_home()?,
                };
                let path = home.join(rel);
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&path, text)?;
            }
            "session" => {
                let [name] = words.as_slice() else { bail!("usage: session NAME") };
                if *name != self.slot {
                    let cur = (self.session.take(), self.home.take(), self.spawned);
                    self.parked.insert(std::mem::replace(&mut self.slot, name.clone()), cur);
                    (self.session, self.home, self.spawned) = self.parked.remove(name).unwrap_or((None, None, false));
                    if let Some(h) = &self.home {
                        self.report.home = Some(h.clone());
                    }
                }
            }
            "paste" => {
                let [text] = words.as_slice() else { bail!("usage: paste \"text\"") };
                self.sess()?.send_raw(&bracketed(text))?;
            }
            "paste-clipboard" => {
                let from = match words.first() {
                    None => self.session.as_ref(),
                    Some(n) if *n == self.slot => self.session.as_ref(),
                    Some(n) => self.parked.get(n).and_then(|p| p.0.as_ref()),
                };
                let from = from.ok_or_else(|| anyhow!("no running session {:?}", words.first()))?;
                let text = last_osc52(&from.raw_output())
                    .ok_or_else(|| anyhow!("that session copied nothing (no OSC 52 in its output)"))?;
                self.sess()?.send_raw(&bracketed(&text))?;
            }
            "size" => {
                let [c, r] = nums::<2>(&words)?;
                (self.cols, self.rows) = (c, r);
                if let Some(s) = self.session.as_mut() {
                    s.resize(c, r)?;
                }
            }
            "keys" => self.sess()?.keys(rest)?,
            "type" => {
                let text = if rest.starts_with('"') { unquote_one(rest)? } else { rest.to_string() };
                self.sess()?.type_text(&text)?;
            }
            "mouse" => {
                let ev = parse_mouse(&words)?;
                self.sess()?.mouse(ev)?;
            }
            "click" => {
                let [x, y] = nums::<2>(&words[..words.len().min(2)])?;
                let b = parse_button(words.get(2).map(String::as_str).unwrap_or("left"))?;
                let s = self.sess()?;
                s.click_with(b, x, y)?;
            }
            "click-text" => {
                let needle = words.first().ok_or_else(|| anyhow!("click-text needs a \"text\""))?;
                let nth: usize = words.get(1).map(|w| w.parse()).transpose()?.unwrap_or(1);
                let s = self.sess()?;
                let Some((x, y)) = s.find_text(needle, nth.max(1)) else {
                    bail!("no {needle:?} (#{nth}) on screen to click:\n{}", framed(&s.screen_text()));
                };
                let b = MouseButton::Left;
                s.click_with(b, x, y)?;
            }
            "drag" => {
                let [x0, y0, x1, y1] = nums::<4>(&words[..words.len().min(4)])?;
                let b = parse_button(words.get(4).map(String::as_str).unwrap_or("left"))?;
                self.sess()?.drag_with(b, (x0, y0), (x1, y1))?;
            }
            "wait" => {
                if rest.starts_with('"') {
                    let text = &words[0];
                    let to = words.get(1).map(|w| parse_duration(w)).transpose()?.unwrap_or(t);
                    self.sess()?.wait_for_text(text, to)?;
                } else if words.first().map(String::as_str) == Some("idle") {
                    let quiet =
                        words.get(1).map(|w| parse_duration(w)).transpose()?.unwrap_or(Duration::from_millis(300));
                    self.sess()?.wait_idle(quiet, t)?;
                } else {
                    let d = parse_duration(words.first().ok_or_else(|| anyhow!("wait needs an argument"))?)?;
                    std::thread::sleep(d);
                }
            }
            "expect" | "expect-not" => {
                let needle = words.first().ok_or_else(|| anyhow!("{cmd} needs a \"text\""))?;
                let s = self.sess()?;
                let has = s.screen_contains(needle);
                let text = s.screen_text();
                if has != (cmd == "expect") {
                    bail!(
                        "screen {} {needle:?}:\n{}",
                        if has { "unexpectedly contains" } else { "does not contain" },
                        framed(&text)
                    );
                }
            }
            "shot" => {
                let name = words.first().ok_or_else(|| anyhow!("shot needs a NAME"))?;
                std::fs::create_dir_all(&self.opts.out_dir)?;
                let png = self.opts.out_dir.join(format!("{name}.png"));
                let s = self.sess()?;
                // Let the app finish reacting to preceding input; animated screens
                // never go quiet, so a timeout here is fine.
                let _ = s.wait_idle(Duration::from_millis(150), Duration::from_secs(2));
                s.screenshot(&png)?;
                let text = s.screen_text();
                std::fs::write(self.opts.out_dir.join(format!("{name}.txt")), text)?;
                if self.opts.verbose {
                    eprintln!("      -> {}", png.display());
                }
                self.report.shots.push(png);
            }
            "kill" => self.sess()?.kill()?,
            "expect-exit" => {
                let mut code = None;
                let mut to = t;
                for w in &words {
                    if let Ok(c) = w.parse::<i32>() {
                        code = Some(c);
                    } else {
                        to = parse_duration(w)?;
                    }
                }
                let s = self.sess()?;
                match s.wait_exit(to)? {
                    None => bail!("process still running after {to:?}; screen:\n{}", framed(&s.screen_text())),
                    Some(c) if code.is_some_and(|want| want != c) => {
                        bail!("exit code {c}, expected {}", code.unwrap_or_default())
                    }
                    Some(_) => {}
                }
            }
            "env" => {
                let [name, value] = words.as_slice() else { bail!("usage: env NAME VALUE") };
                self.set_env(name, value);
            }
            "fake-claude" => {
                let [file] = words.as_slice() else { bail!("usage: fake-claude FILE") };
                let path = match &self.opts.cwd {
                    Some(d) => d.join(file),
                    None => PathBuf::from(file),
                };
                let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
                // Stop any previous fake before starting the next.
                self.fake = None;
                let fake = FakeClaude::from_json(&text).with_context(|| format!("in {}", path.display()))?;
                self.set_env("ANTHROPIC_BASE_URL", &fake.base_url());
                self.set_env("ANTHROPIC_API_KEY", "fake-key");
                self.fake = Some(fake);
            }
            "expect-claude-calls" => {
                let n: usize =
                    words.first().ok_or_else(|| anyhow!("usage: expect-claude-calls N [timeout]"))?.parse()?;
                let to = words.get(1).map(|w| parse_duration(w)).transpose()?.unwrap_or(t);
                let fake = self.fake.as_ref().ok_or_else(|| anyhow!("no fake Claude (missing `fake-claude`?)"))?;
                let deadline = std::time::Instant::now() + to;
                while fake.request_count() < n {
                    if std::time::Instant::now() >= deadline {
                        bail!(
                            "fake Claude got {} request(s) after {to:?}, expected at least {n}",
                            fake.request_count()
                        );
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            "wait-file" => {
                let w = words.first().ok_or_else(|| anyhow!("usage: wait-file PATH [timeout]"))?;
                let path = self.path(w)?;
                let to = words.get(1).map(|w| parse_duration(w)).transpose()?.unwrap_or(t);
                let start = std::time::Instant::now();
                while !non_empty(&path) {
                    if start.elapsed() > to {
                        bail!("timed out after {to:?} waiting for {}", path.display());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            "expect-file" => {
                if words.is_empty() {
                    bail!("usage: expect-file PATH [PATH...]");
                }
                for w in &words {
                    let path = self.path(w)?;
                    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    if len == 0 {
                        bail!("{} is missing or empty", path.display());
                    }
                }
            }
            "expect-no-file" => {
                if words.is_empty() {
                    bail!("usage: expect-no-file PATH [PATH...]");
                }
                for w in &words {
                    let path = self.path(w)?;
                    if path.exists() {
                        bail!("{} exists", path.display());
                    }
                }
            }
            "expect-file-contains" | "expect-file-not-contains" => {
                let [file, text] = words.as_slice() else { bail!("usage: {cmd} PATH \"text\"") };
                let path = self.path(file)?;
                let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                let found = bytes.windows(text.len().max(1)).any(|w| w == text.as_bytes());
                if found != (cmd == "expect-file-contains") {
                    let does = if found { "contains" } else { "does not contain" };
                    bail!("{} {does} {text:?}", path.display());
                }
            }
            "write-file" => {
                let [file, text] = words.as_slice() else { bail!("usage: write-file PATH \"text\"") };
                let path = self.path(file)?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
            }
            #[cfg(unix)]
            "chmod" => {
                use std::os::unix::fs::PermissionsExt;
                let [mode, file] = words.as_slice() else { bail!("usage: chmod MODE PATH") };
                let mode = u32::from_str_radix(mode, 8).with_context(|| format!("bad mode {mode:?}"))?;
                let path = self.path(file)?;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                    .with_context(|| format!("chmod {}", path.display()))?;
            }
            "expect-cell" => {
                let (Some(x), Some(y), Some(glyph)) = (words.first(), words.get(1), words.get(2)) else {
                    bail!("usage: expect-cell X Y \"glyph\" [fg=C] [bg=C]");
                };
                let (x, y): (u16, u16) = (x.parse()?, y.parse()?);
                let s = self.sess()?;
                let cell = s.cell(x, y).ok_or_else(|| anyhow!("no cell at {x},{y}"))?;
                let mut bad = vec![];
                if glyph != "*" && cell.ch.to_string() != *glyph {
                    bad.push(format!("glyph {:?}, expected {glyph:?}", cell.ch));
                }
                for w in &words[3..] {
                    let (which, want) = w.split_once('=').ok_or_else(|| anyhow!("expected fg=C or bg=C, got {w:?}"))?;
                    let got = match which {
                        "fg" => cell.fg,
                        "bg" => cell.bg,
                        _ => bail!("expected fg=C or bg=C, got {w:?}"),
                    };
                    if got != parse_color(want)? {
                        bad.push(format!("{which} {}, expected {want}", hex(got)));
                    }
                }
                if !bad.is_empty() {
                    bail!("cell {x},{y}: {}:\n{}", bad.join(", "), framed(&s.screen_text()));
                }
            }
            other => bail!("unknown command {other:?}"),
        }
        Ok(())
    }
}

/// A file with bytes in it. A `*` in the last component matches any run of
/// characters (`recovery/*.json`).
fn non_empty(path: &Path) -> bool {
    let has_bytes = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some((pre, post)) = name.split_once('*') else { return has_bytes(path) };
    let Some(dir) = path.parent() else { return false };
    std::fs::read_dir(dir).is_ok_and(|d| {
        d.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.len() >= pre.len() + post.len() && n.starts_with(pre) && n.ends_with(post) && has_bytes(&e.path())
        })
    })
}

/// Text as a terminal pastes it, when the app asked for bracketed paste.
fn bracketed(text: &str) -> Vec<u8> {
    [b"\x1b[200~", text.as_bytes(), b"\x1b[201~"].concat()
}

/// The text of the last OSC 52 clipboard write (`ESC ] 52 ; c ; BASE64`
/// ended by BEL or `ESC \`) in raw terminal output.
fn last_osc52(out: &[u8]) -> Option<String> {
    let pat = b"\x1b]52;";
    let at = out.windows(pat.len()).rposition(|w| w == pat)?;
    let rest = &out[at + pat.len()..];
    let semi = rest.iter().position(|&b| b == b';')?;
    let body = &rest[semi + 1..];
    let end = body.iter().position(|&b| b == 0x07 || b == 0x1b)?;
    String::from_utf8(base64_decode(&body[..end])?).ok()
}

/// Standard base64 (padding optional); `None` on anything else.
fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s: Vec<u8> = s.iter().copied().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..chunk.len() - 1]);
    }
    Some(out)
}

fn nums<const N: usize>(words: &[String]) -> Result<[u16; N]> {
    if words.len() != N {
        bail!("expected {N} numbers, got {words:?}");
    }
    let mut out = [0u16; N];
    for (o, w) in out.iter_mut().zip(words) {
        *o = w.parse().with_context(|| format!("not a number: {w:?}"))?;
    }
    Ok(out)
}

/// A VGA palette index `0`-`15` or `#rrggbb`.
fn parse_color(s: &str) -> Result<[u8; 3]> {
    if let Some(h) = s.strip_prefix('#')
        && h.len() == 6
    {
        let v = u32::from_str_radix(h, 16).with_context(|| format!("bad color {s:?}"))?;
        return Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    let i: usize = s.parse().with_context(|| format!("bad color {s:?}"))?;
    acidtrip_core::color::VGA.get(i).copied().ok_or_else(|| anyhow!("VGA index {i} out of range"))
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_button(s: &str) -> Result<MouseButton> {
    Ok(match s {
        "left" | "l" => MouseButton::Left,
        "middle" | "m" => MouseButton::Middle,
        "right" | "r" => MouseButton::Right,
        _ => bail!("unknown mouse button {s:?}"),
    })
}

/// `down|up|drag BUTTON X Y [mods]`, `move X Y [mods]`, `scroll up|down|left|right X Y [mods]`.
fn parse_mouse(w: &[String]) -> Result<MouseEv> {
    let action = w.first().ok_or_else(|| anyhow!("mouse needs an action"))?.as_str();
    let (kind, rest) = match action {
        "down" | "press" | "up" | "release" | "drag" => {
            let b = parse_button(w.get(1).ok_or_else(|| anyhow!("mouse {action} needs a button"))?)?;
            let k = match action {
                "down" | "press" => MouseKind::Press(b),
                "up" | "release" => MouseKind::Release(b),
                _ => MouseKind::Drag(b),
            };
            (k, &w[2..])
        }
        "move" => (MouseKind::Move, &w[1..]),
        "scroll" => {
            let k = match w.get(1).map(String::as_str) {
                Some("up") => MouseKind::ScrollUp,
                Some("down") => MouseKind::ScrollDown,
                Some("left") => MouseKind::ScrollLeft,
                Some("right") => MouseKind::ScrollRight,
                d => bail!("unknown scroll direction {d:?}"),
            };
            (k, &w[2..])
        }
        _ => bail!("unknown mouse action {action:?}"),
    };
    if rest.len() < 2 {
        bail!("mouse {action} needs X Y");
    }
    let [x, y] = nums::<2>(&rest[..2])?;
    let mut mods = Mods::default();
    for m in &rest[2..] {
        match m.as_str() {
            "ctrl" => mods.ctrl = true,
            "alt" | "meta" => mods.alt = true,
            "shift" => mods.shift = true,
            _ => bail!("unknown mouse modifier {m:?}"),
        }
    }
    Ok(MouseEv { kind, x, y, mods })
}

/// `200ms`, `5s`, `1.5s`, `2m`; a bare number is milliseconds.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let bad = || anyhow!("bad duration {s:?}");
    let (num, mult) = if let Some(n) = s.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60.0)
    } else {
        (s, 0.001)
    };
    let v: f64 = num.parse().map_err(|_| bad())?;
    if !(v.is_finite() && v >= 0.0) {
        return Err(bad());
    }
    Ok(Duration::from_secs_f64(v * mult))
}

/// Split on whitespace, honoring double-quoted words with `\" \\ \n \t \e` escapes.
fn split_words(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&c) = chars.peek() else { break };
        let mut word = String::new();
        if c == '"' {
            chars.next();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => word.push(match chars.next() {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('e') => '\x1b',
                        Some(o) => o,
                        None => bail!("dangling escape in {s:?}"),
                    }),
                    o => word.push(o),
                }
            }
            if !closed {
                bail!("unterminated quote in {s:?}");
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                word.push(c);
                chars.next();
            }
        }
        out.push(word);
    }
    Ok(out)
}

fn unquote_one(s: &str) -> Result<String> {
    let w = split_words(s)?;
    match w.as_slice() {
        [one] => Ok(one.clone()),
        _ => bail!("expected one quoted string, got {s:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_durations() {
        assert_eq!(split_words(r#""a b" c "x\"y""#).unwrap(), vec!["a b", "c", "x\"y"]);
        assert_eq!(parse_duration("200ms").unwrap(), Duration::from_millis(200));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_millis(1500));
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn osc52_clipboard() {
        assert_eq!(base64_decode(b"aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode(b"aGk").unwrap(), b"hi");
        assert!(base64_decode(b"a$").is_none());
        let out = b"junk\x1b]52;c;Zmlyc3Q=\x07more\x1b]52;c;c2Vjb25k\x1b\\tail";
        assert_eq!(last_osc52(out).as_deref(), Some("second"));
        assert_eq!(last_osc52(b"nothing here"), None);
        assert_eq!(bracketed("x"), b"\x1b[200~x\x1b[201~");
    }

    #[test]
    fn wait_file_globs() {
        let dir = std::env::temp_dir().join(format!("acidtrip-harness-glob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!non_empty(&dir.join("*.json")));
        std::fs::write(dir.join("a.acid.tmp"), b"x").unwrap();
        assert!(!non_empty(&dir.join("*.json")));
        std::fs::write(dir.join("a.json"), b"{}").unwrap();
        assert!(non_empty(&dir.join("*.json")));
        assert!(non_empty(&dir.join("a.json")));
        assert!(!non_empty(&dir.join("b.json")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("12").unwrap(), [0xff, 0x55, 0x55]);
        assert_eq!(parse_color("#102030").unwrap(), [0x10, 0x20, 0x30]);
        assert!(parse_color("16").is_err());
        assert_eq!(hex([0xff, 0, 0x0a]), "#ff000a");
    }

    #[test]
    fn mouse_parse() {
        let w: Vec<String> = "down left 10 5 ctrl".split(' ').map(String::from).collect();
        let e = parse_mouse(&w).unwrap();
        assert_eq!(e.kind, MouseKind::Press(MouseButton::Left));
        assert_eq!((e.x, e.y, e.mods.ctrl), (10, 5, true));
        let w: Vec<String> = "scroll up 1 2".split(' ').map(String::from).collect();
        assert_eq!(parse_mouse(&w).unwrap().kind, MouseKind::ScrollUp);
    }
}
