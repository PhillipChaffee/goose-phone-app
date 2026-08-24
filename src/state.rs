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
    AcpClient, AcpEvent, ConfigOption, ConnectConfig, MessageChunk, PermissionRequest, SessionInfo,
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// A compile-time seed for a development build.
///
/// Installing over the app wipes its container, so every rebuild otherwise
/// means retyping four fields before anything can be tested against a local
/// server. Setting these when you build fills them in instead:
///
/// ```sh
/// GOOSE_DEV_SERVER_URL=http://127.0.0.1:3285 \
/// GOOSE_DEV_SECRET_KEY=mock-secret \
/// GOOSE_DEV_CODE_URL=http://127.0.0.1:4399 \
/// GOOSE_DEV_CODE_PASSWORD=... \
///   dx build --platform ios --no-default-features --features mobile
/// ```
///
/// A release build expands to an empty string no matter what was set, so a
/// development endpoint cannot ride along into one.
macro_rules! dev_seed {
    ($name:literal) => {{
        #[cfg(debug_assertions)]
        {
            option_env!($name).unwrap_or("").to_owned()
        }
        #[cfg(not(debug_assertions))]
        {
            String::new()
        }
    }};
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: dev_seed!("GOOSE_DEV_SERVER_URL"),
            secret_key: dev_seed!("GOOSE_DEV_SECRET_KEY"),
            fingerprint: String::new(),
            working_dir: dev_seed!("GOOSE_DEV_WORKING_DIR"),
            code_server_url: dev_seed!("GOOSE_DEV_CODE_URL"),
            code_password: dev_seed!("GOOSE_DEV_CODE_PASSWORD"),
        }
    }
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
        /// Files sent with this message. A field rather than an item of its
        /// own, so an attachment cannot shift the indices `marks` and
        /// `CodeChatState::part_index` hold into `items` — and because one
        /// message with three photos is one turn, not four.
        ///
        /// `serde(default)` is load-bearing the same way it is on `Settings`:
        /// a transcript cached by an older build has no such field, and a
        /// parse failure would take the whole cache down with it.
        #[serde(default)]
        attachments: Vec<crate::attach::Attachment>,
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
    /// Where a long pause happened: `(index of the item after the pause,
    /// unix seconds)`. Kept beside `items` rather than as an item of its own
    /// because `CodeChatState::part_index` holds indices into that vector and
    /// inserting into it would silently misroute streamed part updates.
    pub marks: Vec<(usize, i64)>,
    /// When the last item was appended, for deciding the above.
    pub last_at: i64,
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
    /// Session config the agent offers — provider, model, mode, thinking
    /// effort — with the model list already in it. Arrives with session/new
    /// and `session/load`, and is refreshed by `config_option_update`.
    pub config_options: Signal<Vec<ConfigOption>>,
    pub toast: Signal<Option<String>>,
    /// Files picked in the goose composer and not yet sent.
    ///
    /// On the context rather than in the view because the picker is one
    /// document-level listener installed at the app root (`src/attach.rs`
    /// says why the gesture has to live there), and it has to be able to hand
    /// what it read to a composer it does not own.
    pub attachments: Signal<Vec<crate::attach::PendingAttachment>>,
    /// The picker is reading `n` files for a composer. Held so the tray can
    /// say so: resizing three photos takes seconds, and a composer that just
    /// sits there is indistinguishable from one that lost the pick.
    pub attach_reading: Signal<Option<(crate::attach::AttachTarget, usize)>>,

    // ---- Code tab (per-chat OpenCode containers on the brain; src/code.rs) ----
    pub tab: Signal<Tab>,
    /// The navigation drawer. It replaced a bottom tab bar, which cost 100px
    /// of every screen to show two destinations.
    pub drawer_open: Signal<bool>,
    pub code_screen: Signal<crate::code::CodeScreen>,
    pub code_client: Signal<Option<opencode_client::CodeClient>>,
    pub code_conn: Signal<ConnState>,
    pub code_chats: Signal<Vec<opencode_client::ChatMeta>>,
    pub code_chats_loading: Signal<bool>,
    pub code_repos: Signal<Vec<opencode_client::RepoEntry>>,
    /// The chat servers' model catalogue — names, context windows and
    /// thinking-effort tiers. Fetched once, on the first open of the session
    /// settings sheet; nothing else needs it.
    pub code_models: Signal<Vec<opencode_client::ModelInfo>>,
    pub code_models_loading: Signal<bool>,
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
    /// The review screen's state for the open chat. Deliberately not a field
    /// on `CodeChatState`: the chat screen clones its whole state on every
    /// keystroke, and parsed whole-file patches are the largest thing this
    /// tab holds.
    pub code_diff: Signal<crate::code::DiffState>,
    /// Review screen: soft-wrap long code lines (the default) or scroll them
    /// horizontally. One switch for the whole screen rather than one per
    /// file, so flipping it does not leave a handful of independent scroll
    /// offsets to chase. Session-scoped on purpose — it is an escape hatch
    /// for a particular diff, not a setting.
    pub code_diff_wrap: Signal<bool>,
    /// What is typed in the code composer but not yet sent.
    ///
    /// Not component-local, because the review screen is a screen: opening it
    /// unmounts `CodeChatView` and a `use_signal` draft dies with the scope.
    /// The one workflow the review screen exists for — type a correction, go
    /// check what the agent actually changed, come back and send — was the
    /// one that silently lost what you had written.
    pub code_draft: Signal<String>,
    /// Files picked in the code composer and not yet sent. Separate from
    /// `attachments` for the same reason `code_draft` is separate from the
    /// goose draft: they are different conversations.
    pub code_attachments: Signal<Vec<crate::attach::PendingAttachment>>,
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
        config_options: use_signal(Vec::new),
        toast: use_signal(|| None),
        attachments: use_signal(Vec::new),
        attach_reading: use_signal(|| None),
        tab: use_signal(|| Tab::Home),
        drawer_open: use_signal(|| false),
        code_screen: use_signal(|| crate::code::CodeScreen::List),
        code_client: use_signal(|| None),
        code_conn: use_signal(|| ConnState::Disconnected),
        code_chats: use_signal(Vec::new),
        code_chats_loading: use_signal(|| false),
        code_repos: use_signal(Vec::new),
        code_models: use_signal(Vec::new),
        code_models_loading: use_signal(|| false),
        code_chat: use_signal(crate::code::CodeChatState::default),
        code_permissions: use_signal(Vec::new),
        code_cache,
        code_epoch: use_signal(|| 0),
        code_diff: use_signal(crate::code::DiffState::default),
        code_diff_wrap: use_signal(|| true),
        code_draft: use_signal(String::new),
        code_attachments: use_signal(Vec::new),
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
    let fingerprint = goose_acp_client::parse_fingerprint(&settings.fingerprint)?;
    Ok(ConnectConfig {
        base_url: settings.server_url.clone(),
        secret: settings.secret_key.clone(),
        fingerprint,
    })
}

