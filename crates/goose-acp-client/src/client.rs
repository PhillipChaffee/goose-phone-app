//! Async client for the goose ACP WebSocket endpoint.
//!
//! One background task owns the socket. Requests are correlated by JSON-RPC
//! id; notifications and agent-initiated requests are surfaced through an
//! [`AcpEvent`] channel. `session/prompt` stays pending for the whole agent
//! turn, so requests carry no default timeout — callers opt in per call.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::types::ConfigOption;
use tokio_tungstenite::{connect_async_tls_with_config, MaybeTlsStream, WebSocketStream};

use crate::tls;
use crate::types::{
    AcpEvent, ConfigExtensions, ContentBlock, GooseExtension, GooseExtensionEntry, InitializeInfo,
    NewSessionResponse, PermissionRequest, SessionListResponse, SessionUpdate,
};

pub const CLIENT_NAME: &str = "goose-mobile";

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("connection closed")]
    Closed,
    #[error("timed out")]
    Timeout,
    /// A JSON-RPC error object from the agent.
    ///
    /// The reason a human can act on lives in `data`, not in `message`. goose
    /// builds its errors with `Error::internal_error().data(e.to_string())`
    /// and `Error::invalid_params().data(...)`, so `message` is whatever the
    /// JSON-RPC code is canonically called — "Internal error", "Invalid
    /// params" — and every sentence worth reading is in `data`: "Extension
    /// '{}' not found", "SSE is unsupported, migrate to `streamable_http`", the
    /// envKeys-vs-inline-env explanation, an MCP server's startup failure.
    /// goose's own desktop client reads `data` first for exactly this reason.
    ///
    /// So [`Display`](std::fmt::Display) renders a string `data` in preference
    /// to `message`, and falls back to `message` for an agent that sends none.
    #[error("{}", rpc_reason(.message, .data.as_ref()))]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("invalid configuration: {0}")]
    Config(String),
    /// An extension's tool allowlist did not survive the round trip, so the
    /// extension is running unrestricted. Its own variant because it must
    /// never be handled as "some RPC hiccup" — see
    /// [`AcpClient::add_extension_verified`].
    #[error("{0}")]
    Allowlist(String),
}

/// The readable half of a JSON-RPC error: `data` when it carries a non-blank
/// string, otherwise the canned `message`. A non-string `data` (an object, a
/// number) is not something to show on a phone, so it falls back too.
fn rpc_reason<'a>(message: &'a str, data: Option<&'a Value>) -> &'a str {
    data.and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or(message)
}

/// How to reach the server.
#[derive(Debug, Clone, Default)]
pub struct ConnectConfig {
    /// Base URL, e.g. `https://goose-box.tailnet.ts.net`,
    /// `http://100.101.102.103:3284`, or `myhost:3284` (scheme defaults to
    /// `http`, which the tailnet already encrypts).
    pub base_url: String,
    /// The `GOOSE_SERVER__SECRET_KEY` value. Empty only for a server started
    /// with `--dangerously-unauthenticated`.
    pub secret: String,
    /// SHA-256 pin for a `goose serve --tls` self-signed cert
    /// (`GOOSED_CERT_FINGERPRINT`). `None` = normal `WebPKI` validation.
    pub fingerprint: Option<[u8; 32]>,
}

/// Split user input into the `(scheme, host[:port])` pair the public URL
/// helpers are built from. A missing scheme defaults to `http`; any path,
/// query or fragment is dropped.
fn split_base_url(input: &str) -> Result<(&'static str, &str), AcpError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AcpError::Config("server URL is empty".into()));
    }
    let (scheme, rest) = trimmed.split_once("://").unwrap_or(("http", trimmed));
    let scheme = match scheme {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        other => {
            return Err(AcpError::Config(format!(
                "unsupported URL scheme `{other}` (use http, https, ws, or wss)"
            )))
        }
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err(AcpError::Config("server URL has no host".into()));
    }
    Ok((scheme, host))
}

/// Normalize user input to an `http(s)://host[:port]` base with no trailing
/// slash or path.
///
/// # Errors
///
/// [`AcpError::Config`] if the input is blank, carries no host, or names a
/// scheme other than `http`, `https`, `ws` or `wss`.
pub fn normalize_base_url(input: &str) -> Result<String, AcpError> {
    let (scheme, host) = split_base_url(input)?;
    Ok(format!("{scheme}://{host}"))
}

