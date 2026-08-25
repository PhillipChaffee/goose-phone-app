//! Wire types for the Agent Client Protocol (ACP) as served by `goose serve`.
//!
//! Field names on the wire are camelCase; enum discriminants are `snake_case`.
//! Discriminated unions are internally tagged: `ContentBlock` by `type`,
//! session updates by `sessionUpdate`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A block of prompt/message content (ACP `ContentBlock`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotations: Option<Value>,
        #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
    Audio {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
    Resource {
        resource: Value,
        #[serde(flatten)]
        extra: serde_json::Map<String, Value>,
    },
}

impl ContentBlock {
    /// Plain-text rendering of the block for a chat transcript.
    pub fn text_repr(&self) -> String {
        match self {
            Self::Text { text, .. } => text.clone(),
            Self::Image { mime_type, .. } => format!("[image: {mime_type}]"),
            Self::Audio { mime_type, .. } => format!("[audio: {mime_type}]"),
            Self::ResourceLink { uri, name, .. } => {
                format!("[{}]({uri})", name.as_deref().unwrap_or(uri))
            }
            Self::Resource { resource, .. } => resource
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    // A blob has no text to show, but it does have a name.
                    // "[resource]" for every attached PDF told the reader
                    // nothing about which one.
                    Some(format!(
                        "[{}]",
                        resource.get("uri").and_then(Value::as_str)?
                    ))
                })
                .unwrap_or_else(|| "[resource]".to_string()),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            annotations: None,
            meta: None,
        }
    }

    /// An attached image. `data` is base64 of the file's bytes.
    ///
    /// By value, not by reference: the agent runs on another machine and
    /// cannot open a path on this phone, so `uri` is left off entirely
    /// rather than carrying a name the agent would be unable to resolve.
    #[must_use]
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::Image {
            data: data.into(),
            mime_type: mime_type.into(),
            uri: None,
            meta: None,
        }
    }

    /// An attached text file, as an embedded resource — MCP's
    /// `TextResourceContents`, which ACP embeds verbatim. `uri` is what the
    /// agent will call the file, so it carries the name the picker gave it.
    #[must_use]
    pub fn resource_text(uri: &str, mime_type: &str, text: &str) -> Self {
        Self::Resource {
            resource: serde_json::json!({
                "uri": uri,
                "mimeType": mime_type,
                "text": text,
            }),
            extra: serde_json::Map::new(),
        }
    }

    /// An attached binary file — MCP's `BlobResourceContents`, whose `blob`
    /// is base64. Used for anything that is neither an image nor text (a PDF
    /// today), so the agent at least receives the bytes and the name.
    #[must_use]
    pub fn resource_blob(uri: &str, mime_type: &str, blob: &str) -> Self {
        Self::Resource {
            resource: serde_json::json!({
                "uri": uri,
                "mimeType": mime_type,
                "blob": blob,
            }),
            extra: serde_json::Map::new(),
        }
    }

    /// Whether this block is an attachment rather than prose — an image, an
    /// audio clip, a linked resource or an embedded one.
    ///
    /// The transcript renders those as attachments beside the message rather
    /// than as the `text_repr` placeholder that used to stand in for them.
    #[must_use]
    pub const fn is_attachment(&self) -> bool {
        !matches!(self, Self::Text { .. })
    }
}

/// A streamed message chunk (`agent_message_chunk` / `agent_thought_chunk` /
/// `user_message_chunk`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageChunk {
    pub content: ContentBlock,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

/// A `tool_call` or `tool_call_update` payload. In updates every field except
/// `tool_call_id` is optional, so everything is lenient here.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub content: Option<Vec<Value>>,
    pub locations: Option<Vec<Value>>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
    #[serde(rename = "_meta")]
    pub meta: Option<Value>,
}

impl ToolCallUpdate {
    /// Name of the underlying goose tool, e.g. `developer__shell`, when present.
    #[must_use]
    pub fn tool_name(&self) -> Option<&str> {
        self.meta
            .as_ref()?
            .get("goose")?
            .get("toolCall")?
            .get("toolName")?
            .as_str()
    }

    /// Concatenated human-readable text from the `content` entries
    /// (`ToolCallContent` variants `content` / `diff` / `terminal`).
    pub fn content_text(&self) -> String {
        let mut out = String::new();
        for item in self.content.iter().flatten() {
            match item.get("type").and_then(Value::as_str) {
                Some("content") => {
                    if let Some(block) = item.get("content") {
                        if let Ok(block) = serde_json::from_value::<ContentBlock>(block.clone()) {
                            if !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str(&block.text_repr());
                        }
                    }
                }
                Some("diff") => {
                    let path = item.get("path").and_then(Value::as_str).unwrap_or("file");
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("[diff: ");
                    out.push_str(path);
                    out.push(']');
                    if let Some(new_text) = item.get("newText").and_then(Value::as_str) {
                        out.push('\n');
                        out.push_str(new_text);
                    }
                }
                Some("terminal") => {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("[terminal output]");
                }
                _ => {}
            }
        }
        out
    }
}

