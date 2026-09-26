//! CP437 <-> Unicode, using the "graphical" mapping for the control range
//! (0x00-0x1F, 0x7F) so every byte has a visible glyph, as on a VGA screen.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const TABLE: [char; 256] = [
    '\u{0000}', '☺', '☻', '♥', '♦', '♣', '♠', '•', '◘', '○', '◙', '♂', '♀', '♪', '♫', '☼', //
    '►', '◄', '↕', '‼', '¶', '§', '▬', '↨', '↑', '↓', '→', '←', '∟', '↔', '▲', '▼', //
    ' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/', //
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?', //
    '@', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', //
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '[', '\\', ']', '^', '_', //
    '`', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', //
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '{', '|', '}', '~', '⌂', //
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00A0}',
];

fn reverse() -> &'static HashMap<char, u8> {
    static R: OnceLock<HashMap<char, u8>> = OnceLock::new();
    R.get_or_init(|| {
        let mut m: HashMap<char, u8> = TABLE.iter().enumerate().map(|(i, &c)| (c, i as u8)).collect();
        // Common aliases seen in the wild.
        m.insert('\u{2219}', 249); // ∙ bullet operator
        m.insert('\u{03B2}', 225); // β -> ß slot
        m.insert('\u{2126}', 234); // Ω ohm sign
        m.insert('\u{00B5}', 230); // µ
        m.insert('\u{2302}', 127);
        m.insert('\u{25A0}', 254);
        m.insert('\u{2022}', 7);
        m
    })
}

/// CP437 byte -> Unicode char.
pub fn to_char(b: u8) -> char {
    TABLE[b as usize]
}

/// Unicode char -> CP437 byte, if the char exists in CP437.
pub fn from_char(c: char) -> Option<u8> {
    if c == ' ' {
        return Some(32);
    }
    reverse().get(&c).copied()
}

/// Unicode char -> CP437 byte, falling back to a visually close glyph
/// (or `?`) for chars outside CP437. Used when downsampling Modern docs.
pub fn from_char_lossy(c: char) -> u8 {
    if let Some(b) = from_char(c) {
        return b;
    }

    match c {
        '▔' | '🬂' => 223,
        '▁' | '▂' | '▃' => 220,
        '▅' | '▆' | '▇' => 219,
        '▏' | '▎' | '▍' => 221,
        '▋' | '▊' | '▉' => 219,
        '▕' => 222,
        '╭' => 218,
        '╮' => 191,
        '╰' => 192,
        '╯' => 217,
        '━' => 205,
        '┃' => 186,
        '“' | '”' | '„' => b'"',
        '‘' | '’' | '‚' => b'\'',
        '–' | '—' => b'-',
        '…' => 250,
        '⠀'..='⣿' => braille_to_shade(c),
        _ => b'?',
    }
}

fn braille_to_shade(c: char) -> u8 {
    let dots = (c as u32 - 0x2800).count_ones();
    match dots {
        0 => 32,
        1..=2 => 176,
        3..=5 => 177,
        6..=7 => 178,
        _ => 219,
    }
}

pub fn is_cp437(c: char) -> bool {
    from_char(c).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_bytes() {
        for b in 0..=255u8 {
            assert_eq!(from_char(to_char(b)), Some(b), "byte {b}");
        }
    }

    #[test]
    fn lossy_maps_rounded_corners() {
        assert_eq!(from_char_lossy('╭'), 218);
        assert_eq!(from_char_lossy('😀'), b'?');
    }
}
