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

use std::collections::{HashMap, HashSet};
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
    /// `_goose/unstable/config/extensions/list` rows, as
    /// `{extension, enabled, configKey}`.
    config_extensions: Vec<Value>,
    /// Stored secrets, **by name only**. The real server keeps the value in
    /// `secrets.yaml`; this one deliberately does not keep it at all, so there
    /// is nothing here for a careless `config/read` to hand back.
    secret_keys: HashSet<String>,
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
            config_extensions: Vec::new(),
            secret_keys: HashSet::new(),
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

        other => extension_request(other, params, state),
    }
}

// ---------------------------------------------------------------------------
// Extensions — what the Connect screen talks to.

/// The `_goose/unstable/*` extension and config methods.
fn extension_request(method: &str, params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    match method {
        "_goose/unstable/extensions/available" => Ok(json!({"extensions": available_extensions()})),
        "_goose/unstable/config/extensions/list" => {
            let extensions = state.lock().unwrap().config_extensions.clone();
            Ok(json!({"extensions": extensions, "warnings": []}))
        }
        "_goose/unstable/config/extensions/add" => add_config_extension(params, state),
        "_goose/unstable/config/extensions/set-enabled" => set_extension_enabled(params, state),
        "_goose/unstable/config/upsert" => {
            let key = params.get("key").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() {
                return Err((-32602, "key is required".to_string()));
            }
            // Real goose logs "Secret value is not a string; skipping" and
            // starts the extension WITHOUT the credential when the value is,
            // say, a numeric app password that got parsed as a number. Same
            // here: the key is only remembered when a string arrives, so a
            // client with that bug fails the handshake below instead of
            // appearing to work.
            if params.get("value").and_then(Value::as_str).is_some() {
                state.lock().unwrap().secret_keys.insert(key.to_string());
            }
            Ok(json!({}))
        }
        "_goose/unstable/session/extensions/add" => add_session_extension(params, state),
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

fn set_extension_enabled(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let key = params
        .get("configKey")
        .and_then(Value::as_str)
        .unwrap_or("");
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let found = {
        let mut s = state.lock().unwrap();
        s.config_extensions
            .iter_mut()
            .find(|e| e["configKey"].as_str() == Some(key))
            .map(|entry| entry["enabled"] = Value::Bool(enabled))
            .is_some()
    };
    if found {
        Ok(json!({}))
    } else {
        Err((-32602, format!("Extension '{key}' not found")))
    }
}

/// Bring an extension up in a live session — the handshake the app uses to
/// prove a credential without ever reading one back.
fn add_session_extension(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let extension = params.get("extension").cloned().unwrap_or(Value::Null);

    // Inline env values are refused at the session level by the real server,
    // with this wording.
    let has_inline_env = extension
        .pointer("/server/env")
        .and_then(Value::as_array)
        .is_some_and(|env| !env.is_empty());
    if has_inline_env {
        return Err((
            -32602,
            "extension env values must be passed via envKeys referencing stored \
             secrets, not inline env"
                .to_string(),
        ));
    }

    // goose launches the MCP server here, so a declared env key with no stored
    // secret is a hard startup failure.
    let declared: Vec<String> = extension
        .get("envKeys")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let missing: Vec<String> = {
        let s = state.lock().unwrap();
        declared
            .into_iter()
            .filter(|k| !s.secret_keys.contains(k))
            .collect()
    };
    if missing.is_empty() {
        Ok(json!({}))
    } else {
        Err((
            -32603,
            format!(
                "failed to start extension `{}`: missing env {}",
                extension
                    .pointer("/server/name")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                missing.join(", ")
            ),
        ))
    }
}

/// goose's own catalogue, which is unrestricted: these come back with no
/// `available_tools` at all. Reproduced faithfully — a mock that invented
/// allowlists here would hide the fact that enabling a built-in is a
/// different decision from adding a scoped connector.
fn available_extensions() -> Value {
    json!([
        {"type": "builtin", "name": "developer", "display_name": "Developer",
         "description": "Shell, file editing and text tools", "bundled": true},
        {"type": "builtin", "name": "computercontroller", "display_name": "Computer Controller",
         "description": "Web scraping, automation and file caching", "bundled": true},
        {"type": "platform", "name": "memory", "display_name": "Memory",
         "description": "Remembers facts across sessions", "bundled": true},
    ])
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

/// Persist an extension, reproducing the one behaviour that makes the client's
/// read-back necessary: only the `snake_case` `available_tools` key is read.
///
/// A camelCase `availableTools` is left in the stored object and ignored,
/// exactly as goose ignores it — goose sets no `deny_unknown_fields` — so the
/// extension ends up with an empty allowlist, which means every tool is
/// allowed. Setting `MOCK_DROP_ALLOWLIST=1` drops the correct spelling too,
/// simulating a server that has moved the field, so the app's hard error can
/// be exercised by hand.
fn add_config_extension(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let mut extension = params.get("extension").cloned().unwrap_or(Value::Null);
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let Some(obj) = extension.as_object_mut() else {
        return Err((-32602, "extension must be an object".to_string()));
    };
    if std::env::var("MOCK_DROP_ALLOWLIST").is_ok_and(|v| v == "1") {
        obj.remove("available_tools");
    }
    // goose stores the allowlist as `Vec<String>`, so an absent field and an
    // empty one are the same thing on the way back out: omitted entirely.
    let tools = obj
        .get("available_tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if tools.is_empty() {
        obj.remove("available_tools");
    } else {
        obj.insert("available_tools".to_string(), Value::Array(tools));
    }

    let name = extension
        .pointer("/server/name")
        .or_else(|| extension.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return Err((-32602, "extension has no name".to_string()));
    }

    let row = json!({
        "extension": extension,
        "enabled": enabled,
        "configKey": name_to_key(&name),
    });
    let mut s = state.lock().unwrap();
    s.config_extensions.retain(|e| {
        e["extension"]
            .pointer("/server/name")
            .and_then(Value::as_str)
            != Some(&name)
            && e["extension"].get("name").and_then(Value::as_str) != Some(&name)
    });
    s.config_extensions.push(row);
    drop(s);
    Ok(json!({}))
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

    let slow = user_text.to_lowercase().contains("slow");
    let with_tool = !user_text.to_lowercase().contains("notool");
    let mut turn = Turn {
        out,
        sid,
        cancel,
        delay: Duration::from_millis(if slow { 400 } else { 150 }),
        record: Vec::new(),
    };

    // Real goose only replays user chunks on session/load — it does NOT echo
    // them during a live turn. Record for replay without emitting.
    turn.record.push(
        json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":user_text}}),
    );
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
    use goose_acp_client::{GooseExtension, McpServer, StdioMcpServer};

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

    // ---- the extension surface ---------------------------------------------
    //
    // Fed the JSON the real client produces. The point is fidelity in one
    // specific direction: the mock must reproduce goose's *failure*, not an
    // idealised version of it. If it silently kept a camelCase allowlist, every
    // test that ran against it would pass while the app shipped the bug this
    // whole surface exists to prevent.

    fn mail() -> GooseExtension {
        GooseExtension::mcp(
            McpServer::Stdio(StdioMcpServer::new(
                "mail-imap",
                "uvx",
                vec!["stdio".into()],
            )),
            vec!["MCP_EMAIL_SERVER_PASSWORD".into()],
            "IMAP mail",
            vec!["list_mailboxes".into(), "get_emails_content".into()],
        )
    }

    fn add(state: &Shared, extension: &Value) -> Result<Value, (i64, String)> {
        extension_request(
            "_goose/unstable/config/extensions/add",
            &json!({"extension": extension, "enabled": true}),
            state,
        )
    }

    fn listed(state: &Shared) -> Value {
        extension_request(
            "_goose/unstable/config/extensions/list",
            &Value::Null,
            state,
        )
        .unwrap()
    }

    #[test]
    fn a_snake_case_allowlist_is_kept() {
        let state: Shared = Arc::default();
        add(&state, &serde_json::to_value(mail()).unwrap()).unwrap();

        let rows = listed(&state);
        assert_eq!(rows["extensions"][0]["configKey"], json!("mail-imap"));
        assert_eq!(rows["extensions"][0]["enabled"], json!(true));
        assert_eq!(
            rows["extensions"][0]["extension"]["available_tools"],
            json!(["list_mailboxes", "get_emails_content"])
        );
    }

    #[test]
    fn a_camel_case_allowlist_is_dropped_exactly_as_goose_drops_it() {
        let state: Shared = Arc::default();
        let mut extension = serde_json::to_value(mail()).unwrap();
        let obj = extension.as_object_mut().unwrap();
        let tools = obj.remove("available_tools").unwrap();
        obj.insert("availableTools".to_string(), tools);

        add(&state, &extension).unwrap();

        let rows = listed(&state);
        let stored = &rows["extensions"][0]["extension"];
        assert!(
            stored.get("available_tools").is_none(),
            "the camelCase spelling must not become an allowlist: {stored}"
        );
    }

    /// A credential is proved by starting the extension, never by reading the
    /// value back — so a missing secret has to be an error here.
    #[test]
    fn a_session_add_fails_until_the_secret_is_stored() {
        let state: Shared = Arc::default();
        let extension = serde_json::to_value(mail()).unwrap();
        let params = json!({"sessionId": "20260821_1", "extension": extension});

        let err = extension_request("_goose/unstable/session/extensions/add", &params, &state)
            .unwrap_err();
        assert!(
            err.1.contains("MCP_EMAIL_SERVER_PASSWORD"),
            "got: {}",
            err.1
        );

        extension_request(
            "_goose/unstable/config/upsert",
            &json!({"key": "MCP_EMAIL_SERVER_PASSWORD", "value": "pw", "isSecret": true}),
            &state,
        )
        .unwrap();
        extension_request("_goose/unstable/session/extensions/add", &params, &state).unwrap();
    }

    /// goose logs "Secret value is not a string; skipping" for a value that
    /// arrived as a number — an app password of all digits is the realistic
    /// case — and then starts the extension with no credential at all.
    #[test]
    fn a_non_string_secret_is_skipped_and_the_extension_will_not_start() {
        let state: Shared = Arc::default();
        extension_request(
            "_goose/unstable/config/upsert",
            &json!({"key": "MCP_EMAIL_SERVER_PASSWORD", "value": 12_345_678, "isSecret": true}),
            &state,
        )
        .unwrap();

        let err = extension_request(
            "_goose/unstable/session/extensions/add",
            &json!({"sessionId": "s", "extension": serde_json::to_value(mail()).unwrap()}),
            &state,
        )
        .unwrap_err();
        assert!(err.1.contains("missing env"), "got: {}", err.1);
    }

    #[test]
    fn toggling_an_unknown_extension_is_an_error() {
        let state: Shared = Arc::default();
        assert!(extension_request(
            "_goose/unstable/config/extensions/set-enabled",
            &json!({"configKey": "nope", "enabled": true}),
            &state,
        )
        .is_err());
    }
}
