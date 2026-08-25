//! End-to-end tests against a stub server that speaks the same wire protocol
//! as `goose serve`: the auth surface (`/status`, `/acp` 401/406) and ACP
//! JSON-RPC over a WebSocket.

// Test code: a failing unwrap, or a panic on the wrong variant, IS the failing
// check. Both are denied for shipped code. `expect` rather than `allow`: if a
// use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test harness: an unwrap or a wrong-variant panic is the assertion"
)]

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use goose_acp_client::{
    assert_round_trip, probe, AcpClient, AcpError, AcpEvent, ConfigExtensions, ConnectConfig,
    ContentBlock, Feature, GooseExtension, McpServer, ProbeOutcome, SessionKind, SessionQuery,
    SessionUpdate, StdioMcpServer,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

const SECRET: &str = "test-secret";

// ---------------------------------------------------------------------------
// Plain-HTTP stub, for the pre-flight probe.

/// Serves `/status` and the unauthenticated/authenticated `/acp` responses.
async fn spawn_http_stub() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                    match sock.read(&mut byte).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => head.push(byte[0]),
                    }
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let path = text
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let authed = text
                    .lines()
                    .any(|l| l.to_lowercase().starts_with("x-secret-key:") && l.contains(SECRET));

                let (code, body) = match (path.as_str(), authed) {
                    ("/status", _) => ("200 OK", "ok"),
                    ("/acp", true) => ("406 Not Acceptable", ""),
                    ("/acp", false) => ("401 Unauthorized", ""),
                    _ => ("404 Not Found", ""),
                };
                let resp = format!(
                    "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    addr
}

// ---------------------------------------------------------------------------
// WebSocket stub speaking ACP JSON-RPC.

/// What the stub should do once a session is prompted.
#[derive(Clone, Copy, PartialEq)]
enum PromptBehavior {
    /// Stream a thought, a tool call, and assistant text, then end the turn.
    Stream,
    /// Ask permission before finishing; the answer decides the tool status.
    AskPermission,
    /// Drop the connection mid-turn.
    Disconnect,
    /// Refuse `_goose/unstable/session/rename` the way a goose server that
    /// has the feature switched off does: `-32601` with the explanation in
    /// `data`, not in `message`.
    RenameUnsupported,
    /// Send the `prompt` array straight back as assistant text. The outbound
    /// frame is otherwise unobservable from a test, and the whole point of an
    /// attachment is what it puts on the wire.
    EchoPrompt,
}

async fn spawn_ws_stub(behavior: PromptBehavior) -> SocketAddr {
    spawn_counting_ws_stub(behavior).await.0
}

/// [`spawn_ws_stub`] plus the count of `session/rename` frames that actually
/// reached the socket, which is how a test proves a call was short-circuited
/// rather than merely answered the same way twice.
async fn spawn_counting_ws_stub(behavior: PromptBehavior) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let renames = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&renames);
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(handle_ws(sock, behavior, Arc::clone(&counter)));
        }
    });
    (addr, renames)
}

type WsStream = tokio_tungstenite::WebSocketStream<TcpStream>;
type WsTx = futures_util::stream::SplitSink<WsStream, Message>;
type WsRx = futures_util::stream::SplitStream<WsStream>;

/// Reject the upgrade unless the shared secret is present, exactly as the real
/// server does.
#[expect(
    clippy::result_large_err,
    reason = "error type is fixed by `accept_hdr_async`'s callback signature"
)]
fn check_secret(req: &Request, resp: Response) -> Result<Response, ErrorResponse> {
    let authed = req
        .headers()
        .get("X-Secret-Key")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == SECRET)
        || req.uri().query().unwrap_or("").contains(SECRET);
    if req.uri().path() == "/acp" && authed {
        Ok(resp)
    } else {
        let mut err = ErrorResponse::new(None);
        *err.status_mut() = StatusCode::UNAUTHORIZED;
        Err(err)
    }
}

async fn send(tx: &mut WsTx, frame: Value) {
    let _ = tx.send(Message::Text(frame.to_string().into())).await;
}

/// The canned JSON-RPC text goose leaves in `message`.
///
/// This is not decoration. goose builds every error as
/// `Error::internal_error().data(reason)` / `Error::invalid_params().data(..)`,
/// so `message` never says anything specific and `data` says everything. A
/// stub that put the reason in `message` would be testing a shape goose does
/// not send, and would pass whether or not the client reads `data`.
const fn canned(code: i64) -> &'static str {
    match code {
        -32700 => "Parse error",
        -32600 => "Invalid Request",
        -32601 => "Method not found",
        -32602 => "Invalid params",
        -32002 => "Session not found",
        _ => "Internal error",
    }
}

/// A JSON-RPC error frame in goose's shape: canned `message`, reason in
/// `data`.
fn error_frame(id: Option<&Value>, code: i64, reason: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": canned(code), "data": reason},
    })
}

/// Send a `session/update` notification carrying `update`.
async fn notify(tx: &mut WsTx, session_id: &str, update: Value) {
    send(
        tx,
        json!({"jsonrpc":"2.0","method":"session/update",
               "params":{"sessionId": session_id, "update": update}}),
    )
    .await;
}

