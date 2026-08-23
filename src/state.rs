//! App-wide state: connection lifecycle, the event pump that folds ACP
//! session updates into the chat transcript, and persisted settings.
//!
//! Every task that must outlive the screen that started it (the event pump,
//! reconnects, RPC calls that navigate, toast timers) is spawned with
//! `spawn_forever` onto the root scope — a plain `spawn` belongs to the
//! current component and is cancelled when that screen unmounts.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{
    AcpClient, AcpEvent, ConnectConfig, MessageChunk, PermissionRequest, SessionInfo,
    SessionUpdate, ToolCallUpdate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Settings,
    Sessions,
    Chat,
}

/// Top-level tab — the Claude-app-style Home/Code toggle. Each tab keeps its
/// own navigation state (`AppCtx::screen` for Home, `AppCtx::code_screen`
/// for Code), so switching tabs never resets where you were.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Home,
    Code,
}

/// `serde(default)` is load-bearing: settings persisted by older builds lack
/// the code-agent fields, and a parse failure would silently wipe the saved
/// goose server config (the storage layer falls back to `Default`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct Settings {
    pub server_url: String,
    pub secret_key: String,
    pub fingerprint: String,
    pub working_dir: String,
    /// Code-agent gateway on the brain, e.g. `https://brain.tailnet.ts.net:4300`.
    pub code_server_url: String,
    /// `OPENCODE_SERVER_PASSWORD`.
    pub code_password: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ConnState {
    Disconnected,
    Connecting,
    Connected { agent: String },
    Failed(String),
}

impl ConnState {
    pub(crate) const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// One rendered transcript item. Serde derives exist for the Code tab's
/// on-device transcript cache (issue #2, A11) — goose chats are never cached.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ChatItem {
    User {
        text: String,
    },
    Assistant {
        message_id: Option<String>,
        text: String,
    },
    Thought {
        message_id: Option<String>,
        text: String,
    },
    Tool {
        id: String,
        title: String,
        kind: String,
        status: String,
        output: String,
    },
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct ChatState {
    pub session_id: Option<String>,
    pub cwd: String,
    pub title: String,
    pub items: Vec<ChatItem>,
    pub running: bool,
    pub loading: bool,
}

/// Context-window usage: (tokens used, context limit).
pub(crate) type Usage = (u64, u64);

#[derive(Clone, Copy)]
pub(crate) struct AppCtx {
    pub screen: Signal<Screen>,
    pub settings: Signal<Settings>,
    pub conn: Signal<ConnState>,
    pub client: Signal<Option<AcpClient>>,
    pub want_connected: Signal<bool>,
    pub sessions: Signal<Vec<SessionInfo>>,
    pub sessions_cursor: Signal<Option<String>>,
    pub sessions_loading: Signal<bool>,
    pub chat: Signal<ChatState>,
    /// Sessions with a turn currently in flight (client-side view).
    pub running_sessions: Signal<HashSet<String>>,
    /// FIFO of unanswered `session/request_permission` requests. Every entry
    /// MUST eventually be answered (or the transport dropped) — the agent's
    /// turn blocks on it.
    pub permission: Signal<Vec<PermissionRequest>>,
    pub usage: Signal<Option<Usage>>,
    pub toast: Signal<Option<String>>,

