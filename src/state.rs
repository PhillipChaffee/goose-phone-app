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
    AcpClient, AcpError, AcpEvent, ConfigOption, ConnectConfig, MessageChunk, PermissionRequest,
    SessionInfo, SessionKind, SessionListResponse, SessionQuery, SessionUpdate, ToolCallUpdate,
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

/// Which destination's stack is on screen. Each tab keeps its own navigation
/// state (`AppCtx::screen` for Home, `AppCtx::code_screen` for Code), so
/// switching tabs never resets where you were.
///
/// The routing that reads this is `src/nav.rs`; a feature adds a variant here
/// and a row there, and nothing else in the shell changes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Home,
    Code,
    // recipes — PR 3 replaces this line

    // skills — PR 4 replaces this line

    // scheduler — PR 5 replaces this line

    // extensions — PR 6 replaces this line

    // Session history (PR 7) gets no variant: it is the Chats list growing
    // kinds, rename and search, not a destination of its own.
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
    /// What the transcript knew about its attachments before a replay
    /// cleared it, waiting for the replay to bring them back so it can hand
    /// each one its name and its picture again. See
    /// `crate::attach::sent_attachments` for why goose needs this and the
    /// Code tab does not.
    pub attach_replay: Vec<crate::attach::Attachment>,
}

/// Context-window usage: (tokens used, context limit).
pub(crate) type Usage = (u64, u64);

/// A list the server owns, and everything a screen needs to say about it.
///
/// `unsupported` is not an error, and that is the whole reason it is a field
/// of its own. goose gates whole feature areas at startup — the scheduler
/// wants `--enable-scheduler`, older builds simply lack the newer namespaces
/// — so `-32601` here is a *fact about the server*, not a failure of this
/// request. It is a different sentence to the user ("Scheduler is not
/// available on this goose server", not "Couldn't load schedules") and, more
/// importantly, a different offer: there is no Retry, because retrying is not
/// a thing that could work.
///
/// `sticky` is the other half of the same idea. A toast is right for a
/// failure you can shrug at and wrong for one that leaves the screen empty:
/// it fades, and what is left says nothing at all. So a failure with nothing
/// on screen behind it stays on screen, and a failure over a list you can
/// still read is a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
// `cfg_attr(not(test))` because the tests below do use it: an expectation
// that holds in one cfg and not the other is an error in whichever cfg it
// does not hold in.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the shell has no list of its own; the first screen to hold \
                  one arrives in PR 3, and this expectation fails then"
    )
)]
pub(crate) struct Remote<T> {
    pub items: Vec<T>,
    pub loading: bool,
    /// The server does not offer this feature at all. No Retry.
    pub unsupported: bool,
    /// A failure to keep on screen rather than toast away.
    pub sticky: Option<String>,
}

// Derived `Default` would demand `T: Default`, which no list element owes
// anyone: an empty Vec is an empty Vec whatever is not in it.
impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(not(test), expect(dead_code, reason = "as `Remote` above"))]
impl<T> Remote<T> {
    pub(crate) const fn new() -> Self {
        Self {
            items: Vec::new(),
            loading: false,
            unsupported: false,
            sticky: None,
        }
    }

    /// A fetch has started. The previous failure goes now rather than when
    /// the new one lands, so a retry does not read as still-broken while it
    /// is in flight.
    pub(crate) fn begin(&mut self) {
        self.loading = true;
        self.sticky = None;
    }

    /// The fetch came back. Everything the last attempt concluded is dropped,
    /// including `unsupported`: a server that grew the feature (or a phone
    /// that reconnected to a different one) must be able to say so.
    pub(crate) fn settle(&mut self, items: Vec<T>) {
        self.items = items;
        self.loading = false;
        self.unsupported = false;
        self.sticky = None;
    }

    /// The fetch failed. Returns the sentence to toast, or `None` when the
    /// failure has been kept on screen instead — either as `unsupported`, or
    /// as `sticky` because there was nothing else to look at.
    pub(crate) fn fail(&mut self, error: &AcpError) -> Option<String> {
        self.loading = false;
        if error.is_unsupported() {
            // Not a failure to report: the screen hides itself and says why.
            self.items.clear();
            self.sticky = None;
            self.unsupported = true;
            return None;
        }
        let message = error.to_string();
        if self.items.is_empty() {
            self.sticky = Some(message);
            return None;
        }
        Some(message)
    }
}

