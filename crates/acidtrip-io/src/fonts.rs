//! Text fonts: TheDraw .tdf bundles and FIGlet .flf via `retrofont`, and
//! acidtrip's own `.acidfont` (letters cut from art, any size and shape).
//! Bundled FIGlet fonts ship inside the binary; user fonts live in
//! `Paths::fonts_dir()`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use acidtrip_core::{Cell, Clip, Color};
use anyhow::{Context, bail};
use retrofont::tdf::TdfFontType;
use retrofont::{Font, FontError, FontTarget, RenderMode, RenderOptions};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontKind {
    TdfBlock,
    TdfColor,
    TdfOutline,
    Figlet,
    /// An `.acidfont`: letters cut from art.
    Cut,
}

/// acidtrip's own font format (`.acidfont`, JSON). No format limits: a
/// glyph is any clip, cells outside the letter's shape are transparent,
/// and each glyph says which of its rows sits on the baseline so letters
/// of different heights (descenders, overhangs) line up. Other fields in
/// the file (credits, style) are ignored here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CutFont {
    pub name: String,
    /// Columns between letters (negative lets shapes interlock).
    #[serde(default)]
    pub spacing: i32,
    pub glyphs: BTreeMap<char, CutGlyph>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutGlyph {
    pub clip: Clip,
    /// The row on the baseline; None = the last row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<usize>,
}

impl CutGlyph {
    pub fn new(clip: Clip) -> Self {
        CutGlyph { clip, base: None }
    }

    pub fn base(&self) -> usize {
        self.base.unwrap_or(self.clip.height.saturating_sub(1)).min(self.clip.height.saturating_sub(1))
    }
}

pub const CUT_FONT_EXT: &str = "acidfont";

impl CutFont {
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<CutFont> {
        let f: CutFont = serde_json::from_slice(bytes).context("not an .acidfont")?;
        if f.glyphs.is_empty() {
            bail!("font has no letters");
        }
        Ok(f)
    }

    /// The glyph for `ch`, or the other case's.
    pub fn glyph(&self, ch: char) -> Option<&CutGlyph> {
        self.glyphs.get(&ch).or_else(|| {
            let other = if ch.is_lowercase() { ch.to_uppercase().next() } else { ch.to_lowercase().next() };
            other.and_then(|o| self.glyphs.get(&o))
        })
    }

