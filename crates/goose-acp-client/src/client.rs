//! Async client for the goose ACP WebSocket endpoint.
//!
//! One background task owns the socket. Requests are correlated by JSON-RPC
//! id; notifications and agent-initiated requests are surfaced through an
//! [`AcpEvent`] channel. `session/prompt` stays pending for the whole agent
//! turn, so requests carry no default timeout — callers opt in per call.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_tls_with_config, MaybeTlsStream, WebSocketStream};

use crate::tls;
use crate::types::*;

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
    #[error("{message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("invalid configuration: {0}")]
    Config(String),
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
    /// (`GOOSED_CERT_FINGERPRINT`). `None` = normal WebPKI validation.
    pub fingerprint: Option<[u8; 32]>,
}

/// Normalize user input to an `http(s)://host[:port]` base with no trailing
/// slash or path.
pub fn normalize_base_url(input: &str) -> Result<String, AcpError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AcpError::Config("server URL is empty".into()));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let (scheme, rest) = with_scheme.split_once("://").unwrap();
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
    Ok(format!("{scheme}://{host}"))
}

/// Derive the ACP WebSocket URL from a normalized base URL.
pub fn ws_url(base_url: &str) -> Result<String, AcpError> {
    let base = normalize_base_url(base_url)?;
    Ok(if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}/acp")
    } else {
        format!("ws://{}/acp", base.strip_prefix("http://").unwrap())
    })
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
#[derive(Clone)]
pub struct AcpClient {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl AcpClient {
    /// Connect, perform the ACP `initialize` handshake, and return the handle
    /// together with the event stream and the agent's identity.
    pub async fn connect(
        cfg: &ConnectConfig,
    ) -> Result<(AcpClient, mpsc::Receiver<AcpEvent>, InitializeInfo), AcpError> {
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
            .map_err(|e| AcpError::Connect(describe_ws_error(e)))?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel(1024);
        tokio::spawn(actor(socket, cmd_rx, event_tx));

        let client = AcpClient { tx: cmd_tx };
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

    pub async fn session_delete(&self, session_id: &str) -> Result<(), AcpError> {
        self.request_with_timeout(
            "session/delete",
            json!({"sessionId": session_id}),
            Duration::from_secs(30),
        )
        .await
        .map(|_| ())
    }

    pub async fn session_rename(&self, session_id: &str, title: &str) -> Result<(), AcpError> {
        self.request_with_timeout(
            "_goose/unstable/session/rename",
            json!({"sessionId": session_id, "title": title}),
            Duration::from_secs(30),
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

fn describe_ws_error(e: tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error as WsError;
    match &e {
        WsError::Http(resp) => match resp.status().as_u16() {
            401 | 403 => "authentication failed — check the secret key".to_string(),
            code => format!("server rejected the connection (HTTP {code})"),
        },
        _ => e.to_string(),
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

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

    let mut reason = "connection closed".to_string();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    None | Some(Cmd::Close) => {
                        let _ = sink.send(Message::Close(None)).await;
                        reason = "closed by client".to_string();
                        break;
                    }
                    Some(Cmd::Request { method, params, reply }) => {
                        let id = next_id;
                        next_id += 1;
                        let frame = json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "method": method,
                            "params": params,
                        });
                        if sink.send(Message::Text(frame.to_string().into())).await.is_err() {
                            let _ = reply.send(Err(AcpError::Closed));
                            reason = "send failed".to_string();
                            break;
                        }
                        pending.insert(id, reply);
                    }
                    Some(Cmd::Notify { method, params }) => {
                        let frame = json!({
                            "jsonrpc": "2.0",
                            "method": method,
                            "params": params,
                        });
                        if sink.send(Message::Text(frame.to_string().into())).await.is_err() {
                            reason = "send failed".to_string();
                            break;
                        }
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
                        if sink.send(Message::Text(frame.to_string().into())).await.is_err() {
                            reason = "send failed".to_string();
                            break;
                        }
                    }
                }
            }
            msg = stream.next() => {
                match msg {
                    None => break,
                    Some(Err(e)) => {
                        reason = e.to_string();
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<Value>(text.as_str()) {
                            match value {
                                Value::Array(batch) => {
                                    for item in batch {
                                        handle_incoming(item, &mut pending, &event_tx, &mut sink).await;
                                    }
                                }
                                other => handle_incoming(other, &mut pending, &event_tx, &mut sink).await,
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        if let Some(frame) = frame {
                            reason = format!("closed by server: {}", frame.reason);
                        } else {
                            reason = "closed by server".to_string();
                        }
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }
            _ = keepalive.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    reason = "keepalive failed".to_string();
                    break;
                }
            }
        }
    }

    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(AcpError::Closed));
    }
    let _ = event_tx.send(AcpEvent::Disconnected { reason }).await;
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
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else { return };
            let Some(reply) = pending.remove(&id) else { return };
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
        // Request from the agent — must be answered.
        (Some(method), Some(id)) => match method {
            "session/request_permission" => {
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
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
            }
            // We advertise no fs/terminal/elicitation capabilities, so
            // anything else is out of contract — refuse politely.
            other => {
                let frame = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("method not supported by this client: {other}")},
                });
                let _ = sink.send(Message::Text(frame.to_string().into())).await;
            }
        },
        // Notification from the agent.
        (Some(method), None) => {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            match method {
                "session/update" => {
                    let session_id = params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let update = params.get("update").cloned().unwrap_or(Value::Null);
                    let _ = event_tx
                        .send(AcpEvent::Update {
                            session_id,
                            update: SessionUpdate::from_value(update),
                        })
                        .await;
                }
                "_goose/unstable/session/update" => {
                    let session_id = params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
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
        (None, None) => {}
    }
}

#[cfg(test)]
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

    #[test]
    fn builds_ws_urls() {
        assert_eq!(
            ws_url("https://goose.ts.net").unwrap(),
            "wss://goose.ts.net/acp"
        );
        assert_eq!(ws_url("host:3284").unwrap(), "ws://host:3284/acp");
    }
}
