//! Execute a [`ToolCall`] against a document. Each mutating call commits
//! exactly one transaction (one undo step) labelled "AI: <tool>".
//!
//! `execute` may change `state.layer` (layer add/select/remove, load) and
//! `state.file` (save/load/new_canvas); callers should persist both.

use std::fmt::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use acidtrip_core::color::VGA_NAMES;
use acidtrip_core::model::{Canvas, Layer};
use acidtrip_core::render::{RenderOptions, png_bytes, render_grid};
use acidtrip_core::tools::{
    self, BoxStyle, Brush, Ctx, FillMatch, FillWhat, Justify, LayerProps, PaintMode, Rect, ShapeFill, StampMode,
    Symmetry,
};
use acidtrip_core::{Cell, Color, DocKind, Document, Grid, History, Palette, TxBuilder, cp437};
use acidtrip_io::fonts::{FontLibrary, TextRenderOptions};
use acidtrip_io::format::{self, Format, SaveOptions};
use acidtrip_io::import::{Dither, ImportOptions, ImportStyle, Preset};
use acidtrip_io::library::Paths;
use acidtrip_io::stencils::{Stencil, StencilLibrary, StencilMeta};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use crate::{ToolCall, ToolResult, schema};

pub struct ExecState<'a> {
    pub doc: &'a mut Document,
    pub history: &'a mut History,
    /// Layer AI edits go to by default (the app creates an "AI" layer).
    pub layer: usize,
    pub fonts: &'a FontLibrary,
    pub stencils: &'a mut StencilLibrary,
    pub paths: &'a Paths,
    /// The doc's file path, if saved.
    pub file: &'a mut Option<PathBuf>,
}

pub fn execute(state: &mut ExecState, call: &ToolCall) -> ToolResult {
    match std::panic::catch_unwind(AssertUnwindSafe(|| run(state, call))) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => ToolResult::err(format!("{}: {e:#}", call.name)),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            ToolResult::err(format!("{}: internal error: {msg}", call.name))
        }
    }
}

// ------------------------------------------------------------------ arg types

/// Color argument: palette index, VGA name or "#rrggbb".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Col(pub Color);

impl<'de> Deserialize<'de> for Col {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        parse_color(&v).map(Col).map_err(serde::de::Error::custom)
    }
}

/// Parse a color argument (see [`schema::COLOR_HELP`]).
pub fn parse_color(v: &Value) -> Result<Color, String> {
    if let Some(n) = v.as_u64() {
        return u8::try_from(n).map(Color::Pal).map_err(|_| format!("color index {n} out of range 0-255"));
    }
    let Some(s) = v.as_str() else {
        return Err(format!("color must be an index 0-15, a name or \"#rrggbb\", got {v}"));
    };
    let hex = s.trim().trim_start_matches('#');
    if hex.len() == 6
        && let Ok(n) = u32::from_str_radix(hex, 16)
    {
        return Ok(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
    }
    if let Ok(n) = s.trim().parse::<u8>() {
        return Ok(Color::Pal(n));
    }
    let key: String = s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let alias = match key.as_str() {
        "grey" | "gray" | "lightgrey" => "lightgray",
        "darkgrey" => "darkgray",
        "purple" => "magenta",
        "pink" | "brightmagenta" => "lightmagenta",
        "darkyellow" => "brown",
        "brightwhite" => "white",
        "brightblue" => "lightblue",
        "brightgreen" => "lightgreen",
        "brightcyan" => "lightcyan",
        "brightred" => "lightred",
        k => k,
    };
    VGA_NAMES
        .iter()
        .position(|n| n.replace(' ', "") == alias)
        .map(|i| Color::Pal(i as u8))
        .ok_or_else(|| format!("unknown color {s:?}; use 0-15, a VGA name or \"#rrggbb\""))
}

/// Glyph argument: a one-char string or a CP437 code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ch(pub char);

impl<'de> Deserialize<'de> for Ch {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if let Some(n) = v.as_u64() {
            return u8::try_from(n)
                .map(|b| Ch(cp437::to_char(b)))
                .map_err(|_| serde::de::Error::custom("CP437 code must be 0-255"));
        }
        match v.as_str().map(|s| s.chars().collect::<Vec<_>>()) {
            Some(c) if c.len() == 1 => Ok(Ch(c[0])),
            Some(c) if c.is_empty() => Ok(Ch(' ')),
            _ => Err(serde::de::Error::custom(format!("ch must be a single character or a CP437 code, got {v}"))),
        }
    }
}

#[derive(Default, Clone, Copy)]
struct RegionArgs {
    x: Option<usize>,
    y: Option<usize>,
    w: Option<usize>,
    h: Option<usize>,
}

fn parse<T: DeserializeOwned>(call: &ToolCall) -> Result<T> {
    let v = if call.args.is_null() { json!({}) } else { call.args.clone() };
    serde_json::from_value(v)
        .map_err(|e| anyhow!("bad arguments ({e}). Expected: {}", schema::args_summary(&call.name)))
}

fn col(c: Option<Col>, default: Color) -> Color {
    c.map_or(default, |c| c.0)
}

fn ctx(layer: usize, ch: Option<Ch>, fg: Option<Col>, bg: Option<Col>, mode: PaintMode) -> Ctx {
    Ctx {
        layer,
        brush: Brush { ch: ch.map_or('█', |c| c.0), fg: col(fg, Color::LIGHT_GRAY), bg: col(bg, Color::BLACK) },
        mode,
        symmetry: Symmetry::None,
    }
}

fn box_style(s: Option<&str>) -> Result<BoxStyle> {
    Ok(match s.unwrap_or("single") {
        "single" => BoxStyle::Single,
        "double" => BoxStyle::Double,
        "double_h" => BoxStyle::DoubleH,
        "double_v" => BoxStyle::DoubleV,
        "block" => BoxStyle::Block,
        "rounded" => BoxStyle::Rounded,
        "brush" => BoxStyle::Brush,
        o => bail!("unknown box style {o:?} (single, double, double_h, double_v, block, rounded, brush)"),
    })
}

fn stamp_mode(s: Option<&str>, default: StampMode) -> Result<StampMode> {
    Ok(match s {
        None => default,
        Some("transparent") => StampMode::Transparent,
        Some("opaque") => StampMode::Opaque,
        Some("under") => StampMode::Under,
        Some(o) => bail!("unknown mode {o:?} (transparent, opaque, under)"),
    })
}

// ------------------------------------------------------------------ helpers

fn target_layer(state: &ExecState, l: Option<usize>) -> Result<usize> {
    let n = state.doc.canvas.layers.len();
    let l = l.unwrap_or(state.layer.min(n.saturating_sub(1)));
    let Some(layer) = state.doc.canvas.layers.get(l) else {
        bail!("layer {l} does not exist (the doc has {n} layers)")
    };
    if layer.locked {
        bail!("layer {l} ({}) is locked; unlock it with layer set_props", layer.name);
    }
    Ok(l)
}

/// Build one transaction, commit it as one undo step.
fn commit(
    doc: &mut Document,
    history: &mut History,
    tool: &str,
    build: impl FnOnce(&mut TxBuilder) -> Result<String>,
) -> Result<ToolResult> {
    let mut b = TxBuilder::new(doc, format!("AI: {tool}"));
    let msg = build(&mut b)?;
    let tx = b.finish();
    if tx.is_empty() {
        return Ok(ToolResult::ok(format!("{msg} (nothing changed)")));
    }
    let n = tx.cells.len();
    history.commit(doc, tx);
    Ok(ToolResult::ok(if n > 0 { format!("{msg} ({n} cells changed)") } else { msg }))
}

/// Clamp a region to the canvas; defaults to the whole canvas.
fn region(doc: &Document, r: RegionArgs, default_h: usize) -> Result<Rect> {
    let (w, h) = (doc.width(), doc.height());
    let x = r.x.unwrap_or(0);
    let y = r.y.unwrap_or(0);
    if x >= w || y >= h {
        bail!("region origin {x},{y} is outside the {w}x{h} canvas");
    }
    let rw = r.w.unwrap_or(w - x).min(w - x).max(1);
    let rh = r.h.unwrap_or(default_h.saturating_sub(y).max(1)).min(h - y).max(1);
    Ok(Rect::new(x, y, rw, rh))
}

