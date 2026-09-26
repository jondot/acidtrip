//! Native `.acid`: zstd-compressed JSON `{"format":"acidtrip","version":1,"doc":…}`.
//!
//! The edit log (for replay) follows in a zstd *skippable frame*: plain zstd
//! readers and older acidtrip builds skip it, loading decodes only the
//! document frame, and the log's chunks stay compressed until played.

use acidtrip_core::Document;
use acidtrip_core::replay::EditLog;
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
/// Skippable-frame magic that marks the edit log.
const LOG_FRAME: u32 = 0x184D_2A5E;

#[derive(Serialize)]
struct EnvelopeRef<'a> {
    format: &'static str,
    version: u32,
    doc: &'a Document,
}

#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    doc: Document,
}

pub fn save(doc: &Document, log: Option<&EditLog>) -> anyhow::Result<Vec<u8>> {
    let json = serde_json::to_vec(&EnvelopeRef { format: "acidtrip", version: VERSION, doc })?;
    let mut out = zstd::encode_all(json.as_slice(), 9)?;
    if let Some(log) = log.filter(|l| !l.is_empty()) {
        let bytes = log.to_bytes();
        ensure!(bytes.len() <= u32::MAX as usize, "edit log too large");
        out.extend_from_slice(&LOG_FRAME.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

pub fn load(bytes: &[u8]) -> anyhow::Result<Document> {
    Ok(load_with_log(bytes)?.0)
}

/// The document and, when the file has one, its edit log.
pub fn load_with_log(bytes: &[u8]) -> anyhow::Result<(Document, Option<EditLog>)> {
    let first = zstd::zstd_safe::find_frame_compressed_size(bytes).unwrap_or(bytes.len()).min(bytes.len());
    let doc = parse(&bytes[..first])?;
    Ok((doc, find_log(&bytes[first..])))
}

/// Walk the skippable frames after the document for the log.
fn find_log(mut rest: &[u8]) -> Option<EditLog> {
    while rest.len() >= 8 {
        let magic = u32::from_le_bytes(rest[..4].try_into().ok()?);
        if magic & 0xFFFF_FFF0 != 0x184D_2A50 {
            return None;
        }
        let n = u32::from_le_bytes(rest[4..8].try_into().ok()?) as usize;
        let body = rest.get(8..8 + n)?;
        if magic == LOG_FRAME {
            return EditLog::from_bytes(body);
        }
        rest = &rest[8 + n..];
    }
    None
}

fn parse(bytes: &[u8]) -> anyhow::Result<Document> {
    let json = zstd::decode_all(bytes).context("not an .acid file, or it is damaged")?;
    let env: Envelope = serde_json::from_slice(&json).context("damaged .acid document")?;
    ensure!(env.format == "acidtrip", "not an acidtrip document ({})", env.format);
    ensure!(env.version <= VERSION, "document version {} is newer than this acidtrip supports", env.version);
    Ok(env.doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::replay::LogOp;
    use acidtrip_core::{Cell, Color, DocKind, TxBuilder};

    fn logged() -> (Document, EditLog) {
        let mut d = Document::new(DocKind::Classic, 8, 2);
        let mut log = EditLog::new();
        for x in 0..5 {
            let mut b = TxBuilder::new(&d, "put");
            b.set(0, x, 0, Some(Cell::new('#', Color::WHITE, Color::BLACK)));
            let tx = b.finish();
            log.record_at(x as u64 * 100, &d, LogOp::Commit, tx.clone());
            d.apply(&tx);
        }
        (d, log)
    }

    #[test]
    fn log_round_trip() {
        let (d, log) = logged();
        let bytes = save(&d, Some(&log)).unwrap();
        let (back, got) = load_with_log(&bytes).unwrap();
        assert_eq!(back, d);
        let got = got.unwrap();
        assert_eq!(got.len(), 5);
        assert_eq!(got.entries(), log.entries());
        // A plain zstd reader (and older acidtrip) skips the log frame.
        let json = zstd::decode_all(bytes.as_slice()).unwrap();
        let env: Envelope = serde_json::from_slice(&json).unwrap();
        assert_eq!(env.doc, d);
    }

    #[test]
    fn files_without_a_log_still_load() {
        let (d, _) = logged();
        let old = save(&d, None).unwrap();
        let (back, log) = load_with_log(&old).unwrap();
        assert_eq!(back, d);
        assert!(log.is_none());
        assert_eq!(save(&d, Some(&EditLog::new())).unwrap(), old);
    }
}
