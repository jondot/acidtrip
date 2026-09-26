//! Edit log and replay. Every change that reaches a document through
//! [`History`](crate::History) (commit, undo, redo) is appended here with a
//! timestamp, so the piece can be played back the way it was drawn.
//!
//! The log only knows [`Transaction`]s: it replays them with
//! [`Document::apply`] / [`Document::revert`], so new kinds of change work
//! without touching this module.
//!
//! Memory stays bounded: recent entries sit in a small tail, older ones are
//! sealed into zstd-compressed chunks, and past [`MAX_COMPRESSED`] the oldest
//! chunks are folded into the base snapshot.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::Document;
use crate::tx::{DocChange, Transaction};

/// What happened to the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogOp {
    /// A new undo step.
    Commit,
    /// More of the current undo step (the rest of a brush stroke).
    Merge,
    /// The transaction was reverted.
    Undo,
    /// The transaction was applied again.
    Redo,
    /// Someone else's edit in a shared session: not an undo step here.
    Remote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unix time in milliseconds.
    pub t: u64,
    pub op: LogOp,
    pub tx: Transaction,
}

impl LogEntry {
    /// Replay this entry forward.
    pub fn forward(&self, doc: &mut Document) {
        match self.op {
            LogOp::Undo => doc.revert(&self.tx),
            _ => doc.apply(&self.tx),
        }
    }

    /// Step back over this entry.
    pub fn backward(&self, doc: &mut Document) {
        match self.op {
            LogOp::Undo => doc.apply(&self.tx),
            _ => doc.revert(&self.tx),
        }
    }
}

#[derive(Clone, Debug)]
struct Chunk {
    first_t: u64,
    last_t: u64,
    count: u32,
    /// zstd-compressed JSON `Vec<LogEntry>`.
    data: Vec<u8>,
}

const MAGIC: &[u8; 8] = b"ACIDLOG1";
/// Seal the tail into a compressed chunk after this many entries…
const TAIL_ENTRIES: usize = 256;
/// …or this many (estimated) bytes.
const TAIL_BYTES: usize = 1 << 20;
/// Past this, the oldest chunks fold into the base snapshot.
pub const MAX_COMPRESSED: usize = 32 << 20;
const LEVEL: i32 = 3;

/// Append-only, timestamped log of a document's edits.
#[derive(Clone, Debug, Default)]
pub struct EditLog {
    /// The document before the first logged entry (zstd JSON).
    base: Option<Vec<u8>>,
    chunks: Vec<Chunk>,
    tail: Vec<LogEntry>,
    tail_cost: usize,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn pack<T: Serialize + ?Sized>(v: &T) -> Vec<u8> {
    let json = serde_json::to_vec(v).unwrap_or_default();
    zstd::encode_all(json.as_slice(), LEVEL).unwrap_or_default()
}

fn unpack<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Option<T> {
    let json = zstd::decode_all(bytes).ok()?;
    serde_json::from_slice(&json).ok()
}

/// Rough in-memory size of a transaction.
fn cost(tx: &Transaction) -> usize {
    let doc: usize = tx
        .doc
        .iter()
        .map(|d| match d {
            DocChange::Canvas { before, after, .. } => {
                (before.layers.len() * before.width * before.height + after.layers.len() * after.width * after.height)
                    * 12
            }
            DocChange::FrameInsert { frame, .. } | DocChange::FrameRemove { frame, .. } => {
                frame.canvas.as_ref().map_or(0, |c| c.layers.len() * c.width * c.height * 12) + 64
            }
            DocChange::Frames { before, after } => {
                let size = |s: &crate::model::FrameSet| {
                    s.frames
                        .iter()
                        .filter_map(|f| f.canvas.as_ref())
                        .map(|c| c.layers.len() * c.width * c.height)
                        .sum::<usize>()
                };
                (size(before) + size(after)) * 12
            }
            _ => 2048,
        })
        .sum();
    64 + tx.label.len() + tx.cells.len() * 48 + doc
}

impl EditLog {
    pub fn new() -> Self {
        EditLog::default()
    }

    /// Nothing recorded yet.
    pub fn is_empty(&self) -> bool {
        self.base.is_none() || self.len() == 0
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.chunks.iter().map(|c| c.count as usize).sum::<usize>() + self.tail.len()
    }

