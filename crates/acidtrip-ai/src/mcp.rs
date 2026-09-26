//! `acidtrip mcp`: stdio MCP server. Attaches to a live editor if one is
//! running (or `session` pid given), else runs headless on its own doc.
//!
//! Hand-rolled JSON-RPC 2.0 over newline-delimited stdio (the MCP stdio
//! transport): initialize, notifications/*, ping, tools/list, tools/call.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use acidtrip_core::{DocKind, Document, History};
use acidtrip_io::fonts::FontLibrary;
use acidtrip_io::library::Paths;
use acidtrip_io::stencils::StencilLibrary;
use anyhow::{Result, bail};
use base64::Engine;
use serde_json::{Value, json};

use crate::exec::{self, ExecState};
use crate::harvest;
use crate::live::{LiveClient, find_live};
use crate::{ToolCall, ToolResult, schema};

pub struct McpOptions {
    pub session: Option<u32>,
    /// Force headless even if an editor is running.
    pub headless: bool,
}

/// Where tool calls run.
pub trait Backend {
    fn call(&mut self, call: &ToolCall) -> ToolResult;
    /// True when attached to a running editor.
    fn is_live(&self) -> bool;
    /// Library paths + stencils for the harvest tools (None = unavailable).
    fn library(&mut self) -> Option<(&Paths, &mut StencilLibrary)>;
}

/// Own document, executed in-process.
pub struct Headless {
    pub doc: Document,
    pub history: History,
    pub layer: usize,
    pub fonts: FontLibrary,
    pub stencils: StencilLibrary,
    pub paths: Paths,
    pub file: Option<PathBuf>,
}

impl Headless {
    /// 80x25 Classic doc; fonts and stencils from the user library.
    pub fn new(paths: Paths) -> Headless {
        let fonts = FontLibrary::load(Some(&paths.fonts_dir()));
        let stencils = StencilLibrary::load(&paths.stencils_dir());
        Headless {
            doc: Document::new(DocKind::Classic, 80, 25),
            history: History::new(),
            layer: 0,
            fonts,
            stencils,
            paths,
            file: None,
        }
    }
}

impl Backend for Headless {
    fn call(&mut self, call: &ToolCall) -> ToolResult {
        let mut st = ExecState {
            doc: &mut self.doc,
            history: &mut self.history,
            layer: self.layer,
            fonts: &self.fonts,
            stencils: &mut self.stencils,
            paths: &self.paths,
            file: &mut self.file,
        };
        let r = exec::execute(&mut st, call);
        self.layer = st.layer;
        r
    }

    fn is_live(&self) -> bool {
        false
    }

    fn library(&mut self) -> Option<(&Paths, &mut StencilLibrary)> {
        Some((&self.paths, &mut self.stencils))
    }
}

/// Forwards every call to a running editor over its socket.
pub struct Live {
    pub client: LiveClient,
    pub paths: Option<Paths>,
    stencils: Option<StencilLibrary>,
}

impl Live {
    pub fn new(client: LiveClient, paths: Option<Paths>) -> Live {
        Live { client, paths, stencils: None }
    }
}

impl Backend for Live {
    fn call(&mut self, call: &ToolCall) -> ToolResult {
        self.client.call(call).unwrap_or_else(|e| {
            ToolResult::err(format!("lost the connection to the editor ({e:#}); restart the MCP server"))
        })
    }

    fn is_live(&self) -> bool {
        true
    }

    fn library(&mut self) -> Option<(&Paths, &mut StencilLibrary)> {
        let paths = self.paths.as_ref()?;
        let st = self.stencils.get_or_insert_with(|| StencilLibrary::load(&paths.stencils_dir()));
        Some((paths, st))
    }
}

pub const INSTRUCTIONS_LIVE: &str = "acidtrip is a terminal ANSI art editor and you are attached to the user's open editor: \
your edits appear live on their screen. Each tool call is one undo step (the user can Ctrl-Z it). AI edits land on the \
'AI' layer by default so the user can hide, merge or drop them; use the layer tool to target another layer. Start with \
get_info and render_png to see the canvas, draw with few large calls, and render_png again to check your work. Colors \
are VGA palette indices 0-15 (0 black, 1 blue, 2 green, 3 cyan, 4 red, 5 magenta, 6 brown, 7 light gray, 8-15 bright).";

