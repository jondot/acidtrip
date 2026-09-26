//! Draw together: shared drawing sessions over iroh (QUIC, dialled by
//! public key, with hole punching, n0's public relays as the fallback and
//! mDNS for LANs without internet).
//!
//! The host is the source of truth. A guest gets the full document on
//! joining, then every edit as a [`Transaction`], in the host's order. Guests
//! send their edits to the host, which applies them in arrival order and
//! sends them on to everyone (the sender included), so all copies end the
//! same: last writer wins per cell.
//!
//! Threading: the network runs on its own tokio runtime in a background
//! thread. The app talks to it through a [`Session`]: commands in, events
//! out, polled from the draw loop.

use std::future::Future;
use std::str::FromStr;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use acidtrip_core::{Document, Transaction};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::sync::mpsc;

mod guest;
mod host;
pub mod proto;

pub use proto::{HOST, PeerId, PeerInfo};

/// Colors people get, in order of joining (the host takes the first).
pub const COLORS: [[u8; 3]; 8] = [
    [255, 196, 0],
    [0, 200, 255],
    [255, 80, 160],
    [120, 230, 90],
    [180, 130, 255],
    [255, 120, 40],
    [80, 160, 255],
    [240, 240, 120],
];

#[derive(Clone, Debug)]
pub struct Options {
    /// Your name, shown to the others beside your cursor.
    pub name: String,
    /// No relays or discovery, bound to 127.0.0.1 only: for tests, which
    /// must not need the internet.
    pub localhost: bool,
}

#[derive(Debug)]
pub enum Event {
    /// Hosting: the ticket guests join with. Sent again when it changes
    /// (e.g. once a relay is reached); older tickets keep working.
    Ticket(String),
    /// Joined: the host's document and your peer id. Sent again after a
    /// reconnect (`rejoined`), replacing whatever you had.
    Welcome {
        you: PeerId,
        doc: Box<Document>,
        rejoined: bool,
    },
    /// An edit. Hosting: a guest's, to apply and pass on with
    /// [`Session::edit`]. Joined: one to apply, in the host's order.
    Edit(Transaction),
    /// Everyone in the session, the host first.
    Peers(Vec<PeerInfo>),
    Cursor {
        peer: PeerId,
        at: Option<(u32, u32)>,
    },
    /// Connection progress, for the status line.
    Status(String),
    /// The session is over (the host ended it, or it could not go on).
    Ended(String),
}

#[derive(Debug)]
pub(crate) enum Command {
    Edit(Transaction),
    Cursor(Option<(u32, u32)>),
    Leave,
}

pub struct Session {
    cmds: mpsc::UnboundedSender<Command>,
    events: std_mpsc::Receiver<Event>,
    done: std_mpsc::Receiver<()>,
    host: bool,
}

impl Session {
    /// Start hosting `doc`. The ticket arrives as [`Event::Ticket`].
    pub fn host(opts: Options, doc: Document) -> Session {
        spawn(true, move |cmds, events| host::run(opts, doc, cmds, events))
    }

    /// Join the session a ticket points at. Fails at once on a malformed
    /// ticket; connection trouble arrives as events.
    pub fn join(opts: Options, ticket: &str) -> anyhow::Result<Session> {
        let addr = parse_ticket(ticket)?;
        Ok(spawn(false, move |cmds, events| guest::run(opts, addr, cmds, events)))
    }

    pub fn is_host(&self) -> bool {
        self.host
    }

    /// Send an edit: hosting, one applied here (yours or a guest's, in the
    /// order applied); joined, one of yours for the host.
    pub fn edit(&self, tx: Transaction) {
        let _ = self.cmds.send(Command::Edit(tx));
    }

    /// Where your cursor is on the canvas (None: off the canvas).
    pub fn cursor(&self, at: Option<(u32, u32)>) {
        let _ = self.cmds.send(Command::Cursor(at));
    }

    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Leave the session, telling the others; waits briefly for that to go out.
    pub fn leave(self) {
        let _ = self.cmds.send(Command::Leave);
        let _ = self.done.recv_timeout(Duration::from_secs(2));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.cmds.send(Command::Leave);
    }
}

fn spawn<F, Fut>(host: bool, f: F) -> Session
where
    F: FnOnce(mpsc::UnboundedReceiver<Command>, std_mpsc::Sender<Event>) -> Fut + Send + 'static,
    Fut: Future<Output = ()>,
{
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = std_mpsc::channel();
    let (done_tx, done_rx) = std_mpsc::channel();
    let spawned = std::thread::Builder::new().name("acidtrip-net".into()).spawn(move || {
        match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => {
                rt.block_on(f(cmd_rx, ev_tx));
                rt.shutdown_timeout(Duration::from_secs(1));
            }
            Err(e) => {
                let _ = ev_tx.send(Event::Ended(format!("can't start networking: {e}")));
            }
        }
        let _ = done_tx.send(());
    });
    if let Err(e) = spawned {
        // No thread: report through a channel of our own.
        let (tx, rx) = std_mpsc::channel();
        let _ = tx.send(Event::Ended(format!("can't start networking: {e}")));
        return Session { cmds: cmd_tx, events: rx, done: done_rx, host };
    }
    Session { cmds: cmd_tx, events: ev_rx, done: done_rx, host }
}

/// Parse a ticket as pasted: surrounding whitespace and line breaks from
/// wrapping are ignored.
pub fn parse_ticket(s: &str) -> anyhow::Result<EndpointAddr> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let t = EndpointTicket::from_str(&clean).map_err(|_| anyhow::anyhow!("that isn't a session ticket"))?;
    Ok(t.endpoint_addr().clone())
}

/// A ticket for `addr`, kept short: relays plus a few IP addresses (IPv4
/// first); discovery finds the rest from the public key.
fn ticket(addr: &EndpointAddr) -> String {
    let relays = addr.addrs.iter().filter(|a| a.is_relay()).cloned();
    let mut ips: Vec<_> = addr.ip_addrs().copied().collect();
    ips.sort_by_key(|a| (a.is_ipv6(), a.ip().is_loopback()));
    let ips = ips.into_iter().take(4).map(TransportAddr::Ip);
    EndpointTicket::new(EndpointAddr::from_parts(addr.id, relays.chain(ips))).to_string()
}

async fn endpoint(opts: &Options, accept: bool) -> anyhow::Result<Endpoint> {
    let mut b = if opts.localhost {
        Endpoint::builder(presets::Minimal).clear_ip_transports().bind_addr("127.0.0.1:0")?
    } else {
        Endpoint::builder(presets::N0).address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder())
    };
    if accept {
        b = b.alpns(vec![proto::ALPN.to_vec()]);
    }
    Ok(b.bind().await?)
}

/// Run `fut` unless the app asks to leave first (other commands are
/// dropped meanwhile: there is no one to send them to).
async fn or_leave<T>(cmds: &mut mpsc::UnboundedReceiver<Command>, fut: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            v = &mut fut => return Some(v),
            c = cmds.recv() => if matches!(c, None | Some(Command::Leave)) {
                return None;
            },
        }
    }
}
