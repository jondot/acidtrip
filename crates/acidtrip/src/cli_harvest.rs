//! `acidtrip harvest`: fetch scene art, find logos, read letters with Claude
//! (if a key is set), and write fonts + stencils into the library.

use acidtrip_ai::agent::AgentConfig;
use acidtrip_ai::harvest::{self, HarvestOptions};
use acidtrip_io::config::Config;
use acidtrip_io::library::Paths;
use acidtrip_io::stencils::StencilLibrary;

/// Config for CLI subcommands: read it if present, never create it (the
/// editor's first launch creates it and shows the welcome screen).
pub fn read_config(paths: &Paths) -> Config {
    std::fs::read_to_string(paths.config_file()).ok().and_then(|t| Config::parse(&t).ok()).unwrap_or_default()
}

pub fn agent_config(config: &Config) -> Option<AgentConfig> {
    config.api_key().map(|api_key| AgentConfig {
        api_key,
        model: config.ai.model.clone(),
        max_rounds: config.ai.max_tool_rounds,
    })
}

pub fn run(a: crate::cli::HarvestArgs) -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    let config = read_config(&paths);
    let cache = paths.state_dir.join("harvest-cache");
    // Browse 16colo.rs: `16colo.rs:` lists years, `16colo.rs:1997` a year's packs.
    if let Some(rest) = a.source.trim().strip_prefix("16colo.rs").map(|r| r.trim_start_matches(':').trim_matches('/')) {
        if rest.is_empty() {
            for y in harvest::sixteen_colors_years(&cache)? {
                println!("{}  {:>4} packs   acidtrip harvest 16colo.rs:{} --list", y.year, y.packs, y.year);
            }
            return Ok(());
        }
        if let Ok(year) = rest.parse::<u32>()
            && (1980..=2100).contains(&year)
        {
            for p in harvest::sixteen_colors_year(year, &cache)? {
                println!("16colo.rs:{:<16} {}", p.name, p.groups.join(", "));
            }
            return Ok(());
        }
    }
    if a.list {
        let arts = harvest::fetch(&a.source, &cache)?;
        for art in &arts {
            for c in harvest::candidates(art) {
                println!("{:<40} {:>3}x{:<3} score {:.2}  {}", c.id, c.w, c.h, c.score, c.attribution.credit());
            }
        }
        return Ok(());
    }
    let cfg = if a.stencils_only { None } else { agent_config(&config) };
    if cfg.is_none() && !a.stencils_only {
        eprintln!("note: no ANTHROPIC_API_KEY — saving logos as stencils only (no letter reading).");
        eprintln!("      Tip: Claude Code can read the letters instead via the MCP harvest tools.");
    }
    let mut stencils = StencilLibrary::load(&paths.stencils_dir());
    let opts = HarvestOptions { complete: a.complete, ..HarvestOptions::default() };
    let (res, rep) = harvest::run(&a.source, cfg.as_ref(), &opts, &paths, &mut stencils, &mut |m| eprintln!("{m}"))?;
    for f in &rep.font_files {
        println!("font     {}", f.display());
    }
    for id in &rep.stencil_ids {
        println!("stencil  {id}");
    }
    for s in &res.skipped {
        eprintln!("skipped  {s}");
    }
    println!("Harvested art stays in your library for personal use — credit the artists.");
    Ok(())
}
