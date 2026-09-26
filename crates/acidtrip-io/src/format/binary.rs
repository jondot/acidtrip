//! Binary screen formats: BIN, XBin, ArtWorx ADF, iCE Draw IDF, TundraDraw.
//! Readers cross-checked against icy_engine (MIT/Apache).

use acidtrip_core::color::VGA;
use acidtrip_core::{Cell, Color, Document, Grid, Palette};
use anyhow::{bail, ensure};

use super::sauce::{self, CHAR_ANSI, CHAR_TUNDRA, Kind};
use super::{
    SaveOptions, attr_byte, attr_cell, char_byte, classic_grid, dac8, export_grid, finish, palette_from, want_sauce,
};

/// Fill a grid of `width` from char/attr pairs.
fn pairs_grid(data: &[u8], width: usize, min_h: usize) -> Grid {
    let n = data.len() / 2;
    let mut g = Grid::new(width, n.div_ceil(width).max(min_h));
    for (i, p) in data.as_chunks::<2>().0.iter().enumerate() {
        g.set(i % width, i / width, attr_cell(p[0], p[1]));
    }
    g
}

fn pairs(g: &Grid, pal: &Palette) -> Vec<u8> {
    g.cells.iter().flat_map(|c| [char_byte(c.ch), attr_byte(c, pal)]).collect()
}

fn pal16(p: &Palette) -> Vec<[u8; 3]> {
    (0..16).map(|i| p.colors.get(i).copied().unwrap_or(VGA[i])).collect()
}

fn pal63(p: &Palette) -> Vec<u8> {
    pal16(p).iter().flat_map(|c| c.map(|v| v >> 2)).collect()
}

fn from63(b: &[u8]) -> Vec<[u8; 3]> {
    b.as_chunks::<3>().0.iter().map(|&[r, g, b]| [dac8(r), dac8(g), dac8(b)]).collect()
}

// --- BIN -------------------------------------------------------------------

pub fn load_bin(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    let width = sauce::width(rec.as_ref()).unwrap_or(160);
    Ok(finish(pairs_grid(body, width, 1), rec.as_ref(), None, false, false))
}

pub fn save_bin(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = classic_grid(doc, opts);
    let mut out = pairs(&g, &doc.meta.palette);
    if want_sauce(doc, opts, g.width != 160) {
        sauce::append(&mut out, doc, Kind::Binary, g.width, g.height)?;
    }
    Ok(out)
}

// --- XBin ------------------------------------------------------------------

const XB_PALETTE: u8 = 1;
const XB_FONT: u8 = 2;
const XB_COMPRESS: u8 = 4;
const XB_NONBLINK: u8 = 8;
const XB_512: u8 = 16;

pub fn load_xbin(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    if !body.starts_with(b"XBIN") {
        // Misnamed raw BIN files are common in the wild.
        return load_bin(data);
    }
    ensure!(body.len() >= 11, "XBin header truncated");
    let w = u16::from_le_bytes([body[5], body[6]]) as usize;
    let h = u16::from_le_bytes([body[7], body[8]]) as usize;
    let font_h = if body[9] == 0 { 16 } else { body[9] as usize };
    let flags = body[10];
    let mut o = 11;
    let mut palette = None;
    if flags & XB_PALETTE != 0 {
        ensure!(body.len() >= o + 48, "XBin palette truncated");
        palette = palette_from(from63(&body[o..o + 48]), "XBin");
        o += 48;
    }
    if flags & XB_FONT != 0 {
        // The embedded font is skipped: acidtrip renders with its own VGA font.
        o += font_h * if flags & XB_512 != 0 { 512 } else { 256 };
    }
    let data = body.get(o..).unwrap_or_default();
    let mut g = Grid::new(w.max(1), h.max(1));
    let mask = if flags & XB_512 != 0 { 0xF7 } else { 0xFF };
    let mut i = 0;
    let mut put = |ch: u8, attr: u8| {
        if i < w * h {
            g.set(i % w, i / w, attr_cell(ch, attr & mask));
        }
        i += 1;
    };
    if flags & XB_COMPRESS != 0 {
        let mut p = 0;
        while p < data.len() {
            let (kind, n) = (data[p] >> 6, (data[p] & 63) as usize + 1);
            p += 1;
            let rd = |p: usize| data.get(p).copied().unwrap_or(0);
            match kind {
                0 => {
                    for k in 0..n {
                        put(rd(p + 2 * k), rd(p + 2 * k + 1));
                    }
                    p += 2 * n;
                }
                1 => {
                    for k in 0..n {
                        put(rd(p), rd(p + 1 + k));
                    }
                    p += 1 + n;
                }
                2 => {
                    for k in 0..n {
                        put(rd(p + 1 + k), rd(p));
                    }
                    p += 1 + n;
                }
                _ => {
                    for _ in 0..n {
                        put(rd(p), rd(p + 1));
                    }
                    p += 2;
                }
            }
        }
    } else {
        for &[ch, at] in data.as_chunks::<2>().0 {
            put(ch, at);
        }
    }
    Ok(finish(g, rec.as_ref(), palette, false, flags & XB_NONBLINK != 0))
}