    // ---- Code tab (per-chat OpenCode containers on the brain; src/code.rs) ----
    pub tab: Signal<Tab>,
    pub code_screen: Signal<crate::code::CodeScreen>,
    pub code_client: Signal<Option<opencode_client::CodeClient>>,
    pub code_conn: Signal<ConnState>,
    pub code_chats: Signal<Vec<opencode_client::ChatMeta>>,
    pub code_chats_loading: Signal<bool>,
    pub code_repos: Signal<Vec<opencode_client::RepoEntry>>,
    pub code_chat: Signal<crate::code::CodeChatState>,
    /// Pending permission asks from code chats, tagged by chat id. A separate
    /// queue from `permission` by construction: goose and `OpenCode` ids can
    /// never collide or be cross-answered.
    pub code_permissions: Signal<Vec<(String, opencode_client::CodePermission)>>,
    /// On-device transcript cache — instant open while a chat's container
    /// wakes, read-only offline. Server history stays authoritative.
    pub code_cache: Signal<crate::code::CodeCache>,
    /// Bumped whenever a different code chat is opened; stale SSE pumps and
    /// poll loops observe the change and exit.
    pub code_epoch: Signal<u64>,
}

pub(crate) fn use_app_ctx_provider() -> AppCtx {
    let settings = dioxus_sdk_storage::use_persistent("settings", Settings::default);
    let code_cache =
        dioxus_sdk_storage::use_persistent("code_cache", crate::code::CodeCache::default);
    let ctx = AppCtx {
        // Always start on Settings; connecting is an explicit user action.
        screen: use_signal(|| Screen::Settings),
        settings,
        conn: use_signal(|| ConnState::Disconnected),
        client: use_signal(|| None),
        want_connected: use_signal(|| false),
        sessions: use_signal(Vec::new),
        sessions_cursor: use_signal(|| None),
        sessions_loading: use_signal(|| false),
        chat: use_signal(ChatState::default),
        running_sessions: use_signal(HashSet::new),
        permission: use_signal(Vec::new),
        usage: use_signal(|| None),
        toast: use_signal(|| None),
        tab: use_signal(|| Tab::Home),
        code_screen: use_signal(|| crate::code::CodeScreen::List),
        code_client: use_signal(|| None),
        code_conn: use_signal(|| ConnState::Disconnected),
        code_chats: use_signal(Vec::new),
        code_chats_loading: use_signal(|| false),
        code_repos: use_signal(Vec::new),
        code_chat: use_signal(crate::code::CodeChatState::default),
        code_permissions: use_signal(Vec::new),
        code_cache,
        code_epoch: use_signal(|| 0),
    };
    use_context_provider(|| ctx);
    ctx
}

pub(crate) fn use_app_ctx() -> AppCtx {
    use_context()
}

static TOAST_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn show_toast(ctx: &AppCtx, message: impl Into<String>) {
    let mut toast = ctx.toast;
    let id = TOAST_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    toast.set(Some(message.into()));
    spawn_forever(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        // Only clear if no newer toast was shown since.
        if TOAST_SEQ.load(Ordering::Relaxed) == id {
            toast.set(None);
        }
    });
}

fn connect_config(settings: &Settings) -> Result<ConnectConfig, String> {
    let fingerprint =
        goose_acp_client::parse_fingerprint(&settings.fingerprint)?;
    Ok(ConnectConfig {
        base_url: settings.server_url.clone(),
        secret: settings.secret_key.clone(),
        fingerprint,
    })
}

/// Connect (or reconnect) using the saved settings. Returns true on success.
pub(crate) async fn establish(ctx: AppCtx) -> bool {
    let mut conn = ctx.conn;
    let mut client_slot = ctx.client;
    let mut want = ctx.want_connected;

    let cfg = match connect_config(&ctx.settings.peek()) {
        Ok(cfg) => cfg,
        Err(e) => {
            conn.set(ConnState::Failed(e));
            return false;
        }
    };

    // Drop any previous connection first.
    if let Some(old) = client_slot.peek().clone() {
        old.close();
    }
    conn.set(ConnState::Connecting);

    match AcpClient::connect(&cfg).await {
        Ok((client, events, info)) => {
            client_slot.set(Some(client));
            want.set(true);
            let agent = if info.agent_version.is_empty() {
                info.agent_name
            } else {
                format!("{} {}", info.agent_name, info.agent_version)
            };
            conn.set(ConnState::Connected { agent });
            spawn_forever(pump(ctx, events));
            true
        }
        Err(e) => {
            conn.set(ConnState::Failed(e.to_string()));
            false
        }
    }
}

pub(crate) fn disconnect(ctx: &AppCtx) {
    let mut want = ctx.want_connected;
    want.set(false);
    if let Some(client) = ctx.client.peek().clone() {
        client.close();
    }
}

