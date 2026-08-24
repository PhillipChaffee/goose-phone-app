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
                .map_or_else(|| "[resource]".to_string(), str::to_string),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
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

// ---------------------------------------------------------------------------
// Extensions — the wire types behind the Connect screen.
//
// One spelling in here is load bearing, and getting it wrong is a security
// bug rather than a style slip.
//
// goose's own `GooseExtension` carries `#[serde(tag = "type", rename_all =
// "snake_case")]`. On an enum, serde's `rename_all` renames VARIANTS; the
// fields inside struct variants need `rename_all_fields`, which goose does
// not use. So every field keeps its Rust spelling unless it is renamed one at
// a time — and exactly one of them is, `env_keys` -> `envKeys`.
//
// The result is a wire format that mixes cases: `available_tools` and
// `display_name` are snake_case while `envKeys` (and, from 1.47, `clientId` /
// `clientSecretKey`) are camelCase. goose sets no `deny_unknown_fields`, so a
// camelCase `availableTools` is accepted, ignored and silently dropped — and
// a dropped allowlist deserializes as `None`, which goose turns into `vec![]`,
// which means *every tool is allowed*.
//
// Verified against goose 1.46.0 by adding two extensions over ACP, one with
// each spelling, and reading `config.yaml` back: the camelCase one had no
// `available_tools` key at all; the snake_case one persisted. A read-only mail
// connector written with the natural camelCase spelling would ship with its
// send tool live. `serializes_the_exact_wire_spellings` below asserts the
// literal JSON keys, and `AcpClient::add_extension_verified` refuses to
// believe an add worked until the server hands the allowlist back.

/// An HTTP header sent to a remote MCP server (ACP `HttpHeader`).
///
/// `${VAR}` in a value is expanded by goose from its secret store when the
/// extension starts, which is how a bearer token reaches a remote MCP server
/// without ever crossing the ACP frame. Note the failure mode: an *unknown*
/// `${VAR}` is left LITERAL rather than erroring, so the extension starts and
/// the server answers 401 — a header credential fails open, unlike a stdio
/// env key, which fails closed at startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

impl HttpHeader {
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// An environment variable ACP can set on a stdio MCP child (`EnvVariable`).
///
/// Present because the schema has it, not because this client sends one — see
/// [`StdioMcpServer`], which owns the only `env` field here and keeps it
/// private and empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

/// The `type: "http"` discriminator, as its own type so it cannot be spelled
/// wrong or left off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpTransport {
    Http,
}

/// A remote MCP server reached over streamable HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpMcpServer {
    /// Always `http`. Private so the only way to build one is [`Self::new`].
    #[serde(rename = "type")]
    kind: HttpTransport,
    pub name: String,
    pub url: String,
    pub headers: Vec<HttpHeader>,
}

impl HttpMcpServer {
    #[must_use]
    pub fn new(name: impl Into<String>, url: impl Into<String>, headers: Vec<HttpHeader>) -> Self {
        Self {
            kind: HttpTransport::Http,
            name: name.into(),
            url: url.into(),
            headers,
        }
    }
}

/// A local MCP server the agent host launches as a child process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StdioMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// ACP requires this key on a stdio server, and this client always sends
    /// it empty — which is why it is private with no setter.
    ///
    /// Inline `env` values *do* cross the ACP frame in plaintext, and goose
    /// promotes them into its secret store on arrival, so populating it would
    /// put a credential on the wire for no benefit. Worse, the session-level
    /// `session/extensions/add` rejects the whole request when it sees one
    /// ("extension env values must be passed via envKeys referencing stored
    /// secrets"). Credentials travel as `envKeys` — names of secrets already
    /// stored server-side — and nothing else.
    env: Vec<EnvVariable>,
}

impl StdioMcpServer {
    #[must_use]
    pub fn new(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args,
            env: Vec::new(),
        }
    }
}

