//! The EXPORT panel's rows (Figma-style export settings): which files a
//! piece's rows name, and writing them all at once.
//!
//! A row is a format, a scale and a name pattern. `{name}` is the piece's
//! name, `{scale}` the scale ("2x"), `{w}`/`{h}` the size in cells, and
//! `{frame}` writes one file per animation frame (1, 2, ... padded).
//! The extension comes from the format.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use acidtrip_core::tools::Rect;
use acidtrip_core::{Canvas, Document, ExportPreset, ExportSettings, Layer};
use serde::{Deserialize, Serialize};

use crate::format::{self, Format, GifMode, SaveOptions};

/// Formats in the order the row editor lists them: images and web first,
/// then art files, then code.
pub const FORMATS: [Format; 21] = [
    Format::Png,
    Format::Svg,
    Format::Gif,
    Format::Html,
    Format::React,
    Format::Ansi,
    Format::XBin,
    Format::Bin,
    Format::Utf8Ansi,
    Format::Asciicast,
    Format::Acid,
    Format::Ascii,
    Format::Mirc,
    Format::Adf,
    Format::Idf,
    Format::Tnd,
    Format::Pcb,
    Format::Avt,
    Format::CArray,
    Format::PascalArray,
    Format::AsmArray,
];

/// Scales the scale chip steps through.
pub const SCALES: [u32; 6] = [1, 2, 3, 4, 6, 8];

/// The format a row writes (by its extension id).
pub fn format_of(p: &ExportPreset) -> Option<Format> {
    let id = p.format.trim_start_matches('.').to_ascii_lowercase();
    Format::ALL.into_iter().find(|f| f.extensions().contains(&id.as_str()))
}

/// How a row stores a format.
pub fn format_id(f: Format) -> String {
    f.extensions().first().copied().unwrap_or("txt").to_string()
}

/// A short chip label for a format: "PNG", "ANS", "TSX".
pub fn short_label(f: Format) -> String {
    format_id(f).to_uppercase().chars().take(4).collect()
}

/// The format has a pixel scale.
pub fn has_scale(f: Format) -> bool {
    matches!(f, Format::Png | Format::Gif)
}

/// The rows to show and write: the saved ones, or one PNG at 1x.
pub fn rows(s: &ExportSettings) -> Vec<ExportPreset> {
    if s.presets.is_empty() { vec![ExportPreset::default()] } else { s.presets.clone() }
}

/// The name a row gets for a scale until you type your own.
pub fn auto_name(scale: u32) -> String {
    if scale <= 1 { "{name}".into() } else { format!("{{name}}@{scale}x") }
}

/// Change a row's scale; a name that was still automatic follows it.
pub fn set_scale(p: &mut ExportPreset, scale: u32) {
    if p.name == auto_name(p.scale) {
        p.name = auto_name(scale);
    }
    p.scale = scale.clamp(1, 8);
}

/// The next scale up (or down, `dir < 0`) from `s`, wrapping around.
pub fn step_scale(s: u32, dir: i32) -> u32 {
    let n = SCALES.len() as i32;
    let i = SCALES.iter().position(|&x| x >= s).unwrap_or(0) as i32;
    let i = if SCALES.get(i as usize) == Some(&s) || dir < 0 { i + dir } else { i };
    SCALES[i.rem_euclid(n) as usize]
}

/// A row for a newly added line: the next format that isn't used yet,
/// starting with the common ones.
pub fn next_row(rows: &[ExportPreset]) -> ExportPreset {
    let used = |f: Format| rows.iter().any(|r| format_of(r) == Some(f) && r.scale == 1);
    // A 2x PNG after the 1x one is what people add most.
    if rows.iter().any(|r| format_of(r) == Some(Format::Png) && r.scale == 1)
        && !rows.iter().any(|r| format_of(r) == Some(Format::Png) && r.scale == 2)
    {
        return ExportPreset { scale: 2, name: auto_name(2), ..ExportPreset::default() };
    }
    let f = [Format::Svg, Format::Ansi, Format::Html, Format::Gif, Format::Png]
        .into_iter()
        .find(|&f| !used(f))
        .unwrap_or(Format::Png);
    ExportPreset { format: format_id(f), ..ExportPreset::default() }
}

