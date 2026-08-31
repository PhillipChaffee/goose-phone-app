//! Every route this client speaks, end to end against a stub gateway.
//!
//! `tests/pulls.rs` does this for the two pull-request routes; this file does
//! it for the rest, and for the same reason. A decode test pins what the client
//! does with an answer it was handed. It cannot pin **which question was
//! asked** — and that is where this client's mistakes would be: a manager route
//! written as a `/chat/<id>/` one wakes a container, a repo name spliced into a
//! path unencoded builds a request line the gateway rejects, a `purge` flag
//! dropped from the query string deletes a chat and silently keeps its
//! workspace. Every test below asserts the request line the stub saw as well as
//! the value that came back.
//!
//! The second stub — `Mode::Garbled` — answers every route 200 with a body no
//! version of this contract has ever produced. That is the shape a captive
//! portal or a half-deployed manager sends, and each route has a documented
//! answer for it: an empty list, `None`, or an error. Those answers are the
//! difference between a screen that shows less than it could and a screen that
//! lies, so they are asserted one route at a time.

// Test code: a failing unwrap, or a panic on the wrong variant, IS the failing
// check. Both are denied for shipped code. `expect` rather than `allow`: if a
// use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: an unwrap, an expect or a wrong-variant panic is the assertion"
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencode_client::{
    resolve_agent, ChatMeta, CodeClient, CodeConfig, CodeError, CodeEvent, FileStatus, ModelInfo,
    PromptPart,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PASSWORD: &str = "stub-password";
const CHAT: &str = "notes-9f2c1a";

/// One request the stub saw: `METHOD PATH`, and the body that came with it.
#[derive(Clone, Debug)]
struct Seen {
    line: String,
    body: String,
}

type Log = Arc<Mutex<Vec<Seen>>>;

/// Which gateway the client is talking to.
#[derive(Clone, Copy)]
enum Mode {
    /// The contract, as the manager is asked to implement it.
    Contract,
    /// A 200 carrying a body outside every shape this client knows — a captive
    /// portal, a proxy error page, a manager mid-deploy.
    Garbled,
}

// --------------------------------------------------------------- fixtures

fn chats() -> Value {
    json!({"chats": [
        {
            "id": CHAT, "repo": "notes", "title": "Tighten the quickstart",
            "branch": "agent/notes-9f2c1a", "base": "main", "status": "running",
            "model": "opencode/deepseek-v4", "last_active": 1_756_000_000.0
        },
        {"id": "scratch-1", "repo": "scratch", "status": "stopped"}
    ]})
}

fn providers() -> Value {
    json!({"providers": [{"id": "opencode", "models": {
        "deepseek-v4-flash": {"name": "DeepSeek V4 Flash", "limit": {"context": 163_840.0}},
        "claude-sonnet-4-5": {
            "name": "Claude Sonnet 4.5", "limit": {"context": 200_000.0},
            "variants": {"high": {}, "low": {}}
        }
    }}]})
}

fn agents() -> Value {
    json!([
        {"name": "build", "description": "Writes code.", "mode": "primary", "builtIn": true},
        {"name": "reviewer", "mode": "subagent", "builtIn": false},
        // No name: the name IS what the prompt body sends, so this is not an
        // agent anything could be run as.
        {"name": "", "mode": "primary"},
        // Undecodable on its own; must not take the list with it.
        {"name": 5}
    ])
}

fn diff() -> Value {
    json!([
        {
            "path": "src/lib.rs",
            "patch": "Index: src/lib.rs\n@@ -1,2 +1,3 @@\n a\n+b\n",
            "additions": 1, "deletions": 0, "status": "modified"
        },
        {"file": "assets/logo.png", "status": "added"},
        {"file": "gone.txt", "status": "deleted", "patch": "Index: gone.txt\n-x\n", "deletions": 1},
        {"file": 42}
    ])
}

/// The manager half of the contract.
fn manager_answer(method: &str, path: &str) -> Option<(&'static str, Value)> {
    Some(match (method, path) {
        ("GET", "/api/health") => ("200 OK", json!({"ok": true, "chats": 2})),
        ("GET", "/api/repos") => (
            "200 OK",
            json!({"repos": [
                {
                    "name": "notes", "url": "git@github.com:me/notes.git",
                    "edit_only": true, "allow_push": true, "public_throwaway": false
                },
                {"name": "scratch"}
            ]}),
        ),
        // The repo name arrives percent-encoded or not at all: the stub is
        // matching the literal request line the client built.
        ("GET", "/api/repos/shopping%20list/branches") => (
            "200 OK",
            json!({
                "default": "main", "truncated": true,
                "branches": [{"name": "main", "default": true}, {"name": "wip", "default": false}]
            }),
        ),
        ("GET", "/api/chats") => ("200 OK", chats()),
        ("GET", "/api/permissions") => (
            "200 OK",
            json!({
                "permissions": [{
                    "chatId": CHAT, "id": "per_1", "sessionID": "ses_1",
                    "type": "bash", "title": "Run git push",
                    "metadata": {"command": "git push"}
                }],
                "unreachable": ["scratch-1"]
            }),
        ),
        ("POST", "/api/chats") => (
            "200 OK",
            json!({"id": "made-1", "repo": "notes", "title": "new", "status": "running"}),
        ),
        // 204: the manager has nothing to say about a wake that worked.
        ("POST", "/api/chats/notes-9f2c1a/wake") => ("204 No Content", Value::Null),
        ("POST", "/api/chats/ghost/wake") => ("404 Not Found", json!({"error": "unknown chat"})),
        ("POST", "/api/chats/notes-9f2c1a/stop") => ("200 OK", json!({"status": "stopped"})),
        ("DELETE", "/api/chats/notes-9f2c1a" | "/api/chats/notes-9f2c1a?purge=1") => {
            ("200 OK", json!({"deleted": true}))
        }
        _ => return None,
    })
}

/// The per-chat half: everything proxied through to a chat's own server.
fn chat_answer(method: &str, path: &str) -> (&'static str, Value) {
    match (method, path) {
        ("GET", "/chat/notes-9f2c1a/config") => (
            "200 OK",
            json!({"model": "opencode/deepseek-v4-flash", "$schema": "https://opencode.ai"}),
        ),
        ("GET", "/chat/notes-9f2c1a/session") => (
            "200 OK",
            json!([{
                "id": "ses_1", "title": "Fix the quickstart", "directory": "/chat/workspace",
                "model": {"id": "deepseek-v4", "providerID": "opencode", "variant": "high"},
                "agent": "plan"
            }]),
        ),
        ("POST", "/chat/notes-9f2c1a/session?directory=/chat/workspace") => (
            "200 OK",
            json!({"id": "ses_new", "title": "", "directory": "/chat/workspace"}),
        ),
        ("GET", "/chat/notes-9f2c1a/config/providers") => ("200 OK", providers()),
        // A build that has only the newer route: the older one is not there.
        ("GET", "/chat/onlynew/config/providers") => {
            ("404 Not Found", json!({"error": "no such route"}))
        }
        ("GET", "/chat/onlynew/provider") => (
            "200 OK",
            json!({"all": [{"id": "openai", "models": {"gpt-5.2": {}}}], "connected": []}),
        ),
        // Neither route holds a catalogue, and the fallback fails outright.
        ("GET", "/chat/nomodels/config/providers") => ("200 OK", json!({"providers": []})),
        ("GET", "/chat/nomodels/provider") => (
            "500 Internal Server Error",
            json!({"error": "provider registry is down"}),
        ),
        ("GET", "/chat/notes-9f2c1a/agent") => ("200 OK", agents()),
        ("GET", "/chat/notes-9f2c1a/session/ses_1/message") => (
            "200 OK",
            json!([{
                "info": {"id": "msg_1", "role": "user", "sessionID": "ses_1"},
                "parts": [
                    {"id": "prt_1", "messageID": "msg_1", "type": "text", "text": "hello"},
                    {
                        "id": "prt_2", "messageID": "msg_1", "type": "file",
                        "mime": "image/jpeg", "filename": "IMG_0042.jpg",
                        "url": "data:image/jpeg;base64,QUJD"
                    }
                ]
            }]),
        ),
        (
            "POST",
            "/chat/notes-9f2c1a/session/ses_1/prompt_async"
            | "/chat/notes-9f2c1a/session/ses_1/abort"
            | "/chat/notes-9f2c1a/session/ses_1/permissions/per_1",
        ) => ("200 OK", json!({})),
        ("GET", "/chat/notes-9f2c1a/session/ses_1/diff") => ("200 OK", diff()),
        // A refusal whose body is far longer than anything worth carrying into
        // a toast.
        ("GET", "/chat/notes-9f2c1a/session/loud/diff") => (
            "500 Internal Server Error",
            json!({"error": "x".repeat(2000)}),
        ),
        ("GET", "/chat/notes-9f2c1a/permission") => (
            "200 OK",
            json!([{
                "id": "per_1", "sessionID": "ses_1", "type": "bash",
                "title": "Run git push", "metadata": {"command": "git push"}
            }]),
        ),
        _ => (
            "404 Not Found",
            json!({"error": format!("no route: {method} {path}")}),
        ),
    }
}

// ------------------------------------------------------------------- stub

/// The SSE stream `/chat/<id>/event` serves, cut in two so a frame straddles
/// the chunk boundary — the case the read loop's buffer exists for.
/// How many frames `/chat/chatty/event` would send if nobody hung up.
const CHATTY_FRAMES: u32 = 200;

const SSE_HEAD: &str = concat!(
    "event: message\ndata: {\"type\":\"server.connected\",\"properties\":{}}\n\n",
    "data: {\"type\":\"server.heartbeat\"}\n\n",
    "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":",
);
const SSE_TAIL: &str = concat!(
    "{\"id\":\"prt_1\",\"type\":\"text\",\"text\":\"Hello\"},\"delta\":\"Hello\"}}\n\n",
    "data: {\"type\":\"session.idle\",\"properties\":{\"sessionID\":\"ses_1\"}}\n\n",
);

/// A stream that never stops on its own, so the only thing that can end it is
/// the client walking away. The number of frames the stub got out before the
/// socket refused them goes into the log.
async fn serve_chatty(sock: &mut TcpStream, log: &Log) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\nConnection: close\r\n\r\n";
    let _ = sock.write_all(head.as_bytes()).await;
    let mut written = 0u32;
    for i in 0..CHATTY_FRAMES {
        let frame = format!(
            "data: {{\"type\":\"session.idle\",\"properties\":{{\"sessionID\":\"ses_{i}\"}}}}\n\n"
        );
        if sock.write_all(frame.as_bytes()).await.is_err() || sock.flush().await.is_err() {
            break;
        }
        written += 1;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    log.lock().unwrap().push(Seen {
        line: format!("sse-frames-written {written}"),
        body: String::new(),
    });
}

async fn serve_sse(sock: &mut TcpStream, path: &str) {
    if path.starts_with("/chat/dead/") {
        let body = json!({"error": "chat container is gone"}).to_string();
        let resp = format!(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        return;
    }
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\nConnection: close\r\n\r\n";
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(SSE_HEAD.as_bytes()).await;
    let _ = sock.flush().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = sock.write_all(SSE_TAIL.as_bytes()).await;
    let _ = sock.flush().await;
    // Closing the socket is how a chat spinning down ends its stream.
}

async fn handle(mut sock: TcpStream, log: Log, mode: Mode) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
        match sock.read(&mut byte).await {
            Ok(0) | Err(_) => return,
            Ok(_) => head.push(byte[0]),
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut start = text.lines().next().unwrap_or_default().split_whitespace();
    let method = start.next().unwrap_or_default().to_string();
    let path = start.next().unwrap_or_default().to_string();
    let length: usize = text
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        let _ = sock.read_exact(&mut body).await;
    }
    log.lock().unwrap().push(Seen {
        line: format!("{method} {path}"),
        body: String::from_utf8_lossy(&body).to_string(),
    });

    if path == "/chat/chatty/event" {
        serve_chatty(&mut sock, &log).await;
        return;
    }
    if path.ends_with("/event") {
        serve_sse(&mut sock, &path).await;
        return;
    }
    let (code, body) = match mode {
        Mode::Garbled => ("200 OK", json!("<html>sign in to the tailnet</html>")),
        Mode::Contract => {
            manager_answer(&method, &path).unwrap_or_else(|| chat_answer(&method, &path))
        }
    };
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let resp = format!(
        "HTTP/1.1 {code}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
}

async fn stub(mode: Mode) -> (CodeClient, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let spawned = Arc::clone(&log);
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(handle(sock, Arc::clone(&spawned), mode));
        }
    });
    let client = CodeClient::new(&CodeConfig {
        // The trailing slash a hand-typed setting arrives with: joined onto a
        // path that already starts with one it would make `//api/health`.
        base_url: format!("http://{addr}/"),
        password: PASSWORD.to_owned(),
    })
    .unwrap();
    (client, log)
}

