//! Protocol-faithful mock of `goose serve` for exercising the app end-to-end
//! without an AI provider. Speaks ACP JSON-RPC over WebSocket at /acp with
//! the same auth surface as the real server (X-Secret-Key / ?token=, 401,
//! and the 406 auth-success probe), streams scripted turns with thinking,
//! markdown, tool calls and permission round-trips, and replays history on
//! session/load.
//!
//!   cargo run -p mock-goose-server -- [port]          (default 3285)
//!   `MOCK_SECRET`=...                                    (default mock-secret)
//!
//! Prompt keywords: "slow" = long stream (time to hit Stop);
//! "notool" = skip the tool call / permission prompt.

// This binary is a test double, not shipped code: it prints its listening
// address on purpose, and an unwrap on a fixture here is a failing test rather
// than a crash on someone's phone. The reasons those lints are denied
// workspace-wide do not apply to it.
#![expect(
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

struct State {
    sessions: HashMap<String, SessionData>,
    next_session: u64,
    /// Whatever the client last selected on each option, so a switch actually
    /// sticks and a reload shows it.
    config: SessionConfig,
}

/// The four options goose routes in `session/set_config_option`. Anything
/// outside them is an `invalid_params` error there and here.
#[derive(Clone)]
struct SessionConfig {
    provider: String,
    mode: String,
    model: String,
    thinking_effort: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            next_session: 0,
            config: SessionConfig {
                provider: "anthropic".to_string(),
                mode: "auto".to_string(),
                model: "claude-sonnet-5".to_string(),
                thinking_effort: "off".to_string(),
            },
        }
    }
}

type Shared = Arc<Mutex<State>>;

static SERVER_REQ_ID: AtomicU64 = AtomicU64::new(1);

fn seed(state: &Shared) {
    let seeded = SessionData {
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
    };

    let mut s = state.lock().unwrap();
    s.next_session = 2;
    s.sessions.insert("20260820_1".to_string(), seeded);
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
    let silent_after = std::env::var("MOCK_SILENT").is_ok_and(|v| v == "1");

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
    let pending: Pending = Arc::default();
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
                let waiter = pending.lock().unwrap().remove(&key);
                if let Some(tx) = waiter {
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
                    #[expect(
                        clippy::print_stderr,
                        reason = "test double: MOCK_SILENT is opt-in test scaffolding, and this \
                                  note belongs on stderr so it stays out of the stdout the \
                                  harness reads for the listening address"
                    )]
                    {
                        eprintln!("MOCK_SILENT: going silent after initialize");
                    }
                    tokio::time::sleep(Duration::from_secs(600)).await;
                    return;
                }
            }
            _ => {}
        }
    }
    writer.abort();
}

fn notify(out: &mpsc::UnboundedSender<Message>, method: &str, params: &Value) {
    let frame = json!({"jsonrpc":"2.0","method":method,"params":params});
    let _ = out.send(Message::Text(frame.to_string().into()));
}

fn session_update(out: &mpsc::UnboundedSender<Message>, sid: &str, update: &Value) {
    notify(
        out,
        "session/update",
        &json!({"sessionId": sid, "update": update}),
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
            let sid = {
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
                sid
            };
            let config = state.lock().unwrap().config.clone();
            Ok(json!({"sessionId": sid, "modes": null, "configOptions": config_options(&config)}))
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
                    let config = state.lock().unwrap().config.clone();
                    Ok(json!({"modes": null, "configOptions": config_options(&config)}))
                }
                None => Err((-32002, format!("session not found: {sid}"))),
            }
        }
        "session/set_config_option" => {
            let sid = params
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let config_id = params.get("configId").and_then(Value::as_str).unwrap_or("");
            let value = params.get("value").and_then(Value::as_str).unwrap_or("");
            let config = {
                let mut s = state.lock().unwrap();
                match config_id {
                    "provider" => s.config.provider = value.to_string(),
                    "mode" => s.config.mode = value.to_string(),
                    "model" => {
                        s.config.model = value.to_string();
                        // Effort is a property of the model: switching to one
                        // that cannot reason drops the session back to `off`,
                        // exactly as goose's response builder does.
                        if !is_reasoning_model(value) {
                            s.config.thinking_effort = "off".to_string();
                        }
                    }
                    "thinking_effort" => s.config.thinking_effort = value.to_string(),
                    other => return Err((-32602, format!("Unsupported config option: {other}"))),
                }
                s.config.clone()
            };
            let opts = config_options(&config);
            // The real agent pushes this after every change so a second
            // client watching the same session stays in step.
            session_update(
                out,
                sid,
                &json!({"sessionUpdate": "config_option_update", "configOptions": opts}),
            );
            Ok(json!({"configOptions": opts}))
        }
        "session/list" => Ok(json!({"sessions": list_sessions(state), "nextCursor": null})),
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

