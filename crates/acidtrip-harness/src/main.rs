use std::path::PathBuf;
use std::process::ExitCode;

use acidtrip_harness::{RunOptions, default_app_bin, run_script, workspace_root};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "acidtrip-harness", about = "Drive acidtrip in a pty and take screenshots")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a harness script.
    Run {
        script: PathBuf,
        /// Output dir for shots (default: target/shots/<script name>).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Program to spawn (default: target/debug/acidtrip).
        #[arg(long)]
        bin: Option<PathBuf>,
        #[arg(long, default_value_t = 120)]
        cols: u16,
        #[arg(long, default_value_t = 40)]
        rows: u16,
        /// ACIDTRIP_HOME for the app (default: a fresh tempdir).
        #[arg(long)]
        home: Option<PathBuf>,
        /// Default timeout for waits, e.g. 5s.
        #[arg(long, default_value = "5s")]
        timeout: String,
        /// Don't print steps.
        #[arg(long, short)]
        quiet: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run { script, out, bin, cols, rows, home, timeout, quiet } => {
            let text = match std::fs::read_to_string(&script) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: reading {}: {e}", script.display());
                    return ExitCode::from(2);
                }
            };
            let stem = script.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "script".into());
            let out = out.unwrap_or_else(|| workspace_root().join("target").join("shots").join(stem));
            let mut opts = RunOptions::new(bin.unwrap_or_else(default_app_bin), out);
            opts.cols = cols;
            opts.rows = rows;
            opts.home = home;
            opts.verbose = !quiet;
            match acidtrip_harness::script::parse_duration(&timeout) {
                Ok(d) => opts.default_timeout = d,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(2);
                }
            }
            match run_script(&text, &opts) {
                Ok(r) => {
                    eprintln!("ok: {} steps, {} shots in {}", r.steps, r.shots.len(), opts.out_dir.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("FAILED: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
