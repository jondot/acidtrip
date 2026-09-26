//! Font/stencil harvester: fetch scene art, find logo candidates, have
//! Claude read letters, cut glyphs into partial fonts (acidtrip's own
//! `.acidfont`: any size, any shape) and stencils.
//!
//! Pipeline: [`fetch`] → [`candidates`] → [`read_letters`] (or a reading
//! supplied by an MCP client) → [`build`] → [`write`]. [`complete_font`]
//! optionally draws missing glyphs with the agent loop.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use acidtrip_core::render::{RenderOptions, png_bytes, render_grid};
use acidtrip_core::{Cell, Clip, Color, DocKind, Document, Grid, History, Palette};
use acidtrip_io::fonts::{CUT_FONT_EXT, CutFont, CutGlyph, FontLibrary};
use acidtrip_io::format::{self, Format};
use acidtrip_io::library::Paths;
use acidtrip_io::stencils::{Stencil, StencilLibrary, StencilMeta};
use anyhow::{Context, Result, anyhow, bail};
use retrofont::tdf::TdfFont;
use retrofont::{Glyph, GlyphPart};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent::{self, AgentConfig, AgentEvent, HttpTransport, Transport};
use crate::exec::{self, ExecState};
use crate::{ToolRequest, ToolResult};

pub const LICENSE: &str = "harvested — for personal use, credit the artist";
const USER_AGENT: &str = concat!("acidtrip/", env!("CARGO_PKG_VERSION"), " (font harvester)");

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Attribution {
    pub author: String,
    pub group: String,
    pub title: String,
    /// Art pack name (e.g. 16colo.rs pack), if any.
    pub pack: String,
    pub file: String,
    /// Where it was fetched from (path or URL).
    pub source: String,
}

impl Attribution {
    /// "Title by Author / Group (pack/file)".
    pub fn credit(&self) -> String {
        let mut s = String::new();
        if !self.title.is_empty() {
            s.push_str(&format!("{:?} ", self.title));
        }
        s.push_str(&format!("by {}", if self.author.is_empty() { "unknown artist" } else { &self.author }));
        if !self.group.is_empty() {
            s.push_str(&format!(" / {}", self.group));
        }
        let loc = [self.pack.as_str(), self.file.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        if !loc.is_empty() {
            s.push_str(&format!(" ({loc})"));
        }
        s
    }

    /// Who a harvested font belongs to: artist, else group, else "unknown".
    pub fn owner(&self) -> String {
        [&self.author, &self.group]
            .into_iter()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string()
    }
}

/// One parsed art file.
#[derive(Clone, Debug)]
pub struct SourceArt {
    pub doc: Document,
    pub attribution: Attribution,
}

/// A logo-like blob cut from a piece of art.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Stable id: "<file>@<x>,<y>".
    pub id: String,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub score: f32,
    /// Only the blob's cells; everything else in the box is transparent.
    pub clip: Clip,
    pub attribution: Attribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LetterSpan {
    #[serde(rename = "char")]
    pub ch: char,
    /// First column, relative to the candidate's left edge.
    pub x0: usize,
    /// Last column (inclusive).
    pub x1: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LetterReading {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub letters: Vec<LetterSpan>,
    #[serde(default)]
    pub style: String,
}

/// A partial font under construction. No format limits: glyphs are any
/// size and shape (cells outside the letter are transparent).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontSpec {
    pub name: String,
    pub style: String,
    pub spacing: i32,
    pub glyphs: BTreeMap<char, Clip>,
    /// The row of a glyph that sits on the baseline, when it isn't the last
    /// (a letter with a descender).
    pub bases: BTreeMap<char, usize>,
    /// Glyphs drawn by AI rather than cut from art.
    pub generated: BTreeSet<char>,
}

impl FontSpec {
    pub fn height(&self) -> usize {
        self.glyphs.values().map(|c| c.height).max().unwrap_or(0)
    }

    /// Put a glyph in (replacing the old one) with its baseline row.
    pub fn insert(&mut self, ch: char, clip: Clip, base: Option<usize>) {
        match base.filter(|&b| b + 1 < clip.height) {
            Some(b) => self.bases.insert(ch, b),
            None => self.bases.remove(&ch),
        };
        self.glyphs.insert(ch, clip);
    }

    pub fn to_cut(&self) -> CutFont {
        CutFont {
            name: self.name.clone(),
            spacing: self.spacing,
            glyphs: self
                .glyphs
                .iter()
                .map(|(&ch, clip)| (ch, CutGlyph { clip: clip.clone(), base: self.bases.get(&ch).copied() }))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HarvestResult {
    pub fonts: Vec<(FontSpec, Attribution)>,
    pub stencils: Vec<Stencil>,
    /// Letters that could not become glyphs (too big, empty) with the reason.
    pub skipped: Vec<String>,
}

// ------------------------------------------------------------------ fetch

fn art_format(name: &str) -> Option<Format> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "ans" | "ice" | "nfo" | "diz" | "asc" | "bin" | "xb" | "adf" | "idf" | "tnd" | "pcb" | "avt"
    ) {
        Format::from_path(Path::new(name)).filter(|f| !matches!(f, Format::Png | Format::Gif | Format::Acid))
    } else {
        None
    }
}

fn is_zip(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}

/// Parse one art file; SAUCE credits become the attribution.
pub fn parse_art(name: &str, bytes: &[u8], mut attribution: Attribution) -> Result<SourceArt> {
    let fmt = art_format(name).ok_or_else(|| anyhow!("{name}: not an art file"))?;
    let doc = format::load_bytes(bytes, fmt).with_context(|| format!("parsing {name}"))?;
    let s = &doc.meta.sauce;
    attribution.author = s.author.trim().to_string();
    attribution.group = s.group.trim().to_string();
    attribution.title = s.title.trim().to_string();
    attribution.file =
        Path::new(name).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    Ok(SourceArt { doc, attribution })
}

/// Every art file inside a zip (nested dirs included, nested zips skipped).
pub fn from_zip(bytes: &[u8], pack: &str, source: &str) -> Result<Vec<SourceArt>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).context("reading zip")?;
    let mut out = vec![];
    for i in 0..z.len() {
        let Ok(mut f) = z.by_index(i) else { continue };
        let name = f.name().to_string();
        if !f.is_file() || art_format(&name).is_none() {
            continue;
        }
        let mut buf = vec![];
        if f.read_to_end(&mut buf).is_err() {
            continue;
        }
        let at = Attribution { pack: pack.to_string(), source: source.to_string(), ..Default::default() };
        if let Ok(a) = parse_art(&name, &buf, at) {
            out.push(a);
        }
    }
    Ok(out)
}

pub(crate) fn http_get(url: &str) -> Result<Vec<u8>> {
    http_get_with(url, &mut |_| {})
}

/// GET with download progress ("downloading <name>  1.2 / 3.4 MB").
fn http_get_with(url: &str, progress: &mut dyn FnMut(&str)) -> Result<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .build()
        .new_agent();
    let mut r = agent.get(url).header("user-agent", USER_AGENT).call().with_context(|| format!("GET {url}"))?;
    let status = r.status().as_u16();
    if status != 200 {
        bail!("GET {url}: HTTP {status}");
    }
    let total =
        r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<u64>().ok());
    let name = url.rsplit('/').find(|p| !p.is_empty()).unwrap_or(url).to_string();
    let mut reader = r.body_mut().with_config().limit(300 << 20).reader();
    let (mut buf, mut chunk) = (vec![], vec![0u8; 64 * 1024]);
    let mut last = std::time::Instant::now();
    loop {
        let n = reader.read(&mut chunk).with_context(|| format!("reading {url}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if last.elapsed() >= std::time::Duration::from_millis(120) {
            last = std::time::Instant::now();
            progress(&download_status(&name, buf.len() as u64, total));
        }
    }
    Ok(buf)
}

fn download_status(name: &str, got: u64, total: Option<u64>) -> String {
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    match total {
        Some(t) if t > 0 => format!("downloading {name}  {:.1} / {:.1} MB", mb(got), mb(t)),
        _ => format!("downloading {name}  {:.1} MB", mb(got)),
    }
}

/// Download `url` into `cache_dir/<name>` unless already cached.
fn cached_get_with(url: &str, cache_dir: &Path, name: &str, progress: &mut dyn FnMut(&str)) -> Result<Vec<u8>> {
    let path = cache_dir.join(sanitize(name));
    if let Ok(b) = std::fs::read(&path)
        && !b.is_empty()
    {
        progress(&format!("using cached {name}"));
        return Ok(b);
    }
    progress(&format!("downloading {name}…"));
    let b = http_get_with(url, progress)?;
    std::fs::create_dir_all(cache_dir).ok();
    let _ = std::fs::write(&path, &b);
    Ok(b)
}

/// API GET that refreshes a cached copy and falls back to it when offline.
fn api_get(url: &str, cache_dir: &Path, name: &str) -> Result<Vec<u8>> {
    let path = cache_dir.join("api").join(sanitize(name));
    match http_get(url) {
        Ok(b) => {
            std::fs::create_dir_all(path.parent().unwrap_or(cache_dir)).ok();
            let _ = std::fs::write(&path, &b);
            Ok(b)
        }
        Err(e) => std::fs::read(&path).ok().filter(|b| !b.is_empty()).ok_or(e),
    }
}

pub(crate) fn sanitize(s: &str) -> String {
    let t: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    t.trim_matches('_').to_string()
}

/// Download URL of a 16colo.rs pack, via `https://api.16colo.rs/v1/pack/<pack>`.
pub fn sixteen_colors_pack_url(pack: &str) -> Result<String> {
    let body = http_get(&format!("{SIXTEEN_API}/pack/{pack}"))?;
    Ok(parse_pack_detail(pack, &body)?.download)
}

/// Fetch art. `source` is a local file or directory (.ans/.xb/.bin/… or
/// .zip packs), an http(s) URL to a file or zip, `16colo.rs:<pack>`, or a
/// `https://16colo.rs/pack/<pack>[/<file>]` URL. Downloads are cached in
/// `cache_dir`.
pub fn fetch(source: &str, cache_dir: &Path) -> Result<Vec<SourceArt>> {
    fetch_with(source, cache_dir, &mut |_| {})
}

