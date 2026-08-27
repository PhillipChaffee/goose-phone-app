//! Async client for the goose ACP WebSocket endpoint.
//!
//! One background task owns the socket. Requests are correlated by JSON-RPC
//! id; notifications and agent-initiated requests are surfaced through an
//! [`AcpEvent`] channel. `session/prompt` stays pending for the whole agent
//! turn, so requests carry no default timeout — callers opt in per call.
//!
//! This file is the transport and nothing else: the connection handshake,
//! `request`/`notify`/`respond`, and the frame loop. The per-method wrappers
//! that build params and type replies live in [`session`] and, for goose's own
//! namespace, in [`crate::goose`] — separate `impl AcpClient` blocks in
//! separate files, which Rust allows within one crate.

mod session;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use tokio_tungstenite::{connect_async_tls_with_config, MaybeTlsStream, WebSocketStream};

use crate::error::AcpError;
use crate::tls;
use crate::types::{AcpEvent, DisconnectCause, InitializeInfo, PermissionRequest, SessionUpdate};

pub const CLIENT_NAME: &str = "goose-mobile";

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
    /// Methods this server answered with `-32601`, so the next call can fail
    /// without a round trip. Shared behind an `Arc` so every clone of the
    /// handle learns from every other one, and so cloning stays cheap; a
    /// fresh [`AcpClient::connect`] starts empty, which is what we want —
    /// the user may have just restarted goose with the flag that switches the
    /// feature on.
    ///
    /// Keyed by method, deliberately not by feature. `session/rename` has
    /// shipped far longer than `session/share/nostr`, so one absent method
    /// under a per-feature key would darken a whole screen that mostly works.
    pub(crate) unsupported: Arc<Mutex<HashSet<&'static str>>>,
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

        let client = Self {
            tx: cmd_tx,
            unsupported: Arc::default(),
        };
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
    Stop(DisconnectCause, String),
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

    let (cause, reason) = loop {
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
                    Step::Stop(
                        DisconnectCause::Transport,
                        "server stopped responding (no pong)".to_string(),
                    )
                } else if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    Step::Stop(DisconnectCause::Transport, "keepalive failed".to_string())
                } else {
                    unanswered_pings += 1;
                    Step::Continue
                }
            }
        };
        if let Step::Stop(cause, reason) = step {
            break (cause, reason);
        }
    };

    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(AcpError::Closed));
    }
    let _ = event_tx
        .send(AcpEvent::Disconnected { reason, cause })
        .await;
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
        Step::Stop(DisconnectCause::Transport, "send failed".to_string())
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
        // The only local stop there is. `None` means the last [`AcpClient`]
        // handle was dropped, which is app teardown and just as deliberate.
        // Everything else that ends this loop is the transport, and telling
        // the two apart is the whole reason [`DisconnectCause`] exists.
        None | Some(Cmd::Close) => {
            let _ = sink.send(Message::Close(None)).await;
            Step::Stop(DisconnectCause::Local, "closed by client".to_string())
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
                stop @ Step::Stop(..) => {
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
        None => Step::Stop(DisconnectCause::Transport, "connection closed".to_string()),
        Some(Err(e)) => Step::Stop(DisconnectCause::Transport, e.to_string()),
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
        // A Close frame FROM the server is still the transport ending under
        // us, however politely: nothing on this side asked for it.
        Some(Ok(Message::Close(frame))) => Step::Stop(
            DisconnectCause::Transport,
            frame.map_or_else(
                || "closed by server".to_string(),
                |frame| format!("closed by server: {}", frame.reason),
            ),
        ),
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

    #[test]
    fn builds_ws_urls() {
        assert_eq!(
            ws_url("https://goose.ts.net").unwrap(),
            "wss://goose.ts.net/acp"
        );
        assert_eq!(ws_url("host:3284").unwrap(), "ws://host:3284/acp");
    }
}
