//! acidtrip — a modern terminal ANSI art editor in the spirit of ACiDDraw.

mod actions;
mod app;
mod cli;
mod cli_harvest;
mod colorfx;
mod dialogs;
mod exporter;
mod keymap;
mod recent;
mod replay;
mod share;
mod tab;
mod term;
mod together;
mod tools_ctl;
mod ui;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "acidtrip", version, about = "A modern terminal ANSI art editor in the spirit of ACiDDraw")]
struct Cli {
    /// Files to open (.ans .xb .bin .adf .idf .tnd .pcb .avt .asc .acid, or a new file name).
    files: Vec<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Convert between any formats: acidtrip convert in.ans out.png
    Convert(cli::ConvertArgs),
    /// Render to PNG (shortcut for convert … .png).
    Render {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        scale: u32,
    },
    /// Replay how an .acid piece was drawn: acidtrip replay in.acid out.gif (or .cast)
    Replay(cli::ReplayArgs),
    /// Stdio MCP server for Claude Code; attaches to a running editor if any.
    Mcp {
        /// Attach to the editor with this pid.
        #[arg(long)]
        session: Option<u32>,
        /// Ignore running editors and work on a private canvas.
        #[arg(long)]
        headless: bool,
    },
    /// Build font & stencil libraries from scene art (16colo.rs pack, URL, file or dir).
    Harvest(cli::HarvestArgs),
    /// Font library: list or download TheDraw fonts.
    Fonts {
        #[command(subcommand)]
        cmd: cli::FontsCmd,
    },
    /// List a document's saved versions, or restore one to a file.
    Versions(cli::VersionsArgs),
    /// Print where config, library and state live.
    Paths,
    /// List every action id with its keys (for keymap overrides in config.toml).
    Keys,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        None => run_editor(&cli.files),
        Some(Cmd::Convert(a)) => cli::convert(a),
        Some(Cmd::Render { input, output, scale }) => cli::convert(cli::ConvertArgs::png(input, output, scale)),
        Some(Cmd::Mcp { session, headless }) => {
            acidtrip_ai::mcp::run_stdio(acidtrip_ai::mcp::McpOptions { session, headless })
        }
        Some(Cmd::Replay(a)) => cli::replay(a),
        Some(Cmd::Harvest(a)) => cli::harvest(a),
        Some(Cmd::Fonts { cmd }) => cli::fonts(cmd),
        Some(Cmd::Versions(a)) => cli::versions(a),
        Some(Cmd::Paths) => cli::paths(),
        Some(Cmd::Keys) => cli::keys(),
    };
    if let Err(e) = result {
        eprintln!("acidtrip: {e:#}");
        std::process::exit(1);
    }
}

fn run_editor(files: &[PathBuf]) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        anyhow::bail!("the editor needs a terminal (try `acidtrip --help` for command-line tools)");
    }
    let (mut terminal, guard) = term::setup()?;
    // Ask the terminal which graphics protocol it speaks (for the pixel preview).
    let picker = ratatui_image::picker::Picker::from_query_stdio();
    if let Ok(path) = std::env::var("ACIDTRIP_LOG") {
        let msg = match &picker {
            Ok(p) => format!("graphics: {:?} font {:?}\n", p.protocol_type(), p.font_size()),
            Err(e) => format!("graphics query failed: {e}\n"),
        };
        let _ = std::fs::write(path, msg);
    }
    let picker = picker.ok();
    // Pixel-precise mouse needs the cell size in pixels, which the picker knows.
    let cell_px = picker.as_ref().map(|p| p.font_size()).filter(|_| term::enable_pixel_mouse());
    if let Ok(path) = std::env::var("ACIDTRIP_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "pixel mouse: {}", cell_px.is_some());
        }
    }
    let mut app = app::App::new(files, guard.enhanced_keys, picker)?;
    app.cell_px = cell_px.map(|fs| (fs.width.max(1) as f32, fs.height.max(1) as f32));
    // Scripted start (screenshots, demos): comma-separated action ids to run.
    if let Ok(ids) = std::env::var("ACIDTRIP_RUN") {
        for id in ids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match actions::Action::from_id(id) {
                Some(a) => app.run(a),
                None => app.flash(format!("ACIDTRIP_RUN: unknown action \"{id}\""), app::Level::Warn),
            }
        }
    }
    let r = app.run_loop(&mut terminal);
    app.autosave(false);
    drop(terminal);
    drop(guard);
    r
}
