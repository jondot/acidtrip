//! Web exports: SVG (text runs or pixel-exact glyphs), HTML, React and asciicast.

use std::fmt::Write as _;

use acidtrip_core::render::glyph_pixel;
use acidtrip_core::{Color, Document, Grid, cp437};
use minijinja::{Environment, Value, context};

use super::text::identifier;
use super::{SaveOptions, ansi, export_grid, export_rows};

const FONT_STACK: &str =
    r#""Perfect DOS VGA 437", "Px437 IBM VGA 8x16", "Px437 IBM VGA8", Consolas, Menlo, "DejaVu Sans Mono", monospace"#;
const FONT_FACE: &str = r#"@font-face { font-family: "Perfect DOS VGA 437"; src: local("Perfect DOS VGA 437"), local("PerfectDOSVGA437"), url("PerfectDOSVGA437.ttf") format("truetype"); }"#;

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() => o.push('?'),
            c => o.push(c),
        }
    }
    o
}

/// Char as shown on screen: NUL is a space, other C0 chars are their CP437 glyphs.
fn shown(ch: char) -> char {
    match ch {
        '\0' => ' ',
        c if c.is_control() => cp437::to_char(cp437::from_char_lossy(c)),
        c => c,
    }
}

fn space_like(ch: char) -> bool {
    matches!(ch, ' ' | '\0' | '\u{A0}')
}

fn title(doc: &Document) -> String {
    let t = doc.meta.sauce.title.trim();
    if t.is_empty() { "acidtrip".into() } else { t.into() }
}

// --- SVG ---------------------------------------------------------------------

/// Paint of a cell split in quadrants [TL, TR, BL, BR] (`true` = fg) with the
/// top band being `split` rows high. `None` means the glyph isn't made of
/// quadrants and needs a text/glyph draw.
fn quadrants(ch: char, pixel_exact: bool, cw: u32) -> Option<(u32, [bool; 4])> {
    if !pixel_exact {
        return match ch {
            ' ' | '\0' | '\u{A0}' => Some((8, [false; 4])),
            '█' => Some((8, [true; 4])),
            '▀' => Some((8, [true, true, false, false])),
            '▄' => Some((8, [false, false, true, true])),
            '▌' => Some((8, [true, false, true, false])),
            '▐' => Some((8, [false, true, false, true])),
            _ => None,
        };
    }
    let uniform = |xs: std::ops::Range<u32>, y: u32| {
        let on = xs.clone().filter(|&x| pixel(ch, x, y)).count() as u32;
        (on == 0 || on == xs.len() as u32).then_some(on > 0)
    };
    let mut sigs = Vec::with_capacity(16);
    for y in 0..16 {
        sigs.push((uniform(0..4, y)?, uniform(4..cw, y)?));
    }
    let split = (1..16).find(|&y| sigs[y as usize] != sigs[0]).unwrap_or(8);
    let (a, b) = (sigs[0], sigs[split as usize]);
    sigs[split as usize..].iter().all(|&s| s == b).then_some((split, [a.0, a.1, b.0, b.1]))
}

/// Glyph pixel including the VGA 9th column for line-drawing chars.
fn pixel(ch: char, x: u32, y: u32) -> bool {
    if x < 8 {
        glyph_pixel(ch, x, y)
    } else {
        matches!(cp437::from_char(ch), Some(0xC0..=0xDF)) && glyph_pixel(ch, 7, y)
    }
}

