//! Detects terminal queries in the child's output stream and produces the
//! replies a real terminal would send (vt100 itself never answers).

/// A query found in the output stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// `ESC[6n` cursor position report.
    CursorPosition,
    /// `ESC[?6n` DEC extended cursor position report.
    DecCursorPosition,
    /// `ESC[5n` device status.
    DeviceStatus,
    /// `ESC[c` / `ESC[0c` primary device attributes.
    PrimaryDA,
    /// `ESC[>c` secondary device attributes.
    SecondaryDA,
    /// `ESC[14t` text area size in pixels.
    PixelSize,
    /// `ESC[16t` cell size in pixels.
    CellSize,
    /// `ESC[18t` text area size in chars.
    CharSize,
    /// `ESC]10;?` default foreground color.
    ForegroundColor,
    /// `ESC]11;?` default background color.
    BackgroundColor,
}

/// Incremental scanner. Feed chunks; it reports queries together with the
/// byte offset (within the chunk) just past the end of the query, so the
/// caller can feed the terminal emulator up to that point before replying.
#[derive(Default)]
pub struct Scanner {
    tail: Vec<u8>,
}

const MAX_TAIL: usize = 16 * 1024;

enum Seq {
    /// Complete sequence of `len` bytes, maybe a query.
    Done(usize, Option<Query>),
    /// Not complete yet.
    Partial,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns (query, end offset in `chunk`).
    pub fn scan(&mut self, chunk: &[u8]) -> Vec<(Query, usize)> {
        let tail_len = self.tail.len();
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(chunk);
        let mut found = Vec::new();
        let mut i = 0;
        while i < buf.len() {
            if buf[i] != 0x1b {
                i += 1;
                continue;
            }
            match parse_seq(&buf[i..]) {
                Seq::Done(len, q) => {
                    let end = i + len;
                    if let Some(q) = q {
                        // Always ends inside `chunk`: the tail never holds a complete sequence.
                        found.push((q, end.saturating_sub(tail_len)));
                    }
                    i = end;
                }
                Seq::Partial => {
                    if buf.len() - i <= MAX_TAIL {
                        self.tail = buf[i..].to_vec();
                    }
                    return found;
                }
            }
        }
        found
    }
}

fn parse_seq(b: &[u8]) -> Seq {
    debug_assert_eq!(b[0], 0x1b);
    let Some(&kind) = b.get(1) else {
        return Seq::Partial;
    };
    match kind {
        b'[' => {
            // CSI: params 0x30-0x3F, intermediates 0x20-0x2F, final 0x40-0x7E.
            let mut j = 2;
            while j < b.len() {
                let c = b[j];
                if (0x40..=0x7e).contains(&c) {
                    let params = &b[2..j];
                    return Seq::Done(j + 1, classify_csi(params, c));
                }
                if !(0x20..=0x3f).contains(&c) {
                    // Malformed; skip the ESC [.
                    return Seq::Done(2, None);
                }
                j += 1;
            }
            Seq::Partial
        }
        b']' | b'_' | b'P' | b'^' | b'X' => {
            // String sequences: terminated by BEL (OSC only) or ST (ESC \).
            let mut j = 2;
            while j < b.len() {
                if b[j] == 0x07 && kind == b']' {
                    return Seq::Done(j + 1, classify_osc(kind, &b[2..j]));
                }
                if b[j] == 0x1b {
                    match b.get(j + 1) {
                        None => return Seq::Partial,
                        Some(b'\\') => return Seq::Done(j + 2, classify_osc(kind, &b[2..j])),
                        // Aborted string: resume scanning at the new ESC.
                        Some(_) => return Seq::Done(j, None),
                    }
                }
                j += 1;
            }
            Seq::Partial
        }
        _ => Seq::Done(1, None),
    }
}