pub const INSTRUCTIONS_HEADLESS: &str = "acidtrip is an ANSI art editor running headless: you draw on a private 80x25 CP437 \
canvas and save it with save (.ans, .xb, .png, .html, …). Each tool call is one undo step. Start with render_png, draw \
with few large calls, and render_png again to check your work. Colors are VGA palette indices 0-15 (0 black, 1 blue, \
2 green, 3 cyan, 4 red, 5 magenta, 6 brown, 7 light gray, 8-15 bright). harvest_candidates / harvest_commit turn scene \
logos into fonts and stencils: you read the letters from the images.";

fn content(r: &ToolResult) -> Value {
    let mut c = vec![json!({ "type": "text", "text": r.text })];
    if let Some(png) = &r.image_png {
        c.push(json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png" }));
    }
    json!({ "content": c, "isError": r.is_error })
}

fn mcp_tools() -> Vec<Value> {
    schema::tool_definitions()
        .into_iter()
        .chain(schema::mcp_tool_definitions())
        .map(|t| json!({ "name": t["name"], "description": t["description"], "inputSchema": t["input_schema"] }))
        .collect()
}

fn harvest_call(session: &mut harvest::Session, backend: &mut dyn Backend, name: &str, args: &Value) -> Value {
    let Some((paths, stencils)) = backend.library() else {
        return content(&ToolResult::err("the harvester needs the acidtrip library directories"));
    };
    let r = if name == "harvest_candidates" {
        session
            .candidates_tool(args, &paths.state_dir.join("harvest-cache"))
            .map(|c| json!({ "content": c, "isError": false }))
    } else {
        session.commit_tool(args, paths, stencils).map(|t| content(&ToolResult::ok(t)))
    };
    r.unwrap_or_else(|e| content(&ToolResult::err(format!("{name}: {e:#}"))))
}

/// Serve MCP over any line reader/writer until EOF.
pub fn serve_io(reader: impl BufRead, mut writer: impl Write, backend: &mut dyn Backend) -> Result<()> {
    let mut session = harvest::Session::default();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } });
                writeln!(writer, "{err}")?;
                writer.flush()?;
                continue;
            }
        };
        let id = msg.get("id").cloned().filter(|v| !v.is_null());
        let method = msg["method"].as_str().unwrap_or("");
        let params = &msg["params"];
        let result: Option<std::result::Result<Value, (i64, String)>> = match method {
            "initialize" => Some(Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "acidtrip", "version": env!("CARGO_PKG_VERSION") },
                "instructions": if backend.is_live() { INSTRUCTIONS_LIVE } else { INSTRUCTIONS_HEADLESS },
            }))),
            "ping" => Some(Ok(json!({}))),
            "tools/list" => Some(Ok(json!({ "tools": mcp_tools() }))),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("").to_string();
                let args = params.get("arguments").cloned().filter(|v| !v.is_null()).unwrap_or_else(|| json!({}));
                Some(Ok(if name.starts_with("harvest_") {
                    harvest_call(&mut session, backend, &name, &args)
                } else {
                    content(&backend.call(&ToolCall { name, args }))
                }))
            }
            "resources/list" => Some(Ok(json!({ "resources": [] }))),
            "prompts/list" => Some(Ok(json!({ "prompts": [] }))),
            m if m.starts_with("notifications/") => None,
            _ => id.as_ref().map(|_| Err((-32601, format!("method not found: {method}")))),
        };
        if let (Some(res), Some(id)) = (result, id) {
            let resp = match res {
                Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
                Err((code, message)) => {
                    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
                }
            };
            writeln!(writer, "{resp}")?;
            writer.flush()?;
        }
    }
    Ok(())
}