fn lines(log: &Log) -> Vec<String> {
    log.lock().unwrap().iter().map(|s| s.line.clone()).collect()
}

fn bodies(log: &Log) -> Vec<Value> {
    log.lock()
        .unwrap()
        .iter()
        .map(|s| serde_json::from_str(&s.body).unwrap_or(Value::Null))
        .collect()
}

// -------------------------------------------------------- manager routes

/// A base URL is a setting somebody types, so it arrives with the trailing
/// slash a browser would have added — and every path in this client starts with
/// one of its own. Left unstripped, every request in the app goes to `//api/…`.
#[tokio::test]
async fn a_typed_base_url_keeps_its_trailing_slash_out_of_every_path() {
    let (client, log) = stub(Mode::Contract).await;
    let health = client.health().await.unwrap();
    assert_eq!(lines(&log), ["GET /api/health"]);
    assert_eq!(health["ok"], json!(true));
}

/// A URL that is only whitespace is not a server, and building a client from
/// one would turn every later call into a confusing transport error instead of
/// the one thing the settings screen can act on.
#[test]
fn a_blank_base_url_is_refused_at_construction() {
    match CodeClient::new(&CodeConfig {
        base_url: "   \n".to_owned(),
        password: "x".to_owned(),
    }) {
        Err(CodeError::Other(msg)) => assert_eq!(msg, "code server URL is empty"),
        other => panic!("expected the empty-URL refusal, got {other:?}"),
    }
}