/// Read events off the connection and fold them into UI state.
async fn pump(ctx: AppCtx, mut events: mpsc::Receiver<AcpEvent>) {
    let mut chat = ctx.chat;
    let mut permission = ctx.permission;
    let mut usage = ctx.usage;
    let mut conn = ctx.conn;
    let mut client_slot = ctx.client;
    let mut running_sessions = ctx.running_sessions;

    while let Some(event) = events.recv().await {
        match event {
            AcpEvent::Update { session_id, update } => {
                apply_update(ctx, &session_id, update);
            }
            AcpEvent::GooseUpdate { session_id, update } => {
                let is_current = chat.peek().session_id.as_deref() == Some(session_id.as_str());
                if !is_current {
                    continue;
                }
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("usage_update") {
                    let used = update.get("used").and_then(Value::as_u64);
                    let limit = update.get("contextLimit").and_then(Value::as_u64);
                    if let (Some(used), Some(limit)) = (used, limit) {
                        usage.set(Some((used, limit)));
                    }
                }
            }
            AcpEvent::Permission(request) => {
                // Queue, never replace: every request must be answered or the
                // agent's turn hangs.
                permission.write().push(request);
            }
            AcpEvent::RequestCancelled { request_id } => {
                permission.write().retain(|p| p.request_id != request_id);
            }
            AcpEvent::Disconnected { reason } => {
                client_slot.set(None);
                chat.write().running = false;
                running_sessions.write().clear();
                // Transport is gone; the server resolves its own pending
                // permission requests via the transport-error path.
                permission.write().clear();
                if *ctx.want_connected.peek() {
                    conn.set(ConnState::Failed(format!("Connection lost: {reason}")));
                    spawn_forever(reconnect_loop(ctx));
                } else {
                    conn.set(ConnState::Disconnected);
                }
                break;
            }
        }
    }
}

/// Retry until connected or the user disconnects: quick ramp, then a steady
/// 30-second cadence (covers long VPN outages and phone sleep — suspended
/// timers resume on wake).
async fn reconnect_loop(ctx: AppCtx) {
    let mut ramp = [2u64, 4, 8, 15].into_iter();
    loop {
        let delay = ramp.next().unwrap_or(30);
        tokio::time::sleep(Duration::from_secs(delay)).await;
        if !*ctx.want_connected.peek() {
            return;
        }
        if ctx.conn.peek().is_connected() {
            return;
        }
        if establish(ctx).await {
            // Replay the open chat so the transcript is rebuilt.
            let (session_id, cwd) = {
                let chat = ctx.chat.peek();
                (chat.session_id.clone(), chat.cwd.clone())
            };
            if let Some(session_id) = session_id {
                if *ctx.screen.peek() == Screen::Chat {
                    reload_chat(ctx, session_id, cwd).await;
                }
            }
            return;
        }
    }
}

async fn reload_chat(ctx: AppCtx, session_id: String, cwd: String) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let mut chat = ctx.chat;
    {
        let mut c = chat.write();
        c.items.clear();
        c.loading = true;
    }
    let result = client.session_load(&session_id, &cwd).await;
    if chat.peek().session_id.as_deref() == Some(session_id.as_str()) {
        chat.write().loading = false;
        if let Err(e) = result {
            show_toast(&ctx, format!("Failed to reload session: {e}"));
        }
    }
}

fn apply_update(ctx: AppCtx, session_id: &str, update: SessionUpdate) {
    let mut chat = ctx.chat;
    let is_current = chat.peek().session_id.as_deref() == Some(session_id);

    match update {
        SessionUpdate::AgentMessageChunk(chunk) if is_current => {
            push_chunk(&mut chat, chunk, ChunkKind::Assistant);
        }
        SessionUpdate::AgentThoughtChunk(chunk) if is_current => {
            push_chunk(&mut chat, chunk, ChunkKind::Thought);
        }
        SessionUpdate::UserMessageChunk(chunk) if is_current => {
            push_chunk(&mut chat, chunk, ChunkKind::User);
        }
        SessionUpdate::ToolCall(call) if is_current => {
            let mut c = chat.write();
            c.items.push(ChatItem::Tool {
                id: call.tool_call_id.clone(),
                title: call
                    .title
                    .clone()
                    .or_else(|| call.tool_name().map(str::to_string))
                    .unwrap_or_else(|| "tool".to_string()),
                kind: call.kind.clone().unwrap_or_else(|| "other".to_string()),
                status: call.status.clone().unwrap_or_else(|| "pending".to_string()),
                output: call.content_text(),
            });
        }
        SessionUpdate::ToolCallUpdate(update) if is_current => {
            apply_tool_update(&mut chat, update);
        }
        SessionUpdate::SessionInfoUpdate(info) => {
            if let Some(title) = info.title {
                if is_current {
                    chat.write().title = title.clone();
                }
                let mut sessions = ctx.sessions;
                let mut list = sessions.write();
                if let Some(entry) = list.iter_mut().find(|s| s.session_id == session_id) {
                    entry.title = Some(title);
                }
            }
        }
        _ => {}
    }
}