    /// First and last timestamps.
    pub fn span(&self) -> Option<(u64, u64)> {
        let first = self.chunks.first().map(|c| c.first_t).or_else(|| self.tail.first().map(|e| e.t))?;
        let last = self.tail.last().map(|e| e.t).or_else(|| self.chunks.last().map(|c| c.last_t))?;
        Some((first, last))
    }

    /// Compressed bytes held (sealed chunks and base).
    pub fn compressed_size(&self) -> usize {
        self.base.as_ref().map_or(0, Vec::len) + self.chunks.iter().map(|c| c.data.len()).sum::<usize>()
    }

    /// Record `op` of `tx` on `doc`, which is still in its state *before*
    /// the change (it becomes the base snapshot for the first entry).
    pub fn record(&mut self, doc: &Document, op: LogOp, tx: Transaction) {
        self.record_at(now_ms(), doc, op, tx);
    }

    pub fn record_at(&mut self, t: u64, doc: &Document, op: LogOp, tx: Transaction) {
        if self.base.is_none() {
            self.base = Some(pack(doc));
            self.chunks.clear();
            self.tail.clear();
            self.tail_cost = 0;
        }
        let t = t.max(self.tail.last().map(|e| e.t).or_else(|| self.chunks.last().map(|c| c.last_t)).unwrap_or(0));
        self.tail_cost += cost(&tx);
        self.tail.push(LogEntry { t, op, tx });
        if self.tail.len() >= TAIL_ENTRIES || self.tail_cost >= TAIL_BYTES {
            self.seal();
        }
    }

    fn seal(&mut self) {
        if let Some(c) = chunk_of(&self.tail) {
            self.chunks.push(c);
        }
        self.tail.clear();
        self.tail_cost = 0;
        while self.compressed_size() > MAX_COMPRESSED && self.chunks.len() > 1 {
            self.fold_oldest();
        }
    }

    /// Replay the oldest chunk into the base snapshot and drop it.
    fn fold_oldest(&mut self) {
        let Some(mut doc) = self.base_doc() else { return };
        let c = self.chunks.remove(0);
        for e in unpack::<Vec<LogEntry>>(&c.data).unwrap_or_default() {
            e.forward(&mut doc);
        }
        self.base = Some(pack(&doc));
    }

    /// The document before the first entry.
    pub fn base_doc(&self) -> Option<Document> {
        unpack(self.base.as_ref()?)
    }

    /// Every entry, oldest first (decompresses the chunks).
    pub fn entries(&self) -> Vec<LogEntry> {
        let mut out = Vec::with_capacity(self.len());
        for c in &self.chunks {
            out.extend(unpack::<Vec<LogEntry>>(&c.data).unwrap_or_default());
        }
        out.extend(self.tail.iter().cloned());
        out
    }

    /// Serialized form (everything compressed), as stored in `.acid` files.
    pub fn to_bytes(&self) -> Vec<u8> {
        let Some(base) = &self.base else { return Vec::new() };
        let tail = chunk_of(&self.tail);
        let chunks: Vec<&Chunk> = self.chunks.iter().chain(tail.as_ref()).collect();
        let mut o = Vec::with_capacity(self.compressed_size() + 64);
        o.extend_from_slice(MAGIC);
        o.extend_from_slice(&(base.len() as u64).to_le_bytes());
        o.extend_from_slice(base);
        o.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
        for c in chunks {
            o.extend_from_slice(&c.first_t.to_le_bytes());
            o.extend_from_slice(&c.last_t.to_le_bytes());
            o.extend_from_slice(&c.count.to_le_bytes());
            o.extend_from_slice(&(c.data.len() as u64).to_le_bytes());
            o.extend_from_slice(&c.data);
        }
        o
    }

