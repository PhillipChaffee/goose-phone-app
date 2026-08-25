//! Protocol-faithful mock of `goose serve` for exercising the app end-to-end
//! without an AI provider. Speaks ACP JSON-RPC over WebSocket at /acp with
//! the same auth surface as the real server (X-Secret-Key / ?token=, 401,
//! and the 406 auth-success probe), streams scripted turns with thinking,
//! markdown, tool calls and permission round-trips, and replays history on
//! session/load.
//!
//!   cargo run -p mock-goose-server -- [port]          (default 3285, 0 = any)
//!   `MOCK_SECRET`=...                                    (default mock-secret)
//!   `MOCK_FIXTURES`=full|empty|broken                        (default full)
//!   `MOCK_NO_SCHEDULER`=1        (goose started without --enable-scheduler)
//!   `MOCK_DROP_ALLOWLIST`=1     (a goose that drops `available_tools` too)
//!   `MOCK_SILENT`=1                    (go dead after the handshake)
//!
//! Prompt keywords: "slow" = long stream (time to hit Stop);
//! "notool" = skip the tool call / permission prompt.
//!
//! This file is the transport and nothing else: the HTTP head parse that lets
//! one listener serve both HTTP and WebSocket, auth, and the frame loop.
//! [`state`] holds what the mock knows, [`rpc`] builds frames, [`turn`]
//! scripts a prompt, and [`features`] answers requests.

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

mod features;
mod rpc;
mod state;
mod turn;

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::Message;

use crate::rpc::response_frame;
use crate::state::{seed, Shared, State};
use crate::turn::{run_turn, Pending};

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

    let state: Shared = Arc::new(Mutex::new(State::from_env()));
    seed(&state);

    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    // The bound address, not the requested one: port 0 asks the OS for a free
    // port, which is how a test binds without picking a number that a test in
    // the next file over also picked. That only works if the port comes back
    // out, so this line is the harness's input, not decoration.
    let addr = listener.local_addr().expect("local_addr");
    let (fixtures, scheduler) = {
        let s = state.lock().unwrap();
        (
            s.fixtures.label(),
            if s.no_scheduler { "off" } else { "on" },
        )
    };
    println!(
        "mock goose serve listening on http://{addr} \
         (secret: {secret}, fixtures: {fixtures}, scheduler: {scheduler})"
    );

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
                let response = features::dispatch(m, &params, &state, &out_tx);
                let frame = response_frame(&id, response);
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