/// Update to session metadata (`session_info_update`), e.g. auto-generated titles.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionInfoUpdate {
    pub title: Option<String>,
    pub updated_at: Option<String>,
    #[serde(rename = "_meta")]
    pub meta: Option<Value>,
}

/// One `session/update` notification payload, dispatched on its
/// `sessionUpdate` tag. Unknown variants are preserved rather than dropped.
#[derive(Debug, Clone)]
pub enum SessionUpdate {
    UserMessageChunk(MessageChunk),
    AgentMessageChunk(MessageChunk),
    AgentThoughtChunk(MessageChunk),
    ToolCall(ToolCallUpdate),
    ToolCallUpdate(ToolCallUpdate),
    Plan(Value),
    SessionInfoUpdate(SessionInfoUpdate),
    UsageUpdate(Value),
    CurrentModeUpdate(Value),
    ConfigOptionUpdate(Value),
    AvailableCommandsUpdate(Value),
    Unknown { tag: String, raw: Value },
}

/// Deserialize a whole `session/update` payload into one of the typed
/// variants, or `None` if it does not fit the shape that tag implies.
fn parse<T: serde::de::DeserializeOwned>(raw: &Value) -> Option<T> {
    serde_json::from_value(raw.clone()).ok()
}

impl SessionUpdate {
    pub fn from_value(raw: Value) -> Self {
        let tag = raw
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        match tag.as_str() {
            "user_message_chunk" => match parse::<MessageChunk>(&raw) {
                Some(c) => Self::UserMessageChunk(c),
                None => Self::Unknown { tag, raw },
            },
            "agent_message_chunk" => match parse::<MessageChunk>(&raw) {
                Some(c) => Self::AgentMessageChunk(c),
                None => Self::Unknown { tag, raw },
            },
            "agent_thought_chunk" => match parse::<MessageChunk>(&raw) {
                Some(c) => Self::AgentThoughtChunk(c),
                None => Self::Unknown { tag, raw },
            },
            "tool_call" => match parse::<ToolCallUpdate>(&raw) {
                Some(c) => Self::ToolCall(c),
                None => Self::Unknown { tag, raw },
            },
            "tool_call_update" => match parse::<ToolCallUpdate>(&raw) {
                Some(c) => Self::ToolCallUpdate(c),
                None => Self::Unknown { tag, raw },
            },
            "session_info_update" => match parse::<SessionInfoUpdate>(&raw) {
                Some(c) => Self::SessionInfoUpdate(c),
                None => Self::Unknown { tag, raw },
            },
            "plan" => Self::Plan(raw),
            "usage_update" => Self::UsageUpdate(raw),
            "current_mode_update" => Self::CurrentModeUpdate(raw),
            "config_option_update" => Self::ConfigOptionUpdate(raw),
            "available_commands_update" => Self::AvailableCommandsUpdate(raw),
            _ => Self::Unknown { tag, raw },
        }
    }
}

/// One entry from `session/list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

impl SessionInfo {
    fn meta_field(&self, key: &str) -> Option<&Value> {
        self.meta.as_ref()?.get(key)
    }

