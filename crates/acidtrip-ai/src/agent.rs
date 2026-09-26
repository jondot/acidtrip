//! In-app prompt bar agent: Claude Messages API tool loop on a background
//! thread. Tool calls go to the UI thread as [`ToolRequest`]s.
//!
//! The HTTP layer sits behind [`Transport`] so the loop can be tested with
//! canned responses. The UI should wrap a run in `History::begin_group` /
//! `end_group` so the whole run is one undo step.

use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde_json::{Value, json};

use crate::{ToolCall, ToolRequest, ToolResult, schema};

pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MAX_TOKENS: u32 = 16000;
/// How long the agent waits for the UI thread to run one tool.
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub api_key: String,
    pub model: String,
    pub max_rounds: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// Short progress line ("drawing ellipse…").
    Status(String),
    /// Assistant prose (streamed or whole).
    Text(String),
    Done,
    Error(String),
}

pub const SYSTEM_PROMPT: &str = r##"You are an expert ANSI artist in the tradition of ACiD, iCE and Blocktronics, working inside acidtrip, a terminal ANSI art editor. You edit the user's open document only through the tools.

The medium:
- Classic documents use the 256 CP437 glyphs and a 16-color palette in VGA/DOS order: 0 black, 1 blue, 2 green, 3 cyan, 4 red, 5 magenta, 6 brown, 7 light gray, 8 dark gray, 9 light blue, 10 light green, 11 light cyan, 12 light red, 13 light magenta, 14 yellow, 15 white. This is not ANSI SGR order. Backgrounds 8-15 need iCE colors (get_info tells you). Modern documents accept any Unicode char and "#rrggbb".
- Standard canvases are 80 columns wide. Cells are twice as tall as they are wide.
- The shading ramp is space ░ ▒ ▓ █. Half blocks ▀ ▄ ▌ ▐ give sub-cell detail. The pixel_* tools treat every cell as two square pixels (top and bottom half), which is the best way to draw round or picture-like shapes. brush_stroke paints freehand curves like the user's pen: organic outlines (ink, brush pen, calligraphy), soft glows and shading over existing art (airbrush, soft shade), texture (chalk, spray).
- Scene style: strong silhouettes, deliberate color ramps (for example 1→9→11→15 or 4→12→14→15), dithered transitions with ░▒▓, clean outlines and no stray noise. Lettering and logos come from banner (TheDraw/FIGlet fonts; list_fonts first) or hand-drawn blocks.

How to work:
- Look before you draw: the first user message has a render of the canvas. Use get_info and get_canvas (it has a column ruler) for exact coordinates.
- Plan the composition, then draw with as few, large tool calls as you can: fill_rect, draw_box, pixel_rect, pixel_ellipse and set_cells with many cells at once beat hundreds of single-cell calls.
- After a meaningful batch of drawing, call render_png to see the result, then fix what looks wrong. Do this at least once before you finish.
- Keep edits inside the region the user asked about (a selection, if given, is the region). Don't clear or redraw unrelated parts of the canvas.
- Every tool call is one undo step for the user, so do not undo the user's work unless asked.
- When you are done, reply with one or two short sentences about what you drew. No markdown headings."##;

/// The HTTP layer: POST a Messages API request body, return the response JSON.
pub trait Transport: Send {
    fn post(&self, body: &Value) -> Result<Value>;
}

/// Real transport over HTTPS (`ANTHROPIC_BASE_URL` overrides the endpoint host).
pub struct HttpTransport {
    api_key: String,
    url: String,
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new(api_key: &str) -> Self {
        let url = std::env::var("ANTHROPIC_BASE_URL")
            .map_or_else(|_| API_URL.to_string(), |b| format!("{}/v1/messages", b.trim_end_matches('/')));
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .new_agent();
        HttpTransport { api_key: api_key.to_string(), url, agent }
    }

    fn post_once(&self, body: &Value) -> std::result::Result<Value, (bool, anyhow::Error, Option<u64>)> {
        let mut resp = self
            .agent
            .post(&self.url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .send_json(body)
            .map_err(|e| (true, anyhow!("network error talking to the Claude API: {e}"), None))?;
        let status = resp.status().as_u16();
        let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok());
        let v: Value = resp.body_mut().with_config().limit(64 << 20).read_json().unwrap_or(Value::Null);
        if status == 200 {
            return Ok(v);
        }
        let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
        let err = match status {
            401 => anyhow!("invalid API key (set ai.api_key in config.toml or ANTHROPIC_API_KEY)"),
            403 => anyhow!("the API key is not allowed to use this model: {msg}"),
            404 => anyhow!("unknown model or endpoint: {msg} (check ai.model in config.toml)"),
            413 => anyhow!("request too large: {msg}"),
            429 => anyhow!("rate limited by the Claude API, try again shortly: {msg}"),
            529 => anyhow!("the Claude API is overloaded, try again shortly"),
            s => anyhow!("Claude API error {s}: {msg}"),
        };
        Err((matches!(status, 429 | 500..=599), err, retry_after))
    }
}

