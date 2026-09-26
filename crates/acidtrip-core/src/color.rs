//! Colors and palettes. Palette indices use the VGA/DOS attribute order
//! (0 black, 1 blue, 2 green, 3 cyan, 4 red, 5 magenta, 6 brown, 7 light gray,
//! 8-15 bright variants), which is what BIN/XBin store. ANSI SGR order is
//! different; see [`ANSI_TO_VGA`].

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Color {
    /// Index into the document palette.
    Pal(u8),
    /// Direct 24-bit color (Modern docs, or PabloDraw 24-bit extension).
    Rgb(u8, u8, u8),
}

impl Color {
    pub const BLACK: Color = Color::Pal(0);
    pub const LIGHT_GRAY: Color = Color::Pal(7);
    pub const WHITE: Color = Color::Pal(15);

    pub fn rgb(self, pal: &Palette) -> [u8; 3] {
        match self {
            Color::Pal(i) => pal.get(i),
            Color::Rgb(r, g, b) => [r, g, b],
        }
    }

    pub fn index(self) -> Option<u8> {
        match self {
            Color::Pal(i) => Some(i),
            Color::Rgb(..) => None,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::LIGHT_GRAY
    }
}

/// ANSI SGR color number (30-37 minus 30) -> VGA palette index.
pub const ANSI_TO_VGA: [u8; 8] = [0, 4, 2, 6, 1, 5, 3, 7];
/// VGA palette index (0-7) -> ANSI SGR color number.
pub const VGA_TO_ANSI: [u8; 8] = [0, 4, 2, 6, 1, 5, 3, 7];

pub const VGA: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0xAA],
    [0x00, 0xAA, 0x00],
    [0x00, 0xAA, 0xAA],
    [0xAA, 0x00, 0x00],
    [0xAA, 0x00, 0xAA],
    [0xAA, 0x55, 0x00],
    [0xAA, 0xAA, 0xAA],
    [0x55, 0x55, 0x55],
    [0x55, 0x55, 0xFF],
    [0x55, 0xFF, 0x55],
    [0x55, 0xFF, 0xFF],
    [0xFF, 0x55, 0x55],
    [0xFF, 0x55, 0xFF],
    [0xFF, 0xFF, 0x55],
    [0xFF, 0xFF, 0xFF],
];

pub const VGA_NAMES: [&str; 16] = [
    "black",
    "blue",
    "green",
    "cyan",
    "red",
    "magenta",
    "brown",
    "light gray",
    "dark gray",
    "light blue",
    "light green",
    "light cyan",
    "light red",
    "light magenta",
    "yellow",
    "white",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub name: String,
    pub colors: Vec<[u8; 3]>,
}

impl Default for Palette {
    fn default() -> Self {
        Palette { name: "VGA".into(), colors: VGA.to_vec() }
    }
}

impl Palette {
    pub fn get(&self, i: u8) -> [u8; 3] {
        self.colors.get(i as usize).copied().unwrap_or_else(|| xterm256(i))
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    /// Nearest palette index among the first `limit` entries (perceptual-ish
    /// weighted RGB distance; good enough for 16-color downsampling).
    pub fn nearest(&self, rgb: [u8; 3], limit: usize) -> u8 {
        let limit = limit.min(self.colors.len()).max(1);
        let mut best = (u32::MAX, 0u8);
        for (i, c) in self.colors.iter().take(limit).enumerate() {
            let d = color_distance(*c, rgb);
            if d < best.0 {
                best = (d, i as u8);
            }
        }
        best.1
    }

    /// Parse a palette from GIMP .gpl, JASC .pal, or .hex (one RRGGBB per line).
    pub fn parse(name: &str, text: &str) -> Option<Palette> {
        let mut colors = Vec::new();
        let t = text.trim_start_matches('\u{feff}');
        if t.starts_with("GIMP Palette") {
            for line in t.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.contains(':') {
                    continue;
                }
                let n: Vec<u8> = line.split_whitespace().take(3).filter_map(|s| s.parse().ok()).collect();
                if n.len() == 3 {
                    colors.push([n[0], n[1], n[2]]);
                }
            }
        } else if t.starts_with("JASC-PAL") {
            for line in t.lines().skip(3) {
                let n: Vec<u8> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                if n.len() == 3 {
                    colors.push([n[0], n[1], n[2]]);
                }
            }
        } else {
            for line in t.lines() {
                let h = line.trim().trim_start_matches('#');
                if h.len() == 6
                    && let Ok(v) = u32::from_str_radix(h, 16)
                {
                    colors.push([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
                }
            }
        }
        (!colors.is_empty()).then(|| Palette { name: name.into(), colors })
    }

    pub fn to_gpl(&self) -> String {
        let mut s = format!("GIMP Palette\nName: {}\nColumns: 8\n#\n", self.name);
        for (i, c) in self.colors.iter().enumerate() {
            s.push_str(&format!("{:3} {:3} {:3}\tcolor{}\n", c[0], c[1], c[2], i));
        }
        s
    }
}

/// Weighted squared RGB distance ("redmean"), cheap and decent.
pub fn color_distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let rm = (a[0] as i32 + b[0] as i32) / 2;
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    ((((512 + rm) * dr * dr) >> 8) + 4 * dg * dg + (((767 - rm) * db * db) >> 8)) as u32
}

/// RGB of an xterm-256 index (0-15 VGA, 16-231 cube, 232-255 grays).
pub fn xterm256(i: u8) -> [u8; 3] {
    match i {
        0..=15 => VGA[i as usize],
        16..=231 => {
            let i = i - 16;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [lv(i / 36), lv((i / 6) % 6), lv(i % 6)]
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_exact() {
        let p = Palette::default();
        for i in 0..16u8 {
            assert_eq!(p.nearest(VGA[i as usize], 16), i);
        }
    }

    #[test]
    fn parse_gpl_and_hex() {
        let g = Palette::parse("x", "GIMP Palette\nName: x\n#\n255 0 0 red\n0 255 0\n").unwrap();
        assert_eq!(g.colors, vec![[255, 0, 0], [0, 255, 0]]);
        let h = Palette::parse("h", "#ff0000\n00ff00\n").unwrap();
        assert_eq!(h.colors.len(), 2);
    }
}