/// [`fetch`] reporting progress (API lookups, download size, parsing).
pub fn fetch_with(source: &str, cache_dir: &Path, progress: &mut dyn FnMut(&str)) -> Result<Vec<SourceArt>> {
    let s = source.trim();
    let pack_url = ["https://16colo.rs/pack/", "http://16colo.rs/pack/"].iter().find_map(|p| s.strip_prefix(p));
    if let Some(pack) = s.strip_prefix("16colo.rs:").or_else(|| pack_url.filter(|r| !r.trim_matches('/').contains('/')))
    {
        let pack = pack.trim_matches('/');
        if pack.is_empty() {
            bail!("name a pack: 16colo.rs:<pack> (e.g. 16colo.rs:twi-9703)");
        }
        let zip_name = format!("{pack}.zip");
        let bytes = match std::fs::read(cache_dir.join(sanitize(&zip_name))) {
            Ok(b) if !b.is_empty() => {
                progress(&format!("using cached {zip_name}"));
                b
            }
            _ => {
                progress(&format!("asking 16colo.rs about {pack}…"));
                let url = sixteen_colors_pack_url(pack)?;
                cached_get_with(&url, cache_dir, &zip_name, progress)?
            }
        };
        progress(&format!("unpacking {zip_name}…"));
        return from_zip(&bytes, pack, &format!("https://16colo.rs/pack/{pack}"));
    }
    if let Some(rest) = pack_url {
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty() && *p != "raw").collect();
        let (pack, file) = (parts[0], parts[parts.len() - 1]);
        let url = format!("https://16colo.rs/pack/{pack}/raw/{file}");
        let bytes = cached_get_with(&url, cache_dir, &format!("{pack}-{file}"), progress)?;
        let at = Attribution {
            pack: pack.into(),
            source: format!("https://16colo.rs/pack/{pack}/{file}"),
            ..Default::default()
        };
        return Ok(vec![parse_art(file, &bytes, at)?]);
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        let name = s.rsplit('/').find(|p| !p.is_empty()).unwrap_or("download");
        let key = format!("{:016x}-{name}", fxhash(s));
        let bytes = cached_get_with(s, cache_dir, &key, progress)?;
        if is_zip(name) || bytes.starts_with(b"PK\x03\x04") {
            return from_zip(&bytes, name.trim_end_matches(".zip"), s);
        }
        return Ok(vec![parse_art(name, &bytes, Attribution { source: s.into(), ..Default::default() })?]);
    }
    let expanded = expand_home(s);
    let path = Path::new(&expanded);
    if !path.exists() {
        bail!("{s}: no such file or directory (use a path, a URL or 16colo.rs:<pack>)");
    }
    progress(&format!("reading {s}…"));
    let mut out = vec![];
    for f in walk(path) {
        let name = f.to_string_lossy().into_owned();
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        if is_zip(&name) {
            let pack = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            out.extend(from_zip(&bytes, &pack, &name).unwrap_or_default());
        } else if art_format(&name).is_some() {
            let pack = if path.is_dir() {
                path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                String::new()
            };
            if let Ok(a) = parse_art(&name, &bytes, Attribution { pack, source: name.clone(), ..Default::default() }) {
                out.push(a);
            }
        }
    }
    Ok(out)
}

fn expand_home(s: &str) -> String {
    match (s.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => s.to_string(),
    }
}

/// Is `name` an art file the harvester can parse?
pub fn is_art_file(name: &str) -> bool {
    art_format(name).is_some()
}

// ------------------------------------------------------------------ 16colo.rs browsing

pub const SIXTEEN_API: &str = "https://api.16colo.rs/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YearInfo {
    pub year: u32,
    pub packs: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackInfo {
    pub name: String,
    pub year: u32,
    pub groups: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackFile {
    pub name: String,
    pub artists: Vec<String>,
    /// Titles / text the site indexed for the file.
    pub content: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackDetail {
    pub name: String,
    pub year: u32,
    pub download: String,
    pub files: Vec<PackFile>,
}

impl PackDetail {
    /// Files the harvester can parse.
    pub fn art_files(&self) -> usize {
        self.files.iter().filter(|f| is_art_file(&f.name)).count()
    }

    /// Distinct artists, in file order.
    pub fn artists(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for a in self.files.iter().flat_map(|f| &f.artists) {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
        out
    }
}

/// A JSON string or number as text (16colo.rs has packs named `14`).
pub(crate) fn json_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub(crate) fn json_texts(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(json_text).collect()
}

/// `GET /v1/year`: `{"1996": {"packs": 836, "mags": 90}, …}` → years with packs.
pub fn parse_years(json: &[u8]) -> Result<Vec<YearInfo>> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs year index")?;
    let obj = v.as_object().ok_or_else(|| anyhow!("16colo.rs year index is not an object"))?;
    let mut out: Vec<YearInfo> = obj
        .iter()
        .filter_map(|(k, v)| Some(YearInfo { year: k.parse().ok()?, packs: v["packs"].as_u64().unwrap_or(0) as usize }))
        .filter(|y| y.packs > 0)
        .collect();
    out.sort_by_key(|y| y.year);
    Ok(out)
}

/// One page of `GET /v1/year/<y>` or `/v1/pack/?filter=`: packs and the page count.
pub fn parse_pack_page(json: &[u8]) -> Result<(Vec<PackInfo>, usize)> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs pack list")?;
    let pages = v["page"]["pages"].as_u64().unwrap_or(1) as usize;
    let packs = v["results"]
        .as_array()
        .ok_or_else(|| anyhow!("16colo.rs pack list has no results"))?
        .iter()
        .filter_map(|r| {
            Some(PackInfo {
                name: json_text(&r["name"])?,
                year: r["year"].as_u64().unwrap_or(0) as u32,
                groups: json_texts(&r["groups"]),
            })
        })
        .collect();
    Ok((packs, pages))
}

/// `GET /v1/pack/<pack>`: files with artists and titles, and the zip URL.
pub fn parse_pack_detail(pack: &str, json: &[u8]) -> Result<PackDetail> {
    let v: Value = serde_json::from_slice(json).context("16colo.rs API reply")?;
    let r = &v["results"][0];
    let dl = r["download"].as_str().map(str::to_string).or_else(|| {
        let (y, a) = (r["year"].as_u64()?, r["archive"].as_str()?);
        Some(format!("/archive/{y}/{a}"))
    });
    let dl = dl.ok_or_else(|| anyhow!("16colo.rs has no pack named {pack:?}"))?;
    let mut files: Vec<PackFile> = r["files"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, f)| PackFile {
            name: name.clone(),
            artists: json_texts(&f["artists"]),
            content: json_texts(&f["content"]),
        })
        .collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(PackDetail {
        name: pack.to_string(),
        year: r["year"].as_u64().unwrap_or(0) as u32,
        download: if dl.starts_with("http") { dl } else { format!("https://16colo.rs{dl}") },
        files,
    })
}

/// Years on 16colo.rs that have packs (cached for offline use).
pub fn sixteen_colors_years(cache_dir: &Path) -> Result<Vec<YearInfo>> {
    parse_years(&api_get(&format!("{SIXTEEN_API}/year"), cache_dir, "years.json")?)
}

/// Every pack released in `year` (all pages, 500 per request).
pub fn sixteen_colors_year(year: u32, cache_dir: &Path) -> Result<Vec<PackInfo>> {
    let mut out = vec![];
    let mut page = 1;
    loop {
        let url = format!("{SIXTEEN_API}/year/{year}?pagesize=500&page={page}");
        let (packs, pages) = parse_pack_page(&api_get(&url, cache_dir, &format!("year-{year}-{page}.json"))?)?;
        let empty = packs.is_empty();
        out.extend(packs);
        if empty || page >= pages || page >= 40 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

/// Packs whose name matches `filter` (any year).
pub fn sixteen_colors_search(filter: &str, cache_dir: &Path) -> Result<Vec<PackInfo>> {
    let q: String = filter.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
    let url = format!("{SIXTEEN_API}/pack/?filter={q}&pagesize=500&page=1");
    Ok(parse_pack_page(&api_get(&url, cache_dir, &format!("search-{q}.json"))?)?.0)
}

/// Files, artists and titles of one pack.
pub fn sixteen_colors_pack(pack: &str, cache_dir: &Path) -> Result<PackDetail> {
    let body = api_get(&format!("{SIXTEEN_API}/pack/{pack}"), cache_dir, &format!("pack-{pack}.json"))?;
    parse_pack_detail(pack, &body)
}

fn walk(p: &Path) -> Vec<PathBuf> {
    if p.is_file() {
        return vec![p.to_path_buf()];
    }
    let mut out = vec![];
    let mut entries: Vec<PathBuf> =
        std::fs::read_dir(p).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    entries.sort();
    for e in entries {
        out.extend(walk(&e));
    }
    out
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

// ------------------------------------------------------------------ segmentation

/// A connected blob of ink cells.
#[derive(Clone, Debug, PartialEq)]
pub struct Blob {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub cells: Vec<(usize, usize)>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The single color a cell shows, if it is visually flat (space, full block,
/// or a half block with equal halves).
fn flat_color(c: &acidtrip_core::Cell) -> Option<Color> {
    match c.ch {
        ' ' | '\u{0}' | '\u{A0}' => Some(c.bg),
        '█' => Some(c.fg),
        '▀' | '▄' | '▌' | '▐' if c.fg == c.bg => Some(c.fg),
        _ => None,
    }
}

/// Cells that belong to the backdrop: flat cells flood-filled from the
/// canvas edges through the same color (a logo on a gray field separates
/// from the field). Plain blank cells always count as backdrop.
pub fn backdrop(g: &Grid) -> Vec<bool> {
    let (w, h) = (g.width, g.height);
    let mut bg = vec![false; w * h];
    let mut stack = vec![];
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h.saturating_sub(1)));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w.saturating_sub(1), y));
    }
    let mut seed_color = vec![None; w * h];
    for &(x, y) in &stack {
        seed_color[y * w + x] = flat_color(&g.get(x, y));
    }
    while let Some((x, y)) = stack.pop() {
        let i = y * w + x;
        if bg[i] {
            continue;
        }
        let Some(col) = flat_color(&g.get(x, y)) else { continue };
        if seed_color[i].is_some_and(|s| s != col) {
            continue;
        }
        bg[i] = true;
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < w && ny < h && !bg[ny * w + nx] && flat_color(&g.get(nx, ny)) == Some(col) {
                seed_color[ny * w + nx] = Some(col);
                stack.push((nx, ny));
            }
        }
    }
    for (i, c) in g.cells.iter().enumerate() {
        if c.is_blank() {
            bg[i] = true;
        }
    }
    bg
}