/// XBin RLE for one row of char/attr pairs.
fn xbin_compress(row: &[(u8, u8)], out: &mut Vec<u8>) {
    type Key = fn(&(u8, u8)) -> (u8, u8);
    let (both, chr, atr): (Key, Key, Key) = (|c| *c, |c| (c.0, 0), |c| (0, c.1));
    let run = |i: usize, key: Key| row[i..].iter().take(64).take_while(|c| key(c) == key(&row[i])).count();
    let mut i = 0;
    while i < row.len() {
        let (nb, nc, na) = (run(i, both), run(i, chr), run(i, atr));
        if nb >= 2 {
            out.extend([0xC0 | (nb - 1) as u8, row[i].0, row[i].1]);
            i += nb;
        } else if nc >= 3 && nc >= na {
            out.extend([0x40 | (nc - 1) as u8, row[i].0]);
            out.extend(row[i..i + nc].iter().map(|c| c.1));
            i += nc;
        } else if na >= 3 {
            out.extend([0x80 | (na - 1) as u8, row[i].1]);
            out.extend(row[i..i + na].iter().map(|c| c.0));
            i += na;
        } else {
            let mut n = 1;
            while i + n < row.len() && n < 64 && run(i + n, both) < 2 && run(i + n, chr) < 3 && run(i + n, atr) < 3 {
                n += 1;
            }
            out.push((n - 1) as u8);
            out.extend(row[i..i + n].iter().flat_map(|c| [c.0, c.1]));
            i += n;
        }
    }
}

pub fn save_xbin(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = classic_grid(doc, opts);
    ensure!(g.width <= 65535 && g.height <= 65535, "canvas too large for XBin");
    let pal = &doc.meta.palette;
    let mut out = b"XBIN\x1a".to_vec();
    out.extend((g.width as u16).to_le_bytes());
    out.extend((g.height as u16).to_le_bytes());
    out.push(16);
    out.push(XB_PALETTE | XB_COMPRESS | if doc.meta.ice { XB_NONBLINK } else { 0 });
    out.extend(pal63(pal));
    for y in 0..g.height {
        let row: Vec<(u8, u8)> = g.row(y).iter().map(|c| (char_byte(c.ch), attr_byte(c, pal))).collect();
        xbin_compress(&row, &mut out);
    }
    if want_sauce(doc, opts, false) {
        sauce::append(&mut out, doc, Kind::XBin, g.width, g.height)?;
    }
    Ok(out)
}

// --- ADF -------------------------------------------------------------------

/// Where ADF stores the 16 attribute colors inside its 64-color EGA palette.
const EGA_SLOTS: [usize; 16] = [0, 1, 2, 3, 4, 5, 20, 7, 56, 57, 58, 59, 60, 61, 62, 63];

/// Standard 64-color EGA palette, 6-bit components (bits: bgrBGR → rgbRGB).
fn ega64() -> Vec<u8> {
    (0..64u8)
        .flat_map(|i| {
            let c = |hi: u8, lo: u8| ((i >> hi) & 1) * 42 + ((i >> lo) & 1) * 21;
            [c(2, 5), c(1, 4), c(0, 3)]
        })
        .collect()
}

pub fn load_adf(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    ensure!(body.len() >= 1 + 192 + 4096, "ADF file too short");
    ensure!(body[0] == 1, "unsupported ADF version {}", body[0]);
    let ega = &body[1..193];
    let colors = EGA_SLOTS.iter().map(|&s| [dac8(ega[s * 3]), dac8(ega[s * 3 + 1]), dac8(ega[s * 3 + 2])]).collect();
    let g = pairs_grid(&body[1 + 192 + 4096..], 80, 1);
    Ok(finish(g, rec.as_ref(), palette_from(colors, "ADF"), false, true))
}

