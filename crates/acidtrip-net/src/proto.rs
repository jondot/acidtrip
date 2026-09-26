//! The wire protocol: one bidirectional QUIC stream per guest, carrying
//! length-prefixed frames of zstd-compressed JSON messages (the same
//! encoding as a native `.acid` file, so the document snapshot a guest gets
//! on joining is exactly what saving would write).

use acidtrip_core::{Document, Transaction};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The protocol version; also in the ALPN, so mismatched builds refuse each
/// other at the handshake instead of misreading frames.
pub const VERSION: u32 = 1;
pub const ALPN: &[u8] = b"acidtrip/together/1";
/// Largest frame accepted (a big document snapshot fits comfortably).
const MAX_FRAME: usize = 256 << 20;

pub type PeerId = u32;
/// The host is always peer 0.
pub const HOST: PeerId = 0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: PeerId,
    pub name: String,
    /// Their cursor and name color on everyone's canvas.
    pub color: [u8; 3],
}

/// Guest → host.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToHost {
    Hello { version: u32, name: String },
    Edit(Transaction),
    Cursor(Option<(u32, u32)>),
    Bye,
}

/// Host → guest.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToGuest {
    /// The full document, sent once on joining; edits follow.
    Welcome {
        you: PeerId,
        doc: Box<Document>,
    },
    /// An edit in the host's order (the guest's own come back too).
    Edit(Transaction),
    Peers(Vec<PeerInfo>),
    Cursor {
        peer: PeerId,
        at: Option<(u32, u32)>,
    },
    Bye {
        reason: String,
    },
}

pub fn encode<T: Serialize>(msg: &T) -> anyhow::Result<Vec<u8>> {
    let json = serde_json::to_vec(msg)?;
    let body = zstd::encode_all(json.as_slice(), 3)?;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode<T: for<'de> Deserialize<'de>>(body: &[u8]) -> anyhow::Result<T> {
    let json = zstd::decode_all(body).context("bad frame")?;
    serde_json::from_slice(&json).context("bad message")
}

/// Read one frame's body; `None` when the stream ended cleanly.
pub async fn read_frame(r: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        bail!("frame too large ({len} bytes)");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

pub async fn write_frame(w: &mut (impl AsyncWrite + Unpin), frame: &[u8]) -> anyhow::Result<()> {
    w.write_all(frame).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{Cell, Color, DocKind, TxBuilder};

    #[tokio::test]
    async fn frames_roundtrip() {
        let doc = Document::new(DocKind::Classic, 8, 2);
        let mut b = TxBuilder::new(&doc, "draw");
        b.set(0, 3, 1, Some(Cell::new('▒', Color::Pal(12), Color::BLACK)));
        let tx = b.finish();
        let mut buf = encode(&ToGuest::Edit(tx.clone())).unwrap();
        buf.extend(encode(&ToGuest::Welcome { you: 2, doc: Box::new(doc.clone()) }).unwrap());
        let mut r = buf.as_slice();
        let a: ToGuest = decode(&read_frame(&mut r).await.unwrap().unwrap()).unwrap();
        let b: ToGuest = decode(&read_frame(&mut r).await.unwrap().unwrap()).unwrap();
        assert!(matches!(a, ToGuest::Edit(t) if t == tx));
        assert!(matches!(b, ToGuest::Welcome { you: 2, doc: d } if *d == doc));
        assert!(read_frame(&mut r).await.unwrap().is_none());
    }

    /// Animation frames travel in the welcome, and an edit made on one frame
    /// lands there for a guest looking at another.
    #[test]
    fn animated_docs_sync_by_frame() {
        let mut host = Document::new(DocKind::Classic, 8, 2);
        let mut b = TxBuilder::new(&host, "frame");
        b.insert_frame(1, host.blank_frame_canvas(), 3);
        let t = b.finish();
        host.apply(&t);
        let welcome = encode(&ToGuest::Welcome { you: 2, doc: Box::new(host.clone()) }).unwrap();
        let ToGuest::Welcome { doc: guest, .. } = decode(&welcome[4..]).unwrap() else { panic!("not a welcome") };
        let mut guest = *guest;
        assert_eq!(guest, host);
        assert_eq!((guest.frame_count(), guest.hold(1)), (2, 3));
        host.show_frame(1);
        let mut b = TxBuilder::new(&host, "draw");
        b.set(0, 1, 1, Some(Cell::new('x', Color::Pal(10), Color::BLACK)));
        let tx: acidtrip_core::Transaction = decode(&encode(&b.finish()).unwrap()[4..]).unwrap();
        host.apply(&tx);
        guest.apply(&tx);
        assert_eq!(guest.current_frame(), 0);
        assert_eq!(guest.frame_canvas(1).composite(1, 1).ch, 'x');
        assert!(guest.canvas.composite(1, 1).is_blank());
        assert_eq!(guest, host);
    }
}