pub fn run_stdio(opts: McpOptions) -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    let live = if opts.headless { None } else { find_live(&paths.sockets_dir(), opts.session) };
    let mut backend: Box<dyn Backend> = match live {
        Some(sock) => {
            eprintln!("acidtrip mcp: attached to the editor at {}", sock.display());
            Box::new(Live::new(LiveClient::connect(&sock)?, Some(paths)))
        }
        None if opts.session.is_some() && !opts.headless => {
            bail!("no running acidtrip editor with pid {}", opts.session.unwrap_or_default())
        }
        None => {
            eprintln!("acidtrip mcp: no editor running, working headless");
            Box::new(Headless::new(paths))
        }
    };
    let stdin = std::io::stdin();
    serve_io(stdin.lock(), std::io::stdout().lock(), backend.as_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn drive(backend: &mut dyn Backend, msgs: &[Value]) -> Vec<Value> {
        let input: String = msgs.iter().map(|m| format!("{m}\n")).collect();
        let mut out = vec![];
        serve_io(Cursor::new(input), &mut out, backend).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    struct Fake(Vec<String>);

    impl Backend for Fake {
        fn call(&mut self, call: &ToolCall) -> ToolResult {
            self.0.push(call.name.clone());
            ToolResult { text: format!("ran {}", call.name), image_png: Some(b"\x89PNG".to_vec()), is_error: false }
        }
        fn is_live(&self) -> bool {
            true
        }
        fn library(&mut self) -> Option<(&Paths, &mut StencilLibrary)> {
            None
        }
    }

    #[test]
    fn protocol_roundtrip() {
        let mut fake = Fake(vec![]);
        let out = drive(
            &mut fake,
            &[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }),
                json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
                json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "render_png", "arguments": {} } }),
                json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }),
                json!({ "jsonrpc": "2.0", "id": 5, "method": "bogus" }),
                json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "harvest_commit", "arguments": {} } }),
            ],
        );
        assert_eq!(out.len(), 6, "notification gets no reply");
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
        assert!(out[0]["result"]["instructions"].as_str().unwrap().contains("undo step"));
        let tools = out[1]["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        assert!(tools.iter().any(|t| t["name"] == "harvest_candidates"));
        let c = &out[2]["result"]["content"];
        assert_eq!(c[0]["text"], "ran render_png");
        assert_eq!(c[1]["type"], "image");
        assert_eq!(c[1]["mimeType"], "image/png");
        assert_eq!(c[1]["data"], "iVBORw==");
        assert_eq!(out[3]["result"], json!({}));
        assert_eq!(out[4]["error"]["code"], -32601);
        assert_eq!(out[5]["result"]["isError"], true);
        assert_eq!(fake.0, vec!["render_png"]);
    }

    #[test]
    fn headless_draw_and_render() {
        let dir = tempfile::tempdir().unwrap();
        let paths =
            Paths { config_dir: dir.path().join("c"), data_dir: dir.path().join("d"), state_dir: dir.path().join("s") };
        std::fs::create_dir_all(paths.stencils_dir()).unwrap();
        let mut h = Headless::new(paths);
        let out = drive(
            &mut h,
            &[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "draw_box", "arguments": { "x": 0, "y": 0, "w": 80, "h": 25, "style": "double", "fg": 11 } } }),
                json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "render_png", "arguments": {} } }),
                json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "put_text", "arguments": { "x": 0 } } }),
            ],
        );
        assert!(out[0]["result"]["instructions"].as_str().unwrap().contains("headless"));
        assert_eq!(out[1]["result"]["isError"], false, "{}", out[1]);
        let img = &out[2]["result"]["content"][1];
        assert_eq!(img["type"], "image");
        let png = base64::engine::general_purpose::STANDARD.decode(img["data"].as_str().unwrap()).unwrap();
        let im = image::load_from_memory(&png).unwrap();
        assert_eq!((im.width(), im.height()), (640, 400));
        assert_eq!(out[3]["result"]["isError"], true);
        assert_eq!(h.doc.canvas.composite(0, 0).ch, '╔');
        assert_eq!(h.history.len(), 1);
    }
}
