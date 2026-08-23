//! Code tab: state + logic for code-agent chats — per-chat `OpenCode`
//! containers on the brain, fronted by the session manager
//! (personal-ai-setup `docs/code-agents.md`; this repo issue #2).
//!
//! Shape mirrors the Home tab (`state.rs`): signals on `AppCtx`, long-lived
//! work on the root scope via `spawn_forever`, and the same `ChatItem`
//! transcript model so the chat renderer is shared. The differences that
//! matter:
//!   - transport is HTTP + SSE (via `opencode-client`), not ACP/WebSocket
//!   - opening a stopped chat renders the on-device cache instantly, then
//!     reconciles against the server once the container is awake (the
//!     server copy is authoritative)
//!   - permission asks arrive as events and are answered over HTTP with
//!     `once` / `always` / `reject`

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Duration;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use opencode_client::{
    ChatMeta, CodeClient, CodeConfig, CodeEvent, CodePermission, MessageWithParts, Part,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::{show_toast, AppCtx, ChatItem, ConnState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeScreen {
    List,
    New,
    Chat,
}

/// Everything the code chat screen renders.
#[derive(Clone, PartialEq, Default)]
pub(crate) struct CodeChatState {
    pub chat_id: Option<String>,
    pub title: String,
    pub repo: String,
    pub branch: String,
    /// The chat's primary `OpenCode` session (created lazily on first prompt).
    pub session_id: Option<String>,
    pub items: Vec<ChatItem>,
    /// See `ChatState::marks` — same purpose, and the same reason it is not
    /// an item: `part_index` below indexes into `items`.
    pub marks: Vec<(usize, i64)>,
    pub last_at: i64,
    /// part id -> index into `items`, for folding streamed part updates.
    pub part_index: HashMap<String, usize>,
    /// message id -> role, learned from history + `message.updated` events.
    pub roles: HashMap<String, String>,
    pub running: bool,
    pub loading: bool,
    /// Container is booting; the transcript shown is the on-device cache.
    pub waking: bool,
    pub diff: Option<String>,
}

/// On-device transcript cache (issue #2, A11): instant open while the
/// container wakes, read-only offline. LRU-capped; server is authoritative.
#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct CodeCache {
    pub chats: HashMap<String, CachedChat>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct CachedChat {
    pub title: String,
    pub session_id: Option<String>,
    pub items: Vec<ChatItem>,
    /// Unix seconds of the last cache write — the LRU eviction key.
    pub updated: u64,
}

const CACHE_MAX_CHATS: usize = 15;
const CACHE_MAX_ITEMS: usize = 300;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ------------------------------------------------------------- connection

fn build_client(ctx: &AppCtx) -> Result<CodeClient, String> {
    let s = ctx.settings.peek();
    CodeClient::new(&CodeConfig {
        base_url: s.code_server_url.clone(),
        password: s.code_password.clone(),
    })
    .map_err(|e| e.to_string())
}

/// Connect to the manager gateway using saved settings. Returns true on
/// success (health answered), populating the chat list and repo allowlist.
pub(crate) async fn code_connect(ctx: &AppCtx) -> bool {
    let mut conn = ctx.code_conn;
    let mut client_slot = ctx.code_client;
    let client = match build_client(ctx) {
        Ok(c) => c,
        Err(e) => {
            conn.set(ConnState::Failed(e));
            return false;
        }
    };
    conn.set(ConnState::Connecting);
    match client.health().await {
        Ok(h) => {
            let agent = format!(
                "code agents ({} active)",
                h.get("active").and_then(Value::as_u64).unwrap_or(0)
            );
            client_slot.set(Some(client));
            conn.set(ConnState::Connected { agent });
            refresh_code_chats(ctx).await;
            refresh_repos(ctx).await;
            true
        }
        Err(e) => {
            conn.set(ConnState::Failed(e.to_string()));
            false
        }
    }
}

pub(crate) async fn refresh_code_chats(ctx: &AppCtx) {
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let mut chats = ctx.code_chats;
    let mut loading = ctx.code_chats_loading;
    loading.set(true);
    match client.chats().await {
        Ok(list) => chats.set(list),
        Err(e) => show_toast(ctx, format!("Failed to list code chats: {e}")),
    }
    loading.set(false);
}

async fn refresh_repos(ctx: &AppCtx) {
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let mut repos = ctx.code_repos;
    if let Ok(list) = client.repos().await {
        repos.set(list);
    }
}

/// Keep the chat list fresh while the Code tab is visible. One loop per
/// epoch; a tab switch away lets it park (cheap no-op ticks).
pub(crate) fn start_code_poll(ctx: &AppCtx) {
    let epoch = *ctx.code_epoch.peek();
    let ctx = *ctx;
    spawn_forever(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if *ctx.code_epoch.peek() != epoch {
                return;
            }
            if *ctx.tab.peek() != crate::state::Tab::Code {
                continue;
            }
            if ctx.code_client.peek().is_none() {
                continue;
            }
            refresh_code_chats(&ctx).await;
        }
    });
}