/// Which transport an `mcp` extension speaks.
///
/// ACP tags these inconsistently and this mirrors that rather than tidying it
/// up: `McpServerHttp` carries a `type: "http"` discriminator, while
/// `McpServerStdio` carries no `type` at all — stdio is the transport every
/// agent must support, and it kept the original untagged shape. An internally
/// tagged enum would therefore fail to *parse* goose's own
/// `config/extensions/list` reply, which spells stdio servers with no tag. So
/// this is `untagged`, discriminating on `url` versus `command`.
///
/// (Connector manifests in the brain repo do write `server.type: stdio`. That
/// is a manifest-level convenience their validator allows as an extra key; it
/// is not part of the ACP schema, and it is not sent from here.)
///
/// `sse` is deliberately absent. goose's ACP layer refuses it outright — "SSE
/// is unsupported, migrate to `streamable_http`" — and a live `initialize`
/// reports `mcpCapabilities: { http: true, sse: false }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServer {
    Http(HttpMcpServer),
    Stdio(StdioMcpServer),
}

impl McpServer {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Http(s) => &s.name,
            Self::Stdio(s) => &s.name,
        }
    }

    /// `"http"` or `"stdio"`, for a one-line summary in the UI.
    #[must_use]
    pub const fn transport(&self) -> &'static str {
        match self {
            Self::Http(_) => "http",
            Self::Stdio(_) => "stdio",
        }
    }
}

/// One extension, in the shape `_goose/unstable/config/extensions/*` and
/// `_goose/unstable/extensions/available` speak.
///
/// Modelled against goose 1.46.0, the pinned version. `clientId`,
/// `clientSecretKey` and `scopes` exist on the `mcp` variant from 1.47.0 and
/// are absent here on purpose: they are OAuth machinery, `scopes` without
/// `clientId` is a hard config error, and OAuth cannot be completed from a
/// phone anyway (goose binds the redirect URI on the agent host, never puts
/// the authorization URL in an ACP message, and refuses URL-mode elicitation
/// at the ACP bridge). Extra fields a newer server sends are ignored, not
/// rejected, so this parses a 1.47 reply fine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GooseExtension {
    Builtin {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// `snake_case` on the wire. Not a typo — see the note above this type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundled: Option<bool>,
        /// `snake_case` on the wire, and `None` means EVERY TOOL IS ALLOWED.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available_tools: Option<Vec<String>>,
    },
    Platform {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available_tools: Option<Vec<String>>,
    },
    Mcp {
        server: Box<McpServer>,
        /// `camelCase` on the wire — the one field goose renames explicitly.
        /// Names of secrets already in goose's store; never values.
        #[serde(default, rename = "envKeys", skip_serializing_if = "Vec::is_empty")]
        env_keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socket: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundled: Option<bool>,
        /// `snake_case` on the wire, and `None` means EVERY TOOL IS ALLOWED.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        available_tools: Option<Vec<String>>,
    },
}

impl GooseExtension {
    /// Build an `mcp` extension with a tool allowlist.
    ///
    /// `available_tools` is taken by value and stored as-is: an empty vector
    /// would serialize as an empty allowlist, which goose reads as "allow
    /// everything". [`AcpClient::add_extension_verified`] refuses to send
    /// one, so the mistake cannot leave this crate.
    ///
    /// [`AcpClient::add_extension_verified`]: crate::AcpClient::add_extension_verified
    #[must_use]
    pub fn mcp(
        server: McpServer,
        env_keys: Vec<String>,
        description: impl Into<String>,
        available_tools: Vec<String>,
    ) -> Self {
        Self::Mcp {
            server: Box::new(server),
            env_keys,
            description: Some(description.into()),
            // 300s matches the connector manifests: `uvx`/`npx` may fetch the
            // server package on first launch, which the default timeout does
            // not survive on a cold cache.
            timeout: Some(300),
            socket: None,
            bundled: None,
            available_tools: Some(available_tools),
        }
    }