fn classify_csi(params: &[u8], fin: u8) -> Option<Query> {
    match (params, fin) {
        (b"6", b'n') => Some(Query::CursorPosition),
        (b"?6", b'n') => Some(Query::DecCursorPosition),
        (b"5", b'n') => Some(Query::DeviceStatus),
        (b"" | b"0", b'c') => Some(Query::PrimaryDA),
        (b">" | b">0", b'c') => Some(Query::SecondaryDA),
        (b"14", b't') => Some(Query::PixelSize),
        (b"16", b't') => Some(Query::CellSize),
        (b"18", b't') => Some(Query::CharSize),
        // ESC[?u (kitty keyboard flags query): deliberately unanswered so apps
        // conclude the protocol is unsupported once DA1 arrives.
        _ => None,
    }
}

fn classify_osc(kind: u8, body: &[u8]) -> Option<Query> {
    if kind != b']' {
        // APC (kitty graphics), DCS (XTGETTCAP ...) etc: no reply.
        return None;
    }
    match body {
        b"10;?" => Some(Query::ForegroundColor),
        b"11;?" => Some(Query::BackgroundColor),
        _ => None,
    }
}

/// Terminal state the replies depend on.
pub struct ReplyCtx {
    /// 0-based (row, col).
    pub cursor: (u16, u16),
    pub rows: u16,
    pub cols: u16,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
}

fn osc_rgb(c: [u8; 3]) -> String {
    format!("rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}", c[0], c[1], c[2])
}

pub fn reply(q: &Query, ctx: &ReplyCtx) -> Vec<u8> {
    let (r, c) = (ctx.cursor.0 as u32 + 1, ctx.cursor.1 as u32 + 1);
    match q {
        Query::CursorPosition => format!("\x1b[{r};{c}R"),
        Query::DecCursorPosition => format!("\x1b[?{r};{c};1R"),
        Query::DeviceStatus => "\x1b[0n".to_string(),
        Query::PrimaryDA => "\x1b[?62;22c".to_string(),
        Query::SecondaryDA => "\x1b[>1;10;0c".to_string(),
        Query::PixelSize => format!("\x1b[4;{};{}t", ctx.rows as u32 * 16, ctx.cols as u32 * 8),
        Query::CellSize => "\x1b[6;16;8t".to_string(),
        Query::CharSize => format!("\x1b[8;{};{}t", ctx.rows, ctx.cols),
        Query::ForegroundColor => format!("\x1b]10;{}\x1b\\", osc_rgb(ctx.fg)),
        Query::BackgroundColor => format!("\x1b]11;{}\x1b\\", osc_rgb(ctx.bg)),
    }
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qs(s: &mut Scanner, b: &[u8]) -> Vec<(Query, usize)> {
        s.scan(b)
    }

    #[test]
    fn finds_queries() {
        let mut s = Scanner::new();
        let r = qs(&mut s, b"hi\x1b[31mx\x1b[6n\x1b[?u\x1b[c");
        assert_eq!(r, vec![(Query::CursorPosition, 12), (Query::PrimaryDA, 19)]);
    }

    #[test]
    fn split_across_chunks() {
        let mut s = Scanner::new();
        assert!(qs(&mut s, b"abc\x1b[").is_empty());
        assert_eq!(qs(&mut s, b"6nzz"), vec![(Query::CursorPosition, 2)]);
        assert!(qs(&mut s, b"\x1b").is_empty());
        assert!(qs(&mut s, b"]11;?\x1b").is_empty());
        assert_eq!(qs(&mut s, b"\\"), vec![(Query::BackgroundColor, 1)]);
        assert_eq!(qs(&mut s, b"\x1b]10;?\x07"), vec![(Query::ForegroundColor, 7)]);
    }

    #[test]
    fn apc_not_answered() {
        let mut s = Scanner::new();
        assert!(qs(&mut s, b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c").len() == 1);
    }

    #[test]
    fn replies() {
        let ctx = ReplyCtx { cursor: (4, 9), rows: 40, cols: 120, fg: [200, 200, 200], bg: [12, 12, 16] };
        assert_eq!(reply(&Query::CursorPosition, &ctx), b"\x1b[5;10R");
        assert_eq!(reply(&Query::BackgroundColor, &ctx), b"\x1b]11;rgb:0c0c/0c0c/1010\x1b\\");
        assert_eq!(reply(&Query::PixelSize, &ctx), b"\x1b[4;640;960t");
    }
}
