//! Protocol-faithful mock of the code-agent manager and the `OpenCode` servers
//! behind it, for exercising the app's whole Code plane without a container
//! engine or a paid API key.
//!
//!   cargo run -p mock-opencode-server -- [port]      (default 4399, 0 = any)
//!   `MOCK_CODE_PASSWORD`=...                    (default mock-code-secret)
//!   `MOCK_FIXTURES`=full|empty                             (default full)
//!   `MOCK_SILENT`=1                          (accept, then never answer)
//!
//! 4399 is not a guess: `src/state.rs`'s `dev_seed!` doc and the "Testing on
//! a simulator" section of `docs/design.md` have both named
//! `GOOSE_DEV_CODE_URL=http://127.0.0.1:4399` since before there was anything
//! to point it at.
//!
//! ONE LISTENER, TWO TIERS, because the real system is one machine: the
//! manager owns `/api/...` and proxies each chat's own `OpenCode` server at
//! `/chat/<id>/...`, so the client holds a single base URL and a single
//! password for both. Auth is HTTP Basic on every request including the event
//! stream, with the literal username `opencode`.
//!
//! This file is the transport and nothing else — the HTTP head parse, auth,
//! routing and the SSE pump. [`state`] holds what the mock knows, [`wire`]
//! spells it the way the wire does, [`routes`] answers, and [`turn`] scripts a
//! prompt.

// This binary is a test double, not shipped code: it prints its listening
// address on purpose, and an unwrap on a fixture here is a failing test rather
// than a crash on someone's phone. `mock-goose-server` takes the first three
// for those reasons and this takes them for the same ones.
//
// The rest are this crate's own, and they are all one decision: a fixture is
// arithmetic on numbers a person typed, not on numbers a server sent. Dates
// are built by subtracting hours from `now()` and converting to milliseconds,
// so `suboptimal_flops` wants a `mul_add` in the middle of a readable
// expression and the cast lints want a justification per fixture rather than
// per file. `assigning_clones` wants `clone_into` where the line says
// `status = "running"`. None of them is protecting anything here — a fixture
// that is a millisecond out is still a fixture — and taking them one at a time
// would put more `#[expect]` in this crate than code.
#![expect(
    clippy::print_stdout,
    clippy::expect_used,
    clippy::suboptimal_flops,
    clippy::assigning_clones,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::significant_drop_tightening,
    reason = "test double: stdout is its interface, and fixtures are \
              hand-written arithmetic rather than data off a wire"
)]

mod routes;
mod state;
mod turn;
mod wire;

use std::io;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};

use routes::Reply;
use state::{now_ms, State, Step};

type Shared = Arc<Mutex<State>>;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(4399);
    let state: Shared = Arc::new(Mutex::new(State::from_env()));

    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    // THE BANNER IS AN INTERFACE, not decoration: `mock-goose-server`'s tests
    // read its own banner to learn an ephemeral port, and this one is written
    // to be read the same way. It names every switch back, so a run whose
    // fixtures surprised someone says why on its first line.
    {
        let s = state.lock().await;
        println!(
            "mock code-agent manager listening on http://{addr} \
             (user: opencode, chats: {}, repos: {}, fixtures: {})",
            s.chats.len(),
            s.repos.len(),
            if s.chats.is_empty() { "empty" } else { "full" },
        );
    }

    // A closed stdin rather than a signal, for `mock-goose-server`'s reason:
    // catching SIGTERM needs tokio's `signal` feature, and a SIGKILLed process
    // writes no `.profraw`, so `cargo llvm-cov` reported 0.00% for a binary
    // seven test binaries had just driven. A server run by hand from a
    // terminal keeps stdin open and so runs until Ctrl-C.
    let (tx, mut rx) = oneshot::channel::<()>();
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = io::Read::read_to_end(&mut io::stdin(), &mut sink);
        let _ = tx.send(());
    });

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((sock, _)) = accepted else { continue };
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let _ = serve(sock, state).await;
                });
            }
            _ = &mut rx => break,
        }
    }
}

/// One connection. Keep-alive is honoured, because the client is `reqwest` with
/// a pooled connection and closing after every response would work but would
/// exercise a path the real server does not take.
async fn serve(sock: TcpStream, state: Shared) -> io::Result<()> {
    let (read, mut write) = sock.into_split();
    let mut reader = BufReader::new(read);

    loop {
        let Some((method, path, headers)) = read_head(&mut reader).await? else {
            return Ok(());
        };
        let len: usize = header(&headers, "content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        if len > 0 {
            reader.read_exact(&mut body).await?;
        }

        if std::env::var("MOCK_SILENT").is_ok() {
            // Accept and never answer, so the app's own timeout is reachable.
            return Ok(());
        }

        if !authorised(&headers, &state).await {
            // The real manager answers 401 with a WWW-Authenticate, and the
            // client turns any non-2xx into a connection failure the app shows
            // in the band.
            let reply = b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"opencode\"\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
            write.write_all(reply).await?;
            return Ok(());
        }

        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let reply = {
            let mut s = state.lock().await;
            routes::manager(&mut s, &method, &path, &parsed)
                .or_else(|| routes::chat_server(&mut s, &method, &path, &parsed))
        };

        match reply {
            Some(Reply::Json(code, value)) => {
                let text = value.to_string();
                let head = format!(
                    "HTTP/1.1 {code} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                    reason(code),
                    text.len()
                );
                write.write_all(head.as_bytes()).await?;
                write.write_all(text.as_bytes()).await?;
            }
            Some(Reply::Events(chat_id)) => {
                // The stream outlives the request and owns the connection from
                // here, so this returns rather than looping.
                return pump_events(write, state, chat_id).await;
            }
            None => {
                let head = b"HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
                write.write_all(head).await?;
            }
        }
    }
}