enum ChunkKind {
    User,
    Assistant,
    Thought,
}

fn push_chunk(chat: &mut Signal<ChatState>, chunk: MessageChunk, kind: ChunkKind) {
    let text = chunk.content.text_repr();
    if text.is_empty() {
        return;
    }
    let message_id = chunk.message_id;
    let mut c = chat.write();

    // Append to the trailing bubble when it belongs to the same message.
    match (&kind, c.items.last_mut()) {
        (
            ChunkKind::Assistant,
            Some(ChatItem::Assistant {
                message_id: last_id,
                text: last,
            }),
        ) if *last_id == message_id || message_id.is_none() => {
            last.push_str(&text);
            return;
        }
        (
            ChunkKind::Thought,
            Some(ChatItem::Thought {
                message_id: last_id,
                text: last,
            }),
        ) if *last_id == message_id || message_id.is_none() => {
            last.push_str(&text);
            return;
        }
        (ChunkKind::User, Some(ChatItem::User { text: last })) if message_id.is_none() => {
            last.push_str(&text);
            return;
        }
        _ => {}
    }

    c.items.push(match kind {
        ChunkKind::User => ChatItem::User { text },
        ChunkKind::Assistant => ChatItem::Assistant { message_id, text },
        ChunkKind::Thought => ChatItem::Thought { message_id, text },
    });
}

fn apply_tool_update(chat: &mut Signal<ChatState>, update: ToolCallUpdate) {
    let mut c = chat.write();
    let Some(ChatItem::Tool {
        title,
        kind,
        status,
        output,
        ..
    }) = c
        .items
        .iter_mut()
        .rev()
        .find(|item| matches!(item, ChatItem::Tool { id, .. } if *id == update.tool_call_id))
    else {
        return;
    };
    if let Some(t) = update.title.clone() {
        *title = t;
    }
    if let Some(k) = update.kind.clone() {
        *kind = k;
    }
    if let Some(s) = update.status.clone() {
        *status = s;
    }
    let text = update.content_text();
    if !text.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&text);
    } else if output.is_empty() {
        if let Some(raw) = update.raw_output.as_ref() {
            if let Some(s) = raw.as_str() {
                output.push_str(s);
            } else if raw.is_object() || raw.is_array() {
                *output = serde_json::to_string_pretty(raw).unwrap_or_default();
            }
        }
    }
}

/// Fetch the first page of sessions (or the next page when `more` is true).
pub(crate) async fn refresh_sessions(ctx: AppCtx, more: bool) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let mut sessions = ctx.sessions;
    let mut cursor = ctx.sessions_cursor;
    let mut loading = ctx.sessions_loading;

    let page_cursor = if more { cursor.peek().clone() } else { None };
    loading.set(true);
    match client.session_list(page_cursor).await {
        Ok(page) => {
            cursor.set(page.next_cursor.clone());
            if more {
                sessions.write().extend(page.sessions);
            } else {
                sessions.set(page.sessions);
            }
        }
        Err(e) => show_toast(&ctx, format!("Failed to list sessions: {e}")),
    }
    loading.set(false);
}

/// Open an existing session: switch to the chat screen and replay history.
pub(crate) fn open_session(ctx: AppCtx, info: SessionInfo) {
    let mut screen = ctx.screen;
    let mut chat = ctx.chat;
    let mut usage = ctx.usage;
    let cwd = info.cwd.clone().unwrap_or_else(|| "/".to_string());
    let running = ctx.running_sessions.peek().contains(&info.session_id);

    chat.set(ChatState {
        session_id: Some(info.session_id.clone()),
        cwd: cwd.clone(),
        title: info.display_title(),
        items: Vec::new(),
        running,
        loading: true,
    });
    usage.set(None);
    screen.set(Screen::Chat);

    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            let mut chat = ctx.chat;
            if chat.peek().session_id.as_deref() == Some(info.session_id.as_str()) {
                chat.write().loading = false;
            }
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        let result = client.session_load(&info.session_id, &cwd).await;
        let mut chat = ctx.chat;
        if chat.peek().session_id.as_deref() == Some(info.session_id.as_str()) {
            chat.write().loading = false;
            if let Err(e) = result {
                show_toast(&ctx, format!("Failed to load session: {e}"));
            }
        }
    });
}