async fn handle_ws(sock: TcpStream, behavior: PromptBehavior, renames: Arc<AtomicUsize>) {
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(sock, check_secret).await else {
        return;
    };
    let (mut tx, mut rx) = ws.split();

    while let Some(Ok(msg)) = rx.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let id = frame.get("id").cloned();
        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");

        match method {
            "initialize" => {
                send(
                    &mut tx,
                    json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentInfo": {"name": "stub-goose", "version": "1.47.0"}
                        }
                    }),
                )
                .await;
            }
            "session/new" => {
                // Mirror the server's absolute-path requirement.
                let cwd = frame
                    .pointer("/params/cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let out = if cwd.starts_with('/') {
                    json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":"20260821_1"}})
                } else {
                    json!({"jsonrpc":"2.0","id":id,
                           "error":{"code":-32602,"message":"cwd must be absolute"}})
                };
                send(&mut tx, out).await;
            }
            "session/list" => {
                // The stub holds one session and it is a `user` one, so the
                // filter the client sent decides whether it comes back — the
                // same way the real server's `_meta.types` does.
                let wants_user = frame
                    .pointer("/params/_meta/types")
                    .and_then(Value::as_array)
                    .is_some_and(|types| types.iter().any(|t| t == "user"));
                if !wants_user {
                    send(
                        &mut tx,
                        json!({"jsonrpc":"2.0","id":id,"result":{"sessions":[],"nextCursor":null}}),
                    )
                    .await;
                    continue;
                }
                send(
                    &mut tx,
                    json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {"sessions": [{
                            "sessionId": "20260820_1",
                            "cwd": "/home/demo",
                            "title": "Earlier chat",
                            "updatedAt": "2026-08-20T10:00:00Z",
                            "_meta": {"messageCount": 4, "lastMessageSnippet": "all done"}
                        }], "nextCursor": null}
                    }),
                )
                .await;
            }
            "session/prompt" => {
                if !serve_prompt(&mut tx, &mut rx, &frame, id, behavior).await {
                    return; // drop the socket mid-turn
                }
            }
            "session/delete" => {
                send(&mut tx, json!({"jsonrpc":"2.0","id":id,"result":{}})).await;
            }
            "_goose/unstable/session/rename" => {
                renames.fetch_add(1, Ordering::SeqCst);
                let out = if behavior == PromptBehavior::RenameUnsupported {
                    // goose puts the sentence in `data`; `message` is the
                    // canned JSON-RPC text.
                    json!({"jsonrpc":"2.0","id":id,
                           "error":{"code":-32601,"message":"Method not found",
                                    "data":"Session renaming is not enabled"}})
                } else {
                    json!({"jsonrpc":"2.0","id":id,"result":{}})
                };
                send(&mut tx, out).await;
            }
            "unknown/method" => {
                send(
                    &mut tx,
                    json!({"jsonrpc":"2.0","id":id,
                        "error":{"code":-32601,"message":"method not found"}}),
                )
                .await;
            }
            _ => {}
        }
    }
}

/// Stream one turn — thought, tool call, assistant text — and answer the
/// pending `session/prompt` request. Returns `false` when the stub should drop
/// the connection mid-turn instead.
async fn serve_prompt(
    tx: &mut WsTx,
    rx: &mut WsRx,
    frame: &Value,
    id: Option<Value>,
    behavior: PromptBehavior,
) -> bool {
    let sid = frame
        .pointer("/params/sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if behavior == PromptBehavior::Disconnect {
        return false;
    }

    if behavior == PromptBehavior::EchoPrompt {
        echo_prompt(tx, &sid, frame, id).await;
        return true;
    }

    notify(
        tx,
        &sid,
        json!({
            "sessionUpdate": "agent_thought_chunk",
            "messageId": "th1",
            "content": {"type": "text", "text": "thinking"}
        }),
    )
    .await;
    notify(
        tx,
        &sid,
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tc1",
            "title": "shell: ls",
            "kind": "execute",
            "status": "pending",
            "_meta": {"goose": {"toolCall": {"toolName": "developer__shell"}}}
        }),
    )
    .await;

    let allowed = if behavior == PromptBehavior::AskPermission {
        ask_permission(tx, rx, &sid).await
    } else {
        true
    };

    notify(
        tx,
        &sid,
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "tc1",
            "status": if allowed { "completed" } else { "failed" },
            "content": [{"type": "content",
                         "content": {"type": "text", "text": "file_a"}}]
        }),
    )
    .await;
    notify(
        tx,
        &sid,
        json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": "m1",
            "content": {"type": "text", "text": "Hello "}
        }),
    )
    .await;
    notify(
        tx,
        &sid,
        json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": "m1",
            "content": {"type": "text", "text": "world"}
        }),
    )
    .await;
    send(
        tx,
        json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"stopReason": "end_turn"}
        }),
    )
    .await;
    true
}