/// The SSE stream: `data: <json>\n\n` frames, forever.
///
/// The client declares a stream stalled if nothing arrives for 60 seconds, so
/// the heartbeat is not optional — a mock that only spoke when it had something
/// to say would be reported as a dropped connection every minute of an idle
/// window.
async fn pump_events(
    mut write: tokio::net::tcp::OwnedWriteHalf,
    state: Shared,
    chat_id: String,
) -> io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: keep-alive\r\n\r\n";
    write.write_all(head.as_bytes()).await?;
    frame(
        &mut write,
        &json!({ "type": "server.connected", "properties": {} }),
    )
    .await?;

    let mut since_beat = 0u64;
    loop {
        // Take one step, if the chat has any queued.
        let step = {
            let mut s = state.lock().await;
            let queue = s.pending.get_mut(&chat_id);
            queue.and_then(|q| {
                if q.is_empty() {
                    None
                } else {
                    Some(q.remove(0))
                }
            })
        };

        match step {
            Some(Step::Beat(ms)) => {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                since_beat = 0;
            }
            Some(Step::Message { id, role }) => {
                let session = session_of(&state, &chat_id).await;
                frame(
                    &mut write,
                    &json!({
                        "type": "message.updated",
                        "properties": { "info": {
                            "id": id,
                            "role": role,
                            "sessionID": session,
                            "time": { "created": now_ms() },
                        }},
                    }),
                )
                .await?;
            }
            Some(Step::Part { message, part }) => {
                let session = session_of(&state, &chat_id).await;
                frame(
                    &mut write,
                    &json!({
                        "type": "message.part.updated",
                        "properties": { "part": wire::part(&session, &message, &part) },
                    }),
                )
                .await?;
            }
            Some(Step::Ask(ask)) => {
                let session = session_of(&state, &chat_id).await;
                {
                    let mut s = state.lock().await;
                    if let Some(c) = s.chat_mut(&chat_id) {
                        c.asks.push(ask.clone());
                    }
                }
                frame(
                    &mut write,
                    &json!({
                        "type": "permission.updated",
                        "properties": wire::ask(&session, &ask),
                    }),
                )
                .await?;
            }
            Some(Step::Idle) => {
                let session = session_of(&state, &chat_id).await;
                {
                    let mut s = state.lock().await;
                    if let Some(c) = s.chat_mut(&chat_id) {
                        c.status = "running".to_owned();
                    }
                }
                frame(
                    &mut write,
                    &json!({
                        "type": "session.idle",
                        "properties": { "sessionID": session },
                    }),
                )
                .await?;
            }
            None => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                since_beat += 250;
                if since_beat >= 10_000 {
                    since_beat = 0;
                    frame(&mut write, &json!({ "type": "server.heartbeat" })).await?;
                }
            }
        }
    }
}

async fn session_of(state: &Shared, chat_id: &str) -> String {
    state
        .lock()
        .await
        .chat(chat_id)
        .map(|c| c.session.id.clone())
        .unwrap_or_default()
}

async fn frame(write: &mut tokio::net::tcp::OwnedWriteHalf, value: &Value) -> io::Result<()> {
    let text = format!("data: {value}\n\n");
    write.write_all(text.as_bytes()).await
}

/// The request line and headers. `None` at a clean EOF, which is a pooled
/// connection being closed and not an error.
async fn read_head(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> io::Result<Option<(String, String, Vec<(String, String)>)>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut bits = line.split_whitespace();
    let method = bits.next().unwrap_or_default().to_owned();
    let path = bits.next().unwrap_or_default().to_owned();
    if method.is_empty() || path.is_empty() {
        return Ok(None);
    }

    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).await? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_lowercase(), v.trim().to_owned()));
        }
    }
    Ok(Some((method, path, headers)))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// HTTP Basic, with the literal username `opencode`.
///
/// The real manager ignores the username and compares only what follows the
/// first colon, so this does too — a client that sent a different user with
/// the right password would be let in there and must be let in here, or the
/// mock is stricter than the thing it stands for.
async fn authorised(headers: &[(String, String)], state: &Shared) -> bool {
    let want = state.lock().await.password.clone();
    let Some(value) = header(headers, "authorization") else {
        return false;
    };
    let Some(encoded) = value.strip_prefix("Basic ") else {
        return false;
    };
    let Some(decoded) = base64_decode(encoded.trim()) else {
        return false;
    };
    decoded
        .split_once(':')
        .is_some_and(|(_user, pass)| pass == want)
}

/// Base64, decode only. A dependency for sixteen lines that the one caller
/// needs and nothing else would.
fn base64_decode(s: &str) -> Option<String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out: Vec<u8> = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for b in s.bytes() {
        if b == b'=' {
            break;
        }
        let v = TABLE.iter().position(|c| *c == b)?;
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    String::from_utf8(out).ok()
}

const fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Internal Server Error",
    }
}