pub fn save_adf(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let src = classic_grid(doc, opts);
    let mut g = Grid::new(80, src.height);
    for y in 0..src.height {
        for x in 0..80 {
            g.set(x, y, src.get(x, y));
        }
    }
    let mut out = vec![1u8];
    let mut ega = ega64();
    for (i, c) in pal16(&doc.meta.palette).iter().enumerate() {
        ega[EGA_SLOTS[i] * 3..EGA_SLOTS[i] * 3 + 3].copy_from_slice(&c.map(|v| v >> 2));
    }
    out.extend(ega);
    out.extend(super::vga_font_bytes());
    out.extend(pairs(&g, &doc.meta.palette));
    if want_sauce(doc, opts, false) {
        sauce::append(&mut out, doc, Kind::Character(CHAR_ANSI), 80, g.height)?;
    }
    Ok(out)
}

// --- IDF -------------------------------------------------------------------

pub fn load_idf(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    ensure!(body.len() >= 12 + 4096 + 48, "IDF file too short");
    ensure!(body.starts_with(b"\x041.4") || body.starts_with(b"\x041.3"), "not an iCE Draw file");
    let u16at = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]) as usize;
    let (x1, y1, x2) = (u16at(4), u16at(6), u16at(8));
    ensure!(x2 >= x1, "invalid IDF bounds");
    let w = x2 - x1 + 1;
    let end = body.len() - 4096 - 48;
    let mut cells: Vec<(usize, Cell)> = Vec::new();
    let (mut x, mut y) = (x1, y1);
    let mut o = 12;
    while o + 1 < end {
        let (mut ch, mut attr, mut n) = (body[o], body[o + 1], 1usize);
        o += 2;
        if ch == 1 && attr == 0 {
            if o + 3 >= end {
                break;
            }
            n = u16at(o);
            (ch, attr) = (body[o + 2], body[o + 3]);
            o += 4;
        }
        for _ in 0..n {
            if y >= 20_000 {
                break;
            }
            cells.push((y * w + (x - x1), attr_cell(ch, attr)));
            x += 1;
            if x > x2 {
                x = x1;
                y += 1;
            }
        }
    }
    let h = cells.last().map_or(1, |&(i, _)| i / w + 1);
    let mut g = Grid::new(w, h);
    for (i, c) in cells {
        g.cells[i] = c;
    }
    let palette = palette_from(from63(&body[body.len() - 48..]), "IDF");
    Ok(finish(g, rec.as_ref(), palette, false, true))
}

pub fn save_idf(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = classic_grid(doc, opts);
    ensure!(g.width <= 65535 && g.height <= 65535, "canvas too large for IDF");
    let pal = &doc.meta.palette;
    let mut out = b"\x041.4".to_vec();
    for v in [0, 0, g.width - 1, g.height - 1] {
        out.extend((v as u16).to_le_bytes());
    }
    for y in 0..g.height {
        let row: Vec<(u8, u8)> = g.row(y).iter().map(|c| (char_byte(c.ch), attr_byte(c, pal))).collect();
        let mut x = 0;
        while x < row.len() {
            let n = row[x..].iter().take(65535).take_while(|&&c| c == row[x]).count();
            let n = if n > 3 { n } else { 1 };
            let (ch, attr) = row[x];
            if n > 1 || (ch == 1 && attr == 0) {
                out.extend([1, 0]);
                out.extend((n as u16).to_le_bytes());
            }
            out.extend([ch, attr]);
            x += n;
        }
    }
    out.extend(super::vga_font_bytes());
    out.extend(pal63(pal));
    if want_sauce(doc, opts, false) {
        sauce::append(&mut out, doc, Kind::Character(CHAR_ANSI), g.width, g.height)?;
    }
    Ok(out)
}

// --- TundraDraw ------------------------------------------------------------

const TND_POS: u8 = 1;
const TND_FG: u8 = 2;
const TND_BG: u8 = 4;