fn rect_of(x: usize, y: usize, w: usize, h: usize) -> Result<Rect> {
    if w == 0 || h == 0 {
        bail!("w and h must be at least 1");
    }
    Ok(Rect::new(x, y, w, h))
}

fn region_grid(doc: &Document, layer: Option<usize>, r: Rect) -> Grid {
    let mut g = Grid::new(r.w, r.h);
    for y in 0..r.h {
        for x in 0..r.w {
            let c = match layer {
                Some(l) => doc.canvas.get(l, r.x + x, r.y + y).unwrap_or(Cell::BLANK),
                None => doc.canvas.composite(r.x + x, r.y + y),
            };
            g.set(x, y, c);
        }
    }
    g
}

fn color_name(c: Color, pal: &Palette) -> String {
    match c {
        Color::Pal(i) => i.to_string(),
        Color::Rgb(..) => {
            let [r, g, b] = c.rgb(pal);
            format!("#{r:02x}{g:02x}{b:02x}")
        }
    }
}

/// Text rows with a two-line column ruler (tens, ones) and row numbers.
pub fn grid_text(g: &Grid, x0: usize, y0: usize) -> String {
    let mut out = String::new();
    let tens: String =
        (x0..x0 + g.width).map(|x| if x % 10 == 0 { char::from(b'0' + ((x / 10) % 10) as u8) } else { ' ' }).collect();
    let ones: String = (x0..x0 + g.width).map(|x| char::from(b'0' + (x % 10) as u8)).collect();
    let _ = writeln!(out, "     {}", tens.trim_end());
    let _ = writeln!(out, "     {ones}");
    for y in 0..g.height {
        let row: String = g.row(y).iter().map(|c| if c.ch == '\u{0}' { ' ' } else { c.ch }).collect();
        let _ = writeln!(out, "{:>4}|{}", y0 + y, row.trim_end());
    }
    out
}