/// Answer a turn by handing the `prompt` array straight back as assistant
/// text. Nothing else can observe the frame the client sent.
async fn echo_prompt(tx: &mut WsTx, session_id: &str, frame: &Value, id: Option<Value>) {
    let prompt = frame
        .pointer("/params/prompt")
        .cloned()
        .unwrap_or(Value::Null);
    notify(
        tx,
        session_id,
        json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": "echo",
            "content": {"type": "text", "text": prompt.to_string()}
        }),
    )
    .await;
    send(
        tx,
        json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "end_turn"}}),
    )
    .await;
}

/// Ask the client for tool permission and wait for its answer. A client that
/// goes away without answering counts as allowing, as in the `Stream` case.
async fn ask_permission(tx: &mut WsTx, rx: &mut WsRx, session_id: &str) -> bool {
    send(
        tx,
        json!({
            "jsonrpc": "2.0", "id": "perm-1",
            "method": "session/request_permission",
            "params": {
                "sessionId": session_id,
                "toolCall": {"toolCallId": "tc1", "title": "shell: ls"},
                "options": [
                    {"optionId": "allow_once", "name": "allow_once", "kind": "allow_once"},
                    {"optionId": "reject_once", "name": "reject_once", "kind": "reject_once"}
                ]
            }
        }),
    )
    .await;
    while let Some(Ok(Message::Text(t))) = rx.next().await {
        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
        if v.get("id").and_then(Value::as_str) == Some("perm-1") {
            return v
                .pointer("/result/outcome/optionId")
                .and_then(Value::as_str)
                == Some("allow_once");
        }
    }
    true
}

// ---------------------------------------------------------------------------
// WebSocket stub speaking the `_goose/unstable/*` extension methods.
//
// Separate from the prompt stub above because it models something different:
// a tiny config store, held per connection, that behaves the way goose's does
// — including the way it misbehaves.

/// How the stub treats the tool allowlist it is sent.
#[derive(Clone, Copy, PartialEq)]
enum ExtBehavior {
    /// Stores `available_tools` as sent, like a correct exchange.
    Faithful,
    /// Accepts the add, answers OK, and stores the extension *without* its
    /// allowlist. This is not a made-up failure: it is exactly what goose does
    /// when the field arrives as camelCase `availableTools` (no
    /// `deny_unknown_fields`, so the key is dropped in silence), and the
    /// resulting extension has every tool allowed. Nothing in the add's reply
    /// distinguishes it from success, which is why the client re-lists.
    DropsAllowlist,
    /// Answers every extension method with `-32601`, the way a goose server
    /// older than the namespace does.
    NoExtensions,
}

async fn spawn_ext_stub(behavior: ExtBehavior) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(handle_ext_ws(sock, behavior));
        }
    });
    addr
}

/// goose's `name_to_key`: lowercase, whitespace dropped, anything outside
/// `[A-Za-z0-9_-]` folded to `_`.
fn name_to_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

/// The extension's name, which for an `mcp` extension lives on the server.
fn ext_name(ext: &Value) -> String {
    ext.pointer("/server/name")
        .or_else(|| ext.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// The stub's per-connection state.
///
/// Secrets are tracked by NAME ONLY — the stub never keeps a credential's
/// value, for the same reason the app never reads one back.
#[derive(Default)]
struct ExtStore {
    configured: Vec<Value>,
    secret_keys: HashSet<String>,
    /// Live session ids. `session/extensions/add` checks membership, so a
    /// handshake against a session that was never created — or has already
    /// been deleted — fails the way the real server fails it.
    sessions: HashSet<String>,
    next_session: u32,
}

async fn handle_ext_ws(sock: TcpStream, behavior: ExtBehavior) {
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(sock, check_secret).await else {
        return;
    };
    let (mut tx, mut rx) = ws.split();

    let mut store = ExtStore::default();

    while let Some(Ok(msg)) = rx.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let id = frame.get("id").cloned();
        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
        let params = frame.get("params").cloned().unwrap_or(Value::Null);

        let outcome = ext_request(method, &params, behavior, &mut store);
        let reply = match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            // The reason goes in `data`, as goose sends it.
            Err((code, reason)) => error_frame(id.as_ref(), code, &reason),
        };
        send(&mut tx, reply).await;
    }
}