/// Glyph bitmap as a merged rectangle path.
fn glyph_path(ch: char, cw: u32) -> String {
    let mut rects: Vec<(u32, u32, u32, u32)> = Vec::new(); // x, y, w, h
    let mut open: Vec<usize> = Vec::new();
    for y in 0..16 {
        let mut runs = Vec::new();
        let mut x = 0;
        while x < cw {
            if pixel(ch, x, y) {
                let s = x;
                while x < cw && pixel(ch, x, y) {
                    x += 1;
                }
                runs.push((s, x - s));
            } else {
                x += 1;
            }
        }
        let mut next = Vec::new();
        for (x, w) in runs {
            match open.iter().find(|&&i| rects[i].0 == x && rects[i].2 == w && rects[i].1 + rects[i].3 == y) {
                Some(&i) => {
                    rects[i].3 += 1;
                    next.push(i);
                }
                None => {
                    rects.push((x, y, w, 1));
                    next.push(rects.len() - 1);
                }
            }
        }
        open = next;
    }
    rects.iter().map(|(x, y, w, h)| format!("M{x} {y}h{w}v{h}h-{w}z")).collect()
}

/// Half-cell paint column: x, width, top color, bottom color, split row.
type HalfCol = (u32, u32, [u8; 3], [u8; 3], u32);

fn group(groups: &mut Vec<([u8; 3], String)>, fg: [u8; 3]) -> &mut String {
    let i = groups.iter().position(|(c, _)| *c == fg).unwrap_or_else(|| {
        groups.push((fg, String::new()));
        groups.len() - 1
    });
    &mut groups[i].1
}

