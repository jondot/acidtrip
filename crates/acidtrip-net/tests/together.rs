//! Host and guests in one process, over 127.0.0.1 with no relays: the
//! sessions converge, whatever order edits cross in.

use std::time::{Duration, Instant};

use acidtrip_core::{Cell, Color, DocKind, Document, Transaction, TxBuilder};
use acidtrip_net::{Event, HOST, Options, PeerInfo, Session};

fn opts(name: &str) -> Options {
    Options { name: name.into(), localhost: true }
}

/// What the app does with a session, minus the screen.
struct Peer {
    s: Session,
    doc: Option<Document>,
    ticket: Option<String>,
    peers: Vec<PeerInfo>,
    cursors: Vec<(u32, Option<(u32, u32)>)>,
    ended: Option<String>,
    you: Option<u32>,
}

impl Peer {
    fn new(s: Session, doc: Option<Document>) -> Peer {
        Peer { s, doc, ticket: None, peers: vec![], cursors: vec![], ended: None, you: None }
    }

    fn pump(&mut self) {
        while let Some(e) = self.s.poll() {
            match e {
                Event::Ticket(t) => self.ticket = Some(t),
                Event::Welcome { you, doc, .. } => {
                    self.you = Some(you);
                    self.doc = Some(*doc);
                }
                Event::Edit(tx) => {
                    if let Some(d) = &mut self.doc {
                        d.apply(&tx);
                    }
                    // The host sequences: what it applies, it passes on.
                    if self.s.is_host() {
                        self.s.edit(tx);
                    }
                }
                Event::Peers(p) => self.peers = p,
                Event::Cursor { peer, at } => self.cursors.push((peer, at)),
                Event::Status(_) => {}
                Event::Ended(r) => self.ended = Some(r),
            }
        }
    }

    /// Draw locally and share it, as the app does.
    fn draw(&mut self, x: usize, y: usize, ch: char) {
        let tx = paint(self.doc.as_ref().unwrap(), x, y, ch);
        self.doc.as_mut().unwrap().apply(&tx);
        self.s.edit(tx);
    }

    fn at(&self, x: usize, y: usize) -> Option<char> {
        self.doc.as_ref().unwrap().canvas.get(0, x, y).map(|c| c.ch)
    }
}

fn paint(doc: &Document, x: usize, y: usize, ch: char) -> Transaction {
    let mut b = TxBuilder::new(doc, "draw");
    b.set(0, x, y, Some(Cell::new(ch, Color::Pal(12), Color::BLACK)));
    b.finish()
}

/// Pump everyone until `done` holds (or fail after a while).
fn until(peers: &mut [&mut Peer], what: &str, done: impl Fn(&[&mut Peer]) -> bool) {
    let start = Instant::now();
    loop {
        for p in peers.iter_mut() {
            p.pump();
        }
        if done(peers) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn start() -> (Peer, String) {
    let doc = Document::new(DocKind::Classic, 16, 4);
    let mut host = Peer::new(Session::host(opts("ada"), doc.clone()), Some(doc));
    until(&mut [&mut host], "a ticket", |p| p[0].ticket.is_some());
    let t = host.ticket.clone().unwrap();
    (host, t)
}

fn join(ticket: &str, name: &str) -> Peer {
    Peer::new(Session::join(opts(name), ticket).unwrap(), None)
}

#[test]
fn guest_gets_the_document_and_edits_both_ways() {
    let (mut host, ticket) = start();
    host.draw(0, 0, 'A');
    let mut g = join(&ticket, "bob");
    until(&mut [&mut host, &mut g], "the welcome", |p| p[1].doc.is_some() && p[0].peers.len() == 2);
    assert_eq!(g.at(0, 0), Some('A'));
    assert_eq!(g.you, Some(1));
    assert_eq!(host.peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["ada", "bob"]);
    assert_ne!(host.peers[0].color, host.peers[1].color);

    host.draw(1, 0, 'B');
    g.draw(2, 0, 'C');
    until(&mut [&mut host, &mut g], "both edits", |p| p[0].at(2, 0) == Some('C') && p[1].at(1, 0) == Some('B'));
    assert_eq!(host.doc, g.doc);
}

#[test]
fn concurrent_edits_converge() {
    let (mut host, ticket) = start();
    let mut a = join(&ticket, "bob");
    let mut b = join(&ticket, "cy");
    until(&mut [&mut host, &mut a, &mut b], "everyone in", |p| {
        p[1].doc.is_some() && p[2].doc.is_some() && p[0].peers.len() == 3
    });
    // Everyone scribbles on the same cells at once, without waiting.
    for i in 0..30 {
        host.draw(i % 5, 1, 'h');
        a.draw(i % 5, 1, 'a');
        b.draw((i + 2) % 5, 1, 'b');
    }
    let settled = |p: &[&mut Peer]| p[0].doc == p[1].doc && p[1].doc == p[2].doc;
    until(&mut [&mut host, &mut a, &mut b], "convergence", settled);
    // And it stays settled: nothing was still in flight.
    std::thread::sleep(Duration::from_millis(300));
    for p in [&mut host, &mut a, &mut b] {
        p.pump();
    }
    assert!(host.doc == a.doc && a.doc == b.doc);
}

#[test]
fn cursors_and_leaving() {
    let (mut host, ticket) = start();
    let mut g = join(&ticket, "bob");
    until(&mut [&mut host, &mut g], "the welcome", |p| p[1].doc.is_some());
    g.s.cursor(Some((3, 2)));
    host.s.cursor(Some((5, 1)));
    until(&mut [&mut host, &mut g], "cursors", |p| {
        p[0].cursors.contains(&(1, Some((3, 2)))) && p[1].cursors.contains(&(HOST, Some((5, 1))))
    });

    // A guest leaving drops out of the peer list.
    let late = join(&ticket, "cy");
    let mut late = late;
    until(&mut [&mut host, &mut late], "cy in", |p| p[0].peers.len() == 3);
    late.s.leave();
    until(&mut [&mut host], "cy gone", |p| p[0].peers.len() == 2);

    // The host leaving ends it for everyone, with a reason.
    host.s.leave();
    until(&mut [&mut g], "the end", |p| p[0].ended.is_some());
    assert!(g.ended.unwrap().contains("ada ended the session"));
}

#[test]
fn bad_tickets_are_refused() {
    assert!(Session::join(opts("x"), "hello").is_err());
    assert!(Session::join(opts("x"), "").is_err());
}