/// One request against the stub's config store.
fn ext_request(
    method: &str,
    params: &Value,
    behavior: ExtBehavior,
    store: &mut ExtStore,
) -> Result<Value, (i64, String)> {
    if behavior == ExtBehavior::NoExtensions && method.contains("extensions") {
        return Err((-32601, "Extensions are not available".to_string()));
    }
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": 1,
            "agentInfo": {"name": "stub-goose", "version": "1.46.0"}
        })),

        "_goose/unstable/extensions/available" => Ok(json!({"extensions": [
            // Deliberately allowlist-free, as goose's real catalogue is.
            {"type": "builtin", "name": "developer", "display_name": "Developer",
             "description": "Shell and file editing", "bundled": true},
            {"type": "platform", "name": "memory", "display_name": "Memory", "bundled": true},
        ]})),

        "_goose/unstable/config/extensions/list" => {
            Ok(json!({"extensions": store.configured, "warnings": []}))
        }

        "_goose/unstable/config/extensions/add" => {
            let mut extension = params.get("extension").cloned().unwrap_or(Value::Null);
            let enabled = params
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if behavior == ExtBehavior::DropsAllowlist {
                if let Some(obj) = extension.as_object_mut() {
                    obj.remove("available_tools");
                }
            }
            let name = ext_name(&extension);
            store
                .configured
                .retain(|e| ext_name(&e["extension"]) != name);
            store.configured.push(json!({
                "extension": extension,
                "enabled": enabled,
                "configKey": name_to_key(&name),
            }));
            Ok(json!({}))
        }

        "_goose/unstable/config/extensions/set-enabled" => {
            let key = params
                .get("configKey")
                .and_then(Value::as_str)
                .unwrap_or("");
            let enabled = params
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match store
                .configured
                .iter_mut()
                .find(|e| e["configKey"].as_str() == Some(key))
            {
                Some(entry) => {
                    entry["enabled"] = Value::Bool(enabled);
                    Ok(json!({}))
                }
                None => Err((-32602, format!("Extension '{key}' not found"))),
            }
        }

        "_goose/unstable/config/upsert" => {
            let key = params.get("key").and_then(Value::as_str).unwrap_or("");
            // goose logs "Secret value is not a string; skipping" and
            // carries on, leaving the extension credential-less. The stub
            // reproduces the skip so a client that ever sent a bare number
            // would fail the handshake below rather than pass silently.
            if params.get("value").and_then(Value::as_str).is_some() {
                store.secret_keys.insert(key.to_string());
            }
            Ok(json!({}))
        }

        _ => session_request(method, params, store),
    }
}