/// The allowlist's flags decide what the new-chat screen offers: an `edit_only`
/// repo gets no push, and losing a flag in the decode silently widens what the
/// agent is allowed to do with somebody's repository.
#[tokio::test]
async fn the_repo_allowlist_decodes_with_every_flag_it_carries() {
    let (client, log) = stub(Mode::Contract).await;
    let repos = client.repos().await.unwrap();
    assert_eq!(lines(&log), ["GET /api/repos"]);
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[0].name, "notes");
    assert_eq!(repos[0].url, "git@github.com:me/notes.git");
    assert!(repos[0].edit_only);
    assert!(repos[0].allow_push);
    assert!(!repos[0].public_throwaway);
    assert_eq!(
        repos[1].url, "",
        "a repo the manager sent nothing but a name for is still a repo"
    );
    assert!(!repos[1].allow_push, "an absent flag is not a granted one");
}

/// The repo name goes in the path. It comes from the owner's own allowlist, so
/// it is trusted — but a space in one builds a request line with three tokens,
/// which the gateway answers 400 rather than routing.
#[tokio::test]
async fn a_repo_name_reaches_the_wire_percent_encoded() {
    let (client, log) = stub(Mode::Contract).await;
    let branches = client.branches("shopping list").await.unwrap();
    assert_eq!(
        lines(&log),
        ["GET /api/repos/shopping%20list/branches"],
        "an unencoded space would make this request line unparseable"
    );
    assert_eq!(branches.default_name(), Some("main"));
    assert_eq!(branches.names(), ["main", "wip"]);
    assert!(
        branches.truncated,
        "the picker says so above the rows; losing the flag makes a short list look complete"
    );
}