    /// Render text: glyphs sit on a shared baseline, every line is as tall
    /// as the font's tallest ascent plus deepest descent.
    pub fn render(&self, text: &str, extra_spacing: i32) -> Clip {
        let gap = (self.spacing + extra_spacing) as i64;
        let (mut up, mut down) = (1usize, 0usize);
        let mut widths = 0;
        for g in self.glyphs.values() {
            up = up.max(g.base() + 1);
            down = down.max(g.clip.height.saturating_sub(g.base() + 1));
            widths += g.clip.width;
        }
        let line_h = up + down;
        let space = (widths / self.glyphs.len().max(1) / 2).max(1) as i64;
        let mut cells: HashMap<(usize, usize), Cell> = HashMap::new();
        let lines: Vec<&str> = text.lines().collect();
        let mut width = 0usize;
        for (li, line) in lines.iter().enumerate() {
            let mut x: i64 = 0;
            for ch in line.chars() {
                let start = x.max(0);
                match self.glyph(ch).filter(|_| ch != ' ') {
                    Some(g) => {
                        let top = li * line_h + up - (g.base() + 1);
                        for gy in 0..g.clip.height {
                            for gx in 0..g.clip.width {
                                if let Some(c) = g.clip.get(gx, gy) {
                                    cells.insert((start as usize + gx, top + gy), c);
                                }
                            }
                        }
                        width = width.max(start as usize + g.clip.width);
                        x = start + g.clip.width as i64 + gap;
                    }
                    None if ch == ' ' => x = start + space + gap.max(0),
                    // A letter the font lacks is left out (the Font dialog
                    // says so), not a letter-wide hole that can push the
                    // rest out of view.
                    None => {}
                }
            }
        }
        let mut clip = Clip::new(width, lines.len() * line_h);
        for ((x, y), c) in cells {
            clip.set(x, y, Some(c));
        }
        clip
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontInfo {
    /// Unique id: "<file stem>" or "<file stem>#<index>" for bundles.
    pub id: String,
    pub name: String,
    pub kind: FontKind,
    /// None = bundled in the binary.
    pub path: Option<PathBuf>,
    /// Index inside a TDF bundle.
    pub index: usize,
    /// Chars that have glyphs.
    pub charset: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRenderOptions {
    /// TDF outline style 0..19.
    pub outline_style: usize,
    /// Colors for Block/Outline/FIGlet fonts (Color fonts carry their own).
    pub fg: Color,
    pub bg: Color,
    /// Extra columns between letters (can be negative for tight fonts).
    pub spacing: i32,
}

impl Default for TextRenderOptions {
    fn default() -> Self {
        TextRenderOptions { outline_style: 0, fg: Color::WHITE, bg: Color::BLACK, spacing: 0 }
    }
}

#[derive(Default)]
pub struct FontLibrary {
    entries: Vec<Entry>,
    /// The folder it was loaded from and what its font files looked like
    /// then, to notice fonts added, changed or removed since.
    source: Option<(PathBuf, u64)>,
}

struct Entry {
    info: FontInfo,
    face: Face,
}

enum Face {
    Retro(Box<Font>),
    Cut(CutFont),
}

macro_rules! bundled {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/figlet/", $name, ".flf")))),*]
    };
}

/// The FIGlet standard font set (BSD-3, see `assets/figlet/LICENSE`).
const BUNDLED: &[(&str, &[u8])] = bundled![
    "standard", "banner", "big", "block", "bubble", "digital", "lean", "mini", "script", "shadow", "slant", "small",
    "smscript", "smshadow", "smslant", "term",
];

/// License of the bundled FIGlet fonts.
pub const BUNDLED_LICENSE: &str = include_str!("../assets/figlet/LICENSE");

const FONT_EXTS: [&str; 4] = ["tdf", "flf", "tlf", CUT_FONT_EXT];

impl FontLibrary {
    /// Bundled fonts + everything under `dir` (recursively; .tdf, .flf, .zip).
    pub fn load(dir: Option<&Path>) -> FontLibrary {
        let mut lib = FontLibrary::default();
        for (stem, bytes) in BUNDLED {
            lib.add(stem, None, bytes);
        }
        if let Some(dir) = dir {
            let mut files = Vec::new();
            walk(dir, &mut files);
            files.sort();
            lib.source = Some((dir.to_path_buf(), fingerprint(&files)));
            for f in files {
                lib.add_file(&f);
            }
        }
        lib
    }

    /// Whether the folder's fonts changed since loading (a font cut in the
    /// studio, Claude's letters saved after it closed, a file copied in).
    pub fn changed_on_disk(&self) -> bool {
        let Some((dir, was)) = &self.source else { return false };
        let mut files = Vec::new();
        walk(dir, &mut files);
        files.sort();
        fingerprint(&files) != *was
    }

    /// Load again from the same folder.
    pub fn reload(&self) -> FontLibrary {
        FontLibrary::load(self.source.as_ref().map(|(d, _)| d.as_path()))
    }

    fn add_file(&mut self, path: &Path) {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        if ext_of(path).as_deref() == Some("zip") {
            let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
                return;
            };
            for i in 0..zip.len() {
                let Ok(mut f) = zip.by_index(i) else { continue };
                let name = Path::new(f.name()).to_path_buf();
                if !f.is_file() || !ext_of(&name).is_some_and(|e| FONT_EXTS.contains(&e.as_str())) {
                    continue;
                }
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_ok() {
                    self.add(&stem_of(&name), Some(path), &buf);
                }
            }
        } else {
            self.add(&stem_of(path), Some(path), &bytes);
        }
    }

    /// Parse and index the fonts in one file's bytes; returns how many were added.
    fn add(&mut self, stem: &str, path: Option<&Path>, bytes: &[u8]) -> usize {
        if bytes.first() == Some(&b'{') {
            let Ok(font) = CutFont::from_bytes(bytes) else { return 0 };
            let name = if font.name.trim().is_empty() { stem.to_string() } else { font.name.trim().to_string() };
            let charset = font.glyphs.keys().filter(|c| !c.is_control()).collect();
            let info =
                FontInfo { id: self.unique_id(stem), name, kind: FontKind::Cut, path: path.map(Path::to_path_buf), index: 0, charset };
            self.entries.push(Entry { info, face: Face::Cut(font) });
            return 1;
        }
        let Ok(fonts) = parse(bytes) else { return 0 };
        let multi = fonts.len() > 1;
        let mut n = 0;
        for (index, font) in fonts.into_iter().enumerate() {
            let base = if multi { format!("{stem}#{index}") } else { stem.to_string() };
            let id = self.unique_id(&base);
            let name = match &font {
                // Harvested fonts keep their full name next to the file (TheDraw holds 12 chars).
                Font::Tdf(_) if let Some(n) = path.and_then(sidecar_name).filter(|_| !multi) => n,
                Font::Tdf(f) if !f.name.trim().is_empty() => f.name.trim().to_string(),
                _ => stem.to_string(),
            };
            let kind = match &font {
                Font::Figlet(_) => FontKind::Figlet,
                Font::Tdf(f) => match f.font_type {
                    TdfFontType::Block => FontKind::TdfBlock,
                    TdfFontType::Color => FontKind::TdfColor,
                    TdfFontType::Outline => FontKind::TdfOutline,
                },
            };
            let charset = (' '..='\u{ff}').filter(|&c| !c.is_control() && font.has_char(c)).collect();
            let info = FontInfo { id, name, kind, path: path.map(Path::to_path_buf), index, charset };
            self.entries.push(Entry { info, face: Face::Retro(Box::new(font)) });
            n += 1;
        }
        n
    }

    fn unique_id(&self, base: &str) -> String {
        let mut id = base.to_string();
        let mut k = 2;
        while self.entries.iter().any(|e| e.info.id == id) {
            id = format!("{base}~{k}");
            k += 1;
        }
        id
    }

    pub fn list(&self) -> Vec<FontInfo> {
        self.entries.iter().map(|e| e.info.clone()).collect()
    }

    fn entry(&self, id_or_name: &str) -> Option<&Entry> {
        let q = id_or_name.trim();
        self.entries
            .iter()
            .find(|e| e.info.id == q)
            .or_else(|| self.entries.iter().find(|e| e.info.id.eq_ignore_ascii_case(q)))
            .or_else(|| self.entries.iter().find(|e| e.info.name.eq_ignore_ascii_case(q)))
    }

    pub fn find(&self, id_or_name: &str) -> Option<FontInfo> {
        self.entry(id_or_name).map(|e| e.info.clone())
    }

    /// Render text (multi-line allowed) into a transparent clip.
    pub fn render(&self, id: &str, text: &str, opts: &TextRenderOptions) -> anyhow::Result<Clip> {
        let e = self.entry(id).with_context(|| format!("unknown font {id:?}"))?;
        Ok(match &e.face {
            Face::Retro(font) => render_font(font, e.info.kind, text, opts),
            Face::Cut(font) => font.render(text, opts.spacing),
        })
    }

    /// Import a font file into the user library dir (copy) and reload.
    pub fn install(&mut self, src: &Path, fonts_dir: &Path) -> anyhow::Result<Vec<FontInfo>> {
        let bytes = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
        let mut probe = FontLibrary::default();
        let name = src.file_name().context("font path has no file name")?;
        if ext_of(src).as_deref() == Some("zip") {
            probe.add_file(src);
        } else {
            probe.add(&stem_of(src), None, &bytes);
        }
        if probe.entries.is_empty() {
            bail!("{} contains no TheDraw, FIGlet or acidtrip fonts", src.display());
        }
        std::fs::create_dir_all(fonts_dir)?;
        let dest = fonts_dir.join(name);
        crate::library::write_atomic(&dest, &bytes)?;
        *self = FontLibrary::load(Some(fonts_dir));
        Ok(self
            .entries
            .iter()
            .filter(|e| e.info.path.as_deref() == Some(dest.as_path()))
            .map(|e| e.info.clone())
            .collect())
    }
}

/// Parse font bytes, tolerating Latin-1 FIGlet files and TOIlet `.tlf` headers.
fn parse(bytes: &[u8]) -> Result<Vec<Font>, FontError> {
    let mut b = bytes.to_vec();
    if b.starts_with(b"tlf2a") {
        b[0] = b'f';
    }
    if b.starts_with(b"flf2a") {
        let text = match String::from_utf8(b) {
            Ok(t) => t,
            // FIGlet's default encoding is Latin-1.
            Err(e) => e.into_bytes().iter().map(|&c| c as char).collect(),
        };
        b = normalize_endmarks(&text).unwrap_or(text).into_bytes();
    }
    Font::load_owned(b)
}

/// retrofont only accepts '@' endmarks; FIGlet allows any char (e.g. term.flf
/// uses '#' for the '@' glyph). Rewrite the endmarks of the 95 required glyphs.
fn normalize_endmarks(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let parts: Vec<&str> = header.split_whitespace().collect();
    let height: usize = parts.get(1)?.parse().ok()?;
    let comments: usize = parts.get(5)?.parse().ok()?;
    let mut out = String::with_capacity(text.len());
    out.push_str(header);
    out.push('\n');
    for l in lines.by_ref().take(comments) {
        out.push_str(l);
        out.push('\n');
    }
    for _ in 32..=126 {
        for row in 0..height {
            let line = lines.next()?.trim_end();
            let mut body = line.chars();
            let mark = body.next_back()?;
            let mut body = body.as_str();
            if row + 1 == height {
                body = body.strip_suffix(mark).unwrap_or(body);
            }
            out.push_str(body);
            out.push_str(if row + 1 == height { "@@\n" } else { "@\n" });
        }
    }
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}

/// Names, sizes and modification times of `files`, hashed.
fn fingerprint(files: &[PathBuf]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in files {
        f.hash(&mut h);
        if let Ok(m) = std::fs::metadata(f) {
            m.len().hash(&mut h);
            m.modified().ok().hash(&mut h);
        }
    }
    h.finish()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if ext_of(&p).is_some_and(|x| x == "zip" || FONT_EXTS.contains(&x.as_str())) {
            out.push(p);
        }
    }
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
}

