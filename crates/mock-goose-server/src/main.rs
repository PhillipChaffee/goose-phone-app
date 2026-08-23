//! Protocol-faithful mock of `goose serve` for exercising the app end-to-end
//! without an AI provider. Speaks ACP JSON-RPC over WebSocket at /acp with
//! the same auth surface as the real server (X-Secret-Key / ?token=, 401,
//! and the 406 auth-success probe), streams scripted turns with thinking,
//! markdown, tool calls and permission round-trips, and replays history on
//! session/load.
//!
//!   cargo run -p mock-goose-server -- [port]          (default 3285)
//!   MOCK_SECRET=...                                    (default mock-secret)
//!
//! Prompt keywords: "slow" = long stream (time to hit Stop);
//! "notool" = skip the tool call / permission prompt.

// This binary is a test double, not shipped code: it prints its listening
// address on purpose, and an unwrap on a fixture here is a failing test rather
// than a crash on someone's phone. The reasons those lints are denied
// workspace-wide do not apply to it.
#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test double: stdout is its interface and fixtures are trusted"
)]

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Notify};
use tokio_tungstenite::tungstenite::Message;

// ---------------------------------------------------------------------------
// Shared state

#[derive(Clone, Default)]
struct SessionData {
    cwd: String,
    title: String,
    /// Stored `session/update` payloads, replayed verbatim on session/load.
    conversation: Vec<Value>,
    message_count: u64,
    snippet: String,
}

#[derive(Default)]
struct State {
    sessions: HashMap<String, SessionData>,
    next_session: u64,
}

type Shared = Arc<Mutex<State>>;

static SERVER_REQ_ID: AtomicU64 = AtomicU64::new(1);

fn seed(state: &Shared) {
    let mut s = state.lock().unwrap();
    s.next_session = 2;
    s.sessions.insert(
        "20260820_1".to_string(),
        SessionData {
            cwd: "/home/demo".to_string(),
            title: "Seeded example chat".to_string(),
            conversation: vec![
                json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"What files are in my project?"}}),
                json!({"sessionUpdate":"tool_call","toolCallId":"seed_tc1","title":"shell: ls","kind":"execute","status":"pending",
                       "rawInput":{"command":"ls"},
                       "_meta":{"goose":{"toolCall":{"toolName":"developer__shell","extensionName":"developer"}}}}),
                json!({"sessionUpdate":"tool_call_update","toolCallId":"seed_tc1","status":"completed",
                       "content":[{"type":"content","content":{"type":"text","text":"Cargo.toml\nsrc\nREADME.md"}}]}),
                json!({"sessionUpdate":"agent_message_chunk","messageId":"seed_m1",
                       "content":{"type":"text","text":"Your project contains **Cargo.toml**, a `src/` directory, and a README."}}),
            ],
            message_count: 2,
            snippet: "Your project contains Cargo.toml, a src/ directory…".to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// A TcpStream with already-read bytes stitched back on the front, so we can
// inspect the HTTP request head before handing the socket to tungstenite.

struct Prefixed {
    prefix: Vec<u8>,
    pos: usize,
    inner: TcpStream,
}

