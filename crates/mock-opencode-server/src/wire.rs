//! The store, encoded the way the wire spells it.
//!
//! EVERY FIELD NAME HERE IS THE WIRE'S, not the store's and not the client's
//! Rust field. `opencode-client` uses `serde(rename)`, `alias` and
//! `rename_all = "camelCase"` in a dozen places, and the two tiers disagree
//! with each other: the manager is `snake_case` with float SECONDS, the
//! `OpenCode` server behind it is `camelCase` with integer MILLISECONDS.
//!
//! Getting one of these wrong does not fail loudly. `GET /api/chats` and
//! `GET /api/repos` decode their arrays WHOLE with `unwrap_or_default()`, so a
//! single misspelled field empties the entire board, the sidebar and the tiles
//! at once, with no error anywhere — which is exactly the failure this file is
//! written to avoid, and why `tests/` drives the real client rather than
//! asserting on these strings.

use serde_json::{json, Value};

use crate::state::{Ask, Chat, FileDiff, Message, Part, Pull, Repo, Session};

/// `ChatMeta`. `last_active` and `created` are float SECONDS.
pub(crate) fn chat(c: &Chat) -> Value {
    json!({
        "id": c.id,
        "repo": c.repo,
        "title": c.title,
        "branch": c.branch,
        "base": c.base,
        "model": c.model,
        "status": c.status,
        "created": c.created,
        "last_active": c.last_active,
        // The manager's own bookkeeping. The client ignores both, and they are
        // here because a mock that answers a strict subset teaches the next
        // reader that the subset is all there is.
        "port": 4310,
        "url": format!("/chat/{}", c.id),
    })
}

/// `RepoEntry`. `name` is the only required field; the rest are `serde(default)`
/// and are sent because the real manager sends them.
pub(crate) fn repo(r: &Repo) -> Value {
    json!({
        "name": r.name,
        "url": r.url,
        "setup": "",
        "edit_only": false,
        "allow_push": true,
        "public_throwaway": r.public_throwaway,
    })
}

/// `BranchList`. `default` is the wire name at BOTH levels — the list's default
/// branch, and the flag on the branch that is it.
pub(crate) fn branches(r: &Repo) -> Value {
    json!({
        "repo": r.name,
        "default": r.default_branch,
        "truncated": false,
        "branches": r.branches.iter().map(|b| json!({
            "name": b,
            "default": *b == r.default_branch,
        })).collect::<Vec<_>>(),
    })
}

/// `CodePermission` as the MANAGER reports it: the same object the chat's own
/// `/permission` returns, plus a `chatId` saying which container it is parked
/// in. `sessionID` and `chatId` are the exact spellings — not `session_id`,
/// not `chat_id`.
pub(crate) fn ask_with_chat(chat_id: &str, session_id: &str, a: &Ask) -> Value {
    let mut v = ask(session_id, a);
    v["chatId"] = json!(chat_id);
    v
}

/// `CodePermission` as a chat's own server reports it. `type` is the kind.
pub(crate) fn ask(session_id: &str, a: &Ask) -> Value {
    json!({
        "id": a.id,
        "sessionID": session_id,
        "type": a.kind,
        "title": a.title,
        "metadata": { "command": a.command, "cwd": "/chat/workspace" },
    })
}

/// `SessionMeta`. `time.created` / `time.updated` are integer MILLISECONDS,
/// and `projectID` is camelCase.
pub(crate) fn session(c: &Chat, s: &Session) -> Value {
    json!({
        "id": s.id,
        "title": s.title,
        "directory": "/chat/workspace",
        "version": "0.15.3",
        "projectID": format!("prj_{}", c.repo),
        "slug": s.id,
        "time": { "created": s.created_ms, "updated": s.updated_ms },
        "model": c.model.as_ref().map(|m| {
            let (provider, id) = m.split_once('/').unwrap_or(("opencode", m.as_str()));
            json!({ "providerID": provider, "id": id })
        }),
        "agent": "build",
    })
}

