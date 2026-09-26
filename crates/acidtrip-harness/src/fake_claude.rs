//! A fake Claude Messages API: a tiny HTTP/1.1 server on 127.0.0.1 that
//! answers every request with the next canned response, so scripts can drive
//! the app's AI features without a real key.
//!
//! Responses are a JSON array, served in order, one per request. Each element
//! is a full Messages API response, or a shorthand:
//!
//! ```text
//! {"text": "..."}                                  end_turn with that text
//! {"tool": "put_text", "input": {...}}             one tool_use (stop_reason tool_use)
//! {"tools": [{"tool": .., "input": ..}, ...]}      several tool_uses in one message
//! ```
//!
//! A shorthand tool message may also carry `"text"`, sent before the tool uses.
//! Any response may carry `"delay_ms"`: the server waits that long before
//! answering, so a script can act while a request is in flight (e.g. cancel);
//! requests are answered concurrently, each with the next response in order.
//! Once the list runs out every request gets `{"text": "done"}`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

/// Internal key carrying a response's delay; stripped before it is sent.
const DELAY: &str = "_fake_delay_ms";

pub struct FakeClaude {
    addr: SocketAddr,
    /// Request bodies received so far, in order.
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeClaude {
    /// Parse a responses file (see the module docs) and start serving it.
    pub fn from_json(text: &str) -> Result<FakeClaude> {
        let v: Value = serde_json::from_str(text).context("responses are not valid JSON")?;
        let list = v.as_array().ok_or_else(|| anyhow!("responses must be a JSON array"))?;
        let responses = list
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut r = r.clone();
                let delay = r.as_object_mut().and_then(|o| o.remove("delay_ms"));
                let mut out = expand(&r, i).with_context(|| format!("response #{}", i + 1))?;
                if let Some(d) = delay {
                    let ms = d.as_u64().ok_or_else(|| anyhow!("response #{}: \"delay_ms\" must be a number", i + 1))?;
                    out[DELAY] = json!(ms);
                }
                Ok(out)
            })
            .collect::<Result<Vec<_>>>()?;
        FakeClaude::start(responses)
    }

    /// Serve `responses` (full Messages API objects) in order.
    pub fn start(responses: Vec<Value>) -> Result<FakeClaude> {
        let listener = TcpListener::bind("127.0.0.1:0").context("binding the fake Claude server")?;
        let addr = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (reqs, halt) = (requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut queue = responses.into_iter();
            let mut n = 0;
            for conn in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                n += 1;
                let resp = queue.next().unwrap_or_else(|| text_message("done", n));
                // Its own thread, like the real API: a slow (delay_ms)
                // answer the app gave up on doesn't hold up the next one.
                let reqs = reqs.clone();
                std::thread::spawn(move || {
                    if let Err(e) = serve(conn, &resp, &reqs) {
                        eprintln!("fake-claude: {e:#}");
                    }
                });
            }
        });
        Ok(FakeClaude { addr, requests, stop, thread: Some(thread) })
    }

    /// `http://127.0.0.1:PORT`, for `ANTHROPIC_BASE_URL`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Request bodies received so far.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for FakeClaude {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking accept so the thread sees `stop`.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Read one request (headers + Content-Length body), record it, reply, close.
fn serve(conn: TcpStream, resp: &Value, requests: &Mutex<Vec<String>>) -> Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut rd = BufReader::new(conn.try_clone()?);
    let mut len = 0usize;
    let mut first = true;
    loop {
        let mut line = String::new();
        if rd.read_line(&mut line)? == 0 {
            bail!("connection closed before the request ended");
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if first {
            first = false;
            continue;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().context("bad Content-Length")?;
        }
    }
    let mut body = vec![0; len];
    rd.read_exact(&mut body)?;
    requests.lock().unwrap_or_else(|e| e.into_inner()).push(String::from_utf8_lossy(&body).into_owned());
    let mut resp = resp.clone();
    if let Some(ms) = resp.as_object_mut().and_then(|o| o.remove(DELAY)).and_then(|v| v.as_u64()) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    let out = resp.to_string();
    let mut conn = conn;
    write!(
        conn,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    )?;
    conn.flush()?;
    Ok(())
}

