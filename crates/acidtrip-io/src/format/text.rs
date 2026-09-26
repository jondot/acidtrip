//! Text-stream formats: PCBoard, Avatar/0, ASCII, source arrays and mIRC.

use std::fmt::Write as _;

use acidtrip_core::{Cell, Color, Document, Grid, cp437};
use icy_parser_core::{AnsiParser, CommandParser, PcBoardParser};

use super::ansi::{Screen, printable, unprintable};
use super::sauce::{self, CHAR_ASCII, CHAR_AVATAR, CHAR_PCBOARD, Kind};
use super::{Format, SaveOptions, attr_byte, char_byte, classic_grid, export_grid, finish, idx16, want_sauce};

fn skippable(c: &Cell) -> bool {
    matches!(c.ch, ' ' | '\0') && c.bg == Color::BLACK
}

// --- PCBoard -----------------------------------------------------------------

pub fn load_pcb(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    let body = &body[..body.iter().position(|&b| b == 0x1A).unwrap_or(body.len())];
    let mut scr = Screen::new(sauce::width(rec.as_ref()).unwrap_or(80), false);
    PcBoardParser::new().parse(body, &mut scr);
    Ok(finish(scr.into_grid(25), rec.as_ref(), None, false, false))
}

pub fn save_pcb(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = classic_grid(doc, opts);
    let pal = &doc.meta.palette;
    let mut out = Vec::new();
    if opts.clear_screen {
        out.extend(b"@CLS@");
    }
    let mut cur: Option<u8> = None;
    for y in 0..g.height {
        let row = g.row(y);
        let last = row.iter().rposition(|c| !skippable(c));
        for c in &row[..last.map_or(0, |l| l + 1)] {
            let a = attr_byte(c, pal);
            if cur != Some(a) {
                out.extend(format!("@X{a:02X}").bytes());
                cur = Some(a);
            }
            match char_byte(c.ch) {
                b'@' => out.extend(b"@@"),
                b => out.push(printable(b)),
            }
        }
        if last.is_none_or(|l| l + 1 < g.width) && y + 1 < g.height {
            out.extend(b"\r\n");
        }
    }
    if want_sauce(doc, opts, g.width != 80) {
        sauce::append(&mut out, doc, Kind::Character(CHAR_PCBOARD), g.width, g.height)?;
    } else if opts.eof_char {
        out.push(0x1A);
    }
    Ok(out)
}

// --- Avatar/0 ----------------------------------------------------------------

const AVT_CMD: u8 = 0x16;
const AVT_REPEAT: u8 = 0x19;
const AVT_CLS: u8 = 0x0C;

pub fn load_avt(data: &[u8]) -> anyhow::Result<Document> {
    let (b, rec) = sauce::split(data);
    let mut scr = Screen::new(sauce::width(rec.as_ref()).unwrap_or(80), false);
    let mut ansi = AnsiParser::new();
    let at = |i: usize| b.get(i).copied().unwrap_or(0) as usize;
    let (mut i, mut start) = (0, 0);
    while i < b.len() {
        let c = b[i];
        if !matches!(c, AVT_CMD | AVT_REPEAT | AVT_CLS | 0x1A) {
            i += 1;
            continue;
        }
        ansi.parse(&b[start..i], &mut scr);
        match c {
            0x1A => {
                start = b.len();
                break;
            }
            AVT_CLS => {
                scr.clear();
                scr.set_attr(3);
                i += 1;
            }
            AVT_REPEAT => {
                for _ in 0..at(i + 2) {
                    scr.put(cp437::to_char(at(i + 1) as u8));
                }
                i += 3;
            }
            _ => {
                i += 2;
                match b.get(i - 1).copied().unwrap_or(0) {
                    0x01 => {
                        scr.set_attr(at(i) as u8);
                        i += 1;
                    }
                    0x02 => scr.blink = true,
                    0x03 => scr.y = scr.y.saturating_sub(1),
                    0x04 => scr.y += 1,
                    0x05 => scr.x = scr.x.saturating_sub(1),
                    0x06 => scr.x = (scr.x + 1).min(scr.width - 1),
                    0x07 => scr.fill(scr.x, scr.width, scr.y),
                    0x08 => {
                        scr.goto(at(i + 1).max(1) - 1, at(i).max(1) - 1);
                        i += 2;
                    }
                    0x0A | 0x0B => i += 5,
                    0x0C | 0x0D => {
                        let with_char = b[i - 1] == 0x0D;
                        scr.set_attr(at(i) as u8 & 0x7F);
                        let ch = if with_char { cp437::to_char(at(i + 1) as u8) } else { ' ' };
                        let o = i + 1 + usize::from(with_char);
                        let (lines, cols) = (at(o), at(o + 1));
                        let (x0, y0) = (scr.x, scr.y);
                        for y in y0..y0 + lines {
                            for x in x0..x0 + cols {
                                scr.goto(x, y);
                                scr.put(ch);
                            }
                        }
                        scr.goto(x0, y0);
                        i = o + 2;
                    }
                    0x19 => {
                        let n = at(i);
                        let pat: Vec<u8> = b.get(i + 1..i + 1 + n).unwrap_or_default().to_vec();
                        for _ in 0..at(i + 1 + n) {
                            for &p in &pat {
                                scr.put(cp437::to_char(p));
                            }
                        }
                        i += 2 + n;
                    }
                    _ => {}
                }
            }
        }
        start = i.min(b.len());
    }
    ansi.parse(&b[start.min(b.len())..], &mut scr);
    Ok(finish(scr.into_grid(25), rec.as_ref(), None, false, false))
}