/// One `{info, parts}` pair from `session/:id/message`.
///
/// The app's role map is fed ONLY by a message's `info`, so a part whose
/// message was never announced renders as an assistant bubble whatever it
/// really was — which is why `info` carries the role and the parts do not.
pub(crate) fn message(session_id: &str, m: &Message) -> Value {
    json!({
        "info": {
            "id": m.id,
            "role": m.role,
            "sessionID": session_id,
            "time": { "created": m.created_ms },
        },
        "parts": m.parts.iter().map(|p| part(session_id, &m.id, p)).collect::<Vec<_>>(),
    })
}

/// One part. `messageID` and `sessionID` are camelCase; `callID` is too.
pub(crate) fn part(session_id: &str, message_id: &str, p: &Part) -> Value {
    let mut v = json!({
        "id": p.id,
        "messageID": message_id,
        "sessionID": session_id,
        "type": p.kind,
    });
    if let Some(tool) = &p.tool {
        v["tool"] = json!(tool.name);
        v["callID"] = json!(format!("call_{}", p.id));
        v["state"] = json!({
            "status": tool.status,
            "title": tool.title,
            "output": tool.output,
            "input": {},
        });
    } else {
        v["text"] = json!(p.text);
    }
    v
}

/// `FileDiff`. The client's Rust field is `file` with `alias = "path"`; the
/// real server sends `file`, so the mock does too — the alias exists for a
/// different server, not for this one.
pub(crate) fn file_diff(d: &FileDiff) -> Value {
    json!({
        "file": d.file,
        "patch": d.patch,
        "additions": d.additions,
        "deletions": d.deletions,
        "status": d.status,
    })
}

/// `PullRequest`. `state` and `checks` go through custom deserializers that
/// accept exactly these strings and turn anything else into `Unknown` / `None`.
pub(crate) fn pull(p: &Pull) -> Value {
    json!({
        "number": p.number,
        "title": p.title,
        "state": p.state,
        "draft": p.draft,
        "mergeable": p.mergeable,
        "checks": p.checks,
        "url": format!("https://github.com/PhillipChaffee/goose-phone-app/pull/{}", p.number),
        "head": p.head,
        "base": p.base,
        "created_at": "2026-09-01T09:00:00Z",
        "updated_at": "2026-09-02T14:20:00Z",
    })
}

/// The model catalogue, as `/config/providers` spells it.
///
/// `providerID` inside a model and `id` on the provider are both load-bearing:
/// the desktop inspector reads `ModelInfo::reference()`, which is
/// `providerID/id`, and matches it against `ChatMeta.model`. A `snake_case`
/// `provider_id` here leaves the inspector's model row and its context-window
/// fact silently absent.
pub(crate) fn providers() -> Value {
    json!({
        "providers": [{
            "id": "opencode",
            "name": "opencode",
            "models": {
                "claude-sonnet-4-5": model("claude-sonnet-4-5", "Claude Sonnet 4.5", 200_000),
                "claude-opus-4-1": model("claude-opus-4-1", "Claude Opus 4.1", 200_000),
                "minimax-m2.7": model("minimax-m2.7", "MiniMax M2.7", 1_000_000),
            },
        }],
        "default": { "opencode": "claude-sonnet-4-5" },
    })
}

fn model(id: &str, name: &str, context: u64) -> Value {
    json!({
        "id": id,
        "providerID": "opencode",
        "name": name,
        "limit": { "context": context, "output": 64_000 },
        "variants": {},
    })
}

/// `Agent`. `builtIn` is camelCase; `mode` is `primary` or `subagent`.
pub(crate) fn agents() -> Value {
    json!([
        {
            "name": "build",
            "description": "Full access. Edits files and runs commands.",
            "mode": "primary",
            "builtIn": true,
        },
        {
            "name": "plan",
            "description": "Read-only analysis. Cannot edit files.",
            "mode": "primary",
            "builtIn": true,
        },
    ])
}
