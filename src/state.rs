//! App-wide state: connection lifecycle, the event pump that folds ACP
//! session updates into the chat transcript, and persisted settings.

use std::time::Duration;

use dioxus::prelude::*;
use goose_acp_client::{
    AcpClient, AcpEvent, ConnectConfig, MessageChunk, PermissionRequest, SessionInfo,
    SessionUpdate, ToolCallUpdate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Clone, Copy, PartialEq)]
pub enum Screen {
    Settings,
    Sessions,
    Chat,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Settings {
    pub server_url: String,
    pub secret_key: String,
    pub fingerprint: String,
    pub working_dir: String,
}

#[derive(Clone, PartialEq)]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected { agent: String },
    Failed(String),
}

impl ConnState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnState::Connected { .. })
    }
}

#[derive(Clone, PartialEq)]
pub enum ChatItem {
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
pub struct ChatState {
    pub session_id: Option<String>,
    pub cwd: String,
    pub title: String,
    pub items: Vec<ChatItem>,
    pub running: bool,
    pub loading: bool,
}

/// Context-window usage: (tokens used, context limit).
pub type Usage = (u64, u64);

#[derive(Clone, Copy)]
pub struct AppCtx {
    pub screen: Signal<Screen>,
    pub settings: Signal<Settings>,
    pub conn: Signal<ConnState>,
    pub client: Signal<Option<AcpClient>>,
    pub want_connected: Signal<bool>,
    pub sessions: Signal<Vec<SessionInfo>>,
    pub sessions_cursor: Signal<Option<String>>,
    pub sessions_loading: Signal<bool>,
    pub chat: Signal<ChatState>,
    pub permission: Signal<Option<PermissionRequest>>,
    pub usage: Signal<Option<Usage>>,
    pub toast: Signal<Option<String>>,
}

pub fn use_app_ctx_provider() -> AppCtx {
    let settings = dioxus_sdk_storage::use_persistent("settings", Settings::default);
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
        permission: use_signal(|| None),
        usage: use_signal(|| None),
        toast: use_signal(|| None),
    };
    use_context_provider(|| ctx);
    ctx
}

pub fn use_app_ctx() -> AppCtx {
    use_context()
}

pub fn show_toast(ctx: &AppCtx, message: impl Into<String>) {
    let mut toast = ctx.toast;
    let message = message.into();
    toast.set(Some(message.clone()));
    spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        // Only clear if nothing newer replaced it.
        if toast.peek().as_deref() == Some(message.as_str()) {
            toast.set(None);
        }
    });
}

fn connect_config(settings: &Settings) -> Result<ConnectConfig, String> {
    let fingerprint =
        goose_acp_client::parse_fingerprint(&settings.fingerprint).map_err(|e| e.to_string())?;
    Ok(ConnectConfig {
        base_url: settings.server_url.clone(),
        secret: settings.secret_key.clone(),
        fingerprint,
    })
}

/// Connect (or reconnect) using the saved settings. Returns true on success.
pub async fn establish(ctx: AppCtx) -> bool {
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
            spawn(pump(ctx, events));
            true
        }
        Err(e) => {
            conn.set(ConnState::Failed(e.to_string()));
            false
        }
    }
}

pub fn disconnect(ctx: &AppCtx) {
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
                permission.set(Some(request));
            }
            AcpEvent::RequestCancelled { request_id } => {
                let matches_open = permission
                    .peek()
                    .as_ref()
                    .map(|p| p.request_id == request_id)
                    .unwrap_or(false);
                if matches_open {
                    permission.set(None);
                }
            }
            AcpEvent::Disconnected { reason } => {
                client_slot.set(None);
                chat.write().running = false;
                permission.set(None);
                if ctx.want_connected.peek().clone() {
                    conn.set(ConnState::Failed(format!("Connection lost: {reason}")));
                    spawn(reconnect_loop(ctx));
                } else {
                    conn.set(ConnState::Disconnected);
                }
                break;
            }
        }
    }
}

async fn reconnect_loop(ctx: AppCtx) {
    for delay in [2u64, 4, 8, 15] {
        tokio::time::sleep(Duration::from_secs(delay)).await;
        if !ctx.want_connected.peek().clone() {
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
            if let Some(title) = info.title.clone() {
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
    let message_id = chunk.message_id.clone();
    let mut c = chat.write();

    // Append to the trailing bubble when it belongs to the same message.
    match (&kind, c.items.last_mut()) {
        (ChunkKind::Assistant, Some(ChatItem::Assistant { message_id: last_id, text: last }))
            if *last_id == message_id || message_id.is_none() =>
        {
            last.push_str(&text);
            return;
        }
        (ChunkKind::Thought, Some(ChatItem::Thought { message_id: last_id, text: last }))
            if *last_id == message_id || message_id.is_none() =>
        {
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
    let Some(ChatItem::Tool { title, kind, status, output, .. }) = c
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
pub async fn refresh_sessions(ctx: AppCtx, more: bool) {
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
pub fn open_session(ctx: AppCtx, info: SessionInfo) {
    let mut screen = ctx.screen;
    let mut chat = ctx.chat;
    let mut usage = ctx.usage;
    let cwd = info.cwd.clone().unwrap_or_else(|| "/".to_string());

    chat.set(ChatState {
        session_id: Some(info.session_id.clone()),
        cwd: cwd.clone(),
        title: info.display_title(),
        items: Vec::new(),
        running: false,
        loading: true,
    });
    usage.set(None);
    screen.set(Screen::Chat);

    spawn(async move {
        let Some(client) = ctx.client.peek().clone() else {
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
pub fn new_session(ctx: AppCtx) {
    let working_dir = ctx.settings.peek().working_dir.trim().to_string();
    if working_dir.is_empty() || !working_dir.starts_with('/') {
        show_toast(
            &ctx,
            "Set an absolute working directory (a path on the server) in Settings first",
        );
        return;
    }
    spawn(async move {
        let Some(client) = ctx.client.peek().clone() else {
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

/// Send the user's message and run the agent turn.
pub fn send_prompt(ctx: AppCtx, text: String) {
    let mut chat = ctx.chat;
    let Some(session_id) = chat.peek().session_id.clone() else {
        return;
    };
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(&ctx, "Not connected");
        return;
    };
    {
        let mut c = chat.write();
        c.items.push(ChatItem::User { text: text.clone() });
        c.running = true;
    }
    spawn(async move {
        let result = client.prompt(&session_id, &text).await;
        let mut chat = ctx.chat;
        if chat.peek().session_id.as_deref() == Some(session_id.as_str()) {
            chat.write().running = false;
        }
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
}

/// Human-friendly `updatedAt` (RFC3339 → "YYYY-MM-DD HH:MM").
pub fn short_timestamp(ts: &str) -> String {
    let date = ts.get(0..10).unwrap_or(ts);
    let time = ts.get(11..16).unwrap_or("");
    if time.is_empty() {
        date.to_string()
    } else {
        format!("{date} {time}")
    }
}