impl AsyncRead for Prefixed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.pos < self.prefix.len() {
            let n = (self.prefix.len() - self.pos).min(buf.remaining());
            let start = self.pos;
            buf.put_slice(&self.prefix[start..start + n]);
            self.pos += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Prefixed {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(3285);
    let secret = std::env::var("MOCK_SECRET").unwrap_or_else(|_| "mock-secret".to_string());

    let state: Shared = Arc::default();
    seed(&state);

    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    println!("mock goose serve listening on http://127.0.0.1:{port} (secret: {secret})");

    loop {
        let Ok((socket, _)) = listener.accept().await else {
            continue;
        };
        let state = state.clone();
        let secret = secret.clone();
        tokio::spawn(async move {
            let _ = handle_conn(socket, state, secret).await;
        });
    }
}

async fn handle_conn(mut socket: TcpStream, state: Shared, secret: String) -> io::Result<()> {
    // Read the request head.
    let mut head = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16384 {
        if socket.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let request_line = text.lines().next().unwrap_or("").to_string();
    let path_q = request_line.split_whitespace().nth(1).unwrap_or("/");
    let path = path_q.split('?').next().unwrap_or("/");
    let lower = text.to_lowercase();
    let is_upgrade = lower.contains("upgrade: websocket");

    let header = |name: &str| -> Option<String> {
        let prefix = format!("{name}:");
        text.lines()
            .find(|l| l.to_lowercase().starts_with(&prefix))
            .map(|l| l[prefix.len()..].trim().to_string())
    };
    let token = path_q
        .split_once('?')
        .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
        .map(str::to_string);
    let authed = header("x-secret-key").as_deref() == Some(secret.as_str())
        || token.as_deref() == Some(secret.as_str());

    if !is_upgrade {
        let (code, body) = match path {
            "/status" | "/health" => ("200 OK", "ok"),
            "/acp" if authed => ("406 Not Acceptable", ""),
            "/acp" => ("401 Unauthorized", ""),
            _ => ("404 Not Found", ""),
        };
        let resp = format!(
            "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    if path != "/acp" || !authed {
        socket
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }

    let stream = Prefixed {
        prefix: head,
        pos: 0,
        inner: socket,
    };
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    serve_ws(ws, state).await;
    Ok(())
}

type Ws = tokio_tungstenite::WebSocketStream<Prefixed>;

async fn serve_ws(ws: Ws, state: Shared) {
    // MOCK_SILENT=1 simulates a half-open connection: the socket stays open
    // but the peer stops reading and answering (so tungstenite never
    // auto-pongs). Used to verify the client's ping-timeout detection.
    let silent_after = std::env::var("MOCK_SILENT").ok().is_some_and(|v| v == "1");

    let (mut sink, mut stream) = ws.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();

    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Pending server->client requests (permission prompts) and per-session
    // cancellation flags.
    let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> = Arc::default();
    let cancels: Arc<Mutex<HashMap<String, Arc<Notify>>>> = Arc::default();

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(p) => {
                let _ = out_tx.send(Message::Pong(p));
                continue;
            }
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };

        let method = frame
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = frame.get("id").cloned();
        let params = frame.get("params").cloned().unwrap_or(Value::Null);

        match (method.as_deref(), id) {
            // Response to one of OUR requests (permission prompt).
            (None, Some(id)) => {
                let key = id.to_string();
                if let Some(tx) = pending.lock().unwrap().remove(&key) {
                    let _ = tx.send(frame.get("result").cloned().unwrap_or(Value::Null));
                }
            }
            (Some("session/cancel"), None) => {
                let sid = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if let Some(notify) = cancels.lock().unwrap().get(sid) {
                    notify.notify_waiters();
                }
            }
            (Some("session/prompt"), Some(id)) => {
                let out = out_tx.clone();
                let state = state.clone();
                let pending = pending.clone();
                let cancel = Arc::new(Notify::new());
                let sid = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                cancels.lock().unwrap().insert(sid.clone(), cancel.clone());
                let cancels = cancels.clone();
                tokio::spawn(async move {
                    run_turn(id, params, out, state, pending, cancel).await;
                    cancels.lock().unwrap().remove(&sid);
                });
            }
            (Some(m), Some(id)) => {
                let response = handle_request(m, &params, &state, &out_tx);
                let frame = match response {
                    Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                    Err((code, msg)) => {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})
                    }
                };
                let _ = out_tx.send(Message::Text(frame.to_string().into()));

                if silent_after && m == "initialize" {
                    // Answer the handshake, then go dead while holding the
                    // socket open: stop polling the stream so no pongs are
                    // ever sent. A correct client notices via ping timeout.
                    eprintln!("MOCK_SILENT: going silent after initialize");
                    tokio::time::sleep(Duration::from_secs(600)).await;
                    return;
                }
            }
            _ => {}
        }
    }
    writer.abort();
}

fn notify(out: &mpsc::UnboundedSender<Message>, method: &str, params: Value) {
    let frame = json!({"jsonrpc":"2.0","method":method,"params":params});
    let _ = out.send(Message::Text(frame.to_string().into()));
}

fn session_update(out: &mpsc::UnboundedSender<Message>, sid: &str, update: &Value) {
    notify(
        out,
        "session/update",
        json!({"sessionId": sid, "update": update}),
    );
}