/// Derive the ACP WebSocket URL from a normalized base URL.
///
/// # Errors
///
/// [`AcpError::Config`] for the same inputs [`normalize_base_url`] rejects:
/// blank, host-less, or an unsupported scheme.
pub fn ws_url(base_url: &str) -> Result<String, AcpError> {
    let (scheme, host) = split_base_url(base_url)?;
    let ws_scheme = if scheme == "https" { "wss" } else { "ws" };
    Ok(format!("{ws_scheme}://{host}/acp"))
}

enum Cmd {
    Request {
        method: String,
        params: Value,
        reply: oneshot::Sender<Result<Value, AcpError>>,
    },
    Notify {
        method: String,
        params: Value,
    },
    Respond {
        id: Value,
        result: Result<Value, (i64, String)>,
    },
    Close,
}

/// Cloneable handle to a live ACP connection.
#[derive(Clone, Debug)]
pub struct AcpClient {
    tx: mpsc::UnboundedSender<Cmd>,
}

/// Timeout for the config-plane calls (listing, adding and toggling
/// extensions, writing a secret). These touch a file on the server and return;
/// none of them starts an MCP process, so they are quick or they are broken.
const CONFIG_TIMEOUT: Duration = Duration::from_secs(30);

/// Reply to `_goose/unstable/extensions/available`. Private: callers get the
/// vector, not the envelope.
#[derive(Debug, serde::Deserialize)]
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