pub fn save_avt(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = classic_grid(doc, opts);
    let pal = &doc.meta.palette;
    let mut out = Vec::new();
    if opts.clear_screen {
        out.push(AVT_CLS);
    }
    let mut cur: Option<u8> = None;
    for y in 0..g.height {
        let row = g.row(y);
        let last = row.iter().rposition(|c| !skippable(c));
        let row = &row[..last.map_or(0, |l| l + 1)];
        let mut x = 0;
        while x < row.len() {
            let c = &row[x];
            let a = attr_byte(c, pal);
            if cur != Some(a) {
                // Standard AVT/0 masks bit 7; bright backgrounds use ^V^B (blink).
                out.extend([AVT_CMD, 0x01, a & 0x7F]);
                if a & 0x80 != 0 {
                    out.extend([AVT_CMD, 0x02]);
                }
                cur = Some(a);
            }
            let n = row[x..].iter().take(255).take_while(|&o| o == c).count();
            let ch = char_byte(c.ch);
            let special = matches!(ch, AVT_CMD | AVT_REPEAT | AVT_CLS | 0x7F) || unprintable(ch);
            if n >= 4 || special {
                let n = if n >= 4 { n } else { 1 };
                out.extend([AVT_REPEAT, ch, n as u8]);
                x += n;
            } else {
                out.push(ch);
                x += 1;
            }
        }
        if last.is_none_or(|l| l + 1 < g.width) && y + 1 < g.height {
            out.extend(b"\r\n");
        }
    }
    if want_sauce(doc, opts, g.width != 80) {
        sauce::append(&mut out, doc, Kind::Character(CHAR_AVATAR), g.width, g.height)?;
    } else if opts.eof_char {
        out.push(0x1A);
    }
    Ok(out)
}

// --- ASCII -------------------------------------------------------------------

pub fn load_ascii(data: &[u8]) -> anyhow::Result<Document> {
    let (body, rec) = sauce::split(data);
    let body = &body[..body.iter().position(|&b| b == 0x1A).unwrap_or(body.len())];
    let utf8 = rec.is_none() && body.iter().any(|&b| b >= 0x80) && std::str::from_utf8(body).is_ok();
    let text: Vec<Vec<char>> = if utf8 {
        String::from_utf8_lossy(body).lines().map(|l| l.chars().collect()).collect()
    } else {
        body.split(|&b| b == b'\n')
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l).iter().map(|&b| cp437::to_char(b)).collect())
            .collect()
    };
    let lines: Vec<Vec<char>> = text
        .into_iter()
        .map(|l| {
            let mut out = Vec::new();
            for ch in l {
                if (ch == '○' && !utf8) || ch == '\t' {
                    // CP437 0x09 is a TAB in text files.
                    out.resize((out.len() / 8 + 1) * 8, ' ');
                } else {
                    out.push(ch);
                }
            }
            out
        })
        .collect();
    let lines = &lines[..lines.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1)];
    let w = lines.iter().map(Vec::len).max().unwrap_or(0).max(sauce::width(rec.as_ref()).unwrap_or(80));
    let mut g = Grid::new(w, lines.len().max(25));
    for (y, l) in lines.iter().enumerate() {
        for (x, &ch) in l.iter().enumerate() {
            g.set(x, y, Cell { ch, ..Cell::BLANK });
        }
    }
    Ok(finish(g, rec.as_ref(), None, utf8, false))
}