pub fn save_svg(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    let g = export_grid(doc, opts);
    let pal = &doc.meta.palette;
    let exact = opts.svg_pixel_exact;
    let cw: u32 = if doc.meta.letter_spacing_9px { 9 } else { 8 };
    let (w, h) = (g.width as u32 * cw, g.height as u32 * 16);
    let s = opts.scale.clamp(1, 8);
    let base = pal.get(0);
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{}" height="{}" viewBox="0 0 {w} {h}" shape-rendering="crispEdges">"#,
        w * s,
        h * s
    );
    let _ = write!(out, "<title>{}</title>", esc(&title(doc)));
    if !exact {
        let _ = write!(out, "<style>{FONT_FACE} text {{ font-family: {FONT_STACK}; font-size: 16px; }}</style>");
    }
    let quads: Vec<Option<(u32, [bool; 4])>> = g.cells.iter().map(|c| quadrants(c.ch, exact, cw)).collect();

    // Background + block paint: half-cell columns with (top, bottom) colors.
    let _ = write!(out, r#"<rect width="{w}" height="{h}" fill="{}"/>"#, hex(base));
    for y in 0..g.height {
        let mut cols: Vec<HalfCol> = Vec::new();
        for x in 0..g.width {
            let c = g.get(x, y);
            let (split, q) = quads[y * g.width + x].unwrap_or((8, [false; 4]));
            let col = |on: bool| if on { c.fg.rgb(pal) } else { c.bg.rgb(pal) };
            let x0 = x as u32 * cw;
            for (dx, w, t, b) in [(0, 4, q[0], q[2]), (4, cw - 4, q[1], q[3])] {
                let (t, b) = (col(t), col(b));
                cols.push((x0 + dx, w, t, b, if t == b { 8 } else { split }));
            }
        }
        let y0 = y as u32 * 16;
        let mut i = 0;
        while i < cols.len() {
            let (x, _, t, b, split) = cols[i];
            let mut j = i;
            let mut width = 0;
            while j < cols.len() && cols[j].2 == t && cols[j].3 == b && cols[j].4 == split {
                width += cols[j].1;
                j += 1;
            }
            let mut rect = |yy: u32, hh: u32, c: [u8; 3]| {
                if c != base {
                    let _ = write!(out, r#"<rect x="{x}" y="{yy}" width="{width}" height="{hh}" fill="{}"/>"#, hex(c));
                }
            };
            if t == b {
                rect(y0, 16, t);
            } else {
                rect(y0, split, t);
                rect(y0 + split, 16 - split, b);
            }
            i = j;
        }
    }

    // Glyphs, grouped by foreground color.
    let mut groups: Vec<([u8; 3], String)> = Vec::new();
    if exact {
        let mut used: Vec<char> = Vec::new();
        for (i, c) in g.cells.iter().enumerate() {
            if quads[i].is_some() || c.fg.rgb(pal) == c.bg.rgb(pal) {
                continue;
            }
            let ch = shown(c.ch);
            if !used.contains(&ch) {
                used.push(ch);
            }
            let (x, y) = ((i % g.width) as u32 * cw, (i / g.width) as u32 * 16);
            let _ =
                write!(group(&mut groups, c.fg.rgb(pal)), r##"<use xlink:href="#g{:x}" x="{x}" y="{y}"/>"##, ch as u32);
        }
        out.push_str("<defs>");
        for ch in used {
            let _ = write!(
                out,
                r#"<symbol id="g{:x}" overflow="visible"><path d="{}"/></symbol>"#,
                ch as u32,
                glyph_path(ch, cw)
            );
        }
        out.push_str("</defs>");
        for (fg, body) in &groups {
            let _ = write!(out, r#"<g fill="{}">{body}</g>"#, hex(*fg));
        }
    } else {
        for y in 0..g.height {
            let row = g.row(y);
            let glyph = |x: usize| quads[y * g.width + x].is_none();
            let mut x = 0;
            while x < g.width {
                if !glyph(x) {
                    x += 1;
                    continue;
                }
                let fg = row[x].fg;
                let mut end = x;
                let mut text = String::new();
                let mut j = x;
                while j < g.width && (!glyph(j) || row[j].fg == fg) {
                    text.push(if glyph(j) { shown(row[j].ch) } else { ' ' });
                    if glyph(j) {
                        end = j + 1;
                    }
                    j += 1;
                }
                let n = end - x;
                let text: String = text.chars().take(n).collect();
                let _ = write!(
                    group(&mut groups, fg.rgb(pal)),
                    r#"<text x="{}" y="{}" textLength="{}" lengthAdjust="spacingAndGlyphs">{}</text>"#,
                    x as u32 * cw,
                    y as u32 * 16 + 12,
                    n as u32 * cw,
                    esc(&text)
                );
                x = end;
            }
        }
        for (fg, body) in &groups {
            let _ = write!(out, r#"<g fill="{}" xml:space="preserve">{body}</g>"#, hex(*fg));
        }
    }
    out.push_str("</svg>\n");
    out.into_bytes()
}

// --- HTML / React ------------------------------------------------------------

struct Run {
    text: String,
    fg: Option<Color>,
    bg: Color,
}

/// Merge each row into runs of equal colors; spaces join any run with the same bg.
fn runs(g: &Grid) -> Vec<Vec<Run>> {
    (0..g.height)
        .map(|y| {
            let row = g.row(y);
            let end = row.iter().rposition(|c| !(space_like(c.ch) && c.bg == Color::BLACK)).map_or(0, |i| i + 1);
            let mut out: Vec<Run> = Vec::new();
            for c in &row[..end] {
                let fg = (!space_like(c.ch)).then_some(c.fg);
                match out.last_mut() {
                    Some(r) if r.bg == c.bg && (fg.is_none() || r.fg.is_none() || r.fg == fg) => {
                        r.fg = r.fg.or(fg);
                        r.text.push(shown(c.ch));
                    }
                    _ => out.push(Run { text: shown(c.ch).to_string(), fg, bg: c.bg }),
                }
            }
            out
        })
        .collect()
}

/// Stable color table: index of each distinct color.
struct Colors(Vec<[u8; 3]>);

impl Colors {
    fn id(&mut self, c: [u8; 3]) -> usize {
        self.0.iter().position(|&x| x == c).unwrap_or_else(|| {
            self.0.push(c);
            self.0.len() - 1
        })
    }
}

fn env() -> Environment<'static> {
    let mut e = Environment::new();
    e.add_template("page.html", include_str!("templates/page.html")).expect("html template");
    e.add_template("component.tsx", include_str!("templates/component.tsx")).expect("tsx template");
    e
}

pub fn save_html(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = export_grid(doc, opts);
    let pal = &doc.meta.palette;
    let (dfg, dbg) = (Color::LIGHT_GRAY.rgb(pal), Color::BLACK.rgb(pal));
    let mut colors = Colors(Vec::new());
    let mut body = String::new();
    for (y, row) in runs(&g).iter().enumerate() {
        if y > 0 {
            body.push('\n');
        }
        for r in row {
            let mut cls = Vec::new();
            if let Some(fg) = r.fg.map(|c| c.rgb(pal)).filter(|&c| c != dfg) {
                cls.push(format!("f{}", colors.id(fg)));
            }
            if r.bg.rgb(pal) != dbg {
                cls.push(format!("b{}", colors.id(r.bg.rgb(pal))));
            }
            if cls.is_empty() {
                body.push_str(&esc(&r.text));
            } else {
                let _ = write!(body, r#"<span class="{}">{}</span>"#, cls.join(" "), esc(&r.text));
            }
        }
    }
    let mut css = String::new();
    for (i, c) in colors.0.iter().enumerate() {
        let _ = writeln!(css, ".acid .f{i} {{ color: {h}; }} .acid .b{i} {{ background: {h}; }}", h = hex(*c));
    }
    let s = &doc.meta.sauce;
    let html = env().get_template("page.html")?.render(context! {
        title => title(doc),
        author => [s.author.as_str(), s.group.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" / "),
        comments => s.comments.join(" "),
        cols => g.width,
        fg => hex(dfg),
        bg => hex(dbg),
        font_face => Value::from_safe_string(FONT_FACE.into()),
        font_stack => Value::from_safe_string(FONT_STACK.into()),
        css => Value::from_safe_string(css),
        body => Value::from_safe_string(body),
    })?;
    Ok(html.into_bytes())
}

pub fn save_react(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = export_grid(doc, opts);
    let pal = &doc.meta.palette;
    let mut id = identifier(&opts.identifier, "AcidArt");
    if let Some(f) = id.chars().next().filter(|c| c.is_ascii_lowercase()) {
        id.replace_range(..1, &f.to_ascii_uppercase().to_string());
    }
    if id.starts_with('_') {
        id.insert(0, 'A');
    }
    let mut colors = Colors(Vec::new());
    let dfg = colors.id(Color::LIGHT_GRAY.rgb(pal));
    let black = Color::BLACK.rgb(pal);
    let lines: Vec<String> = runs(&g)
        .iter()
        .map(|row| {
            let items: Vec<String> = row
                .iter()
                .map(|r| {
                    let fg = r.fg.map_or(dfg, |c| colors.id(c.rgb(pal)));
                    let bg = if r.bg.rgb(pal) == black { -1 } else { colors.id(r.bg.rgb(pal)) as i64 };
                    format!("[{}, {fg}, {bg}]", serde_json::to_string(&r.text).unwrap_or_default())
                })
                .collect();
            format!("[{}]", items.join(", "))
        })
        .collect();
    let color_list: Vec<String> = colors.0.iter().map(|&c| format!("\"{}\"", hex(c))).collect();
    let tsx = env().get_template("component.tsx")?.render(context! {
        ident => id,
        title => serde_json::to_string(&title(doc))?,
        comment_title => title(doc).replace("*/", "* /"),
        cols => g.width,
        rows => g.height,
        colors => format!("[{}]", color_list.join(", ")),
        lines => format!("[\n  {}\n]", lines.join(",\n  ")),
        default_fg => dfg,
        background => serde_json::to_string(&hex(black))?,
        font_stack => serde_json::to_string(FONT_STACK)?,
    })?;
    Ok(tsx.into_bytes())
}

// --- asciicast v2 ------------------------------------------------------------

pub fn save_asciicast(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    if opts.animate && doc.is_animated() {
        return asciicast_frames(doc, opts);
    }
    let stream = ansi::utf8_string(doc, opts).replace('\n', "\r\n");
    let header = serde_json::json!({
        "version": 2,
        "width": doc.width(),
        "height": export_rows(doc, opts),
        "title": title(doc),
        "env": { "TERM": "xterm-256color", "SHELL": "/bin/sh" },
    });
    let mut out = format!("{header}\n");
    let bps = (opts.baud as f64 / 10.0).max(1.0);
    let chunk = (bps / 20.0).ceil().max(1.0) as usize;
    let (mut sent, mut buf) = (0usize, String::new());
    let emit = |sent: usize, buf: &mut String, out: &mut String| {
        let t = sent as f64 / bps;
        let _ = writeln!(out, "[{t:.6}, \"o\", {}]", serde_json::to_string(buf).unwrap_or_default());
        buf.clear();
    };
    for ch in stream.chars() {
        buf.push(ch);
        if buf.len() >= chunk {
            let n = buf.len();
            emit(sent, &mut buf, &mut out);
            sent += n;
        }
    }
    if !buf.is_empty() {
        let n = buf.len();
        emit(sent, &mut buf, &mut out);
        sent += n;
    }
    // Hold the finished piece on screen for 3 s.
    let _ = writeln!(out, "[{:.6}, \"o\", \"\"]", sent as f64 / bps + 3.0);
    out.into_bytes()
}

/// Animation frames as a cast: each frame redrawn from the top left at its
/// own time, looped until about ten seconds have played.
fn asciicast_frames(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    let frames = super::frame_grids(doc, opts);
    let pal = &doc.meta.palette;
    let rows = frames.first().map_or(1, |(g, _)| g.height);
    let header = serde_json::json!({
        "version": 2,
        "width": doc.width(),
        "height": rows,
        "title": title(doc),
        "env": { "TERM": "xterm-256color", "SHELL": "/bin/sh" },
    });
    let mut out = format!("{header}\n");
    let shots: Vec<String> = frames
        .iter()
        .map(|(g, _)| {
            let s = ansi::utf8_grid(g, pal);
            format!("\x1b[H{}", s.trim_end_matches('\n').replace('\n', "\r\n"))
        })
        .collect();
    let tick = 1.0 / doc.fps() as f64;
    let loop_len: f64 = frames.iter().map(|(_, h)| *h as f64 * tick).sum();
    let loops = (10.0 / loop_len.max(0.01)).ceil().clamp(1.0, 100.0) as usize;
    let mut t = 0.0;
    let _ = writeln!(out, "[0.000000, \"o\", \"\\u001b[2J\"]");
    for _ in 0..loops {
        for (s, (_, hold)) in shots.iter().zip(&frames) {
            let _ = writeln!(out, "[{t:.6}, \"o\", {}]", serde_json::to_string(s).unwrap_or_default());
            t += *hold as f64 * tick;
        }
    }
    let _ = writeln!(out, "[{t:.6}, \"o\", \"\"]");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{Cell, DocKind};

    #[test]
    fn block_glyphs_decompose_into_quadrants() {
        assert_eq!(quadrants('▀', true, 8), Some((7, [true, true, false, false])));
        assert_eq!(quadrants('▐', true, 9), Some((8, [false, true, false, true])));
        assert_eq!(quadrants('█', true, 9), Some((8, [true; 4])));
        assert_eq!(quadrants('░', true, 8), None);
        assert_eq!(quadrants('A', true, 8), None);
        assert_eq!(quadrants(' ', false, 8), Some((8, [false; 4])));
    }

    #[test]
    fn glyph_path_covers_pixels() {
        assert_eq!(glyph_path('█', 8), "M0 0h8v16h-8z");
        assert_eq!(glyph_path(' ', 8), "");
    }

    #[test]
    fn html_runs_merge() {
        let mut d = Document::new(DocKind::Classic, 6, 1);
        for (i, ch) in "ab cd".chars().enumerate() {
            d.canvas.layers[0].cells[i] = Some(Cell::new(ch, Color::Pal(12), Color::BLACK));
        }
        let r = runs(&d.flatten());
        assert_eq!(r[0].len(), 1);
        assert_eq!(r[0][0].text, "ab cd");
    }
}
