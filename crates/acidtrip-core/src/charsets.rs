//! ACiDDraw's 15 F-key character sets (extracted from ACIDDRAW.EXE v1.00),
//! plus user-defined sets.

use crate::cp437;

pub const ACID_SETS: [[u8; 10]; 15] = [
    [218, 191, 192, 217, 196, 179, 195, 180, 193, 194],
    [201, 187, 200, 188, 205, 186, 204, 185, 202, 203],
    [213, 184, 212, 190, 205, 179, 198, 181, 207, 209],
    [214, 183, 211, 189, 196, 186, 199, 182, 208, 210],
    [197, 206, 216, 215, 232, 233, 155, 156, 153, 239],
    [176, 177, 178, 219, 223, 220, 221, 222, 254, 250],
    [1, 2, 3, 4, 5, 6, 240, 127, 14, 15],
    [24, 25, 30, 31, 16, 17, 18, 29, 20, 21],
    [174, 175, 242, 243, 169, 170, 253, 246, 171, 172],
    [227, 241, 244, 245, 234, 157, 228, 248, 251, 252],
    [224, 225, 226, 229, 230, 231, 235, 236, 237, 238],
    [128, 135, 165, 164, 152, 159, 247, 249, 173, 168],
    [131, 132, 133, 160, 166, 134, 142, 143, 145, 146],
    [136, 137, 138, 130, 144, 140, 139, 141, 161, 158],
    [147, 148, 149, 162, 167, 150, 129, 151, 163, 154],
];

pub const ACID_SET_NAMES: [&str; 15] = [
    "single lines",
    "double lines",
    "double-h lines",
    "double-v lines",
    "crossings",
    "blocks & shades",
    "symbols",
    "arrows",
    "math 1",
    "math 2",
    "greek",
    "latin 1",
    "latin a",
    "latin e/i",
    "latin o/u",
];

/// Default active set: blocks & shades (index 5), as in Moebius/IcyDraw.
pub const DEFAULT_SET: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Charset {
    pub name: String,
    pub chars: [char; 10],
}

pub fn builtin() -> Vec<Charset> {
    ACID_SETS
        .iter()
        .zip(ACID_SET_NAMES)
        .map(|(set, name)| Charset { name: name.into(), chars: set.map(cp437::to_char) })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn set6_is_blocks() {
        let s = super::builtin();
        assert_eq!(s.len(), 15);
        assert_eq!(s[5].chars[3], '█');
        assert_eq!(s[0].chars[0], '┌');
    }
}
