//! Load/save/export for every supported format. The format is picked by
//! extension (see [`Format::from_path`]) or explicitly.

mod ansi;
mod binary;
mod native;
mod raster;
mod replay;
mod sauce;
mod text;
mod web;

use std::path::Path;

use acidtrip_core::replay::EditLog;
use acidtrip_core::{Cell, Color, DocKind, DocMeta, Document, Grid, LayerKind, Palette, cp437};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

pub use replay::{ReplayExport, replay_cast, replay_gif};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Format {
    /// Native: zstd-compressed JSON `Document` (all layers).
    Acid,
    /// Classic CP437 ANSI (.ans, .diz, .nfo, .ice).
    Ansi,
    /// UTF-8 ANSI with truecolor SGR (.utf8ans).
    Utf8Ansi,
    Bin,
    XBin,
    Adf,
    Idf,
    Tnd,
    Pcb,
    Avt,
    Ascii,
    CArray,
    PascalArray,
    AsmArray,
    Mirc,
    Png,
    Gif,
    Svg,
    Html,
    React,
    Asciicast,
}

impl Format {
    pub const ALL: [Format; 21] = [
        Format::Acid,
        Format::Ansi,
        Format::Utf8Ansi,
        Format::Bin,
        Format::XBin,
        Format::Adf,
        Format::Idf,
        Format::Tnd,
        Format::Pcb,
        Format::Avt,
        Format::Ascii,
        Format::CArray,
        Format::PascalArray,
        Format::AsmArray,
        Format::Mirc,
        Format::Png,
        Format::Gif,
        Format::Svg,
        Format::Html,
        Format::React,
        Format::Asciicast,
    ];

