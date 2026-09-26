//! Live editor socket: the running TUI listens on a Unix socket; clients
//! (the `acidtrip mcp` bridge) send JSON-lines `{"id","call"}` and receive
//! `{"id","result"}`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::{ToolCall, ToolRequest, ToolResult};

/// How long a connection waits for the UI thread to run a tool.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize)]
struct Req {
    id: u64,
    call: ToolCall,
}

#[derive(Serialize, Deserialize)]
struct Resp {
    id: u64,
    result: ToolResult,
}

/// Bind `<sockets_dir>/<pid>.sock` and forward requests to `tx`. Runs on
/// background threads; dropping the returned guard removes the socket.
pub fn serve(sockets_dir: &Path, tx: Sender<ToolRequest>) -> anyhow::Result<LiveGuard> {
    serve_at(&sockets_dir.join(format!("{}.sock", std::process::id())), tx)
}

/// Like [`serve`] with an explicit socket path.
pub fn serve_at(path: &Path, tx: Sender<ToolRequest>) -> anyhow::Result<LiveGuard> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            bail!("another editor is already listening on {}", path.display());
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::Builder::new().name("acidtrip-live".into()).spawn(move || {
        for stream in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let tx = tx.clone();
            let _ = std::thread::Builder::new().name("acidtrip-live-conn".into()).spawn(move || handle(stream, tx));
        }
    })?;
    Ok(LiveGuard { path: path.to_path_buf(), stop })
}

fn handle(stream: UnixStream, tx: Sender<ToolRequest>) {
    let Ok(read) = stream.try_clone() else { return };
    let mut w = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (id, result) = match serde_json::from_str::<Req>(&line) {
            Ok(req) => (req.id, dispatch(&tx, req.call)),
            Err(e) => (0, ToolResult::err(format!("bad request: {e}"))),
        };
        let Ok(mut out) = serde_json::to_string(&Resp { id, result }) else {
            break;
        };
        out.push('\n');
        if w.write_all(out.as_bytes()).and_then(|_| w.flush()).is_err() {
            break;
        }
    }
}

fn dispatch(tx: &Sender<ToolRequest>, call: ToolCall) -> ToolResult {
    let (reply, rx) = mpsc::channel();
    if tx.send(ToolRequest { call, origin: "mcp".into(), reply }).is_err() {
        return ToolResult::err("the editor is shutting down");
    }
    rx.recv_timeout(REPLY_TIMEOUT)
        .unwrap_or_else(|_| ToolResult::err("the editor did not answer within 30 s (busy in a modal dialog?)"))
}

pub struct LiveGuard {
    pub path: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so its thread exits.
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Most recently started live editor socket that accepts connections.
/// Stale sockets (nobody listening) are removed along the way.
pub fn find_live(sockets_dir: &Path, pid: Option<u32>) -> Option<PathBuf> {
    if let Some(pid) = pid {
        let p = sockets_dir.join(format!("{pid}.sock"));
        return UnixStream::connect(&p).is_ok().then_some(p);
    }
    let mut socks: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(sockets_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sock"))
        .map(|p| (std::fs::symlink_metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    socks.sort_by_key(|s| std::cmp::Reverse(s.0));
    for (_, p) in socks {
        if UnixStream::connect(&p).is_ok() {
            return Some(p);
        }
        let _ = std::fs::remove_file(&p);
    }
    None
}

pub struct LiveClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    pub path: PathBuf,
}

impl LiveClient {
    pub fn connect(path: &Path) -> anyhow::Result<LiveClient> {
        let s = UnixStream::connect(path).with_context(|| format!("connecting to {}", path.display()))?;
        s.set_read_timeout(Some(REPLY_TIMEOUT + Duration::from_secs(10)))?;
        let reader = BufReader::new(s.try_clone()?);
        Ok(LiveClient { writer: s, reader, next_id: 1, path: path.to_path_buf() })
    }

    pub fn call(&mut self, call: &ToolCall) -> anyhow::Result<ToolResult> {
        let id = self.next_id;
        self.next_id += 1;
        let mut line = serde_json::to_string(&Req { id, call: call.clone() })?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).context("the editor closed the connection")?;
        loop {
            let mut buf = String::new();
            if self.reader.read_line(&mut buf).context("reading from the editor")? == 0 {
                bail!("the editor closed the connection");
            }
            let resp: Resp = serde_json::from_str(&buf).context("bad reply from the editor")?;
            if resp.id == id {
                return Ok(resp.result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{self, ExecState};
    use serde_json::json;

    /// A fake UI thread: executes requests on its own doc.
    fn ui_thread(rx: mpsc::Receiver<ToolRequest>) -> std::thread::JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let mut env = exec::tests::Env::new();
            let mut origins = vec![];
            for req in rx {
                origins.push(req.origin.clone());
                let mut st = ExecState {
                    doc: &mut env.doc,
                    history: &mut env.history,
                    layer: env.layer,
                    fonts: &env.fonts,
                    stencils: &mut env.stencils,
                    paths: &env.paths,
                    file: &mut env.file,
                };
                let r = exec::execute(&mut st, &req.call);
                env.layer = st.layer;
                let _ = req.reply.send(r);
            }
            origins
        })
    }

    #[test]
    fn serve_call_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let ui = ui_thread(rx);
        let guard = serve(dir.path(), tx).unwrap();
        assert!(guard.path.exists());
        let found = find_live(dir.path(), None).expect("live socket");
        assert_eq!(found, guard.path);
        assert_eq!(find_live(dir.path(), Some(std::process::id())), Some(guard.path.clone()));
        let mut c = LiveClient::connect(&found).unwrap();
        let r = c.call(&ToolCall { name: "put_text".into(), args: json!({ "x": 0, "y": 0, "text": "LIVE" }) }).unwrap();
        assert!(!r.is_error, "{}", r.text);
        let r = c.call(&ToolCall { name: "get_canvas".into(), args: json!({}) }).unwrap();
        assert!(r.text.contains("LIVE"), "{}", r.text);
        let r = c.call(&ToolCall { name: "render_png".into(), args: json!({}) }).unwrap();
        assert!(r.image_png.is_some_and(|p| p.starts_with(b"\x89PNG")));
        let path = guard.path.clone();
        drop(guard);
        assert!(!path.exists());
        drop(c);
        let origins = ui.join().unwrap();
        assert_eq!(origins, vec!["mcp"; 3]);
    }

    #[test]
    fn stale_sockets_are_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("999999.sock");
        drop(UnixListener::bind(&stale).unwrap());
        assert!(stale.exists());
        assert_eq!(find_live(dir.path(), None), None);
        assert!(!stale.exists());
        assert_eq!(find_live(dir.path(), Some(999999)), None);
    }

    #[test]
    fn closed_ui_channel_errors() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        assert!(dispatch(&tx, ToolCall { name: "undo".into(), args: json!({}) }).is_error);
    }
}