/// The index is what the chat list draws from, and `status` is what decides
/// whether a row is shown as awake.
#[tokio::test]
async fn the_chat_index_decodes_with_its_status_and_model() {
    let (client, log) = stub(Mode::Contract).await;
    let chats = client.chats().await.unwrap();
    assert_eq!(lines(&log), ["GET /api/chats"]);
    let [running, stopped] = chats.as_slice() else {
        panic!("expected two chats, got {chats:?}")
    };
    assert_eq!(running.id, CHAT);
    assert_eq!(running.title, "Tighten the quickstart");
    assert_eq!(running.branch, "agent/notes-9f2c1a");
    assert_eq!(running.base, "main");
    assert!(running.is_running());
    assert_eq!(running.model.as_deref(), Some("opencode/deepseek-v4"));
    assert!((running.last_active - 1_756_000_000.0).abs() < f64::EPSILON);

    assert!(!stopped.is_running());
    assert_eq!(stopped.model, None);
    assert_eq!(
        stopped.base, "",
        "a chat made before the base picker existed names no base"
    );
}

/// The aggregate is the one route that must not be per-chat: `/chat/<id>/…`
/// goes through the wake-on-request proxy, so polling it once per chat would
/// hold every container open and undo the idle spin-down.
#[tokio::test]
async fn the_pending_asks_come_off_one_manager_route() {
    let (client, log) = stub(Mode::Contract).await;
    let report = client.pending_permissions().await.unwrap();
    assert_eq!(lines(&log), ["GET /api/permissions"]);
    assert_eq!(report.permissions.len(), 1);
    assert_eq!(report.permissions[0].chat_id, CHAT);
    assert_eq!(report.permissions[0].permission.title, "Run git push");
    assert_eq!(
        report.unreachable,
        ["scratch-1"],
        "a container that did not answer must be named, not read as having nothing pending"
    );
}

/// The three optional halves of a new chat. A model or a base sent as `""`
/// would be a model and a branch the manager goes looking for and does not
/// find, so nothing chosen means nothing sent.
#[tokio::test]
async fn creating_a_chat_sends_only_the_fields_that_were_chosen() {
    let (client, log) = stub(Mode::Contract).await;
    let made = client
        .create_chat(
            "notes",
            "tighten it",
            Some("opencode/deepseek-v4"),
            Some("dev"),
        )
        .await
        .unwrap();
    assert_eq!(made.id, "made-1");

    client
        .create_chat("notes", "tighten it", None, Some(""))
        .await
        .unwrap();

    assert_eq!(lines(&log), ["POST /api/chats", "POST /api/chats"]);
    let sent = bodies(&log);
    assert_eq!(
        sent[0],
        json!({
            "repo": "notes", "task": "tighten it",
            "model": "opencode/deepseek-v4", "base": "dev"
        })
    );
    assert_eq!(
        sent[1],
        json!({"repo": "notes", "task": "tighten it"}),
        "an empty base is a branch name the manager would go looking for"
    );
}

/// A wake that worked has nothing to report, and the manager says so with a
/// 204. Reading a body that is not there must not turn a success into an error.
#[tokio::test]
async fn a_wake_answered_with_no_content_still_succeeds() {
    let (client, log) = stub(Mode::Contract).await;
    client.wake_chat(CHAT).await.unwrap();
    client.stop_chat(CHAT).await.unwrap();
    assert_eq!(
        lines(&log),
        [
            "POST /api/chats/notes-9f2c1a/wake",
            "POST /api/chats/notes-9f2c1a/stop"
        ]
    );
}

#[tokio::test]
async fn waking_a_chat_the_manager_does_not_know_is_a_404() {
    let (client, _log) = stub(Mode::Contract).await;
    match client.wake_chat("ghost").await {
        Err(e @ CodeError::Status { status: 404, .. }) => assert_eq!(e.message(), "unknown chat"),
        other => panic!("expected 404, got {other:?}"),
    }
}

/// `purge` is the difference between "this conversation is over" and "the work
/// is gone". It rides the query string, so dropping it fails silently: the
/// delete still succeeds and the workspace it was meant to discard survives.
#[tokio::test]
async fn purging_a_chat_says_so_in_the_query_string() {
    let (client, log) = stub(Mode::Contract).await;
    client.delete_chat(CHAT, true).await.unwrap();
    client.delete_chat(CHAT, false).await.unwrap();
    assert_eq!(
        lines(&log),
        [
            "DELETE /api/chats/notes-9f2c1a?purge=1",
            "DELETE /api/chats/notes-9f2c1a"
        ]
    );
}

// ------------------------------------------------------- per-chat routes

/// The resolved config is the only place the model exists for a chat created
/// without one: its own record says `null` until a turn has been sent.
#[tokio::test]
async fn the_chats_resolved_config_names_the_model_it_runs() {
    let (client, log) = stub(Mode::Contract).await;
    let model = client.default_model(CHAT).await.unwrap();
    assert_eq!(lines(&log), ["GET /chat/notes-9f2c1a/config"]);
    assert_eq!(model.as_deref(), Some("opencode/deepseek-v4-flash"));
}

#[tokio::test]
async fn a_session_arrives_with_the_model_and_agent_its_last_turn_used() {
    let (client, log) = stub(Mode::Contract).await;
    let sessions = client.sessions(CHAT).await.unwrap();
    assert_eq!(lines(&log), ["GET /chat/notes-9f2c1a/session"]);
    let [only] = sessions.as_slice() else {
        panic!("expected one session, got {sessions:?}")
    };
    assert_eq!(only.id, "ses_1");
    assert_eq!(only.title, "Fix the quickstart");
    assert_eq!(only.directory, "/chat/workspace");
    assert_eq!(only.agent(), Some("plan"));
    let model = only.model.clone().unwrap();
    assert_eq!(model.reference().as_deref(), Some("opencode/deepseek-v4"));
    assert_eq!(model.effort(), Some("high"));
}

