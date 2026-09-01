//! Wire types for the base Agent Client Protocol (ACP) as served by
//! `goose serve`.
//!
//! Field names on the wire are camelCase; enum discriminants are `snake_case`.
//! Discriminated unions are internally tagged: `ContentBlock` by `type`,
//! session updates by `sessionUpdate`. Base ACP is uniformly camelCase by
//! specification, so the blanket `#[serde(rename_all = "camelCase")]` on the
//! types here is a statement of that spec — unlike goose's own namespace,
//! where casing varies per type and the rule is the opposite (see
//! [`crate::goose`]).

mod config;
mod session;

pub use config::*;
pub use session::*;

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

/// Who ended the connection.
///
/// The distinction is not cosmetic and it is not derivable upstack. An app
/// that reports a lost turn has to be certain it is not reporting one the
/// user threw away, and the app's own `want_connected` flag cannot tell it:
/// reconnecting from Settings closes the live client while that flag is still
/// true, so "we wanted to be connected and are not" covers both a dropped
/// tailnet and a deliberate press of Connect. The transport is the only layer
/// that knows, so it is the layer that says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectCause {
    /// This side asked: [`crate::AcpClient::close`], or the last handle being
    /// dropped.
    Local,
    /// The socket ended under us — EOF, a stream error, a Close frame from
    /// the server, a failed send, or the keepalive giving up.
    Transport,
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
    Disconnected {
        reason: String,
        cause: DisconnectCause,
    },
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

    /// Every block a transcript can be handed has to render as *something* a
    /// reader recognises. These strings are what stands in the message body
    /// beside the attachment itself, so an empty or misleading one is a hole
    /// in the conversation.
    #[test]
    fn every_block_kind_renders_something_a_reader_can_place() {
        fn block(v: Value) -> ContentBlock {
            serde_json::from_value(v).unwrap()
        }

        assert_eq!(
            ContentBlock::image("QUJD", "image/png").text_repr(),
            "[image: image/png]"
        );
        assert_eq!(
            block(json!({"type": "audio", "data": "QUJD", "mimeType": "audio/wav"})).text_repr(),
            "[audio: audio/wav]"
        );
        // A link renders as the name it was given...
        assert_eq!(
            block(json!({"type": "resource_link", "uri": "file:///src/a.rs", "name": "a.rs"}))
                .text_repr(),
            "[a.rs](file:///src/a.rs)"
        );
        // ...and falls back to the URI when the server sent no name, rather
        // than an empty pair of brackets.
        assert_eq!(
            block(json!({"type": "resource_link", "uri": "file:///src/a.rs"})).text_repr(),
            "[file:///src/a.rs](file:///src/a.rs)"
        );
        // An embedded resource with text shows the text.
        assert_eq!(
            ContentBlock::resource_text("file:///n.md", "text/markdown", "# hi").text_repr(),
            "# hi"
        );
        // Neither text nor a uri leaves nothing to name it by.
        assert_eq!(
            block(json!({"type": "resource", "resource": {}})).text_repr(),
            "[resource]"
        );
    }

    /// What a tool call shows once it has run. Every entry `ToolCallContent`
    /// can hold has to contribute, separated so two results never run
    /// together into one line — and an entry this crate does not understand
    /// must be skipped silently rather than emptying the whole block.
    #[test]
    fn tool_call_content_joins_every_kind_it_understands() {
        let update: ToolCallUpdate = serde_json::from_value(json!({
            "toolCallId": "call_9",
            "content": [
                {"type": "content", "content": {"type": "text", "text": "first"}},
                {"type": "content", "content": {"type": "text", "text": "second"}},
                {"type": "content", "content": {"type": "not_a_block"}},
                {"type": "content"},
                {"type": "diff", "path": "src/lib.rs", "newText": "fn main() {}"},
                {"type": "diff"},
                {"type": "terminal", "terminalId": "term_1"},
                {"type": "something_new"},
                {"notATypeAtAll": true},
            ]
        }))
        .unwrap();

        assert_eq!(
            update.content_text(),
            "first\nsecond\n[diff: src/lib.rs]\nfn main() {}\n[diff: file]\n[terminal output]",
            "each understood entry gets its own line and an unreadable one is skipped"
        );
        assert_eq!(
            ToolCallUpdate::default().content_text(),
            "",
            "a tool call with no content yet must render as nothing, not as a placeholder"
        );
        assert_eq!(
            update.tool_name(),
            None,
            "no goose _meta means no tool name to show"
        );
    }

    /// The three chunk tags are three different things on screen — what the
    /// user said, what the agent said, and what it was thinking — so mixing
    /// them up puts the agent's reasoning in the user's own bubble.
    #[test]
    fn each_chunk_tag_lands_in_its_own_variant() {
        let chunk = |tag: &str| {
            SessionUpdate::from_value(json!({
                "sessionUpdate": tag,
                "content": {"type": "text", "text": tag},
            }))
        };
        match chunk("user_message_chunk") {
            SessionUpdate::UserMessageChunk(c) => {
                assert_eq!(c.content.text_repr(), "user_message_chunk");
            }
            other => panic!("wrong variant: {other:?}"),
        }
        match chunk("agent_thought_chunk") {
            SessionUpdate::AgentThoughtChunk(c) => {
                assert_eq!(c.content.text_repr(), "agent_thought_chunk");
                assert!(c.message_id.is_none() && c.meta.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let renamed = SessionUpdate::from_value(json!({
            "sessionUpdate": "session_info_update",
            "title": "Refactoring the parser",
            "updatedAt": "2026-08-25T09:34:12Z",
        }));
        match renamed {
            SessionUpdate::SessionInfoUpdate(info) => {
                assert_eq!(info.title.as_deref(), Some("Refactoring the parser"));
                assert_eq!(info.updated_at.as_deref(), Some("2026-08-25T09:34:12Z"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// A payload whose tag this crate knows but whose body it cannot read is
    /// still kept whole. Dropping it would lose a message with no trace; the
    /// app can log or display the raw JSON instead, and the tag says which
    /// shape changed under us.
    #[test]
    fn a_typed_tag_with_an_unreadable_body_survives_as_unknown() {
        for tag in [
            "user_message_chunk",
            "agent_message_chunk",
            "agent_thought_chunk",
            "tool_call",
            "tool_call_update",
            "session_info_update",
        ] {
            // Wrong types for the one field each of those three shapes needs.
            let raw = json!({
                "sessionUpdate": tag,
                "content": 42,
                "toolCallId": 7,
                "title": 9,
            });
            match SessionUpdate::from_value(raw.clone()) {
                SessionUpdate::Unknown {
                    tag: seen,
                    raw: kept,
                } => {
                    assert_eq!(seen, tag, "the tag must survive so the loss is traceable");
                    assert_eq!(kept, raw, "the payload must be kept byte for byte");
                }
                other => panic!("a malformed {tag} must not parse: {other:?}"),
            }
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
}