fn stem_of(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("font").to_string()
}

/// Collects drawn cells at absolute positions; blanks stay transparent.
struct Stamp<'a> {
    cells: HashMap<(usize, usize), Cell>,
    ox: usize,
    x: usize,
    y: usize,
    max_x: usize,
    color_font: bool,
    opts: &'a TextRenderOptions,
}

impl FontTarget for Stamp<'_> {
    type Error = ();

    fn draw(&mut self, c: retrofont::Cell) -> Result<(), ()> {
        let blank = matches!(c.ch, ' ' | '\0' | '\u{a0}');
        let cell = match (c.fg, c.bg) {
            (Some(fg), Some(bg)) if self.color_font => {
                (!blank || bg != 0).then(|| Cell::new(if blank { ' ' } else { c.ch }, Color::Pal(fg), Color::Pal(bg)))
            }
            _ => (!blank).then(|| Cell::new(c.ch, self.opts.fg, self.opts.bg)),
        };
        if let Some(cell) = cell {
            self.cells.insert((self.x, self.y), cell);
        }
        self.x += 1;
        self.max_x = self.max_x.max(self.x);
        Ok(())
    }

    fn next_line(&mut self) -> Result<(), ()> {
        self.x = self.ox;
        self.y += 1;
        Ok(())
    }
}