/// Create a fresh session in the configured working directory and open it.
pub(crate) fn new_session(ctx: AppCtx) {
    let working_dir = ctx.settings.peek().working_dir.trim().to_string();
    if working_dir.is_empty() || !working_dir.starts_with('/') {
        show_toast(
            &ctx,
            "Set an absolute working directory (a path on the server) in Settings first",
        );
        return;
    }
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.session_new(&working_dir).await {
            Ok(resp) => {
                let mut chat = ctx.chat;
                let mut screen = ctx.screen;
                let mut usage = ctx.usage;
                chat.set(ChatState {
                    session_id: Some(resp.session_id),
                    cwd: working_dir,
                    title: "New chat".to_string(),
                    items: Vec::new(),
                    running: false,
                    loading: false,
                });
                usage.set(None);
                screen.set(Screen::Chat);
            }
            Err(e) => show_toast(&ctx, format!("Failed to create session: {e}")),
        }
    });
}

/// Send the user's message and run the agent turn. Returns false (leaving the
/// caller's draft untouched) if the message could not be submitted.
pub(crate) fn send_prompt(ctx: AppCtx, text: String) -> bool {
    let mut chat = ctx.chat;
    let Some(session_id) = chat.peek().session_id.clone() else {
        return false;
    };
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(&ctx, "Not connected — reconnect in Settings");
        return false;
    };
    {
        let mut c = chat.write();
        c.items.push(ChatItem::User { text: text.clone() });
        c.running = true;
    }
    let mut running_sessions = ctx.running_sessions;
    running_sessions.write().insert(session_id.clone());

    spawn_forever(async move {
        let result = client.prompt(&session_id, &text).await;

        let mut running_sessions = ctx.running_sessions;
        running_sessions.write().remove(&session_id);
        let mut chat = ctx.chat;
        if chat.peek().session_id.as_deref() == Some(session_id.as_str()) {
            chat.write().running = false;
        }
        // The turn is over; answer any permission prompt it left behind
        // (covers error paths where the run dies with a request outstanding).
        answer_pending_permissions(&ctx, &client, &session_id);

        match result {
            Ok(stop) => match stop.as_str() {
                "end_turn" | "cancelled" => {}
                "max_tokens" => show_toast(&ctx, "Stopped: max tokens reached"),
                "refusal" => show_toast(&ctx, "The agent declined to continue"),
                other => show_toast(&ctx, format!("Turn ended: {other}")),
            },
            Err(e) => show_toast(&ctx, format!("Prompt failed: {e}")),
        }
    });
    true
}

/// Stop the current chat's running turn. Any open permission prompt for the
/// session is answered "cancelled" FIRST (the frames travel in order, so the
/// parked run unparks and then observes the cancel), matching goose Desktop.
pub(crate) fn stop_turn(ctx: AppCtx) {
    let Some(session_id) = ctx.chat.peek().session_id.clone() else {
        return;
    };
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    answer_pending_permissions(&ctx, &client, &session_id);
    client.cancel(&session_id);
}

/// Respond "cancelled" to every queued permission request for `session_id`
/// and drop them from the queue.
fn answer_pending_permissions(ctx: &AppCtx, client: &AcpClient, session_id: &str) {
    let mut permission = ctx.permission;
    permission.write().retain(|req| {
        if req.session_id == session_id {
            client.respond_permission(req.request_id.clone(), None);
            false
        } else {
            true
        }
    });
}

/// Answer the front-of-queue permission request and remove it.
pub(crate) fn answer_permission(ctx: AppCtx, request_id: Value, option_id: Option<String>) {
    if let Some(client) = ctx.client.peek().clone() {
        client.respond_permission(request_id.clone(), option_id);
    }
    let mut permission = ctx.permission;
    permission
        .write()
        .retain(|req| req.request_id != request_id);
}

/// Human-friendly `updatedAt` (RFC3339 → "YYYY-MM-DD HH:MM").
pub(crate) fn short_timestamp(ts: &str) -> String {
    let date = ts.get(0..10).unwrap_or(ts);
    let time = ts.get(11..16).unwrap_or("");
    if time.is_empty() {
        date.to_string()
    } else {
        format!("{date} {time}")
    }
}