impl Transport for HttpTransport {
    fn post(&self, body: &Value) -> Result<Value> {
        let mut attempt = 0;
        loop {
            match self.post_once(body) {
                Ok(v) => return Ok(v),
                Err((retry, e, after)) if retry && attempt < 2 => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_secs(after.unwrap_or(2 << attempt).min(20)));
                    let _ = e;
                }
                Err((_, e, _)) => return Err(e),
            }
        }
    }
}

/// Start a run. `context` is extra text (selection bounds, doc kind…).
pub fn spawn(
    cfg: AgentConfig,
    prompt: String,
    context: String,
    requests: Sender<ToolRequest>,
    events: Sender<AgentEvent>,
) -> JoinHandle<()> {
    let transport = HttpTransport::new(&cfg.api_key);
    spawn_with(cfg, Box::new(transport), prompt, context, requests, events)
}

/// [`spawn`] with an explicit transport.
pub fn spawn_with(
    cfg: AgentConfig,
    transport: Box<dyn Transport>,
    prompt: String,
    context: String,
    requests: Sender<ToolRequest>,
    events: Sender<AgentEvent>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("acidtrip-agent".into())
        .spawn(move || run(&cfg, transport.as_ref(), &prompt, &context, &requests, &events))
        .expect("spawn agent thread")
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn image_block(png: &[u8]) -> Value {
    json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": b64(png) } })
}

/// Tool definitions with a cache breakpoint; the system prompt carries the
/// other one so tools + system are cached together.
fn request_body(cfg: &AgentConfig, system: &str, tools: &[Value], messages: &[Value]) -> Value {
    let mut msgs = messages.to_vec();
    // Rolling breakpoint on the newest block: each round reuses the last one's prefix.
    if let Some(last) = msgs.last_mut().and_then(|m| m["content"].as_array_mut()).and_then(|c| c.last_mut()) {
        last["cache_control"] = json!({ "type": "ephemeral" });
    }
    let mut body = json!({
        "model": cfg.model,
        "max_tokens": MAX_TOKENS,
        "system": [{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }],
        "messages": msgs,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    body
}

/// Send one tool call to the UI thread and wait for the result. `None` when
/// the UI is gone.
pub fn request_tool(requests: &Sender<ToolRequest>, call: ToolCall, origin: &str) -> Option<ToolResult> {
    let (reply, rx) = mpsc::channel();
    requests.send(ToolRequest { call, origin: origin.into(), reply }).ok()?;
    match rx.recv_timeout(TOOL_TIMEOUT) {
        Ok(r) => Some(r),
        Err(mpsc::RecvTimeoutError::Timeout) => Some(ToolResult::err("the editor did not run the tool in time")),
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

fn tool_result_block(id: &str, r: &ToolResult) -> Value {
    let mut content = vec![json!({ "type": "text", "text": if r.text.is_empty() { "ok" } else { r.text.as_str() } })];
    if let Some(png) = &r.image_png {
        content.push(image_block(png));
    }
    json!({ "type": "tool_result", "tool_use_id": id, "content": content, "is_error": r.is_error })
}

/// Run the tool loop to completion on the current thread. Always ends with
/// `Done` or `Error`.
pub fn run(
    cfg: &AgentConfig,
    transport: &dyn Transport,
    prompt: &str,
    context: &str,
    requests: &Sender<ToolRequest>,
    events: &Sender<AgentEvent>,
) {
    match run_inner(cfg, transport, prompt, context, requests, events) {
        Ok(()) => {
            let _ = events.send(AgentEvent::Done);
        }
        Err(e) => {
            let _ = events.send(AgentEvent::Error(format!("{e:#}")));
        }
    }
}

fn run_inner(
    cfg: &AgentConfig,
    transport: &dyn Transport,
    prompt: &str,
    context: &str,
    requests: &Sender<ToolRequest>,
    events: &Sender<AgentEvent>,
) -> Result<()> {
    let status = |s: String| {
        let _ = events.send(AgentEvent::Status(s));
    };
    status("ai: looking at the canvas…".into());
    let render = request_tool(requests, ToolCall { name: "render_png".into(), args: json!({}) }, "ai")
        .ok_or_else(|| anyhow!("the editor closed"))?;
    if render.is_cancelled() {
        bail!(crate::CANCELLED);
    }
    let mut first = vec![];
    if let Some(png) = render.image_png.as_deref().filter(|_| !render.is_error) {
        first.push(image_block(png));
    }
    let context = if context.trim().is_empty() { String::new() } else { format!("Context:\n{}\n\n", context.trim()) };
    first.push(json!({ "type": "text", "text": format!("{context}The image is the current canvas ({}).\n\nRequest: {prompt}", render.text) }));
    let mut messages = vec![json!({ "role": "user", "content": first })];
    let tools = schema::tool_definitions();

    for round in 1..=cfg.max_rounds.max(1) {
        status(if round == 1 { "ai: thinking…".into() } else { format!("ai: thinking… (round {round})") });
        let resp = transport.post(&request_body(cfg, SYSTEM_PROMPT, &tools, &messages))?;
        let content = resp["content"].as_array().cloned().unwrap_or_default();
        let stop = resp["stop_reason"].as_str().unwrap_or("");
        messages.push(json!({ "role": "assistant", "content": content }));
        let mut results = vec![];
        for block in &content {
            match block["type"].as_str() {
                Some("text") => {
                    let t = block["text"].as_str().unwrap_or("").trim();
                    if !t.is_empty() {
                        let _ = events.send(AgentEvent::Text(t.to_string()));
                    }
                }
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or("").to_string();
                    let id = block["id"].as_str().unwrap_or("").to_string();
                    status(format!("ai: {name}…"));
                    let call = ToolCall { name, args: block["input"].clone() };
                    let r = request_tool(requests, call, "ai").ok_or_else(|| anyhow!("the editor closed"))?;
                    // Stopped by the user: no more tools, no more API rounds.
                    if r.is_cancelled() {
                        bail!(crate::CANCELLED);
                    }
                    results.push(tool_result_block(&id, &r));
                }
                _ => {}
            }
        }
        match stop {
            "refusal" => bail!("Claude declined this request"),
            "max_tokens" if results.is_empty() => bail!("the response was cut off (max_tokens)"),
            "pause_turn" => continue,
            _ => {}
        }
        if results.is_empty() {
            return Ok(());
        }
        messages.push(json!({ "role": "user", "content": results }));
        if round == cfg.max_rounds.max(1) {
            status(format!("ai: stopped after {round} rounds (ai.max_tool_rounds)"));
        }
    }
    Ok(())
}

/// Single request with optional image, returns the concatenated text (used
/// by the harvester to read letters).
pub fn one_shot(cfg: &AgentConfig, system: &str, user_text: &str, image_png: Option<&[u8]>) -> Result<String> {
    one_shot_with(&HttpTransport::new(&cfg.api_key), cfg, system, user_text, image_png)
}

pub fn one_shot_with(
    transport: &dyn Transport,
    cfg: &AgentConfig,
    system: &str,
    user_text: &str,
    image_png: Option<&[u8]>,
) -> Result<String> {
    let mut content: Vec<Value> = image_png.map(image_block).into_iter().collect();
    content.push(json!({ "type": "text", "text": user_text }));
    let resp = transport
        .post(&request_body(cfg, system, &[], &[json!({ "role": "user", "content": content })]))
        .context("Claude request failed")?;
    if resp["stop_reason"] == "refusal" {
        bail!("Claude declined this request");
    }
    let text: Vec<&str> = resp["content"]
        .as_array()
        .map(|c| c.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
        .unwrap_or_default();
    Ok(text.join("\n"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::exec::{self, ExecState};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// Canned responses; records every request body.
    #[derive(Clone, Default)]
    pub struct Mock {
        pub responses: Arc<Mutex<VecDeque<Result<Value, String>>>>,
        pub requests: Arc<Mutex<Vec<Value>>>,
    }

    impl Mock {
        pub fn new(responses: Vec<Value>) -> Mock {
            Mock { responses: Arc::new(Mutex::new(responses.into_iter().map(Ok).collect())), ..Default::default() }
        }
    }

    impl Transport for Mock {
        fn post(&self, body: &Value) -> Result<Value> {
            self.requests.lock().unwrap().push(body.clone());
            match self.responses.lock().unwrap().pop_front() {
                Some(Ok(v)) => Ok(v),
                Some(Err(e)) => Err(anyhow!(e)),
                None => Ok(json!({ "content": [{ "type": "text", "text": "done" }], "stop_reason": "end_turn" })),
            }
        }
    }

    pub fn cfg() -> AgentConfig {
        AgentConfig { api_key: "test".into(), model: "claude-sonnet-5".into(), max_rounds: 5 }
    }

    /// Fake UI thread executing requests on a fresh 20x10 doc.
    type Ui = JoinHandle<(Vec<(String, String)>, acidtrip_core::Document)>;

    fn ui() -> (Sender<ToolRequest>, Ui) {
        let (tx, rx) = mpsc::channel::<ToolRequest>();
        let h = std::thread::spawn(move || {
            let mut env = exec::tests::Env::new();
            let mut seen = vec![];
            for req in rx {
                seen.push((req.origin.clone(), req.call.name.clone()));
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
                let _ = req.reply.send(r);
            }
            (seen, env.doc)
        });
        (tx, h)
    }

    #[test]
    fn tool_loop_feeds_results_back() {
        let mock = Mock::new(vec![
            json!({ "content": [
                { "type": "text", "text": "Drawing a label." },
                { "type": "tool_use", "id": "t1", "name": "put_text", "input": { "x": 1, "y": 1, "text": "OK", "fg": 14 } },
                { "type": "tool_use", "id": "t2", "name": "render_png", "input": {} },
            ], "stop_reason": "tool_use" }),
            json!({ "content": [{ "type": "text", "text": "Added a yellow OK." }], "stop_reason": "end_turn" }),
        ]);
        let (tx, ui_h) = ui();
        let (etx, erx) = mpsc::channel();
        let h = spawn_with(cfg(), Box::new(mock.clone()), "write OK".into(), "selection: none".into(), tx, etx);
        h.join().unwrap();
        let events: Vec<AgentEvent> = erx.iter().collect();
        let (seen, doc) = ui_h.join().unwrap();
        assert_eq!(doc.canvas.composite(1, 1).ch, 'O');
        assert_eq!(seen.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(), ["render_png", "put_text", "render_png"]);
        assert!(seen.iter().all(|s| s.0 == "ai"));
        assert!(events.contains(&AgentEvent::Status("ai: put_text…".into())));
        assert!(events.contains(&AgentEvent::Text("Drawing a label.".into())));
        assert!(events.contains(&AgentEvent::Text("Added a yellow OK.".into())));
        assert_eq!(events.last(), Some(&AgentEvent::Done));

        let reqs = mock.requests.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        let first = &reqs[0];
        assert_eq!(first["model"], "claude-sonnet-5");
        assert_eq!(first["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(first["tools"].as_array().unwrap().len() > 20);
        let c0 = &first["messages"][0]["content"];
        assert_eq!(c0[0]["type"], "image");
        assert!(c0[1]["text"].as_str().unwrap().contains("selection: none"));
        let second = &reqs[1]["messages"];
        assert_eq!(second[1]["role"], "assistant");
        let results = second[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2, "all tool results in one user message");
        assert_eq!(results[0]["tool_use_id"], "t1");
        assert_eq!(results[0]["is_error"], false);
        assert_eq!(results[1]["content"][1]["type"], "image");
        // Only the newest block carries the rolling breakpoint.
        assert!(second[0]["content"][1].get("cache_control").is_none());
        assert_eq!(results[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn api_errors_and_round_limit() {
        let mock = Mock {
            responses: Arc::new(Mutex::new(VecDeque::from([Err("invalid API key".to_string())]))),
            ..Default::default()
        };
        let (tx, ui_h) = ui();
        let (etx, erx) = mpsc::channel();
        run(&cfg(), &mock, "x", "", &tx, &etx);
        drop(tx);
        ui_h.join().unwrap();
        let events: Vec<AgentEvent> = erx.try_iter().collect();
        assert!(matches!(events.last(), Some(AgentEvent::Error(e)) if e.contains("invalid API key")), "{events:?}");

        let looping = json!({ "content": [{ "type": "tool_use", "id": "t", "name": "get_info", "input": {} }], "stop_reason": "tool_use" });
        let mock = Mock::new(vec![looping.clone(), looping.clone(), looping]);
        let (tx, ui_h) = ui();
        let (etx, erx) = mpsc::channel();
        run(&AgentConfig { max_rounds: 2, ..cfg() }, &mock, "x", "", &tx, &etx);
        drop(tx);
        ui_h.join().unwrap();
        assert_eq!(mock.requests.lock().unwrap().len(), 2);
        let events: Vec<AgentEvent> = erx.try_iter().collect();
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Status(s) if s.contains("stopped after 2"))));
        assert_eq!(events.last(), Some(&AgentEvent::Done));
    }

    #[test]
    fn a_run_the_ui_let_go_of_mid_request_stops_at_its_next_tool() {
        // The user stops while the API is answering: the app drops the run's
        // tool channel at once. The late answer's tools go nowhere, and no
        // more rounds are made.
        let tool = json!({ "content": [
            { "type": "tool_use", "id": "a", "name": "put_text", "input": { "x": 0, "y": 0, "text": "A" } },
        ], "stop_reason": "tool_use" });
        let mock = Mock::new(vec![tool.clone(), tool]);
        let (tx, rx) = mpsc::channel::<ToolRequest>();
        let ui = std::thread::spawn(move || {
            let req = rx.recv().unwrap();
            let name = req.call.name.clone();
            drop(rx);
            let _ = req.reply.send(ToolResult::ok("canvas"));
            name
        });
        let (etx, erx) = mpsc::channel();
        run(&cfg(), &mock, "x", "", &tx, &etx);
        assert_eq!(ui.join().unwrap(), "render_png", "only the first look reached the UI");
        assert_eq!(mock.requests.lock().unwrap().len(), 1, "no API round after the UI let go");
        let events: Vec<AgentEvent> = erx.try_iter().collect();
        assert!(matches!(events.last(), Some(AgentEvent::Error(e)) if e.contains("the editor closed")), "{events:?}");
    }

    #[test]
    fn a_cancelled_tool_ends_the_run() {
        let two = json!({ "content": [
            { "type": "tool_use", "id": "a", "name": "put_text", "input": { "x": 0, "y": 0, "text": "A" } },
            { "type": "tool_use", "id": "b", "name": "put_text", "input": { "x": 0, "y": 1, "text": "B" } },
        ], "stop_reason": "tool_use" });
        let mock = Mock::new(vec![two.clone(), two]);
        // The UI answers the first render, then the user stops the run.
        let (tx, rx) = mpsc::channel::<ToolRequest>();
        let ui = std::thread::spawn(move || {
            let mut n = 0;
            for req in rx {
                n += 1;
                let _ = req.reply.send(if n == 1 { ToolResult::ok("canvas") } else { ToolResult::cancelled() });
            }
            n
        });
        let (etx, erx) = mpsc::channel();
        run(&cfg(), &mock, "x", "", &tx, &etx);
        drop(tx);
        assert_eq!(ui.join().unwrap(), 2, "the second tool of the batch is never asked for");
        assert_eq!(mock.requests.lock().unwrap().len(), 1, "no API round after the stop");
        let events: Vec<AgentEvent> = erx.try_iter().collect();
        assert!(matches!(events.last(), Some(AgentEvent::Error(e)) if e == crate::CANCELLED), "{events:?}");
    }

    #[test]
    fn one_shot_returns_text() {
        let mock =
            Mock::new(vec![json!({ "content": [{ "type": "text", "text": "{\"a\":1}" }], "stop_reason": "end_turn" })]);
        let out = one_shot_with(&mock, &cfg(), "sys", "hi", Some(b"\x89PNG")).unwrap();
        assert_eq!(out, "{\"a\":1}");
        let req = &mock.requests.lock().unwrap()[0];
        assert!(req.get("tools").is_none());
        assert_eq!(req["messages"][0]["content"][0]["type"], "image");
    }

    #[test]
    #[ignore = "calls the real Claude API; needs ANTHROPIC_API_KEY"]
    fn live_api_draws() {
        let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY");
        let (tx, ui_h) = ui();
        let (etx, erx) = mpsc::channel();
        let c = AgentConfig { api_key: key, model: "claude-sonnet-5".into(), max_rounds: 6 };
        spawn(c, "Draw a small double-line box around the whole canvas in light cyan.".into(), String::new(), tx, etx)
            .join()
            .unwrap();
        let events: Vec<AgentEvent> = erx.iter().collect();
        let (_, doc) = ui_h.join().unwrap();
        eprintln!("{events:#?}");
        assert_eq!(events.last(), Some(&AgentEvent::Done));
        assert!(doc.canvas.used_height() > 0);
    }
}