pub fn load_tnd(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    ensure!(body.len() >= 9 && &body[1..9] == b"TUNDRA24", "not a TundraDraw file");
    let w = sauce::width(rec.as_ref()).unwrap_or(80);
    let mut cells: Vec<(usize, Cell)> = Vec::new();
    let (mut fg, mut bg) = (Color::LIGHT_GRAY, Color::BLACK);
    let mut pos = 0usize;
    let mut o = 9;
    let rd = |o: usize| body.get(o).copied();
    let rgb = |o: usize| Some(Color::Rgb(rd(o + 1)?, rd(o + 2)?, rd(o + 3)?));
    while let Some(mut cmd) = rd(o) {
        o += 1;
        if cmd == TND_POS {
            let Some(b) = body.get(o..o + 8) else { break };
            let (y, x) = (u32::from_be_bytes([b[0], b[1], b[2], b[3]]), u32::from_be_bytes([b[4], b[5], b[6], b[7]]));
            if x as usize >= w || y >= 20_000 {
                bail!("TundraDraw jump out of bounds ({x}, {y})");
            }
            pos = y as usize * w + x as usize;
            o += 8;
            continue;
        }
        if (2..=6).contains(&cmd) {
            let Some(ch) = rd(o) else { break };
            o += 1;
            if cmd & TND_FG != 0 {
                let Some(c) = rgb(o) else { break };
                fg = c;
                o += 4;
            }
            if cmd & TND_BG != 0 {
                let Some(c) = rgb(o) else { break };
                bg = c;
                o += 4;
            }
            cmd = ch;
        }
        if pos / w >= 20_000 {
            break;
        }
        cells.push((pos, Cell::new(acidtrip_core::cp437::to_char(cmd), fg, bg)));
        pos += 1;
    }
    let h = cells.iter().map(|&(i, _)| i / w + 1).max().unwrap_or(1);
    let mut g = Grid::new(w, h.max(25));
    for (i, c) in cells {
        g.cells[i] = c;
    }
    Ok(finish(g, rec.as_ref(), None, false, true))
}

pub fn save_tnd(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = export_grid(doc, opts);
    let pal = &doc.meta.palette;
    let mut out = vec![24u8];
    out.extend(b"TUNDRA24");
    let (mut fg, mut bg) = (Color::LIGHT_GRAY.rgb(pal), Color::BLACK.rgb(pal));
    let mut skip: Option<usize> = None;
    for (i, c) in g.cells.iter().enumerate() {
        if *c == Cell::BLANK {
            skip.get_or_insert(i);
            continue;
        }
        if let Some(s) = skip.take() {
            if i - s > 9 {
                out.push(TND_POS);
                out.extend(((i / g.width) as u32).to_be_bytes());
                out.extend(((i % g.width) as u32).to_be_bytes());
            } else {
                // Short gaps: write the blanks (they need the default colors).
                for _ in s..i {
                    encode_tnd(&Cell::BLANK, pal, &mut fg, &mut bg, &mut out);
                }
            }
        }
        encode_tnd(c, pal, &mut fg, &mut bg, &mut out);
    }
    if want_sauce(doc, opts, g.width != 80) {
        sauce::append(&mut out, doc, Kind::Character(CHAR_TUNDRA), g.width, g.height)?;
    }
    Ok(out)
}

fn encode_tnd(c: &Cell, pal: &Palette, fg: &mut [u8; 3], bg: &mut [u8; 3], out: &mut Vec<u8>) {
    let ch = char_byte(c.ch);
    let (f, b) = (c.fg.rgb(pal), c.bg.rgb(pal));
    let wf = f != *fg || (1..=6).contains(&ch);
    let wb = b != *bg;
    if wf || wb {
        out.push(if wf { TND_FG } else { 0 } | if wb { TND_BG } else { 0 });
        out.push(ch);
        if wf {
            out.extend([0, f[0], f[1], f[2]]);
        }
        if wb {
            out.extend([0, b[0], b[1], b[2]]);
        }
        (*fg, *bg) = (f, b);
    } else {
        out.push(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbin_compression_roundtrip() {
        let row: Vec<(u8, u8)> = b"aaaaabcdeeeeeeefg"
            .iter()
            .enumerate()
            .map(|(i, &c)| (c, if i < 12 { 7 } else { i as u8 }))
            .chain((0..100).map(|_| (b' ', 0)))
            .collect();
        let mut out = Vec::new();
        xbin_compress(&row, &mut out);
        assert!(out.len() < row.len() * 2);
        let mut file = b"XBIN\x1a".to_vec();
        file.extend((row.len() as u16).to_le_bytes());
        file.extend(1u16.to_le_bytes());
        file.extend([16, XB_COMPRESS]);
        file.extend(&out);
        let d = load_xbin(&file).unwrap();
        let g = d.flatten();
        for (x, &(ch, attr)) in row.iter().enumerate() {
            assert_eq!(g.get(x, 0), attr_cell(ch, attr), "x={x}");
        }
    }

    #[test]
    fn ega_palette_has_vga_colors_in_slots() {
        let e = ega64();
        for (i, &s) in EGA_SLOTS.iter().enumerate() {
            assert_eq!([dac8(e[s * 3]), dac8(e[s * 3 + 1]), dac8(e[s * 3 + 2])], VGA[i], "color {i}");
        }
    }
}