fn render_font(font: &Font, kind: FontKind, text: &str, opts: &TextRenderOptions) -> Clip {
    let ro = RenderOptions { render_mode: RenderMode::Display, outline_style: opts.outline_style.min(18) };
    let (glyph_h, glyph_w_sum, glyph_n) = glyph_stats(font);
    let line_h = glyph_h.max(1);
    let letter_gap = match font {
        Font::Tdf(f) => f.spacing,
        Font::Figlet(_) => 0,
    } + opts.spacing;
    // TDF fonts rarely define ' '; retrofont draws `spacing` columns, which is
    // too narrow for wide letters, so keep at least half an average glyph.
    let min_space = match font {
        Font::Tdf(_) if !font.has_char(' ') => glyph_w_sum / glyph_n.max(1) / 2,
        _ => 1,
    };
    let mut st =
        Stamp { cells: HashMap::new(), ox: 0, x: 0, y: 0, max_x: 0, color_font: kind == FontKind::TdfColor, opts };
    let lines: Vec<&str> = text.lines().collect();
    let mut width = 0usize;
    for (li, line) in lines.iter().enumerate() {
        let mut x: i64 = 0;
        for ch in line.chars() {
            let start = x.max(0) as usize;
            (st.ox, st.x, st.y, st.max_x) = (start, start, li * line_h, start);
            match font.render_glyph(&mut st, ch, &ro) {
                Ok(()) => {
                    let mut w = st.max_x - start;
                    if ch == ' ' {
                        w = w.max(min_space);
                    }
                    width = width.max(start + w);
                    x = start as i64 + w as i64 + letter_gap as i64;
                }
                // Missing glyph: leave a one-column gap.
                Err(_) => x = start as i64 + 1,
            }
        }
    }
    let mut clip = Clip::new(width, lines.len() * line_h);
    for ((x, y), c) in st.cells {
        clip.set(x, y, Some(c));
    }
    clip
}