/// The session half of the stub: enough of `session/new` and `session/delete`
/// for the credential handshake, which needs a session to run in and makes a
/// throwaway one when no chat is open.
///
/// Session ids are predictable (`ext-1`, `ext-2`, …) so a test can ask
/// afterwards whether a throwaway really was deleted, and
/// `session/extensions/add` checks the id — an unverified credential must not
/// be able to masquerade as a verified one just because the session was made
/// up.
fn session_request(
    method: &str,
    params: &Value,
    store: &mut ExtStore,
) -> Result<Value, (i64, String)> {
    match method {
        "session/new" => {
            let cwd = params.get("cwd").and_then(Value::as_str).unwrap_or("");
            if !cwd.starts_with('/') {
                return Err((-32602, format!("cwd must be absolute, got `{cwd}`")));
            }
            store.next_session += 1;
            let sid = format!("ext-{}", store.next_session);
            store.sessions.insert(sid.clone());
            Ok(json!({"sessionId": sid}))
        }

        "session/delete" => {
            let sid = session_id(params);
            if store.sessions.remove(sid) {
                Ok(json!({}))
            } else {
                Err((-32002, format!("session not found: {sid}")))
            }
        }

        "_goose/unstable/session/extensions/add" => {
            // The handshake. goose launches the MCP server here, and a
            // stdio child whose declared env key has no stored secret is a
            // hard startup failure — so that is what a missing secret does.
            let sid = session_id(params);
            if !store.sessions.contains(sid) {
                return Err((-32002, format!("session not found: {sid}")));
            }
            let extension = params.get("extension").cloned().unwrap_or(Value::Null);
            let missing: Vec<String> = extension
                .get("envKeys")
                .and_then(Value::as_array)
                .map(|keys| {
                    keys.iter()
                        .filter_map(Value::as_str)
                        .filter(|k| !store.secret_keys.contains(*k))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if missing.is_empty() {
                Ok(json!({}))
            } else {
                Err((
                    -32603,
                    format!(
                        "failed to start extension: missing env {}",
                        missing.join(", ")
                    ),
                ))
            }
        }

        other => Err((-32601, format!("method not found: {other}"))),
    }
}

fn session_id(params: &Value) -> &str {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// The connector the extension tests add: a stdio MCP server with a
/// credential and a read-biased allowlist, shaped like the manifests in the
/// brain repo's `config/connectors/`.
fn mail_extension() -> GooseExtension {
    GooseExtension::mcp(
        McpServer::Stdio(StdioMcpServer::new(
            "mail-imap",
            "uvx",
            vec!["mcp-email-server@1.4.2".into(), "stdio".into()],
        )),
        vec!["MCP_EMAIL_SERVER_PASSWORD".into()],
        "IMAP/SMTP mail via a provider app password",
        vec![
            "list_mailboxes".into(),
            "list_emails_metadata".into(),
            "get_emails_content".into(),
        ],
    )
}

fn config(addr: SocketAddr, secret: &str) -> ConnectConfig {
    ConnectConfig {
        base_url: format!("http://{addr}"),
        secret: secret.to_string(),
        fingerprint: None,
    }
}

// ---------------------------------------------------------------------------
// Probe

#[tokio::test]
async fn probe_reports_ok_auth_failure_and_unreachable() {
    let addr = spawn_http_stub().await;
    let base = format!("http://{addr}");

    assert_eq!(probe(&base, SECRET, false).await, ProbeOutcome::Ok);
    assert_eq!(probe(&base, "wrong", false).await, ProbeOutcome::AuthFailed);

    // Nothing listening on this port.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    match probe(&format!("http://{dead_addr}"), SECRET, false).await {
        ProbeOutcome::Unreachable(_) => {}
        other => panic!("expected Unreachable, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Connection lifecycle

#[tokio::test]
async fn connect_performs_handshake_and_reports_agent() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let (client, _events, info) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    assert_eq!(info.agent_name, "stub-goose");
    assert_eq!(info.agent_version, "1.47.0");
    client.close();
}

#[tokio::test]
async fn connect_with_a_bad_secret_is_rejected() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let err = AcpClient::connect(&config(addr, "wrong"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("authentication failed"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn session_new_and_list_round_trip() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    let session = client.session_new("/home/demo").await.unwrap();
    assert_eq!(session.session_id, "20260821_1");

    let page = client
        .session_list(&SessionQuery::new(&[SessionKind::User], None))
        .await
        .unwrap();
    assert_eq!(page.sessions.len(), 1);
    let s = &page.sessions[0];
    assert_eq!(s.display_title(), "Earlier chat");
    assert_eq!(s.message_count(), Some(4));
    assert_eq!(s.last_message_snippet().as_deref(), Some("all done"));

    // The kind filter reaches the wire: the stub's one session is a user
    // session, so asking for scheduled ones only comes back empty.
    let scheduled = client
        .session_list(&SessionQuery::new(&[SessionKind::Scheduled], None))
        .await
        .unwrap();
    assert!(scheduled.sessions.is_empty());

    client.close();
}

// ---------------------------------------------------------------------------
// Feature detection

/// `-32601` is goose's signal that a feature is absent or switched off, and
/// the sentence explaining which is in `data`. Once a method has been refused
/// there is nothing to gain from asking again on the same connection.
#[tokio::test]
async fn an_unsupported_goose_method_is_reported_once_and_then_cached() {
    let (addr, renames) = spawn_counting_ws_stub(PromptBehavior::RenameUnsupported).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    match client.session_rename("20260821_1", "New title").await {
        Err(AcpError::Unsupported {
            feature,
            method,
            reason,
        }) => {
            assert_eq!(feature, Feature::SessionHistory);
            assert_eq!(method, "_goose/unstable/session/rename");
            assert_eq!(reason.as_deref(), Some("Session renaming is not enabled"));
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert_eq!(renames.load(Ordering::SeqCst), 1);

    let err = client
        .session_rename("20260821_1", "Another title")
        .await
        .unwrap_err();
    assert!(err.is_unsupported(), "second call: {err}");
    assert_eq!(
        renames.load(Ordering::SeqCst),
        1,
        "the second call should not have reached the socket"
    );

    client.close();
}

/// The same method on a server that does implement it must still work — the
/// cache is populated by a refusal, never by a successful call.
#[tokio::test]
async fn a_supported_goose_method_is_not_cached_as_missing() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    client
        .session_rename("20260821_1", "New title")
        .await
        .unwrap();
    client
        .session_rename("20260821_1", "Newer title")
        .await
        .unwrap();

    client.close();
}

#[tokio::test]
async fn server_errors_surface_as_rpc_errors() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    // A relative cwd is rejected by the server.
    let err = client.session_new("not/absolute").await.unwrap_err();
    assert!(err.to_string().contains("absolute"), "got: {err}");

    client.close();
}

// ---------------------------------------------------------------------------
// Streaming a turn

#[tokio::test]
async fn prompt_streams_updates_and_ends_the_turn() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let (client, mut events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let session = client.session_new("/home/demo").await.unwrap();

    let c = client.clone();
    let sid = session.session_id.clone();
    let turn = tokio::spawn(async move { c.prompt(&sid, &[ContentBlock::text("hi")]).await });

    let mut text = String::new();
    let mut saw_thought = false;
    let mut tool_status = None;

    // Collect until the turn's updates have all arrived.
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let AcpEvent::Update { update, .. } = event {
            match update {
                SessionUpdate::AgentThoughtChunk(_) => saw_thought = true,
                SessionUpdate::AgentMessageChunk(c) => text.push_str(&c.content.text_repr()),
                SessionUpdate::ToolCall(t) => {
                    assert_eq!(t.tool_name(), Some("developer__shell"));
                    assert_eq!(t.title.as_deref(), Some("shell: ls"));
                }
                SessionUpdate::ToolCallUpdate(t) => {
                    tool_status = t.status.clone();
                    assert_eq!(t.content_text(), "file_a");
                }
                _ => {}
            }
        }
        if text == "Hello world" && tool_status.is_some() {
            break;
        }
    }

    assert_eq!(turn.await.unwrap().unwrap(), "end_turn");
    assert!(saw_thought, "no thought chunk received");
    assert_eq!(text, "Hello world", "assistant chunks should accumulate");
    assert_eq!(tool_status.as_deref(), Some("completed"));

    client.close();
}

/// An attachment is another entry in the `prompt` array, and the array is the
/// only thing the agent ever sees — so what matters is the wire shape, not the
/// Rust type that produced it.
#[tokio::test]
async fn attachments_ride_in_the_prompt_array_beside_the_text() {
    let addr = spawn_ws_stub(PromptBehavior::EchoPrompt).await;
    let (client, mut events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let session = client.session_new("/home/demo").await.unwrap();

    let c = client.clone();
    let sid = session.session_id.clone();
    let turn = tokio::spawn(async move {
        c.prompt(
            &sid,
            &[
                ContentBlock::text("what is in this"),
                ContentBlock::image("QUJD", "image/jpeg"),
                ContentBlock::resource_text("file:///notes.md", "text/markdown", "# notes"),
                ContentBlock::resource_blob("file:///spec.pdf", "application/pdf", "QUJD"),
            ],
        )
        .await
    });

    let mut echoed = String::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let AcpEvent::Update {
            update: SessionUpdate::AgentMessageChunk(c),
            ..
        } = event
        {
            echoed = c.content.text_repr();
            break;
        }
    }
    assert_eq!(turn.await.unwrap().unwrap(), "end_turn");

    let sent: Value = serde_json::from_str(&echoed).unwrap();
    assert_eq!(
        sent,
        json!([
            {"type": "text", "text": "what is in this"},
            {"type": "image", "data": "QUJD", "mimeType": "image/jpeg"},
            {"type": "resource", "resource": {
                "uri": "file:///notes.md", "mimeType": "text/markdown", "text": "# notes"}},
            {"type": "resource", "resource": {
                "uri": "file:///spec.pdf", "mimeType": "application/pdf", "blob": "QUJD"}},
        ]),
        "the prompt array must be ACP's ContentBlock shapes verbatim"
    );

    client.close();
}

/// A turn with nothing in it is refused here rather than at the agent, which
/// answers `invalid_params` and leaves the caller guessing.
#[tokio::test]
async fn an_empty_prompt_is_refused_before_it_is_sent() {
    let addr = spawn_ws_stub(PromptBehavior::EchoPrompt).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let err = client.prompt("20260821_1", &[]).await.unwrap_err();
    assert!(err.to_string().contains("no content"), "got: {err}");
    client.close();
}

#[tokio::test]
async fn permission_request_is_surfaced_and_answered() {
    let addr = spawn_ws_stub(PromptBehavior::AskPermission).await;
    let (client, mut events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let session = client.session_new("/home/demo").await.unwrap();

    let c = client.clone();
    let sid = session.session_id.clone();
    let turn =
        tokio::spawn(async move { c.prompt(&sid, &[ContentBlock::text("run a tool")]).await });

    let mut answered = false;
    let mut tool_status = None;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        match event {
            AcpEvent::Permission(req) => {
                assert_eq!(req.options.len(), 2);
                assert_eq!(req.options[0].option_id, "allow_once");
                client.respond_permission(req.request_id, Some("allow_once".into()));
                answered = true;
            }
            AcpEvent::Update {
                update: SessionUpdate::ToolCallUpdate(t),
                ..
            } => {
                tool_status = t.status;
                break;
            }
            _ => {}
        }
    }

    assert!(answered, "no permission request was surfaced");
    assert_eq!(
        tool_status.as_deref(),
        Some("completed"),
        "allowing should let the tool run"
    );
    assert_eq!(turn.await.unwrap().unwrap(), "end_turn");
    client.close();
}

#[tokio::test]
async fn losing_the_connection_emits_disconnected_and_fails_pending_requests() {
    let addr = spawn_ws_stub(PromptBehavior::Disconnect).await;
    let (client, mut events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let session = client.session_new("/home/demo").await.unwrap();

    let c = client.clone();
    let sid = session.session_id.clone();
    let turn = tokio::spawn(async move { c.prompt(&sid, &[ContentBlock::text("hi")]).await });

    let mut disconnected = false;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let AcpEvent::Disconnected { .. } = event {
            disconnected = true;
            break;
        }
    }
    assert!(disconnected, "no Disconnected event");

    // The in-flight prompt must not hang once the socket is gone.
    assert!(turn.await.unwrap().is_err(), "pending request should fail");
}

// ---------------------------------------------------------------------------
// Extensions: the add -> read-back assertion

#[tokio::test]
async fn adding_an_extension_round_trips_its_tool_allowlist() {
    let addr = spawn_ext_stub(ExtBehavior::Faithful).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    let extension = mail_extension();
    let entry = client
        .add_extension_verified(&extension, true)
        .await
        .unwrap();

    assert!(entry.enabled);
    assert_eq!(entry.config_key.as_deref(), Some("mail-imap"));
    assert_eq!(
        entry.extension.available_tools(),
        extension.available_tools(),
        "the persisted allowlist must be exactly what was sent"
    );

    // And it is visible to a plain list, with its credential named but never
    // valued.
    let listed = client.config_extensions_list().await.unwrap();
    assert_eq!(listed.extensions.len(), 1);
    assert_eq!(
        listed.extensions[0].extension.env_keys(),
        ["MCP_EMAIL_SERVER_PASSWORD"]
    );

    client.close();
}

/// The test the whole read-back exists for. The server accepts the add and
/// answers OK; only the re-list reveals that the extension is unrestricted.
/// A client that trusted the OK would have quietly given an MCP server every
/// tool it publishes.
#[tokio::test]
async fn an_extension_that_loses_its_allowlist_is_a_hard_error() {
    let addr = spawn_ext_stub(ExtBehavior::DropsAllowlist).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    // The bare add is happy — that is the trap.
    client
        .config_extension_add(&mail_extension(), true)
        .await
        .unwrap();

    let err = client
        .add_extension_verified(&mail_extension(), true)
        .await
        .unwrap_err();
    match &err {
        AcpError::Verification(message) => {
            assert!(
                message.contains("every tool is allowed"),
                "the error must say what is now permitted: {message}"
            );
            assert!(
                message.contains("snake_case"),
                "and why it happened: {message}"
            );
        }
        other => panic!("expected a Verification error, got {other:?}"),
    }

    // And it is not running. The add is sent with `enabled: false` whatever
    // the caller asked for, so the unrestricted-and-live state the quarantine
    // code used to clean up never exists in the first place.
    let listed = client.config_extensions_list().await.unwrap();
    assert_eq!(listed.extensions.len(), 1);
    assert!(
        !listed.extensions[0].enabled,
        "a failed verification must leave the extension switched off"
    );

    client.close();
}

/// An empty allowlist reads like "nothing allowed" and means the opposite, so
/// it never reaches the wire.
#[tokio::test]
async fn an_empty_allowlist_is_refused_before_it_is_sent() {
    let addr = spawn_ext_stub(ExtBehavior::Faithful).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    let unrestricted = GooseExtension::mcp(
        McpServer::Stdio(StdioMcpServer::new("anything", "uvx", vec![])),
        vec![],
        "no allowlist at all",
        vec![],
    );
    let err = client
        .add_extension_verified(&unrestricted, true)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AcpError::Verification(m) if m.contains("empty tool allowlist")),
        "got: {err:?}"
    );

    // Nothing was persisted: the refusal happens client-side.
    assert!(client
        .config_extensions_list()
        .await
        .unwrap()
        .extensions
        .is_empty());

    client.close();
}

/// A credential is proved by handshake — bringing the extension up in a live
/// session, which is where a missing secret becomes a startup failure — and
/// never by reading the value back.
#[tokio::test]
async fn a_credential_is_verified_by_handshake_not_by_reading_it_back() {
    let addr = spawn_ext_stub(ExtBehavior::Faithful).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let extension = mail_extension();
    client
        .add_extension_verified(&extension, true)
        .await
        .unwrap();
    let session = client.session_new("/home/demo").await.unwrap();

    // Without the secret the extension cannot start, and the ACP error says
    // so — in `data`, which is the only place goose puts a reason.
    let err = client
        .session_extension_add(&session.session_id, &extension)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("MCP_EMAIL_SERVER_PASSWORD"),
        "got: {err}"
    );

    client
        .store_secret("MCP_EMAIL_SERVER_PASSWORD", "an-app-password")
        .await
        .unwrap();
    client
        .session_extension_add(&session.session_id, &extension)
        .await
        .unwrap();

    client.close();
}

/// The fresh-install case: nothing has opened a chat, so there is no session
/// to hand shake in. Skipping the handshake would report a mistyped credential
/// as connected, so one is created for the check and deleted afterwards —
/// including when the check fails.
#[tokio::test]
async fn the_handshake_borrows_a_throwaway_session_when_none_is_open() {
    let addr = spawn_ext_stub(ExtBehavior::Faithful).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let extension = mail_extension();
    client
        .add_extension_verified(&extension, true)
        .await
        .unwrap();

    // No session id: the check still runs, and still catches the missing
    // credential. "session not found" here would mean it never ran at all.
    let err = client
        .verify_extension_starts(None, "/home/demo", &extension)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("MCP_EMAIL_SERVER_PASSWORD"),
        "the throwaway session must reach the real startup check, got: {err}"
    );

    client
        .store_secret("MCP_EMAIL_SERVER_PASSWORD", "an-app-password")
        .await
        .unwrap();
    client
        .verify_extension_starts(None, "/home/demo", &extension)
        .await
        .unwrap();

    // Both throwaways are gone — the failing one too, which is the case a
    // plain `?` would have leaked.
    for sid in ["ext-1", "ext-2"] {
        let err = client
            .session_extension_add(sid, &extension)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("session not found"),
            "throwaway session {sid} was left behind: {err}"
        );
    }

    // An open session is used as-is: no new session is created for it, so the
    // next throwaway is still ext-3.
    let session = client.session_new("/home/demo").await.unwrap();
    assert_eq!(session.session_id, "ext-3");
    client
        .verify_extension_starts(Some(&session.session_id), "/home/demo", &extension)
        .await
        .unwrap();
    client
        .session_extension_add(&session.session_id, &extension)
        .await
        .unwrap(); // a session the caller owns must survive the handshake

    client.close();
}