// ------------------------------------------------------------ open / fold

/// Open a chat: cached transcript instantly (read-only, "waking…" when the
/// container is down), then wake + reconcile from the server, then live SSE.
pub(crate) fn open_code_chat(ctx: &AppCtx, meta: ChatMeta) {
    let mut chat = ctx.code_chat;
    let mut screen = ctx.code_screen;
    let mut epoch = ctx.code_epoch;
    let new_epoch = *epoch.peek() + 1;
    epoch.set(new_epoch);

    let cached = ctx.code_cache.peek().chats.get(&meta.id).cloned();
    let waking = !meta.is_running();
    chat.set(CodeChatState {
        marks: Vec::new(),
        last_at: 0,
        chat_id: Some(meta.id.clone()),
        title: if meta.title.is_empty() {
            meta.id.clone()
        } else {
            meta.title.clone()
        },
        repo: meta.repo.clone(),
        branch: meta.branch.clone(),
        session_id: cached.as_ref().and_then(|c| c.session_id.clone()),
        items: cached.map(|c| c.items).unwrap_or_default(),
        part_index: HashMap::new(),
        roles: HashMap::new(),
        running: false,
        loading: true,
        waking,
        diff: None,
    });
    screen.set(CodeScreen::Chat);
    let ctx = *ctx;
    spawn_forever(async move { attach_chat(&ctx, meta.id, new_epoch).await });
}

/// Wake (implicit in any request), fetch authoritative history, catch up on
/// pending permissions, and attach the SSE stream. Also the reconnect path.
async fn attach_chat(ctx: &AppCtx, chat_id: String, epoch: u64) {
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let mut chat = ctx.code_chat;

    // Resolve the chat's session: prefer the cached one when the server
    // still knows it, else the first existing, else none yet (created on
    // the first prompt).
    let sessions = client.sessions(&chat_id).await;
    if *ctx.code_epoch.peek() != epoch {
        return;
    }
    let session_id = match sessions {
        Ok(list) => {
            let cached = chat.peek().session_id.clone();
            cached
                .filter(|id| list.iter().any(|s| &s.id == id))
                .or_else(|| list.first().map(|s| s.id.clone()))
        }
        Err(e) => {
            chat.write().loading = false;
            chat.write().waking = false;
            show_toast(ctx, format!("Chat unreachable: {e}"));
            return;
        }
    };
    chat.write().session_id.clone_from(&session_id);
    chat.write().waking = false;

    // Authoritative history replaces whatever the cache showed (A5/A11:
    // reconcile with no duplicates — full replace by construction).
    if let Some(sid) = &session_id {
        if !load_history(ctx, &client, &chat_id, sid, epoch).await {
            return;
        }
    }
    chat.write().loading = false;

    catch_up_permissions(ctx, &client, &chat_id).await;
    stream_events(ctx, &client, &chat_id, epoch).await;
}