    /// Parse [`to_bytes`](Self::to_bytes) output. Chunks stay compressed.
    pub fn from_bytes(b: &[u8]) -> Option<EditLog> {
        let mut r = Reader(b);
        if r.take(8)? != MAGIC {
            return None;
        }
        let n = r.u64()? as usize;
        let base = r.take(n)?.to_vec();
        let count = r.u32()?;
        let mut chunks = Vec::new();
        for _ in 0..count {
            let (first_t, last_t, count) = (r.u64()?, r.u64()?, r.u32()?);
            let n = r.u64()? as usize;
            chunks.push(Chunk { first_t, last_t, count, data: r.take(n)?.to_vec() });
        }
        Some(EditLog { base: Some(base), chunks, tail: Vec::new(), tail_cost: 0 })
    }
}

fn chunk_of(entries: &[LogEntry]) -> Option<Chunk> {
    let (first, last) = (entries.first()?, entries.last()?);
    Some(Chunk { first_t: first.t, last_t: last.t, count: entries.len() as u32, data: pack(entries) })
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

// ------------------------------------------------------------- playback

/// Pauses longer than this are cut when idle time is skipped.
pub const IDLE_CAP_MS: u64 = 1000;
/// Even in real time, gaps between sessions shrink to this.
pub const SESSION_CAP_MS: u64 = 5 * 60 * 1000;
/// Time before the first entry.
const LEAD_IN_MS: u64 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineOptions {
    pub skip_idle: bool,
    /// Leave out work that was undone and never redone.
    pub hide_undone: bool,
}

impl Default for TimelineOptions {
    fn default() -> Self {
        TimelineOptions { skip_idle: true, hide_undone: false }
    }
}

/// Playback speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    Times(f64),
    /// Whatever speed makes the whole replay last this many seconds (or
    /// less: never slower than it was drawn).
    Fit(u32),
}

impl Speed {
    /// Speed-up factor for a replay of `duration_ms`.
    pub fn factor(self, duration_ms: u64) -> f64 {
        match self {
            Speed::Times(f) => f.max(0.01),
            // Long pieces speed up to fit; short ones play as drawn.
            Speed::Fit(s) => (duration_ms as f64 / (s.max(1) as f64 * 1000.0)).max(1.0),
        }
    }
}

/// A playable view of an [`EditLog`]: the document at any point of its
/// history, reached by applying or reverting entries from where it is now.
pub struct Timeline {
    base: Document,
    all: Vec<LogEntry>,
    /// Indices into `all` of the entries played.
    kept: Vec<usize>,
    /// Virtual time (ms from the start) of each kept entry.
    times: Vec<u64>,
    opts: TimelineOptions,
    doc: Document,
    /// Number of kept entries applied to `doc`.
    pos: usize,
}

impl Timeline {
    /// `None` when the log has nothing to play.
    pub fn new(log: &EditLog, opts: TimelineOptions) -> Option<Timeline> {
        if log.is_empty() {
            return None;
        }
        let base = log.base_doc()?;
        let all = log.entries();
        let mut t = Timeline { doc: base.clone(), base, all, kept: Vec::new(), times: Vec::new(), opts, pos: 0 };
        t.index();
        Some(t)
    }

    fn index(&mut self) {
        self.kept = if self.opts.hide_undone { surviving(&self.all) } else { (0..self.all.len()).collect() };
        let cap = if self.opts.skip_idle { IDLE_CAP_MS } else { SESSION_CAP_MS };
        let mut now = LEAD_IN_MS;
        let mut prev: Option<u64> = None;
        self.times = self
            .kept
            .iter()
            .map(|&i| {
                let t = self.all[i].t;
                if let Some(p) = prev {
                    now += t.saturating_sub(p).min(cap);
                }
                prev = Some(t);
                now
            })
            .collect();
    }

    pub fn options(&self) -> TimelineOptions {
        self.opts
    }

    /// Change what is played, keeping the playhead near the same moment.
    pub fn set_options(&mut self, opts: TimelineOptions) {
        if opts == self.opts {
            return;
        }
        let raw = if self.pos == 0 { 0 } else { self.kept[self.pos - 1] + 1 };
        self.opts = opts;
        self.index();
        self.doc = self.base.clone();
        self.pos = 0;
        let pos = self.kept.partition_point(|&i| i < raw);
        self.seek(pos);
    }