pub fn save_ascii(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let g = classic_grid(doc, opts);
    let mut out = Vec::new();
    for y in 0..g.height {
        let mut line: Vec<u8> = g.row(y).iter().map(|c| if c.ch == '\0' { b' ' } else { char_byte(c.ch) }).collect();
        if opts.ascii_optimize {
            line.truncate(line.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1));
        }
        out.extend(line);
        if y + 1 < g.height {
            out.extend(b"\r\n");
        }
    }
    if want_sauce(doc, opts, false) {
        sauce::append(&mut out, doc, Kind::Character(CHAR_ASCII), g.width, g.height)?;
    }
    Ok(out)
}

// --- Source arrays (ACiDDraw parity) -------------------------------------------

/// A valid identifier (letters, digits, `_`; never starting with a digit).
pub fn identifier(s: &str, fallback: &str) -> String {
    let mut id: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if id.trim_matches('_').is_empty() {
        id = fallback.into();
    }
    if id.starts_with(|c: char| c.is_ascii_digit()) {
        id.insert(0, '_');
    }
    id
}

pub fn save_array(doc: &Document, format: Format, opts: &SaveOptions) -> Vec<u8> {
    let g = classic_grid(doc, opts);
    let pal = &doc.meta.palette;
    let id = identifier(&opts.identifier, "AcidArt");
    let up = id.to_uppercase();
    let (w, h) = (g.width, g.height);
    let n = w * h * 2;
    let title = if doc.meta.sauce.title.is_empty() { String::new() } else { format!(" {}", doc.meta.sauce.title) };
    let rows: Vec<Vec<u8>> =
        (0..h).map(|y| g.row(y).iter().flat_map(|c| [char_byte(c.ch), attr_byte(c, pal)]).collect()).collect();
    let mut s = String::new();
    let lines = |fmt: &dyn Fn(u8) -> String, sep: &str| -> Vec<String> {
        rows.iter()
            .flat_map(|r| r.chunks(32).map(|ch| ch.iter().map(|&b| fmt(b)).collect::<Vec<_>>().join(sep)))
            .collect()
    };
    match format {
        Format::PascalArray => {
            let _ = writeln!(s, "{{{title} {w}x{h} char/attribute pairs (acidtrip) }}");
            let _ = writeln!(s, "const\n  {id}Width = {w};\n  {id}Height = {h};\n  {id}Length = {n};");
            let _ = writeln!(s, "  {id} : array [0..{}] of Byte = (", n - 1);
            let _ = writeln!(s, "    {}", lines(&|b| format!("${b:02X}"), ",").join(",\n    "));
            s.push_str("  );\n");
        }
        Format::AsmArray => {
            let _ = writeln!(s, ";{title} {w}x{h} char/attribute pairs (acidtrip)");
            let _ = writeln!(s, "{up}_WIDTH  EQU {w}\n{up}_HEIGHT EQU {h}\n{up}_LENGTH EQU {n}\n");
            let _ = writeln!(s, "{id} LABEL BYTE");
            for l in lines(&|b| format!("0{b:02X}h"), ",") {
                let _ = writeln!(s, "    DB {l}");
            }
        }
        _ => {
            let _ = writeln!(s, "/*{title} {w}x{h} char/attribute pairs (acidtrip) */");
            let _ = writeln!(s, "#define {up}_WIDTH {w}\n#define {up}_HEIGHT {h}\n#define {up}_LENGTH {n}\n");
            let _ = writeln!(s, "const unsigned char {id}[{up}_LENGTH] = {{");
            let _ = writeln!(s, "    {}", lines(&|b| format!("0x{b:02X}"), ", ").join(",\n    "));
            s.push_str("};\n");
        }
    }
    s.into_bytes()
}

