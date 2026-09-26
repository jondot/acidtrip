//! acidtrip AI: one tool schema shared by the live MCP server and the in-app
//! prompt bar, an executor that turns tool calls into undoable transactions,
//! a Claude Messages API agent loop, and the font/stencil harvester.
//!
//! Threading model: anything that wants to edit the open document (the live
//! socket server, the in-app agent) sends a [`ToolRequest`] over a channel to
//! the UI thread, which executes it with [`exec::execute`] and replies.

use serde::{Deserialize, Serialize};

pub mod agent;
pub mod exec;
pub mod gallery;
pub mod harvest;
pub mod live;
pub mod mcp;
pub mod schema;

/// A tool invocation: `name` is one of [`schema::tool_definitions`], `args`
/// matches its JSON schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolResult {
    pub text: String,
    /// PNG bytes (render_png and friends) so models can see the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "b64opt")]
    pub image_png: Option<Vec<u8>>,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(text: impl Into<String>) -> Self {
        ToolResult { text: text.into(), image_png: None, is_error: false }
    }

    pub fn err(text: impl Into<String>) -> Self {
        ToolResult { text: text.into(), image_png: None, is_error: true }
    }

    /// The editor's answer to every AI tool call once the user stopped the
    /// run; the agent ends the run when it sees it.
    pub fn cancelled() -> Self {
        ToolResult::err(CANCELLED)
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_error && self.text == CANCELLED
    }
}

pub const CANCELLED: &str = "cancelled by the user";

/// A request to run a tool against the UI's open document.
pub struct ToolRequest {
    pub call: ToolCall,
    /// Where it came from, shown in the status bar ("mcp", "ai").
    pub origin: String,
    pub reply: std::sync::mpsc::Sender<ToolResult>,
}

mod b64opt {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_some(&base64::engine::general_purpose::STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let o: Option<String> = Option::deserialize(d)?;
        o.map(|s| base64::engine::general_purpose::STANDARD.decode(s).map_err(serde::de::Error::custom)).transpose()
    }
}