/// The fields `session/list` reports, copied out under the lock so the JSON
/// is built without holding it.
struct Listed {
    id: String,
    cwd: String,
    title: String,
    message_count: u64,
    snippet: String,
}

/// The `session/list` payload: every session that has messages, newest
/// session id first.
/// Whether the model takes an extended-thinking effort at all.
///
/// The distinction is the point: goose offers the five effort tiers only for
/// a reasoning model and collapses to a lone `off` otherwise, which is what
/// makes the app's fact-row path reachable without a real provider.
fn is_reasoning_model(model: &str) -> bool {
    model != "qwen3-coder-480b"
}

/// The `configOptions` array a real agent returns, in the shape ACP schema
/// 1.5 defines: a flattened kind tagged by `type`, an optional `description`,
/// and select options keyed on `value`.
///
/// All four options goose builds, in its order — `session/set_config_option`
/// routes exactly these ids, so a fifth here would be a control the real
/// agent rejects.
fn config_options(config: &SessionConfig) -> Value {
    let efforts: Value = if is_reasoning_model(&config.model) {
        json!([
            {"value": "off", "name": "off"},
            {"value": "low", "name": "low"},
            {"value": "medium", "name": "medium"},
            {"value": "high", "name": "high"},
            {"value": "max", "name": "max"},
        ])
    } else {
        json!([{"value": "off", "name": "off"}])
    };
    json!([
        {
            "configId": "provider",
            "name": "Provider",
            "type": "select",
            "currentValue": config.provider,
            "options": [
                {"value": "anthropic", "name": "Anthropic"},
                {"value": "openai", "name": "OpenAI"},
            ]
        },
        {
            "configId": "mode",
            "name": "Mode",
            "category": "mode",
            "type": "select",
            "currentValue": config.mode,
            "options": [
                {"value": "auto", "name": "Auto", "description": "Run tools without asking."},
                {"value": "approve", "name": "Manual approval",
                 "description": "Ask before every tool call."},
                {"value": "chat", "name": "Chat only", "description": "No tools at all."},
            ]
        },
        {
            "configId": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": config.model,
            "options": [
                {"value": "claude-opus-5", "name": "Claude Opus 5"},
                {"value": "claude-sonnet-5", "name": "Claude Sonnet 5"},
                {"value": "gpt-5.2", "name": "GPT-5.2"},
                {"value": "qwen3-coder-480b", "name": "Qwen3 Coder 480B"},
            ]
        },
        {
            "configId": "thinking_effort",
            "name": "Thinking effort",
            "category": "thought_level",
            "type": "select",
            "description":
                "Controls reasoning effort for models that support extended thinking.",
            "currentValue": config.thinking_effort,
            "options": efforts
        }
    ])
}