/// Connect (or reconnect) using the saved settings. Returns true on success.
pub(crate) async fn establish(ctx: &AppCtx) -> bool {
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
            let ctx = *ctx;
            spawn_forever(async move { pump(&ctx, events).await });
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
async fn pump(ctx: &AppCtx, mut events: mpsc::Receiver<AcpEvent>) {
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
                    let ctx = *ctx;
                    spawn_forever(async move { reconnect_loop(&ctx).await });
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
async fn reconnect_loop(ctx: &AppCtx) {
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

async fn reload_chat(ctx: &AppCtx, session_id: String, cwd: String) {
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
        match result {
            Ok(raw) => {
                let opts = goose_acp_client::config_options_from(&raw);
                if !opts.is_empty() {
                    ctx.config_options.clone().set(opts);
                }
            }
            Err(e) => show_toast(ctx, format!("Failed to reload session: {e}")),
        }
    }
}

fn apply_update(ctx: &AppCtx, session_id: &str, update: SessionUpdate) {
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
            let ChatState {
                items,
                marks,
                last_at,
                ..
            } = &mut *c;
            mark_gap(items.len(), marks, last_at);
            items.push(ChatItem::Tool {
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
            apply_tool_update(&mut chat, &update);
        }
        SessionUpdate::ConfigOptionUpdate(raw) => {
            // The agent pushes this after every change, including ones made
            // from another client, so the picker never shows a stale model.
            let opts = goose_acp_client::config_options_from(&raw);
            if !opts.is_empty() {
                ctx.config_options.clone().set(opts);
            }
        }
        SessionUpdate::SessionInfoUpdate(info) => {
            if let Some(title) = info.title {
                if is_current {
                    chat.write().title.clone_from(&title);
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

#[derive(Clone, Copy)]
enum ChunkKind {
    User,
    Assistant,
    Thought,
}

fn push_chunk(chat: &mut Signal<ChatState>, chunk: MessageChunk, kind: ChunkKind) {
    // A user chunk that is not text is a file they attached, and it belongs
    // beside the message rather than inside it as "[image: image/jpeg]",
    // which is all `text_repr` could ever say about it. Agent chunks keep the
    // placeholder: an image the agent sends is not an attachment on a turn
    // the reader made.
    if matches!(kind, ChunkKind::User) && chunk.content.is_attachment() {
        push_user_attachment(chat, &chunk.content);
        return;
    }
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
        (ChunkKind::User, Some(ChatItem::User { text: last, .. })) if message_id.is_none() => {
            last.push_str(&text);
            return;
        }
        _ => {}
    }

    let ChatState {
        items,
        marks,
        last_at,
        ..
    } = &mut *c;
    mark_gap(items.len(), marks, last_at);
    items.push(match kind {
        ChunkKind::User => ChatItem::User {
            text,
            attachments: Vec::new(),
        },
        ChunkKind::Assistant => ChatItem::Assistant { message_id, text },
        ChunkKind::Thought => ChatItem::Thought { message_id, text },
    });
}

/// Hang a replayed attachment off the user's turn.
///
/// goose sends the message's blocks as separate chunks, and the text one
/// arrives first, so the bubble is normally already there. When it is not —
/// a message that is nothing but a photo — the attachment opens one.
fn push_user_attachment(chat: &mut Signal<ChatState>, block: &goose_acp_client::ContentBlock) {
    let record = crate::attach::from_content_block(block);
    let mut c = chat.write();
    if let Some(ChatItem::User { attachments, .. }) = c.items.last_mut() {
        attachments.push(record);
        return;
    }
    let ChatState {
        items,
        marks,
        last_at,
        ..
    } = &mut *c;
    mark_gap(items.len(), marks, last_at);
    items.push(ChatItem::User {
        text: String::new(),
        attachments: vec![record],
    });
}

fn apply_tool_update(chat: &mut Signal<ChatState>, update: &ToolCallUpdate) {
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
pub(crate) async fn refresh_sessions(ctx: &AppCtx, more: bool) {
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
        Err(e) => show_toast(ctx, format!("Failed to list sessions: {e}")),
    }
    loading.set(false);
}

/// Open an existing session: switch to the chat screen and replay history.
pub(crate) fn open_session(ctx: &AppCtx, info: SessionInfo) {
    let mut screen = ctx.screen;
    let mut chat = ctx.chat;
    let mut usage = ctx.usage;
    let cwd = info.cwd.clone().unwrap_or_else(|| "/".to_string());
    let running = ctx.running_sessions.peek().contains(&info.session_id);

    // An attachment belongs to the message it was picked for. The draft is
    // component-local and dies with the screen; the tray lives on the context
    // (the picker has to be able to reach it from the app root), so it has to
    // be told.
    ctx.attachments.clone().set(Vec::new());
    chat.set(ChatState {
        marks: Vec::new(),
        last_at: 0,
        session_id: Some(info.session_id.clone()),
        cwd: cwd.clone(),
        title: info.display_title(),
        items: Vec::new(),
        running,
        loading: true,
    });
    usage.set(None);
    screen.set(Screen::Chat);

    let ctx = *ctx;
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
            match result {
                Ok(raw) => {
                    let opts = goose_acp_client::config_options_from(&raw);
                    if !opts.is_empty() {
                        ctx.config_options.clone().set(opts);
                    }
                }
                Err(e) => show_toast(&ctx, format!("Failed to load session: {e}")),
            }
        }
    });
}

/// Create a fresh session in the configured working directory and open it.
pub(crate) fn new_session(ctx: &AppCtx) {
    let working_dir = ctx.settings.peek().working_dir.trim().to_string();
    if working_dir.is_empty() || !working_dir.starts_with('/') {
        show_toast(
            ctx,
            "Set an absolute working directory (a path on the server) in Settings first",
        );
        return;
    }
    let ctx = *ctx;
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
                ctx.config_options.clone().set(resp.config_options);
                ctx.attachments.clone().set(Vec::new());
                chat.set(ChatState {
                    marks: Vec::new(),
                    last_at: 0,
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

/// Send the user's message, with whatever they attached to it, and run the
/// agent turn. Returns false (leaving the caller's draft and attachments
/// untouched) if the message could not be submitted.
///
/// `files` is passed in rather than read off the context so that a message
/// the *app* composes carries nothing: the caller decides.
pub(crate) fn send_prompt(
    ctx: &AppCtx,
    text: String,
    files: &[crate::attach::PendingAttachment],
) -> bool {
    let mut chat = ctx.chat;
    let Some(session_id) = chat.peek().session_id.clone() else {
        return false;
    };
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(ctx, "Not connected — reconnect in Settings");
        return false;
    };
    let blocks = crate::attach::goose_blocks(&text, files);
    if blocks.is_empty() {
        return false;
    }
    {
        let mut c = chat.write();
        let ChatState {
            items,
            marks,
            last_at,
            ..
        } = &mut *c;
        mark_gap(items.len(), marks, last_at);
        items.push(ChatItem::User {
            text,
            attachments: crate::attach::records(files),
        });
        c.running = true;
    }
    let mut running_sessions = ctx.running_sessions;
    running_sessions.write().insert(session_id.clone());

    let ctx = *ctx;
    spawn_forever(async move {
        let result = client.prompt(&session_id, &blocks).await;

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
pub(crate) fn stop_turn(ctx: &AppCtx) {
    let Some(session_id) = ctx.chat.peek().session_id.clone() else {
        return;
    };
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    answer_pending_permissions(ctx, &client, &session_id);
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
pub(crate) fn answer_permission(ctx: &AppCtx, request_id: &Value, option_id: Option<String>) {
    if let Some(client) = ctx.client.peek().clone() {
        client.respond_permission(request_id.clone(), option_id);
    }
    let mut permission = ctx.permission;
    permission
        .write()
        .retain(|req| req.request_id != *request_id);
}

/// A pause long enough that the reader wants to know when things resumed.
/// Ten minutes: shorter and an ordinary think-time would litter the
/// transcript with rules.
const GAP_SECS: i64 = 600;

/// Unix seconds now.
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().cast_signed())
}

/// Record a time mark before the item about to be appended at `items_len`,
/// if enough time has passed since the last one.
///
/// Only live appends are stamped. History arrives from the server in one
/// burst with no per-message times, so a replayed transcript carries no
/// marks — which is honest: we do not know when those turns happened.
pub(crate) fn mark_gap(items_len: usize, marks: &mut Vec<(usize, i64)>, last_at: &mut i64) {
    let now = now_secs();
    if *last_at != 0 && now - *last_at >= GAP_SECS {
        marks.push((items_len, now));
    }
    *last_at = now;
}

/// Days since the Unix epoch for a civil date, by Howard Hinnant's
/// `days_from_civil`. Written out rather than pulling in a date crate: this
/// and `relative_time` below are the only date arithmetic the app does.
const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix seconds for an RFC3339 timestamp, or `None` if it is not one.
/// Offsets are ignored — every timestamp the servers send is UTC, and an
/// hour of drift cannot change which side of a "2h" boundary a row lands on
/// in a way anybody would notice.
pub(crate) fn rfc3339_to_epoch(ts: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| ts.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm) = (num(11..13).unwrap_or(0), num(14..16).unwrap_or(0));
    let ss = num(17..19).unwrap_or(0);
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3_600 + mm * 60 + ss)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Age of a floating-point Unix timestamp, as the `OpenCode` API reports it.
/// The fractional part is sub-second and the value is far inside i64, so the
/// saturating cast cannot lose anything that matters here.
#[expect(
    clippy::cast_possible_truncation,
    reason = "sub-second precision is irrelevant to a row badge, and the               value is many orders of magnitude inside i64"
)]
pub(crate) fn relative_time_secs(epoch: f64) -> String {
    relative_time(epoch as i64)
}

/// Age of `epoch` as a list-row badge: "now", "5m", "2h", "3d", then a date.
/// Recent things get a duration because that is what you are tracking; old
/// things get a date because "47d" is not a fact anybody holds in their head.
pub(crate) fn relative_time(epoch: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().cast_signed());
    let age = now - epoch;
    match age {
        ..60 => "now".to_owned(),
        60..3_600 => format!("{}m", age / 60),
        3_600..86_400 => format!("{}h", age / 3_600),
        86_400..604_800 => format!("{}d", age / 86_400),
        _ => {
            // Walk back from the epoch day to a civil date for the label.
            let mut days = epoch.div_euclid(86_400) + 719_468;
            let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
            days -= era * 146_097;
            let doy = days - (365 * (days / 365) + (days / 365) / 4 - (days / 365) / 100);
            let yoe = (days - doy / 365) / 365;
            let doy = days - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            MONTHS
                .get(usize::try_from(m - 1).unwrap_or(0))
                .map_or_else(|| "earlier".to_owned(), |name| format!("{name} {d}"))
        }
    }
}

/// Set one session config option, whichever one the sheet was pointed at.
///
/// Deliberately id-agnostic: goose routes `provider`, `mode`, `model` and
/// `thinking_effort` and rejects anything else, so the list of what is
/// settable is the agent's to state and this app's to relay.
///
/// The agent applies it to the session immediately and answers with the full
/// option set, which is also pushed as a `config_option_update`; both paths
/// land in `ctx.config_options`, so whichever arrives first wins and they
/// agree.
pub(crate) fn set_config_option(ctx: &AppCtx, config_id: &str, value: &str) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let Some(session_id) = ctx.chat.peek().session_id.clone() else {
        return;
    };
    let (ctx, config_id, value) = (*ctx, config_id.to_owned(), value.to_owned());
    spawn_forever(async move {
        match client
            .set_config_option(&session_id, &config_id, &value)
            .await
        {
            Ok(opts) if !opts.is_empty() => ctx.config_options.clone().set(opts),
            Ok(_) => {}
            Err(e) => show_toast(&ctx, format!("Could not switch: {e}")),
        }
    });
}
