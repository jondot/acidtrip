//! Hosting: accept guests, keep a copy of the document in the app's order
//! for newcomers, and pass edits and cursors on.

use std::collections::BTreeMap;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acidtrip_core::{Document, Transaction};
use iroh::endpoint::Incoming;
use iroh::{Endpoint, EndpointAddr, TransportAddr, Watcher};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::proto::{self, HOST, PeerId, PeerInfo, ToGuest, ToHost};
use crate::{COLORS, Command, Event, Options};

type Frame = Arc<Vec<u8>>;

struct Guest {
    info: PeerInfo,
    out: mpsc::UnboundedSender<Frame>,
    cursor: Option<(u32, u32)>,
}

struct State {
    /// The document as the app has it: every edit the app passed on, in order.
    doc: Document,
    me: PeerInfo,
    my_cursor: Option<(u32, u32)>,
    guests: BTreeMap<PeerId, Guest>,
    next_id: PeerId,
}

impl State {
    fn peers(&self) -> Vec<PeerInfo> {
        std::iter::once(self.me.clone()).chain(self.guests.values().map(|g| g.info.clone())).collect()
    }

    /// Send to every guest but `except`.
    fn send_all(&self, msg: &ToGuest, except: Option<PeerId>) {
        let Ok(frame) = proto::encode(msg) else {
            return;
        };
        let frame = Arc::new(frame);
        for (id, g) in &self.guests {
            if Some(*id) != except {
                let _ = g.out.send(frame.clone());
            }
        }
    }

    fn edit(&mut self, tx: Transaction) {
        self.doc.apply(&tx);
        self.send_all(&ToGuest::Edit(tx), None);
    }

    /// Admit a guest: all under one lock, so the snapshot and the edits
    /// after it line up exactly.
    fn join(&mut self, name: String, out: mpsc::UnboundedSender<Frame>) -> (PeerId, Vec<PeerInfo>) {
        let id = self.next_id;
        self.next_id += 1;
        let used: Vec<[u8; 3]> = self.peers().iter().map(|p| p.color).collect();
        let color = COLORS.iter().find(|c| !used.contains(c)).copied().unwrap_or(COLORS[id as usize % COLORS.len()]);
        let send = |m: &ToGuest| {
            if let Ok(f) = proto::encode(m) {
                let _ = out.send(Arc::new(f));
            }
        };
        send(&ToGuest::Welcome { you: id, doc: Box::new(self.doc.clone()) });
        if self.my_cursor.is_some() {
            send(&ToGuest::Cursor { peer: HOST, at: self.my_cursor });
        }
        for (gid, g) in &self.guests {
            if g.cursor.is_some() {
                send(&ToGuest::Cursor { peer: *gid, at: g.cursor });
            }
        }
        self.guests.insert(id, Guest { info: PeerInfo { id, name, color }, out, cursor: None });
        let peers = self.peers();
        self.send_all(&ToGuest::Peers(peers.clone()), None);
        (id, peers)
    }

    fn leave(&mut self, id: PeerId) -> Vec<PeerInfo> {
        self.guests.remove(&id);
        let peers = self.peers();
        self.send_all(&ToGuest::Peers(peers.clone()), None);
        peers
    }
}

pub async fn run(
    opts: Options,
    doc: Document,
    mut cmds: mpsc::UnboundedReceiver<Command>,
    events: std_mpsc::Sender<Event>,
) {
    let ep = match crate::endpoint(&opts, true).await {
        Ok(ep) => ep,
        Err(e) => {
            let _ = events.send(Event::Ended(format!("can't start the session: {e:#}")));
            return;
        }
    };
    let me = PeerInfo { id: HOST, name: opts.name.clone(), color: COLORS[0] };
    let state =
        Arc::new(Mutex::new(State { doc, me: me.clone(), my_cursor: None, guests: BTreeMap::new(), next_id: 1 }));
    let _ = events.send(Event::Peers(vec![me]));
    let mut addrs = ep.watch_addr();
    let mut last_ticket = ticket_for(&ep, &addrs.get(), &opts);
    let _ = events.send(Event::Ticket(last_ticket.clone()));
    let mut conns = JoinSet::new();
    loop {
        tokio::select! {
            c = cmds.recv() => match c {
                None | Some(Command::Leave) => break,
                Some(Command::Edit(tx)) => state.lock().unwrap().edit(tx),
                Some(Command::Cursor(at)) => {
                    let mut s = state.lock().unwrap();
                    s.my_cursor = at;
                    s.send_all(&ToGuest::Cursor { peer: HOST, at }, None);
                }
            },
            inc = ep.accept() => match inc {
                Some(inc) => {
                    conns.spawn(serve(inc, state.clone(), events.clone()));
                }
                None => break,
            },
            a = addrs.updated() => {
                if let Ok(a) = a {
                    let t = ticket_for(&ep, &a, &opts);
                    if t != last_ticket {
                        last_ticket = t.clone();
                        let _ = events.send(Event::Ticket(t));
                    }
                }
            }
            Some(_) = conns.join_next() => {}
        }
    }
    // Say goodbye, then let the writers flush before closing.
    {
        let mut s = state.lock().unwrap();
        s.send_all(&ToGuest::Bye { reason: format!("{} ended the session", opts.name) }, None);
        s.guests.clear();
    }
    let _ =
        tokio::time::timeout(Duration::from_millis(1500), async { while conns.join_next().await.is_some() {} }).await;
    conns.abort_all();
    let _ = tokio::time::timeout(Duration::from_secs(1), ep.close()).await;
}