    #[must_use]
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.session_id.clone())
    }

    #[must_use]
    pub fn message_count(&self) -> Option<u64> {
        self.meta_field("messageCount")?.as_u64()
    }

    #[must_use]
    pub fn last_message_snippet(&self) -> Option<String> {
        Some(self.meta_field("lastMessageSnippet")?.as_str()?.to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResponse {
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    pub session_id: String,
    /// Session configuration the agent offers — provider, model, mode and
    /// thinking effort. This is where the list of available models arrives:
    /// no separate call is needed, and it was previously parsed away.
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

/// One configurable knob on a session.
///
/// Wire shape per ACP schema 1.5 (`SessionConfigOption`): `configId`, `name`,
/// an optional `description`, and a flattened kind payload tagged by `type`.
/// For `type: "select"` the payload is `currentValue` plus `options`, each of
/// which keys on `value` (not `id`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOption {
    pub config_id: String,
    #[serde(default)]
    pub name: String,
    /// The agent's own words about what this option does. goose sends one
    /// for `thinking_effort`, which is exactly the option a user is most
    /// likely to find stuck on a single value.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub current_value: Option<String>,
    #[serde(default)]
    pub options: Vec<ConfigChoice>,
}

impl ConfigOption {
    /// The label for the current value, falling back to the raw id.
    #[must_use]
    pub fn current_label(&self) -> Option<&str> {
        let current = self.current_value.as_deref()?;
        Some(
            self.options
                .iter()
                .find(|o| o.value == current)
                .map_or(current, |o| o.name.as_str()),
        )
    }

    /// Whether choosing between the values would change anything.
    ///
    /// An option with one value is a fact, not a control: goose ships
    /// `thinking_effort` as a select whose only value is `off` whenever the
    /// session's model is not a reasoning model. Offering that as a menu
    /// would be a control that does nothing (design rule 11); reporting it
    /// tells the user *why* effort is not adjustable here.
    #[must_use]
    pub const fn is_adjustable(&self) -> bool {
        self.options.len() > 1
    }
}

/// One selectable value of a `select` config option.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChoice {
    pub value: String,
    #[serde(default)]
    pub name: String,
    /// What choosing this one does, per `SessionConfigSelectOption`. goose
    /// sends one for each of its modes, which is the option whose values a
    /// reader is least able to guess at from the name alone.
    #[serde(default)]
    pub description: Option<String>,
}

/// One choice offered by a `session/request_permission` request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

/// A tool-permission request from the agent that the client must answer.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    /// JSON-RPC id of the incoming request; pass back to
    /// [`crate::AcpClient::respond_permission`].
    pub request_id: Value,
    pub session_id: String,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
}

/// Result of `initialize`.
#[derive(Debug, Clone)]
pub struct InitializeInfo {
    pub agent_name: String,
    pub agent_version: String,
    pub raw: Value,
}