fn grid_ansi(g: &Grid, pal: &Palette) -> String {
    let mut out = String::new();
    for y in 0..g.height {
        let mut last: Option<([u8; 3], [u8; 3])> = None;
        for c in g.row(y) {
            let cur = (c.fg.rgb(pal), c.bg.rgb(pal));
            if last != Some(cur) {
                let ([fr, fg, fb], [br, bgc, bb]) = cur;
                let _ = write!(out, "\x1b[38;2;{fr};{fg};{fb};48;2;{br};{bgc};{bb}m");
                last = Some(cur);
            }
            out.push(if c.ch == '\u{0}' { ' ' } else { c.ch });
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

fn color_runs(g: &Grid, x0: usize, y0: usize, pal: &Palette) -> String {
    let mut out = String::from("color runs per row (x0-x1 fg/bg; default 7/0 omitted):\n");
    for y in 0..g.height {
        let row = g.row(y);
        let mut runs = vec![];
        let mut start = 0;
        for x in 1..=row.len() {
            if x == row.len() || (row[x].fg, row[x].bg) != (row[start].fg, row[start].bg) {
                let c = row[start];
                if (c.fg, c.bg) != (Color::LIGHT_GRAY, Color::BLACK) {
                    runs.push(format!(
                        "{}-{} {}/{}",
                        x0 + start,
                        x0 + x - 1,
                        color_name(c.fg, pal),
                        color_name(c.bg, pal)
                    ));
                }
                start = x;
            }
        }
        if !runs.is_empty() {
            let _ = writeln!(out, "{:>4}: {}", y0 + y, runs.join(", "));
        }
    }
    out
}

// ------------------------------------------------------------------ dispatch

fn run(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    let name = call.name.as_str();
    match name {
        "get_info" => get_info(state),
        "get_canvas" => get_canvas(state, call),
        "render_png" => render_png(state, call),
        "new_canvas" => new_canvas(state, call),
        "resize" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                width: usize,
                height: usize,
            }
            let a: A = parse(call)?;
            if a.width == 0 || a.height == 0 || a.width > 4000 || a.height > 10000 {
                bail!("size must be 1..4000 x 1..10000");
            }
            commit(state.doc, state.history, name, |b| {
                tools::resize(b, a.width, a.height);
                Ok(format!("Resized to {}x{}.", a.width, a.height))
            })
        }
        "set_cells" => set_cells(state, call),
        "put_text" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                text: String,
                fg: Option<Col>,
                bg: Option<Col>,
                #[serde(default)]
                transparent_spaces: bool,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let c = ctx(target_layer(state, a.layer)?, None, a.fg, a.bg, PaintMode::Char);
            commit(state.doc, state.history, name, |b| {
                let (w, h) = tools::put_text(b, &c, a.x, a.y, &a.text, a.transparent_spaces);
                Ok(format!("Wrote {w}x{h} text at {},{}.", a.x, a.y))
            })
        }
        "fill_rect" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                w: usize,
                h: usize,
                ch: Option<Ch>,
                fg: Option<Col>,
                bg: Option<Col>,
                what: Option<String>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let what = match a.what.as_deref().unwrap_or("all") {
                "all" => FillWhat::All,
                "char" => FillWhat::Char,
                "fg" => FillWhat::Fg,
                "bg" => FillWhat::Bg,
                "colors" => FillWhat::Colors,
                o => bail!("unknown what {o:?} (all, char, fg, bg, colors)"),
            };
            let c = ctx(target_layer(state, a.layer)?, a.ch, a.fg, a.bg, PaintMode::Char);
            let r = rect_of(a.x, a.y, a.w, a.h)?;
            commit(state.doc, state.history, name, |b| {
                tools::fill_rect(b, &c, r, what);
                Ok(format!("Filled {}x{} at {},{}.", a.w, a.h, a.x, a.y))
            })
        }
        "draw_box" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                w: usize,
                h: usize,
                style: Option<String>,
                #[serde(default)]
                filled: bool,
                ch: Option<Ch>,
                fg: Option<Col>,
                bg: Option<Col>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let style = box_style(a.style.as_deref())?;
            let c = ctx(target_layer(state, a.layer)?, a.ch, a.fg, a.bg, PaintMode::Char);
            let r = rect_of(a.x, a.y, a.w, a.h)?;
            let fill = if a.filled { ShapeFill::Filled } else { ShapeFill::Outline };
            commit(state.doc, state.history, name, |b| {
                tools::rect(b, &c, r, fill, style);
                Ok(format!("Drew {}x{} box at {},{}.", a.w, a.h, a.x, a.y))
            })
        }
        "draw_line" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x0: usize,
                y0: usize,
                x1: usize,
                y1: usize,
                ch: Option<Ch>,
                fg: Option<Col>,
                bg: Option<Col>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let c = ctx(target_layer(state, a.layer)?, a.ch, a.fg, a.bg, PaintMode::Char);
            commit(state.doc, state.history, name, |b| {
                tools::paint_line(b, &c, a.x0, a.y0, a.x1, a.y1);
                Ok(format!("Line {},{} -> {},{}.", a.x0, a.y0, a.x1, a.y1))
            })
        }
        "draw_ellipse" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                w: usize,
                h: usize,
                #[serde(default)]
                filled: bool,
                ch: Option<Ch>,
                fg: Option<Col>,
                bg: Option<Col>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let c = ctx(target_layer(state, a.layer)?, a.ch, a.fg, a.bg, PaintMode::Char);
            let r = rect_of(a.x, a.y, a.w, a.h)?;
            let fill = if a.filled { ShapeFill::Filled } else { ShapeFill::Outline };
            commit(state.doc, state.history, name, |b| {
                tools::ellipse(b, &c, r, fill);
                Ok(format!("Ellipse in {}x{} at {},{}.", a.w, a.h, a.x, a.y))
            })
        }
        "flood_fill" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct M {
                ch: Option<bool>,
                fg: Option<bool>,
                bg: Option<bool>,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                ch: Option<Ch>,
                fg: Option<Col>,
                bg: Option<Col>,
                mode: Option<String>,
                #[serde(rename = "match")]
                matching: Option<M>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let mode = match a.mode.as_deref().unwrap_or("all") {
                "all" => PaintMode::Char,
                "colors" => PaintMode::Color,
                "fg" => PaintMode::Fg,
                "bg" => PaintMode::Bg,
                o => bail!("unknown mode {o:?} (all, colors, fg, bg)"),
            };
            let m = a.matching.map_or(FillMatch::default(), |m| FillMatch {
                ch: m.ch.unwrap_or(true),
                fg: m.fg.unwrap_or(true),
                bg: m.bg.unwrap_or(true),
            });
            let c = ctx(target_layer(state, a.layer)?, a.ch, a.fg, a.bg, mode);
            commit(state.doc, state.history, name, |b| {
                tools::flood_fill(b, &c, a.x, a.y, m);
                Ok(format!("Flood filled from {},{}.", a.x, a.y))
            })
        }
        "erase_rect" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                w: usize,
                h: usize,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            let r = rect_of(a.x, a.y, a.w, a.h)?;
            commit(state.doc, state.history, name, |b| {
                tools::erase(b, l, r);
                Ok(format!("Erased {}x{} at {},{} on layer {l}.", a.w, a.h, a.x, a.y))
            })
        }
        "pixel_set" => pixel_set(state, call),
        "pixel_line" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x0: i64,
                y0: i64,
                x1: i64,
                y1: i64,
                color: Col,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            commit(state.doc, state.history, name, |b| {
                tools::pixel::line(b, l, a.x0, a.y0, a.x1, a.y1, a.color.0);
                Ok(format!("Pixel line {},{} -> {},{}.", a.x0, a.y0, a.x1, a.y1))
            })
        }
        "pixel_rect" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                w: usize,
                h: usize,
                color: Col,
                color2: Option<Col>,
                mix: Option<f32>,
                filled: Option<bool>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            rect_of(a.x, a.y, a.w, a.h)?;
            commit(state.doc, state.history, name, |b| {
                match a.color2 {
                    Some(c2) => {
                        let mix = a.mix.unwrap_or(0.5).clamp(0.0, 1.0);
                        tools::pixel::dither_rect(b, l, a.x, a.y, a.w, a.h, a.color.0, c2.0, mix);
                    }
                    None => tools::pixel::rect(b, l, a.x, a.y, a.w, a.h, a.color.0, a.filled.unwrap_or(true)),
                }
                Ok(format!("Pixel rect {}x{} at {},{}.", a.w, a.h, a.x, a.y))
            })
        }
        "pixel_ellipse" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                cx: i64,
                cy: i64,
                rx: i64,
                ry: i64,
                color: Col,
                filled: Option<bool>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            commit(state.doc, state.history, name, |b| {
                tools::pixel::ellipse(b, l, a.cx, a.cy, a.rx, a.ry, a.color.0, a.filled.unwrap_or(true));
                Ok(format!("Pixel ellipse at {},{} r {}x{}.", a.cx, a.cy, a.rx, a.ry))
            })
        }
        "brush_stroke" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                points: Vec<[f32; 2]>,
                brush: Option<String>,
                size: Option<f32>,
                fg: Option<Col>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let want = a.brush.as_deref().unwrap_or("ink");
            let Some(mut spec) = tools::brush::presets().into_iter().find(|b| b.name.eq_ignore_ascii_case(want)) else {
                bail!(
                    "unknown brush {want:?} (ink, brush pen, marker, calligraphy, airbrush, soft shade, chalk, spray, ascii, line art)"
                );
            };
            if let Some(s) = a.size {
                spec.size = s;
            }
            let spec = spec.sanitized();
            if a.points.is_empty() {
                bail!("points must hold at least one [x, y]");
            }
            let c = ctx(
                target_layer(state, a.layer)?,
                None,
                Some(a.fg.unwrap_or(Col(Color::WHITE))),
                None,
                PaintMode::Char,
            );
            let cands =
                spec.glyphs.candidates(&acidtrip_core::charsets::builtin()[acidtrip_core::charsets::DEFAULT_SET].chars);
            // Pixel space, sampled like mouse reports (~3px apart) so the
            // brush dynamics (speed, spacing) behave as when drawn by hand.
            let px: Vec<(f32, f32)> = a.points.iter().map(|p| (p[0] * 8.0, p[1] * 16.0)).collect();
            let mut path = vec![px[0]];
            for w in px.windows(2) {
                let (p, q) = (w[0], w[1]);
                let n = ((q.0 - p.0).hypot(q.1 - p.1) / 3.0).ceil().max(1.0) as usize;
                path.extend(
                    (1..=n).map(|i| i as f32 / n as f32).map(|t| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t)),
                );
            }
            commit(state.doc, state.history, name, |b| {
                let mut s = tools::pen::PenStroke::with_brush(&spec);
                for &(x, y) in &path {
                    let changed = s.add_point(x, y);
                    tools::pen::apply(b, &c, &s, changed, &cands);
                }
                let changed = s.finish();
                tools::pen::apply(b, &c, &s, changed, &cands);
                Ok(format!("{} stroke through {} points.", spec.name, a.points.len()))
            })
        }
        "pixel_fill" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                color: Col,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            commit(state.doc, state.history, name, |b| {
                tools::pixel::flood_fill(b, l, a.x, a.y, a.color.0);
                Ok(format!("Pixel fill from {},{}.", a.x, a.y))
            })
        }
        "banner" => banner(state, call),
        "list_fonts" => list_fonts(state, call),
        "list_stencils" => list_stencils(state, call),
        "stamp_stencil" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: String,
                x: usize,
                y: usize,
                mode: Option<String>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            let mode = stamp_mode(a.mode.as_deref(), StampMode::Transparent)?;
            let st = state.stencils.get(&a.id).ok_or_else(|| anyhow!("no stencil {:?}; use list_stencils", a.id))?;
            commit(state.doc, state.history, name, |b| {
                tools::stamp(b, l, &st.clip, a.x, a.y, mode);
                Ok(format!("Stamped {} ({}x{}) at {},{}.", st.meta.name, st.clip.width, st.clip.height, a.x, a.y))
            })
        }
        "save_stencil" => save_stencil(state, call),
        "transform_region" => transform_region(state, call),
        "move_region" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                x: usize,
                y: usize,
                w: usize,
                h: usize,
                to_x: usize,
                to_y: usize,
                #[serde(default)]
                copy: bool,
                mode: Option<String>,
                layer: Option<usize>,
            }
            let a: A = parse(call)?;
            let l = target_layer(state, a.layer)?;
            let r = rect_of(a.x, a.y, a.w, a.h)?;
            let mode = stamp_mode(a.mode.as_deref(), StampMode::Opaque)?;
            commit(state.doc, state.history, name, |b| {
                let clip = tools::copy(b, Some(l), r);
                if !a.copy {
                    tools::erase(b, l, r);
                }
                tools::stamp(b, l, &clip, a.to_x, a.to_y, mode);
                Ok(format!(
                    "{} {}x{} from {},{} to {},{}.",
                    if a.copy { "Copied" } else { "Moved" },
                    a.w,
                    a.h,
                    a.x,
                    a.y,
                    a.to_x,
                    a.to_y
                ))
            })
        }
        "layer" => layer_op(state, call),
        "import_image" => import_image(state, call),
        "undo" | "redo" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                steps: Option<usize>,
            }
            let a: A = parse(call)?;
            let mut done = vec![];
            for _ in 0..a.steps.unwrap_or(1).max(1) {
                let label = if name == "undo" { state.history.undo(state.doc) } else { state.history.redo(state.doc) };
                match label {
                    Some(l) => done.push(l),
                    None => break,
                }
            }
            let n = state.doc.canvas.layers.len();
            state.layer = state.layer.min(n - 1);
            if done.is_empty() {
                return Ok(ToolResult::ok(format!("Nothing to {name}.")));
            }
            Ok(ToolResult::ok(format!("{}: {}", if name == "undo" { "Undid" } else { "Redid" }, done.join(", "))))
        }
        "save" => save(state, call),
        "load" => load(state, call),
        "harvest_candidates" | "harvest_commit" => {
            bail!("{name} is only available through the MCP server")
        }
        _ => bail!(
            "unknown tool; available: {}",
            schema::tool_definitions().iter().filter_map(|t| t["name"].as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

// ------------------------------------------------------------------ tools

fn get_info(state: &ExecState) -> Result<ToolResult> {
    let d = &*state.doc;
    let pal = &d.meta.palette;
    let layers: Vec<Value> = d
        .canvas
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({
                "index": i, "name": l.name, "visible": l.visible, "locked": l.locked,
                "reference": l.kind == acidtrip_core::LayerKind::Reference, "empty": l.is_empty(),
                "ai_target": i == state.layer,
            })
        })
        .collect();
    let palette: Vec<String> = (0..pal.len().min(16))
        .map(|i| {
            let [r, g, b] = pal.get(i as u8);
            format!("{i} {} #{r:02x}{g:02x}{b:02x}", VGA_NAMES.get(i).copied().unwrap_or(""))
        })
        .collect();
    let info = json!({
        "kind": if d.is_classic() { "classic (CP437 glyphs, 16-color palette)" } else { "modern (Unicode, RGB)" },
        "width": d.width(),
        "height": d.height(),
        "used_rows": d.canvas.used_height(),
        "ice": d.meta.ice,
        "font": d.meta.font_name,
        "layers": layers,
        "ai_layer": state.layer,
        "palette": palette,
        "sauce": { "title": d.meta.sauce.title, "author": d.meta.sauce.author, "group": d.meta.sauce.group },
        "file": state.file.as_ref().map(|p| p.display().to_string()),
        "undo_steps": state.history.len(),
    });
    Ok(ToolResult::ok(serde_json::to_string_pretty(&info)?))
}

