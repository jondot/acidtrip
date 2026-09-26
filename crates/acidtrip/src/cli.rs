//! Non-interactive subcommands.

use std::path::PathBuf;

use acidtrip_io::fonts::{FontLibrary, TextRenderOptions};
use acidtrip_io::format::{self, Format, GifMode, SaveOptions};
use acidtrip_io::library::Paths;
use acidtrip_io::versions::VersionStore;
use anyhow::Context;
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct ConvertArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Output format (default: from the output extension).
    #[arg(long)]
    pub format: Option<String>,
    /// Pixel scale for PNG/GIF.
    #[arg(long, default_value_t = 1)]
    pub scale: u32,
    /// GIF animation: still, reveal, layers, frames (auto: frames when the piece is animated).
    #[arg(long, default_value = "auto")]
    pub gif: String,
    /// Baud rate for reveal animations.
    #[arg(long, default_value_t = 14400)]
    pub baud: u32,
    /// ANSI: wrap lines at N chars for BBSes.
    #[arg(long)]
    pub line_length: Option<usize>,
    /// Don't attach a SAUCE record.
    #[arg(long)]
    pub no_sauce: bool,
    /// SVG with pixel-exact bitmap glyphs.
    #[arg(long)]
    pub pixel_exact: bool,
    /// Identifier for C/Pascal/ASM arrays and React components.
    #[arg(long, default_value = "AcidArt")]
    pub identifier: String,
}

impl ConvertArgs {
    pub fn png(input: PathBuf, output: PathBuf, scale: u32) -> Self {
        ConvertArgs {
            input,
            output,
            format: Some("png".into()),
            scale,
            gif: "auto".into(),
            baud: 14400,
            line_length: None,
            no_sauce: false,
            pixel_exact: false,
            identifier: "AcidArt".into(),
        }
    }
}

fn parse_format(s: &str) -> anyhow::Result<Format> {
    Format::from_path(std::path::Path::new(&format!("x.{s}")))
        .or_else(|| {
            Format::ALL
                .into_iter()
                .find(|f| f.name().eq_ignore_ascii_case(s) || format!("{f:?}").eq_ignore_ascii_case(s))
        })
        .with_context(|| format!("unknown format {s:?}"))
}

pub fn convert(a: ConvertArgs) -> anyhow::Result<()> {
    let doc = format::load(&a.input).with_context(|| format!("loading {}", a.input.display()))?;
    let fmt = match &a.format {
        Some(f) => parse_format(f)?,
        None => Format::from_path(&a.output)
            .with_context(|| format!("can't tell the format of {} — use --format", a.output.display()))?,
    };
    let opts = SaveOptions {
        sauce: if a.no_sauce { Some(false) } else { None },
        line_length: a.line_length,
        scale: a.scale.clamp(1, 8),
        gif_mode: match a.gif.as_str() {
            "reveal" => GifMode::Reveal,
            "layers" => GifMode::LayersAsFrames,
            "frames" => GifMode::Frames,
            "auto" if doc.is_animated() => GifMode::Frames,
            _ => GifMode::Still,
        },
        baud: a.baud,
        svg_pixel_exact: a.pixel_exact,
        identifier: a.identifier,
        ..SaveOptions::default()
    };
    for w in format::loss_warnings(&doc, fmt) {
        eprintln!("warning: {w}");
    }
    format::save(&doc, &a.output, fmt, &opts)?;
    eprintln!("wrote {} ({})", a.output.display(), fmt.name());
    Ok(())
}

#[derive(Args)]
pub struct ReplayArgs {
    /// An .acid file (only .acid keeps the edit history).
    pub input: PathBuf,
    /// .gif or .cast (asciinema).
    pub output: PathBuf,
    /// Play N times as fast as it was drawn.
    #[arg(long, conflicts_with = "fit")]
    pub speed: Option<f64>,
    /// Fit the whole replay into SECS seconds (the default: 30).
    #[arg(long)]
    pub fit: Option<u32>,
    /// GIF pixel scale.
    #[arg(long, default_value_t = 1)]
    pub scale: u32,
    /// Play long pauses in real time instead of cutting them to a second.
    #[arg(long)]
    pub keep_idle: bool,
    /// Leave out work that was undone.
    #[arg(long)]
    pub hide_undone: bool,
}

pub fn replay(a: ReplayArgs) -> anyhow::Result<()> {
    use acidtrip_core::replay::{Speed, TimelineOptions};
    let (_, log) = format::load_with_log(&a.input).with_context(|| format!("loading {}", a.input.display()))?;
    let log = log.with_context(|| format!("{} has no edit history to replay", a.input.display()))?;
    let o = format::ReplayExport {
        timeline: TimelineOptions { skip_idle: !a.keep_idle, hide_undone: a.hide_undone },
        speed: match (a.speed, a.fit) {
            (Some(x), _) => Speed::Times(x.max(0.01)),
            (None, s) => Speed::Fit(s.unwrap_or(30).max(1)),
        },
        scale: a.scale.clamp(1, 8),
    };
    let ext = a.output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let bytes = match ext.as_str() {
        "gif" => format::replay_gif(&log, &o)?,
        "cast" => format::replay_cast(&log, &o)?,
        _ => anyhow::bail!("replay writes .gif or .cast, not {}", a.output.display()),
    };
    acidtrip_io::library::write_atomic(&a.output, &bytes)?;
    eprintln!("wrote {} ({} steps)", a.output.display(), log.len());
    Ok(())
}