/// The save options a row writes with. Rows that never had options set get
/// the ones the Export dialog starts with for this piece.
pub fn options(p: &ExportPreset, doc: &Document) -> SaveOptions {
    let mut o = if p.options.is_null() {
        let mut o =
            SaveOptions { sauce: Some(doc.meta.sauce.attach), ice_hint: doc.meta.ice, ..SaveOptions::default() };
        if doc.is_animated() {
            o.gif_mode = GifMode::Frames;
        }
        o
    } else {
        serde_json::from_value(p.options.clone()).unwrap_or_default()
    };
    o.scale = p.scale.clamp(1, 8);
    o
}

/// Store edited options on a row.
pub fn set_options(p: &mut ExportPreset, o: &SaveOptions) {
    p.scale = o.scale.clamp(1, 8);
    p.options = serde_json::to_value(o).unwrap_or_default();
}

/// What the name pattern's placeholders stand for.
pub struct Vars<'a> {
    pub name: &'a str,
    pub scale: u32,
    pub w: usize,
    pub h: usize,
    /// The frame number, already padded ("01").
    pub frame: Option<&'a str>,
}

/// Fill in a name pattern. Path separators become dashes, so a row always
/// names a file in the export folder.
pub fn expand(pattern: &str, v: &Vars) -> String {
    let out = pattern
        .replace("{name}", v.name)
        .replace("{scale}", &format!("{}x", v.scale))
        .replace("{w}", &v.w.to_string())
        .replace("{h}", &v.h.to_string())
        .replace("{frame}", v.frame.unwrap_or("1"));
    let out: String = out.trim().chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '-' } else { c }).collect();
    let out = out.trim_matches('.').to_string();
    if out.is_empty() { v.name.to_string() } else { out }
}

/// The pattern writes one file per frame.
pub fn per_frame(p: &ExportPreset) -> bool {
    p.name.contains("{frame}")
}

/// `name.ext`; React components get a PascalCase file name.
pub fn file_name(stem: &str, f: Format) -> String {
    let ext = format_id(f);
    if f == Format::React { format!("{}.{ext}", pascal(stem)) } else { format!("{stem}.{ext}") }
}

/// "my-cool art" → "MyCoolArt".
pub fn pascal(s: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(if up { c.to_uppercase().collect::<Vec<_>>() } else { vec![c] });
            up = false;
        } else {
            up = true;
        }
    }
    if out.is_empty() { "AcidArt".into() } else { out }
}

/// The piece's name for `{name}`: its file's stem, else its SAUCE title,
/// else "untitled".
pub fn doc_name(file: Option<&Path>, doc: &Document) -> String {
    if let Some(stem) = file.and_then(|f| f.file_stem()) {
        return stem.to_string_lossy().into_owned();
    }
    let t = doc.meta.sauce.title.trim();
    if t.is_empty() { "untitled".into() } else { t.to_lowercase().replace(' ', "-") }
}

/// The folder the files go to. `base` is the piece's folder (or the working
/// directory for an unsaved piece).
pub fn folder(s: &ExportSettings, base: &Path) -> PathBuf {
    match s.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        None => base.to_path_buf(),
        Some(f) if Path::new(f).is_absolute() => PathBuf::from(f),
        Some(f) => base.join(f),
    }
}

/// How to remember a chosen folder: nothing when it's the piece's own,
/// relative when it's inside it (so the piece can move), else absolute.
pub fn store_folder(chosen: &Path, base: &Path) -> Option<String> {
    if chosen == base {
        return None;
    }
    match chosen.strip_prefix(base) {
        Ok(rel) if !rel.as_os_str().is_empty() => Some(rel.display().to_string()),
        _ => Some(chosen.display().to_string()),
    }
}

/// One file to write.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub path: PathBuf,
    pub format: Format,
    pub opts: SaveOptions,
    /// Write only this frame.
    pub frame: Option<usize>,
}

/// Every file the rows write for `doc` into `dir`, or why they can't be
/// written (an unknown format, two rows naming the same file).
pub fn plan(doc: &Document, rows: &[ExportPreset], dir: &Path, name: &str) -> Result<Vec<Job>, String> {
    plan_sized(doc, (doc.width(), doc.height()), rows, dir, name)
}

