//! Native `.acid` encoding: zstd-compressed JSON envelope
//! `{"format":"acidtrip","version":1,"doc":{...}}`.

use acidtrip_core::Document;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "acidtrip";
pub const VERSION: u32 = 1;
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

#[derive(Serialize)]
struct EnvelopeRef<'a> {
    format: &'a str,
    version: u32,
    doc: &'a Document,
}

#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    doc: Document,
}

/// Serialize a document to compressed native bytes.
pub fn to_bytes(doc: &Document) -> anyhow::Result<Vec<u8>> {
    let json = serde_json::to_vec(&EnvelopeRef { format: FORMAT, version: VERSION, doc })?;
    Ok(zstd::encode_all(json.as_slice(), 9)?)
}

/// Parse native bytes (zstd-compressed, or plain JSON for hand-edited files).
pub fn from_bytes(b: &[u8]) -> anyhow::Result<Document> {
    let json =
        if b.starts_with(&ZSTD_MAGIC) { zstd::decode_all(b).context("corrupt .acid (zstd)")? } else { b.to_vec() };
    let env: Envelope = serde_json::from_slice(&json).context("corrupt .acid (json)")?;
    if env.format != FORMAT {
        bail!("not an acidtrip document (format {:?})", env.format);
    }
    if env.version > VERSION {
        bail!("document version {} is newer than supported ({VERSION})", env.version);
    }
    Ok(env.doc)
}

#[cfg(test)]
mod tests {
    use acidtrip_core::{Cell, Color, DocKind};

    use super::*;

    #[test]
    fn roundtrip() {
        let mut d = Document::new(DocKind::Classic, 4, 2);
        d.canvas.layers[0].cells[1] = Some(Cell::new('A', Color::Pal(4), Color::Rgb(1, 2, 3)));
        let b = to_bytes(&d).unwrap();
        assert!(b.starts_with(&ZSTD_MAGIC));
        assert_eq!(from_bytes(&b).unwrap(), d);
        assert!(from_bytes(br#"{"format":"x","version":1,"doc":null}"#).is_err());
    }
}