#[tokio::test]
async fn extensions_can_be_toggled_and_the_catalogue_listed() {
    let addr = spawn_ext_stub(ExtBehavior::Faithful).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    let available = client.extensions_available().await.unwrap();
    assert_eq!(available.len(), 2);
    assert_eq!(available[0].name(), "developer");
    assert_eq!(available[0].transport(), "builtin");
    assert!(
        available[0].available_tools().is_empty(),
        "goose's own catalogue is unrestricted, and the type must not hide it"
    );

    client
        .add_extension_verified(&mail_extension(), true)
        .await
        .unwrap();
    client
        .config_extension_set_enabled("mail-imap", false)
        .await
        .unwrap();
    let listed = client.config_extensions_list().await.unwrap();
    assert!(!listed.extensions[0].enabled);

    // The reason names the key, which only `data` carries: `message` is the
    // canned "Invalid params" and would tell the user nothing.
    let err = client
        .config_extension_set_enabled("no-such-extension", true)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("Extension 'no-such-extension' not found"),
        "got: {err}"
    );

    client.close();
}

/// Every extension method goes out through `goose_request`, so a server that
/// has never heard of the namespace produces the same "feature is absent"
/// error the rest of the app already knows how to render, not a raw `-32601`
/// the Connect screen would have to spell out for itself.
#[tokio::test]
async fn an_extension_method_on_an_older_server_is_unsupported() {
    let addr = spawn_ext_stub(ExtBehavior::NoExtensions).await;
    let (client, _events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();

    match client.config_extensions_list().await {
        Err(AcpError::Unsupported {
            feature,
            method,
            reason,
        }) => {
            assert_eq!(feature, Feature::Extensions);
            assert_eq!(method, "_goose/unstable/config/extensions/list");
            assert_eq!(reason.as_deref(), Some("Extensions are not available"));
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }

    // Each method is cached separately, and every one of them classifies as
    // Extensions — including the session-scoped handshake, which lives under
    // `session/` and would otherwise read as session history.
    for err in [
        client.extensions_available().await.unwrap_err(),
        client
            .session_extension_add("ext-1", &mail_extension())
            .await
            .unwrap_err(),
        client
            .config_extension_add(&mail_extension(), false)
            .await
            .unwrap_err(),
        client
            .config_extension_set_enabled("mail-imap", true)
            .await
            .unwrap_err(),
    ] {
        assert!(err.is_unsupported(), "got: {err:?}");
        assert_eq!(
            err.to_string(),
            "Extensions: Extensions are not available",
            "every extension method reports the same feature"
        );
    }

    client.close();
}

/// A whole `config/extensions/list` reply, parsed and written straight back.
///
/// The fixture is a complete response with every optional field present, for
/// the reason `assert_round_trip` documents: an absent field cannot be shown
/// to have been read. It covers the three shapes one reply mixes — a stdio
/// `mcp` server with no `type` tag, an http one that carries `type: "http"`
/// plus the 1.47 OAuth fields this crate does not model, and the tagless
/// `builtin`/`platform` pair — and proves each survives a write-back.
///
/// This complements rather than replaces
/// `serializes_the_exact_wire_spellings`. `GooseExtension` keeps goose's own
/// `skip_serializing_if`, so a field this crate spelled camelCase would come
/// back out of `extra` and the round trip would still match; only the literal
/// frame assertion catches that. What the round trip catches here is the rest:
/// a value mangled in transit, an untagged variant matching the wrong arm, and
/// `configKey`, which is *not* skipped and so shows up as an invented
/// `config_key: null` the moment its casing is wrong.
#[test]
fn the_list_fixture_round_trips() {
    let raw: Value = serde_json::from_str(include_str!("fixtures/extensions.json")).unwrap();
    let listed: ConfigExtensions = assert_round_trip(&raw);

    let names: Vec<&str> = listed
        .extensions
        .iter()
        .map(|e| e.extension.name())
        .collect();
    assert_eq!(names, ["mail-imap", "todoist", "developer", "memory"]);

    let transports: Vec<&str> = listed
        .extensions
        .iter()
        .map(|e| e.extension.transport())
        .collect();
    assert_eq!(transports, ["stdio", "http", "builtin", "platform"]);

    // The OAuth machinery a 1.47 server sends is carried, not modelled.
    assert_eq!(
        listed.extensions[1].extension.available_tools(),
        ["find-tasks"]
    );
    assert_eq!(
        listed.extensions.iter().filter(|e| e.enabled).count(),
        3,
        "todoist is the one switched off"
    );
    assert_eq!(listed.warnings.len(), 1);
}

#[tokio::test]
async fn closing_the_client_reports_disconnected() {
    let addr = spawn_ws_stub(PromptBehavior::Stream).await;
    let (client, mut events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    client.close();

    match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        Ok(Some(AcpEvent::Disconnected { reason })) => {
            assert!(reason.contains("closed"), "unexpected reason: {reason}");
        }
        other => panic!("expected Disconnected, got {other:?}"),
    }
}