/// Replace the transcript with the server's history. Returns false when a
/// different chat was opened while the fetch was in flight, meaning the
/// caller must stop touching this chat's state.
async fn load_history(
    ctx: &AppCtx,
    client: &CodeClient,
    chat_id: &str,
    session_id: &str,
    epoch: u64,
) -> bool {
    match client.messages(chat_id, session_id).await {
        Ok(msgs) => {
            if *ctx.code_epoch.peek() != epoch {
                return false;
            }
            let (items, part_index, roles, running) = fold_history(&msgs);
            {
                let mut chat = ctx.code_chat;
                let mut c = chat.write();
                c.items = items;
                c.part_index = part_index;
                c.roles = roles;
                c.running = running;
            }
            write_cache(ctx);
        }
        Err(e) => show_toast(ctx, format!("History load failed: {e}")),
    }
    true
}

/// Permission catch-up: asks raised while we were away still block their
/// tool calls server-side.
async fn catch_up_permissions(ctx: &AppCtx, client: &CodeClient, chat_id: &str) {
    if let Ok(pending) = client.permissions(chat_id).await {
        let mut queue = ctx.code_permissions;
        let mut q = queue.write();
        for p in pending {
            if !q
                .iter()
                .any(|(cid, e)| cid.as_str() == chat_id && e.id == p.id)
            {
                q.push((chat_id.to_string(), p));
            }
        }
    }
}

/// Fold the live SSE stream into the open chat until it ends, the user opens
/// another chat, or the container goes away (which schedules a re-attach).
async fn stream_events(ctx: &AppCtx, client: &CodeClient, chat_id: &str, epoch: u64) {
    let mut events = client.events(chat_id);
    while let Some(event) = events.recv().await {
        if *ctx.code_epoch.peek() != epoch {
            return; // another chat was opened; detach silently
        }
        match event {
            CodeEvent::PartUpdated { part, delta } => {
                let is_current = ctx
                    .code_chat
                    .peek()
                    .session_id
                    .as_deref()
                    .is_some_and(|sid| sid == part.session_id);
                if is_current {
                    fold_part(&mut ctx.code_chat.clone(), &part, delta.as_deref());
                }
            }
            CodeEvent::MessageUpdated { info } => {
                if !info.id.is_empty() {
                    ctx.code_chat
                        .clone()
                        .write()
                        .roles
                        .insert(info.id, info.role);
                }
            }
            CodeEvent::PermissionAsked(p) => {
                let mut queue = ctx.code_permissions;
                let exists = queue
                    .peek()
                    .iter()
                    .any(|(cid, e)| cid.as_str() == chat_id && e.id == p.id);
                if !exists {
                    queue.write().push((chat_id.to_string(), p));
                }
            }
            CodeEvent::PermissionReplied { id } => {
                ctx.code_permissions
                    .clone()
                    .write()
                    .retain(|(cid, p)| !(cid.as_str() == chat_id && p.id == id));
            }
            CodeEvent::SessionIdle { session_id: sid } => {
                let mut c = ctx.code_chat;
                if c.peek().session_id.as_deref() == Some(sid.as_str()) {
                    c.write().running = false;
                    write_cache(ctx);
                }
            }
            CodeEvent::Disconnected { .. } => {
                // Container spin-down, network blip, or gateway restart.
                // Re-attach (which also transparently wakes) after a pause,
                // unless the user has moved on.
                ctx.code_chat.clone().write().running = false;
                write_cache(ctx);
                tokio::time::sleep(Duration::from_secs(5)).await;
                if *ctx.code_epoch.peek() == epoch && *ctx.tab.peek() == crate::state::Tab::Code {
                    let ctx = *ctx;
                    let chat_id = chat_id.to_string();
                    spawn_forever(async move { attach_chat(&ctx, chat_id, epoch).await });
                }
                return;
            }
            CodeEvent::Connected | CodeEvent::SessionStatus(_) | CodeEvent::Unknown { .. } => {}
        }
    }
}

