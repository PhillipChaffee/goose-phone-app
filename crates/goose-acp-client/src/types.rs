//! Wire types for the Agent Client Protocol (ACP) as served by `goose serve`.
//!
//! Field names on the wire are camelCase; enum discriminants are snake_case.
//! Discriminated unions are internally tagged: `ContentBlock` by `type`,
//! session updates by `sessionUpdate`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A block of prompt/message content (ACP `ContentBlock`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
            ContentBlock::Text { text, .. } => text.clone(),
            ContentBlock::Image { mime_type, .. } => format!("[image: {mime_type}]"),
            ContentBlock::Audio { mime_type, .. } => format!("[audio: {mime_type}]"),
            ContentBlock::ResourceLink { uri, name, .. } => {
                format!("[{}]({uri})", name.as_deref().unwrap_or(uri))
            }
            ContentBlock::Resource { resource, .. } => resource
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "[resource]".to_string()),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text {
            text: text.into(),
            annotations: None,
            meta: None,
        }
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
                    out.push_str(&format!("[diff: {path}]"));
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

impl SessionUpdate {
    pub fn from_value(raw: Value) -> Self {
        let tag = raw
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        fn parse<T: serde::de::DeserializeOwned>(raw: &Value) -> Option<T> {
            serde_json::from_value(raw.clone()).ok()
        }

        match tag.as_str() {
            "user_message_chunk" => match parse::<MessageChunk>(&raw) {
                Some(c) => SessionUpdate::UserMessageChunk(c),
                None => SessionUpdate::Unknown { tag, raw },
            },
            "agent_message_chunk" => match parse::<MessageChunk>(&raw) {
                Some(c) => SessionUpdate::AgentMessageChunk(c),
                None => SessionUpdate::Unknown { tag, raw },
            },
            "agent_thought_chunk" => match parse::<MessageChunk>(&raw) {
                Some(c) => SessionUpdate::AgentThoughtChunk(c),
                None => SessionUpdate::Unknown { tag, raw },
            },
            "tool_call" => match parse::<ToolCallUpdate>(&raw) {
                Some(c) => SessionUpdate::ToolCall(c),
                None => SessionUpdate::Unknown { tag, raw },
            },
            "tool_call_update" => match parse::<ToolCallUpdate>(&raw) {
                Some(c) => SessionUpdate::ToolCallUpdate(c),
                None => SessionUpdate::Unknown { tag, raw },
            },
            "session_info_update" => match parse::<SessionInfoUpdate>(&raw) {
                Some(c) => SessionUpdate::SessionInfoUpdate(c),
                None => SessionUpdate::Unknown { tag, raw },
            },
            "plan" => SessionUpdate::Plan(raw),
            "usage_update" => SessionUpdate::UsageUpdate(raw),
            "current_mode_update" => SessionUpdate::CurrentModeUpdate(raw),
            "config_option_update" => SessionUpdate::ConfigOptionUpdate(raw),
            "available_commands_update" => SessionUpdate::AvailableCommandsUpdate(raw),
            _ => SessionUpdate::Unknown { tag, raw },
        }
    }
}

/// One entry from `session/list`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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

    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.session_id.clone())
    }

    pub fn message_count(&self) -> Option<u64> {
        self.meta_field("messageCount")?.as_u64()
    }

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
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

/// One choice offered by a `session/request_permission` request.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_block_serializes_to_acp_shape() {
        let block = ContentBlock::text("Hello goose");
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v, json!({"type": "text", "text": "Hello goose"}));
    }

    #[test]
    fn parses_agent_message_chunk() {
        // Shape taken from goose 1.47 `session/update` notifications.
        let raw = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "Hi!"},
            "messageId": "msg_20260821_1_ab12",
            "_meta": {"goose": {"created": 1755763200u64, "messageId": "msg_20260821_1_ab12"}}
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
}