fn get_canvas(state: &ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        x: Option<usize>,
        y: Option<usize>,
        w: Option<usize>,
        h: Option<usize>,
        format: Option<String>,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let ra = RegionArgs { x: a.x, y: a.y, w: a.w, h: a.h };
    let d = &*state.doc;
    if let Some(l) = a.layer
        && l >= d.canvas.layers.len()
    {
        bail!("layer {l} does not exist");
    }
    let r = region(d, ra, d.canvas.used_height().max(1))?;
    let g = region_grid(d, a.layer, r);
    let text = match a.format.as_deref().unwrap_or("text") {
        "text" => grid_text(&g, r.x, r.y),
        "colors" => format!("{}{}", grid_text(&g, r.x, r.y), color_runs(&g, r.x, r.y, &d.meta.palette)),
        "ansi" => grid_ansi(&g, &d.meta.palette),
        o => bail!("unknown format {o:?} (text, colors, ansi)"),
    };
    Ok(ToolResult::ok(format!(
        "Region x={} y={} w={} h={} of {}x{}:\n{text}",
        r.x,
        r.y,
        r.w,
        r.h,
        d.width(),
        d.height()
    )))
}

/// Rows rendered without an explicit region: the whole canvas up to 60
/// rows, else the used area (25..200 rows).
fn default_rows(doc: &Document) -> usize {
    let h = doc.height();
    if h <= 60 { h.max(1) } else { doc.canvas.used_height().clamp(25, 200).min(h) }
}

/// Render the document (or a region) to PNG bytes. Without a region, renders
/// the full canvas when it is at most 60 rows, else the used area (max 200).
pub fn render_region_png(doc: &Document, r: Option<Rect>, scale: u32) -> Vec<u8> {
    let r = r.unwrap_or_else(|| Rect::new(0, 0, doc.width(), default_rows(doc)));
    let g = region_grid(doc, None, r);
    let img = render_grid(
        &g,
        &doc.meta.palette,
        RenderOptions { scale: scale.clamp(1, 4), nine_px: doc.meta.letter_spacing_9px },
    );
    png_bytes(&img)
}

fn render_png(state: &ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        x: Option<usize>,
        y: Option<usize>,
        w: Option<usize>,
        h: Option<usize>,
        scale: Option<u32>,
        path: Option<PathBuf>,
    }
    let a: A = parse(call)?;
    let ra = RegionArgs { x: a.x, y: a.y, w: a.w, h: a.h };
    let d = &*state.doc;
    let any = a.x.is_some() || a.y.is_some() || a.w.is_some() || a.h.is_some();
    let r = if any { Some(region(d, ra, d.height())?) } else { None };
    let scale = a.scale.unwrap_or(1).clamp(1, 4);
    let png = render_region_png(d, r, scale);
    let (rw, rh) = r.map_or_else(|| (d.width(), default_rows(d)), |r| (r.w, r.h));
    let mut text = format!(
        "Rendered {rw}x{rh} cells from {},{} ({}x{} px, scale {scale}; each cell is {}x{} px). Canvas is {}x{}, {} rows used.",
        r.map_or(0, |r| r.x),
        r.map_or(0, |r| r.y),
        rw as u32 * 8 * scale,
        rh as u32 * 16 * scale,
        8 * scale,
        16 * scale,
        d.width(),
        d.height(),
        d.canvas.used_height()
    );
    if let Some(p) = a.path {
        std::fs::write(&p, &png).with_context(|| format!("writing {}", p.display()))?;
        let _ = write!(text, " Saved to {}.", p.display());
    }
    Ok(ToolResult { text, image_png: Some(png), is_error: false })
}

fn new_canvas(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        width: Option<usize>,
        height: Option<usize>,
        kind: Option<String>,
        ice: Option<bool>,
    }
    let a: A = parse(call)?;
    let (w, h) = (a.width.unwrap_or(80), a.height.unwrap_or(25));
    if w == 0 || h == 0 || w > 4000 || h > 10000 {
        bail!("size must be 1..4000 x 1..10000");
    }
    let kind = match a.kind.as_deref().unwrap_or("classic") {
        "classic" => DocKind::Classic,
        "modern" => DocKind::Modern,
        o => bail!("unknown kind {o:?} (classic, modern)"),
    };
    let ice = a.ice.unwrap_or(true);
    let r = commit(state.doc, state.history, "new_canvas", |b| {
        b.replace_meta(|m| {
            m.kind = kind;
            m.ice = ice;
            m.palette = Palette::default();
            m.sauce = Default::default();
        });
        b.keep_only_current_frame();
        b.replace_canvas(|c| {
            let mut n = Canvas::new(w, h);
            n.layers = c
                .layers
                .iter()
                .enumerate()
                .map(|(i, l)| Layer { cells: vec![(i == 0).then_some(Cell::BLANK); w * h], ..l.clone() })
                .collect();
            n
        });
        Ok(format!(
            "New {w}x{h} {} canvas{}.",
            if kind == DocKind::Classic { "classic" } else { "modern" },
            if ice { " (iCE)" } else { "" }
        ))
    })?;
    *state.file = None;
    Ok(r)
}