/// The workspace is not the container's home directory, and a session created
/// anywhere else looks at the wrong tree — no files, no diff, no repo.
#[tokio::test]
async fn a_new_session_is_created_in_the_chats_workspace() {
    let (client, log) = stub(Mode::Contract).await;
    let session = client.create_session(CHAT).await.unwrap();
    assert_eq!(
        lines(&log),
        ["POST /chat/notes-9f2c1a/session?directory=/chat/workspace"]
    );
    assert_eq!(session.id, "ses_new");
}

/// The catalogue the model sheet renders, with each entry's provider filled in
/// from the provider that offered it — the models map keys them by id and
/// mostly leaves `providerID` off, and a reference missing its provider is not
/// something `prompt_async` can send.
#[tokio::test]
async fn the_model_catalogue_comes_off_the_older_route_when_it_answers() {
    let (client, log) = stub(Mode::Contract).await;
    let models = client.models(CHAT).await.unwrap();
    assert_eq!(
        lines(&log),
        ["GET /chat/notes-9f2c1a/config/providers"],
        "the newer route must not be asked once the older one has answered"
    );
    let refs: Vec<String> = models.iter().map(ModelInfo::reference).collect();
    assert_eq!(
        refs,
        ["opencode/claude-sonnet-4-5", "opencode/deepseek-v4-flash"],
        "ordered by the name a reader sees, not by the map key"
    );
    assert_eq!(models[0].limit.context_tokens(), Some(200_000));
    assert_eq!(
        models[0].efforts(),
        ["low", "high"],
        "weakest first: alphabetical would put high before low"
    );
    assert!(models[1].efforts().is_empty());
}

/// The container tracks a rolling `:latest` tag, so which of the two catalogue
/// routes exists is not something this client can know in advance. A build with
/// only the newer one has to work.
#[tokio::test]
async fn a_build_with_only_the_newer_catalogue_route_still_answers() {
    let (client, log) = stub(Mode::Contract).await;
    let models = client.models("onlynew").await.unwrap();
    assert_eq!(
        lines(&log),
        [
            "GET /chat/onlynew/config/providers",
            "GET /chat/onlynew/provider"
        ]
    );
    let [only] = models.as_slice() else {
        panic!("expected one model, got {models:?}")
    };
    assert_eq!(only.reference(), "openai/gpt-5.2");
    assert_eq!(
        only.name, "gpt-5.2",
        "a model the server named nothing is labelled with its id, not blank"
    );
}

/// When neither route holds a catalogue and the last one failed outright, the
/// failure is what the caller gets — an empty list would read as "this chat
/// offers no models", which is a different and wrong sentence.
#[tokio::test]
async fn both_catalogue_routes_failing_surfaces_the_last_failure() {
    let (client, _log) = stub(Mode::Contract).await;
    match client.models("nomodels").await {
        Err(e @ CodeError::Status { status: 500, .. }) => {
            assert_eq!(e.message(), "provider registry is down");
        }
        other => panic!("expected the fallback route's 500, got {other:?}"),
    }
}

/// One agent definition this client cannot read must not empty the picker, and
/// an agent with no name cannot be sent in a prompt body at all.
#[tokio::test]
async fn the_agent_list_drops_only_the_entries_it_cannot_use() {
    let (client, log) = stub(Mode::Contract).await;
    let list = client.agents(CHAT).await.unwrap();
    assert_eq!(lines(&log), ["GET /chat/notes-9f2c1a/agent"]);
    let names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["build", "reviewer"]);
    assert!(list[0].is_primary());
    assert_eq!(list[0].description.as_deref(), Some("Writes code."));
    assert!(
        !list[1].is_primary(),
        "a subagent is invoked by another agent, never picked by a person"
    );
    assert_eq!(resolve_agent(None, &list), Some("build"));
}

/// The transcript's own source. Both part shapes have to survive the trip:
/// text, and the file part a re-opened chat gets its thumbnails back from.
#[tokio::test]
async fn a_sessions_messages_arrive_with_their_parts() {
    let (client, log) = stub(Mode::Contract).await;
    let messages = client.messages(CHAT, "ses_1").await.unwrap();
    assert_eq!(
        lines(&log),
        ["GET /chat/notes-9f2c1a/session/ses_1/message"]
    );
    let [only] = messages.as_slice() else {
        panic!("expected one message, got {messages:?}")
    };
    assert_eq!(only.info.id, "msg_1");
    assert_eq!(only.info.role, "user");
    assert_eq!(only.info.session_id, "ses_1");
    assert_eq!(only.parts.len(), 2);
    assert_eq!(only.parts[0].text.as_deref(), Some("hello"));
    assert_eq!(only.parts[1].filename.as_deref(), Some("IMG_0042.jpg"));
    assert_eq!(only.parts[1].data_url_base64(), Some("QUJD"));
}

