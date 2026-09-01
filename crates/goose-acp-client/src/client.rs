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
///
/// Whitespace is trimmed and trailing slashes are not, deliberately: taking
/// them off ahead of the scheme split turned a half-typed `http://` into the
/// bare word `http:`, which then read as a *host* by that name and came back
/// as `http://http:`. The path split below already drops a trailing slash on
/// any input that has a host, so the earlier trim bought nothing and cost the
/// one input it was most likely to meet.
fn split_base_url(input: &str) -> Result<(&'static str, &str), AcpError> {
    let trimmed = input.trim();
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
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the check"
)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use futures_util::Sink;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::Error as WsError;

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

    /// A scheme with nothing after it is a half-typed address. It has to be
    /// refused, and named as the thing it is: the Connect screen shows this
    /// error verbatim, and every input below was until now accepted as a
    /// *host* — `http://` came back as `http://http:` — so the reader was told
    /// their server could not be reached rather than that they had not
    /// finished typing its name.
    #[test]
    fn a_url_with_a_scheme_and_no_host_is_refused() {
        for input in ["http://", "https://", "https:///acp", "ws://?key=x", "/"] {
            let err = normalize_base_url(input).unwrap_err();
            assert!(
                err.to_string().contains("no host"),
                "`{input}` should be refused for having no host, got: {err}"
            );
        }
        assert!(ws_url("wss://").is_err());
    }

    /// The trailing slash a browser bar adds still comes off, which is the
    /// whole job the trim that caused the bug above was doing.
    #[test]
    fn a_trailing_slash_is_still_dropped_from_a_real_url() {
        assert_eq!(
            normalize_base_url("https://goose-box.tailnet.ts.net/").unwrap(),
            "https://goose-box.tailnet.ts.net"
        );
        assert_eq!(
            normalize_base_url("host:3284/").unwrap(),
            "http://host:3284"
        );
        assert_eq!(
            ws_url("https://goose.ts.net/").unwrap(),
            "wss://goose.ts.net/acp"
        );
    }

    /// Two ways to be unusable before a socket is ever opened. Both used to
    /// reach the user as the twenty-second connect timeout, which says nothing
    /// about the one thing they could have fixed.
    #[tokio::test]
    async fn connect_refuses_an_unusable_config_without_dialling() {
        let blank = AcpClient::connect(&ConnectConfig {
            base_url: "   ".to_string(),
            secret: "s".to_string(),
            fingerprint: None,
        })
        .await
        .unwrap_err();
        assert!(
            matches!(&blank, AcpError::Config(m) if m.contains("empty")),
            "an empty server URL should be a config error, got: {blank:?}"
        );

        // A secret with a newline in it cannot go in an HTTP header. Pasting
        // one out of a terminal is exactly how that happens, and the port here
        // is never dialled — the check is ahead of the socket.
        let pasted = AcpClient::connect(&ConnectConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            secret: "secret\nkey".to_string(),
            fingerprint: None,
        })
        .await
        .unwrap_err();
        assert!(
            matches!(&pasted, AcpError::Config(m) if m.contains("invalid characters")),
            "a secret with a newline should be a config error, got: {pasted:?}"
        );
    }

    /// The connect failure a user is most likely to hit is the one they can
    /// act on, and "HTTP error: 401" is not that sentence.
    #[test]
    fn a_refused_upgrade_is_described_in_the_reader_s_terms() {
        let http = |code: u16| {
            WsError::Http(Box::new(
                tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(code)
                    .body(None)
                    .unwrap(),
            ))
        };
        for code in [401, 403] {
            assert_eq!(
                describe_ws_error(&http(code)),
                "authentication failed — check the secret key",
                "HTTP {code} is the secret being rejected"
            );
        }
        assert_eq!(
            describe_ws_error(&http(502)),
            "server rejected the connection (HTTP 502)",
            "a refusal that is not about auth must not blame the secret"
        );
        // Anything that is not an HTTP refusal keeps tungstenite's own words:
        // "connection refused" is more use to a reader than a sentence of ours.
        assert_eq!(
            describe_ws_error(&WsError::ConnectionClosed),
            "Connection closed normally"
        );
    }

    // ---- the frame layer ---------------------------------------------------
    //
    // `handle_command`, `handle_frame` and `handle_incoming` are generic over
    // their sink, so the whole JSON-RPC envelope — the ids, the batch, the
    // pong, the refusal owed to a request this client cannot serve — is
    // checkable against a recording sink with no socket anywhere.

    /// A `Sink<Message>` that keeps what was written to it, and that can be
    /// told to fail every send the way a socket does once the far end is gone.
    #[derive(Debug, Default)]
    struct Wire {
        sent: Vec<Message>,
        broken: bool,
    }

    /// The sink's error type. Its value is never read — [`send_frame`] asks
    /// only *whether* the send failed — so it carries nothing.
    #[derive(Debug)]
    struct SinkDead;

    impl Wire {
        const fn broken() -> Self {
            Self {
                sent: Vec::new(),
                broken: true,
            }
        }

        /// The JSON frames written, in order.
        fn frames(&self) -> Vec<Value> {
            self.sent
                .iter()
                .map(|msg| match msg {
                    Message::Text(text) => serde_json::from_str(text.as_str()).unwrap(),
                    other => panic!("expected a text frame, got {other:?}"),
                })
                .collect()
        }

        fn one_frame(&self) -> Value {
            let mut frames = self.frames();
            assert_eq!(frames.len(), 1, "expected exactly one frame: {frames:?}");
            frames.remove(0)
        }
    }

    impl Sink<Message> for Wire {
        type Error = SinkDead;

        fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), SinkDead>> {
            Poll::Ready(if self.broken { Err(SinkDead) } else { Ok(()) })
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), SinkDead> {
            self.get_mut().sent.push(item);
            Ok(())
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), SinkDead>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), SinkDead>> {
            Poll::Ready(Ok(()))
        }
    }

    fn stopped(step: Step) -> (DisconnectCause, String) {
        match step {
            Step::Stop(cause, reason) => (cause, reason),
            Step::Continue => panic!("expected the connection to end and it did not"),
        }
    }

    fn assert_continues(step: Step) {
        if let Step::Stop(cause, reason) = step {
            panic!("expected the connection to survive; it ended: {cause:?} — {reason}");
        }
    }

    /// Feed one incoming message to the dispatcher and hand back the event it
    /// surfaced, if any.
    async fn incoming(value: Value) -> Option<AcpEvent> {
        let (event_tx, mut event_rx) = mpsc::channel(4);
        handle_incoming(value, &mut HashMap::new(), &event_tx, &mut Wire::default()).await;
        event_rx.try_recv().ok()
    }

    /// Every request goes out as a numbered JSON-RPC 2.0 envelope, and the
    /// number is the only thing a reply is matched on. Two requests sharing an
    /// id, or a reply routed to the wrong one, is a caller that waits for ever
    /// while another is handed an answer meant for somebody else.
    #[tokio::test]
    async fn requests_are_numbered_envelopes_and_the_number_routes_the_reply() {
        let mut wire = Wire::default();
        let mut pending = HashMap::new();
        let mut next_id = 1;

        let (first_tx, mut first_rx) = oneshot::channel();
        assert_continues(
            handle_command(
                Some(Cmd::Request {
                    method: "session/prompt".to_string(),
                    params: json!({"sessionId": "20260821_1"}),
                    reply: first_tx,
                }),
                &mut wire,
                &mut pending,
                &mut next_id,
            )
            .await,
        );

        let (second_tx, second_rx) = oneshot::channel();
        assert_continues(
            handle_command(
                Some(Cmd::Request {
                    method: "session/delete".to_string(),
                    params: json!({"sessionId": "20260820_1"}),
                    reply: second_tx,
                }),
                &mut wire,
                &mut pending,
                &mut next_id,
            )
            .await,
        );

        assert_eq!(
            wire.frames(),
            vec![
                json!({"jsonrpc": "2.0", "id": 1, "method": "session/prompt",
                       "params": {"sessionId": "20260821_1"}}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "session/delete",
                       "params": {"sessionId": "20260820_1"}}),
            ],
            "the envelope goose validates is version, id, method, params"
        );
        assert_eq!(
            next_id, 3,
            "an id must not be reused while its request is open"
        );
        assert_eq!(pending.len(), 2);

        // The reply to the second request resolves the second caller only.
        let (event_tx, _event_rx) = mpsc::channel(4);
        handle_incoming(
            json!({"jsonrpc": "2.0", "id": 2, "result": {"ok": true}}),
            &mut pending,
            &event_tx,
            &mut wire,
        )
        .await;
        assert_eq!(second_rx.await.unwrap().unwrap(), json!({"ok": true}));
        assert!(
            first_rx.try_recv().is_err(),
            "a reply routed by id resolved the wrong caller"
        );

        // And a reply for an id nothing is waiting on — a duplicate, or one
        // that arrived after its request was abandoned — is dropped without
        // disturbing the request that is still open.
        handle_incoming(
            json!({"jsonrpc": "2.0", "id": 99, "result": {}}),
            &mut pending,
            &event_tx,
            &mut wire,
        )
        .await;
        assert_eq!(
            pending.len(),
            1,
            "the open request must survive a stray reply"
        );
        assert!(first_rx.try_recv().is_err());
    }

    /// A send that fails means the socket is already gone. The caller waiting
    /// on that request has to be told, so the screen can report a failed turn
    /// — and the request must not be recorded as sent, or the reply that never
    /// comes leaves an entry in `pending` for the life of the connection.
    #[tokio::test]
    async fn a_send_that_fails_ends_the_connection_and_fails_the_caller() {
        let mut wire = Wire::broken();
        let mut pending = HashMap::new();
        let (reply_tx, reply_rx) = oneshot::channel();

        let (cause, reason) = stopped(
            handle_command(
                Some(Cmd::Request {
                    method: "session/list".to_string(),
                    params: json!({}),
                    reply: reply_tx,
                }),
                &mut wire,
                &mut pending,
                &mut 1,
            )
            .await,
        );

        assert_eq!(
            cause,
            DisconnectCause::Transport,
            "a dead socket is not a local close"
        );
        assert_eq!(reason, "send failed");
        assert!(
            matches!(reply_rx.await.unwrap(), Err(AcpError::Closed)),
            "the caller must be failed, not left pending for ever"
        );
        assert!(
            pending.is_empty(),
            "a request that never reached the wire must not be waited on"
        );
    }

    /// A notification carries no id. One that did would be answered, and this
    /// client is waiting on no such reply — the agent's bookkeeping would end
    /// the turn one response out.
    #[tokio::test]
    async fn a_notification_goes_out_without_an_id() {
        let mut wire = Wire::default();
        let mut pending = HashMap::new();
        let mut next_id = 1;

        assert_continues(
            handle_command(
                Some(Cmd::Notify {
                    method: "session/cancel".to_string(),
                    params: json!({"sessionId": "20260821_1"}),
                }),
                &mut wire,
                &mut pending,
                &mut next_id,
            )
            .await,
        );

        assert_eq!(
            wire.one_frame(),
            json!({"jsonrpc": "2.0", "method": "session/cancel",
                   "params": {"sessionId": "20260821_1"}})
        );
        assert_eq!(next_id, 1, "a notification must not consume a request id");
        assert!(pending.is_empty(), "nothing is waiting on a notification");
    }

    /// Answering the agent is `result` or `error`, never both: a frame with
    /// the two of them is malformed JSON-RPC, and the agent is blocked on this
    /// reply for the rest of its turn.
    #[tokio::test]
    async fn a_response_carries_either_a_result_or_an_error() {
        let mut wire = Wire::default();
        for (id, result) in [
            (
                json!("perm-1"),
                Ok(json!({"outcome": {"outcome": "cancelled"}})),
            ),
            (json!(7), Err((-32601, "not supported".to_string()))),
        ] {
            assert_continues(
                handle_command(
                    Some(Cmd::Respond { id, result }),
                    &mut wire,
                    &mut HashMap::new(),
                    &mut 1,
                )
                .await,
            );
        }

        assert_eq!(
            wire.frames(),
            vec![
                json!({"jsonrpc": "2.0", "id": "perm-1",
                       "result": {"outcome": {"outcome": "cancelled"}}}),
                json!({"jsonrpc": "2.0", "id": 7,
                       "error": {"code": -32601, "message": "not supported"}}),
            ]
        );
    }

    /// The last handle going away is app teardown, and it has to read as a
    /// *local* close: the app announces a thrown-away turn on a transport
    /// disconnect, and would otherwise announce one every time the client is
    /// dropped.
    #[tokio::test]
    async fn the_last_handle_going_away_closes_politely_and_locally() {
        let mut wire = Wire::default();
        let (cause, reason) =
            stopped(handle_command(None, &mut wire, &mut HashMap::new(), &mut 1).await);

        assert_eq!(cause, DisconnectCause::Local);
        assert_eq!(reason, "closed by client");
        assert!(
            matches!(wire.sent.as_slice(), [Message::Close(None)]),
            "the server is owed a close frame, got: {:?}",
            wire.sent
        );
    }

    /// Every way the socket can end under us reports `Transport`, and a close
    /// frame carries the server's own reason — which is the only explanation
    /// the reader is ever going to get.
    #[tokio::test]
    async fn a_socket_ending_under_us_reports_transport_and_the_reason_given() {
        let ended = |msg| async {
            let (event_tx, _event_rx) = mpsc::channel(4);
            stopped(handle_frame(msg, &mut HashMap::new(), &event_tx, &mut Wire::default()).await)
        };

        assert_eq!(
            ended(None).await,
            (DisconnectCause::Transport, "connection closed".to_string()),
            "an end of stream is the transport, not a local close"
        );
        assert_eq!(
            ended(Some(Ok(Message::Close(None)))).await,
            (DisconnectCause::Transport, "closed by server".to_string())
        );
        assert_eq!(
            ended(Some(Ok(Message::Close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "restarting".into(),
            })))))
            .await,
            (
                DisconnectCause::Transport,
                "closed by server: restarting".to_string()
            ),
            "the server said why; the reader should be told"
        );
    }

    /// A ping must come back as a pong carrying the same payload. Silence is
    /// what a NAT gateway prunes an idle mapping on, and the phone is then
    /// holding a socket that is gone without ever being told.
    #[tokio::test]
    async fn a_ping_is_answered_with_the_same_payload() {
        let mut wire = Wire::default();
        let (event_tx, _event_rx) = mpsc::channel(4);
        assert_continues(
            handle_frame(
                Some(Ok(Message::Ping(vec![7, 8].into()))),
                &mut HashMap::new(),
                &event_tx,
                &mut wire,
            )
            .await,
        );
        match wire.sent.as_slice() {
            [Message::Pong(payload)] => assert_eq!(
                payload.as_ref(),
                [7, 8],
                "a pong with the wrong payload does not answer the ping"
            ),
            other => panic!("expected exactly one pong, got {other:?}"),
        }
    }

    /// Frames this client does not read must not end the connection. A binary
    /// frame, a pong, text that is not JSON, and a JSON-RPC frame with neither
    /// a method nor an id are all things a proxy or a newer server can put on
    /// the wire, and dropping the chat over one would be a phone that
    /// disconnects for no reason the user can see.
    #[tokio::test]
    async fn frames_this_client_cannot_read_are_ignored_rather_than_fatal() {
        for msg in [
            Message::Binary(vec![0, 1].into()),
            Message::Pong(Vec::new().into()),
            Message::Text("not json at all".into()),
            Message::Text(json!({"jsonrpc": "2.0"}).to_string().into()),
        ] {
            let mut wire = Wire::default();
            let (event_tx, mut event_rx) = mpsc::channel(4);
            assert_continues(
                handle_frame(
                    Some(Ok(msg.clone())),
                    &mut HashMap::new(),
                    &event_tx,
                    &mut wire,
                )
                .await,
            );
            assert!(
                wire.sent.is_empty(),
                "nothing should have been written back for {msg:?}"
            );
            assert!(
                event_rx.try_recv().is_err(),
                "nothing should have been surfaced for {msg:?}"
            );
        }
    }

    /// A JSON-RPC batch is one text frame holding several messages, and every
    /// one of them has to be acted on. A client that read only the first would
    /// silently lose updates the moment a server coalesced them — a transcript
    /// missing the middle of an answer, with no error anywhere.
    #[tokio::test]
    async fn a_batch_frame_is_handled_entry_by_entry() {
        let chunk = |text: &str| {
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "s1",
                "update": {"sessionUpdate": "agent_message_chunk", "messageId": "m1",
                           "content": {"type": "text", "text": text}}}})
        };
        let (event_tx, mut event_rx) = mpsc::channel(8);
        assert_continues(
            handle_frame(
                Some(Ok(Message::Text(
                    json!([chunk("one"), chunk("two")]).to_string().into(),
                ))),
                &mut HashMap::new(),
                &event_tx,
                &mut Wire::default(),
            )
            .await,
        );

        let mut texts = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            match event {
                AcpEvent::Update {
                    update: SessionUpdate::AgentMessageChunk(chunk),
                    ..
                } => texts.push(chunk.content.text_repr()),
                other => panic!("expected a message chunk, got {other:?}"),
            }
        }
        assert_eq!(texts, ["one", "two"], "every entry in a batch is a message");
    }

    /// This client advertises no filesystem and no terminal, so a server that
    /// asks for one anyway is answered by return. Ignoring the request would
    /// leave the agent waiting on a reply that never comes — a turn that never
    /// ends, and a chat that looks frozen.
    #[tokio::test]
    async fn an_agent_request_we_cannot_serve_is_refused_by_return() {
        let mut wire = Wire::default();
        let (event_tx, mut event_rx) = mpsc::channel(4);
        handle_incoming(
            json!({"jsonrpc": "2.0", "id": 41, "method": "fs/read_text_file",
                   "params": {"path": "/etc/passwd"}}),
            &mut HashMap::new(),
            &event_tx,
            &mut wire,
        )
        .await;

        assert_eq!(
            wire.one_frame(),
            json!({"jsonrpc": "2.0", "id": 41, "error": {
                "code": -32601,
                "message": "method not supported by this client: fs/read_text_file"}}),
            "the refusal has to name the method, and be addressed to the id that asked"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "an out-of-contract request must not reach the UI"
        );
    }

    /// The three notifications this client acts on, and the shape of each that
    /// decides which event a screen gets. A tag read wrongly here is a
    /// transcript that stops updating with nothing anywhere saying so.
    #[tokio::test]
    async fn the_notifications_this_client_acts_on() {
        match incoming(
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": "s1",
            "update": {"sessionUpdate": "agent_thought_chunk", "messageId": "t1",
                       "content": {"type": "text", "text": "hm"}}}}),
        )
        .await
        {
            Some(AcpEvent::Update {
                session_id,
                update: SessionUpdate::AgentThoughtChunk(chunk),
            }) => {
                assert_eq!(session_id, "s1", "an update belongs to one chat");
                assert_eq!(chunk.content.text_repr(), "hm");
            }
            other => panic!("expected a thought chunk, got {other:?}"),
        }

        // goose's own namespace is passed through raw: the payloads there
        // change between releases, so the app reads them, not this crate.
        match incoming(
            json!({"jsonrpc": "2.0", "method": "_goose/unstable/session/update",
                              "params": {"sessionId": "s2",
                                         "update": {"sessionUpdate": "usage", "totalTokens": 41}}}),
        )
        .await
        {
            Some(AcpEvent::GooseUpdate { session_id, update }) => {
                assert_eq!(session_id, "s2");
                assert_eq!(update["totalTokens"], json!(41));
            }
            other => panic!("expected a goose update, got {other:?}"),
        }

        // A permission prompt the agent gives up on: the sheet has to come
        // down by itself, because nothing the user does will answer it now.
        match incoming(json!({"jsonrpc": "2.0", "method": "$/cancel_request",
                              "params": {"requestId": "perm-1"}}))
        .await
        {
            Some(AcpEvent::RequestCancelled { request_id }) => {
                assert_eq!(request_id, json!("perm-1"));
            }
            other => panic!("expected a cancellation, got {other:?}"),
        }

        // A cancellation naming no request cancels nothing — taking the sheet
        // down on it would dismiss a prompt the agent is still waiting on.
        assert!(
            incoming(json!({"jsonrpc": "2.0", "method": "$/cancel_request", "params": {}}))
                .await
                .is_none()
        );
        // And an unknown notification is ignored, as JSON-RPC requires.
        assert!(
            incoming(json!({"jsonrpc": "2.0", "method": "session/telemetry", "params": {}}))
                .await
                .is_none()
        );
    }

    /// A handle over a channel the test drains itself. Every call below puts
    /// one [`Cmd`] on it, which is the frame the server would have seen.
    fn detached() -> (AcpClient, mpsc::UnboundedReceiver<Cmd>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            AcpClient {
                tx,
                unsupported: Arc::default(),
            },
            rx,
        )
    }

    /// `notify` is fire-and-forget by design: `session/cancel` goes out while
    /// the request it is cancelling is still holding the socket, so a version
    /// of it that waited for a reply would be waiting on the very turn it is
    /// trying to stop.
    #[test]
    fn notify_queues_a_frame_and_returns_without_waiting() {
        let (client, mut rx) = detached();
        client.notify("session/cancel", json!({"sessionId": "20260821_1"}));
        match rx.try_recv() {
            Ok(Cmd::Notify { method, params }) => {
                assert_eq!(method, "session/cancel");
                assert_eq!(params, json!({"sessionId": "20260821_1"}));
            }
            _ => panic!("notify should have queued a notification"),
        }
    }
}