/// (max glyph height, sum of glyph widths, glyph count).
fn glyph_stats(font: &Font) -> (usize, usize, usize) {
    let mut seen = HashSet::new();
    let mut stats = (0, 0, 0);
    let mut acc = |g: &retrofont::Glyph| {
        stats.0 = stats.0.max(g.height);
        stats.1 += g.width;
        stats.2 += 1;
    };
    match font {
        Font::Tdf(f) => f.iter_glyphs().for_each(|(c, g)| {
            if seen.insert(c) {
                acc(g)
            }
        }),
        Font::Figlet(f) => f.iter_glyphs().for_each(|(_, g)| acc(g)),
    }
    stats
}

/// The tdfiglet commit the TheDraw font pack is pinned to, so the archive
/// never changes under us.
pub const TDF_PACK_COMMIT: &str = "0225b8881d06773b92d9db2c7881f03ff0bdece7";

/// SHA-256 of the pinned pack zip; a download that doesn't match is refused.
pub const TDF_PACK_SHA256: &str = "f01deb2f165e10dcc8b39b4c7fd81a23cac020dbdc4c5df905cd2c3361870737";

/// Where "Get more fonts" downloads from, tried in order: the tdfiglet
/// collection at the pinned commit, then the copy kept in acidtrip's own
/// repo (`fonts/tdf/`, see its NOTICE.md) in case that one goes away.
pub const TDF_PACK_URLS: &[&str] = &[
    "https://codeload.github.com/tat3r/tdfiglet/zip/0225b8881d06773b92d9db2c7881f03ff0bdece7",
    "https://raw.githubusercontent.com/jondot/acidtrip/main/fonts/tdf/tdfiglet-0225b8881d06773b92d9db2c7881f03ff0bdece7.zip",
];