/// The turn's three riders reach the wire in `OpenCode`'s own shape — a split
/// `providerID`/`modelID` pair rather than the `provider/model` string the app
/// speaks internally.
#[tokio::test]
async fn a_prompt_puts_its_parts_model_and_agent_on_the_wire() {
    let (client, log) = stub(Mode::Contract).await;
    client
        .prompt_async(
            CHAT,
            "ses_1",
            &[PromptPart::text("ship it")],
            Some("opencode/claude-sonnet-4-5"),
            Some("high"),
            Some("plan"),
        )
        .await
        .unwrap();
    assert_eq!(
        lines(&log),
        ["POST /chat/notes-9f2c1a/session/ses_1/prompt_async"]
    );
    assert_eq!(
        bodies(&log)[0],
        json!({
            "parts": [{"type": "text", "text": "ship it"}],
            "model": {"providerID": "opencode", "modelID": "claude-sonnet-4-5"},
            "variant": "high",
            "agent": "plan"
        })
    );
}

/// A message with nothing in it is a 400 from the server and a woken container
/// for nothing. It is refused here, before the request exists.
#[tokio::test]
async fn an_empty_prompt_never_reaches_the_gateway() {
    let (client, log) = stub(Mode::Contract).await;
    match client
        .prompt_async(CHAT, "ses_1", &[], None, None, None)
        .await
    {
        Err(CodeError::Other(msg)) => assert_eq!(msg, "prompt has no content"),
        other => panic!("expected the empty-prompt refusal, got {other:?}"),
    }
    assert!(
        lines(&log).is_empty(),
        "an empty prompt must not wake a container to be told no"
    );
}

#[tokio::test]
async fn aborting_posts_to_the_sessions_own_abort() {
    let (client, log) = stub(Mode::Contract).await;
    client.abort(CHAT, "ses_1").await.unwrap();
    assert_eq!(lines(&log), ["POST /chat/notes-9f2c1a/session/ses_1/abort"]);
}

/// One entry this client cannot read is one file missing from the review
/// screen; a whole-array decode would make it "no changes", which is the same
/// thing the screen says when the agent has done nothing.
#[tokio::test]
async fn the_diff_drops_a_bad_entry_rather_than_the_whole_answer() {
    let (client, log) = stub(Mode::Contract).await;
    let files = client.diff(CHAT, "ses_1").await.unwrap();
    assert_eq!(lines(&log), ["GET /chat/notes-9f2c1a/session/ses_1/diff"]);
    let [edited, image, gone] = files.as_slice() else {
        panic!("expected three files, got {files:?}")
    };
    assert_eq!(
        edited.file, "src/lib.rs",
        "upstream names the field `file` and the mock names it `path`"
    );
    assert_eq!(edited.status, FileStatus::Modified);
    assert_eq!(edited.additions, 1);
    assert!(!edited.is_binary());

    assert_eq!(image.status, FileStatus::Added);
    assert!(
        image.is_binary(),
        "no patch is how the server says a file has no text to show"
    );
    assert_eq!(gone.status, FileStatus::Deleted);
    assert_eq!(gone.deletions, 1);
}