/// Pull a `configOptions` array out of any response that carries one.
///
/// `session/new` types it; `session/load` and `session/set_config_option`
/// come back as raw JSON, and all three carry the same array.
#[must_use]
pub fn config_options_from(raw: &Value) -> Vec<ConfigOption> {
    raw.get("configOptions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

impl AcpClient {
    /// Connect, perform the ACP `initialize` handshake, and return the handle
    /// together with the event stream and the agent's identity.
    ///
    /// # Errors
    ///
    /// [`AcpError::Config`] if `base_url` is unusable or the secret cannot go
    /// in an HTTP header; [`AcpError::Timeout`] if the socket is not up within
    /// 20 s or `initialize` is unanswered for 15 s; [`AcpError::Connect`] if
    /// the server refuses the upgrade — a 401/403 there means the secret was
    /// rejected; [`AcpError::Rpc`] if the agent itself fails `initialize`.
    pub async fn connect(
        cfg: &ConnectConfig,
    ) -> Result<(Self, mpsc::Receiver<AcpEvent>, InitializeInfo), AcpError> {
        let url = ws_url(&cfg.base_url)?;
        let mut request = url
            .into_client_request()
            .map_err(|e| AcpError::Config(e.to_string()))?;
        if !cfg.secret.is_empty() {
            let value = HeaderValue::from_str(&cfg.secret)
                .map_err(|_| AcpError::Config("secret key contains invalid characters".into()))?;
            request.headers_mut().insert("X-Secret-Key", value);
        }

        let connector = tls::build_connector(cfg.fingerprint);
        let connect = connect_async_tls_with_config(request, None, false, Some(connector));
        let (socket, _response) = tokio::time::timeout(Duration::from_secs(20), connect)
            .await
            .map_err(|_| AcpError::Timeout)?
            .map_err(|e| AcpError::Connect(describe_ws_error(&e)))?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel(1024);
        tokio::spawn(actor(socket, cmd_rx, event_tx));

        let client = Self { tx: cmd_tx };
        let init = client
            .request_with_timeout(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientInfo": {
                        "name": CLIENT_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "clientCapabilities": {
                        "fs": {"readTextFile": false, "writeTextFile": false},
                        "terminal": false,
                        "_meta": {"goose": {"customNotifications": true}},
                    },
                }),
                Duration::from_secs(15),
            )
            .await?;

        let agent_name = init
            .pointer("/agentInfo/name")
            .and_then(Value::as_str)
            .unwrap_or("goose")
            .to_string();
        let agent_version = init
            .pointer("/agentInfo/version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        Ok((
            client,
            event_rx,
            InitializeInfo {
                agent_name,
                agent_version,
                raw: init,
            },
        ))
    }

    /// Send a JSON-RPC request and await its response (no timeout — used for
    /// `session/prompt`, which stays pending for the whole agent turn).
    ///
    /// # Errors
    ///
    /// [`AcpError::Closed`] if the connection task is gone or the socket dies
    /// before the response arrives, or [`AcpError::Rpc`] carrying the agent's
    /// JSON-RPC error object.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(Cmd::Request {
                method: method.to_string(),
                params,
                reply: reply_tx,
            })
            .map_err(|_| AcpError::Closed)?;
        reply_rx.await.map_err(|_| AcpError::Closed)?
    }

    /// # Errors
    ///
    /// [`AcpError::Timeout`] if no response arrives within `timeout`; the
    /// request is abandoned, not cancelled on the server. Otherwise as
    /// [`Self::request`]: [`AcpError::Closed`] or [`AcpError::Rpc`].
    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AcpError> {
        tokio::time::timeout(timeout, self.request(method, params))
            .await
            .map_err(|_| AcpError::Timeout)?
    }

    pub fn notify(&self, method: &str, params: Value) {
        let _ = self.tx.send(Cmd::Notify {
            method: method.to_string(),
            params,
        });
    }

    /// Close the connection. The event stream will yield `Disconnected`.
    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }

    // ---- convenience wrappers -------------------------------------------

    /// Create a session. `cwd` must be an absolute path on the *server*.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects `cwd` (relative paths and paths
    /// it cannot open are refused), [`AcpError::Timeout`] after 60 s,
    /// [`AcpError::Closed`] if the connection drops, or
    /// [`AcpError::Transport`] if the reply is not a [`NewSessionResponse`].
    pub async fn session_new(&self, cwd: &str) -> Result<NewSessionResponse, AcpError> {
        let result = self
            .request_with_timeout(
                "session/new",
                json!({
                    "cwd": cwd,
                    "mcpServers": [],
                    "_meta": {"client": CLIENT_NAME},
                }),
                Duration::from_secs(60),
            )
            .await?;
        serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))
    }

    /// Load an existing session. The server replays its history as
    /// `session/update` events *before* this resolves.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server does not know `session_id` or refuses
    /// `cwd`, [`AcpError::Timeout`] if the replay takes longer than 120 s, or
    /// [`AcpError::Closed`] if the connection drops mid-replay.
    pub async fn session_load(&self, session_id: &str, cwd: &str) -> Result<Value, AcpError> {
        self.request_with_timeout(
            "session/load",
            json!({
                "sessionId": session_id,
                "cwd": cwd,
                "mcpServers": [],
            }),
            Duration::from_secs(120),
        )
        .await
    }

    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects the cursor,
    /// [`AcpError::Timeout`] after 30 s, [`AcpError::Closed`] if the
    /// connection drops, or [`AcpError::Transport`] if the reply is not a
    /// [`SessionListResponse`].
    pub async fn session_list(
        &self,
        cursor: Option<String>,
    ) -> Result<SessionListResponse, AcpError> {
        let mut params = json!({
            "_meta": {
                "types": ["user"],
                "goose": {"includeLastMessageSnippet": true},
            }
        });
        if let Some(cursor) = cursor {
            params["cursor"] = Value::String(cursor);
        }
        let result = self
            .request_with_timeout("session/list", params, Duration::from_secs(30))
            .await?;
        serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))
    }

    /// Send a user message; resolves at end of turn with the stop reason
    /// (`end_turn`, `max_tokens`, `refusal`, `cancelled`, …).
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the agent fails the turn (an unknown session id, a
    /// provider error), or [`AcpError::Closed`] if the connection drops before
    /// the turn ends. There is no timeout — a turn may legitimately run for
    /// minutes.
    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<String, AcpError> {
        let result = self
            .request(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [ContentBlock::text(text)],
                }),
            )
            .await?;
        Ok(result
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
            .to_string())
    }

    /// Cancel the running turn; the pending `prompt` resolves with
    /// `cancelled`.
    pub fn cancel(&self, session_id: &str) {
        self.notify("session/cancel", json!({"sessionId": session_id}));
    }

    /// Change one session config option — `provider`, `model`, `mode` or
    /// `thinking_effort` — and get the full option set back.
    ///
    /// Takes effect on the session immediately; the next `session/prompt`
    /// uses it. The agent also pushes a `config_option_update` notification,
    /// so a second client watching the same session stays in step.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects the option or its value,
    /// [`AcpError::Timeout`], or [`AcpError::Closed`] if the connection
    /// drops.
    pub async fn set_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>, AcpError> {
        // `type: "id"` is the discriminator every id-based option kind uses;
        // `value` is flattened alongside it, not nested under it.
        let raw = self
            .request(
                "session/set_config_option",
                json!({
                    "sessionId": session_id,
                    "configId": config_id,
                    "type": "id",
                    "value": value,
                }),
            )
            .await?;
        Ok(config_options_from(&raw))
    }

    /// Delete a session on the server.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server does not know `session_id`,
    /// [`AcpError::Timeout`] after 30 s, or [`AcpError::Closed`] if the
    /// connection drops.
    pub async fn session_delete(&self, session_id: &str) -> Result<(), AcpError> {
        self.request_with_timeout(
            "session/delete",
            json!({"sessionId": session_id}),
            Duration::from_secs(30),
        )
        .await
        .map(|_| ())
    }

    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server does not know `session_id` or does not
    /// implement this unstable goose extension, [`AcpError::Timeout`] after
    /// 30 s, or [`AcpError::Closed`] if the connection drops.
    pub async fn session_rename(&self, session_id: &str, title: &str) -> Result<(), AcpError> {
        self.request_with_timeout(
            "_goose/unstable/session/rename",
            json!({"sessionId": session_id, "title": title}),
            Duration::from_secs(30),
        )
        .await
        .map(|_| ())
    }

    // ---- extensions (the Connect surface) --------------------------------
    //
    // All of these are `_goose/unstable/*` methods, present at goose 1.46.0 —
    // no protocol version bump is needed for any of them.
    //
    // A note on transport, because goose's HTTP ACP mode has a trap that does
    // NOT apply here: over `POST /acp` the server *assigns* a connection id in
    // the `acp-connection-id` response header on `initialize`, every later
    // request has to echo it back, and the replies arrive on a separate SSE
    // channel. This client speaks ACP over a WebSocket instead — one socket is
    // the connection, the actor in this module correlates by JSON-RPC id, and
    // there is no header to carry. These methods are therefore plain
    // `request` calls like every other, with nothing extra to thread through.

    /// Extensions goose knows how to offer but that are not necessarily
    /// configured — the built-ins and platform extensions it ships with.
    ///
    /// Note that most of these come back with no `available_tools`, i.e.
    /// unrestricted. That is a fact about goose's catalogue, not a suggestion.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server does not implement this unstable
    /// method, [`AcpError::Timeout`] after 30 s, [`AcpError::Closed`], or
    /// [`AcpError::Transport`] if the reply does not parse.
    pub async fn extensions_available(&self) -> Result<Vec<GooseExtension>, AcpError> {
        let raw = self
            .request_with_timeout(
                "_goose/unstable/extensions/available",
                json!({}),
                CONFIG_TIMEOUT,
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
            .request_with_timeout(
                "_goose/unstable/config/extensions/list",
                json!({}),
                CONFIG_TIMEOUT,
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
    /// unsupported field combination), plus the usual timeout/closed cases.
    pub async fn config_extension_add(
        &self,
        extension: &GooseExtension,
        enabled: bool,
    ) -> Result<(), AcpError> {
        self.request_with_timeout(
            "_goose/unstable/config/extensions/add",
            json!({"extension": extension, "enabled": enabled}),
            CONFIG_TIMEOUT,
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
    /// Anything else is [`AcpError::Allowlist`] — a hard error, never a
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
    /// [`AcpError::Allowlist`] if the allowlist is empty going out, or is
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
            return Err(AcpError::Allowlist(format!(
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
            return Err(AcpError::Allowlist(format!(
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
            return Err(AcpError::Allowlist(format!(
                "`{name}` came back with NO tool allowlist, which means every tool \
                 is allowed. The field is `available_tools` (snake_case) on the ACP \
                 wire; a camelCase spelling is accepted and silently dropped."
            )));
        }
        if got != sent {
            let missing: Vec<&str> = sent.difference(&got).copied().collect();
            let extra: Vec<&str> = got.difference(&sent).copied().collect();
            return Err(AcpError::Allowlist(format!(
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
    /// plus the usual timeout/closed cases.
    pub async fn config_extension_set_enabled(
        &self,
        config_key: &str,
        enabled: bool,
    ) -> Result<(), AcpError> {
        self.request_with_timeout(
            "_goose/unstable/config/extensions/set-enabled",
            json!({"configKey": config_key, "enabled": enabled}),
            CONFIG_TIMEOUT,
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
    /// produce one), [`AcpError::Timeout`] after 120 s, or
    /// [`AcpError::Closed`].
    pub async fn session_extension_add(
        &self,
        session_id: &str,
        extension: &GooseExtension,
    ) -> Result<(), AcpError> {
        self.request_with_timeout(
            "_goose/unstable/session/extensions/add",
            json!({"sessionId": session_id, "extension": extension}),
            Duration::from_secs(120),
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
    /// [`AcpError::Timeout`] after 30 s, or [`AcpError::Closed`].
    pub async fn store_secret(&self, key: &str, value: &str) -> Result<(), AcpError> {
        self.request_with_timeout(
            "_goose/unstable/config/upsert",
            json!({"key": key, "value": value, "isSecret": true}),
            CONFIG_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// Answer a `session/request_permission` request. `option_id = None`
    /// reports the prompt as cancelled.
    pub fn respond_permission(&self, request_id: Value, option_id: Option<String>) {
        let outcome = match option_id {
            Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
            None => json!({"outcome": {"outcome": "cancelled"}}),
        };
        let _ = self.tx.send(Cmd::Respond {
            id: request_id,
            result: Ok(outcome),
        });
    }
}

fn describe_ws_error(e: &tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error as WsError;
    match e {
        WsError::Http(resp) => match resp.status().as_u16() {
            401 | 403 => "authentication failed — check the secret key".to_string(),
            code => format!("server rejected the connection (HTTP {code})"),
        },
        _ => e.to_string(),
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// How many pings may go unanswered before the connection counts as dead. A
/// phone that moves between networks — or a NAT/VPN gateway that drops an idle
/// mapping — leaves a half-open socket: sends still "succeed" into a dead
/// connection. Any inbound frame proves liveness; silence across two ping
/// intervals means the connection is gone.
const MAX_MISSED_PONGS: u32 = 2;

/// Whether the actor keeps looping, or stops with the reason it ended.
enum Step {
    Continue,
    Stop(String),
}

async fn actor(
    socket: Socket,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    event_tx: mpsc::Sender<AcpEvent>,
) {
    let (mut sink, mut stream) = socket.split();
    let mut pending: HashMap<u64, oneshot::Sender<Result<Value, AcpError>>> = HashMap::new();
    let mut next_id: u64 = 1;
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    keepalive.reset(); // don't ping immediately

    // Pings we have sent with nothing heard back since.
    let mut unanswered_pings: u32 = 0;

    let reason = loop {
        let step = tokio::select! {
            cmd = cmd_rx.recv() => {
                handle_command(cmd, &mut sink, &mut pending, &mut next_id).await
            }
            msg = stream.next() => {
                // Anything received — data, pong, even a ping — proves the
                // peer is still there.
                unanswered_pings = 0;
                handle_frame(msg, &mut pending, &event_tx, &mut sink).await
            }
            _ = keepalive.tick() => {
                if unanswered_pings >= MAX_MISSED_PONGS {
                    Step::Stop("server stopped responding (no pong)".to_string())
                } else if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    Step::Stop("keepalive failed".to_string())
                } else {
                    unanswered_pings += 1;
                    Step::Continue
                }
            }
        };
        if let Step::Stop(reason) = step {
            break reason;
        }
    };

    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(AcpError::Closed));
    }
    let _ = event_tx.send(AcpEvent::Disconnected { reason }).await;
}

/// Write one JSON-RPC frame; a failed send means the socket is already gone.
async fn send_frame<S>(sink: &mut S, frame: &Value) -> Step
where
    S: futures_util::Sink<Message> + Unpin,
{
    if sink
        .send(Message::Text(frame.to_string().into()))
        .await
        .is_err()
    {
        Step::Stop("send failed".to_string())
    } else {
        Step::Continue
    }
}

/// Handle one queued command. `None` means every [`AcpClient`] handle was
/// dropped, which closes the connection just like [`AcpClient::close`].
async fn handle_command<S>(
    cmd: Option<Cmd>,
    sink: &mut S,
    pending: &mut HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>,
    next_id: &mut u64,
) -> Step
where
    S: futures_util::Sink<Message> + Unpin,
{
    match cmd {
        None | Some(Cmd::Close) => {
            let _ = sink.send(Message::Close(None)).await;
            Step::Stop("closed by client".to_string())
        }
        Some(Cmd::Request {
            method,
            params,
            reply,
        }) => {
            let id = *next_id;
            *next_id += 1;
            let frame = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            });
            match send_frame(sink, &frame).await {
                Step::Continue => {
                    pending.insert(id, reply);
                    Step::Continue
                }
                stop @ Step::Stop(_) => {
                    let _ = reply.send(Err(AcpError::Closed));
                    stop
                }
            }
        }
        Some(Cmd::Notify { method, params }) => {
            let frame = json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            });
            send_frame(sink, &frame).await
        }
        Some(Cmd::Respond { id, result }) => {
            let frame = match result {
                Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
                Err((code, message)) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": code, "message": message},
                }),
            };
            send_frame(sink, &frame).await
        }
    }
}

/// Handle one frame off the socket. `None` is a clean end of stream.
async fn handle_frame<S>(
    msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    pending: &mut HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>,
    event_tx: &mpsc::Sender<AcpEvent>,
    sink: &mut S,
) -> Step
where
    S: futures_util::Sink<Message> + Unpin,
{
    match msg {
        None => Step::Stop("connection closed".to_string()),
        Some(Err(e)) => Step::Stop(e.to_string()),
        Some(Ok(Message::Text(text))) => {
            if let Ok(value) = serde_json::from_str::<Value>(text.as_str()) {
                match value {
                    Value::Array(batch) => {
                        for item in batch {
                            handle_incoming(item, pending, event_tx, sink).await;
                        }
                    }
                    other => handle_incoming(other, pending, event_tx, sink).await,
                }
            }
            Step::Continue
        }
        Some(Ok(Message::Ping(payload))) => {
            let _ = sink.send(Message::Pong(payload)).await;
            Step::Continue
        }
        Some(Ok(Message::Close(frame))) => Step::Stop(frame.map_or_else(
            || "closed by server".to_string(),
            |frame| format!("closed by server: {}", frame.reason),
        )),
        Some(Ok(_)) => Step::Continue,
    }
}

async fn handle_incoming<S>(
    value: Value,
    pending: &mut HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>,
    event_tx: &mpsc::Sender<AcpEvent>,
    sink: &mut S,
) where
    S: futures_util::Sink<Message> + Unpin,
{
    let method = value.get("method").and_then(Value::as_str);
    let id = value.get("id").cloned();

    match (method, id) {
        // Response to one of our requests.
        (None, Some(id)) => complete_request(&value, &id, pending),
        // Request from the agent — must be answered.
        (Some(method), Some(id)) => answer_agent_request(method, id, &value, event_tx, sink).await,
        // Notification from the agent.
        (Some(method), None) => forward_notification(method, &value, event_tx).await,
        (None, None) => {}
    }
}

/// Resolve the caller waiting on this JSON-RPC id, if it is still waiting.
fn complete_request(
    value: &Value,
    id: &Value,
    pending: &mut HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>,
) {
    let Some(id) = id.as_u64() else { return };
    let Some(reply) = pending.remove(&id) else {
        return;
    };
    let result = if let Some(error) = value.get("error") {
        Err(AcpError::Rpc {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
            data: error.get("data").cloned(),
        })
    } else {
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    };
    let _ = reply.send(result);
}

fn session_id_of(params: &Value) -> String {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Answer an agent-initiated request. Permission prompts go to the UI; we
/// advertise no fs/terminal/elicitation capabilities, so anything else is out
/// of contract and is refused politely.
async fn answer_agent_request<S>(
    method: &str,
    id: Value,
    value: &Value,
    event_tx: &mpsc::Sender<AcpEvent>,
    sink: &mut S,
) where
    S: futures_util::Sink<Message> + Unpin,
{
    if method == "session/request_permission" {
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let session_id = session_id_of(&params);
        let tool_call = params
            .get("toolCall")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();
        let options = params
            .get("options")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();
        let _ = event_tx
            .send(AcpEvent::Permission(PermissionRequest {
                request_id: id,
                session_id,
                tool_call,
                options,
            }))
            .await;
    } else {
        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("method not supported by this client: {method}")},
        });
        let _ = sink.send(Message::Text(frame.to_string().into())).await;
    }
}

/// Turn an agent notification into an [`AcpEvent`]. Unknown methods are
/// ignored, as JSON-RPC requires.
async fn forward_notification(method: &str, value: &Value, event_tx: &mpsc::Sender<AcpEvent>) {
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "session/update" => {
            let session_id = session_id_of(&params);
            let update = params.get("update").cloned().unwrap_or(Value::Null);
            let _ = event_tx
                .send(AcpEvent::Update {
                    session_id,
                    update: SessionUpdate::from_value(update),
                })
                .await;
        }
        "_goose/unstable/session/update" => {
            let session_id = session_id_of(&params);
            let update = params.get("update").cloned().unwrap_or(Value::Null);
            let _ = event_tx
                .send(AcpEvent::GooseUpdate { session_id, update })
                .await;
        }
        "$/cancel_request" => {
            if let Some(request_id) = params.get("requestId").cloned() {
                let _ = event_tx
                    .send(AcpEvent::RequestCancelled { request_id })
                    .await;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_urls() {
        assert_eq!(
            normalize_base_url("goose-box.tailnet.ts.net").unwrap(),
            "http://goose-box.tailnet.ts.net"
        );
        assert_eq!(
            normalize_base_url("https://goose-box.tailnet.ts.net/").unwrap(),
            "https://goose-box.tailnet.ts.net"
        );
        assert_eq!(
            normalize_base_url("wss://host:3284/acp").unwrap(),
            "https://host:3284"
        );
        assert_eq!(
            normalize_base_url(" http://100.101.102.103:3284 ").unwrap(),
            "http://100.101.102.103:3284"
        );
        assert!(normalize_base_url("").is_err());
        assert!(normalize_base_url("ftp://x").is_err());
    }

    fn rpc(message: &str, data: Option<Value>) -> AcpError {
        AcpError::Rpc {
            code: -32603,
            message: message.to_string(),
            data,
        }
    }

    /// goose puts the reason in `data` and leaves `message` as the canned
    /// JSON-RPC text, so rendering `message` shows every failure as "Internal
    /// error". The reason has to win.
    #[test]
    fn an_rpc_error_prefers_the_reason_in_data() {
        assert_eq!(
            rpc(
                "Internal error",
                Some(json!("failed to start extension `mail-imap`: missing env")),
            )
            .to_string(),
            "failed to start extension `mail-imap`: missing env"
        );
        assert_eq!(
            rpc(
                "Invalid params",
                Some(json!("Extension 'mail-imap' not found")),
            )
            .to_string(),
            "Extension 'mail-imap' not found"
        );
    }

    /// Nothing usable in `data` — absent, blank, or not a string at all —
    /// falls back to `message` rather than to an empty error.
    #[test]
    fn an_rpc_error_falls_back_to_message() {
        assert_eq!(rpc("Internal error", None).to_string(), "Internal error");
        assert_eq!(
            rpc("Internal error", Some(json!("   "))).to_string(),
            "Internal error"
        );
        assert_eq!(
            rpc("Internal error", Some(json!({"reason": "structured"}))).to_string(),
            "Internal error"
        );
    }

    #[test]
    fn builds_ws_urls() {
        assert_eq!(
            ws_url("https://goose.ts.net").unwrap(),
            "wss://goose.ts.net/acp"
        );
        assert_eq!(ws_url("host:3284").unwrap(), "ws://host:3284/acp");
    }
}