/// [`plan`] for a part of `doc` that is `size` cells (for `{w}`, `{h}`).
pub fn plan_sized(
    doc: &Document,
    size: (usize, usize),
    rows: &[ExportPreset],
    dir: &Path,
    name: &str,
) -> Result<Vec<Job>, String> {
    let n = doc.frame_count();
    let pad = n.to_string().len();
    let mut jobs = vec![];
    let mut seen = HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        let Some(fmt) = format_of(row) else {
            return Err(format!("row {}: unknown format .{}", i + 1, row.format));
        };
        let frames: Vec<Option<usize>> = if per_frame(row) { (0..n).map(Some).collect() } else { vec![None] };
        for frame in frames {
            let label = frame.map(|f| format!("{:0pad$}", f + 1));
            let vars = Vars { name, scale: row.scale, w: size.0, h: size.1, frame: label.as_deref() };
            let file = file_name(&expand(&row.name, &vars), fmt);
            if !seen.insert(file.clone()) {
                return Err(format!("two rows write {file}: give one another name"));
            }
            jobs.push(Job { path: dir.join(file), format: fmt, opts: options(row, doc), frame });
        }
    }
    Ok(jobs)
}

/// Write every job, creating the folder. Stops at the first failure.
pub fn write(doc: &Document, jobs: &[Job]) -> anyhow::Result<()> {
    if let Some(dir) = jobs.first().and_then(|j| j.path.parent()) {
        std::fs::create_dir_all(dir)?;
    }
    for j in jobs {
        match j.frame {
            Some(i) => format::save(&frame_doc(doc, i), &j.path, j.format, &j.opts)?,
            None => format::save(doc, &j.path, j.format, &j.opts)?,
        }
    }
    Ok(())
}

/// A still piece of frame `i`.
pub fn frame_doc(doc: &Document, i: usize) -> Document {
    Document::with_canvas(doc.meta.clone(), doc.frame_canvas(i).clone())
}

/// The piece cut to `r` (clipped to the canvas), every frame and layer.
pub fn scoped(doc: &Document, r: Rect) -> Document {
    let mut out = doc.clone();
    out.canvas = crop(&doc.canvas, r);
    for f in &mut out.frames.list {
        if let Some(c) = &f.canvas {
            f.canvas = Some(crop(c, r));
        }
    }
    out
}

/// `c` cut to `r`, clipped to the canvas (at least one cell).
pub fn crop(c: &Canvas, r: Rect) -> Canvas {
    let x = r.x.min(c.width.saturating_sub(1));
    let y = r.y.min(c.height.saturating_sub(1));
    let w = r.w.min(c.width - x).max(1);
    let h = r.h.min(c.height - y).max(1);
    Canvas {
        width: w,
        height: h,
        layers: c
            .layers
            .iter()
            .map(|l| Layer {
                cells: (y..y + h)
                    .flat_map(|row| l.cells[row * c.width + x..row * c.width + x + w].iter().copied())
                    .collect(),
                ..l.clone()
            })
            .collect(),
    }
}

// Rows for art files. A .acid keeps its rows inside; .ans, .xb and the other
// art formats have no room for them, so they are kept in the data folder
// (`exports/<id>.json`), known by the file's path the way its versions are.

#[derive(Serialize, Deserialize)]
struct Kept {
    /// The art file, for anyone looking in the folder.
    file: String,
    exports: ExportSettings,
}

fn kept_path(dir: &Path, file: &Path) -> PathBuf {
    dir.join(format!("{}.json", crate::versions::file_doc_id(file)))
}

/// Keep `file`'s rows in `dir`. No rows (the default one PNG) forgets them.
pub fn remember(dir: &Path, file: &Path, s: &ExportSettings) -> anyhow::Result<()> {
    let path = kept_path(dir, file);
    if s.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    let kept = Kept { file: file.display().to_string(), exports: s.clone() };
    crate::library::write_atomic(&path, &serde_json::to_vec_pretty(&kept)?)
}

/// The rows kept for `file`, or none.
pub fn recall(dir: &Path, file: &Path) -> ExportSettings {
    std::fs::read(kept_path(dir, file))
        .ok()
        .and_then(|b| serde_json::from_slice::<Kept>(&b).ok())
        .map(|k| k.exports)
        .unwrap_or_default()
}