// --- mIRC --------------------------------------------------------------------

/// VGA palette index → mIRC color number.
const VGA_TO_MIRC: [u8; 16] = [1, 2, 3, 10, 5, 6, 7, 15, 14, 12, 9, 11, 4, 13, 8, 0];

pub fn save_mirc(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    let g = export_grid(doc, opts);
    let pal = &doc.meta.palette;
    let mut s = String::new();
    for y in 0..g.height {
        let mut cur = None;
        for c in g.row(y) {
            let a = (VGA_TO_MIRC[idx16(c.fg, pal) as usize], VGA_TO_MIRC[idx16(c.bg, pal) as usize]);
            if cur != Some(a) {
                let _ = write!(s, "\x03{:02},{:02}", a.0, a.1);
                cur = Some(a);
            }
            s.push(if c.ch == '\0' { ' ' } else { c.ch });
        }
        s.push_str("\x0F\n");
    }
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::DocKind;

    fn doc() -> Document {
        let mut d = Document::new(DocKind::Classic, 80, 2);
        d.canvas.layers[0].cells[0] = Some(Cell::new('@', Color::Pal(14), Color::Pal(1)));
        d.canvas.layers[0].cells[1] = Some(Cell::new('▬', Color::Pal(14), Color::Pal(9)));
        for x in 2..10 {
            d.canvas.layers[0].cells[x] = Some(Cell::new('▒', Color::Pal(3), Color::Pal(12)));
        }
        d.canvas.layers[0].cells[80] = Some(Cell::new('x', Color::Pal(7), Color::BLACK));
        d
    }

    #[test]
    fn pcb_roundtrip() {
        let d = doc();
        let out = save_pcb(&d, &SaveOptions::default()).unwrap();
        assert!(out.starts_with(b"@X1E@@"));
        let back = load_pcb(&out).unwrap();
        assert_eq!(back.flatten().row(0)[0], d.flatten().row(0)[0]);
        assert_eq!(back.flatten().get(0, 1).ch, 'x');
    }

    #[test]
    fn avt_roundtrip_with_specials() {
        let d = doc();
        let out = save_avt(&d, &SaveOptions::default()).unwrap();
        let back = load_avt(&out).unwrap().flatten();
        let orig = d.flatten();
        for x in 0..80 {
            assert_eq!(back.get(x, 0), orig.get(x, 0), "x={x}");
        }
        assert_eq!(back.get(0, 1).ch, 'x');
    }

    #[test]
    fn ascii_and_arrays() {
        let d = doc();
        let out = save_ascii(&d, &SaveOptions::default()).unwrap();
        assert_eq!(&out[..2], b"@\x16");
        let back = load_ascii(b"ab\tc\r\nd").unwrap().flatten();
        assert_eq!(back.get(8, 0).ch, 'c');
        assert_eq!(back.get(0, 1).ch, 'd');
        let c = String::from_utf8(save_array(&d, Format::CArray, &SaveOptions::default())).unwrap();
        assert!(c.contains("const unsigned char AcidArt[ACIDART_LENGTH] = {\n    0x40, 0x1E,"), "{c}");
        let p = String::from_utf8(save_array(&d, Format::PascalArray, &SaveOptions::default())).unwrap();
        assert!(p.contains("AcidArt : array [0..319] of Byte = (\n    $40,$1E,"), "{p}");
        let a = String::from_utf8(save_array(&d, Format::AsmArray, &SaveOptions::default())).unwrap();
        assert!(a.contains("DB 040h,01Eh,"), "{a}");
        assert_eq!(identifier("9 lives!", "X"), "_9_lives_");
    }

    #[test]
    fn mirc_codes() {
        let s = String::from_utf8(save_mirc(&doc(), &SaveOptions::default())).unwrap();
        assert!(s.starts_with("\x0308,02@"), "{s:?}");
    }
}
