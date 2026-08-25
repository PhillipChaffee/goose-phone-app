//! Extensions: the `_goose/unstable/{config,session}/extensions/*` methods and
//! the wire types behind them.
//!
//! An "extension" is goose's word for an MCP server it has been told about.
//! Two properties shape everything in this file, and both of them are about
//! failing closed.
//!
//! **The tool allowlist is one word away from being a no-op.** See the casing
//! note above [`GooseExtension`]: `available_tools` is `snake_case` while its
//! neighbours are camelCase, goose sets no `deny_unknown_fields`, and an
//! allowlist that does not arrive means *every* tool the MCP server publishes
//! is callable. So [`AcpClient::add_extension_verified`] does not believe an
//! `add` that returned OK.
//!
//! **A credential goes one way only.** Secrets are written with
//! [`AcpClient::store_secret`] and never read back — there is deliberately no
//! `config/read` wrapper in this crate, because `config/read` on a secret
//! returns a clear prefix. Verification is a handshake instead:
//! [`AcpClient::verify_extension_starts`].

use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{LIST_TIMEOUT, MUTATE_TIMEOUT};
use crate::client::AcpClient;
use crate::error::AcpError;

/// Timeout for bringing an extension up inside a live session.
///
/// Generous, and deliberately not [`MUTATE_TIMEOUT`]: the config-plane calls
/// touch a file and return, while this one launches an MCP server. A first
/// launch may fetch the server package (`uvx`, `npx`) over the network, which
/// a 30 s budget does not survive on a cold cache.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Wire types.
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
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HttpHeader {
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            extra: Map::new(),
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
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `type: "http"` discriminator, as its own type so it cannot be spelled
/// wrong or left off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpTransport {
    #[serde(rename = "http")]
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
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HttpMcpServer {
    #[must_use]
    pub fn new(name: impl Into<String>, url: impl Into<String>, headers: Vec<HttpHeader>) -> Self {
        Self {
            kind: HttpTransport::Http,
            name: name.into(),
            url: url.into(),
            headers,
            extra: Map::new(),
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
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl StdioMcpServer {
    #[must_use]
    pub fn new(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args,
            env: Vec::new(),
            extra: Map::new(),
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
///
/// No `extra` catch-all here, and there cannot be one: serde has nowhere to
/// put a flattened map on an enum. The two variants carry one each, so a
/// field goose adds to either shape still round-trips.
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
/// at the ACP bridge). They land in `extra` instead, so a 1.47 reply parses
/// and writes back unchanged rather than being rejected or truncated.
///
/// Every `Option` here keeps goose's own `skip_serializing_if`, against the
/// module's usual rule. That is not a slip: this type is *sent* as well as
/// received, and `available_tools: null` on the way out is a different wire
/// message from an absent key. Fixtures for this type are therefore complete
/// in the strong sense — every optional field present and non-null — because
/// a `null` is the one thing they cannot use as evidence.
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
        #[serde(flatten)]
        extra: Map<String, Value>,
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
        #[serde(flatten)]
        extra: Map<String, Value>,
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
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

impl GooseExtension {
    /// Build an `mcp` extension with a tool allowlist.
    ///
    /// `available_tools` is taken by value and stored as-is: an empty vector
    /// would serialize as an empty allowlist, which goose reads as "allow
    /// everything". [`AcpClient::add_extension_verified`] refuses to send
    /// one, so the mistake cannot leave this crate.
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
            extra: Map::new(),
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
pub struct GooseExtensionEntry {
    pub extension: GooseExtension,
    #[serde(default)]
    pub enabled: bool,
    /// `camelCase` on the wire: goose puts `rename_all = "camelCase"` on the
    /// *entry* struct, unlike the extension inside it. Spelled out per field
    /// rather than by a blanket rule, so the two casings sit side by side in
    /// the diff instead of one being inferred.
    #[serde(default, rename = "configKey")]
    pub config_key: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Reply to `_goose/unstable/config/extensions/list`.
///
/// `warnings` carries config-file problems goose noticed while loading —
/// worth showing, because an extension that failed to parse is simply missing
/// from `extensions` otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigExtensions {
    #[serde(default)]
    pub extensions: Vec<GooseExtensionEntry>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Reply to `_goose/unstable/extensions/available`. Private: callers get the
/// vector, not the envelope.
///
/// The one DTO in this module with no `extra` catch-all. It is never sent
/// back and never round-trip tested, so a flattened map here would be a field
/// nothing ever reads — a dead-code warning standing in for a guarantee it
/// cannot give. The `GooseExtension`s inside it carry their own.
#[derive(Debug, Deserialize)]
struct AvailableExtensions {
    #[serde(default)]
    extensions: Vec<GooseExtension>,
}

/// `a, b, c`, or `(none)` for an empty list, for an error message a human
/// reads on a phone.
fn fmt_list(items: &[&str]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

// ---------------------------------------------------------------------------
// The methods.
//
// All of these are `_goose/unstable/*` methods, present at goose 1.46.0 — no
// protocol version bump is needed for any of them.
//
// A note on transport, because goose's HTTP ACP mode has a trap that does NOT
// apply here: over `POST /acp` the server *assigns* a connection id in the
// `acp-connection-id` response header on `initialize`, every later request has
// to echo it back, and the replies arrive on a separate SSE channel. This
// client speaks ACP over a WebSocket instead — one socket is the connection,
// the actor correlates by JSON-RPC id, and there is no header to carry. These
// methods are therefore plain requests like every other, with nothing extra to
// thread through.

impl AcpClient {
    /// Extensions goose knows how to offer but that are not necessarily
    /// configured — the built-ins and platform extensions it ships with.
    ///
    /// Note that most of these come back with no `available_tools`, i.e.
    /// unrestricted. That is a fact about goose's catalogue, not a suggestion.
    ///
    /// The Connect screen deliberately does not use this. It offers a small
    /// curated catalogue compiled into the app instead, for two reasons this
    /// method cannot fix: goose's own catalogue is unrestricted, so adding
    /// from it would mean adding without an allowlist; and OAuth-based
    /// services are absent from the curated list *on purpose*, because goose
    /// binds the redirect URI on the agent host, never puts the authorization
    /// URL in an ACP message, and refuses URL-mode elicitation at the ACP
    /// bridge — an OAuth connector cannot be finished from a phone at all.
    /// What works from a phone is a bearer token or an app password. This
    /// stays because it is tested library surface for callers that are not
    /// this app.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if the server does not offer extensions at
    /// all, [`AcpError::Timeout`], [`AcpError::Closed`], [`AcpError::Rpc`], or
    /// [`AcpError::Transport`] if the reply does not parse.
    pub async fn extensions_available(&self) -> Result<Vec<GooseExtension>, AcpError> {
        let raw = self
            .goose_request(
                "_goose/unstable/extensions/available",
                json!({}),
                LIST_TIMEOUT,
            )
            .await?;
        let listed: AvailableExtensions =
            serde_json::from_value(raw).map_err(|e| AcpError::Transport(e.to_string()))?;
        Ok(listed.extensions)
    }

    /// The extensions persisted in the server's global goose config, each
    /// with its enabled flag and the `config_key` that addresses it.
    ///
    /// # Errors
    ///
    /// As [`Self::extensions_available`].
    pub async fn config_extensions_list(&self) -> Result<ConfigExtensions, AcpError> {
        let raw = self
            .goose_request(
                "_goose/unstable/config/extensions/list",
                json!({}),
                LIST_TIMEOUT,
            )
            .await?;
        serde_json::from_value(raw).map_err(|e| AcpError::Transport(e.to_string()))
    }

    /// Persist an extension to the server's global goose config.
    ///
    /// Prefer [`Self::add_extension_verified`]: this one returns as soon as
    /// the server says OK, and the server saying OK is *not* evidence that
    /// the tool allowlist was applied.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if goose rejects the extension (an `sse` server, an
    /// unsupported field combination), [`AcpError::Unsupported`] on a server
    /// without extensions, plus the usual timeout/closed cases.
    pub async fn config_extension_add(
        &self,
        extension: &GooseExtension,
        enabled: bool,
    ) -> Result<(), AcpError> {
        self.goose_request(
            "_goose/unstable/config/extensions/add",
            json!({"extension": extension, "enabled": enabled}),
            MUTATE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// Add an extension and then prove the tool allowlist actually stuck.
    ///
    /// This is the only `add` the app calls, and the read-back is not
    /// belt-and-braces — it is the control itself. Three things can leave an
    /// extension unrestricted with no error anywhere:
    ///
    /// 1. sending `availableTools` instead of `available_tools` (goose has no
    ///    `deny_unknown_fields`, so the field is dropped in silence);
    /// 2. sending an empty allowlist, which goose stores and then reads back
    ///    as "allow everything";
    /// 3. a server old or new enough to have moved the field.
    ///
    /// All three end the same way: `available_tools` comes back empty or
    /// absent. So an empty allowlist is refused before anything is sent, and
    /// afterwards the persisted set must match the sent set exactly.
    /// Anything else is [`AcpError::Verification`] — a hard error, never a
    /// warning, because failing open here is the security bug.
    ///
    /// The order matters as much as the check. The extension is **always**
    /// added switched off, whatever `enabled` asks for, and only switched on
    /// after the read-back matches. Adding it enabled and switching it off on
    /// failure would leave it live and unrestricted for a round trip — a
    /// window that need not exist, and one that stays open forever if the
    /// switch-off call is the thing that fails.
    ///
    /// The extension stays persisted-and-disabled when this fails; the caller
    /// should surface the error and offer to remove it.
    ///
    /// # Errors
    ///
    /// [`AcpError::Verification`] if the allowlist is empty going out, or is
    /// missing, empty or different coming back; [`AcpError::Config`] if the
    /// server stored it without a `config_key`, leaving nothing to address the
    /// switch-on with. Otherwise as [`Self::config_extension_add`] — and every
    /// one of those leaves it disabled.
    pub async fn add_extension_verified(
        &self,
        extension: &GooseExtension,
        enabled: bool,
    ) -> Result<GooseExtensionEntry, AcpError> {
        let name = extension.name().to_string();
        let sent: BTreeSet<&str> = extension
            .available_tools()
            .iter()
            .map(String::as_str)
            .collect();
        if sent.is_empty() {
            return Err(AcpError::Verification(format!(
                "refusing to add `{name}` with an empty tool allowlist — goose reads \
                 an empty allowlist as \"allow every tool this server publishes\""
            )));
        }

        // Disabled, always. See the note above: an unrestricted extension
        // must never be live, not even for the length of the read-back.
        self.config_extension_add(extension, false).await?;

        let listed = self.config_extensions_list().await?;
        let Some(entry) = listed
            .extensions
            .into_iter()
            .find(|e| e.extension.name() == name)
        else {
            return Err(AcpError::Verification(format!(
                "`{name}` was accepted but is not in config/extensions/list — \
                 refusing to treat it as configured"
            )));
        };

        let got: BTreeSet<&str> = entry
            .extension
            .available_tools()
            .iter()
            .map(String::as_str)
            .collect();
        if got.is_empty() {
            return Err(AcpError::Verification(format!(
                "`{name}` came back with NO tool allowlist, which means every tool \
                 is allowed. The field is `available_tools` (snake_case) on the ACP \
                 wire; a camelCase spelling is accepted and silently dropped."
            )));
        }
        if got != sent {
            let missing: Vec<&str> = sent.difference(&got).copied().collect();
            let extra: Vec<&str> = got.difference(&sent).copied().collect();
            return Err(AcpError::Verification(format!(
                "`{name}` came back with a different tool allowlist — dropped: {}; \
                 added: {}",
                fmt_list(&missing),
                fmt_list(&extra)
            )));
        }

        if !enabled {
            return Ok(entry);
        }
        let Some(key) = entry.config_key.clone() else {
            return Err(AcpError::Config(format!(
                "`{name}` was stored without a config key, so there is nothing to \
                 switch it on with. It is configured but disabled."
            )));
        };
        self.config_extension_set_enabled(&key, true).await?;
        Ok(GooseExtensionEntry {
            enabled: true,
            ..entry
        })
    }

    /// Switch a configured extension on or off. `config_key` is the value
    /// [`GooseExtensionEntry::config_key`] carries.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] with `invalid_params` if no extension has that key,
    /// [`AcpError::Unsupported`] on a server without extensions, plus the
    /// usual timeout/closed cases.
    pub async fn config_extension_set_enabled(
        &self,
        config_key: &str,
        enabled: bool,
    ) -> Result<(), AcpError> {
        self.goose_request(
            "_goose/unstable/config/extensions/set-enabled",
            json!({"configKey": config_key, "enabled": enabled}),
            MUTATE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// Bring an extension up in one live session, which is also the only
    /// honest way to check a credential: goose launches the MCP server here
    /// and fails if it cannot start. A stdio server whose `envKeys` name a
    /// secret that is missing dies at startup, and that error comes back
    /// through this call.
    ///
    /// The timeout is generous because a first launch may fetch the server
    /// package (`uvx`, `npx`) over the network.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the extension will not start, if the session is
    /// unknown, or if the extension carries inline `env` values (goose
    /// rejects those outright at the session level — this crate cannot
    /// produce one), [`AcpError::Unsupported`] on a server without
    /// extensions, [`AcpError::Timeout`] after 120 s, or [`AcpError::Closed`].
    pub async fn session_extension_add(
        &self,
        session_id: &str,
        extension: &GooseExtension,
    ) -> Result<(), AcpError> {
        self.goose_request(
            "_goose/unstable/session/extensions/add",
            json!({"sessionId": session_id, "extension": extension}),
            HANDSHAKE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// Prove an extension actually starts, whether or not a session is open.
    ///
    /// [`Self::session_extension_add`] is the only honest credential check
    /// there is, and it needs a session. On a fresh install there is none —
    /// connecting a service does not open a chat — so this creates a throwaway
    /// one, hand shakes in it, and deletes it again. Skipping the check
    /// instead would report "connected" for a mistyped credential, and for a
    /// `${VAR}` bearer header goose leaves an unknown variable LITERAL rather
    /// than erroring, so the failure would not surface until a 401 mid-task.
    ///
    /// The throwaway session is deleted whether the handshake passed or
    /// failed, and its deletion is best-effort: a session left behind is
    /// clutter, while the handshake's verdict is the answer the caller asked
    /// for.
    ///
    /// `cwd` must be an absolute path on the server; it is only ever used for
    /// the throwaway session.
    ///
    /// # Errors
    ///
    /// As [`Self::session_extension_add`], plus [`Self::session_new`]'s errors
    /// when there is no session to borrow and one cannot be created.
    pub async fn verify_extension_starts(
        &self,
        session_id: Option<&str>,
        cwd: &str,
        extension: &GooseExtension,
    ) -> Result<(), AcpError> {
        if let Some(session_id) = session_id {
            return self.session_extension_add(session_id, extension).await;
        }
        let session = self.session_new(cwd).await?;
        let outcome = self
            .session_extension_add(&session.session_id, extension)
            .await;
        let _ = self.session_delete(&session.session_id).await;
        outcome
    }

    /// Write one credential into goose's secret store, where `envKeys` and
    /// `${VAR}` header substitution resolve it from.
    ///
    /// `isSecret` is hard-coded true and the value is always sent as a JSON
    /// string, both deliberately:
    ///
    /// - a non-secret write lands in plaintext `config.yaml` instead of
    ///   `secrets.yaml` (mode 0600);
    /// - a value that is not a JSON *string* — a numeric app password like
    ///   `12345678` parsed as a number, say — makes goose log "Secret value is
    ///   not a string; skipping" and start the extension with **no**
    ///   credential. Sending `String` unconditionally makes that impossible.
    ///
    /// There is intentionally no `config/read` wrapper in this crate.
    /// `config/read` on a secret returns the first `min(len/2, 8)` characters
    /// in clear plus the exact length, so "just check what we stored" is a
    /// leak. Verify with [`Self::session_extension_add`] instead.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if goose cannot write its secret store,
    /// [`AcpError::Unsupported`] if the server has no config plane,
    /// [`AcpError::Timeout`] after 30 s, or [`AcpError::Closed`].
    pub async fn store_secret(&self, key: &str, value: &str) -> Result<(), AcpError> {
        self.goose_request(
            "_goose/unstable/config/upsert",
            json!({"key": key, "value": value, "isSecret": true}),
            MUTATE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;
    use serde_json::json;

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
            extra: Map::new(),
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
    /// the same reason goose has none — and they must survive a write-back,
    /// which is what `extra` is for.
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
        let ext: GooseExtension = crate::assert_round_trip(&raw);
        assert_eq!(ext.transport(), "http");
        assert_eq!(ext.available_tools(), ["a"]);
    }
}
