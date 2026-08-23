//! End-to-end tests against a stub server that speaks the same wire protocol
//! as `goose serve`: the auth surface (`/status`, `/acp` 401/406) and ACP
//! JSON-RPC over a WebSocket.

// Test/example code: unwrapping a fixture is a failing check, and stdout is
// how an example reports what it verified. Both are denied for shipped code.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "test/example harness: assertions and progress output are the point"
)]


use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use goose_acp_client::{probe, AcpClient, AcpEvent, ConnectConfig, ProbeOutcome, SessionUpdate};
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

// The error type here is fixed by `accept_hdr_async`'s callback signature.
#[allow(clippy::result_large_err)]
async fn handle_ws(sock: TcpStream, behavior: PromptBehavior) {
    // Reject the upgrade unless the shared secret is present, exactly as the
    // real server does.
    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
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
    };

    let Ok(ws) = tokio_tungstenite::accept_hdr_async(sock, check).await else {
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

        let reply = |v: Value| Message::Text(v.to_string().into());

        match method {
            "initialize" => {
                let _ = tx
                    .send(reply(json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentInfo": {"name": "stub-goose", "version": "1.47.0"}
                        }
                    })))
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
                let _ = tx.send(reply(out)).await;
            }
            "session/list" => {
                let _ = tx
                    .send(reply(json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {"sessions": [{
                            "sessionId": "20260820_1",
                            "cwd": "/home/demo",
                            "title": "Earlier chat",
                            "updatedAt": "2026-08-20T10:00:00Z",
                            "_meta": {"messageCount": 4, "lastMessageSnippet": "all done"}
                        }], "nextCursor": null}
                    })))
                    .await;
            }
            "session/prompt" => {
                let sid = frame
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let note = |update: Value| {
                    Message::Text(
                        json!({"jsonrpc":"2.0","method":"session/update",
                               "params":{"sessionId": sid, "update": update}})
                        .to_string()
                        .into(),
                    )
                };

                if behavior == PromptBehavior::Disconnect {
                    return; // drop the socket mid-turn
                }

                let _ = tx
                    .send(note(json!({
                        "sessionUpdate": "agent_thought_chunk",
                        "messageId": "th1",
                        "content": {"type": "text", "text": "thinking"}
                    })))
                    .await;
                let _ = tx
                    .send(note(json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tc1",
                        "title": "shell: ls",
                        "kind": "execute",
                        "status": "pending",
                        "_meta": {"goose": {"toolCall": {"toolName": "developer__shell"}}}
                    })))
                    .await;

                let mut allowed = true;
                if behavior == PromptBehavior::AskPermission {
                    let _ = tx
                        .send(reply(json!({
                            "jsonrpc": "2.0", "id": "perm-1",
                            "method": "session/request_permission",
                            "params": {
                                "sessionId": sid,
                                "toolCall": {"toolCallId": "tc1", "title": "shell: ls"},
                                "options": [
                                    {"optionId": "allow_once", "name": "allow_once", "kind": "allow_once"},
                                    {"optionId": "reject_once", "name": "reject_once", "kind": "reject_once"}
                                ]
                            }
                        })))
                        .await;
                    // Wait for the client's answer to that request.
                    while let Some(Ok(Message::Text(t))) = rx.next().await {
                        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                        if v.get("id").and_then(Value::as_str) == Some("perm-1") {
                            allowed = v
                                .pointer("/result/outcome/optionId")
                                .and_then(Value::as_str)
                                == Some("allow_once");
                            break;
                        }
                    }
                }

                let _ = tx
                    .send(note(json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "tc1",
                        "status": if allowed { "completed" } else { "failed" },
                        "content": [{"type": "content",
                                     "content": {"type": "text", "text": "file_a"}}]
                    })))
                    .await;
                let _ = tx
                    .send(note(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "m1",
                        "content": {"type": "text", "text": "Hello "}
                    })))
                    .await;
                let _ = tx
                    .send(note(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "m1",
                        "content": {"type": "text", "text": "world"}
                    })))
                    .await;
                let _ = tx
                    .send(reply(json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {"stopReason": "end_turn"}
                    })))
                    .await;
            }
            "session/delete" => {
                let _ = tx
                    .send(reply(json!({"jsonrpc":"2.0","id":id,"result":{}})))
                    .await;
            }
            "unknown/method" => {
                let _ = tx
                    .send(reply(json!({"jsonrpc":"2.0","id":id,
                        "error":{"code":-32601,"message":"method not found"}})))
                    .await;
            }
            _ => {}
        }
    }
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