fn handle_request(
    method: &str,
    params: &Value,
    state: &Shared,
    out: &mpsc::UnboundedSender<Message>,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": 1,
            "agentInfo": {"name": "goose-mock", "version": "1.47.0"},
            "agentCapabilities": {
                "loadSession": true,
                "sessionCapabilities": {"list": {}, "delete": {}, "close": {}},
                "promptCapabilities": {"image": true, "embeddedContext": true}
            },
            "authMethods": []
        })),
        "session/new" => {
            let cwd = params
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !cwd.starts_with('/') {
                return Err((-32602, format!("cwd must be an absolute path, got `{cwd}`")));
            }
            let mut s = state.lock().unwrap();
            let n = s.next_session;
            s.next_session += 1;
            let sid = format!("20260821_{n}");
            s.sessions.insert(
                sid.clone(),
                SessionData {
                    cwd: cwd.to_string(),
                    ..Default::default()
                },
            );
            Ok(json!({"sessionId": sid, "modes": null, "configOptions": []}))
        }
        "session/load" => {
            let sid = params
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let data = state.lock().unwrap().sessions.get(sid).cloned();
            match data {
                Some(data) => {
                    for update in &data.conversation {
                        session_update(out, sid, update);
                    }
                    Ok(json!({"modes": null, "configOptions": []}))
                }
                None => Err((-32002, format!("session not found: {sid}"))),
            }
        }
        "session/list" => {
            let s = state.lock().unwrap();
            let mut sessions: Vec<&String> = s.sessions.keys().collect();
            sessions.sort();
            sessions.reverse();
            let list: Vec<Value> = sessions
                .into_iter()
                .filter(|sid| s.sessions[*sid].message_count > 0)
                .map(|sid| {
                    let d = &s.sessions[sid];
                    json!({
                        "sessionId": sid,
                        "cwd": d.cwd,
                        "additionalDirectories": [],
                        "title": if d.title.is_empty() { Value::Null } else { Value::String(d.title.clone()) },
                        "updatedAt": "2026-08-21T12:00:00Z",
                        "_meta": {
                            "messageCount": d.message_count,
                            "createdAt": "2026-08-21T09:00:00Z",
                            "userSetName": false,
                            "sessionType": "user",
                            "hasRecipe": false,
                            "lastMessageSnippet": d.snippet,
                        }
                    })
                })
                .collect();
            Ok(json!({"sessions": list, "nextCursor": null}))
        }
        "session/delete" => {
            let sid = params
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("");
            state.lock().unwrap().sessions.remove(sid);
            Ok(json!({}))
        }
        "session/close" => Ok(json!({})),
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