/// Fold full message history into transcript items. Returns
/// (items, part-index, roles, any-assistant-turn-still-streaming).
fn fold_history(
    msgs: &[MessageWithParts],
) -> (
    Vec<ChatItem>,
    HashMap<String, usize>,
    HashMap<String, String>,
    bool,
) {
    let mut items = Vec::new();
    let mut part_index = HashMap::new();
    let mut roles = HashMap::new();
    let (mut replay_marks, mut replay_at) = (Vec::new(), 0);
    let mut gap = GapSink {
        marks: &mut replay_marks,
        last_at: &mut replay_at,
    };
    for msg in msgs {
        roles.insert(msg.info.id.clone(), msg.info.role.clone());
        for part in &msg.parts {
            fold_part_into(&mut items, &mut part_index, &roles, part, None, &mut gap);
        }
    }
    // Heuristic: OpenCode marks completion via session.idle events, not the
    // message list — assume not running; the SSE stream corrects us.
    (items, part_index, roles, false)
}

fn fold_part(chat: &mut Signal<CodeChatState>, part: &Part, delta: Option<&str>) {
    let mut c = chat.write();
    let CodeChatState {
        items,
        part_index,
        roles,
        running,
        marks,
        last_at,
        ..
    } = &mut *c;
    // A streamed part means the turn is alive.
    if part.kind == "text" || part.kind == "reasoning" || part.kind == "tool" {
        *running = true;
    }
    let mut gap = GapSink { marks, last_at };
    fold_part_into(items, part_index, roles, part, delta, &mut gap);
}

/// Where time-gap marks accumulate as items are appended.
///
/// History replay passes a throwaway: it rebuilds the whole transcript in one
/// instant, so every gap it could compute would be zero anyway.
pub(crate) struct GapSink<'a> {
    pub marks: &'a mut Vec<(usize, i64)>,
    pub last_at: &'a mut i64,
}

/// The single folding rule for both history and live updates:
///   - known part id + delta  -> append the delta
///   - known part id, no delta -> replace with the full snapshot
///   - new part id            -> new transcript item
fn fold_part_into(
    items: &mut Vec<ChatItem>,
    part_index: &mut HashMap<String, usize>,
    roles: &HashMap<String, String>,
    part: &Part,
    delta: Option<&str>,
    gap: &mut GapSink<'_>,
) {
    match part.kind.as_str() {
        "text" | "reasoning" => {
            let role = roles
                .get(&part.message_id)
                .map_or("assistant", String::as_str);
            let full = part.text.clone().unwrap_or_default();
            if let Some(&idx) = part_index.get(&part.id) {
                if let Some(
                    ChatItem::Assistant { text, .. }
                    | ChatItem::Thought { text, .. }
                    | ChatItem::User { text },
                ) = items.get_mut(idx)
                {
                    match delta {
                        Some(d) => text.push_str(d),
                        None => *text = full,
                    }
                }
                return;
            }
            if full.is_empty() && delta.is_none() {
                return;
            }
            let text = if full.is_empty() {
                delta.unwrap_or_default().to_string()
            } else {
                full
            };
            let item = match (part.kind.as_str(), role) {
                ("reasoning", _) => ChatItem::Thought {
                    message_id: Some(part.id.clone()),
                    text,
                },
                (_, "user") => ChatItem::User { text },
                _ => ChatItem::Assistant {
                    message_id: Some(part.id.clone()),
                    text,
                },
            };
            part_index.insert(part.id.clone(), items.len());
            crate::state::mark_gap(items.len(), gap.marks, gap.last_at);
            items.push(item);
        }
        "tool" => {
            let state = part.state.clone().unwrap_or(Value::Null);
            let status = state
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending")
                .to_string();
            let title = state
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| part.tool.clone())
                .unwrap_or_else(|| "tool".to_string());
            let output = state
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let id = part.call_id.clone().unwrap_or_else(|| part.id.clone());
            if let Some(&idx) = part_index.get(&part.id) {
                if let Some(ChatItem::Tool {
                    title: t,
                    status: s,
                    output: o,
                    ..
                }) = items.get_mut(idx)
                {
                    *t = title;
                    *s = status;
                    if !output.is_empty() {
                        *o = output;
                    }
                }
                return;
            }
            part_index.insert(part.id.clone(), items.len());
            crate::state::mark_gap(items.len(), gap.marks, gap.last_at);
            items.push(ChatItem::Tool {
                id,
                title,
                kind: "execute".to_string(),
                status,
                output,
            });
        }
        // step-start / step-finish / file / snapshot etc. — nothing to render.
        _ => {}
    }
}