/// Connected components of non-backdrop cells: 8-connected, and horizontal
/// gaps of one blank column are bridged (letters of a logo stay together).
pub fn segment(g: &Grid) -> Vec<Blob> {
    let (w, h) = (g.width, g.height);
    let back = backdrop(g);
    let ink = |x: usize, y: usize| !back[y * w + x];
    let mut parent: Vec<usize> = (0..w * h).collect();
    for y in 0..h {
        for x in 0..w {
            if !ink(x, y) {
                continue;
            }
            let i = y * w + x;
            let mut neigh = vec![(x + 1, y), (x + 2, y)];
            if y + 1 < h {
                for nx in x.saturating_sub(2)..=(x + 2) {
                    neigh.push((nx, y + 1));
                }
            }
            for (nx, ny) in neigh {
                if nx < w && ny < h && ink(nx, ny) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, ny * w + nx));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            if ink(x, y) {
                let r = find(&mut parent, y * w + x);
                groups.entry(r).or_default().push((x, y));
            }
        }
    }
    groups
        .into_values()
        .map(|cells| {
            let (x0, x1) = cells.iter().fold((usize::MAX, 0), |(a, b), &(x, _)| (a.min(x), b.max(x)));
            let (y0, y1) = cells.iter().fold((usize::MAX, 0), |(a, b), &(_, y)| (a.min(y), b.max(y)));
            Blob { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1, cells }
        })
        .collect()
}

const BLOCK_GLYPHS: &str = "█▀▄▌▐░▒▓■▬▪";

/// Logo-likeness in 0..1; 0 when outside the size window (width 12-80,
/// height 3-20, at least 20 ink cells).
pub fn score(g: &Grid, b: &Blob) -> f32 {
    if !(12..=80).contains(&b.w) || !(3..=20).contains(&b.h) || b.cells.len() < 20 {
        return 0.0;
    }
    let n = b.cells.len() as f32;
    let aspect = b.w as f32 / b.h as f32;
    let aspect_s = ((aspect - 1.5) / 3.0).clamp(0.0, 1.0);
    let density = n / (b.w * b.h) as f32;
    let density_s = (1.0 - (density - 0.6).abs() / 0.6).clamp(0.0, 1.0);
    let mut colors = std::collections::HashSet::new();
    let (mut blocks, mut letters) = (0usize, 0usize);
    for &(x, y) in &b.cells {
        let c = g.get(x, y);
        if c.ch != ' ' {
            colors.insert(c.fg);
        }
        if c.bg != Color::BLACK {
            colors.insert(c.bg);
        }
        if BLOCK_GLYPHS.contains(c.ch) || (c.ch == ' ' && c.bg != Color::BLACK) {
            blocks += 1;
        }
        if c.ch.is_ascii_alphanumeric() || ".,:;!?'\"()-".contains(c.ch) {
            letters += 1;
        }
    }
    let colors_s = (colors.len().min(5) as f32 - 1.0).max(0.0) / 4.0;
    let block_s = blocks as f32 / n;
    let text_frac = letters as f32 / n;
    let base = 0.25 * aspect_s + 0.2 * density_s + 0.2 * colors_s + 0.35 * block_s;
    (base * (1.0 - 0.9 * text_frac)).clamp(0.0, 1.0)
}

/// Top logo candidates of a piece (score > 0.2, best first, at most 12).
pub fn candidates(art: &SourceArt) -> Vec<Candidate> {
    let g = art.doc.flatten();
    let stem = sanitize(&art.attribution.file);
    let mut out: Vec<Candidate> = segment(&g)
        .into_iter()
        .filter_map(|b| {
            let s = score(&g, &b);
            (s > 0.2).then(|| {
                let mut clip = Clip::new(b.w, b.h);
                for &(x, y) in &b.cells {
                    clip.set(x - b.x, y - b.y, Some(g.get(x, y)));
                }
                Candidate {
                    id: format!("{stem}@{},{}", b.x, b.y),
                    x: b.x,
                    y: b.y,
                    w: b.w,
                    h: b.h,
                    score: s,
                    clip,
                    attribution: art.attribution.clone(),
                }
            })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    // Tutorials and packs repeat the same drawing; keep one of each.
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| seen.insert(serde_json::to_string(&c.clip).unwrap_or_default()));
    out.truncate(12);
    out
}

/// Render a candidate at `scale` (8x16 px per cell at scale 1).
pub fn render_candidate(c: &Candidate, scale: u32) -> Vec<u8> {
    png_bytes(&render_grid(&c.clip.to_grid(), &Palette::default(), RenderOptions { scale, nine_px: false }))
}

// ------------------------------------------------------------------ letter reading

const READ_SYSTEM: &str =
    "You read lettering in ANSI art logos from the BBS art scene. Answer with one strict JSON object and nothing else.";

pub fn reading_prompt(c: &Candidate, scale: u32) -> String {
    format!(
        "This image is an ANSI art logo made of a grid of text cells: {w} columns x {h} rows. Each cell is {cw} px wide and {ch} px \
         tall in the image (so the image is {pw}x{ph} px); column 0 is the left edge.\n\
         Read the text it spells and give the column span of every letter: x0 = first column, x1 = last column (inclusive), \
         0 <= x0 <= x1 <= {last}. Spans should cover the letter's full body including its outline/shadow but not the next \
         letter; spans must not overlap. Skip decorations, artist tags and small signatures that are not part of the word.\n\
         Also give a short lowercase style tag (e.g. \"chrome\", \"fire\", \"blocky\", \"outline\", \"graffiti\", \"shaded\").\n\
         Reply exactly: {{\"text\": \"...\", \"letters\": [{{\"char\": \"A\", \"x0\": 0, \"x1\": 7}}], \"style\": \"...\"}}\n\
         If it is not lettering, reply {{\"text\": \"\", \"letters\": [], \"style\": \"not-text\"}}.",
        w = c.w,
        h = c.h,
        cw = 8 * scale,
        ch = 16 * scale,
        pw = c.w as u32 * 8 * scale,
        ph = c.h as u32 * 16 * scale,
        last = c.w.saturating_sub(1)
    )
}

/// Parse and validate a model reply for a candidate `width` cells wide:
/// JSON may be wrapped in prose or a code fence; invalid spans are dropped,
/// overlapping spans are trimmed.
pub fn parse_reading(text: &str, width: usize) -> Result<LetterReading> {
    let (a, b) = (text.find('{'), text.rfind('}'));
    let (Some(a), Some(b)) = (a, b) else { bail!("no JSON object in reply: {text:?}") };
    let v: Value = serde_json::from_str(&text[a..=b]).context("reply is not valid JSON")?;
    let mut r = LetterReading {
        text: v["text"].as_str().unwrap_or("").to_string(),
        style: v["style"].as_str().unwrap_or("").trim().to_lowercase(),
        letters: vec![],
    };
    for l in v["letters"].as_array().into_iter().flatten() {
        let ch = l["char"].as_str().and_then(|s| s.chars().next());
        let (x0, x1) = (l["x0"].as_u64(), l["x1"].as_u64());
        if let (Some(ch), Some(x0), Some(x1)) = (ch, x0, x1) {
            r.letters.push(LetterSpan { ch, x0: x0 as usize, x1: x1 as usize });
        }
    }
    Ok(validate_reading(r, width))
}

pub fn validate_reading(mut r: LetterReading, width: usize) -> LetterReading {
    r.letters.retain(|l| !l.ch.is_whitespace() && l.x0 <= l.x1 && l.x0 < width);
    for l in &mut r.letters {
        l.x1 = l.x1.min(width.saturating_sub(1));
    }
    r.letters.sort_by_key(|l| l.x0);
    let mut out: Vec<LetterSpan> = vec![];
    for mut l in r.letters {
        if let Some(prev) = out.last()
            && l.x0 <= prev.x1
        {
            l.x0 = prev.x1 + 1;
        }
        if l.x0 <= l.x1 {
            out.push(l);
        }
    }
    r.letters = out;
    r
}

/// Ask Claude (vision) to read a candidate's letters.
pub fn read_letters(cfg: &AgentConfig, cand: &Candidate) -> Result<LetterReading> {
    read_letters_with(&HttpTransport::new(&cfg.api_key), cfg, cand)
}

pub fn read_letters_with(t: &dyn Transport, cfg: &AgentConfig, cand: &Candidate) -> Result<LetterReading> {
    let scale = if cand.w <= 40 { 2 } else { 1 };
    let reply =
        agent::one_shot_with(t, cfg, READ_SYSTEM, &reading_prompt(cand, scale), Some(&render_candidate(cand, scale)))?;
    parse_reading(&reply, cand.w)
}

// ------------------------------------------------------------------ cutting

fn is_ink(c: Option<Cell>) -> bool {
    c.is_some_and(|c| !c.is_blank())
}

/// Sub-clip of columns x0..=x1 and rows y0..=y1.
pub fn cut(clip: &Clip, x0: usize, x1: usize, y0: usize, y1: usize) -> Clip {
    let (w, h) = (x1 + 1 - x0, y1 + 1 - y0);
    let mut out = Clip::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.set(x, y, clip.get(x0 + x, y0 + y).filter(|c| !c.is_blank()));
        }
    }
    out
}

/// Trim blank columns on both sides (rows are kept so glyphs share a baseline).
pub fn trim_cols(clip: &Clip) -> Clip {
    let used: Vec<usize> = (0..clip.width).filter(|&x| (0..clip.height).any(|y| is_ink(clip.get(x, y)))).collect();
    match (used.first(), used.last()) {
        (Some(&a), Some(&b)) => cut(clip, a, b, 0, clip.height - 1),
        _ => Clip::new(0, clip.height),
    }
}

/// Rows `(first, last)` that contain ink, if any.
fn ink_rows(clip: &Clip) -> Option<(usize, usize)> {
    let rows: Vec<usize> = (0..clip.height).filter(|&y| (0..clip.width).any(|x| is_ink(clip.get(x, y)))).collect();
    Some((*rows.first()?, *rows.last()?))
}

/// A TheDraw glyph (from fonts harvested before `.acidfont`) as a clip (black spaces become transparent).
pub fn glyph_to_clip(g: &Glyph) -> Clip {
    let mut clip = Clip::new(g.width, g.height);
    let (mut x, mut y) = (0, 0);
    for p in &g.parts {
        match p {
            GlyphPart::NewLine => {
                x = 0;
                y += 1;
            }
            GlyphPart::AnsiChar { ch, fg, bg, .. } => {
                let c = Cell::new(*ch, Color::Pal(*fg), Color::Pal(*bg));
                clip.set(x, y, (!c.is_blank()).then_some(c));
                x += 1;
            }
            GlyphPart::Char(ch) => {
                clip.set(x, y, (*ch != ' ').then(|| Cell::new(*ch, Color::LIGHT_GRAY, Color::BLACK)));
                x += 1;
            }
            _ => x += 1,
        }
    }
    clip
}