/// Fetch into a [`Remote`], keeping the loading flag, the unsupported flag
/// and the failure in step with each other.
///
/// Every one of the five features does this, and each of them getting it
/// slightly wrong is five screens that disagree about what a missing feature
/// looks like.
#[expect(dead_code, reason = "as `Remote` above")]
pub(crate) async fn load_remote<T: 'static>(
    ctx: &AppCtx,
    mut slot: Signal<Remote<T>>,
    fetch: impl std::future::Future<Output = Result<Vec<T>, AcpError>>,
) {
    slot.write().begin();
    match fetch.await {
        Ok(items) => slot.write().settle(items),
        Err(e) => {
            // The guard is dropped before the toast: `show_toast` reads the
            // context, and holding a write borrow across an unrelated signal
            // is how a re-entrant read turns into a panic.
            let toast = slot.write().fail(&e);
            if let Some(message) = toast {
                show_toast(ctx, message);
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AppCtx {
    pub screen: Signal<Screen>,
    pub settings: Signal<Settings>,
    pub conn: Signal<ConnState>,
    pub client: Signal<Option<AcpClient>>,
    pub want_connected: Signal<bool>,
    pub sessions: Signal<Vec<SessionInfo>>,
    /// The request that would fetch the *next* page of the chats list, or
    /// `None` when the list is complete. It carries the filters as well as
    /// the cursor because the server refuses a cursor that arrives beside
    /// different filters — see [`SessionQuery`].
    pub sessions_next: Signal<Option<SessionQuery>>,
    pub sessions_loading: Signal<bool>,
    /// What is typed in the chats search box. Not component-local: the field
    /// debounces into it, and the "Load more" button a screen away has to ask
    /// what is being searched before it asks for another page.
    pub sessions_query: Signal<String>,
    /// Which `session/list` fetch owns the list. Two can be in flight at once
    /// — tap "Load more", then type in the search box — and they answer in
    /// whatever order the server chooses; a response that is no longer the
    /// newest writes nothing at all. The Code tab's `code_epoch` below parks a
    /// stale SSE pump the same way, and this is deliberately the same shape.
    pub sessions_epoch: Signal<u64>,
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
    /// What is typed in the goose composer but not yet sent.
    ///
    /// Not component-local, for the same reason `code_draft` below is not:
    /// navigating away unmounts the view and a `use_signal` draft dies with
    /// the scope. Two of the screens arriving after this one fill it in from
    /// elsewhere — a recipe's prompt, a scheduled run's instructions — which
    /// only works if the draft outlives the screen that shows it.
    pub chat_draft: Signal<String>,
    pub toast: Signal<Option<String>>,
    /// Files picked in the goose composer and not yet sent.
    ///
    /// On the context rather than in the view because the picker is one
    /// document-level listener installed at the app root (`src/attach.rs`
    /// says why the gesture has to live there), and it has to be able to hand
    /// what it read to a composer it does not own.
    pub attachments: Signal<Vec<crate::attach::PendingAttachment>>,
    /// The picks the browser is still reading. Held so the tray can say so:
    /// resizing three photos takes seconds, and a composer that just sits
    /// there is indistinguishable from one that lost the pick.
    ///
    /// A list, and each entry naming its own pick and conversation, because
    /// two reads can overlap and either can outlive the chat it was started
    /// in — see `crate::attach::Pick`.
    pub attach_reading: Signal<Vec<crate::attach::Pick>>,

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
    /// thinking-effort tiers. Fetched once, on the first open of a picker that
    /// needs it, and kept for the app's whole run. The new-session screen has
    /// no chat of its own, so it borrows a running one to ask
    /// (`crate::code::catalogue_donor`).
    pub code_models: Signal<Vec<opencode_client::ModelInfo>>,
    pub code_models_loading: Signal<bool>,
    /// The open chat's agents — what the composer's mode chip picks between,
    /// and what its label is resolved out of.
    ///
    /// Fetched quietly as part of opening a chat, because a chip that names
    /// the mode has to be told what the modes are; the tap on the chip is a
    /// loud retry for the case where that failed. Dropped when another chat is
    /// opened, because a repository can define agents of its own.
    pub code_agents: Signal<Vec<opencode_client::Agent>>,
    /// The id of the chat whose container answered `code_agents`, or empty.
    ///
    /// The list is a *repository's* as much as a server's, so "some list is
    /// already here" is not the same question as "the right list is here". The
    /// new-session screen is where the difference shows: it borrows a
    /// container to ask, and without this it would keep showing the agents of
    /// whichever chat was last opened however many times its repo pill moved.
    pub code_agents_from: Signal<String>,
    pub code_agents_loading: Signal<bool>,
    /// The branches of the repo the new-session screen is pointed at.
    /// Answered by the manager from GitHub with its own credential, so it
    /// wakes no container — the same plane `code_pulls` is fetched on.
    pub code_branches: Signal<crate::code::BranchList>,
    pub code_chat: Signal<crate::code::CodeChatState>,
    /// Pending permission asks from code chats, tagged by chat id. A separate
    /// queue from `permission` by construction: goose and `OpenCode` ids can
    /// never collide or be cross-answered.
    ///
    /// The open chat's asks arrive on its event stream; every other chat's
    /// come from the manager's aggregate over running containers
    /// (`crate::code::refresh_code_permissions`).
    pub code_permissions: Signal<Vec<(String, opencode_client::CodePermission)>>,
    /// `(chat id, permission id)` answered on this device, kept until the
    /// server stops reporting them as pending.
    ///
    /// The aggregate is a snapshot: one taken a moment before a reply landed
    /// still lists the ask, and merging it would put the panel back under the
    /// thumb that just dismissed it. A tombstone that clears itself when the
    /// server agrees is the smallest thing that cannot get stuck — and a
    /// reply that *fails* removes its own, so an ask still blocking the agent
    /// comes back rather than being hidden by a lie.
    pub code_answered: Signal<HashSet<(String, String)>>,
    /// On-device transcript cache — instant open while a chat's container
    /// wakes, read-only offline. Server history stays authoritative.
    pub code_cache: Signal<crate::code::CodeCache>,
    /// Bumped whenever a different code chat is opened; stale SSE pumps
    /// observe the change and exit.
    ///
    /// Not the list poll's business, and it used to be: reading this on the
    /// ten-second tick meant the first tap on any row retired the loop that
    /// carries the pending-ask aggregate, and nothing ever started another.
    /// The poll has `code_poll` for that.
    pub code_epoch: Signal<u64>,
    /// Generation of the chat-list poll loop. A loop retires when a newer one
    /// takes its place, and for no other reason.
    pub code_poll: Signal<u64>,
    /// The chat whose SSE stream is pumping events into the app, if any.
    ///
    /// Which is not the same question as "which chat is open": `code_chat`
    /// keeps its chat id after you back out to the list, and a stream can die
    /// for good while it does. This says who is genuinely speaking for a
    /// chat, and so who the manager's aggregate must not overrule — and, the
    /// moment the stream ends, must take back over
    /// (`crate::code::merge_permission_report`).
    pub code_stream: Signal<Option<String>>,
    /// The review screen's state for the open chat. Deliberately not a field
    /// on `CodeChatState`: the chat screen clones its whole state on every
    /// keystroke, and parsed whole-file patches are the largest thing this
    /// tab holds.
    pub code_diff: Signal<crate::code::DiffState>,
    /// The pull-request screen's state for the open chat: what this chat's
    /// branch has open on GitHub. Fetched on chat open — the route is the
    /// manager's own GitHub call and never reaches the container, so unlike
    /// the diff it costs nothing and does not wait for a wake.
    pub code_pulls: Signal<crate::code::PullsState>,
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
    /// Files picked on the new-session composer and not yet sent.
    ///
    /// Its own tray for the same reason it has its own conversation key
    /// (`crate::code::NEW_CONVERSATION`): the session it belongs to does not
    /// exist yet, and `code_attachments` belongs to whichever chat was last
    /// opened. Sharing one Vec put a photo picked in that chat into the tray
    /// of a new session pointed at a different repo — and, sent, into its
    /// first prompt. `conversation_key` only ever gated the *arrival* of a
    /// pick; what a tray already holds needed the same line drawn through it.
    pub new_attachments: Signal<Vec<crate::attach::PendingAttachment>>,
    // ---- one field per feature, each a Copy struct from its own module ----
    //
    // A feature's state is a struct it defines and this holds, rather than a
    // handful of loose signals: five branches adding five fields to one
    // struct merge, five branches adding thirty do not.

    // recipes — PR 3 replaces this line

    // skills — PR 4 replaces this line

    // scheduler — PR 5 replaces this line

    // extensions — PR 6 replaces this line

    // Session history (PR 7) contributes no struct: its state is the chats
    // list, which was already here as `sessions*` above. A second home for
    // the same three signals would be the merge hazard this region exists to
    // avoid rather than an instance of the pattern.
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
        sessions_next: use_signal(|| None),
        sessions_loading: use_signal(|| false),
        sessions_query: use_signal(String::new),
        sessions_epoch: use_signal(|| 0),
        chat: use_signal(ChatState::default),
        running_sessions: use_signal(HashSet::new),
        permission: use_signal(Vec::new),
        usage: use_signal(|| None),
        config_options: use_signal(Vec::new),
        chat_draft: use_signal(String::new),
        toast: use_signal(|| None),
        attachments: use_signal(Vec::new),
        attach_reading: use_signal(Vec::new),
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
        code_agents: use_signal(Vec::new),
        code_agents_from: use_signal(String::new),
        code_agents_loading: use_signal(|| false),
        code_branches: use_signal(crate::code::BranchList::default),
        code_chat: use_signal(crate::code::CodeChatState::default),
        code_permissions: use_signal(Vec::new),
        code_answered: use_signal(HashSet::new),
        code_cache,
        code_epoch: use_signal(|| 0),
        code_poll: use_signal(|| 0),
        code_stream: use_signal(|| None),
        code_diff: use_signal(crate::code::DiffState::default),
        code_pulls: use_signal(crate::code::PullsState::default),
        code_diff_wrap: use_signal(|| true),
        code_draft: use_signal(String::new),
        code_attachments: use_signal(Vec::new),
        new_attachments: use_signal(Vec::new),
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
        let carry = crate::attach::sent_attachments(&c.items);
        c.attach_replay = carry;
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
    let mut record = crate::attach::from_content_block(block);
    let mut c = chat.write();
    // A replayed image is bytes and a mime type and nothing else, so this is
    // where a photo this phone sent gets its name and its thumbnail back.
    crate::attach::adopt_sent(&mut c.attach_replay, &mut record);
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

/// One `session/list` request, and what its response may still do when it
/// lands.
///
/// The generation is the reason this is a struct rather than three arguments.
/// Two fetches can be in flight at once — "Load more" is tapped, and 250 ms of
/// typing later a search starts — and they come back in whatever order the
/// server answers them. The older one's page belongs to filters nobody is
/// looking at any more, and worse, so does the cursor riding on it: writing
/// that cursor back re-arms "Load more" with a page the next tap would ask for
/// beside *today's* query, which is the `-32602` "session list cursor does not
/// match filters" that [`SessionQuery`] exists to make unreachable. So a fetch
/// that is no longer the newest writes nowhere.
struct Fetch {
    /// Which fetch this is. Compared against `AppCtx::sessions_epoch` when the
    /// response arrives; a mismatch means a later one has claimed the list.
    generation: u64,
    /// The next page of the list on screen, rather than a new list.
    more: bool,
    /// The filters that went out, kept because the cursor in the response is
    /// only meaningful beside them.
    query: SessionQuery,
}

impl Fetch {
    /// Claim the list for a new request: one generation past `epoch`, which
    /// the caller writes back before it awaits anything.
    const fn claim(epoch: u64, more: bool, query: SessionQuery) -> Self {
        Self {
            generation: epoch + 1,
            more,
            query,
        }
    }

    /// Fold `page` into the list, or discard it when `latest` says a newer
    /// fetch has started since. Returns whether anything was written.
    ///
    /// A discarded response must not clear the loading flag either, which is
    /// what the return value is for: the fetch that superseded this one is
    /// still running and owns it.
    fn land(
        &self,
        latest: u64,
        page: SessionListResponse,
        list: &mut Vec<SessionInfo>,
        next: &mut Option<SessionQuery>,
    ) -> bool {
        if latest != self.generation {
            return false;
        }
        *next = self.query.next_page(&page);
        if self.more {
            list.extend(page.sessions);
        } else {
            *list = page.sessions;
        }
        true
    }
}

/// Fetch the first page of sessions (or the next page when `more` is true).
///
/// Every kind, not just the ones a person started. The standup recipe ran at
/// nine and you asked something at ten: that is one timeline, and splitting it
/// into a second screen would make a filter out of a fact. A scheduled run
/// says what it is in its own row (`kind_label`) and is otherwise a chat like
/// any other — it has a transcript, and it can be opened and read.
pub(crate) async fn refresh_sessions(ctx: &AppCtx, more: bool) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let mut sessions = ctx.sessions;
    let mut next = ctx.sessions_next;
    let mut loading = ctx.sessions_loading;
    let mut epoch = ctx.sessions_epoch;

    // `more` with no next page is not a re-fetch of page one: the list is
    // already whole, so there is nothing to ask for.
    let query = if more {
        let Some(query) = next.peek().clone() else {
            return;
        };
        query
    } else {
        SessionQuery::new(&SessionKind::ALL, Some(&ctx.sessions_query.peek()))
    };

    // Claim the list. Whatever was already in flight is a previous fetch from
    // this moment on, and finds that out where it lands.
    let fetch = Fetch::claim(*epoch.peek(), more, query);
    epoch.set(fetch.generation);

    loading.set(true);
    let result = client.session_list(&fetch.query).await;
    let latest = *epoch.peek();
    match result {
        Ok(page) => {
            let mut list = sessions.write();
            let mut next_page = next.write();
            if !fetch.land(latest, page, &mut list, &mut next_page) {
                return;
            }
        }
        // Whatever went wrong, the list already on screen stays: offline with
        // yesterday's chats still readable beats an empty page.
        //
        // A `-32601` is in that "whatever" deliberately. `session/list` is
        // base ACP, not one of goose's `_goose/unstable/*` extensions, so a
        // server that does not answer it is a broken server rather than one
        // with a feature switched off — `Feature::of_method` says exactly that
        // by classifying every base method as `Other`, and only
        // `goose_request` ever mints an `AcpError::Unsupported`. A branch here
        // that told the reader "this goose server does not list sessions"
        // would be telling them the one thing that cannot have happened.
        Err(e) => {
            if latest != fetch.generation {
                return;
            }
            show_toast(ctx, format!("Failed to list sessions: {e}"));
        }
    }
    loading.set(false);
}

/// Run a new search over the chats list.
///
/// Note what goose actually searches: `message_keyword_clause` matches the
/// text of the messages, splitting the query on whitespace and OR-ing the
/// terms. It does not look at titles, so a chat named "Deploy" does not match
/// "deploy" unless somebody said the word inside it.
pub(crate) async fn search_sessions(ctx: &AppCtx, query: String) {
    let mut current = ctx.sessions_query;
    if !search_changed(&current.peek(), &query) {
        return;
    }
    current.set(query);

    // The stored next page belongs to the *previous* search, and it is not
    // made harmless by the fetch below replacing it a moment later: until that
    // response lands, "Load more" is still on screen and still armed, and the
    // page it would fetch is the next page of a search nobody is running any
    // more. goose ties a cursor to a hash of the filters it was minted under
    // and rejects a mismatched pair outright ("session list cursor does not
    // match filters"), which is why `SessionQuery` will not let the two be
    // separated — this is the same rule one level up. Dropping the page here
    // is what makes the button go away the instant the query changes.
    //
    // It closes the sequential case only. A "Load more" that is already in
    // flight would put its cursor back the moment it answers; that one is
    // [`Fetch`]'s generation, which the refresh below takes out.
    let mut next = ctx.sessions_next;
    next.set(None);

    refresh_sessions(ctx, false).await;
}

/// Whether a search box's new contents describe a different request.
///
/// Trimmed on both sides because `SessionQuery::new` trims too, and the server
/// before it: a half-typed word with a trailing space is the same search as
/// the word, and re-fetching for the space would throw away the list you are
/// reading and scroll it back to the top.
fn search_changed(current: &str, next: &str) -> bool {
    current.trim() != next.trim()
}

/// Give a chat the title goose's guess should have been.
///
/// goose names a session from its first message, which is a name chosen before
/// the conversation happened — the row that reads "quick question" is the one
/// that turned into an afternoon. The list is updated in place rather than
/// re-fetched: the server has already agreed to the new title, and a refetch
/// would take the reader's scroll position with it.
pub(crate) async fn rename_session(ctx: &AppCtx, session_id: &str, title: &str) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    if let Err(e) = client.session_rename(session_id, title).await {
        show_toast(ctx, format!("Rename failed: {e}"));
        return;
    }

    let mut sessions = ctx.sessions;
    for info in sessions
        .write()
        .iter_mut()
        .filter(|info| info.session_id == session_id)
    {
        info.title = Some(title.to_owned());
    }
    // The chat screen holds its own copy of the title for its bar, so a rename
    // from the list has to reach it as well — and one from the sheet inside it
    // has to reach the list.
    let mut chat = ctx.chat;
    if chat.peek().session_id.as_deref() == Some(session_id) {
        title.clone_into(&mut chat.write().title);
    }
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
    // Walking out of a chat and back into it replays it from scratch, and the
    // replay cannot say what a photo was called or what it looked like. Only
    // from the same session: two conversations' attachments have nothing to
    // say about each other.
    let carry = {
        let current = ctx.chat.peek();
        if current.session_id.as_deref() == Some(info.session_id.as_str()) {
            crate::attach::sent_attachments(&current.items)
        } else {
            Vec::new()
        }
    };
    chat.set(ChatState {
        marks: Vec::new(),
        last_at: 0,
        session_id: Some(info.session_id.clone()),
        cwd: cwd.clone(),
        title: info.display_title(),
        items: Vec::new(),
        running,
        loading: true,
        attach_replay: carry,
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
    new_session_with(ctx, Value::Null);
}

/// Create a session that exists for a reason, and say what the reason is.
///
/// `_meta` is how goose is told *why* a session was started — launching a
/// recipe means `_meta.recipeId` — and it is the server that acts on it, so
/// the app cannot fake it after the fact by prompting the session into
/// character. `Value::Null` means "no reason", which is [`new_session`].
pub(crate) fn new_session_with(ctx: &AppCtx, meta: Value) {
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
        let meta = (!meta.is_null()).then_some(&meta);
        match client.session_new_with(&working_dir, meta).await {
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
                    attach_replay: Vec::new(),
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
/// untouched) if the message could not be handed to the transport at all.
///
/// True does not mean delivered — the request is answered on a task of its
/// own — so a send that fails on the wire puts the files back in the tray
/// itself, which is the only place they can come back to.
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

    // Held for the length of the request, so a failure has something to give
    // back. It costs a second copy of the payload while the turn runs, which
    // is the price of not making a lost connection eat the photo.
    let carried = files.to_vec();
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
            Err(e) => {
                // The transport can also die mid-turn, in which case the
                // message did arrive and these chips reappear next to a
                // bubble that already shows them. That is a thing the reader
                // can see and undo; a photo that is simply gone is not.
                //
                // Named with the session it was sent in, for the same reason
                // `running` is guarded on it three lines up: this answers long
                // after the send, and the tray it empties into is the one on
                // screen now.
                let note = crate::attach::return_to_tray(
                    &ctx,
                    crate::attach::AttachTarget::Goose,
                    &session_id,
                    carried,
                );
                show_toast(&ctx, format!("Prompt failed: {e}{note}"));
            }
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

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::{search_changed, Fetch, Remote};
    use goose_acp_client::{
        AcpError, Feature, SessionInfo, SessionKind, SessionListResponse, SessionQuery,
    };
    use serde_json::{json, Value};

    fn missing() -> AcpError {
        AcpError::Unsupported {
            feature: Feature::Scheduler,
            method: "_goose/unstable/schedules/list".to_owned(),
            reason: None,
        }
    }

    /// The load that finds nothing there leaves a screen that says so, not
    /// one that says it failed — and offers no Retry, because there is
    /// nothing on the other end to retry against.
    #[test]
    fn an_unsupported_feature_is_stated_rather_than_reported() {
        let mut remote = Remote::<u8>::new();
        remote.begin();
        assert_eq!(remote.fail(&missing()), None, "nothing to toast");
        assert!(remote.unsupported);
        assert!(remote.sticky.is_none());
        assert!(!remote.loading);
    }

    /// A first load that fails leaves an empty screen behind it, so the
    /// failure has to stay on it: a toast fades and takes the only
    /// explanation with it.
    #[test]
    fn a_failure_with_an_empty_list_sticks() {
        let mut remote = Remote::<u8>::new();
        remote.begin();
        assert_eq!(remote.fail(&AcpError::Timeout), None, "kept on screen");
        assert_eq!(remote.sticky.as_deref(), Some("timed out"));
        assert!(!remote.unsupported);
    }

    /// A refresh that fails over a list you can still read is a toast: the
    /// list is the screen, and a banner would push it around to say something
    /// about a call that changed nothing.
    #[test]
    fn a_failure_over_a_loaded_list_is_a_toast() {
        let mut remote = Remote::new();
        remote.settle(vec![1, 2, 3]);
        remote.begin();
        assert_eq!(
            remote.fail(&AcpError::Timeout).as_deref(),
            Some("timed out")
        );
        assert!(remote.sticky.is_none());
        assert_eq!(remote.items, vec![1, 2, 3], "the list is still readable");
    }

    /// Reconnecting to a server that does have the feature has to be able to
    /// take the word back — otherwise the screen stays hidden until the app
    /// is restarted.
    #[test]
    fn a_later_success_clears_both_the_verdict_and_the_failure() {
        let mut remote = Remote::new();
        remote.fail(&missing());
        remote.begin();
        remote.settle(vec![7]);
        assert!(!remote.unsupported);
        assert!(remote.sticky.is_none());
        assert_eq!(remote.items, vec![7]);
    }

    /// The previous failure goes when the retry starts, not when it lands:
    /// a spinner under a stale error message reads as still-broken.
    #[test]
    fn starting_a_load_clears_the_last_failure() {
        let mut remote = Remote::<u8>::new();
        remote.fail(&AcpError::Timeout);
        remote.begin();
        assert!(remote.loading);
        assert!(remote.sticky.is_none());
    }

    /// The predicate that decides whether the stored next page is thrown
    /// away. Getting it wrong in one direction re-fetches on every space bar;
    /// in the other, it pages a search that is no longer on screen.
    #[test]
    fn only_a_different_search_discards_the_page_after_this_one() {
        assert!(search_changed("", "deploy"));
        assert!(search_changed("deploy", ""));
        assert!(search_changed("deploy", "deployment"));

        // What the server would do with these is identical, so the list must
        // not flicker between them: `SessionQuery::new` trims and drops a
        // blank, and this has to agree with it.
        assert!(!search_changed("deploy", "  deploy "));
        assert!(!search_changed("", "   "));
        assert!(!search_changed("deploy", "deploy"));
    }

    /// A `session/list` reply, parsed the way a real one is so these tests
    /// cannot invent a field name the server does not send.
    fn page(ids: &[&str], next_cursor: Option<&str>) -> SessionListResponse {
        let sessions: Vec<Value> = ids.iter().map(|id| json!({"sessionId": id})).collect();
        serde_json::from_value(json!({"sessions": sessions, "nextCursor": next_cursor})).unwrap()
    }

    fn ids(list: &[SessionInfo]) -> Vec<&str> {
        list.iter().map(|info| info.session_id.as_str()).collect()
    }

    /// The concurrent half of the trap `search_sessions` opens with by
    /// dropping the stored next page: "Load more" is tapped, the search box is
    /// typed into before that page answers, and the page arrives *after* the
    /// search has claimed the list.
    ///
    /// Allowed to land, it would file the previous search's rows under the new
    /// search's — and, worse, hand its cursor back to a re-enabled button. The
    /// next tap would then send that cursor beside today's filters, which is
    /// the "session list cursor does not match filters" `invalid_params` the
    /// whole [`SessionQuery`] design exists to make unreachable.
    #[test]
    fn a_page_from_a_superseded_fetch_is_written_nowhere() {
        // The list as it stands: page one of the unfiltered chats, with more
        // behind it — which is why there is a "Load more" to tap at all.
        let mut list = page(&["chat_1", "chat_2"], None).sessions;
        let mut next = None;
        let mut epoch = 0;

        // Tapped first.
        let more = Fetch::claim(epoch, true, SessionQuery::new(&SessionKind::ALL, None));
        epoch = more.generation;

        // A quarter of a second of typing later, the search claims the list.
        let search = Fetch::claim(
            epoch,
            false,
            SessionQuery::new(&SessionKind::ALL, Some("deploy")),
        );
        epoch = search.generation;

        // And the page from before the search is what answers first.
        let landed = more.land(
            epoch,
            page(&["chat_3"], Some("cursor-of-the-unfiltered-list")),
            &mut list,
            &mut next,
        );
        assert!(!landed, "a superseded fetch wrote to the list");
        assert_eq!(ids(&list), ["chat_1", "chat_2"], "rows of another search");
        assert_eq!(next, None, "the pre-search cursor is armed again");

        // The search's own page still lands, and it is the one that decides
        // whether there is a page after it.
        assert!(search.land(
            epoch,
            page(&["deploy_1"], Some("cursor-of-deploy")),
            &mut list,
            &mut next
        ));
        assert_eq!(ids(&list), ["deploy_1"]);
        assert_eq!(next.as_ref().unwrap().query(), Some("deploy"));
    }

    /// The other ordering of the same two fetches, because a token that
    /// discarded both would pass the test above and leave the screen blank:
    /// the search answers first, then the stale page.
    #[test]
    fn the_newest_fetch_still_lands_whichever_order_they_answer_in() {
        let mut list = page(&["chat_1"], None).sessions;
        let mut next = None;
        let mut epoch = 0;

        let more = Fetch::claim(epoch, true, SessionQuery::new(&SessionKind::ALL, None));
        epoch = more.generation;
        let search = Fetch::claim(
            epoch,
            false,
            SessionQuery::new(&SessionKind::ALL, Some("deploy")),
        );
        epoch = search.generation;

        assert!(search.land(epoch, page(&["deploy_1"], None), &mut list, &mut next));
        assert!(!more.land(
            epoch,
            page(&["chat_2"], Some("cursor-of-the-unfiltered-list")),
            &mut list,
            &mut next
        ));
        assert_eq!(ids(&list), ["deploy_1"], "the old page arrived late");
        assert_eq!(next, None, "and re-armed the button on its way past");
    }

    /// Nothing racing: "Load more" adds to the list it was tapped on rather
    /// than replacing it, and carries the chain forward.
    #[test]
    fn an_uncontested_load_more_appends() {
        let mut list = page(&["chat_1"], None).sessions;
        let mut next = None;
        let more = Fetch::claim(0, true, SessionQuery::new(&SessionKind::ALL, None));
        assert!(more.land(
            more.generation,
            page(&["chat_2"], Some("cursor-2")),
            &mut list,
            &mut next
        ));
        assert_eq!(ids(&list), ["chat_1", "chat_2"]);
        assert!(
            next.is_some(),
            "there is another page and the button says so"
        );
    }
}
