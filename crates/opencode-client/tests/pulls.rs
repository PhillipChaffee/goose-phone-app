//! The pull-request routes, end to end against a stub session manager.
//!
//! These two routes do not exist on the real manager yet — this file *is* the
//! contract, written down as something that runs. Every fixture below is the
//! JSON the manager is being asked to send, and every status is one of the
//! failures it is being asked to send it with, so a manager built to match
//! makes these pass unchanged.
//!
//! The one thing a decode test cannot cover and this can: that the client asks
//! the right question. Both routes hang off `/api/`, never `/chat/<id>/`, and
//! that is not cosmetic — the `/chat/` prefix is the manager's wake-on-request
//! proxy, so a pull-request list served from there would boot a container
//! every time a chat was opened.

// Test code: a failing unwrap, or a panic on the wrong variant, IS the failing
// check. Both are denied for shipped code. `expect` rather than `allow`: if a
// use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test harness: an unwrap or a wrong-status panic is the assertion"
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use opencode_client::{Checks, CodeClient, CodeConfig, CodeError, PullState};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PASSWORD: &str = "stub-password";

/// Every request line the stub saw, as `METHOD PATH auth=yes|no`.
type Seen = Arc<Mutex<Vec<String>>>;

fn pull(number: u64, state: &str, checks: &str, mergeable: &Value) -> Value {
    json!({
        "number": number,
        "title": "Tighten the README quickstart",
        "state": state,
        "draft": false,
        "mergeable": mergeable.clone(),
        "checks": checks,
        "url": format!("https://github.com/me/notes/pull/{number}"),
        "head": "agent/notes-9f2c1a",
        "base": "main",
        "created_at": "2026-08-24T09:12:31Z",
        "updated_at": "2026-08-24T10:02:04Z"
    })
}

/// The answer for one request: HTTP status line and JSON body.
fn answer(method: &str, path: &str) -> (&'static str, Value) {
    match (method, path) {
        ("GET", "/api/chats/notes-9f2c1a/pulls") => (
            "200 OK",
            json!({"pulls": [
                pull(13, "open", "pending", &json!(true)),
                pull(12, "merged", "passing", &json!(null)),
            ]}),
        ),
        ("GET", "/api/chats/ghost/pulls") => ("404 Not Found", json!({"error": "unknown chat"})),
        ("GET", "/api/chats/dropped/pulls") => (
            "409 Conflict",
            json!({"error": "repo 'notes' is not in the allowlist any more"}),
        ),
        ("GET", "/api/chats/offline/pulls") => {
            ("502 Bad Gateway", json!({"error": "GitHub is unreachable"}))
        }
        ("POST", "/api/chats/notes-9f2c1a/pulls/13/merge") => (
            "200 OK",
            json!({
                "merged": true,
                "sha": "9f2c1adb0f4e",
                "pull": pull(13, "merged", "passing", &json!(false)),
            }),
        ),
        ("POST", "/api/chats/notes-9f2c1a/pulls/12/merge") => {
            ("409 Conflict", json!({"error": "#12 is already merged."}))
        }
        ("POST", "/api/chats/notes-9f2c1a/pulls/99/merge") => (
            "404 Not Found",
            json!({"error": "pull 99 is not from this chat's branch"}),
        ),
        ("POST", "/api/chats/notes-9f2c1a/pulls/14/merge") => (
            "422 Unprocessable Entity",
            json!({"error": "At least 1 approving review is required by reviewers with write access."}),
        ),
        _ => (
            "404 Not Found",
            json!({"error": format!("no route: {method} {path}")}),
        ),
    }
}

async fn spawn_stub(seen: Seen) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let seen = Arc::clone(&seen);
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
                let mut start = text.lines().next().unwrap_or_default().split_whitespace();
                let method = start.next().unwrap_or_default().to_string();
                let path = start.next().unwrap_or_default().to_string();
                let authed = text
                    .lines()
                    .any(|l| l.to_ascii_lowercase().starts_with("authorization: basic "));
                // Drain the request body, so a POST is not answered while its
                // sender is still writing.
                let length: usize = text
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse().ok())
                    })
                    .unwrap_or(0);
                if length > 0 {
                    let mut body = vec![0u8; length];
                    let _ = sock.read_exact(&mut body).await;
                }
                seen.lock().unwrap().push(format!(
                    "{method} {path} auth={}",
                    if authed { "yes" } else { "no" }
                ));

                let (code, body) = if authed {
                    answer(&method, &path)
                } else {
                    ("401 Unauthorized", json!({"error": "no credential"}))
                };
                let body = body.to_string();
                let resp = format!(
                    "HTTP/1.1 {code}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    addr
}

async fn stub() -> (CodeClient, Seen) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_stub(Arc::clone(&seen)).await;
    let client = CodeClient::new(&CodeConfig {
        base_url: format!("http://{addr}"),
        password: PASSWORD.to_owned(),
    })
    .unwrap();
    (client, seen)
}

fn requests(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().clone()
}

