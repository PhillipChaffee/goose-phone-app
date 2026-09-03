//! What the mock answers, for both tiers.
//!
//! `mock-goose-server`'s `features/mod.rs` walks a table of handlers, each
//! returning `None` for "not mine", and its final fallthrough is a load-bearing
//! contract. This is the same shape against a REST surface: one function per
//! tier, `None` meaning "no route here", and `main.rs` turning that into a 404.
//!
//! The two tiers are one listener because the real system is: the manager
//! proxies each chat's own `OpenCode` server at `/chat/<id>`, so the client
//! holds ONE base URL and one password for both.

use serde_json::{json, Value};

use crate::state::{now, now_ms, Message, Part, State, Step, Tool};
use crate::wire;

/// A response the transport should send.
pub(crate) enum Reply {
    Json(u16, Value),
    /// The SSE stream. `main.rs` owns it, because it is the one reply that
    /// outlives the request.
    Events(String),
}

/// `/api/...` — the gateway's own surface, the half that never touches a
/// container and so the half a mock can serve completely.
pub(crate) fn manager(state: &mut State, method: &str, path: &str, body: &Value) -> Option<Reply> {
    let rest = path.strip_prefix("/api/")?;
    // The client appends exactly one query string, ever: `?purge=1` on DELETE.
    let (rest, _query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();

    Some(match (method, parts.as_slice()) {
        // The gate for the whole Code plane. The client reads exactly one
        // field — `active` — to build the connection badge.
        ("GET", ["health"]) => Reply::Json(
            200,
            json!({
                "ok": true,
                "engine": "mock",
                "chats": state.chats.len(),
                "active": state.chats.iter().filter(|c| c.status == "running").count(),
                "max_active": 4,
            }),
        ),

        ("GET", ["repos"]) => Reply::Json(
            200,
            json!({ "repos": state.repos.iter().map(wire::repo).collect::<Vec<_>>() }),
        ),

        ("GET", ["repos", name, "branches"]) => {
            let name = percent_decode(name);
            match state.repos.iter().find(|r| r.name == name) {
                Some(r) => Reply::Json(200, wire::branches(r)),
                None => Reply::Json(404, json!({ "status": "unknown repo" })),
            }
        }

        ("GET", ["chats"]) => {
            // NEWEST FIRST, which the real manager does and the app relies on:
            // `views::code` renders the list in wire order.
            let mut chats = state.chats.clone();
            chats.sort_by(|a, b| b.last_active.total_cmp(&a.last_active));
            Reply::Json(
                200,
                json!({ "chats": chats.iter().map(wire::chat).collect::<Vec<_>>() }),
            )
        }

        ("POST", ["chats"]) => {
            let repo = body.get("repo").and_then(Value::as_str).unwrap_or("");
            let task = body.get("task").and_then(Value::as_str).unwrap_or("");
            let base = body.get("base").and_then(Value::as_str).unwrap_or("main");
            let model = body.get("model").and_then(Value::as_str);
            let chat = crate::state::new_chat(repo, task, base, model);
            let made = wire::chat(&chat);
            state.chats.push(chat);
            // 201 with a BARE ChatMeta — no wrapper, unlike the list.
            Reply::Json(201, made)
        }

        ("POST", [id, "wake"]) if parts.first() == Some(&"chats") => unreachable(id),
        ("POST", ["chats", id, "wake"]) => {
            if let Some(c) = state.chat_mut(id) {
                c.status = "running".to_owned();
                c.last_active = now();
            }
            Reply::Json(200, json!({ "status": "woken" }))
        }

        ("POST", ["chats", id, "stop"]) => {
            if let Some(c) = state.chat_mut(id) {
                c.status = "stopped".to_owned();
            }
            state.pending.remove(*id);
            Reply::Json(200, json!({ "status": "stopped" }))
        }

        ("DELETE", ["chats", id]) => {
            state.chats.retain(|c| c.id != *id);
            state.pending.remove(*id);
            Reply::Json(200, json!({ "status": "deleted", "volume": "purged" }))
        }

        ("GET", ["chats", id, "pulls"]) => {
            let pulls = state
                .chat(id)
                .map(|c| c.pulls.iter().map(wire::pull).collect::<Vec<_>>())
                .unwrap_or_default();
            Reply::Json(200, json!({ "pulls": pulls }))
        }

        ("POST", ["chats", id, "pulls", number, "merge"]) => {
            let n: u64 = number.parse().unwrap_or(0);
            let merged = state.chat_mut(id).and_then(|c| {
                let p = c.pulls.iter_mut().find(|p| p.number == n)?;
                p.state = "merged".to_owned();
                Some(wire::pull(p))
            });
            match merged {
                Some(pull) => Reply::Json(
                    200,
                    json!({ "merged": true, "sha": "9f2c1ad3b7e40f1c2d5a8e6b09", "pull": pull }),
                ),
                None => Reply::Json(404, json!({ "status": "unknown pull" })),
            }
        }

        // THE STRICTEST DECODE IN THE CLIENT, and the only route where a
        // sloppy body is an error rather than an empty screen: the body must
        // be an object carrying an array `permissions`. A bare array is
        // refused on purpose, so that a gateway answering nonsense preserves
        // the queue the app already has instead of clearing it.
        ("GET", ["permissions"]) => {
            let mut all = Vec::new();
            for c in &state.chats {
                for a in &c.asks {
                    all.push(wire::ask_with_chat(&c.id, &c.session.id, a));
                }
            }
            Reply::Json(
                200,
                json!({ "permissions": all, "unreachable": state.unreachable }),
            )
        }

        _ => return None,
    })
}

/// `/chat/<id>/...` — one chat's own `OpenCode` server, reached through the
/// manager's proxy. Same origin, same password.
///
/// One `match` over thirteen routes, and long because of it. Splitting it to
/// satisfy a line count would put the routing decision in two places, which is
/// the thing a router most needs to keep in one.
#[expect(clippy::too_many_lines, reason = "one match, thirteen routes")]
pub(crate) fn chat_server(
    state: &mut State,
    method: &str,
    path: &str,
    body: &Value,
) -> Option<Reply> {
    let rest = path.strip_prefix("/chat/")?;
    let (rest, _query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let chat_id = parts.remove(0).to_owned();

    // A chat the manager has forgotten answers 404 rather than an empty list,
    // because "no such container" and "a container with nothing in it" are
    // different answers and the app says different things about them.
    if state.chat(&chat_id).is_none() {
        return Some(Reply::Json(404, json!({ "status": "unknown chat" })));
    }

    Some(match (method, parts.as_slice()) {
        ("GET", ["config"]) => Reply::Json(
            200,
            json!({
                "$schema": "https://opencode.ai/config.json",
                "model": state.chat(&chat_id).and_then(|c| c.model.clone()),
            }),
        ),

        // BOTH catalogue routes, keyed differently. The client tries
        // `/config/providers` first and falls back to `/provider`; answering
        // both is what the real server does and the client dedupes.
        ("GET", ["config", "providers"]) => Reply::Json(200, wire::providers()),
        ("GET", ["provider"]) => {
            let mut v = wire::providers();
            // `/provider` keys the same list under `all`, not `providers`.
            let all = v["providers"].take();
            Reply::Json(
                200,
                json!({ "all": all, "default": {}, "connected": ["opencode"] }),
            )
        }

        ("GET", ["agent"]) => Reply::Json(200, wire::agents()),

        ("GET", ["session"]) => {
            let c = state.chat(&chat_id)?;
            // An ARRAY, newest first.
            Reply::Json(200, json!([wire::session(c, &c.session)]))
        }

        ("POST", ["session"]) => {
            let c = state.chat(&chat_id)?;
            // A single object, NOT an array — the one asymmetry on this route.
            Reply::Json(200, wire::session(c, &c.session))
        }

        ("GET", ["session", _sid, "message"]) => {
            let c = state.chat(&chat_id)?;
            let msgs = c
                .session
                .messages
                .iter()
                .map(|m| wire::message(&c.session.id, m))
                .collect::<Vec<_>>();
            Reply::Json(200, json!(msgs))
        }

        // The turn's output arrives on the SSE stream; this only has to be 2xx.
        ("POST", ["session", _sid, "prompt_async"]) => {
            let text = prompt_text(body);
            let steps = crate::turn::script(&text);
            state.pending.insert(chat_id.clone(), steps);
            if let Some(c) = state.chat_mut(&chat_id) {
                c.status = "running".to_owned();
                c.last_active = now();
                let n = c.session.messages.len() + 1;
                c.session.messages.push(Message {
                    id: format!("msg_{n}"),
                    role: "user".to_owned(),
                    created_ms: now_ms(),
                    parts: vec![Part {
                        id: format!("prt_{n}u"),
                        kind: "text".to_owned(),
                        text,
                        tool: None,
                    }],
                });
            }
            Reply::Json(200, json!({}))
        }

        ("POST", ["session", _sid, "abort"]) => {
            state.pending.remove(&chat_id);
            Reply::Json(200, json!({}))
        }

        ("GET", ["session", _sid, "diff"]) => {
            let c = state.chat(&chat_id)?;
            // A BARE ARRAY, not an object.
            Reply::Json(
                200,
                json!(c.diff.iter().map(wire::file_diff).collect::<Vec<_>>()),
            )
        }

        ("GET", ["permission"]) => {
            let c = state.chat(&chat_id)?;
            Reply::Json(
                200,
                json!(c
                    .asks
                    .iter()
                    .map(|a| wire::ask(&c.session.id, a))
                    .collect::<Vec<_>>()),
            )
        }

        ("POST", ["session", _sid, "permissions", ask_id]) => {
            let answered = *ask_id;
            let once = body
                .get("response")
                .and_then(Value::as_str)
                .unwrap_or("once")
                .to_owned();
            if let Some(c) = state.chat_mut(&chat_id) {
                c.asks.retain(|a| a.id != answered);
                c.last_active = now();
                // An approved ask lets the parked turn finish; a rejection
                // ends it. Both are what the real thing does, and the app has
                // different transcripts for them.
                let n = c.session.messages.len() + 1;
                let said = if once == "reject" {
                    "Stopped — I will leave the branch unpushed."
                } else {
                    "Pushed. The branch is up and CI has started."
                };
                c.session.messages.push(Message {
                    id: format!("msg_{n}"),
                    role: "assistant".to_owned(),
                    created_ms: now_ms(),
                    parts: vec![Part {
                        id: format!("prt_{n}a"),
                        kind: "text".to_owned(),
                        text: said.to_owned(),
                        tool: None,
                    }],
                });
            }
            state.pending.entry(chat_id).or_default().push(Step::Idle);
            Reply::Json(200, json!({}))
        }

        ("GET", ["event"]) => Reply::Events(chat_id),

        _ => return None,
    })
}

/// A chat id that reached a route expecting `chats/<id>`; never taken, and
/// present so the match above stays exhaustive rather than silently falling
/// through to a 404 that hides a routing mistake.
fn unreachable(id: &str) -> Reply {
    Reply::Json(
        500,
        json!({ "status": format!("mock routing error on {id}") }),
    )
}

/// The text of a prompt, out of whichever shape the client sent.
fn prompt_text(body: &Value) -> String {
    body.get("parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|s| !s.is_empty())
        .or_else(|| {
            body.get("text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

/// `%XX` back to bytes. Only `/api/repos/<name>/branches` is encoded by the
/// client, and only because a repo name may contain a slash.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// One tool part, for the scripted turn.
pub(crate) fn tool_part(id: &str, name: &str, title: &str, status: &str, output: &str) -> Part {
    Part {
        id: id.to_owned(),
        kind: "tool".to_owned(),
        text: String::new(),
        tool: Some(Tool {
            name: name.to_owned(),
            status: status.to_owned(),
            title: title.to_owned(),
            output: output.to_owned(),
        }),
    }
}