// --------------------------------------------------------------- actions

/// Send a prompt into the open code chat (creating its `OpenCode` session on
/// first use). Returns false if the message could not be submitted.
pub(crate) fn send_code_prompt(ctx: &AppCtx, text: String) -> bool {
    let mut chat = ctx.code_chat;
    let Some(chat_id) = chat.peek().chat_id.clone() else {
        return false;
    };
    let Some(client) = ctx.code_client.peek().clone() else {
        show_toast(ctx, "Code plane not connected — check Settings");
        return false;
    };
    {
        let mut c = chat.write();
        let CodeChatState {
            items,
            marks,
            last_at,
            ..
        } = &mut *c;
        crate::state::mark_gap(items.len(), marks, last_at);
        items.push(ChatItem::User { text: text.clone() });
        c.running = true;
    }
    let ctx = *ctx;
    spawn_forever(async move {
        // Bound to a local first: a `peek()` guard used directly as the match
        // scrutinee stays alive across every arm, and the create path below
        // writes to the same signal — which aborted the app on the first
        // prompt of any new code session.
        let existing = ctx.code_chat.peek().session_id.clone();
        let sid = match existing {
            Some(sid) => sid,
            None => match client.create_session(&chat_id).await {
                Ok(s) => {
                    ctx.code_chat.clone().write().session_id = Some(s.id.clone());
                    s.id
                }
                Err(e) => {
                    ctx.code_chat.clone().write().running = false;
                    show_toast(&ctx, format!("Session create failed: {e}"));
                    return;
                }
            },
        };
        if let Err(e) = client.prompt_async(&chat_id, &sid, &text, None).await {
            ctx.code_chat.clone().write().running = false;
            show_toast(&ctx, format!("Prompt failed: {e}"));
        }
        write_cache(&ctx);
    });
    true
}

pub(crate) fn stop_code_turn(ctx: &AppCtx) {
    let chat = ctx.code_chat.peek();
    let (Some(chat_id), Some(sid)) = (chat.chat_id.clone(), chat.session_id.clone()) else {
        return;
    };
    drop(chat);
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let ctx = *ctx;
    spawn_forever(async move {
        if let Err(e) = client.abort(&chat_id, &sid).await {
            show_toast(&ctx, format!("Stop failed: {e}"));
        } else {
            ctx.code_chat.clone().write().running = false;
        }
    });
}

/// Answer a permission ask: `once` | `always` | `reject`.
pub(crate) fn answer_code_permission(
    ctx: &AppCtx,
    chat_id: String,
    perm: CodePermission,
    response: &str,
) {
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let response = response.to_string();
    ctx.code_permissions
        .clone()
        .write()
        .retain(|(cid, p)| !(cid == &chat_id && p.id == perm.id));
    let ctx = *ctx;
    spawn_forever(async move {
        if let Err(e) = client
            .reply_permission(&chat_id, &perm.session_id, &perm.id, &response)
            .await
        {
            show_toast(&ctx, format!("Permission reply failed: {e}"));
        }
    });
}

/// Fetch and render the session's cumulative diff into the chat state.
pub(crate) fn load_code_diff(ctx: &AppCtx) {
    let chat = ctx.code_chat.peek();
    let (Some(chat_id), Some(sid)) = (chat.chat_id.clone(), chat.session_id.clone()) else {
        show_toast(ctx, "No changes yet — the chat has no session");
        return;
    };
    drop(chat);
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let ctx = *ctx;
    spawn_forever(async move {
        match client.diff(&chat_id, &sid).await {
            Ok(v) => {
                let rendered = render_diff(&v);
                ctx.code_chat.clone().write().diff = Some(rendered);
            }
            Err(e) => show_toast(&ctx, format!("Diff failed: {e}")),
        }
    });
}