    /// The extension's name — for `mcp`, the server's name, which is where
    /// goose reads it from too.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin { name, .. } | Self::Platform { name, .. } => name,
            Self::Mcp { server, .. } => server.name(),
        }
    }

    #[must_use]
    pub fn description(&self) -> Option<&str> {
        match self {
            Self::Builtin { description, .. }
            | Self::Platform { description, .. }
            | Self::Mcp { description, .. } => description.as_deref(),
        }
    }

    /// The tool allowlist. **An empty slice means every tool is allowed** —
    /// it is not the safe default it reads like, and the UI says so out loud.
    #[must_use]
    pub fn available_tools(&self) -> &[String] {
        match self {
            Self::Builtin {
                available_tools, ..
            }
            | Self::Platform {
                available_tools, ..
            }
            | Self::Mcp {
                available_tools, ..
            } => available_tools.as_deref().unwrap_or(&[]),
        }
    }

    /// Names of the secrets this extension needs from goose's secret store.
    #[must_use]
    pub fn env_keys(&self) -> &[String] {
        match self {
            Self::Mcp { env_keys, .. } => env_keys,
            Self::Builtin { .. } | Self::Platform { .. } => &[],
        }
    }

    /// `"builtin"`, `"platform"`, `"stdio"` or `"http"`.
    #[must_use]
    pub fn transport(&self) -> &'static str {
        match self {
            Self::Builtin { .. } => "builtin",
            Self::Platform { .. } => "platform",
            Self::Mcp { server, .. } => server.transport(),
        }
    }
}

/// One row of `_goose/unstable/config/extensions/list`.
///
/// An extension plus whether it is switched on, and the key `set-enabled` and
/// `remove` address it by. goose derives `config_key` from the name
/// (lowercased, with anything outside `[A-Za-z0-9_-]` folded to `_`), but it
/// is echoed here so the client never has to reimplement that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GooseExtensionEntry {
    pub extension: GooseExtension,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub config_key: Option<String>,
}

