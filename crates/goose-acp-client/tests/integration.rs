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
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use goose_acp_client::{
    probe, AcpClient, AcpError, AcpEvent, ConnectConfig, GooseExtension, McpServer, ProbeOutcome,
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
}

async fn spawn_ws_stub(behavior: PromptBehavior) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(handle_ws(sock, behavior));
        }
    });
    addr
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

/// Send a `session/update` notification carrying `update`.
async fn notify(tx: &mut WsTx, session_id: &str, update: Value) {
    send(
        tx,
        json!({"jsonrpc":"2.0","method":"session/update",
               "params":{"sessionId": session_id, "update": update}}),
    )
    .await;
}

async fn handle_ws(sock: TcpStream, behavior: PromptBehavior) {
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

async fn handle_ext_ws(sock: TcpStream, behavior: ExtBehavior) {
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(sock, check_secret).await else {
        return;
    };
    let (mut tx, mut rx) = ws.split();

    // Per-connection config store. Secrets are tracked by NAME ONLY — the
    // stub never keeps a credential's value, for the same reason the app never
    // reads one back.
    let mut configured: Vec<Value> = Vec::new();
    let mut secret_keys: HashSet<String> = HashSet::new();

    while let Some(Ok(msg)) = rx.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let id = frame.get("id").cloned();
        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
        let params = frame.get("params").cloned().unwrap_or(Value::Null);

        let outcome = ext_request(method, &params, behavior, &mut configured, &mut secret_keys);
        let reply = match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        };
        send(&mut tx, reply).await;
    }
}

/// One request against the stub's config store.
fn ext_request(
    method: &str,
    params: &Value,
    behavior: ExtBehavior,
    configured: &mut Vec<Value>,
    secret_keys: &mut HashSet<String>,
) -> Result<Value, (i64, String)> {
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
            Ok(json!({"extensions": configured, "warnings": []}))
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
            configured.retain(|e| ext_name(&e["extension"]) != name);
            configured.push(json!({
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
            match configured
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
                secret_keys.insert(key.to_string());
            }
            Ok(json!({}))
        }

        "_goose/unstable/session/extensions/add" => {
            // The handshake. goose launches the MCP server here, and a
            // stdio child whose declared env key has no stored secret is a
            // hard startup failure — so that is what a missing secret does.
            let extension = params.get("extension").cloned().unwrap_or(Value::Null);
            let missing: Vec<String> = extension
                .get("envKeys")
                .and_then(Value::as_array)
                .map(|keys| {
                    keys.iter()
                        .filter_map(Value::as_str)
                        .filter(|k| !secret_keys.contains(*k))
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

        _ => Err((-32601, format!("method not found: {method}"))),
    }
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

    let page = client.session_list(None).await.unwrap();
    assert_eq!(page.sessions.len(), 1);
    let s = &page.sessions[0];
    assert_eq!(s.display_title(), "Earlier chat");
    assert_eq!(s.message_count(), Some(4));
    assert_eq!(s.last_message_snippet().as_deref(), Some("all done"));

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
    let turn = tokio::spawn(async move { c.prompt(&sid, "hi").await });

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

#[tokio::test]
async fn permission_request_is_surfaced_and_answered() {
    let addr = spawn_ws_stub(PromptBehavior::AskPermission).await;
    let (client, mut events, _) = AcpClient::connect(&config(addr, SECRET)).await.unwrap();
    let session = client.session_new("/home/demo").await.unwrap();

    let c = client.clone();
    let sid = session.session_id.clone();
    let turn = tokio::spawn(async move { c.prompt(&sid, "run a tool").await });

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
    let turn = tokio::spawn(async move { c.prompt(&sid, "hi").await });

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
        AcpError::Allowlist(message) => {
            assert!(
                message.contains("every tool is allowed"),
                "the error must say what is now permitted: {message}"
            );
            assert!(
                message.contains("snake_case"),
                "and why it happened: {message}"
            );
        }
        other => panic!("expected an Allowlist error, got {other:?}"),
    }

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
        matches!(&err, AcpError::Allowlist(m) if m.contains("empty tool allowlist")),
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

    // Without the secret the extension cannot start, and the ACP error says so.
    let err = client
        .session_extension_add("20260821_1", &extension)
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
        .session_extension_add("20260821_1", &extension)
        .await
        .unwrap();

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

    let err = client
        .config_extension_set_enabled("no-such-extension", true)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"), "got: {err}");

    client.close();
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