/// A candidate as a stencil named for what it spells.
pub fn stencil_for(c: &Candidate, r: &LetterReading) -> Stencil {
    let a = &c.attribution;
    let mut tags = vec!["logo".to_string(), "harvested".to_string()];
    if !r.style.is_empty() && r.style != "not-text" {
        tags.push(r.style.clone());
    }
    let name = if r.text.trim().is_empty() { format!("{} logo", a.owner()) } else { r.text.trim().to_string() };
    Stencil {
        meta: StencilMeta {
            id: String::new(),
            name,
            tags,
            author: a.author.clone(),
            group: a.group.clone(),
            source: [a.source.as_str(), a.file.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" : "),
            license: LICENSE.into(),
            generated: false,
            created: chrono::Local::now().to_rfc3339(),
        },
        clip: c.clip.clone(),
    }
}

/// Cut letters into partial fonts grouped by (artist, style) and turn every
/// candidate into a stencil.
pub fn build(readings: &[(Candidate, LetterReading)]) -> HarvestResult {
    let mut res = HarvestResult::default();
    let mut fonts: BTreeMap<(String, String), (FontSpec, Attribution)> = BTreeMap::new();
    for (cand, rd) in readings {
        res.stencils.push(stencil_for(cand, rd));
        let rd = validate_reading(rd.clone(), cand.w);
        if rd.letters.is_empty() {
            continue;
        }
        // Rows shared by all letters of this logo, so glyphs keep one baseline.
        let spans: Vec<Clip> = rd.letters.iter().map(|l| cut(&cand.clip, l.x0, l.x1, 0, cand.h - 1)).collect();
        let rows: Vec<(usize, usize)> = spans.iter().filter_map(ink_rows).collect();
        let Some(y0) = rows.iter().map(|r| r.0).min() else {
            continue;
        };
        let y1 = rows.iter().map(|r| r.1).max().unwrap_or(y0);
        let owner = cand.attribution.owner();
        let style = if rd.style.is_empty() { "logo".to_string() } else { rd.style.clone() };
        let key = (owner.to_lowercase(), style.to_lowercase());
        let (spec, _) = fonts.entry(key).or_insert_with(|| {
            let a = cand.attribution.clone();
            (
                FontSpec {
                    name: format!("{owner} {style}"),
                    style: style.clone(),
                    spacing: 1,
                    ..Default::default()
                },
                a,
            )
        });
        for (l, span) in rd.letters.iter().zip(&spans) {
            let g = trim_cols(&cut(span, 0, span.width - 1, y0, y1));
            if g.width == 0 {
                res.skipped.push(format!("{:?} in {}: empty", l.ch, cand.id));
            } else if l.ch.is_control() || l.ch.is_whitespace() {
                res.skipped.push(format!("{:?} in {}: not a letter", l.ch, cand.id));
            } else {
                spec.glyphs.entry(l.ch).or_insert(g);
            }
        }
    }
    res.fonts = fonts.into_values().filter(|(s, _)| !s.glyphs.is_empty()).collect();
    res
}

// ------------------------------------------------------------------ letter splits
//
// A "split" `s` is a boundary between columns `s - 1` and `s` of a candidate;
// `n` letters need `n - 1` increasing splits in `1..width`.

/// Ink cells per column.
pub fn column_ink(clip: &Clip) -> Vec<usize> {
    (0..clip.width).map(|x| (0..clip.height).filter(|&y| is_ink(clip.get(x, y))).count()).collect()
}

/// Cost of cutting between columns `s - 1` and `s`: rows where ink touches
/// across the cut weigh most, then the thinner column's ink.
fn cut_cost(clip: &Clip, ink: &[usize], s: usize) -> f32 {
    let touching = (0..clip.height).filter(|&y| is_ink(clip.get(s - 1, y)) && is_ink(clip.get(s, y))).count();
    2.0 * touching as f32 + ink[s - 1].min(ink[s]) as f32
}

/// Splits in the middle of every blank column run between inked columns.
pub fn gap_splits(clip: &Clip) -> Vec<usize> {
    let ink = column_ink(clip);
    let (Some(a), Some(b)) = (ink.iter().position(|&n| n > 0), ink.iter().rposition(|&n| n > 0)) else {
        return vec![];
    };
    let mut out = vec![];
    let mut x = a;
    while x <= b {
        if ink[x] == 0 {
            let start = x;
            while ink[x] == 0 {
                x += 1;
            }
            out.push(start + (x - start) / 2);
        } else {
            x += 1;
        }
    }
    out
}

/// `letters - 1` splits spread evenly over `width` columns.
pub fn even_splits(width: usize, letters: usize) -> Vec<usize> {
    if letters < 2 || width < letters {
        return vec![];
    }
    (1..letters).map(|i| i * width / letters).collect()
}

/// Best `letters - 1` splits: through blank or low-ink columns where the
/// letters don't touch, keeping letter widths near even. With `letters`
/// 0 (text unknown) the blank gaps alone decide.
pub fn auto_splits(clip: &Clip, letters: usize) -> Vec<usize> {
    if letters == 0 {
        return gap_splits(clip);
    }
    let ink = column_ink(clip);
    let (Some(a), Some(b)) = (ink.iter().position(|&n| n > 0), ink.iter().rposition(|&n| n > 0)) else {
        return even_splits(clip.width, letters);
    };
    let span = b + 1 - a;
    let k = letters - 1;
    if k == 0 {
        return vec![];
    }
    if span < letters {
        return even_splits(clip.width, letters);
    }
    let avg = span as f32 / letters as f32;
    let lambda = 1.5 * clip.height.max(1) as f32;
    let min_w = ((avg * 0.35).round() as usize).max(1);
    let width_pen = |w: usize| {
        if w < min_w {
            return f32::INFINITY;
        }
        let d = (w as f32 - avg) / avg;
        lambda * d * d
    };
    // Positions a+1..=b; dp[j][s] = best cost with split j at s.
    let pos: Vec<usize> = (a + 1..=b).collect();
    let cost: Vec<f32> = pos.iter().map(|&s| cut_cost(clip, &ink, s)).collect();
    let n = pos.len();
    let inf = f32::INFINITY;
    let mut dp = vec![vec![inf; n]; k];
    let mut from = vec![vec![usize::MAX; n]; k];
    for i in 0..n {
        dp[0][i] = cost[i] + width_pen(pos[i] - a);
    }
    for j in 1..k {
        for i in 0..n {
            for p in 0..i {
                if dp[j - 1][p].is_finite() {
                    let c = dp[j - 1][p] + cost[i] + width_pen(pos[i] - pos[p]);
                    if c < dp[j][i] {
                        dp[j][i] = c;
                        from[j][i] = p;
                    }
                }
            }
        }
    }
    let best = (0..n)
        .map(|i| (i, dp[k - 1][i] + width_pen(b + 1 - pos[i])))
        .filter(|(_, c)| c.is_finite())
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(i, _)| i);
    let Some(mut i) = best else {
        return even_splits(clip.width, letters);
    };
    let mut out = vec![pos[i]];
    for j in (1..k).rev() {
        i = from[j][i];
        out.push(pos[i]);
    }
    out.reverse();
    out
}

/// Splits between the letters of a reading (midway through any gap).
pub fn splits_from_reading(r: &LetterReading, width: usize) -> Vec<usize> {
    let r = validate_reading(r.clone(), width);
    r.letters.windows(2).map(|w| (w[0].x1 + 1 + w[1].x0).div_ceil(2).clamp(1, width.saturating_sub(1).max(1))).collect()
}

/// A reading from the cutter: segment `i` (between splits) gets the `i`-th
/// char of `text`; spaces and segments past the text end are skipped.
pub fn reading_from_splits(text: &str, splits: &[usize], width: usize, style: &str) -> LetterReading {
    let mut bounds = vec![0];
    bounds.extend(splits.iter().copied().filter(|&s| s > 0 && s < width));
    bounds.push(width);
    let letters = text
        .chars()
        .zip(bounds.windows(2))
        .filter(|(ch, w)| !ch.is_whitespace() && w[1] > w[0])
        .map(|(ch, w)| LetterSpan { ch, x0: w[0], x1: w[1] - 1 })
        .collect();
    LetterReading { text: text.trim().to_string(), letters, style: style.trim().to_lowercase() }
}

/// Give every letter its other case too when that is missing, so a logo read
/// as "abraxas" also types "ABRAXAS". Copies of AI-drawn letters stay marked
/// as drawn.
pub fn fill_other_case(spec: &mut FontSpec) {
    let copies: Vec<(char, char, Clip)> = spec
        .glyphs
        .iter()
        .filter_map(|(&ch, clip)| {
            let other = if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else {
                return None;
            };
            (!spec.glyphs.contains_key(&other)).then(|| (ch, other, clip.clone()))
        })
        .collect();
    for (from, to, clip) in copies {
        let base = spec.bases.get(&from).copied();
        spec.insert(to, clip, base);
        if spec.generated.contains(&from) {
            spec.generated.insert(to);
        }
    }
}

/// File name (inside `<fonts_dir>/harvested/`) of the font for an artist and style.
pub fn font_file_name(attribution: &Attribution, style: &str) -> String {
    let style = if style.trim().is_empty() { "logo" } else { style.trim() };
    format!("{}.{CUT_FONT_EXT}", sanitize(&format!("{}-{}", attribution.owner(), style)).to_lowercase())
}

// ------------------------------------------------------------------ writing

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WriteReport {
    pub font_files: Vec<PathBuf>,
    pub stencil_ids: Vec<String>,
}

/// A harvested font file: the `.acidfont` fields plus where it came from.
#[derive(Serialize, Deserialize, Default)]
struct FontFile {
    #[serde(flatten)]
    font: CutFont,
    #[serde(default)]
    style: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    attribution: Vec<Attribution>,
    /// Letters Claude drew.
    #[serde(default)]
    generated: String,
}

/// What fonts harvested before `.acidfont` kept next to their `.tdf`.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct FontSidecar {
    name: String,
    style: String,
    license: String,
    attribution: Vec<Attribution>,
    glyphs: String,
    generated: String,
}

/// The chars a complete font covers, in the order coverage is shown.
pub const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// What Claude draws when asked to complete a font (lowercase follows by case).
pub const COMPLETE_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// A harvested font on disk: `<fonts_dir>/harvested/<owner>-<style>.acidfont`,
/// holding the artists it came from and which letters Claude drew. Fonts
/// harvested earlier as TheDraw `.tdf` + `.tdf.json` still load; saving one
/// moves it to `.acidfont`.
#[derive(Clone, Debug, PartialEq)]
pub struct HarvestedFont {
    pub file: PathBuf,
    pub spec: FontSpec,
    pub attribution: Vec<Attribution>,
    /// The `.tdf` it was loaded from, removed on save.
    legacy: Option<PathBuf>,
}

impl HarvestedFont {
    pub fn dir(fonts_dir: &Path) -> PathBuf {
        fonts_dir.join("harvested")
    }