fn set_cells(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct C {
        x: usize,
        y: usize,
        ch: Option<Ch>,
        fg: Option<Col>,
        bg: Option<Col>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        cells: Vec<C>,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let l = target_layer(state, a.layer)?;
    let (w, h) = (state.doc.width(), state.doc.height());
    let outside = a.cells.iter().filter(|c| c.x >= w || c.y >= h).count();
    let mut r = commit(state.doc, state.history, "set_cells", |b| {
        for c in &a.cells {
            let base = b.get(l, c.x, c.y).unwrap_or_else(|| b.composite(c.x, c.y));
            let cell =
                Cell::new(c.ch.map_or(base.ch, |x| x.0), c.fg.map_or(base.fg, |x| x.0), c.bg.map_or(base.bg, |x| x.0));
            b.set(l, c.x, c.y, Some(cell));
        }
        Ok(format!("Set {} cells.", a.cells.len() - outside))
    })?;
    if outside > 0 {
        let _ = write!(r.text, " {outside} were outside the {w}x{h} canvas and ignored.");
    }
    Ok(r)
}

fn pixel_set(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        pixels: Vec<Value>,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let l = target_layer(state, a.layer)?;
    let mut pts = Vec::with_capacity(a.pixels.len());
    for p in &a.pixels {
        let (x, y, c) = match p {
            Value::Array(t) if t.len() == 3 => (t[0].as_u64(), t[1].as_u64(), &t[2]),
            Value::Object(o) => (
                o.get("x").and_then(Value::as_u64),
                o.get("y").and_then(Value::as_u64),
                o.get("color").unwrap_or(&Value::Null),
            ),
            _ => bail!("each pixel must be [x, y, color] or {{x, y, color}}, got {p}"),
        };
        let (Some(x), Some(y)) = (x, y) else { bail!("pixel {p}: x and y must be non-negative integers") };
        let c = parse_color(c).map_err(|e| anyhow!("pixel {p}: {e}"))?;
        pts.push((x as usize, y as usize, c));
    }
    commit(state.doc, state.history, "pixel_set", |b| {
        for &(x, y, c) in &pts {
            tools::pixel::set(b, l, x, y, c);
        }
        Ok(format!("Set {} pixels.", pts.len()))
    })
}

fn banner(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        text: String,
        font: String,
        x: Option<usize>,
        y: Option<usize>,
        #[serde(default)]
        center: bool,
        outline_style: Option<usize>,
        spacing: Option<i32>,
        fg: Option<Col>,
        bg: Option<Col>,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let l = target_layer(state, a.layer)?;
    let info = state.fonts.find(&a.font).ok_or_else(|| anyhow!("no font {:?}; call list_fonts to see ids", a.font))?;
    let opts = TextRenderOptions {
        outline_style: a.outline_style.unwrap_or(0).min(18),
        fg: col(a.fg, Color::WHITE),
        bg: col(a.bg, Color::BLACK),
        spacing: a.spacing.unwrap_or(0),
    };
    let clip = state.fonts.render(&info.id, &a.text, &opts)?;
    let cw = state.doc.width();
    let x = if a.center { cw.saturating_sub(clip.width) / 2 } else { a.x.unwrap_or(0) };
    let y = a.y.unwrap_or(0);
    let mut r = commit(state.doc, state.history, "banner", |b| {
        tools::stamp(b, l, &clip, x, y, StampMode::Transparent);
        Ok(format!(
            "Stamped {:?} in {} ({}) at {x},{y}: {}x{} cells.",
            a.text, info.name, info.id, clip.width, clip.height
        ))
    })?;
    if x + clip.width > cw {
        let _ = write!(
            r.text,
            " Warning: it is wider than the {cw}-column canvas and was clipped; use a smaller font or shorter text."
        );
    }
    Ok(r)
}

fn list_fonts(state: &ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        filter: Option<String>,
        text: Option<String>,
    }
    let a: A = parse(call)?;
    let f = a.filter.unwrap_or_default().to_lowercase();
    let fonts: Vec<_> = state
        .fonts
        .list()
        .into_iter()
        .filter(|i| f.is_empty() || format!("{} {} {:?}", i.id, i.name, i.kind).to_lowercase().contains(&f))
        .collect();
    let total = fonts.len();
    let measure = a.text.as_deref().filter(|_| total <= 30);
    let mut out = format!("{total} fonts{}:\n", if f.is_empty() { String::new() } else { format!(" matching {f:?}") });
    for i in fonts.iter().take(150) {
        let charset: String = i.charset.chars().take(48).collect();
        let _ = write!(out, "{} | {} | {:?} | chars: {charset}", i.id, i.name, i.kind);
        if let Some(t) = measure {
            match state.fonts.render(&i.id, t, &TextRenderOptions::default()) {
                Ok(c) => {
                    let _ = write!(out, " | {t:?} = {}x{}", c.width, c.height);
                }
                Err(e) => {
                    let _ = write!(out, " | can't render: {e}");
                }
            }
        }
        out.push('\n');
    }
    if total > 150 {
        let _ = writeln!(out, "... {} more; narrow with filter", total - 150);
    }
    Ok(ToolResult::ok(out))
}

fn list_stencils(state: &ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        query: Option<String>,
    }
    let a: A = parse(call)?;
    let q = a.query.unwrap_or_default();
    let list = if q.trim().is_empty() { state.stencils.list() } else { state.stencils.search(&q) };
    let mut out = format!("{} stencils:\n", list.len());
    for m in list.iter().take(60) {
        let size = state.stencils.get(&m.id).map(|s| format!("{}x{}", s.clip.width, s.clip.height)).unwrap_or_default();
        let _ = writeln!(
            out,
            "{} | {} | {size} | tags: {} | by {} {} | {}",
            m.id,
            m.name,
            m.tags.join(","),
            m.author,
            m.group,
            m.source
        );
    }
    Ok(ToolResult::ok(out))
}

fn save_stencil(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        name: String,
        #[serde(default)]
        tags: Vec<String>,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let r = rect_of(a.x, a.y, a.w, a.h)?;
    if let Some(l) = a.layer
        && l >= state.doc.canvas.layers.len()
    {
        bail!("layer {l} does not exist");
    }
    let clip = tools::copy(&TxBuilder::new(state.doc, ""), a.layer, r);
    let sauce = &state.doc.meta.sauce;
    let meta = StencilMeta {
        name: a.name,
        tags: a.tags,
        author: sauce.author.clone(),
        group: sauce.group.clone(),
        source: state.file.as_ref().map_or_else(|| "acidtrip (AI)".into(), |p| p.display().to_string()),
        created: chrono::Local::now().to_rfc3339(),
        ..Default::default()
    };
    let m = state.stencils.save(&state.paths.stencils_dir(), Stencil { meta, clip })?;
    Ok(ToolResult::ok(format!("Saved stencil {} ({}), {}x{}.", m.id, m.name, a.w, a.h)))
}

fn transform_region(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        op: String,
        style: Option<String>,
        fg: Option<Col>,
        bg: Option<Col>,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let l = target_layer(state, a.layer)?;
    let r = rect_of(a.x, a.y, a.w, a.h)?;
    let style = box_style(a.style.as_deref())?;
    let c = ctx(l, None, a.fg, a.bg, PaintMode::Char);
    let op = a.op.clone();
    commit(state.doc, state.history, "transform_region", |b| {
        match op.as_str() {
            "flip_x" | "flip_y" | "rotate_180" => {
                let clip = tools::copy(b, Some(l), r);
                let t = match op.as_str() {
                    "flip_x" => tools::flip_x(&clip, true),
                    "flip_y" => tools::flip_y(&clip, true),
                    _ => tools::rotate_180(&clip),
                };
                tools::stamp(b, l, &t, r.x, r.y, StampMode::Opaque);
            }
            "justify_left" | "left" => tools::justify(b, l, r, Justify::Left),
            "justify_center" | "center" => tools::justify(b, l, r, Justify::Center),
            "justify_right" | "right" => tools::justify(b, l, r, Justify::Right),
            "outline" => tools::outline(b, &c, r, style),
            o => bail!(
                "unknown op {o:?} (flip_x, flip_y, rotate_180, justify_left, justify_center, justify_right, outline)"
            ),
        }
        Ok(format!("{op} on {}x{} at {},{}.", a.w, a.h, a.x, a.y))
    })
}

fn layer_op(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        action: String,
        index: Option<usize>,
        name: Option<String>,
        visible: Option<bool>,
        locked: Option<bool>,
        reference: Option<bool>,
    }
    let a: A = parse(call)?;
    let n = state.doc.canvas.layers.len();
    let need = |i: Option<usize>| -> Result<usize> {
        let i = i.ok_or_else(|| anyhow!("{} needs index", a.action))?;
        if i >= n {
            bail!("layer {i} does not exist (the doc has {n} layers)");
        }
        Ok(i)
    };
    match a.action.as_str() {
        "list" => {
            let mut out = String::new();
            for (i, l) in state.doc.canvas.layers.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "{i}: {}{}{}{}{}{}",
                    l.name,
                    if l.visible { "" } else { " [hidden]" },
                    if l.locked { " [locked]" } else { "" },
                    if l.kind == acidtrip_core::LayerKind::Reference { " [reference]" } else { "" },
                    if l.is_empty() { " [empty]" } else { "" },
                    if i == state.layer { " <- AI edits go here" } else { "" }
                );
            }
            Ok(ToolResult::ok(out))
        }
        "add" => {
            let name = a.name.clone().unwrap_or_else(|| format!("Layer {}", n + 1));
            let at = a.index.unwrap_or(n).min(n);
            let mut idx = at;
            let r = commit(state.doc, state.history, "layer add", |b| {
                idx = tools::add_layer(b, &name, at);
                Ok(format!("Added layer {name:?}"))
            })?;
            state.layer = idx;
            Ok(ToolResult::ok(format!("{} at index {idx}; it is now the edit target.", r.text)))
        }
        "select" => {
            state.layer = need(a.index)?;
            Ok(ToolResult::ok(format!(
                "Edits now go to layer {} ({}).",
                state.layer, state.doc.canvas.layers[state.layer].name
            )))
        }
        "set_props" => {
            let i = need(a.index)?;
            let props =
                LayerProps { name: a.name.clone(), visible: a.visible, locked: a.locked, reference: a.reference };
            commit(state.doc, state.history, "layer set_props", |b| {
                tools::set_layer_props(b, i, &props);
                Ok(format!("Updated layer {i}."))
            })
        }
        "merge_down" => {
            let i = need(a.index.or(Some(state.layer)))?;
            if i == 0 {
                bail!("layer 0 has nothing below it");
            }
            let r = commit(state.doc, state.history, "layer merge_down", |b| {
                tools::merge_down(b, i);
                Ok(format!("Merged layer {i} into {}.", i - 1))
            })?;
            if state.layer >= i {
                state.layer = state.layer.saturating_sub(1);
            }
            Ok(r)
        }
        "remove" => {
            let i = need(a.index)?;
            if n == 1 {
                bail!("can't remove the only layer");
            }
            let r = commit(state.doc, state.history, "layer remove", |b| {
                tools::remove_layer(b, i);
                Ok(format!("Removed layer {i}."))
            })?;
            if state.layer > i || state.layer >= n - 1 {
                state.layer = state.layer.saturating_sub(1);
            }
            Ok(r)
        }
        o => bail!("unknown action {o:?} (list, add, select, set_props, merge_down, remove)"),
    }
}