/// A refusal's body goes into a toast. The whole of a server's stack trace does
/// not fit in one, so the client keeps a readable prefix and no more.
#[tokio::test]
async fn a_long_refusal_body_is_cut_to_something_showable() {
    let (client, _log) = stub(Mode::Contract).await;
    match client.diff(CHAT, "loud").await {
        Err(CodeError::Status { status: 500, body }) => {
            assert_eq!(body.chars().count(), 300);
            assert!(body.starts_with(r#"{"error":"xxx"#));
        }
        other => panic!("expected a truncated 500, got {other:?}"),
    }
}

#[tokio::test]
async fn the_reconnect_catch_up_reads_the_chats_pending_asks() {
    let (client, log) = stub(Mode::Contract).await;
    let pending = client.permissions(CHAT).await.unwrap();
    assert_eq!(lines(&log), ["GET /chat/notes-9f2c1a/permission"]);
    let [only] = pending.as_slice() else {
        panic!("expected one ask, got {pending:?}")
    };
    assert_eq!(only.id, "per_1");
    assert_eq!(only.session_id, "ses_1");
    assert_eq!(only.kind, "bash");
    assert_eq!(
        only.metadata.get("command").and_then(Value::as_str),
        Some("git push")
    );
}

/// The answer is a word in a body, and the ask is identified by two ids in the
/// path. Getting either wrong answers a different ask, or none.
#[tokio::test]
async fn answering_an_ask_names_the_session_the_ask_and_the_word() {
    let (client, log) = stub(Mode::Contract).await;
    client
        .reply_permission(CHAT, "ses_1", "per_1", "once")
        .await
        .unwrap();
    assert_eq!(
        lines(&log),
        ["POST /chat/notes-9f2c1a/session/ses_1/permissions/per_1"]
    );
    assert_eq!(bodies(&log)[0], json!({"response": "once"}));
}

// ---------------------------------------------------------------- events

async fn next_event(rx: &mut tokio::sync::mpsc::Receiver<CodeEvent>) -> CodeEvent {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the event stream went silent")
        .expect("the channel closed before the disconnect arrived")
}

/// The stream, end to end: frames split across chunk boundaries reassemble, a
/// heartbeat is swallowed rather than delivered — it is not something the UI
/// has any use for and would repaint on ten times a minute — and the socket
/// closing arrives as a `Disconnected` the caller can reconnect from rather
/// than as a channel that just stops.
#[tokio::test]
async fn the_event_stream_reassembles_frames_and_hides_heartbeats() {
    let (client, log) = stub(Mode::Contract).await;
    let mut rx = client.events(CHAT);

    match next_event(&mut rx).await {
        CodeEvent::Connected => {}
        other => panic!("expected Connected first, got {other:?}"),
    }
    match next_event(&mut rx).await {
        CodeEvent::PartUpdated { part, delta } => {
            assert_eq!(part.id, "prt_1");
            assert_eq!(part.text.as_deref(), Some("Hello"));
            assert_eq!(
                delta.as_deref(),
                Some("Hello"),
                "the frame carrying this one was cut in half by the stub"
            );
        }
        other => panic!("expected the part update, got {other:?}"),
    }
    match next_event(&mut rx).await {
        CodeEvent::SessionIdle { session_id } => assert_eq!(session_id, "ses_1"),
        other => panic!("expected session.idle, got {other:?}"),
    }
    match next_event(&mut rx).await {
        CodeEvent::Disconnected { reason } => assert_eq!(reason, "stream ended"),
        other => panic!("expected the disconnect, got {other:?}"),
    }
    assert_eq!(lines(&log), ["GET /chat/notes-9f2c1a/event"]);
}

/// A stream that never opens has to report why. Without the status on the
/// `Disconnected`, a gateway refusing the connection is indistinguishable from
/// a chat that simply went quiet, and the app retries forever against a 503.
#[tokio::test]
async fn a_stream_the_gateway_refuses_arrives_as_a_disconnect_naming_the_status() {
    let (client, _log) = stub(Mode::Contract).await;
    let mut rx = client.events("dead");
    match next_event(&mut rx).await {
        CodeEvent::Disconnected { reason } => {
            assert!(
                reason.starts_with("server said 503"),
                "the disconnect must carry the status, got {reason:?}"
            );
            assert!(reason.contains("chat container is gone"));
        }
        other => panic!("expected a disconnect, got {other:?}"),
    }
}

/// Dropping the receiver is the documented way to detach, and it has to really
/// detach: every open `/chat/<id>/event` holds that chat's container awake
/// through the wake-on-request proxy, so a stream left running for a screen
/// nobody is looking at defeats the idle spin-down the whole code plane is
/// built on — and does it invisibly, because the events go nowhere.
#[tokio::test]
async fn dropping_the_receiver_hangs_up_on_the_stream() {
    let (client, log) = stub(Mode::Contract).await;
    let mut rx = client.events("chatty");
    match next_event(&mut rx).await {
        CodeEvent::SessionIdle { session_id } => assert_eq!(session_id, "ses_0"),
        other => panic!("expected the first frame, got {other:?}"),
    }
    drop(rx);

    // Far less than the stub's whole run (200 frames, 10ms apart) and far more
    // than the couple of frames it takes a write to a hung-up socket to fail.
    tokio::time::sleep(Duration::from_millis(600)).await;

    let written = log
        .lock()
        .unwrap()
        .iter()
        .find_map(|s| {
            s.line
                .strip_prefix("sse-frames-written ")
                .and_then(|n| n.parse::<u32>().ok())
        })
        .expect("the stub was still sending: the stream outlived its receiver");
    assert!(
        written < CHATTY_FRAMES,
        "the stub ran to completion, so nothing hung up on it: {written} frames"
    );
}

// ------------------------------------------------- a gateway talking nonsense

/// The captive-portal case: 200s all the way down, carrying a body from no
/// version of this contract. Each route has a documented answer for it, and
/// each one is a deliberate choice between "show less" and "say something
/// false". A list route shows nothing; the two routes that create something,
/// and the aggregate that is read as authority over what is *not* pending, must
/// fail loudly instead.
#[tokio::test]
async fn a_gateway_talking_nonsense_degrades_route_by_route() {
    let (client, _log) = stub(Mode::Garbled).await;

    assert!(client.repos().await.unwrap().is_empty());
    assert!(client.chats().await.unwrap().is_empty());
    assert!(client.sessions(CHAT).await.unwrap().is_empty());
    assert!(client.messages(CHAT, "ses_1").await.unwrap().is_empty());
    assert!(client.diff(CHAT, "ses_1").await.unwrap().is_empty());
    assert!(client.permissions(CHAT).await.unwrap().is_empty());
    assert!(client.agents(CHAT).await.unwrap().is_empty());
    assert!(client.models(CHAT).await.unwrap().is_empty());
    assert_eq!(client.default_model(CHAT).await.unwrap(), None);

    let branches = client.branches("notes").await.unwrap();
    assert!(branches.names().is_empty());
    assert_eq!(branches.default_name(), None);

    // A merge that did not report itself as merged is not one the app repaints
    // a row from.
    let outcome = client.merge_pull(CHAT, 12).await.unwrap();
    assert!(!outcome.merged);
    assert_eq!(outcome.sha, "");

    match client.create_chat("notes", "go", None, None).await {
        Err(CodeError::Other(msg)) => assert!(
            msg.starts_with("bad chat payload:"),
            "a chat that may not exist must not be handed back as one: {msg}"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
    match client.create_session(CHAT).await {
        Err(CodeError::Other(msg)) => assert!(
            msg.starts_with("bad session payload:"),
            "a session id nobody sent cannot be prompted: {msg}"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
    match client.pending_permissions().await {
        Err(CodeError::Other(msg)) => assert!(
            msg.starts_with("bad permission aggregate:"),
            "an unreadable aggregate must not read as `nothing is waiting on you`: {msg}"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The debug line is what ends up in a log when a connection fails. The
/// derived form would print the gateway password into it verbatim.
#[tokio::test]
async fn the_clients_debug_line_carries_no_password() {
    let (client, _log) = stub(Mode::Contract).await;
    let shown = format!("{client:?}");
    assert!(
        !shown.contains(PASSWORD),
        "the gateway password must never reach a log line: {shown}"
    );
    assert!(shown.contains("<redacted>"));
    assert!(
        shown.contains("http://127.0.0.1:"),
        "the base URL is the half worth logging: {shown}"
    );
}

/// A chat id the manager does not know is a 404 on the manager's own routes,
/// and this client hands the status through rather than flattening it into a
/// generic failure — the app tells those two apart.
#[tokio::test]
async fn an_unrouted_request_keeps_its_status_and_sentence() {
    let (client, _log) = stub(Mode::Contract).await;
    match client.sessions("ghost").await {
        Err(e @ CodeError::Status { status: 404, .. }) => {
            assert_eq!(e.message(), "no route: GET /chat/ghost/session");
        }
        other => panic!("expected a 404, got {other:?}"),
    }
}

/// A gateway that is not there at all is a transport failure, not a status —
/// and it has to be, because there is no status to report.
#[tokio::test]
async fn an_unreachable_gateway_is_a_transport_error() {
    let client = CodeClient::new(&CodeConfig {
        // Port 1 on the loopback: nothing binds it, so the connect is refused
        // rather than left hanging.
        base_url: "http://127.0.0.1:1".to_owned(),
        password: PASSWORD.to_owned(),
    })
    .unwrap();
    match client.health().await {
        Err(CodeError::Http(e)) => assert!(
            e.is_connect() || e.is_request(),
            "a refused connection, not a status: {e}"
        ),
        other => panic!("expected a transport error, got {other:?}"),
    }
}

/// Every route, against a gateway that is not there.
///
/// The degrade-to-empty half of this client is right for a 2xx body it cannot
/// read and catastrophic for a network that is down: "no sessions", "no
/// changes", "no models" and "nothing is waiting on you" are all sentences the
/// app would otherwise say confidently about a phone that has simply left the
/// tailnet. The `?` on each route's `send()` is what keeps the two apart, and
/// this is the test that would fail the day one of them grew an
/// `unwrap_or_default` around the whole call.
#[tokio::test]
async fn every_route_reports_an_unreachable_gateway_rather_than_an_empty_answer() {
    // Port 1 on the loopback: nothing binds it, so the connect is refused at
    // once rather than left to time out.
    let client = CodeClient::new(&CodeConfig {
        base_url: "http://127.0.0.1:1".to_owned(),
        password: PASSWORD.to_owned(),
    })
    .unwrap();

    macro_rules! dead {
        ($label:literal, $call:expr) => {
            match $call.await {
                Err(CodeError::Http(_)) => {}
                Err(other) => panic!("{} reported {other:?}, not a transport failure", $label),
                Ok(value) => panic!(
                    "{} answered {value:?} from a gateway that is not there",
                    $label
                ),
            }
        };
    }

    dead!("health", client.health());
    dead!("repos", client.repos());
    dead!("branches", client.branches("notes"));
    dead!("chats", client.chats());
    dead!("pending_permissions", client.pending_permissions());
    dead!("create_chat", client.create_chat("notes", "go", None, None));
    dead!("wake_chat", client.wake_chat(CHAT));
    dead!("stop_chat", client.stop_chat(CHAT));
    dead!("delete_chat", client.delete_chat(CHAT, false));
    dead!("pulls", client.pulls(CHAT));
    dead!("merge_pull", client.merge_pull(CHAT, 12));
    dead!("default_model", client.default_model(CHAT));
    dead!("sessions", client.sessions(CHAT));
    dead!("models", client.models(CHAT));
    dead!("agents", client.agents(CHAT));
    dead!("create_session", client.create_session(CHAT));
    dead!("messages", client.messages(CHAT, "ses_1"));
    dead!(
        "prompt_async",
        client.prompt_async(CHAT, "ses_1", &[PromptPart::text("hi")], None, None, None)
    );
    dead!("abort", client.abort(CHAT, "ses_1"));
    dead!("diff", client.diff(CHAT, "ses_1"));
    dead!("permissions", client.permissions(CHAT));
    dead!(
        "reply_permission",
        client.reply_permission(CHAT, "ses_1", "per_1", "once")
    );

    // The stream has no return value to fail with, so it says the same thing
    // on the channel — and it has to say it, or the caller waits forever on a
    // socket that was never opened.
    let mut rx = client.events(CHAT);
    match next_event(&mut rx).await {
        CodeEvent::Disconnected { reason } => assert!(
            !reason.is_empty() && reason != "stream ended",
            "a stream that never opened must not look like one that ran and finished: {reason:?}"
        ),
        other => panic!("expected a disconnect, got {other:?}"),
    }
}

/// `ChatMeta` is public and constructed by the app's own tests as well as by
/// the wire, so its defaults are part of the contract.
#[test]
fn a_chat_is_running_only_when_the_manager_says_the_word() {
    for (status, running) in [
        ("running", true),
        ("stopped", false),
        ("absent", false),
        ("", false),
    ] {
        let chat: ChatMeta = serde_json::from_value(json!({"id": "c", "status": status})).unwrap();
        assert_eq!(
            chat.is_running(),
            running,
            "{status:?} must{} read as awake",
            if running { "" } else { " not" }
        );
    }
}