/// Render the `FileDiff[]` payload from `GET /session/:id/diff` as readable
/// text. Lenient on shape: common field names first, pretty JSON as the
/// fallback so a server change degrades visibly instead of blankly.
fn render_diff(v: &Value) -> String {
    let Some(files) = v.as_array() else {
        return serde_json::to_string_pretty(v).unwrap_or_default();
    };
    if files.is_empty() {
        return "No changes yet.".to_string();
    }
    let mut out = String::new();
    for f in files {
        let path = f
            .get("path")
            .or_else(|| f.get("file"))
            .or_else(|| f.get("filename"))
            .and_then(Value::as_str)
            .unwrap_or("(unknown file)");
        let adds = f.get("additions").and_then(Value::as_u64);
        let dels = f.get("deletions").and_then(Value::as_u64);
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(out, "── {path}");
        if let (Some(a), Some(d)) = (adds, dels) {
            let _ = write!(out, "  (+{a} −{d})");
        }
        out.push('\n');
        if let Some(patch) = f
            .get("patch")
            .or_else(|| f.get("diff"))
            .and_then(Value::as_str)
        {
            out.push_str(patch);
            if !patch.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/// The "Open PR" action is an instruction to the agent — git is its job
/// (push is permission-gated; the ask pops here when it runs).
pub(crate) fn request_pr(ctx: &AppCtx) {
    send_code_prompt(
        ctx,
        "Push this chat's branch and open a pull request for the work so far. \
         Summarize the changes in the PR body, then reply with the PR URL."
            .to_string(),
    );
}

/// Create a new code chat and open it, sending the task as the first prompt.
pub(crate) fn new_code_chat(ctx: &AppCtx, repo: String, task: String, model: Option<String>) {
    let Some(client) = ctx.code_client.peek().clone() else {
        show_toast(ctx, "Code plane not connected — check Settings");
        return;
    };
    let ctx = *ctx;
    spawn_forever(async move {
        show_toast(&ctx, format!("Preparing workspace for {repo}…"));
        match client.create_chat(&repo, &task, model.as_deref()).await {
            Ok(meta) => {
                refresh_code_chats(&ctx).await;
                open_code_chat(&ctx, meta);
                if !task.trim().is_empty() {
                    // First prompt after the open flow resolves the session.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    send_code_prompt(&ctx, task);
                }
            }
            Err(e) => show_toast(&ctx, format!("Create failed: {e}")),
        }
    });
}

pub(crate) fn delete_code_chat(ctx: &AppCtx, chat_id: String) {
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let ctx = *ctx;
    spawn_forever(async move {
        match client.delete_chat(&chat_id, true).await {
            Ok(()) => {
                let mut cache = ctx.code_cache;
                cache.write().chats.remove(&chat_id);
                refresh_code_chats(&ctx).await;
            }
            Err(e) => show_toast(&ctx, format!("Delete failed: {e}")),
        }
    });
}

// ----------------------------------------------------------------- cache

/// Write-through of the open chat's transcript into the persisted cache,
/// truncated and LRU-capped.
pub(crate) fn write_cache(ctx: &AppCtx) {
    let chat = ctx.code_chat.peek();
    let Some(chat_id) = chat.chat_id.clone() else {
        return;
    };
    let mut items = chat.items.clone();
    if items.len() > CACHE_MAX_ITEMS {
        items.drain(..items.len() - CACHE_MAX_ITEMS);
    }
    let entry = CachedChat {
        title: chat.title.clone(),
        session_id: chat.session_id.clone(),
        items,
        updated: now_secs(),
    };
    drop(chat);
    let mut cache = ctx.code_cache;
    let mut c = cache.write();
    c.chats.insert(chat_id, entry);
    while c.chats.len() > CACHE_MAX_CHATS {
        if let Some(oldest) = c
            .chats
            .iter()
            .min_by_key(|(_, v)| v.updated)
            .map(|(k, _)| k.clone())
        {
            c.chats.remove(&oldest);
        } else {
            break;
        }
    }
}

/// Human label for a chat's lifecycle status in the list.
pub(crate) fn status_label(meta: &ChatMeta, running_turn: bool) -> (&'static str, String) {
    match (meta.status.as_str(), running_turn) {
        ("running", true) => ("dot busy", "working".to_string()),
        ("running", false) => ("dot on", "idle".to_string()),
        ("stopped" | "absent", _) => ("dot off", "asleep".to_string()),
        (other, _) => ("dot err", other.to_string()),
    }
}