    /// Number of steps.
    pub fn len(&self) -> usize {
        self.kept.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// The document after `pos` steps.
    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// The shown document, to swap into a view while drawing it. Whatever
    /// is swapped in must be swapped back out before the next seek.
    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Total length in virtual ms.
    pub fn duration(&self) -> u64 {
        self.times.last().copied().unwrap_or(0)
    }

    /// Virtual time at which step `pos` is reached.
    pub fn time_at(&self, pos: usize) -> u64 {
        if pos == 0 { 0 } else { self.times[(pos - 1).min(self.times.len() - 1)] }
    }

    /// Label of the last applied step.
    pub fn label(&self) -> Option<&str> {
        let e = &self.all[*self.kept.get(self.pos.checked_sub(1)?)?];
        Some(e.tx.label.as_str())
    }

    /// Op of the last applied step.
    pub fn op(&self) -> Option<LogOp> {
        Some(self.all[*self.kept.get(self.pos.checked_sub(1)?)?].op)
    }

    /// Wall-clock time of the last applied step (unix ms).
    pub fn wall_time(&self) -> Option<u64> {
        Some(self.all[*self.kept.get(self.pos.checked_sub(1)?)?].t)
    }

    pub fn seek(&mut self, pos: usize) {
        let pos = pos.min(self.kept.len());
        while self.pos < pos {
            self.all[self.kept[self.pos]].forward(&mut self.doc);
            self.pos += 1;
        }
        while self.pos > pos {
            self.pos -= 1;
            self.all[self.kept[self.pos]].backward(&mut self.doc);
        }
        // Show the frame the last step drew on.
        let last = self.pos.checked_sub(1).map(|p| &self.all[self.kept[p]].tx);
        if let Some(i) = last.and_then(Transaction::frame).and_then(|id| self.doc.frame_index(id)) {
            self.doc.show_frame(i);
        }
    }

    /// Show the document as it was at virtual time `t`.
    pub fn seek_time(&mut self, t: u64) {
        let pos = self.times.partition_point(|&v| v <= t);
        self.seek(pos);
    }
}

/// Indices of entries whose work survives: undo/redo is simulated over
/// undo steps (a commit plus its merges); steps left undone are dropped,
/// and so are the undo/redo entries themselves.
fn surviving(all: &[LogEntry]) -> Vec<usize> {
    let mut undo: Vec<Vec<usize>> = Vec::new();
    let mut redo: Vec<Vec<usize>> = Vec::new();
    let mut others = Vec::new();
    for (i, e) in all.iter().enumerate() {
        match e.op {
            LogOp::Commit => {
                redo.clear();
                undo.push(vec![i]);
            }
            LogOp::Merge => match undo.last_mut() {
                Some(g) => g.push(i),
                None => undo.push(vec![i]),
            },
            LogOp::Undo => {
                if let Some(g) = undo.pop() {
                    redo.push(g);
                }
            }
            LogOp::Redo => {
                if let Some(g) = redo.pop() {
                    undo.push(g);
                }
            }
            LogOp::Remote => others.push(i),
        }
    }
    let mut kept: Vec<usize> = undo.into_iter().flatten().chain(others).collect();
    kept.sort_unstable();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::model::{Cell, DocKind};
    use crate::tx::TxBuilder;

    fn put(doc: &Document, x: usize, ch: char) -> Transaction {
        let mut b = TxBuilder::new(doc, "put");
        b.set(0, x, 0, Some(Cell::new(ch, Color::WHITE, Color::BLACK)));
        b.finish()
    }

    fn row(d: &Document) -> String {
        (0..d.width()).map(|x| d.canvas.composite(x, 0).ch).collect()
    }

    /// A: commit A at 0, B at 1; undo B; commit C at 2; with timestamps.
    fn sample() -> (Document, EditLog) {
        let mut d = Document::new(DocKind::Classic, 4, 1);
        let mut log = EditLog::new();
        for (t, x, ch) in [(1000, 0, 'A'), (1100, 1, 'B')] {
            let tx = put(&d, x, ch);
            log.record_at(t, &d, LogOp::Commit, tx.clone());
            d.apply(&tx);
        }
        let b = log.tail[1].tx.clone();
        log.record_at(60_000, &d, LogOp::Undo, b.clone());
        d.revert(&b);
        let tx = put(&d, 2, 'C');
        log.record_at(60_500, &d, LogOp::Commit, tx.clone());
        d.apply(&tx);
        (d, log)
    }

    #[test]
    fn plays_forward_and_back() {
        let (live, log) = sample();
        let mut tl = Timeline::new(&log, TimelineOptions::default()).unwrap();
        assert_eq!(tl.len(), 4);
        assert_eq!(row(tl.doc()), "    ");
        tl.seek(2);
        assert_eq!(row(tl.doc()), "AB  ");
        tl.seek(3);
        assert_eq!(row(tl.doc()), "A   ");
        tl.seek(4);
        assert_eq!(tl.doc(), &live);
        tl.seek(1);
        assert_eq!(row(tl.doc()), "A   ");
        tl.seek(0);
        assert_eq!(row(tl.doc()), "    ");
    }

    #[test]
    fn idle_is_skipped() {
        let (_, log) = sample();
        let tl = Timeline::new(&log, TimelineOptions::default()).unwrap();
        // lead-in, +100, +1000 (cut from ~59 s), +500
        assert_eq!(tl.duration(), 250 + 100 + 1000 + 500);
        let real = Timeline::new(&log, TimelineOptions { skip_idle: false, hide_undone: false }).unwrap();
        assert_eq!(real.duration(), 250 + 100 + 58_900 + 500);
        let mut tl = tl;
        tl.seek_time(360);
        assert_eq!(tl.pos(), 2);
    }

    #[test]
    fn hides_undone_work() {
        let (live, log) = sample();
        let mut tl = Timeline::new(&log, TimelineOptions { skip_idle: true, hide_undone: true }).unwrap();
        assert_eq!(tl.len(), 2);
        tl.seek(1);
        assert_eq!(row(tl.doc()), "A   ");
        tl.seek(2);
        assert_eq!(tl.doc(), &live);
        // Switching back keeps the playhead at the same moment.
        tl.set_options(TimelineOptions::default());
        assert_eq!(tl.pos(), 4);
        assert_eq!(tl.doc(), &live);
    }

    #[test]
    fn undo_then_redo_survives() {
        let mut d = Document::new(DocKind::Classic, 3, 1);
        let mut log = EditLog::new();
        let tx = put(&d, 0, 'A');
        log.record_at(1, &d, LogOp::Commit, tx.clone());
        d.apply(&tx);
        let more = put(&d, 1, 'a');
        log.record_at(2, &d, LogOp::Merge, more.clone());
        d.apply(&more);
        let mut step = tx.clone();
        step.merge(more);
        log.record_at(3, &d, LogOp::Undo, step.clone());
        d.revert(&step);
        log.record_at(4, &d, LogOp::Redo, step.clone());
        d.apply(&step);
        let mut tl = Timeline::new(&log, TimelineOptions { skip_idle: true, hide_undone: true }).unwrap();
        assert_eq!(tl.len(), 2);
        tl.seek(2);
        assert_eq!(tl.doc(), &d);
    }

    #[test]
    fn bytes_round_trip_and_sealing() {
        let mut d = Document::new(DocKind::Classic, 40, 20);
        let mut log = EditLog::new();
        for i in 0..600 {
            let tx = put(&d, i % 40, char::from(b'a' + (i % 26) as u8));
            log.record_at(i as u64 * 10, &d, LogOp::Commit, tx.clone());
            d.apply(&tx);
        }
        assert_eq!(log.chunks.len(), 2);
        assert_eq!(log.len(), 600);
        let back = EditLog::from_bytes(&log.to_bytes()).unwrap();
        assert_eq!(back.len(), 600);
        assert_eq!(back.span(), Some((0, 5990)));
        let mut tl = Timeline::new(&back, TimelineOptions::default()).unwrap();
        tl.seek(600);
        assert_eq!(tl.doc(), &d);
        assert!(EditLog::from_bytes(b"nope").is_none());
    }

    #[test]
    fn folds_old_chunks_into_base() {
        let mut d = Document::new(DocKind::Classic, 4, 1);
        let mut log = EditLog::new();
        for i in 0..(TAIL_ENTRIES * 3) {
            let tx = put(&d, i % 4, if i % 2 == 0 { 'x' } else { 'y' });
            log.record_at(i as u64, &d, LogOp::Commit, tx.clone());
            d.apply(&tx);
        }
        log.fold_oldest();
        assert_eq!(log.len(), TAIL_ENTRIES * 2);
        let mut tl = Timeline::new(&log, TimelineOptions::default()).unwrap();
        tl.seek(tl.len());
        assert_eq!(tl.doc(), &d);
    }

    #[test]
    fn speed_factor() {
        assert_eq!(Speed::Times(4.0).factor(1000), 4.0);
        assert_eq!(Speed::Fit(30).factor(120_000), 4.0);
        assert_eq!(Speed::Fit(30).factor(5_000), 1.0);
    }
}