/// Events surfaced to the application from the connection.
#[derive(Debug)]
pub enum AcpEvent {
    /// `session/update` notification.
    Update {
        session_id: String,
        update: SessionUpdate,
    },
    /// `_goose/unstable/session/update` notification (token usage, status
    /// messages). Payload is the raw `update` object tagged by `sessionUpdate`.
    GooseUpdate { session_id: String, update: Value },
    /// The agent asks permission to run a tool; answer with
    /// [`crate::AcpClient::respond_permission`].
    Permission(PermissionRequest),
    /// The agent cancelled one of its own outstanding requests
    /// (`$/cancel_request`), e.g. a permission prompt that timed out.
    RequestCancelled { request_id: Value },
    /// The connection is gone. No further events will arrive.
    Disconnected { reason: String },
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the failing check"
)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_block_serializes_to_acp_shape() {
        let block = ContentBlock::text("Hello goose");
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v, json!({"type": "text", "text": "Hello goose"}));
    }

    /// The three shapes an attachment can take on the wire. `data` and `blob`
    /// are base64; ACP takes MCP's resource contents verbatim, so an embedded
    /// resource is `{uri, mimeType, text|blob}` and nothing else.
    #[test]
    fn attachment_blocks_serialize_to_acp_shapes() {
        assert_eq!(
            serde_json::to_value(ContentBlock::image("QUJD", "image/png")).unwrap(),
            json!({"type": "image", "data": "QUJD", "mimeType": "image/png"})
        );
        assert_eq!(
            serde_json::to_value(ContentBlock::resource_text(
                "file:///notes.md",
                "text/markdown",
                "# hi"
            ))
            .unwrap(),
            json!({"type": "resource", "resource": {
                "uri": "file:///notes.md", "mimeType": "text/markdown", "text": "# hi"}})
        );
        assert_eq!(
            serde_json::to_value(ContentBlock::resource_blob(
                "file:///spec.pdf",
                "application/pdf",
                "QUJD"
            ))
            .unwrap(),
            json!({"type": "resource", "resource": {
                "uri": "file:///spec.pdf", "mimeType": "application/pdf", "blob": "QUJD"}})
        );
    }

    /// A blob has no text to render, so the transcript falls back to naming
    /// the file — "[resource]" for every attached PDF said nothing about
    /// which one.
    #[test]
    fn a_blob_resource_renders_as_its_name() {
        let block = ContentBlock::resource_blob("file:///spec.pdf", "application/pdf", "QUJD");
        assert_eq!(block.text_repr(), "[file:///spec.pdf]");
        assert!(block.is_attachment());
        assert!(!ContentBlock::text("hi").is_attachment());
    }

    #[test]
    fn parses_agent_message_chunk() {
        // Shape taken from goose 1.47 `session/update` notifications.
        let raw = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "Hi!"},
            "messageId": "msg_20260821_1_ab12",
            "_meta": {"goose": {"created": 1_755_763_200u64, "messageId": "msg_20260821_1_ab12"}}
        });
        match SessionUpdate::from_value(raw) {
            SessionUpdate::AgentMessageChunk(c) => {
                assert_eq!(c.content.text_repr(), "Hi!");
                assert_eq!(c.message_id.as_deref(), Some("msg_20260821_1_ab12"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_tool_call_and_update() {
        let call = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_1",
            "title": "shell: ls",
            "kind": "execute",
            "status": "pending",
            "content": [],
            "locations": [],
            "rawInput": {"command": "ls"},
            "_meta": {"goose": {"toolCall": {"toolName": "developer__shell", "extensionName": "developer"}}}
        });
        match SessionUpdate::from_value(call) {
            SessionUpdate::ToolCall(c) => {
                assert_eq!(c.tool_call_id, "call_1");
                assert_eq!(c.kind.as_deref(), Some("execute"));
                assert_eq!(c.tool_name(), Some("developer__shell"));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let update = json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "completed",
            "content": [
                {"type": "content", "content": {"type": "text", "text": "file_a\nfile_b"}}
            ],
            "rawOutput": {"stdout": "file_a\nfile_b"}
        });
        match SessionUpdate::from_value(update) {
            SessionUpdate::ToolCallUpdate(c) => {
                assert_eq!(c.status.as_deref(), Some("completed"));
                assert_eq!(c.content_text(), "file_a\nfile_b");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn unknown_update_is_preserved() {
        let raw = json!({"sessionUpdate": "brand_new_thing", "x": 1});
        match SessionUpdate::from_value(raw.clone()) {
            SessionUpdate::Unknown { tag, raw: r } => {
                assert_eq!(tag, "brand_new_thing");
                assert_eq!(r, raw);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn parses_session_info() {
        let raw = json!({
            "sessionId": "20260821_1",
            "cwd": "/home/me/project",
            "title": "Fix the build",
            "updatedAt": "2026-08-21T09:00:00Z",
            "additionalDirectories": [],
            "_meta": {
                "messageCount": 12,
                "createdAt": "2026-08-20T18:00:00Z",
                "userSetName": false,
                "sessionType": "user",
                "hasRecipe": false,
                "lastMessageSnippet": "Done — the build is green."
            }
        });
        let info: SessionInfo = serde_json::from_value(raw).unwrap();
        assert_eq!(info.session_id, "20260821_1");
        assert_eq!(info.display_title(), "Fix the build");
        assert_eq!(info.message_count(), Some(12));
        assert_eq!(
            info.last_message_snippet().as_deref(),
            Some("Done — the build is green.")
        );
    }

    /// The four options goose builds in `acp::response_builder`, verbatim.
    #[test]
    fn parses_every_config_option_goose_sends() {
        let raw = json!([
            {"configId": "provider", "name": "Provider", "type": "select",
             "currentValue": "anthropic",
             "options": [{"value": "anthropic", "name": "Anthropic"},
                         {"value": "openai", "name": "OpenAI"}]},
            {"configId": "mode", "name": "Mode", "category": "mode", "type": "select",
             "currentValue": "auto",
             "options": [{"value": "auto", "name": "Auto"},
                         {"value": "approve", "name": "Manual approval"}]},
            {"configId": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": "claude-opus-5",
             "options": [{"value": "claude-opus-5", "name": "Claude Opus 5"}]},
            {"configId": "thinking_effort", "name": "Thinking effort",
             "category": "thought_level", "type": "select",
             "description": "Controls reasoning effort for models that support extended thinking.",
             "currentValue": "off",
             "options": [{"value": "off", "name": "off"}]}
        ]);
        let opts: Vec<ConfigOption> = serde_json::from_value(raw).unwrap();
        let ids: Vec<&str> = opts.iter().map(|o| o.config_id.as_str()).collect();
        assert_eq!(ids, ["provider", "mode", "model", "thinking_effort"]);

        assert!(opts[0].is_adjustable());
        assert!(opts[1].is_adjustable());
        // One value is a fact, not a control — see `is_adjustable`.
        assert!(!opts[2].is_adjustable());
        assert!(!opts[3].is_adjustable());
        assert_eq!(opts[2].current_label(), Some("Claude Opus 5"));
        assert!(opts[3]
            .description
            .as_deref()
            .is_some_and(|d| d.starts_with("Controls reasoning effort")));
    }

    /// `configOptions` is absent whenever the session has no provider/model
    /// yet, so the sheet has to survive an empty set rather than assume one.
    #[test]
    fn missing_config_options_is_an_empty_set() {
        assert!(crate::config_options_from(&json!({"sessionId": "x"})).is_empty());
    }
}