fn import_image(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        path: Option<PathBuf>,
        png_base64: Option<String>,
        x: Option<usize>,
        y: Option<usize>,
        width: Option<usize>,
        preset: Option<String>,
        style: Option<String>,
        #[serde(default)]
        dither: bool,
        #[serde(default)]
        ink: bool,
        layer: Option<usize>,
    }
    let a: A = parse(call)?;
    let l = target_layer(state, a.layer)?;
    let bytes = match (&a.path, &a.png_base64) {
        (Some(p), _) => std::fs::read(p).with_context(|| format!("reading {}", p.display()))?,
        (None, Some(b)) => {
            base64::engine::general_purpose::STANDARD.decode(b.trim()).context("png_base64 is not valid base64")?
        }
        _ => bail!("give path or png_base64"),
    };
    let (x, y) = (a.x.unwrap_or(0), a.y.unwrap_or(0));
    let cw = state.doc.width();
    if x >= cw {
        bail!("x {x} is outside the {cw}-column canvas");
    }
    let style = match a.style.as_deref().unwrap_or("halfblock") {
        "halfblock" => ImportStyle::HalfBlock,
        "blocks" => ImportStyle::Blocks,
        "ascii" => ImportStyle::Ascii,
        o => bail!("unknown style {o:?} (halfblock, blocks, ascii)"),
    };
    let base = ImportOptions {
        width: a.width.unwrap_or(cw - x).max(1),
        style,
        kind: state.doc.meta.kind,
        dither: if a.dither { Dither::Diffuse } else { Dither::None },
        ink: a.ink,
        palette: Some(state.doc.meta.palette.clone()),
        ..ImportOptions::default()
    };
    let opts = match a.preset.as_deref() {
        None => base,
        Some(p) => {
            let preset = match p {
                "photo" => Preset::Photo,
                "scene" => Preset::Scene,
                "pixel_art" => Preset::PixelArt,
                "cel" => Preset::Cel,
                "comic" => Preset::Comic,
                "line_art" => Preset::LineArt,
                "ascii" => Preset::Ascii,
                o => bail!("unknown preset {o:?} (photo, scene, pixel_art, cel, comic, line_art, ascii)"),
            };
            ImportOptions { ink: a.ink || preset == Preset::LineArt, ..preset.apply(&base) }
        }
    };
    let clip = acidtrip_io::import::image_to_clip_ice(&bytes, &opts, state.doc.meta.ice)?;
    let ch = state.doc.height();
    commit(state.doc, state.history, "import_image", |b| {
        if y + clip.height > ch {
            tools::resize(b, cw, y + clip.height);
        }
        tools::stamp(b, l, &clip, x, y, StampMode::Opaque);
        Ok(format!("Imported image as {}x{} cells at {x},{y}. Check it with render_png.", clip.width, clip.height))
    })
}

fn save(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        path: Option<PathBuf>,
        title: Option<String>,
        author: Option<String>,
        group: Option<String>,
    }
    let a: A = parse(call)?;
    let mut path = a
        .path
        .clone()
        .or_else(|| state.file.clone())
        .ok_or_else(|| anyhow!("no path given and the document has never been saved"))?;
    if path.extension().is_none() {
        path.set_extension("ans");
    }
    let fmt = Format::from_path(&path)
        .ok_or_else(|| anyhow!("unknown file extension for {}; try .ans, .acid, .xb, .png", path.display()))?;
    let sauce_given = a.title.is_some() || a.author.is_some() || a.group.is_some();
    let mut note = String::new();
    if sauce_given {
        let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        commit(state.doc, state.history, "save (SAUCE)", |b| {
            b.replace_meta(|m| {
                if let Some(t) = &a.title {
                    m.sauce.title = cut(t, 35);
                }
                if let Some(t) = &a.author {
                    m.sauce.author = cut(t, 20);
                }
                if let Some(t) = &a.group {
                    m.sauce.group = cut(t, 20);
                }
                m.sauce.attach = true;
            });
            Ok(String::new())
        })?;
        note.push_str(" SAUCE attached.");
    }
    let opts = SaveOptions { sauce: sauce_given.then_some(true), ..SaveOptions::default() };
    format::save(state.doc, &path, fmt, &opts).with_context(|| format!("saving {}", path.display()))?;
    let warnings = format::loss_warnings(state.doc, fmt);
    if fmt.can_load() && !matches!(fmt, Format::Png | Format::Gif) {
        state.history.mark_saved();
        *state.file = Some(path.clone());
    }
    if !warnings.is_empty() {
        let _ = write!(note, " Note: {}", warnings.join("; "));
    }
    Ok(ToolResult::ok(format!("Saved {} as {}.{note}", path.display(), fmt.name())))
}