    /// An empty font at `file` (nothing written yet).
    pub fn new(file: PathBuf, name: &str, style: &str) -> HarvestedFont {
        let spec = FontSpec { name: name.into(), style: style.into(), spacing: 1, ..Default::default() };
        HarvestedFont { file, spec, attribution: vec![], legacy: None }
    }

    /// The font at `file` (or its older `.tdf`), or a new empty one there.
    pub fn open(file: PathBuf, name: &str, style: &str) -> HarvestedFont {
        let old = file.with_extension("tdf");
        HarvestedFont::load(&file)
            .or_else(|_| HarvestedFont::load(&old))
            .unwrap_or_else(|_| HarvestedFont::new(file, name, style))
    }

    /// Every harvested font, by name. Unreadable files are skipped.
    pub fn list(fonts_dir: &Path) -> Vec<HarvestedFont> {
        let Ok(rd) = std::fs::read_dir(Self::dir(fonts_dir)) else { return vec![] };
        let files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        let ext = |p: &Path, x: &str| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(x));
        let mut v: Vec<HarvestedFont> = files
            .iter()
            // An old .tdf that has an .acidfont beside it was moved already.
            .filter(|p| {
                ext(p, CUT_FONT_EXT) || (ext(p, "tdf") && !files.contains(&p.with_extension(CUT_FONT_EXT)))
            })
            .filter_map(|p| HarvestedFont::load(p).ok())
            .collect();
        v.sort_by_key(|f| f.title().to_lowercase());
        v
    }

    pub fn load(file: &Path) -> Result<HarvestedFont> {
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        if !file.extension().is_some_and(|e| e.eq_ignore_ascii_case("tdf")) {
            let f: FontFile = serde_json::from_slice(&bytes).with_context(|| format!("reading {}", file.display()))?;
            let glyphs: BTreeMap<char, Clip> = f.font.glyphs.iter().map(|(&c, g)| (c, g.clip.clone())).collect();
            let bases = f.font.glyphs.iter().filter_map(|(&c, g)| Some((c, g.base?))).collect();
            let generated = f.generated.chars().filter(|c| glyphs.contains_key(c)).collect();
            let spec =
                FontSpec { name: f.font.name, style: f.style, spacing: f.font.spacing, glyphs, bases, generated };
            return Ok(HarvestedFont { file: file.to_path_buf(), spec, attribution: f.attribution, legacy: None });
        }
        let font = TdfFont::load(&bytes)
            .map_err(|e| anyhow!("{}: {e:?}", file.display()))?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("{}: no font inside", file.display()))?;
        let meta: FontSidecar =
            std::fs::read(Self::sidecar(file)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let glyphs: BTreeMap<char, Clip> = font.iter_glyphs().map(|(ch, g)| (ch, glyph_to_clip(g))).collect();
        let generated = meta.generated.chars().filter(|c| glyphs.contains_key(c)).collect();
        let spec = FontSpec {
            name: if meta.name.is_empty() { font.name.trim().to_string() } else { meta.name },
            style: meta.style,
            spacing: font.spacing,
            glyphs,
            bases: BTreeMap::new(),
            generated,
        };
        Ok(HarvestedFont {
            file: file.with_extension(CUT_FONT_EXT),
            spec,
            attribution: meta.attribution,
            legacy: Some(file.to_path_buf()),
        })
    }

    fn sidecar(file: &Path) -> PathBuf {
        file.with_extension("tdf.json")
    }

    pub fn save(&self) -> Result<()> {
        if let Some(d) = self.file.parent() {
            std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        let spec = &self.spec;
        let f = FontFile {
            font: spec.to_cut(),
            style: spec.style.clone(),
            license: LICENSE.into(),
            attribution: self.attribution.clone(),
            generated: spec.generated.iter().collect(),
        };
        acidtrip_io::library::write_atomic(&self.file, &serde_json::to_vec(&f)?)
            .with_context(|| format!("writing {}", self.file.display()))?;
        self.remove_legacy();
        Ok(())
    }

    fn remove_legacy(&self) {
        if let Some(old) = &self.legacy {
            let _ = std::fs::remove_file(old);
            let _ = std::fs::remove_file(Self::sidecar(old));
        }
    }

    /// Remove the font (and the `.tdf` it came from).
    pub fn delete(&self) -> Result<()> {
        if self.file.exists() || self.legacy.is_none() {
            std::fs::remove_file(&self.file).with_context(|| format!("deleting {}", self.file.display()))?;
        }
        self.remove_legacy();
        Ok(())
    }

    /// "<name>" plus the style when the name doesn't already say it.
    pub fn title(&self) -> String {
        let s = &self.spec;
        if s.style.is_empty() || s.name.to_lowercase().contains(&s.style.to_lowercase()) {
            s.name.clone()
        } else {
            format!("{} ({})", s.name, s.style)
        }
    }

    /// Artists credited, first one first.
    pub fn artists(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![];
        for a in &self.attribution {
            let o = a.owner();
            if !v.contains(&o) {
                v.push(o);
            }
        }
        v
    }

    /// Chars of `want` the font has no glyph for.
    pub fn missing(&self, want: &str) -> Vec<char> {
        want.chars().filter(|c| !self.spec.glyphs.contains_key(c)).collect()
    }

    /// Drop one glyph.
    pub fn remove(&mut self, ch: char) {
        self.spec.glyphs.remove(&ch);
        self.spec.bases.remove(&ch);
        self.spec.generated.remove(&ch);
    }

    /// Put new glyphs in (they replace old ones), filling the other case
    /// where it's missing, and credit `attribution` (unless it's empty:
    /// Claude's letters credit nobody new).
    pub fn merge(&mut self, new: &FontSpec, attribution: &Attribution) {
        for (&ch, clip) in &new.glyphs {
            self.spec.insert(ch, clip.clone(), new.bases.get(&ch).copied());
            if new.generated.contains(&ch) {
                self.spec.generated.insert(ch);
            } else {
                self.spec.generated.remove(&ch);
            }
        }
        fill_other_case(&mut self.spec);
        if *attribution != Attribution::default() && !self.attribution.contains(attribution) {
            self.attribution.push(attribution.clone());
        }
    }
}

/// Write fonts as `<fonts_dir>/harvested/<owner>-<style>.acidfont` (merging
/// new glyphs into an existing file) and save stencils into the library.
pub fn write(
    res: &HarvestResult,
    fonts_dir: &Path,
    stencils: &mut StencilLibrary,
    stencils_dir: &Path,
) -> Result<WriteReport> {
    let mut rep = WriteReport::default();
    for (spec, attribution) in &res.fonts {
        let file = HarvestedFont::dir(fonts_dir).join(font_file_name(attribution, &spec.style));
        let mut font = HarvestedFont::open(file.clone(), &spec.name, &spec.style);
        font.spec.name = spec.name.clone();
        font.spec.style = spec.style.clone();
        font.spec.spacing = spec.spacing;
        font.merge(spec, attribution);
        font.save()?;
        rep.font_files.push(file);
    }
    for st in &res.stencils {
        rep.stencil_ids.push(stencils.save(stencils_dir, st.clone())?.id);
    }
    Ok(rep)
}

// ------------------------------------------------------------------ completion

/// A throwaway document plus everything [`exec::execute`] needs.
pub struct Scratch {
    pub doc: Document,
    pub history: History,
    pub fonts: FontLibrary,
    pub stencils: StencilLibrary,
    pub paths: Paths,
    pub file: Option<PathBuf>,
    pub layer: usize,
    _dir: tempfile::TempDir,
}

impl Scratch {
    pub fn new(doc: Document) -> Result<Scratch> {
        let dir = tempfile::tempdir()?;
        let paths = Paths {
            config_dir: dir.path().join("config"),
            data_dir: dir.path().join("data"),
            state_dir: dir.path().join("state"),
        };
        std::fs::create_dir_all(paths.stencils_dir())?;
        let stencils = StencilLibrary::load(&paths.stencils_dir());
        Ok(Scratch {
            doc,
            history: History::new(),
            fonts: FontLibrary::default(),
            stencils,
            paths,
            file: None,
            layer: 0,
            _dir: dir,
        })
    }

    pub fn execute(&mut self, call: &crate::ToolCall) -> ToolResult {
        let mut st = ExecState {
            doc: &mut self.doc,
            history: &mut self.history,
            layer: self.layer,
            fonts: &self.fonts,
            stencils: &mut self.stencils,
            paths: &self.paths,
            file: &mut self.file,
        };
        let r = exec::execute(&mut st, call);
        self.layer = st.layer;
        r
    }
}

/// Box on the scratch canvas where a missing char gets drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
    pub ch: char,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// Lay out existing glyphs as samples and boxes for `missing` on a scratch
/// doc. Returns (doc, sample positions, boxes).
pub fn completion_layout(spec: &FontSpec, missing: &[char]) -> (Document, Vec<GlyphBox>, Vec<GlyphBox>) {
    const W: usize = 80;
    let gh = spec.height().max(1);
    let bw = spec.glyphs.values().map(|c| c.width).max().unwrap_or(8).clamp(4, 40);
    let (mut x, mut y) = (0, 0);
    let mut samples = vec![];
    for (&ch, c) in &spec.glyphs {
        if x + c.width > W {
            x = 0;
            y += gh + 1;
        }
        samples.push(GlyphBox { ch, x, y, w: c.width, h: c.height });
        x += c.width + 2;
    }
    let (mut bx, mut by) = (0, y + gh + 2);
    let mut boxes = vec![];
    for &ch in missing {
        if bx + bw > W {
            bx = 0;
            by += gh + 2;
        }
        boxes.push(GlyphBox { ch, x: bx, y: by, w: bw, h: gh });
        bx += bw + 3;
    }
    let height = boxes.iter().map(|b| b.y + b.h).chain(samples.iter().map(|s| s.y + s.h)).max().unwrap_or(1) + 1;
    let mut doc = Document::new(DocKind::Classic, W, height);
    for (s, c) in samples.iter().zip(spec.glyphs.values()) {
        for gy in 0..c.height {
            for gx in 0..c.width {
                if let Some(cell) = c.get(gx, gy) {
                    doc.canvas.layers[0].cells[(s.y + gy) * W + s.x + gx] = Some(cell);
                }
            }
        }
    }
    (doc, samples, boxes)
}

/// Draw missing glyphs of `chars` (default A-Z0-9) in the font's style with
/// the agent loop on a scratch doc; new glyphs are marked `generated`.
pub fn complete_font(cfg: &AgentConfig, spec: &FontSpec, chars: Option<&str>) -> Result<FontSpec> {
    complete_font_with(Box::new(HttpTransport::new(&cfg.api_key)), cfg, spec, chars)
}

