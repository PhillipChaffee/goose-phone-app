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

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use goose_acp_client::{
    probe, AcpClient, AcpError, AcpEvent, ConnectConfig, Feature, ProbeOutcome, SessionKind,
    SessionQuery, SessionUpdate,
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