/// The scripted agent turn.
async fn run_turn(
    request_id: Value,
    params: Value,
    out: mpsc::UnboundedSender<Message>,
    state: Shared,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    cancel: Arc<Notify>,
) {
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let user_text = params
        .pointer("/prompt/0/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if !state.lock().unwrap().sessions.contains_key(&sid) {
        let frame = json!({"jsonrpc":"2.0","id":request_id,
            "error":{"code":-32002,"message":format!("session not found: {sid}")}});
        let _ = out.send(Message::Text(frame.to_string().into()));
        return;
    }

    let mut record: Vec<Value> = Vec::new();
    let slow = user_text.to_lowercase().contains("slow");
    let with_tool = !user_text.to_lowercase().contains("notool");
    let delay = Duration::from_millis(if slow { 400 } else { 150 });
    let mut cancelled = false;

    let emit = |u: Value, record: &mut Vec<Value>| {
        session_update(&out, &sid, &u);
        record.push(u);
    };

    macro_rules! step {
        () => {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = cancel.notified() => { cancelled = true; }
            }
            if cancelled {
                finish(
                    &out,
                    &state,
                    &sid,
                    request_id,
                    record,
                    "cancelled",
                    user_text,
                )
                .await;
                return;
            }
        };
    }

    // Real goose only replays user chunks on session/load — it does NOT echo
    // them during a live turn. Record for replay without emitting.
    record.push(
        json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":user_text}}),
    );
    notify(
        &out,
        "_goose/unstable/session/update",
        json!({"sessionId": sid, "update": {"sessionUpdate":"usage_update","used":18432,"contextLimit":128000}}),
    );

    // Thinking stream.
    for part in ["Let me think about ", "what you're asking…"] {
        step!();
        emit(
            json!({"sessionUpdate":"agent_thought_chunk","messageId":"th_1",
                   "content":{"type":"text","text":part}}),
            &mut record,
        );
    }

    // Tool call with a permission round-trip.
    if with_tool {
        step!();
        emit(
            json!({"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"shell: uname -a",
                   "kind":"execute","status":"pending","rawInput":{"command":"uname -a"},
                   "_meta":{"goose":{"toolCall":{"toolName":"developer__shell","extensionName":"developer"}}}}),
            &mut record,
        );

        let (tx, rx) = oneshot::channel();
        let req_id = format!("srv-{}", SERVER_REQ_ID.fetch_add(1, Ordering::Relaxed));
        pending.lock().unwrap().insert(format!("\"{req_id}\""), tx);
        let frame = json!({"jsonrpc":"2.0","id":req_id,"method":"session/request_permission","params":{
        "sessionId": sid,
        "toolCall": {"toolCallId":"tc_1","title":"shell: uname -a","kind":"execute","rawInput":{"command":"uname -a"}},
        "options": [
            {"optionId":"allow_always","name":"allow_always","kind":"allow_always"},
            {"optionId":"allow_once","name":"allow_once","kind":"allow_once"},
            {"optionId":"reject_once","name":"reject_once","kind":"reject_once"},
            {"optionId":"reject_always","name":"reject_always","kind":"reject_always"}
        ]}});
        let _ = out.send(Message::Text(frame.to_string().into()));

        let outcome = tokio::select! {
            r = rx => r.unwrap_or(Value::Null),
            _ = cancel.notified() => {
                finish(&out, &state, &sid, request_id, record, "cancelled", user_text).await;
                return;
            }
        };
        let allowed = outcome
            .pointer("/outcome/optionId")
            .and_then(Value::as_str)
            .map(|o| o.starts_with("allow"))
            .unwrap_or(false);

        if allowed {
            emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"in_progress"}),
                &mut record,
            );
            step!();
            emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"completed",
                       "content":[{"type":"content","content":{"type":"text","text":"Linux goose-box 6.8.0 x86_64 GNU/Linux"}}]}),
                &mut record,
            );
        } else {
            emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"failed",
                       "content":[{"type":"content","content":{"type":"text","text":"Tool call rejected by the user"}}]}),
                &mut record,
            );
        }
    }

    // Assistant message stream (markdown showcase).
    let chunks: Vec<String> = if slow {
        (1..=40)
            .map(|i| format!("chunk {i} of a very long streaming answer… "))
            .collect()
    } else {
        vec![
            "Here's what I found:\n\n".into(),
            "1. Your server is a **Linux x86_64** box\n".into(),
            "2. Everything looks healthy\n\n".into(),
            "```bash\nuname -a  # the command I ran\n```\n\n".into(),
            "| Check | Result |\n|---|---|\n| Kernel | 6.8.0 |\n| Arch | x86_64 |\n\n".into(),
            "Anything ~~broken~~ else you'd like me to look at?".into(),
        ]
    };
    for chunk in chunks {
        step!();
        emit(
            json!({"sessionUpdate":"agent_message_chunk","messageId":"m_1",
                   "content":{"type":"text","text":chunk}}),
            &mut record,
        );
    }

    // Auto-title + final usage, then resolve the prompt.
    emit(
        json!({"sessionUpdate":"session_info_update","title":auto_title(&user_text),
               "updatedAt":"2026-08-21T12:00:00Z"}),
        &mut record,
    );
    notify(
        &out,
        "_goose/unstable/session/update",
        json!({"sessionId": sid, "update": {"sessionUpdate":"usage_update","used":21580,"contextLimit":128000}}),
    );
    finish(
        &out, &state, &sid, request_id, record, "end_turn", user_text,
    )
    .await;
}

fn auto_title(user_text: &str) -> String {
    let words: Vec<&str> = user_text.split_whitespace().take(5).collect();
    if words.is_empty() {
        "New chat".to_string()
    } else {
        words.join(" ")
    }
}

async fn finish(
    out: &mpsc::UnboundedSender<Message>,
    state: &Shared,
    sid: &str,
    request_id: Value,
    record: Vec<Value>,
    stop_reason: &str,
    user_text: String,
) {
    {
        let mut s = state.lock().unwrap();
        if let Some(data) = s.sessions.get_mut(sid) {
            data.conversation.extend(record);
            data.message_count += 2;
            data.snippet = format!("Re: {user_text}");
            if data.title.is_empty() {
                data.title = auto_title(&user_text);
            }
        }
    }
    let frame = json!({"jsonrpc":"2.0","id":request_id,"result":{"stopReason":stop_reason}});
    let _ = out.send(Message::Text(frame.to_string().into()));
}