#[derive(Args)]
pub struct HarvestArgs {
    /// 16colo.rs pack name (e.g. `16colo.rs:acid-50`), URL, file or directory.
    pub source: String,
    /// Only list logo candidates (no AI).
    #[arg(long)]
    pub list: bool,
    /// Save every candidate as a stencil without reading letters.
    #[arg(long)]
    pub stencils_only: bool,
    /// Ask the AI to draw the missing letters of each harvested font.
    #[arg(long)]
    pub complete: bool,
}

pub fn harvest(a: HarvestArgs) -> anyhow::Result<()> {
    crate::cli_harvest::run(a)
}

#[derive(Subcommand)]
pub enum FontsCmd {
    /// List installed fonts.
    List,
    /// Download TheDraw font packs into the library.
    Get,
    /// Render text with a font to the terminal.
    Show { font: String, text: String },
    /// Install a .tdf/.flf/.zip into the library.
    Install { file: PathBuf },
}

pub fn fonts(cmd: FontsCmd) -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    let mut lib = FontLibrary::load(Some(&paths.fonts_dir()));
    match cmd {
        FontsCmd::List => {
            for f in lib.list() {
                println!("{:<28} {:<11} {}", f.id, format!("{:?}", f.kind), f.name);
            }
        }
        FontsCmd::Get => {
            let n = acidtrip_io::fonts::download_packs(&paths.fonts_dir())?;
            println!("installed {n} font files into {}", paths.fonts_dir().display());
        }
        FontsCmd::Show { font, text } => {
            let clip = lib.render(&font, &text, &TextRenderOptions::default())?;
            let doc = acidtrip_core::Document::from_grid(acidtrip_core::DocKind::Classic, &clip.to_grid());
            let b = format::save_bytes(&doc, Format::Utf8Ansi, &SaveOptions::default())?;
            print!("{}", String::from_utf8_lossy(&b));
        }
        FontsCmd::Install { file } => {
            let added = lib.install(&file, &paths.fonts_dir())?;
            for f in added {
                println!("installed {} ({:?})", f.name, f.kind);
            }
        }
    }
    Ok(())
}

#[derive(Args)]
pub struct VersionsArgs {
    /// The document (.acid by its document id, any other art file by its path).
    pub file: PathBuf,
    /// Restore this version hash (prefix ok) into OUT.
    #[arg(long, requires = "out")]
    pub restore: Option<String>,
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub fn versions(a: VersionsArgs) -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    let doc = format::load(&a.file)?;
    let store = VersionStore::open(&paths.versions_dir(), acidtrip_io::versions::store_id(&doc, Some(&a.file)))?;
    let list = store.list();
    match (a.restore, a.out) {
        (Some(h), Some(out)) => {
            let v = list.iter().find(|v| v.hash.starts_with(&h)).with_context(|| format!("no version {h}"))?;
            let d = store.load(&v.hash)?;
            let fmt = Format::from_path(&out).unwrap_or(Format::Acid);
            format::save(&d, &out, fmt, &SaveOptions::default())?;
            println!("restored {} to {}", &v.hash[..12], out.display());
        }
        _ => {
            if list.is_empty() {
                println!("no versions for {} (only files saved from acidtrip have history)", a.file.display());
            }
            for v in list {
                println!("{}  {}  {}", &v.hash[..12], v.timestamp.chars().take(19).collect::<String>(), v.label);
            }
        }
    }
    Ok(())
}

pub fn paths() -> anyhow::Result<()> {
    let p = Paths::resolve()?;
    println!("config    {}", p.config_file().display());
    println!("fonts     {}", p.fonts_dir().display());
    println!("stencils  {}", p.stencils_dir().display());
    println!("versions  {}", p.versions_dir().display());
    println!("exports   {}", p.exports_dir().display());
    println!("recovery  {}", p.recovery_dir().display());
    println!("sockets   {}", p.sockets_dir().display());
    Ok(())
}

pub fn keys() -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    let config = crate::cli_harvest::read_config(&paths);
    let (km, errs) = crate::keymap::Keymap::from_config(&config.keymap.preset, &config.keymap.bindings);
    for e in errs {
        eprintln!("warning: {e}");
    }
    // Written by hand so `acidtrip keys | head` stops quietly.
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let mut print = || -> std::io::Result<()> {
        writeln!(out, "# preset: {:?} — override in {} under [keymap.bindings]", km.preset, paths.config_file().display())?;
        let mut cat = "";
        for a in crate::actions::Action::ALL {
            if a.category() != cat {
                cat = a.category();
                writeln!(out, "\n# {cat}")?;
            }
            writeln!(out, "{:<20} {:<36} {}", a.id(), a.title(), km.keys_for(*a).join(", "))?;
        }
        out.flush()
    };
    match print() {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}