    /// Human name, e.g. "ANSI (CP437)".
    pub fn name(self) -> &'static str {
        match self {
            Format::Acid => "acidtrip document",
            Format::Ansi => "ANSI (CP437)",
            Format::Utf8Ansi => "UTF-8 ANSI (truecolor)",
            Format::Bin => "BIN (raw char/attribute)",
            Format::XBin => "XBin",
            Format::Adf => "ArtWorx ADF",
            Format::Idf => "iCE Draw IDF",
            Format::Tnd => "TundraDraw",
            Format::Pcb => "PCBoard @X",
            Format::Avt => "Avatar/0",
            Format::Ascii => "ASCII text",
            Format::CArray => "C array",
            Format::PascalArray => "Pascal array",
            Format::AsmArray => "Assembler array",
            Format::Mirc => "mIRC colors",
            Format::Png => "PNG image",
            Format::Gif => "GIF image",
            Format::Svg => "SVG image",
            Format::Html => "HTML page",
            Format::React => "React component (.tsx)",
            Format::Asciicast => "asciicast v2",
        }
    }

    /// Lowercase extensions without dots; the first is the default.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Format::Acid => &["acid"],
            Format::Ansi => &["ans", "ice", "diz", "nfo", "ansi", "cia", "lit", "drk"],
            Format::Utf8Ansi => &["utf8ans", "txt"],
            Format::Bin => &["bin"],
            Format::XBin => &["xb", "xbin"],
            Format::Adf => &["adf"],
            Format::Idf => &["idf"],
            Format::Tnd => &["tnd"],
            Format::Pcb => &["pcb"],
            Format::Avt => &["avt"],
            Format::Ascii => &["asc"],
            Format::CArray => &["c", "h"],
            Format::PascalArray => &["pas", "inc"],
            Format::AsmArray => &["asm", "s"],
            Format::Mirc => &["mirc", "irc"],
            Format::Png => &["png"],
            Format::Gif => &["gif"],
            Format::Svg => &["svg"],
            Format::Html => &["html", "htm"],
            Format::React => &["tsx", "jsx"],
            Format::Asciicast => &["cast"],
        }
    }

    pub fn from_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Format::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }

    /// Can be opened as a document (Png means: import image → ANSI).
    pub fn can_load(self) -> bool {
        matches!(
            self,
            Format::Acid
                | Format::Ansi
                | Format::Utf8Ansi
                | Format::Bin
                | Format::XBin
                | Format::Adf
                | Format::Idf
                | Format::Tnd
                | Format::Pcb
                | Format::Avt
                | Format::Ascii
                | Format::Png
        )
    }

    pub fn can_save(self) -> bool {
        true
    }

    /// Saving to this format keeps working on the file: it opens back as the
    /// same art (a PNG opens as an image import, so it is only ever exported).
    pub fn reopens(self) -> bool {
        self.can_load() && self != Format::Png
    }

    /// Saving to this format loses nothing for Classic docs (layers are
    /// flattened for everything but Acid).
    pub fn is_lossless_classic(self) -> bool {
        matches!(
            self,
            Format::Acid
                | Format::Ansi
                | Format::Bin
                | Format::XBin
                | Format::Idf
                | Format::Tnd
                | Format::Pcb
                | Format::Avt
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GifMode {
    /// One frame of the whole piece.
    #[default]
    Still,
    /// BBS modem-speed "download" reveal at `SaveOptions::baud`.
    Reveal,
    /// Each visible layer (bottom to top) is a frame.
    LayersAsFrames,
    /// The document's animation frames, timed by fps and hold.
    Frames,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveOptions {
    /// Attach a SAUCE record (classic formats). Defaults to `doc.meta.sauce.attach`.
    pub sauce: Option<bool>,
    /// ANSI: max chars per output line before a forced CR/LF (BBS-safe); None = unlimited.
    pub line_length: Option<usize>,
    /// ANSI: prefix `ESC[2J` (clear) — otherwise `ESC[H`-less plain start.
    pub clear_screen: bool,
    /// ANSI: append 0x1A before SAUCE.
    pub eof_char: bool,
    /// ANSI: emit the iCE hint `ESC[?33h` when the doc uses iCE colors.
    pub ice_hint: bool,
    /// Only save up to the last used row (true) or the full canvas height.
    pub trim_height: bool,
    /// Pixel scale for PNG/GIF/SVG-bitmap (1..=8).
    pub scale: u32,
    pub gif_mode: GifMode,
    /// Reveal speed in bits per second (e.g. 2400, 9600, 14400, 28800, 57600).
    pub baud: u32,
    /// SVG: `true` = pixel-exact bitmap glyphs, `false` = text runs with a web font.
    pub svg_pixel_exact: bool,
    /// Identifier for C/Pascal/ASM arrays and React component name.
    pub identifier: String,
    /// ASCII: strip colors and emit only chars; `optimize` trims trailing spaces.
    pub ascii_optimize: bool,
    /// Animated documents: ANSI and asciicast play every frame (ANSI as a
    /// cursor-home "ansimation"). Off, only the frame being shown is saved.
    #[serde(default = "yes")]
    pub animate: bool,
}

fn yes() -> bool {
    true
}

impl Default for SaveOptions {
    fn default() -> Self {
        SaveOptions {
            sauce: None,
            line_length: None,
            clear_screen: false,
            eof_char: true,
            ice_hint: false,
            trim_height: true,
            scale: 1,
            gif_mode: GifMode::Still,
            baud: 14400,
            svg_pixel_exact: false,
            identifier: "AcidArt".into(),
            ascii_optimize: true,
            animate: true,
        }
    }
}

/// Load a document from a file, detecting format by extension then content.
pub fn load(path: &Path) -> anyhow::Result<Document> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let format = Format::from_path(path).filter(|f| f.can_load()).unwrap_or_else(|| sniff(&bytes));
    load_bytes(&bytes, format).with_context(|| format!("loading {} as {}", path.display(), format.name()))
}

/// Load a document and, from `.acid` files, its edit log (for replay).
pub fn load_with_log(path: &Path) -> anyhow::Result<(Document, Option<EditLog>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let format = Format::from_path(path).filter(|f| f.can_load()).unwrap_or_else(|| sniff(&bytes));
    let loaded = match format {
        Format::Acid => native::load_with_log(&bytes),
        f => load_bytes(&bytes, f).map(|d| (d, None)),
    };
    loaded.with_context(|| format!("loading {} as {}", path.display(), format.name()))
}

/// Guess a loadable format from magic bytes (falls back to ANSI).
fn sniff(b: &[u8]) -> Format {
    if b.starts_with(b"XBIN\x1a") {
        Format::XBin
    } else if b.starts_with(b"\x041.4") || b.starts_with(b"\x041.3") {
        Format::Idf
    } else if b.len() > 9 && &b[1..9] == b"TUNDRA24" {
        Format::Tnd
    } else if b.starts_with(b"\x89PNG") {
        Format::Png
    } else if b.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Format::Acid
    } else {
        Format::Ansi
    }
}

pub fn load_bytes(bytes: &[u8], format: Format) -> anyhow::Result<Document> {
    match format {
        Format::Acid => native::load(bytes),
        Format::Ansi => ansi::load(bytes, None),
        Format::Utf8Ansi => ansi::load(bytes, Some(std::str::from_utf8(sauce::split(bytes).0).is_ok())),
        Format::Bin => binary::load_bin(bytes),
        Format::XBin => binary::load_xbin(bytes),
        Format::Adf => binary::load_adf(bytes),
        Format::Idf => binary::load_idf(bytes),
        Format::Tnd => binary::load_tnd(bytes),
        Format::Pcb => text::load_pcb(bytes),
        Format::Avt => text::load_avt(bytes),
        Format::Ascii => text::load_ascii(bytes),
        Format::Png => crate::import::image_to_doc(bytes, &crate::import::ImportOptions::default()),
        f => bail!("{} can't be loaded", f.name()),
    }
}

/// Save (atomically: temp file + rename).
pub fn save(doc: &Document, path: &Path, format: Format, opts: &SaveOptions) -> anyhow::Result<()> {
    let bytes = save_bytes(doc, format, opts)?;
    crate::library::write_atomic(path, &bytes)
}

/// Save; `.acid` also keeps the edit log. Other formats ignore it.
pub fn save_with_log(
    doc: &Document,
    log: Option<&EditLog>,
    path: &Path,
    format: Format,
    opts: &SaveOptions,
) -> anyhow::Result<()> {
    let bytes = match format {
        Format::Acid => native::save(doc, log)?,
        f => save_bytes(doc, f, opts)?,
    };
    crate::library::write_atomic(path, &bytes)
}

pub fn save_bytes(doc: &Document, format: Format, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    Ok(match format {
        Format::Acid => native::save(doc, None)?,
        Format::Ansi => ansi::save(doc, opts)?,
        Format::Utf8Ansi => ansi::save_utf8(doc, opts),
        Format::Bin => binary::save_bin(doc, opts)?,
        Format::XBin => binary::save_xbin(doc, opts)?,
        Format::Adf => binary::save_adf(doc, opts)?,
        Format::Idf => binary::save_idf(doc, opts)?,
        Format::Tnd => binary::save_tnd(doc, opts)?,
        Format::Pcb => text::save_pcb(doc, opts)?,
        Format::Avt => text::save_avt(doc, opts)?,
        Format::Ascii => text::save_ascii(doc, opts)?,
        Format::CArray | Format::PascalArray | Format::AsmArray => text::save_array(doc, format, opts),
        Format::Mirc => text::save_mirc(doc, opts),
        Format::Png => raster::save_png(doc, opts),
        Format::Gif => raster::save_gif(doc, opts)?,
        Format::Svg => web::save_svg(doc, opts),
        Format::Html => web::save_html(doc, opts)?,
        Format::React => web::save_react(doc, opts)?,
        Format::Asciicast => web::save_asciicast(doc, opts),
    })
}

/// Warnings about what saving `doc` as `format` would lose (e.g. "3 layers
/// will be flattened", "RGB colors will be reduced to 16"). Empty = lossless.
pub fn loss_warnings(doc: &Document, format: Format) -> Vec<String> {
    let mut w = Vec::new();
    if format == Format::Acid {
        return w;
    }
    if doc.is_animated() && !can_animate(format) {
        w.push(format!(
            "{} frames: only the one shown is saved (animate as .ans, .cast or a GIF in frames mode)",
            doc.frame_count()
        ));
    }
    let normal: Vec<_> = doc.canvas.layers.iter().filter(|l| l.kind == LayerKind::Normal).collect();
    let visible = normal.iter().filter(|l| l.visible && !l.is_empty()).count();
    if visible > 1 {
        w.push(format!("{visible} layers will be flattened into one"));
    }
    let hidden = normal.iter().filter(|l| !l.visible && !l.is_empty()).count();
    if hidden > 0 {
        w.push(format!("{hidden} hidden layer(s) will not be saved"));
    }
    let g = doc.flatten();
    let pal = &doc.meta.palette;
    let (width, height) = (doc.width(), doc.height());
    let rgb = g.cells.iter().filter(|c| [c.fg, c.bg].iter().any(|&col| !in_palette16(col, pal))).count();
    let non437 = g.cells.iter().filter(|c| !cp437::is_cp437(c.ch)).count();
    let bright_bg = g.cells.iter().any(|c| idx16(c.bg, pal) >= 8);

    let cp437_only = !matches!(
        format,
        Format::Utf8Ansi
            | Format::Mirc
            | Format::Png
            | Format::Gif
            | Format::Svg
            | Format::Html
            | Format::React
            | Format::Asciicast
    );
    if cp437_only && non437 > 0 {
        w.push(format!("{non437} character(s) are not in CP437 and will be replaced"));
    }
    let rgb_ok = matches!(
        format,
        Format::Utf8Ansi
            | Format::Tnd
            | Format::Png
            | Format::Gif
            | Format::Svg
            | Format::Html
            | Format::React
            | Format::Asciicast
    ) || (format == Format::Ansi && doc.is_classic());
    if rgb > 0 && !rgb_ok && format != Format::Ascii {
        w.push(format!("{rgb} cell(s) use RGB colors that will be reduced to the 16-color palette"));
    }
    if format == Format::Ascii {
        w.push("colors will be discarded (text only)".into());
    }
    if format == Format::Ansi && doc.is_classic() && rgb > 0 {
        w.push("RGB colors use PabloDraw 24-bit codes that most ANSI viewers ignore".into());
    }
    if bright_bg && matches!(format, Format::Ansi | Format::Pcb | Format::Avt | Format::Bin) {
        w.push("bright backgrounds need iCE colors; non-iCE viewers will show blinking text".into());
    }
    if matches!(format, Format::Ansi | Format::Pcb | Format::Avt | Format::Tnd) && width != 80 {
        w.push(format!("width {width} ≠ 80: many viewers assume 80 columns (width is stored in SAUCE)"));
    }
    if format == Format::Adf && width != 80 {
        w.push(format!("ADF is always 80 columns; width {width} will be cropped/padded"));
    }
    if format == Format::Bin && (width % 2 == 1 || width > 510) {
        w.push(format!("BIN width {width} can't be stored in SAUCE (must be even and ≤ 510)"));
    }
    if matches!(format, Format::XBin | Format::Idf) && (width > 65535 || height > 65535) {
        w.push("canvas is too large for this format".into());
    }
    if matches!(format, Format::Ansi | Format::Pcb) {
        let bad = g.cells.iter().filter(|c| ansi::unprintable(char_byte(c.ch))).count();
        if bad > 0 {
            w.push(format!("{bad} control character(s) can't be written raw and will be substituted"));
        }
    }
    w
}

// ---------------------------------------------------------------------------
// Shared helpers.

/// Palette index 0..16 of a color (nearest for RGB or out-of-range indices).
pub(crate) fn idx16(c: Color, pal: &Palette) -> u8 {
    match c {
        Color::Pal(i) if i < 16 => i,
        other => pal.nearest(other.rgb(pal), 16),
    }
}

/// DOS attribute byte: fg in bits 0-3, bg in 4-7 (bit 7 = bright bg / blink).
pub(crate) fn attr_byte(c: &Cell, pal: &Palette) -> u8 {
    idx16(c.fg, pal) | idx16(c.bg, pal) << 4
}

pub(crate) fn attr_cell(ch: u8, attr: u8) -> Cell {
    Cell::new(cp437::to_char(ch), Color::Pal(attr & 15), Color::Pal(attr >> 4))
}

pub(crate) fn char_byte(ch: char) -> u8 {
    cp437::from_char_lossy(ch)
}

fn in_palette16(c: Color, pal: &Palette) -> bool {
    match c {
        Color::Pal(i) => i < 16,
        Color::Rgb(r, g, b) => pal.colors.iter().take(16).any(|&p| p == [r, g, b]),
    }
}

/// Rows to export: the used height (≥ 1) or the full canvas.
pub(crate) fn export_rows(doc: &Document, opts: &SaveOptions) -> usize {
    if opts.trim_height { doc.canvas.used_height().max(1).min(doc.height().max(1)) } else { doc.height() }
}

/// Flattened grid cut to the exported rows.
pub(crate) fn export_grid(doc: &Document, opts: &SaveOptions) -> Grid {
    let mut g = doc.flatten();
    let rows = export_rows(doc, opts).min(g.height);
    g.cells.truncate(rows * g.width);
    g.height = rows;
    g
}

/// Grid with every cell reduced to CP437 + 16 colors (bright backgrounds kept).
pub(crate) fn classic_grid(doc: &Document, opts: &SaveOptions) -> Grid {
    let mut g = export_grid(doc, opts);
    to_classic(&mut g, &doc.meta.palette);
    g
}

pub(crate) fn to_classic(g: &mut Grid, pal: &Palette) {
    for c in &mut g.cells {
        *c = Cell::new(cp437::to_char(char_byte(c.ch)), Color::Pal(idx16(c.fg, pal)), Color::Pal(idx16(c.bg, pal)));
    }
}

/// Formats that can carry every animation frame (GIF in frames mode).
pub fn can_animate(format: Format) -> bool {
    matches!(format, Format::Ansi | Format::Asciicast | Format::Gif)
}

/// Every animation frame, flattened and cut to the same exported rows (the
/// most any frame uses), with how many ticks each stays up.
pub fn frame_grids(doc: &Document, opts: &SaveOptions) -> Vec<(Grid, u32)> {
    let n = doc.frame_count();
    let rows = if opts.trim_height {
        (0..n).map(|i| doc.frame_canvas(i).used_height()).max().unwrap_or(0).max(1).min(doc.height().max(1))
    } else {
        doc.height()
    };
    (0..n)
        .map(|i| {
            let mut g = doc.frame_canvas(i).flatten();
            let rows = rows.min(g.height);
            g.cells.truncate(rows * g.width);
            g.height = rows;
            (g, doc.hold(i))
        })
        .collect()
}

/// How long a frame held for `hold` ticks stays up, in GIF centiseconds.
pub fn frame_delay_cs(fps: u32, hold: u32) -> u16 {
    ((100 * hold as u64 + fps.max(1) as u64 / 2) / fps.max(1) as u64).clamp(2, u16::MAX as u64) as u16
}

/// Whether to attach SAUCE: explicit option, else the doc's flag or when
/// `needed` (e.g. a width the format can't imply).
pub(crate) fn want_sauce(doc: &Document, opts: &SaveOptions, needed: bool) -> bool {
    opts.sauce.unwrap_or(doc.meta.sauce.attach || needed)
}

/// Build a document from a loaded grid: RGB colors that match the palette
/// become palette indices; the doc is Modern if anything else remains.
pub(crate) fn finish(
    mut grid: Grid,
    rec: Option<&icy_sauce::SauceRecord>,
    palette: Option<Palette>,
    force_modern: bool,
    ice_hint: bool,
) -> Document {
    let mut meta = DocMeta { ice: false, ..DocMeta::default() };
    if let Some(p) = palette {
        meta.palette = p;
    }
    if let Some(r) = rec {
        sauce::apply(r, &mut meta);
    }
    let pal16: Vec<[u8; 3]> = meta.palette.colors.iter().take(16).copied().collect();
    let canon = |c: Color| match c {
        Color::Rgb(r, g, b) => pal16.iter().position(|&p| p == [r, g, b]).map_or(c, |i| Color::Pal(i as u8)),
        Color::Pal(i) if i >= 16 => {
            let [r, g, b] = meta.palette.get(i);
            Color::Rgb(r, g, b)
        }
        c => c,
    };
    let mut modern = force_modern;
    for c in &mut grid.cells {
        c.fg = canon(c.fg);
        c.bg = canon(c.bg);
        modern |= matches!(c.fg, Color::Rgb(..)) || matches!(c.bg, Color::Rgb(..)) || !cp437::is_cp437(c.ch);
    }
    meta.ice = meta.ice || ice_hint || grid.cells.iter().any(|c| matches!(c.bg, Color::Pal(8..=15)));
    meta.kind = if modern { DocKind::Modern } else { DocKind::Classic };
    let mut d = Document::from_grid(meta.kind, &grid);
    d.meta = meta;
    d
}

/// Standard VGA palette check (so loaders keep the default palette name).
pub(crate) fn palette_from(colors: Vec<[u8; 3]>, name: &str) -> Option<Palette> {
    (colors.as_slice() != acidtrip_core::color::VGA.as_slice()).then(|| Palette { name: name.into(), colors })
}

/// 6-bit VGA DAC value → 8-bit.
pub(crate) fn dac8(v: u8) -> u8 {
    let v = v & 63;
    v << 2 | v >> 4
}

/// The VGA 8x16 font as 4096 raw bytes (for formats that embed a font).
pub(crate) fn vga_font_bytes() -> Vec<u8> {
    let mut out = Vec::with_capacity(4096);
    for b in 0..=255u8 {
        let ch = cp437::to_char(b);
        for py in 0..16 {
            let mut row = 0u8;
            for px in 0..8 {
                if acidtrip_core::render::glyph_pixel(ch, px, py) {
                    row |= 0x80 >> px;
                }
            }
            out.push(row);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path_is_case_insensitive() {
        assert_eq!(Format::from_path(Path::new("X.ANS")), Some(Format::Ansi));
        assert_eq!(Format::from_path(Path::new("a/b.Xb")), Some(Format::XBin));
        assert_eq!(Format::from_path(Path::new("file_id.diz")), Some(Format::Ansi));
        assert_eq!(Format::from_path(Path::new("x.cast")), Some(Format::Asciicast));
        assert_eq!(Format::from_path(Path::new("noext")), None);
        for f in Format::ALL {
            assert!(!f.name().is_empty());
            assert_eq!(Format::from_path(Path::new(&format!("a.{}", f.extensions()[0]))), Some(f));
        }
    }

    #[test]
    fn font_bytes_match_renderer() {
        let f = vga_font_bytes();
        assert_eq!(f.len(), 4096);
        assert!(f[0xDB * 16..0xDC * 16].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn warnings() {
        let mut d = Document::new(DocKind::Modern, 100, 2);
        d.canvas.layers[0].cells[0] = Some(Cell::new('😀', Color::Rgb(1, 2, 3), Color::BLACK));
        let w = loss_warnings(&d, Format::Ansi);
        assert!(w.iter().any(|s| s.contains("CP437")), "{w:?}");
        assert!(w.iter().any(|s| s.contains("RGB")), "{w:?}");
        assert!(w.iter().any(|s| s.contains("width 100")), "{w:?}");
        assert!(loss_warnings(&d, Format::Acid).is_empty());
        let c = Document::new(DocKind::Classic, 80, 25);
        assert!(loss_warnings(&c, Format::XBin).is_empty());
    }
}
