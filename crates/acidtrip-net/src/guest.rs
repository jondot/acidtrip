//! Joining: connect to the host, keep the connection, and reconnect when it
//! drops (a new snapshot then replaces the document).

use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use iroh::EndpointAddr;
use iroh::endpoint::{Connection, SendStream};
use tokio::sync::mpsc;

use crate::proto::{self, ToGuest, ToHost};
use crate::{Command, Event, Options, or_leave};

/// Keep trying to reach the host this long on joining, and after losing it.
const FIRST_TRY: Duration = Duration::from_secs(30);
const RECONNECT_FOR: Duration = Duration::from_secs(300);

enum End {
    Left,
    HostEnded(String),
    Lost(String),
}

pub async fn run(
    opts: Options,
    addr: EndpointAddr,
    mut cmds: mpsc::UnboundedReceiver<Command>,
    events: std_mpsc::Sender<Event>,
) {
    let ep = match crate::endpoint(&opts, false).await {
        Ok(ep) => ep,
        Err(e) => {
            let _ = events.send(Event::Ended(format!("can't start networking: {e:#}")));
            return;
        }
    };
    let mut joined = false;
    let mut since = Instant::now();
    let mut backoff = Duration::from_millis(500);
    let mut last_err = String::new();
    loop {
        let limit = if joined { RECONNECT_FOR } else { FIRST_TRY };
        if since.elapsed() > limit {
            let msg = if joined {
                format!("lost the connection to the host ({last_err})")
            } else {
                format!("couldn't reach the host ({last_err})")
            };
            let _ = events.send(Event::Ended(msg));
            break;
        }
        let _ = events.send(Event::Status(if joined { "reconnecting…" } else { "connecting…" }.into()));
        let attempt = tokio::time::timeout(Duration::from_secs(15), ep.connect(addr.clone(), proto::ALPN));
        let Some(attempt) = or_leave(&mut cmds, attempt).await else {
            break;
        };
        match attempt {
            Ok(Ok(conn)) => match session(&conn, &opts, &mut cmds, &events, joined).await {
                End::Left => break,
                End::HostEnded(reason) => {
                    let _ = events.send(Event::Ended(reason));
                    break;
                }
                End::Lost(e) => {
                    // Only a session that got going counts as joined.
                    joined |= e != NOT_WELCOMED;
                    last_err = e;
                    since = Instant::now();
                    backoff = Duration::from_millis(500);
                }
            },
            Ok(Err(e)) => last_err = short_err(&e.to_string()),
            Err(_) => last_err = "timed out".into(),
        }
        if or_leave(&mut cmds, tokio::time::sleep(backoff)).await.is_none() {
            break;
        }
        backoff = (backoff * 2).min(Duration::from_secs(8));
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), ep.close()).await;
}

const NOT_WELCOMED: &str = "the host didn't let us in";

async fn session(
    conn: &Connection,
    opts: &Options,
    cmds: &mut mpsc::UnboundedReceiver<Command>,
    events: &std_mpsc::Sender<Event>,
    rejoined: bool,
) -> End {
    let Ok((mut send, mut recv)) = conn.open_bi().await else {
        return End::Lost("couldn't open a stream".into());
    };
    if write(&mut send, &ToHost::Hello { version: proto::VERSION, name: opts.name.clone() }).await.is_err() {
        return End::Lost("couldn't say hello".into());
    }
    // Frames are read by their own task: a read is not safe to cancel midway.
    let (in_tx, mut inq) = mpsc::unbounded_channel::<anyhow::Result<ToGuest>>();
    let reader = tokio::spawn(async move {
        loop {
            let m = match proto::read_frame(&mut recv).await {
                Ok(Some(body)) => proto::decode::<ToGuest>(&body),
                Ok(None) => Err(anyhow::anyhow!("the host closed the connection")),
                Err(e) => Err(e),
            };
            let stop = m.is_err();
            if in_tx.send(m).is_err() || stop {
                return;
            }
        }
    });
    let _abort = AbortOnDrop(reader);
    match tokio::time::timeout(Duration::from_secs(60), inq.recv()).await {
        Ok(Some(Ok(ToGuest::Welcome { you, doc }))) => {
            let _ = events.send(Event::Welcome { you, doc, rejoined });
        }
        Ok(Some(Ok(ToGuest::Bye { reason }))) => return End::HostEnded(reason),
        _ => return End::Lost(NOT_WELCOMED.into()),
    }
    loop {
        tokio::select! {
            m = inq.recv() => match m {
                Some(Ok(ToGuest::Edit(tx))) => { let _ = events.send(Event::Edit(tx)); }
                Some(Ok(ToGuest::Peers(p))) => { let _ = events.send(Event::Peers(p)); }
                Some(Ok(ToGuest::Cursor { peer, at })) => { let _ = events.send(Event::Cursor { peer, at }); }
                Some(Ok(ToGuest::Bye { reason })) => {
                    conn.close(0u32.into(), b"bye");
                    return End::HostEnded(reason);
                }
                Some(Ok(ToGuest::Welcome { .. })) => {}
                Some(Err(e)) => return End::Lost(short_err(&format!("{e:#}"))),
                None => return End::Lost("connection closed".into()),
            },
            c = cmds.recv() => match c {
                Some(Command::Edit(tx)) => {
                    if let Err(e) = write(&mut send, &ToHost::Edit(tx)).await {
                        return End::Lost(short_err(&format!("{e:#}")));
                    }
                }
                Some(Command::Cursor(at)) => {
                    if let Err(e) = write(&mut send, &ToHost::Cursor(at)).await {
                        return End::Lost(short_err(&format!("{e:#}")));
                    }
                }
                None | Some(Command::Leave) => {
                    let _ = write(&mut send, &ToHost::Bye).await;
                    let _ = send.finish();
                    let _ = tokio::time::timeout(Duration::from_millis(500), conn.closed()).await;
                    conn.close(0u32.into(), b"bye");
                    return End::Left;
                }
            },
        }
    }
}

async fn write(send: &mut SendStream, msg: &ToHost) -> anyhow::Result<()> {
    proto::write_frame(send, &proto::encode(msg)?).await
}

/// First line only: status lines are one row.
fn short_err(e: &str) -> String {
    e.lines().next().unwrap_or(e).chars().take(80).collect()
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