/// Subdirectory of the fonts dir that downloaded packs land in.
pub const TDF_PACK_DIR: &str = "tdfiglet";

/// `"name"` from a font's `<file>.json` sidecar, if it has one.
fn sidecar_name(path: &Path) -> Option<String> {
    let mut side = path.as_os_str().to_owned();
    side.push(".json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(side).ok()?).ok()?;
    v["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(String::from)
}

/// Download TDF font packs into `fonts_dir` (network; used by "Get more fonts").
pub fn download_packs(fonts_dir: &Path) -> anyhow::Result<usize> {
    let bytes = fetch_pack(TDF_PACK_URLS, TDF_PACK_SHA256, |url| {
        let mut resp = ureq::get(url).call()?;
        Ok(resp.body_mut().with_config().limit(64 << 20).read_to_vec()?)
    })?;
    extract_pack(&bytes, &fonts_dir.join(TDF_PACK_DIR))
}

/// The first of `urls` whose download has the SHA-256 `sha256`. Every
/// failure is kept for the error, so a dead mirror doesn't hide why the
/// next one failed too.
fn fetch_pack(
    urls: &[&str],
    sha256: &str,
    mut fetch: impl FnMut(&str) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    let mut errors = Vec::new();
    for url in urls {
        match fetch(url) {
            Ok(bytes) => {
                let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                if got == sha256 {
                    return Ok(bytes);
                }
                errors.push(format!("{url}: checksum mismatch"));
            }
            Err(e) => errors.push(format!("{url}: {e:#}")),
        }
    }
    anyhow::bail!("no font pack source worked ({})", errors.join("; "))
}

/// Extract every parseable `.tdf` file of a pack zip into `dest` (flat).
/// Returns the number of files written.
pub fn extract_pack(zip_bytes: &[u8], dest: &Path) -> anyhow::Result<usize> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).context("font pack is not a zip")?;
    std::fs::create_dir_all(dest)?;
    let mut n = 0;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let Some(name) = f.enclosed_name() else {
            continue;
        };
        if !f.is_file() || ext_of(&name).as_deref() != Some("tdf") {
            continue;
        }
        let Some(file) = name.file_name() else {
            continue;
        };
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        if parse(&buf).is_ok() {
            crate::library::write_atomic(&dest.join(file), &buf)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_comes_from_the_first_source_with_the_right_checksum() {
        // SHA-256 of "good".
        let good = "770e607624d689265ca6c44884d0807d9b054d23c473c106c72be9de08b7376c";
        let mut tried = Vec::new();
        let got = fetch_pack(&["down", "tampered", "mirror", "unused"], good, |url| {
            tried.push(url.to_string());
            match url {
                "down" => anyhow::bail!("connection refused"),
                "tampered" => Ok(b"evil".to_vec()),
                _ => Ok(b"good".to_vec()),
            }
        })
        .unwrap();
        assert_eq!(got, b"good");
        assert_eq!(tried, ["down", "tampered", "mirror"]);

        let err = fetch_pack(&["down", "tampered"], good, |url| match url {
            "down" => anyhow::bail!("connection refused"),
            _ => Ok(b"evil".to_vec()),
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("down: connection refused") && err.contains("tampered: checksum mismatch"), "{err}");
    }

    #[test]
    fn the_sources_are_pinned_to_the_same_commit() {
        assert!(TDF_PACK_URLS.iter().all(|u| u.contains(TDF_PACK_COMMIT)));
    }

    #[test]
    fn the_backup_pack_in_the_repo_matches_the_checksum() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../fonts/tdf/tdfiglet-{TDF_PACK_COMMIT}.zip"));
        let bytes = std::fs::read(&path).unwrap();
        fetch_pack(&["repo"], TDF_PACK_SHA256, |_| Ok(bytes.clone())).unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(extract_pack(&bytes, dir.path()).unwrap() > 1000);
    }

    #[test]
    fn a_library_notices_fonts_added_after_loading() {
        let dir = tempfile::tempdir().unwrap();
        let lib = FontLibrary::load(Some(dir.path()));
        assert!(!lib.changed_on_disk());
        let (stem, bytes) = BUNDLED[0];
        std::fs::write(dir.path().join(format!("copy-of-{stem}.flf")), bytes).unwrap();
        assert!(lib.changed_on_disk());
        let again = lib.reload();
        assert_eq!(again.list().len(), lib.list().len() + 1);
        assert!(!again.changed_on_disk());
        assert!(!FontLibrary::load(None).changed_on_disk());
    }

    #[test]
    fn endmarks_are_normalized() {
        let mut src = String::from("flf2a$ 2 1 4 -1 1\ncomment\n");
        for c in 32u8..=126 {
            src.push_str(&format!("{}#\n{}##\n", c as char, c as char));
        }
        let out = normalize_endmarks(&src).unwrap();
        assert!(out.contains("\nA@\nA@@\n") && out.contains("\n#@\n#@@\n"));
        let fonts = parse(src.as_bytes()).unwrap();
        assert!(fonts[0].has_char('~'));
    }

    #[test]
    fn cut_fonts_share_a_baseline_and_keep_their_shapes() {
        let cell = |ch| Some(Cell::new(ch, Color::WHITE, Color::BLACK));
        // "a": 2x2; "g": 2x3 with its last row under the baseline; "L": an
        // L shape whose empty corner is transparent.
        let mut a = Clip::new(2, 2);
        (0..4).for_each(|i| a.cells[i] = cell('a'));
        let mut g = Clip::new(2, 3);
        (0..6).for_each(|i| g.cells[i] = cell('g'));
        let mut l = Clip::new(2, 2);
        l.cells = vec![cell('L'), None, cell('L'), cell('L')];
        let font = CutFont {
            name: "t".into(),
            spacing: -1,
            glyphs: BTreeMap::from([
                ('a', CutGlyph::new(a)),
                ('g', CutGlyph { clip: g, base: Some(1) }),
                ('L', CutGlyph::new(l)),
            ]),
        };
        let bytes = serde_json::to_vec(&font).unwrap();
        assert_eq!(CutFont::from_bytes(&bytes).unwrap(), font);
        let out = font.render("agl", 0);
        // One row of ascent above the shared baseline row 1, one of descent.
        assert_eq!((out.width, out.height), (4, 3));
        assert_eq!(out.get(0, 0).unwrap().ch, 'a');
        assert_eq!(out.get(1, 2).unwrap().ch, 'g');
        assert_eq!(out.get(0, 2), None);
        // Spacing -1: each letter covers the last column of the one before.
        assert_eq!(out.get(1, 0).unwrap().ch, 'g');
        // "l" falls back to "L"; its empty corner stays transparent.
        assert_eq!(out.get(2, 0).unwrap().ch, 'L');
        assert_eq!(out.get(3, 0), None);
        assert_eq!(out.get(3, 1).unwrap().ch, 'L');
        // Letters the font lacks are left out; a space still makes a gap.
        assert_eq!(font.render("?a#g!", 0), font.render("ag", 0));
        assert!(font.render("a g", 0).width > font.render("ag", 0).width);
    }

    #[test]
    fn latin1_figlet_parses() {
        let mut src = b"flf2a\xa0 1 1 2 -1 1\ncaf\xe9\n".to_vec();
        for c in 32u8..=126 {
            src.extend_from_slice(&[c, b'@', b'@', b'\n']);
        }
        assert!(parse(&src).is_ok());
    }
}