fn load(state: &mut ExecState, call: &ToolCall) -> Result<ToolResult> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct A {
        path: PathBuf,
    }
    let a: A = parse(call)?;
    let loaded = format::load(&a.path).with_context(|| format!("loading {}", a.path.display()))?;
    let had_ai = state.doc.canvas.layers.get(state.layer).is_some_and(|l| l.name == "AI")
        && !loaded.canvas.layers.iter().any(|l| l.name == "AI");
    let (w, h) = (loaded.width(), loaded.height());
    let r = commit(state.doc, state.history, "load", |b| {
        b.replace_meta(|m| *m = loaded.meta.clone());
        b.keep_only_current_frame();
        b.replace_canvas(|_| {
            let mut c = loaded.canvas.clone();
            if had_ai {
                c.layers.push(Layer::new("AI", w, h));
            }
            c
        });
        Ok(format!("Loaded {} ({w}x{h}, {} rows used).", a.path.display(), loaded.canvas.used_height()))
    })?;
    let n = state.doc.canvas.layers.len();
    state.layer = if had_ai { n - 1 } else { state.layer.min(n - 1) };
    *state.file = Some(a.path.clone());
    state.history.mark_saved();
    let s = &state.doc.meta.sauce;
    let credit = if s.title.is_empty() && s.author.is_empty() {
        String::new()
    } else {
        format!(" SAUCE: {:?} by {} / {}.", s.title, s.author, s.group)
    };
    Ok(ToolResult::ok(format!("{}{credit}", r.text)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct Env {
        pub doc: Document,
        pub history: History,
        pub fonts: FontLibrary,
        pub stencils: StencilLibrary,
        pub paths: Paths,
        pub file: Option<PathBuf>,
        pub layer: usize,
        pub dir: tempfile::TempDir,
    }

    impl Env {
        pub fn new() -> Env {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths {
                config_dir: dir.path().join("c"),
                data_dir: dir.path().join("d"),
                state_dir: dir.path().join("s"),
            };
            std::fs::create_dir_all(paths.stencils_dir()).unwrap();
            std::fs::create_dir_all(paths.fonts_dir()).unwrap();
            let stencils = StencilLibrary::load(&paths.stencils_dir());
            let fonts = FontLibrary::load(Some(&paths.fonts_dir()));
            Env {
                doc: Document::new(DocKind::Classic, 20, 10),
                history: History::new(),
                fonts,
                stencils,
                paths,
                file: None,
                layer: 0,
                dir,
            }
        }

        pub fn call(&mut self, name: &str, args: Value) -> ToolResult {
            let mut st = ExecState {
                doc: &mut self.doc,
                history: &mut self.history,
                layer: self.layer,
                fonts: &self.fonts,
                stencils: &mut self.stencils,
                paths: &self.paths,
                file: &mut self.file,
            };
            let r = execute(&mut st, &ToolCall { name: name.into(), args });
            self.layer = st.layer;
            r
        }

        fn cell(&self, x: usize, y: usize) -> Cell {
            self.doc.canvas.composite(x, y)
        }

        /// Run a mutating call: one undo step, undo restores, redo reapplies.
        fn one_step(&mut self, name: &str, args: Value) -> ToolResult {
            let before = self.doc.clone();
            let n = self.history.len();
            let r = self.call(name, args);
            assert!(!r.is_error, "{name}: {}", r.text);
            assert_eq!(self.history.len(), n + 1, "{name} must add exactly one undo step: {}", r.text);
            assert!(self.history.undo_label().unwrap().starts_with(&format!("AI: {name}")));
            let after = self.doc.clone();
            assert_ne!(before, after, "{name} changed nothing");
            self.history.undo(&mut self.doc);
            assert_eq!(self.doc, before, "{name}: undo must restore the doc");
            self.history.redo(&mut self.doc);
            assert_eq!(self.doc, after);
            r
        }
    }

    fn red() -> Color {
        Color::Pal(4)
    }

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color(&json!(12)), Ok(Color::Pal(12)));
        assert_eq!(parse_color(&json!("yellow")), Ok(Color::Pal(14)));
        assert_eq!(parse_color(&json!("light gray")), Ok(Color::Pal(7)));
        assert_eq!(parse_color(&json!("#ff8000")), Ok(Color::Rgb(255, 128, 0)));
        assert!(parse_color(&json!("chartreuse")).is_err());
        assert!(parse_color(&json!(300)).is_err());
    }

    #[test]
    fn every_schema_tool_is_dispatched() {
        for t in schema::tool_definitions() {
            let name = t["name"].as_str().unwrap();
            let r = Env::new().call(name, json!({ "__probe": 1 }));
            assert!(!r.text.contains("unknown tool"), "{name}: {}", r.text);
        }
    }

    #[test]
    fn bad_args_list_expected() {
        let mut e = Env::new();
        let r = e.call("put_text", json!({ "x": 1 }));
        assert!(r.is_error);
        assert!(r.text.contains("text*"), "{}", r.text);
        let r = e.call("put_text", json!({ "x": 1, "y": 1, "text": "a", "colour": 3 }));
        assert!(r.is_error && r.text.contains("colour"), "{}", r.text);
        assert!(e.call("nope", json!({})).is_error);
        assert_eq!(e.history.len(), 0);
    }

    #[test]
    fn read_only_tools() {
        let mut e = Env::new();
        e.one_step("put_text", json!({ "x": 2, "y": 1, "text": "HI", "fg": 14 }));
        let r = e.call("get_info", json!({}));
        assert!(!r.is_error && r.text.contains("\"width\": 20") && r.text.contains("yellow"), "{}", r.text);
        let r = e.call("get_canvas", json!({}));
        assert!(r.text.contains("   1|  HI"), "{}", r.text);
        assert!(r.text.contains("01234567890123456789"), "{}", r.text);
        let r = e.call("get_canvas", json!({ "format": "colors" }));
        assert!(r.text.contains("2-3 14/0"), "{}", r.text);
        let r = e.call("get_canvas", json!({ "format": "ansi", "x": 2, "y": 1, "w": 2, "h": 1 }));
        assert!(r.text.contains("\x1b[38;2;255;255;85"), "{}", r.text);
        let r = e.call("render_png", json!({ "scale": 2 }));
        let png = r.image_png.expect("image");
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (20 * 16, 10 * 32));
        let r = e.call("render_png", json!({ "x": 2, "y": 1, "w": 2, "h": 1 }));
        let img = image::load_from_memory(&r.image_png.unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (16, 16));
        assert_eq!(e.history.len(), 1);
    }

    #[test]
    fn put_text_and_set_cells() {
        let mut e = Env::new();
        e.one_step("put_text", json!({ "x": 1, "y": 2, "text": "AB\nC", "fg": "red", "bg": 1 }));
        assert_eq!(e.cell(1, 2), Cell::new('A', red(), Color::Pal(1)));
        assert_eq!(e.cell(1, 3).ch, 'C');
        e.one_step("set_cells", json!({ "cells": [{ "x": 0, "y": 0, "ch": "▓", "fg": 10 }, { "x": 1, "y": 2, "fg": 15 }, { "x": 3, "y": 0, "ch": 219 }] }));
        assert_eq!(e.cell(0, 0), Cell::new('▓', Color::Pal(10), Color::BLACK));
        assert_eq!(e.cell(1, 2), Cell::new('A', Color::WHITE, Color::Pal(1)));
        assert_eq!(e.cell(3, 0).ch, '█');
    }

    #[test]
    fn brush_strokes() {
        let mut e = Env::new();
        let r = e.one_step("brush_stroke", json!({ "points": [[1.5, 2.0], [18.5, 2.0]], "fg": 12 }));
        assert!(r.text.contains("Ink"), "{}", r.text);
        let row: String = (0..20).map(|x| e.cell(x, 1).ch).chain((0..20).map(|x| e.cell(x, 2).ch)).collect();
        assert!(row.contains('▄') || row.contains('▀') || row.contains('█'), "{row:?}");
        assert!((0..20).any(|x| e.cell(x, 2).fg == Color::Pal(12) && e.cell(x, 2).ch != ' '));
        e.one_step("brush_stroke", json!({ "points": [[2, 6], [10, 8], [18, 6]], "brush": "Airbrush", "size": 10 }));
        let shades = (0..20)
            .flat_map(|x| (4..11).map(move |y| (x, y)))
            .filter(|&(x, y)| "░▒▓".contains(e.cell(x, y).ch))
            .count();
        assert!(shades > 3, "airbrush should shade");
        let bad = e.call("brush_stroke", json!({ "points": [[0, 0]], "brush": "crayon" }));
        assert!(bad.is_error && bad.text.contains("airbrush"), "{}", bad.text);
    }

    #[test]
    fn shapes() {
        let mut e = Env::new();
        e.one_step("fill_rect", json!({ "x": 0, "y": 0, "w": 3, "h": 2, "ch": "░", "fg": 2 }));
        assert_eq!(e.cell(2, 1), Cell::new('░', Color::Pal(2), Color::BLACK));
        e.one_step("fill_rect", json!({ "x": 0, "y": 0, "w": 1, "h": 1, "fg": 12, "bg": 1, "what": "colors" }));
        assert_eq!(e.cell(0, 0), Cell::new('░', Color::Pal(12), Color::Pal(1)));
        e.one_step("draw_box", json!({ "x": 5, "y": 1, "w": 6, "h": 4, "style": "double", "fg": 11 }));
        assert_eq!(e.cell(5, 1).ch, '╔');
        assert_eq!(e.cell(10, 4).ch, '╝');
        assert_eq!(e.cell(7, 1).ch, '═');
        e.one_step("draw_line", json!({ "x0": 0, "y0": 9, "x1": 19, "y1": 9, "ch": "─" }));
        assert!((0..20).all(|x| e.cell(x, 9).ch == '─'));
        e.one_step("draw_ellipse", json!({ "x": 12, "y": 0, "w": 8, "h": 5, "filled": true, "fg": 4 }));
        assert_eq!(e.cell(16, 2), Cell::new('█', red(), Color::BLACK));
        e.one_step("erase_rect", json!({ "x": 0, "y": 0, "w": 20, "h": 10 }));
        assert!(e.doc.canvas.used_height() == 0);
    }

    #[test]
    fn flood_fill_fills_region() {
        let mut e = Env::new();
        e.one_step("draw_box", json!({ "x": 0, "y": 0, "w": 5, "h": 5 }));
        e.one_step("flood_fill", json!({ "x": 2, "y": 2, "ch": "▒", "fg": 9 }));
        assert_eq!(e.cell(2, 2), Cell::new('▒', Color::Pal(9), Color::BLACK));
        assert_eq!(e.cell(10, 2), Cell::BLANK);
    }

    #[test]
    fn pixels() {
        let mut e = Env::new();
        e.one_step("pixel_set", json!({ "pixels": [[0, 0, 12], { "x": 0, "y": 1, "color": "blue" }] }));
        let c = e.cell(0, 0);
        assert!(matches!(c.ch, '▀' | '▄'), "{c:?}");
        e.one_step("pixel_line", json!({ "x0": 0, "y0": 4, "x1": 9, "y1": 4, "color": 14 }));
        assert_eq!(e.cell(5, 2).ch, '▀');
        e.one_step("pixel_rect", json!({ "x": 10, "y": 0, "w": 4, "h": 4, "color": 2 }));
        assert_eq!(e.cell(11, 1), Cell::new('█', Color::Pal(2), Color::BLACK));
        e.one_step("pixel_rect", json!({ "x": 10, "y": 6, "w": 4, "h": 4, "color": 1, "color2": 9, "mix": 0.5 }));
        e.one_step("pixel_ellipse", json!({ "cx": 4, "cy": 14, "rx": 3, "ry": 3, "color": 13 }));
        assert_ne!(e.cell(4, 7), Cell::BLANK);
        e.one_step("pixel_fill", json!({ "x": 19, "y": 19, "color": 4 }));
        assert_eq!(e.cell(19, 9).fg, red());
    }

    #[test]
    fn regions_transform_and_move() {
        let mut e = Env::new();
        e.one_step("put_text", json!({ "x": 0, "y": 0, "text": "▌AB" }));
        e.one_step("transform_region", json!({ "x": 0, "y": 0, "w": 3, "h": 1, "op": "flip_x" }));
        assert_eq!((e.cell(0, 0).ch, e.cell(2, 0).ch), ('B', '▐'));
        e.one_step("transform_region", json!({ "x": 0, "y": 0, "w": 10, "h": 1, "op": "justify_right" }));
        assert_eq!(e.cell(9, 0).ch, '▐');
        e.one_step("move_region", json!({ "x": 7, "y": 0, "w": 3, "h": 1, "to_x": 0, "to_y": 5 }));
        assert_eq!(e.cell(2, 5).ch, '▐');
        assert_eq!(e.cell(9, 0), Cell::BLANK);
        e.one_step("move_region", json!({ "x": 0, "y": 5, "w": 3, "h": 1, "to_x": 0, "to_y": 6, "copy": true }));
        assert_eq!((e.cell(2, 5).ch, e.cell(2, 6).ch), ('▐', '▐'));
        e.one_step("transform_region", json!({ "x": 0, "y": 0, "w": 20, "h": 10, "op": "outline", "style": "single" }));
        assert_eq!(e.cell(0, 0).ch, '┌');
        e.one_step("transform_region", json!({ "x": 0, "y": 5, "w": 3, "h": 2, "op": "rotate_180" }));
    }

    #[test]
    fn layers_and_default_target() {
        let mut e = Env::new();
        let r = e.one_step("layer", json!({ "action": "add", "name": "AI" }));
        assert!(r.text.contains("index 1"), "{}", r.text);
        assert_eq!(e.layer, 1);
        e.one_step("put_text", json!({ "x": 0, "y": 0, "text": "X" }));
        assert_eq!(e.doc.canvas.get(1, 0, 0).unwrap().ch, 'X');
        assert_eq!(e.doc.canvas.get(0, 0, 0), Some(Cell::BLANK));
        e.one_step("layer", json!({ "action": "set_props", "index": 1, "visible": false }));
        assert_eq!(e.cell(0, 0), Cell::BLANK);
        e.one_step("layer", json!({ "action": "set_props", "index": 1, "visible": true, "name": "ink" }));
        let r = e.call("layer", json!({ "action": "list" }));
        assert!(r.text.contains("1: ink") && r.text.contains("AI edits go here"), "{}", r.text);
        e.one_step("layer", json!({ "action": "merge_down", "index": 1 }));
        assert_eq!(e.doc.canvas.layers.len(), 1);
        assert_eq!(e.doc.canvas.get(0, 0, 0).unwrap().ch, 'X');
        assert_eq!(e.layer, 0);
        e.one_step("layer", json!({ "action": "add" }));
        e.call("layer", json!({ "action": "select", "index": 0 }));
        assert_eq!(e.layer, 0);
        e.one_step("layer", json!({ "action": "remove", "index": 1 }));
        e.one_step("layer", json!({ "action": "set_props", "index": 0, "locked": true }));
        let r = e.call("put_text", json!({ "x": 0, "y": 0, "text": "Y" }));
        assert!(r.is_error && r.text.contains("locked"));
    }

    #[test]
    fn canvas_ops() {
        let mut e = Env::new();
        e.one_step("put_text", json!({ "x": 0, "y": 0, "text": "keep?" }));
        e.one_step("resize", json!({ "width": 30, "height": 12 }));
        assert_eq!((e.doc.width(), e.doc.height()), (30, 12));
        e.one_step("new_canvas", json!({ "width": 40, "height": 5, "kind": "modern" }));
        assert_eq!((e.doc.width(), e.doc.height(), e.doc.is_classic()), (40, 5, false));
        assert_eq!(e.cell(0, 0), Cell::BLANK);
        e.one_step("put_text", json!({ "x": 0, "y": 0, "text": "é", "fg": "#123456" }));
        assert_eq!(e.cell(0, 0), Cell::new('é', Color::Rgb(0x12, 0x34, 0x56), Color::BLACK));
    }

    #[test]
    fn undo_redo_tools() {
        let mut e = Env::new();
        e.call("put_text", json!({ "x": 0, "y": 0, "text": "A" }));
        e.call("put_text", json!({ "x": 1, "y": 0, "text": "B" }));
        let r = e.call("undo", json!({ "steps": 2 }));
        assert!(r.text.contains("AI: put_text"), "{}", r.text);
        assert_eq!(e.doc.canvas.used_height(), 0);
        e.call("redo", json!({}));
        assert_eq!(e.cell(0, 0).ch, 'A');
        assert_eq!(e.cell(1, 0), Cell::BLANK);
        assert!(e.call("redo", json!({ "steps": 5 })).text.contains("Redid"));
        assert!(e.call("redo", json!({})).text.contains("Nothing"));
    }

    #[test]
    fn save_load_roundtrip() {
        let mut e = Env::new();
        e.one_step("put_text", json!({ "x": 0, "y": 0, "text": "SAVE ME", "fg": 12 }));
        let path = e.dir.path().join("out.ans");
        let r = e.call("save", json!({ "path": path, "title": "t", "author": "ai" }));
        assert!(!r.is_error, "{}", r.text);
        assert_eq!(e.history.undo_label(), Some("AI: save (SAUCE)"));
        assert_eq!(e.file.as_deref(), Some(path.as_path()));
        e.one_step("new_canvas", json!({}));
        assert!(e.file.is_none());
        let r = e.one_step("load", json!({ "path": path }));
        assert!(r.text.contains("SAUCE"), "{}", r.text);
        assert_eq!(e.cell(0, 0), Cell::new('S', Color::Pal(12), Color::BLACK));
        assert_eq!(e.doc.meta.sauce.author, "ai");
    }

    #[test]
    fn import_image_stamps() {
        let mut e = Env::new();
        let mut img = image::RgbaImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = image::Rgba([255, 0, 0, 255]);
        }
        let mut png = std::io::Cursor::new(vec![]);
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
        e.one_step("import_image", json!({ "png_base64": b64, "x": 2, "y": 1, "width": 4 }));
        let c = e.cell(3, 1);
        assert!(c.fg.rgb(&e.doc.meta.palette)[0] > 0xA0 || c.bg.rgb(&e.doc.meta.palette)[0] > 0xA0, "{c:?}");
    }

    #[test]
    fn stencils_roundtrip() {
        let mut e = Env::new();
        e.one_step("put_text", json!({ "x": 0, "y": 0, "text": "<*>", "fg": 13 }));
        let r = e.call("save_stencil", json!({ "x": 0, "y": 0, "w": 3, "h": 1, "name": "star", "tags": ["deco"] }));
        assert!(!r.is_error, "{}", r.text);
        let r = e.call("list_stencils", json!({ "query": "star" }));
        let id = r.text.lines().nth(1).and_then(|l| l.split(" | ").next()).unwrap().to_string();
        e.one_step("stamp_stencil", json!({ "id": id, "x": 5, "y": 5 }));
        assert_eq!(e.cell(6, 5), Cell::new('*', Color::Pal(13), Color::BLACK));
    }

    #[test]
    fn fonts_banner() {
        let mut e = Env::new();
        let r = e.call("list_fonts", json!({}));
        assert!(!r.is_error, "{}", r.text);
        let id =
            r.text.lines().nth(1).and_then(|l| l.split(" | ").next()).expect("at least one bundled font").to_string();
        e.one_step("banner", json!({ "text": "HI", "font": id, "x": 0, "y": 0 }));
        assert!(e.doc.canvas.used_height() >= 1);
    }
}