/// A document just loaded from `file` gets the rows kept for it, unless the
/// file held its own (a .acid).
pub fn adopt(dir: &Path, file: &Path, doc: &mut Document) {
    if doc.meta.exports.is_empty() && Format::from_path(file) != Some(Format::Acid) {
        doc.meta.exports = recall(dir, file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{Cell, Color, DocKind, Frame};

    fn vars(name: &str) -> Vars<'_> {
        Vars { name, scale: 2, w: 80, h: 25, frame: None }
    }

    #[test]
    fn expands_placeholders() {
        assert_eq!(expand("{name}", &vars("art")), "art");
        assert_eq!(expand("{name}@{scale}", &vars("art")), "art@2x");
        assert_eq!(expand("{name}-{w}x{h}", &vars("art")), "art-80x25");
        assert_eq!(expand("{name}-{frame}", &Vars { frame: Some("03"), ..vars("art") }), "art-03");
        // No frame number outside per-frame export.
        assert_eq!(expand("{name}-{frame}", &vars("art")), "art-1");
        // Unknown braces stay.
        assert_eq!(expand("{name}{x}", &vars("a")), "a{x}");
    }

    #[test]
    fn names_stay_in_the_folder() {
        assert_eq!(expand("../{name}/x", &vars("art")), "-art-x");
        assert_eq!(expand("  ", &vars("art")), "art");
        assert_eq!(expand("...", &vars("art")), "art");
    }

    #[test]
    fn file_names_take_the_format_extension() {
        assert_eq!(file_name("art@2x", Format::Png), "art@2x.png");
        assert_eq!(file_name("art", Format::Ansi), "art.ans");
        assert_eq!(file_name("my cool-art", Format::React), "MyCoolArt.tsx");
    }

    #[test]
    fn default_row_is_png_1x() {
        let rows = rows(&ExportSettings::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(format_of(&rows[0]), Some(Format::Png));
        assert_eq!(rows[0].scale, 1);
        assert_eq!(rows[0].name, "{name}");
    }

    #[test]
    fn scale_moves_an_automatic_name_only() {
        let mut p = ExportPreset::default();
        set_scale(&mut p, 2);
        assert_eq!(p.name, "{name}@2x");
        set_scale(&mut p, 1);
        assert_eq!(p.name, "{name}");
        p.name = "hero".into();
        set_scale(&mut p, 4);
        assert_eq!((p.name.as_str(), p.scale), ("hero", 4));
    }

    #[test]
    fn scale_steps_wrap() {
        assert_eq!(step_scale(1, 1), 2);
        assert_eq!(step_scale(4, 1), 6);
        assert_eq!(step_scale(8, 1), 1);
        assert_eq!(step_scale(1, -1), 8);
        assert_eq!(step_scale(5, 1), 6);
        assert_eq!(step_scale(5, -1), 4);
    }

    #[test]
    fn added_rows_pick_something_new() {
        let one = rows(&ExportSettings::default());
        let two = next_row(&one);
        assert_eq!((format_of(&two), two.scale, two.name.as_str()), (Some(Format::Png), 2, "{name}@2x"));
        let three = next_row(&[one[0].clone(), two]);
        assert_eq!(format_of(&three), Some(Format::Svg));
    }

    #[test]
    fn format_ids_round_trip() {
        for f in FORMATS {
            let p = ExportPreset { format: format_id(f), ..ExportPreset::default() };
            assert_eq!(format_of(&p), Some(f));
        }
        let p = ExportPreset { format: "ANS".into(), ..ExportPreset::default() };
        assert_eq!(format_of(&p), Some(Format::Ansi));
        assert_eq!(short_label(Format::Utf8Ansi), "UTF8");
    }

    #[test]
    fn plans_every_row_and_refuses_clashes() {
        let doc = Document::new(DocKind::Classic, 10, 4);
        let a = ExportPreset::default();
        let mut b = ExportPreset::default();
        set_scale(&mut b, 2);
        let svg = ExportPreset { format: "svg".into(), ..ExportPreset::default() };
        let jobs = plan(&doc, &[a.clone(), b, svg], Path::new("/out"), "art").unwrap();
        let paths: Vec<_> = jobs.iter().map(|j| j.path.display().to_string()).collect();
        assert_eq!(paths, ["/out/art.png", "/out/art@2x.png", "/out/art.svg"]);
        assert_eq!(jobs[1].opts.scale, 2);
        let err = plan(&doc, &[a.clone(), a], Path::new("/out"), "art").unwrap_err();
        assert!(err.contains("art.png"), "{err}");
        let bad = ExportPreset { format: "zzz".into(), ..ExportPreset::default() };
        assert!(plan(&doc, &[bad], Path::new("/out"), "art").is_err());
    }

    #[test]
    fn frame_rows_write_one_file_per_frame() {
        let mut doc = Document::new(DocKind::Classic, 4, 2);
        for id in 2..=10 {
            doc.frames.list.push(Frame { id, hold: 1, canvas: Some(doc.canvas.clone()) });
        }
        let row = ExportPreset { name: "{name}-{frame}".into(), ..ExportPreset::default() };
        let jobs = plan(&doc, &[row], Path::new("/o"), "a").unwrap();
        assert_eq!(jobs.len(), 10);
        assert_eq!(jobs[0].path, Path::new("/o/a-01.png"));
        assert_eq!(jobs[9].path, Path::new("/o/a-10.png"));
        assert_eq!(jobs[9].frame, Some(9));
    }

    #[test]
    fn options_start_from_the_piece_and_keep_edits() {
        let mut doc = Document::new(DocKind::Classic, 4, 2);
        doc.meta.ice = false;
        let mut p = ExportPreset { scale: 3, ..ExportPreset::default() };
        let o = options(&p, &doc);
        assert_eq!((o.scale, o.ice_hint, o.sauce), (3, false, Some(doc.meta.sauce.attach)));
        let edited = SaveOptions { trim_height: false, scale: 2, ..o };
        set_options(&mut p, &edited);
        assert_eq!(p.scale, 2);
        assert!(!options(&p, &doc).trim_height);
        // Rows survive a trip through the document file.
        doc.meta.exports.presets.push(p.clone());
        let back: acidtrip_core::DocMeta = serde_json::from_value(serde_json::to_value(&doc.meta).unwrap()).unwrap();
        assert_eq!(back.exports.presets, [p]);
    }

    #[test]
    fn folders_are_remembered_relative_to_the_piece() {
        let base = Path::new("/art");
        assert_eq!(store_folder(Path::new("/art"), base), None);
        assert_eq!(store_folder(Path::new("/art/out"), base).as_deref(), Some("out"));
        assert_eq!(store_folder(Path::new("/tmp/x"), base).as_deref(), Some("/tmp/x"));
        let s = ExportSettings { folder: Some("out".into()), ..ExportSettings::default() };
        assert_eq!(folder(&s, base), Path::new("/art/out"));
        assert_eq!(folder(&ExportSettings::default(), base), base);
    }

    #[test]
    fn scoped_cuts_every_frame() {
        let mut doc = Document::new(DocKind::Classic, 6, 4);
        doc.canvas.layers[0].cells[6 + 2] = Some(Cell::new('X', Color::Pal(7), Color::Pal(0)));
        doc.frames.list.push(Frame { id: 2, hold: 1, canvas: Some(doc.canvas.clone()) });
        let s = scoped(&doc, Rect { x: 2, y: 1, w: 3, h: 10 });
        assert_eq!((s.width(), s.height()), (3, 3));
        assert_eq!(s.frame_canvas(1).width, 3);
        assert_eq!(s.canvas.layers[0].cells[0].unwrap().ch, 'X');
    }

    #[test]
    fn art_files_keep_their_rows_outside() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("exports");
        let art = home.path().join("a.ans");
        let s = ExportSettings {
            presets: vec![ExportPreset { scale: 2, name: auto_name(2), ..ExportPreset::default() }],
            folder: Some("out".into()),
        };
        remember(&dir, &art, &s).unwrap();
        // Reopened (and saved over) the .ans keeps them; another file doesn't get them.
        let mut doc = Document::new(DocKind::Classic, 80, 25);
        adopt(&dir, &art, &mut doc);
        assert_eq!(doc.meta.exports, s);
        assert!(recall(&dir, &home.path().join("b.ans")).is_empty());
        // A .acid keeps what's inside it.
        let mut acid = Document::new(DocKind::Classic, 80, 25);
        adopt(&dir, &home.path().join("a.acid"), &mut acid);
        assert!(acid.meta.exports.is_empty());
        // Back to the default row: nothing is kept.
        remember(&dir, &art, &ExportSettings::default()).unwrap();
        assert!(recall(&dir, &art).is_empty());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        remember(&dir, &art, &ExportSettings::default()).unwrap();
    }

    #[test]
    fn writes_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let doc = Document::new(DocKind::Classic, 8, 2);
        let rows = [ExportPreset::default(), ExportPreset { format: "ans".into(), ..ExportPreset::default() }];
        let out = dir.path().join("sub");
        let jobs = plan(&doc, &rows, &out, "t").unwrap();
        write(&doc, &jobs).unwrap();
        assert!(out.join("t.png").exists());
        assert!(out.join("t.ans").exists());
    }
}