fn ticket_for(ep: &Endpoint, addr: &EndpointAddr, opts: &Options) -> String {
    let mut addr = addr.clone();
    if opts.localhost && addr.ip_addrs().next().is_none() {
        addr.addrs.extend(ep.bound_sockets().into_iter().map(TransportAddr::Ip));
    }
    crate::ticket(&addr)
}

/// One guest's connection, from handshake to goodbye.
async fn serve(inc: Incoming, state: Arc<Mutex<State>>, events: std_mpsc::Sender<Event>) {
    let Ok(conn) = inc.await else {
        return;
    };
    let Ok((mut send, mut recv)) = conn.accept_bi().await else {
        return;
    };
    let hello = tokio::time::timeout(Duration::from_secs(15), proto::read_frame(&mut recv)).await;
    let name = match hello.ok().and_then(|r| r.ok()).flatten().and_then(|b| proto::decode::<ToHost>(&b).ok()) {
        Some(ToHost::Hello { version, name }) if version == proto::VERSION => clean_name(&name),
        Some(ToHost::Hello { .. }) => {
            if let Ok(f) = proto::encode(&ToGuest::Bye { reason: "the host runs a different acidtrip version".into() })
            {
                let _ = proto::write_frame(&mut send, &f).await;
                let _ = send.finish();
                let _ = tokio::time::timeout(Duration::from_secs(1), conn.closed()).await;
            }
            return;
        }
        _ => return,
    };
    let (out, mut outq) = mpsc::unbounded_channel::<Frame>();
    let (id, peers) = state.lock().unwrap().join(name, out);
    let _ = events.send(Event::Peers(peers));
    let writer = {
        let conn = conn.clone();
        tokio::spawn(async move {
            while let Some(f) = outq.recv().await {
                if proto::write_frame(&mut send, &f).await.is_err() {
                    return;
                }
            }
            // Dropped by the host leaving: finish, and give the guest a moment to read.
            let _ = send.finish();
            let _ = tokio::time::timeout(Duration::from_secs(1), conn.closed()).await;
        })
    };
    while let Ok(Some(body)) = proto::read_frame(&mut recv).await {
        match proto::decode::<ToHost>(&body) {
            Ok(ToHost::Edit(tx)) => {
                let _ = events.send(Event::Edit(tx));
            }
            Ok(ToHost::Cursor(at)) => {
                let mut s = state.lock().unwrap();
                if let Some(g) = s.guests.get_mut(&id) {
                    g.cursor = at;
                }
                s.send_all(&ToGuest::Cursor { peer: id, at }, Some(id));
                let _ = events.send(Event::Cursor { peer: id, at });
            }
            Ok(ToHost::Bye) | Err(_) => break,
            Ok(ToHost::Hello { .. }) => {}
        }
    }
    let still_in = state.lock().unwrap().guests.contains_key(&id);
    if still_in {
        let peers = state.lock().unwrap().leave(id);
        let _ = events.send(Event::Peers(peers));
        conn.close(0u32.into(), b"bye");
    }
    let _ = writer.await;
}

/// Names are shown on one line beside a cursor: keep them short and plain.
fn clean_name(s: &str) -> String {
    let n: String = s.chars().filter(|c| !c.is_control()).take(16).collect();
    let n = n.trim();
    if n.is_empty() { "guest".into() } else { n.to_string() }
}