fn list_sessions(state: &Shared) -> Vec<Value> {
    let mut listed: Vec<Listed> = {
        let s = state.lock().unwrap();
        s.sessions
            .iter()
            .filter(|(_, d)| d.message_count > 0)
            .map(|(id, d)| Listed {
                id: id.clone(),
                cwd: d.cwd.clone(),
                title: d.title.clone(),
                message_count: d.message_count,
                snippet: d.snippet.clone(),
            })
            .collect()
    };
    listed.sort_by(|a, b| b.id.cmp(&a.id));
    listed
        .into_iter()
        .map(|d| {
            json!({
                "sessionId": d.id,
                "cwd": d.cwd,
                "additionalDirectories": [],
                "title": if d.title.is_empty() { Value::Null } else { Value::String(d.title) },
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
        .collect()
}

// ---------------------------------------------------------------------------
// The scripted agent turn

/// Server->client requests awaiting an answer (permission prompts), keyed by
/// the JSON-encoded request id the client echoes back.
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// One scripted turn in flight: where its updates go, how it is cancelled,
/// and the transcript accumulated for replay on `session/load`.
struct Turn {
    out: mpsc::UnboundedSender<Message>,
    sid: String,
    cancel: Arc<Notify>,
    delay: Duration,
    record: Vec<Value>,
    /// How many blocks of the prompt were not text. Said back in the answer:
    /// against a mock, "did the attachment actually reach the server" is
    /// otherwise a question only a packet capture can settle.
    attachments: usize,
}

/// The client sent `session/cancel` while the turn was streaming.
struct Cancelled;

impl Turn {
    /// Send a `session/update` and keep it for replay on `session/load`.
    fn emit(&mut self, update: Value) {
        session_update(&self.out, &self.sid, &update);
        self.record.push(update);
    }

    /// Pause between updates, ending the turn early if a cancel lands first.
    async fn step(&self) -> Result<(), Cancelled> {
        tokio::select! {
            () = tokio::time::sleep(self.delay) => Ok(()),
            () = self.cancel.notified() => Err(Cancelled),
        }
    }

    /// Thinking stream.
    async fn think(&mut self) -> Result<(), Cancelled> {
        for part in ["Let me think about ", "what you're asking…"] {
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"agent_thought_chunk","messageId":"th_1",
                       "content":{"type":"text","text":part}}),
            );
        }
        Ok(())
    }

    /// Ask the client to approve the scripted tool call and wait for its
    /// answer; `true` once one of the `allow*` options comes back.
    async fn ask_permission(&self, pending: &Pending) -> Result<bool, Cancelled> {
        let (tx, rx) = oneshot::channel();
        let req_id = format!("srv-{}", SERVER_REQ_ID.fetch_add(1, Ordering::Relaxed));
        pending.lock().unwrap().insert(format!("\"{req_id}\""), tx);
        let sid = &self.sid;
        let frame = json!({"jsonrpc":"2.0","id":req_id,"method":"session/request_permission","params":{
        "sessionId": sid,
        "toolCall": {"toolCallId":"tc_1","title":"shell: uname -a","kind":"execute","rawInput":{"command":"uname -a"}},
        "options": [
            {"optionId":"allow_always","name":"allow_always","kind":"allow_always"},
            {"optionId":"allow_once","name":"allow_once","kind":"allow_once"},
            {"optionId":"reject_once","name":"reject_once","kind":"reject_once"},
            {"optionId":"reject_always","name":"reject_always","kind":"reject_always"}
        ]}});
        let _ = self.out.send(Message::Text(frame.to_string().into()));

        let outcome = tokio::select! {
            r = rx => r.unwrap_or(Value::Null),
            () = self.cancel.notified() => return Err(Cancelled),
        };
        Ok(outcome
            .pointer("/outcome/optionId")
            .and_then(Value::as_str)
            .is_some_and(|o| o.starts_with("allow")))
    }

    /// Tool call with a permission round-trip.
    async fn tool_call(&mut self, pending: &Pending) -> Result<(), Cancelled> {
        self.step().await?;
        self.emit(
            json!({"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"shell: uname -a",
                   "kind":"execute","status":"pending","rawInput":{"command":"uname -a"},
                   "_meta":{"goose":{"toolCall":{"toolName":"developer__shell","extensionName":"developer"}}}}),
        );

        if self.ask_permission(pending).await? {
            self.emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"in_progress"}),
            );
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"completed",
                       "content":[{"type":"content","content":{"type":"text","text":"Linux goose-box 6.8.0 x86_64 GNU/Linux"}}]}),
            );
        } else {
            self.emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"failed",
                       "content":[{"type":"content","content":{"type":"text","text":"Tool call rejected by the user"}}]}),
            );
        }
        Ok(())
    }

    /// Assistant message stream (markdown showcase).
    async fn answer(&mut self, slow: bool) -> Result<(), Cancelled> {
        if self.attachments > 0 {
            let n = self.attachments;
            let plural = if n == 1 { "" } else { "s" };
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"agent_message_chunk","messageId":"m_1",
                   "content":{"type":"text","text":format!("Got {n} attachment{plural}.\n\n")}}),
            );
        }
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
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"agent_message_chunk","messageId":"m_1",
                       "content":{"type":"text","text":chunk}}),
            );
        }
        Ok(())
    }

    /// The scripted turn in order: thinking, the optional tool call, the
    /// answer. Stops at the first step the client cancelled.
    async fn script(
        &mut self,
        pending: &Pending,
        with_tool: bool,
        slow: bool,
    ) -> Result<(), Cancelled> {
        self.think().await?;
        if with_tool {
            self.tool_call(pending).await?;
        }
        self.answer(slow).await
    }
}