fn message(content: Vec<Value>, stop_reason: &str, n: usize) -> Value {
    json!({
        "id": format!("msg_fake_{n}"),
        "type": "message",
        "role": "assistant",
        "model": "fake-claude",
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": { "input_tokens": 0, "output_tokens": 0 },
    })
}

fn text_message(text: &str, n: usize) -> Value {
    message(vec![json!({ "type": "text", "text": text })], "end_turn", n)
}

/// Turn a shorthand response into a full Messages API response; anything
/// else (e.g. an object with `content`) passes through untouched.
fn expand(r: &Value, i: usize) -> Result<Value> {
    let Some(obj) = r.as_object() else { bail!("expected an object, got {r}") };
    if obj.contains_key("content") {
        return Ok(r.clone());
    }
    let tools: Vec<&Value> = match (obj.get("tool"), obj.get("tools")) {
        (Some(_), Some(_)) => bail!("use either \"tool\" or \"tools\", not both"),
        (Some(_), None) => vec![r],
        (None, Some(ts)) => ts.as_array().ok_or_else(|| anyhow!("\"tools\" must be an array"))?.iter().collect(),
        (None, None) => vec![],
    };
    let text = obj.get("text").map(|t| t.as_str().ok_or_else(|| anyhow!("\"text\" must be a string"))).transpose()?;
    let mut content: Vec<Value> = text.map(|t| json!({ "type": "text", "text": t })).into_iter().collect();
    for (k, t) in tools.iter().enumerate() {
        let name = t["tool"].as_str().ok_or_else(|| anyhow!("tool use needs a \"tool\" name: {t}"))?;
        let input = t.get("input").cloned().unwrap_or_else(|| json!({}));
        content.push(json!({ "type": "tool_use", "id": format!("toolu_fake_{i}_{k}"), "name": name, "input": input }));
    }
    if content.is_empty() {
        bail!("expected \"text\", \"tool\", \"tools\" or a full response with \"content\": {r}");
    }
    Ok(message(content, if tools.is_empty() { "end_turn" } else { "tool_use" }, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthands_expand() {
        let t = expand(&json!({"text": "hi"}), 0).unwrap();
        assert_eq!(t["stop_reason"], "end_turn");
        assert_eq!(t["content"][0]["text"], "hi");
        let u = expand(&json!({"tool": "put_text", "input": {"x": 1}}), 2).unwrap();
        assert_eq!(u["stop_reason"], "tool_use");
        assert_eq!(u["content"][0]["name"], "put_text");
        assert_eq!(u["content"][0]["input"]["x"], 1);
        let m = expand(&json!({"text": "a", "tools": [{"tool": "a"}, {"tool": "b"}]}), 3).unwrap();
        let ids: Vec<&str> = m["content"].as_array().unwrap().iter().filter_map(|b| b["id"].as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        let full = json!({"content": [], "stop_reason": "end_turn"});
        assert_eq!(expand(&full, 0).unwrap(), full);
        assert!(expand(&json!({"nope": 1}), 0).is_err());
    }

    #[test]
    fn serves_in_order_then_done() {
        let fake = FakeClaude::from_json(r#"[{"text": "one"}, {"tool": "get_info"}]"#).unwrap();
        let post = |body: &str| -> Value {
            let mut c = TcpStream::connect(fake.addr).unwrap();
            write!(c, "POST /v1/messages HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).unwrap();
            assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
            serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).unwrap()
        };
        assert_eq!(post("{\"a\":1}")["content"][0]["text"], "one");
        assert_eq!(post("{}")["content"][0]["name"], "get_info");
        assert_eq!(post("{}")["content"][0]["text"], "done");
        assert_eq!(fake.request_count(), 3);
        assert_eq!(fake.requests()[0], "{\"a\":1}");
    }

    #[test]
    fn delay_is_honoured_and_stripped() {
        let fake = FakeClaude::from_json(r#"[{"text": "slow", "delay_ms": 300}]"#).unwrap();
        let t = std::time::Instant::now();
        let mut c = TcpStream::connect(fake.addr).unwrap();
        write!(c, "POST /v1/messages HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        assert!(t.elapsed() >= Duration::from_millis(300));
        let v: Value = serde_json::from_str(out.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(v["content"][0]["text"], "slow");
        assert!(v.get(DELAY).is_none() && v.get("delay_ms").is_none());
        assert!(FakeClaude::from_json(r#"[{"text": "x", "delay_ms": "soon"}]"#).is_err());
    }
}