#[tokio::test]
async fn the_list_comes_off_a_manager_route_and_decodes_in_order() {
    let (client, seen) = stub().await;
    let pulls = client.pulls("notes-9f2c1a").await.unwrap();

    assert_eq!(
        requests(&seen),
        ["GET /api/chats/notes-9f2c1a/pulls auth=yes"],
        "the list must be a manager route — /chat/<id>/ would wake the container"
    );
    assert_eq!(pulls.len(), 2);
    assert_eq!(pulls[0].number, 13);
    assert_eq!(pulls[0].state, PullState::Open);
    assert_eq!(pulls[0].checks, Checks::Pending);
    assert!(
        pulls[0].is_mergeable(),
        "open, mergeable, and checks merely running"
    );
    assert_eq!(pulls[1].state, PullState::Merged);
    assert!(!pulls[1].is_mergeable());
}

#[tokio::test]
async fn a_chat_the_manager_does_not_know_is_a_404() {
    let (client, _seen) = stub().await;
    match client.pulls("ghost").await {
        Err(e @ CodeError::Status { status: 404, .. }) => {
            assert_eq!(e.message(), "unknown chat");
        }
        other => panic!("expected 404, got {other:?}"),
    }
}

/// The chat outlives its allowlist entry, and with the entry goes the clone
/// URL — there is no repo left to ask GitHub about.
#[tokio::test]
async fn a_chat_whose_repo_left_the_allowlist_is_a_409() {
    let (client, _seen) = stub().await;
    match client.pulls("dropped").await {
        Err(e @ CodeError::Status { status: 409, .. }) => {
            assert_eq!(e.message(), "repo 'notes' is not in the allowlist any more");
        }
        other => panic!("expected 409, got {other:?}"),
    }
}

#[tokio::test]
async fn github_being_unreachable_is_a_502_the_app_can_state() {
    let (client, _seen) = stub().await;
    match client.pulls("offline").await {
        Err(e @ CodeError::Status { status: 502, .. }) => {
            assert_eq!(e.message(), "GitHub is unreachable");
        }
        other => panic!("expected 502, got {other:?}"),
    }
}

#[tokio::test]
async fn merging_posts_to_the_pull_and_hands_back_the_refreshed_row() {
    let (client, seen) = stub().await;
    let outcome = client.merge_pull("notes-9f2c1a", 13).await.unwrap();

    assert_eq!(
        requests(&seen),
        ["POST /api/chats/notes-9f2c1a/pulls/13/merge auth=yes"]
    );
    assert!(outcome.merged);
    assert_eq!(outcome.sha, "9f2c1adb0f4e");
    let fresh = outcome.pull.unwrap();
    assert_eq!(fresh.state, PullState::Merged);
    assert!(
        !fresh.is_mergeable(),
        "the row the app repaints must not still offer Merge"
    );
}

/// The number in the path is not enough on its own: the manager checks the
/// pull request is really on this chat's branch, or the route would be "merge
/// anything in the repo" with a chat id in front of it.
#[tokio::test]
async fn merging_a_pull_from_another_branch_is_refused() {
    let (client, _seen) = stub().await;
    match client.merge_pull("notes-9f2c1a", 99).await {
        Err(e @ CodeError::Status { status: 404, .. }) => {
            assert_eq!(e.message(), "pull 99 is not from this chat's branch");
        }
        other => panic!("expected 404, got {other:?}"),
    }
}

#[tokio::test]
async fn a_pull_that_cannot_merge_is_a_409_naming_why() {
    let (client, _seen) = stub().await;
    match client.merge_pull("notes-9f2c1a", 12).await {
        Err(e @ CodeError::Status { status: 409, .. }) => {
            assert_eq!(e.message(), "#12 is already merged.");
        }
        other => panic!("expected 409, got {other:?}"),
    }
}

/// Branch protection, a required review, a head that moved: GitHub's refusals
/// arrive as several statuses and the manager normalises them to one, so the
/// app has a single "GitHub said no" case to show.
#[tokio::test]
async fn github_refusing_the_merge_is_a_422_carrying_githubs_words() {
    let (client, _seen) = stub().await;
    match client.merge_pull("notes-9f2c1a", 14).await {
        Err(e @ CodeError::Status { status: 422, .. }) => {
            assert_eq!(
                e.message(),
                "At least 1 approving review is required by reviewers with write access."
            );
        }
        other => panic!("expected 422, got {other:?}"),
    }
}

/// Both routes are behind the gateway's Basic auth like every other one.
#[tokio::test]
async fn an_unauthenticated_request_is_a_401() {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let addr = spawn_stub(Arc::clone(&seen)).await;
    // A client with no password still sends the header, so the stub's own
    // check cannot be exercised that way; ask it directly instead.
    let raw = reqwest::get(format!("http://{addr}/api/chats/notes-9f2c1a/pulls"))
        .await
        .unwrap();
    assert_eq!(raw.status().as_u16(), 401);
    assert_eq!(
        requests(&seen),
        ["GET /api/chats/notes-9f2c1a/pulls auth=no"]
    );
}