/// Reply to `_goose/unstable/config/extensions/list`.
///
/// `warnings` carries config-file problems goose noticed while loading —
/// worth showing, because an extension that failed to parse is simply missing
/// from `extensions` otherwise.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConfigExtensions {
    #[serde(default)]
    pub extensions: Vec<GooseExtensionEntry>,
    #[serde(default)]
    pub warnings: Vec<String>,
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

    /// The test this whole module exists for.
    ///
    /// `available_tools` must serialize `snake_case` and `envKeys` `camelCase`,
    /// in the same object. If a future refactor adds a blanket
    /// `rename_all_fields = "camelCase"` — the obvious tidy-up — this fails,
    /// which is the point: goose would accept the camelCase allowlist,
    /// silently drop it, and allow every tool the MCP server publishes.
    #[test]
    fn serializes_the_exact_wire_spellings() {
        let ext = GooseExtension::mcp(
            McpServer::Stdio(StdioMcpServer::new(
                "mail-imap",
                "uvx",
                vec!["mcp-email-server@1.4.2".into(), "stdio".into()],
            )),
            vec!["MCP_EMAIL_SERVER_PASSWORD".into()],
            "IMAP mail",
            vec!["list_mailboxes".into(), "get_emails_content".into()],
        );
        let v = serde_json::to_value(&ext).unwrap();
        let obj = v.as_object().unwrap();

        assert!(
            obj.contains_key("available_tools"),
            "the allowlist MUST be snake_case; camelCase is silently dropped \
             by goose and an absent allowlist allows every tool: {v}"
        );
        assert!(
            !obj.contains_key("availableTools"),
            "camelCase availableTools would be dropped on the floor: {v}"
        );
        assert!(
            obj.contains_key("envKeys"),
            "envKeys MUST be camelCase — it is the one field goose renames: {v}"
        );
        assert!(
            !obj.contains_key("env_keys"),
            "snake_case env_keys is the config.yaml spelling, not the wire one: {v}"
        );

        // And the whole frame, so an accidental extra field is visible too.
        assert_eq!(
            v,
            json!({
                "type": "mcp",
                "server": {
                    "name": "mail-imap",
                    "command": "uvx",
                    "args": ["mcp-email-server@1.4.2", "stdio"],
                    // Always present, always empty: an inline value would put
                    // the credential in the ACP frame in plaintext.
                    "env": [],
                },
                "envKeys": ["MCP_EMAIL_SERVER_PASSWORD"],
                "description": "IMAP mail",
                "timeout": 300,
                "available_tools": ["list_mailboxes", "get_emails_content"],
            })
        );
    }

    /// `display_name` is `snake_case` too, for exactly the same reason: the
    /// `rename_all = "snake_case"` on goose's enum renames variants, not
    /// fields, and nothing renames this one.
    #[test]
    fn builtin_display_name_is_snake_case_on_the_wire() {
        let ext = GooseExtension::Builtin {
            name: "developer".into(),
            description: None,
            display_name: Some("Developer".into()),
            timeout: None,
            bundled: Some(true),
            available_tools: Some(vec!["shell".into()]),
        };
        let v = serde_json::to_value(&ext).unwrap();
        assert_eq!(
            v,
            json!({
                "type": "builtin",
                "name": "developer",
                "display_name": "Developer",
                "bundled": true,
                "available_tools": ["shell"],
            })
        );
    }

    /// An http server carries `type: "http"` and its headers are an ARRAY of
    /// `{name, value}` — not the mapping the connector manifests use.
    #[test]
    fn http_server_is_tagged_and_headers_are_a_list() {
        let ext = GooseExtension::mcp(
            McpServer::Http(HttpMcpServer::new(
                "todoist",
                "https://ai.todoist.net/mcp",
                vec![HttpHeader::new(
                    "Authorization",
                    "Bearer ${TODOIST_API_KEY}",
                )],
            )),
            vec!["TODOIST_API_KEY".into()],
            "Todoist tasks",
            vec!["find-tasks".into()],
        );
        let v = serde_json::to_value(&ext).unwrap();
        assert_eq!(
            v.pointer("/server/type").and_then(Value::as_str),
            Some("http")
        );
        assert_eq!(
            v.pointer("/server/headers"),
            Some(&json!([{"name": "Authorization", "value": "Bearer ${TODOIST_API_KEY}"}]))
        );
        // No `url`-less variant confusion: the field is `url`, not `uri`.
        // `uri` is what goose writes into config.yaml, one layer down.
        assert_eq!(
            v.pointer("/server/url").and_then(Value::as_str),
            Some("https://ai.todoist.net/mcp")
        );
    }

    /// goose spells stdio servers with no `type` at all (stdio is ACP's
    /// untagged fallback transport), so a list reply must still parse.
    #[test]
    fn parses_a_stdio_server_with_no_type_tag() {
        let raw = json!({
            "extension": {
                "type": "mcp",
                "server": {"name": "mail-imap", "command": "uvx", "args": ["x"], "env": []},
                "envKeys": ["MCP_EMAIL_SERVER_PASSWORD"],
                "timeout": 300,
                "available_tools": ["list_mailboxes"]
            },
            "enabled": true,
            "configKey": "mail-imap"
        });
        let entry: GooseExtensionEntry = serde_json::from_value(raw).unwrap();
        assert!(entry.enabled);
        assert_eq!(entry.config_key.as_deref(), Some("mail-imap"));
        assert_eq!(entry.extension.name(), "mail-imap");
        assert_eq!(entry.extension.transport(), "stdio");
        assert_eq!(entry.extension.available_tools(), ["list_mailboxes"]);
        assert_eq!(entry.extension.env_keys(), ["MCP_EMAIL_SERVER_PASSWORD"]);
    }

    /// The failure this codebase is built around: goose omits
    /// `available_tools` entirely when the stored allowlist is empty, and an
    /// omitted allowlist means every tool is allowed. Parsing must surface
    /// that as an empty slice so callers can reject it — not as something
    /// that reads like a safe default.
    #[test]
    fn a_missing_allowlist_reads_as_empty_meaning_everything() {
        let raw = json!({
            "type": "mcp",
            "server": {"name": "probe_camel", "command": "uvx", "args": [], "env": []},
        });
        let ext: GooseExtension = serde_json::from_value(raw).unwrap();
        assert!(ext.available_tools().is_empty());
    }

    /// A 1.47 server sends OAuth fields this client does not model. They must
    /// be ignored, not rejected — there is no `deny_unknown_fields` here for
    /// the same reason goose has none.
    #[test]
    fn newer_server_fields_are_ignored_not_fatal() {
        let raw = json!({
            "type": "mcp",
            "server": {"type": "http", "name": "x", "url": "https://x/mcp", "headers": []},
            "clientId": "abc",
            "clientSecretKey": "X_SECRET",
            "scopes": ["read"],
            "available_tools": ["a"]
        });
        let ext: GooseExtension = serde_json::from_value(raw).unwrap();
        assert_eq!(ext.transport(), "http");
        assert_eq!(ext.available_tools(), ["a"]);
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
