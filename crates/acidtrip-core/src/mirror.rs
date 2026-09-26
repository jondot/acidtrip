//! Glyph mirroring for flip operations (ACiDDraw's "V"/"W" flips).

/// Pairs that swap under a horizontal (left-right) flip.
const H_PAIRS: &[(char, char)] = &[
    ('▌', '▐'),
    ('┌', '┐'),
    ('└', '┘'),
    ('├', '┤'),
    ('╔', '╗'),
    ('╚', '╝'),
    ('╠', '╣'),
    ('╒', '╕'),
    ('╘', '╛'),
    ('╞', '╡'),
    ('╓', '╖'),
    ('╙', '╜'),
    ('╟', '╢'),
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('/', '\\'),
    ('«', '»'),
    ('►', '◄'),
    ('→', '←'),
    ('d', 'b'),
    ('p', 'q'),
    ('⌐', '¬'),
    ('╭', '╮'),
    ('╰', '╯'),
];

/// Pairs that swap under a vertical (top-bottom) flip.
const V_PAIRS: &[(char, char)] = &[
    ('▀', '▄'),
    ('┌', '└'),
    ('┐', '┘'),
    ('┬', '┴'),
    ('╔', '╚'),
    ('╗', '╝'),
    ('╦', '╩'),
    ('╒', '╘'),
    ('╕', '╛'),
    ('╤', '╧'),
    ('╓', '╙'),
    ('╖', '╜'),
    ('╥', '╨'),
    ('▲', '▼'),
    ('↑', '↓'),
    ('/', '\\'),
    ('∩', 'u'),
    ('╭', '╰'),
    ('╮', '╯'),
    ('\'', ','),
    ('`', ','),
    ('^', 'v'),
    ('b', 'p'),
    ('d', 'q'),
];

fn lookup(pairs: &[(char, char)], c: char) -> char {
    for &(a, b) in pairs {
        if c == a {
            return b;
        }
        if c == b {
            return a;
        }
    }
    c
}

pub fn mirror_h(c: char) -> char {
    lookup(H_PAIRS, c)
}

pub fn mirror_v(c: char) -> char {
    lookup(V_PAIRS, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_are_involutions() {
        for c in ['▀', '▌', '┌', '╔', 'x'] {
            assert_eq!(mirror_h(mirror_h(c)), c);
            assert_eq!(mirror_v(mirror_v(c)), c);
        }
        assert_eq!(mirror_v('▀'), '▄');
        assert_eq!(mirror_h('┌'), '┐');
    }

    #[test]
    fn letters_turn_upside_down() {
        // a 180° turn is both flips: b turns into q, not d
        let turn = |c| mirror_v(mirror_h(c));
        assert_eq!(turn('b'), 'q');
        assert_eq!(turn('d'), 'p');
        assert_eq!(turn('p'), 'd');
        assert_eq!(turn('q'), 'b');
        assert_eq!(mirror_v('b'), 'p');
    }
}