async fn run_turn(
    request_id: Value,
    params: Value,
    out: mpsc::UnboundedSender<Message>,
    state: Shared,
    pending: Pending,
    cancel: Arc<Notify>,
) {
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // The first TEXT block, not the first block: an attached image is another
    // entry in the same array, and `prompt/0/text` read as empty the moment
    // one arrived ahead of the message.
    let prompt: Vec<Value> = params
        .get("prompt")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let user_text = prompt
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let attachments = prompt
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) != Some("text"))
        .count();

    if !state.lock().unwrap().sessions.contains_key(&sid) {
        let frame = json!({"jsonrpc":"2.0","id":request_id,
            "error":{"code":-32002,"message":format!("session not found: {sid}")}});
        let _ = out.send(Message::Text(frame.to_string().into()));
        return;
    }

    let slow = user_text.to_lowercase().contains("slow");
    let with_tool = !user_text.to_lowercase().contains("notool");
    let mut turn = Turn {
        out,
        sid,
        cancel,
        delay: Duration::from_millis(if slow { 400 } else { 150 }),
        record: Vec::new(),
        attachments,
    };

    // Real goose only replays user chunks on session/load — it does NOT echo
    // them during a live turn. Record for replay without emitting. Every
    // block, not just the text one: replaying an attachment is how the
    // transcript gets it back after a reconnect, and a mock that dropped them
    // would make that path untestable without a real server.
    for content in prompt {
        turn.record
            .push(json!({"sessionUpdate": "user_message_chunk", "content": content}));
    }
    notify(
        &turn.out,
        "_goose/unstable/session/update",
        &json!({"sessionId": turn.sid, "update": {"sessionUpdate":"usage_update","used":18432,"contextLimit":128_000}}),
    );

    let stop_reason = if turn.script(&pending, with_tool, slow).await.is_ok() {
        // Auto-title + final usage, then resolve the prompt.
        turn.emit(
            json!({"sessionUpdate":"session_info_update","title":auto_title(&user_text),
                   "updatedAt":"2026-08-21T12:00:00Z"}),
        );
        notify(
            &turn.out,
            "_goose/unstable/session/update",
            &json!({"sessionId": turn.sid, "update": {"sessionUpdate":"usage_update","used":21580,"contextLimit":128_000}}),
        );
        "end_turn"
    } else {
        "cancelled"
    };

    let Turn {
        out, sid, record, ..
    } = turn;
    finish(
        &out,
        &state,
        &sid,
        &request_id,
        record,
        stop_reason,
        &user_text,
    );
}

fn auto_title(user_text: &str) -> String {
    let words: Vec<&str> = user_text.split_whitespace().take(5).collect();
    if words.is_empty() {
        "New chat".to_string()
    } else {
        words.join(" ")
    }
}

fn finish(
    out: &mpsc::UnboundedSender<Message>,
    state: &Shared,
    sid: &str,
    request_id: &Value,
    record: Vec<Value>,
    stop_reason: &str,
    user_text: &str,
) {
    {
        let mut s = state.lock().unwrap();
        if let Some(data) = s.sessions.get_mut(sid) {
            data.conversation.extend(record);
            data.message_count += 2;
            data.snippet = format!("Re: {user_text}");
            if data.title.is_empty() {
                data.title = auto_title(user_text);
            }
        }
    }
    let frame = json!({"jsonrpc":"2.0","id":request_id,"result":{"stopReason":stop_reason}});
    let _ = out.send(Message::Text(frame.to_string().into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|o| o["configId"].as_str().unwrap().to_string())
            .collect()
    }

    fn effort_values(v: &Value) -> usize {
        let effort = &v.as_array().unwrap()[3];
        assert_eq!(effort["configId"], "thinking_effort");
        effort["options"].as_array().unwrap().len()
    }

    /// Exactly the ids goose routes — no more, since the real agent answers
    /// `invalid_params` to anything else, and no fewer, since the app renders
    /// whatever arrives instead of naming ids of its own.
    #[test]
    fn offers_the_four_options_goose_routes() {
        let config = State::default().config;
        assert_eq!(
            ids(&config_options(&config)),
            ["provider", "mode", "model", "thinking_effort"]
        );
    }

    /// The edge case the app's fact row exists for: a model that cannot
    /// reason leaves exactly one effort to "choose" between.
    #[test]
    fn a_non_reasoning_model_collapses_effort_to_one_value() {
        let mut config = State::default().config;
        config.model = "qwen3-coder-480b".to_string();
        assert_eq!(effort_values(&config_options(&config)), 1);

        config.model = "claude-opus-5".to_string();
        assert_eq!(effort_values(&config_options(&config)), 5);
    }
}