pub fn complete_font_with(
    t: Box<dyn Transport>,
    cfg: &AgentConfig,
    spec: &FontSpec,
    chars: Option<&str>,
) -> Result<FontSpec> {
    let want = chars.unwrap_or("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789");
    let missing: Vec<char> = want.chars().filter(|c| !spec.glyphs.contains_key(c) && !c.is_whitespace()).collect();
    if missing.is_empty() || spec.glyphs.is_empty() {
        return Ok(spec.clone());
    }
    let (doc, samples, boxes) = completion_layout(spec, &missing);
    let list = |v: &[GlyphBox]| {
        v.iter()
            .map(|b| format!("'{}' at x={} y={} w={} h={}", b.ch, b.x, b.y, b.w, b.h))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let prompt = format!(
        "You are completing a color text font cut from ANSI art, named {:?} (style: {}). The existing glyphs are drawn on the canvas as samples: {}.\n\
         Draw each of these missing characters inside its own empty box, in exactly the same style - same height ({} rows), similar \
         width, the same colors, glyph vocabulary, shading and outline treatment, so they could sit next to the samples in one word: {}.\n\
         Keep every character strictly inside its box, leave at least one blank column at the box edges if the character is narrow, \
         and don't touch the samples. Use set_cells (many cells per call) and fill_rect, check with render_png, and fix mistakes.",
        spec.name,
        spec.style,
        list(&samples),
        spec.height(),
        list(&boxes)
    );
    let (tx, rx) = mpsc::channel::<ToolRequest>();
    let worker = std::thread::spawn(move || -> Result<Document> {
        let mut s = Scratch::new(doc)?;
        for req in rx {
            let r = s.execute(&req.call);
            let _ = req.reply.send(r);
        }
        Ok(s.doc)
    });
    let (etx, erx) = mpsc::channel();
    let rounds = cfg.max_rounds.max(8 + 2 * missing.len());
    agent::run(&AgentConfig { max_rounds: rounds, ..cfg.clone() }, t.as_ref(), &prompt, "", &tx, &etx);
    drop(tx);
    let doc = worker.join().map_err(|_| anyhow!("scratch worker panicked"))??;
    if let Some(AgentEvent::Error(e)) = erx.try_iter().find(|e| matches!(e, AgentEvent::Error(_))) {
        bail!("font completion failed: {e}");
    }
    let mut out = spec.clone();
    // New letters sit on the baseline most of the old ones share.
    let mut rows: HashMap<usize, usize> = HashMap::new();
    for (ch, c) in &spec.glyphs {
        let under = c.height - 1 - spec.bases.get(ch).copied().unwrap_or(c.height - 1);
        *rows.entry(under).or_default() += 1;
    }
    let under = rows.into_iter().max_by_key(|&(u, n)| (n, std::cmp::Reverse(u))).map_or(0, |(u, _)| u);
    let g = doc.flatten();
    for b in boxes {
        let mut clip = Clip::new(b.w, b.h);
        for y in 0..b.h {
            for x in 0..b.w {
                let c = g.get(b.x + x, b.y + y);
                clip.set(x, y, (!c.is_blank()).then_some(c));
            }
        }
        let glyph = trim_cols(&clip);
        if glyph.width > 0 {
            let base = glyph.height.checked_sub(1 + under);
            out.insert(b.ch, glyph, base);
            out.generated.insert(b.ch);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ pipeline

#[derive(Clone, Debug)]
pub struct HarvestOptions {
    /// Candidates kept per art file.
    pub per_file: usize,
    /// Minimum logo score (0..1).
    pub min_score: f32,
    /// Ask the AI to draw missing A-Z0-9 glyphs.
    pub complete: bool,
}

impl Default for HarvestOptions {
    fn default() -> Self {
        HarvestOptions { per_file: 3, min_score: 0.35, complete: false }
    }
}

/// The whole pipeline for `acidtrip harvest`: fetch, find candidates, read
/// letters with Claude (without `cfg`, candidates become stencils only),
/// build, optionally complete fonts, write.
pub fn run(
    source: &str,
    cfg: Option<&AgentConfig>,
    opts: &HarvestOptions,
    paths: &Paths,
    stencils: &mut StencilLibrary,
    progress: &mut dyn FnMut(&str),
) -> Result<(HarvestResult, WriteReport)> {
    let cache = paths.state_dir.join("harvest-cache");
    progress(&format!("fetching {source}…"));
    let arts = fetch(source, &cache)?;
    progress(&format!("{} art files", arts.len()));
    let mut readings = vec![];
    for art in &arts {
        for c in candidates(art).into_iter().filter(|c| c.score >= opts.min_score).take(opts.per_file) {
            let rd = match cfg {
                Some(cfg) => {
                    progress(&format!("reading {} ({}x{}, score {:.2})…", c.id, c.w, c.h, c.score));
                    read_letters(cfg, &c).unwrap_or_else(|e| {
                        progress(&format!("  {e:#}"));
                        LetterReading::default()
                    })
                }
                None => LetterReading::default(),
            };
            if !rd.text.is_empty() {
                progress(&format!("  {:?} ({}), {} letters", rd.text, rd.style, rd.letters.len()));
            }
            readings.push((c, rd));
        }
    }
    let mut res = build(&readings);
    if let (true, Some(cfg)) = (opts.complete, cfg) {
        for (spec, _) in &mut res.fonts {
            progress(&format!("completing font {}…", spec.name));
            match complete_font(cfg, spec, None) {
                Ok(s) => *spec = s,
                Err(e) => progress(&format!("  {e:#}")),
            }
        }
    }
    let rep = write(&res, &paths.fonts_dir(), stencils, &paths.stencils_dir())?;
    progress(&format!("wrote {} fonts and {} stencils", rep.font_files.len(), rep.stencil_ids.len()));
    Ok((res, rep))
}

// ------------------------------------------------------------------ MCP session

/// State behind the MCP `harvest_candidates` / `harvest_commit` tools.
#[derive(Default)]
pub struct Session {
    pub candidates: HashMap<String, Candidate>,
}

impl Session {
    /// Returns MCP content blocks (text + one image per candidate).
    pub fn candidates_tool(&mut self, args: &Value, cache_dir: &Path) -> Result<Vec<Value>> {
        let source = args["source"].as_str().ok_or_else(|| anyhow!("source is required"))?;
        let limit = args["limit"].as_u64().unwrap_or(8).clamp(1, 24) as usize;
        let arts = fetch(source, cache_dir)?;
        let mut all: Vec<Candidate> = arts.iter().flat_map(candidates).collect();
        all.sort_by(|a, b| b.score.total_cmp(&a.score));
        all.truncate(limit);
        let mut content = vec![json!({ "type": "text", "text": format!(
            "{} art files, {} candidates. For each image: read the letters and call harvest_commit with the id, text, style and \
             per-letter column spans (x0..x1 inclusive; each cell is 8 px wide in the image at scale 1, 16 px at scale 2 as noted).",
            arts.len(), all.len()) })];
        for c in all {
            let scale = if c.w <= 40 { 2 } else { 1 };
            content.push(json!({ "type": "text", "text": format!(
                "id {} — {}x{} cells, score {:.2}, image scale {scale} ({} px per cell column), {}",
                c.id, c.w, c.h, c.score, 8 * scale, c.attribution.credit()) }));
            content.push(json!({ "type": "image", "data": base64_encode(&render_candidate(&c, scale)), "mimeType": "image/png" }));
            self.candidates.insert(c.id.clone(), c);
        }
        Ok(content)
    }

    pub fn commit_tool(&mut self, args: &Value, paths: &Paths, stencils: &mut StencilLibrary) -> Result<String> {
        let mut readings = vec![];
        let unknown = |id: &str| anyhow!("unknown candidate id {id:?}; call harvest_candidates first");
        for r in args["readings"].as_array().into_iter().flatten() {
            let id = r["id"].as_str().ok_or_else(|| anyhow!("each reading needs an id"))?;
            let c = self.candidates.get(id).ok_or_else(|| unknown(id))?.clone();
            let rd: LetterReading = serde_json::from_value(r.clone()).map_err(|e| anyhow!("reading {id}: {e}"))?;
            readings.push((c.clone(), validate_reading(rd, c.w)));
        }
        for id in args["ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !readings.iter().any(|(c, _)| c.id == id) {
                readings.push((self.candidates.get(id).ok_or_else(|| unknown(id))?.clone(), LetterReading::default()));
            }
        }
        if readings.is_empty() {
            bail!("give readings and/or ids");
        }
        let res = build(&readings);
        let rep = write(&res, &paths.fonts_dir(), stencils, &paths.stencils_dir())?;
        let mut s = format!("Saved {} stencils.", rep.stencil_ids.len());
        for ((spec, a), f) in res.fonts.iter().zip(&rep.font_files) {
            s.push_str(&format!(
                "\nFont {:?} ({} glyphs: {}) → {} — {}",
                spec.name,
                spec.glyphs.len(),
                spec.glyphs.keys().collect::<String>(),
                f.display(),
                a.credit()
            ));
        }
        for k in &res.skipped {
            s.push_str(&format!("\nSkipped {k}"));
        }
        Ok(s)
    }
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tests::{Mock, cfg};

    const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../acidtrip-io/tests/corpus");

    /// Three 5x5 block "letters" at x=10,16,22 rows 2..7 (1-col gaps), plus
    /// three rows of prose and some noise.
    fn synthetic() -> SourceArt {
        let mut g = Grid::new(80, 30);
        for (i, &(fg, bg)) in [(12u8, 4u8), (14, 6), (11, 3)].iter().enumerate() {
            let x0 = 10 + i * 6;
            for y in 2..7 {
                for x in x0..x0 + 5 {
                    let edge = y == 2 || y == 6 || x == x0 || x == x0 + 4;
                    let ch = if edge {
                        '█'
                    } else if i == 1 {
                        '▓'
                    } else {
                        '▄'
                    };
                    g.set(x, y, Cell::new(ch, Color::Pal(fg), Color::Pal(if edge { 0 } else { bg })));
                }
            }
        }
        for (dy, line) in
            ["this is a line of plain text here", "and another line of plain prose", "the third line of boring words"]
                .iter()
                .enumerate()
        {
            for (dx, ch) in line.chars().enumerate() {
                g.set(5 + dx, 15 + dy, Cell::new(ch, Color::LIGHT_GRAY, Color::BLACK));
            }
        }
        g.set(70, 25, Cell::new('*', Color::WHITE, Color::BLACK));
        let mut doc = Document::from_grid(DocKind::Classic, &g);
        doc.meta.sauce.author = "Tester".into();
        doc.meta.sauce.group = "ACiD".into();
        SourceArt {
            doc,
            attribution: Attribution {
                author: "Tester".into(),
                group: "ACiD".into(),
                file: "TS-LOGO.ANS".into(),
                pack: "acid-99".into(),
                ..Default::default()
            },
        }
    }

    fn abc() -> LetterReading {
        LetterReading {
            text: "ABC".into(),
            style: "blocky".into(),
            letters: vec![
                LetterSpan { ch: 'A', x0: 0, x1: 4 },
                LetterSpan { ch: 'B', x0: 6, x1: 10 },
                LetterSpan { ch: 'C', x0: 12, x1: 16 },
            ],
        }
    }

    #[test]
    fn segmentation_bridges_one_column_gaps() {
        let art = synthetic();
        let blobs = segment(&art.doc.flatten());
        let logo = blobs.iter().find(|b| b.x == 10 && b.y == 2).expect("logo blob");
        assert_eq!((logo.w, logo.h, logo.cells.len()), (17, 5, 75));
        assert!(blobs.iter().any(|b| (b.x, b.y, b.w, b.h) == (70, 25, 1, 1)));
    }

    #[test]
    fn logo_outscores_text() {
        let art = synthetic();
        let c = candidates(&art);
        assert!(!c.is_empty());
        let top = &c[0];
        assert_eq!((top.x, top.y, top.w, top.h), (10, 2, 17, 5));
        assert_eq!(top.id, "TS-LOGO.ANS@10,2");
        assert!(top.score > 0.5, "{}", top.score);
        let text_score = c.iter().find(|c| c.y == 15).map_or(0.0, |c| c.score);
        assert!(text_score < 0.25, "prose scored {text_score}");
        assert_eq!(top.clip.get(5, 0), None, "gap column is transparent");
        assert_eq!(top.clip.get(0, 0).unwrap().fg, Color::Pal(12));
        let png = render_candidate(top, 2);
        assert!(png.starts_with(b"\x89PNG"));
    }

    #[test]
    fn reading_parse_and_validate() {
        let r = parse_reading("Sure!\n```json\n{\"text\":\"AB\",\"letters\":[{\"char\":\"B\",\"x0\":5,\"x1\":99},{\"char\":\"A\",\"x0\":0,\"x1\":6},{\"char\":\" \",\"x0\":1,\"x1\":2},{\"char\":\"Z\",\"x0\":40,\"x1\":41}],\"style\":\"Chrome\"}\n```", 17).unwrap();
        assert_eq!(r.style, "chrome");
        assert_eq!(r.letters, vec![LetterSpan { ch: 'A', x0: 0, x1: 6 }, LetterSpan { ch: 'B', x0: 7, x1: 16 }]);
        assert!(parse_reading("no json", 5).is_err());
    }

    #[test]
    fn build_cuts_glyphs_and_stencils() {
        let art = synthetic();
        let top = candidates(&art).remove(0);
        let res = build(&[(top.clone(), abc())]);
        assert_eq!(res.stencils.len(), 1);
        let st = &res.stencils[0].meta;
        assert_eq!((st.name.as_str(), st.author.as_str(), st.license.as_str()), ("ABC", "Tester", LICENSE));
        assert!(st.tags.contains(&"blocky".to_string()));
        assert_eq!(res.fonts.len(), 1);
        let (spec, a) = &res.fonts[0];
        assert_eq!(a.author, "Tester");
        assert_eq!(spec.name, "Tester blocky", "the full name, no 12-char limit");
        assert_eq!(spec.glyphs.len(), 3);
        let b = &spec.glyphs[&'B'];
        assert_eq!((b.width, b.height), (5, 5));
        assert_eq!(b.get(2, 2).unwrap().ch, '▓');

        // Round trip through the .acidfont format, colors intact.
        let bytes = serde_json::to_vec(&spec.to_cut()).unwrap();
        let f = CutFont::from_bytes(&bytes).unwrap();
        assert_eq!(f.name, "Tester blocky");
        assert_eq!(f.glyphs.len(), 3);
        assert_eq!(f.glyphs[&'A'].clip, spec.glyphs[&'A']);
        assert_eq!(f.render("AB", 0).height, 5);
    }

    #[test]
    fn any_letter_goes_in_but_blanks_are_skipped() {
        let art = synthetic();
        let mut top = candidates(&art).remove(0);
        top.attribution.author.clear();
        let rd = LetterReading {
            text: "é ".into(),
            style: String::new(),
            letters: vec![LetterSpan { ch: 'é', x0: 0, x1: 4 }, LetterSpan { ch: '\u{7}', x0: 6, x1: 10 }],
        };
        let res = build(&[(top, rd)]);
        assert_eq!(res.fonts[0].0.glyphs.keys().collect::<String>(), "é");
        assert_eq!(res.skipped.len(), 1);
        assert_eq!(res.stencils[0].meta.group, "ACiD");
    }

    #[test]
    fn write_merges_fonts_and_saves_stencils() {
        let dir = tempfile::tempdir().unwrap();
        let (fonts_dir, st_dir) = (dir.path().join("fonts"), dir.path().join("stencils"));
        std::fs::create_dir_all(&st_dir).unwrap();
        let mut lib = StencilLibrary::load(&st_dir);
        let top = candidates(&synthetic()).remove(0);
        let mut first = abc();
        first.letters.truncate(1);
        let rep = write(&build(&[(top.clone(), first)]), &fonts_dir, &mut lib, &st_dir).unwrap();
        assert_eq!(rep.font_files.len(), 1);
        let mut second = abc();
        second.letters.remove(0);
        let rep2 = write(&build(&[(top, second)]), &fonts_dir, &mut lib, &st_dir).unwrap();
        assert_eq!(rep.font_files, rep2.font_files);
        let f = CutFont::from_bytes(&std::fs::read(&rep.font_files[0]).unwrap()).unwrap();
        assert_eq!(f.glyphs.keys().collect::<String>(), "ABCabc", "second write merges, other case filled in");
        let file: Value = serde_json::from_slice(&std::fs::read(&rep.font_files[0]).unwrap()).unwrap();
        assert_eq!(file["attribution"][0]["author"], "Tester");
        assert!(lib.list().len() >= 2);
    }

    #[test]
    fn letter_reading_via_model() {
        let top = candidates(&synthetic()).remove(0);
        let mock = Mock::new(vec![
            json!({ "content": [{ "type": "text", "text": "{\"text\":\"ABC\",\"letters\":[{\"char\":\"A\",\"x0\":0,\"x1\":4},{\"char\":\"B\",\"x0\":6,\"x1\":10},{\"char\":\"C\",\"x0\":12,\"x1\":16}],\"style\":\"blocky\"}" }], "stop_reason": "end_turn" }),
        ]);
        let r = read_letters_with(&mock, &cfg(), &top).unwrap();
        assert_eq!(r, abc());
        let req = &mock.requests.lock().unwrap()[0];
        assert!(req["messages"][0]["content"][1]["text"].as_str().unwrap().contains("17 columns x 5 rows"));
    }

    #[test]
    fn completion_draws_missing_glyphs() {
        let top = candidates(&synthetic()).remove(0);
        let spec = build(&[(top, abc())]).fonts.remove(0).0;
        let (_, _, boxes) = completion_layout(&spec, &['D']);
        let b = boxes[0];
        let cells: Vec<Value> = (0..5).map(|y| json!({ "x": b.x + 1, "y": b.y + y, "ch": "█", "fg": 10 })).collect();
        let mock = Mock::new(vec![
            json!({ "content": [{ "type": "tool_use", "id": "t1", "name": "set_cells", "input": { "cells": cells } }], "stop_reason": "tool_use" }),
            json!({ "content": [{ "type": "text", "text": "drew D" }], "stop_reason": "end_turn" }),
        ]);
        let out = complete_font_with(Box::new(mock.clone()), &cfg(), &spec, Some("ABCD")).unwrap();
        assert!(out.generated.contains(&'D') && out.generated.len() == 1);
        let d = &out.glyphs[&'D'];
        assert_eq!((d.width, d.height), (1, 5));
        assert!(
            mock.requests.lock().unwrap()[0]["messages"][0]["content"][1]["text"]
                .as_str()
                .unwrap()
                .contains("'D' at x=")
        );
    }

    #[test]
    fn local_dir_and_zip_sources() {
        let dir = tempfile::tempdir().unwrap();
        let doc = synthetic().doc;
        let ans = format::save_bytes(&doc, Format::Ansi, &Default::default()).unwrap();
        std::fs::write(dir.path().join("ts-logo.ans"), &ans).unwrap();
        let mut zbuf = Cursor::new(vec![]);
        {
            let mut z = zip::ZipWriter::new(&mut zbuf);
            z.start_file(
                "sub/in-zip.ans",
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            std::io::Write::write_all(&mut z, &ans).unwrap();
            z.start_file(
                "readme.txt",
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            z.finish().unwrap();
        }
        std::fs::write(dir.path().join("pack-01.zip"), zbuf.into_inner()).unwrap();
        let arts = fetch(dir.path().to_str().unwrap(), &dir.path().join("cache")).unwrap();
        assert_eq!(arts.len(), 2);
        let z = arts.iter().find(|a| a.attribution.pack == "pack-01").expect("zip entry");
        assert_eq!(z.attribution.file, "in-zip.ans");
        assert!(arts.iter().all(|a| { candidates(a).first().is_some_and(|c| (c.x, c.y, c.w) == (10, 2, 17)) }));
    }

    #[test]
    fn corpus_candidates_are_sane() {
        let Ok(rd) = std::fs::read_dir(CORPUS) else {
            return eprintln!("corpus not present, skipping");
        };
        let mut total = 0;
        for e in rd.flatten() {
            let p = e.path();
            let name = p.to_string_lossy().to_string();
            if art_format(&name).is_none() {
                continue;
            }
            let art = parse_art(&name, &std::fs::read(&p).unwrap(), Attribution::default()).unwrap();
            for c in candidates(&art) {
                total += 1;
                assert!((12..=80).contains(&c.w) && (3..=20).contains(&c.h), "{} {}x{}", c.id, c.w, c.h);
                assert!(c.score > 0.2 && c.score <= 1.0);
                assert_eq!(c.clip.cells.len(), c.w * c.h);
            }
        }
        assert!(total > 0, "expected some logo candidates in the icy_draw docs");
    }

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/16colors");

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURES}/{name}")).unwrap()
    }

    #[test]
    fn parses_16colors_year_index() {
        let years = parse_years(&fixture("years.json")).unwrap();
        assert_eq!(years.first().map(|y| y.year), Some(1990));
        let y97 = years.iter().find(|y| y.year == 1997).unwrap();
        assert_eq!(y97.packs, 808);
        assert!(years.windows(2).all(|w| w[0].year < w[1].year));
    }

    #[test]
    fn parses_16colors_pack_page() {
        let (packs, pages) = parse_pack_page(&fixture("year-1997-page.json")).unwrap();
        assert_eq!(pages, 162);
        assert_eq!(packs.len(), 5);
        assert_eq!(packs[0], PackInfo { name: "01ninja".into(), year: 1997, groups: vec!["voff".into()] });
        assert_eq!(packs[1].name, "14", "numeric pack names become text");
        assert_eq!(packs[4].groups, vec!["303".to_string()]);
    }

    #[test]
    fn parses_16colors_pack_detail() {
        let d = parse_pack_detail("twi-9703", &fixture("pack-twi-9703.json")).unwrap();
        assert_eq!(d.year, 1997);
        assert_eq!(d.download, "https://16colo.rs/archive/1997/twi-9703.zip");
        let abx = d.files.iter().find(|f| f.name == "STY-ABX.ANS").unwrap();
        assert_eq!(abx.artists, vec!["stygian".to_string()]);
        assert_eq!(abx.content[0], "abraxas");
        assert!(d.art_files() >= 20 && d.art_files() < d.files.len(), "{}", d.art_files());
        assert!(d.artists().contains(&"coug".to_string()));
        assert!(parse_pack_detail("nope", b"{\"results\":[]}").is_err());
    }

    /// Letters "A", "B", "C" of the synthetic logo, touching (no gap).
    fn touching() -> Clip {
        let mut c = candidates(&synthetic()).remove(0).clip;
        // Close the 1-column gaps at x=5 and x=11 on the middle row only.
        for x in [5, 11] {
            c.set(x, 2, Some(Cell::new('▄', Color::Pal(8), Color::BLACK)));
        }
        c
    }

    #[test]
    fn auto_splits_find_gaps_and_valleys() {
        let top = candidates(&synthetic()).remove(0);
        assert_eq!(gap_splits(&top.clip), vec![5, 11]);
        assert_eq!(auto_splits(&top.clip, 0), vec![5, 11]);
        let s = auto_splits(&top.clip, 3);
        assert!(s[0] == 5 || s[0] == 6, "{s:?}");
        assert!(s[1] == 11 || s[1] == 12, "{s:?}");
        // Touching letters: cut through the thin bridge, not a letter body.
        let t = auto_splits(&touching(), 3);
        assert!((5..=6).contains(&t[0]) && (11..=12).contains(&t[1]), "{t:?}");
        // More letters than gaps: still n - 1 increasing splits.
        let m = auto_splits(&top.clip, 5);
        assert_eq!(m.len(), 4);
        assert!(m.windows(2).all(|w| w[0] < w[1]) && m[0] > 0 && m[3] < top.w);
        assert_eq!(auto_splits(&top.clip, 1), Vec::<usize>::new());
        assert_eq!(even_splits(20, 4), vec![5, 10, 15]);
        assert_eq!(auto_splits(&Clip::new(9, 2), 3), vec![3, 6], "blank clip splits evenly");
    }

    #[test]
    fn splits_round_trip_to_readings() {
        let top = candidates(&synthetic()).remove(0);
        assert_eq!(splits_from_reading(&abc(), top.w), vec![6, 12]);
        let r = reading_from_splits("ABC", &[6, 12], top.w, "Blocky ");
        assert_eq!(r.style, "blocky");
        assert_eq!(
            r.letters,
            vec![
                LetterSpan { ch: 'A', x0: 0, x1: 5 },
                LetterSpan { ch: 'B', x0: 6, x1: 11 },
                LetterSpan { ch: 'C', x0: 12, x1: 16 }
            ]
        );
        // A space skips a segment (a decoration between letters).
        let r = reading_from_splits("A C", &[6, 12], top.w, "");
        assert_eq!(r.letters.iter().map(|l| l.ch).collect::<String>(), "AC");
        let res = build(&[(top, reading_from_splits("ABC", &[6, 12], 17, "blocky"))]);
        let spec = &res.fonts[0].0;
        assert_eq!(spec.glyphs[&'B'].width, 5, "trimmed to the letter's ink");
    }

    #[test]
    fn writing_fills_the_other_case() {
        let dir = tempfile::tempdir().unwrap();
        let (fonts_dir, st_dir) = (dir.path().join("fonts"), dir.path().join("stencils"));
        std::fs::create_dir_all(&st_dir).unwrap();
        let mut lib = StencilLibrary::load(&st_dir);
        let top = candidates(&synthetic()).remove(0);
        let res = build(&[(top, reading_from_splits("aBc", &[6, 12], 17, ""))]);
        assert_eq!(font_file_name(&res.fonts[0].1, ""), "tester-logo.acidfont");
        let rep = write(&res, &fonts_dir, &mut lib, &st_dir).unwrap();
        let f = HarvestedFont::load(&rep.font_files[0]).unwrap();
        assert_eq!(f.spec.glyphs.keys().collect::<String>(), "ABCabc");
    }

    #[test]
    fn a_recut_letter_replaces_the_old_one_but_case_copies_never_do() {
        let dir = tempfile::tempdir().unwrap();
        let (fonts_dir, st_dir) = (dir.path().join("fonts"), dir.path().join("stencils"));
        std::fs::create_dir_all(&st_dir).unwrap();
        let mut lib = StencilLibrary::load(&st_dir);
        let top = candidates(&synthetic()).remove(0);
        // "AB" first; A is the 5-wide block at 0..4.
        write(&build(&[(top.clone(), reading_from_splits("AB", &[6], 12, ""))]), &fonts_dir, &mut lib, &st_dir)
            .unwrap();
        let file = HarvestedFont::dir(&fonts_dir).join("tester-logo.acidfont");
        let before = HarvestedFont::load(&file).unwrap();
        assert_eq!(before.spec.glyphs[&'a'], before.spec.glyphs[&'A'], "a mirrors A");
        // Recut: the whole top as "a" (wider). a changes; A (a real cut) stays.
        write(&build(&[(top, reading_from_splits("a", &[], 17, ""))]), &fonts_dir, &mut lib, &st_dir).unwrap();
        let after = HarvestedFont::load(&file).unwrap();
        assert!(after.spec.glyphs[&'a'].width > before.spec.glyphs[&'a'].width);
        assert_eq!(after.spec.glyphs[&'A'], before.spec.glyphs[&'A']);
        assert_eq!(after.attribution.len(), 1, "same artist credited once");
    }

    #[test]
    fn harvested_fonts_list_edit_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let (fonts_dir, st_dir) = (dir.path().join("fonts"), dir.path().join("stencils"));
        std::fs::create_dir_all(&st_dir).unwrap();
        let mut lib = StencilLibrary::load(&st_dir);
        let top = candidates(&synthetic()).remove(0);
        let mut res = build(&[(top, abc())]);
        res.fonts[0].0.generated.insert('C');
        write(&res, &fonts_dir, &mut lib, &st_dir).unwrap();
        let all = HarvestedFont::list(&fonts_dir);
        assert_eq!(all.len(), 1);
        let mut f = all[0].clone();
        assert_eq!(f.artists(), ["Tester"]);
        assert_eq!(f.spec.generated.iter().collect::<String>(), "Cc", "the copy of a drawn letter is drawn too");
        assert_eq!(f.missing("ABCD"), ['D']);
        f.remove('B');
        f.save().unwrap();
        let back = HarvestedFont::load(&f.file).unwrap();
        assert_eq!(back.spec.glyphs.keys().collect::<String>(), "ACabc");
        assert_eq!(back, f);
        back.delete().unwrap();
        assert!(HarvestedFont::list(&fonts_dir).is_empty());
        assert!(!f.file.exists());
    }

    #[test]
    fn fonts_harvested_as_thedraw_move_to_acidfont_on_save() {
        use retrofont::tdf::TdfFontType;
        let dir = tempfile::tempdir().unwrap();
        let (fonts_dir, st_dir) = (dir.path().join("fonts"), dir.path().join("stencils"));
        std::fs::create_dir_all(&st_dir).unwrap();
        let hdir = HarvestedFont::dir(&fonts_dir);
        std::fs::create_dir_all(&hdir).unwrap();
        let mut old = TdfFont::new("Tester logo".to_string(), TdfFontType::Color, 1);
        let z = GlyphPart::AnsiChar { ch: '█', fg: 12, bg: 0, blink: false };
        old.add_glyph('Z', Glyph { width: 1, height: 1, parts: vec![z] });
        let tdf = hdir.join("tester-logo.tdf");
        std::fs::write(&tdf, TdfFont::serialize_bundle(&[old]).unwrap()).unwrap();
        let side = json!({"name": "Tester logo", "style": "logo", "attribution": [{"author": "Tester"}]});
        std::fs::write(tdf.with_extension("tdf.json"), side.to_string()).unwrap();
        let listed = HarvestedFont::list(&fonts_dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].artists(), ["Tester"]);
        // Cutting more letters into it merges them and moves the file.
        let mut lib = StencilLibrary::load(&st_dir);
        let top = candidates(&synthetic()).remove(0);
        let rep = write(&build(&[(top, reading_from_splits("AB", &[6], 12, ""))]), &fonts_dir, &mut lib, &st_dir)
            .unwrap();
        assert_eq!(rep.font_files[0], hdir.join("tester-logo.acidfont"));
        assert!(!tdf.exists() && !tdf.with_extension("tdf.json").exists());
        let f = HarvestedFont::load(&rep.font_files[0]).unwrap();
        assert_eq!(f.spec.glyphs.keys().collect::<String>(), "ABZabz");
        let lib = FontLibrary::load(Some(&fonts_dir));
        assert_eq!(lib.list().iter().filter(|i| i.path.as_deref() == Some(rep.font_files[0].as_path())).count(), 1);
    }

    #[test]
    #[ignore = "network: downloads a pack from 16colo.rs"]
    fn fetch_16colors_pack() {
        let dir = tempfile::tempdir().unwrap();
        let arts = fetch("16colo.rs:twi-9703", dir.path()).unwrap();
        assert!(arts.len() > 3, "{}", arts.len());
        assert!(arts.iter().any(|a| a.attribution.author == "Coug"));
        let one = fetch("https://16colo.rs/pack/twi-9703/CG-MALP.ANS", dir.path()).unwrap();
        assert_eq!(one[0].attribution.title, "Malpractice");
        let n: usize = arts.iter().map(|a| candidates(a).len()).sum();
        eprintln!("{} files, {n} candidates", arts.len());
    }

    #[test]
    #[ignore = "calls the real Claude API; needs ANTHROPIC_API_KEY"]
    fn live_letter_reading() {
        let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY");
        let top = candidates(&synthetic()).remove(0);
        let r =
            read_letters(&AgentConfig { api_key: key, model: "claude-sonnet-5".into(), max_rounds: 1 }, &top).unwrap();
        eprintln!("{r:?}");
    }
}
