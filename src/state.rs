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
    AcpClient, AcpError, AcpEvent, ConfigOption, ConnectConfig, DisconnectCause, MessageChunk,
    PermissionRequest, SessionInfo, SessionKind, SessionListResponse, SessionQuery, SessionUpdate,
    ToolCallUpdate,
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
    Recipes,
    Skills,
    Scheduler,
    Extensions,
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
///   dx build --platform ios
/// ```
///
/// Against a real brain rather than the mocks, add `GOOSE_DEV_FINGERPRINT`:
/// a `goose serve` holding its own TLS is self-signed, and without the pin the
/// client falls back to chain verification and refuses the certificate.
///
/// `option_env!` is read by the COMPILER, not at run time. Cargo does track it
/// — rustc records env dependencies in its dep-info, so changing a seed does
/// rebuild this crate, and no `build.rs` is needed to make that happen
/// (measured, because the opposite is an easy thing to assume). What it cannot
/// do is reach a `dx serve` that was started without the variables set: the
/// value is baked from the environment of the process that BUILDS, so an
/// already-running server keeps handing out whatever it was launched with.
/// Every field arriving empty means the build ran without the seeds, not that
/// the app ignored them.
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
            // Seeded like the rest, and it has to be: a `goose serve` that
            // holds its own TLS presents a SELF-SIGNED certificate, so an empty
            // fingerprint is not "no pinning, carry on" — `parse_fingerprint`
            // returns `Ok(None)`, the client falls back to ordinary chain
            // verification, and the connection fails. Every other field could
            // be seeded and the run would still not reach a real brain.
            fingerprint: dev_seed!("GOOSE_DEV_FINGERPRINT"),
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
// The `expect(dead_code)` that stood here is gone rather than moved: the
// scaffolding wrote it saying "this expectation fails when the first screen
// to hold a list arrives". Every feature screen in this stack is that screen —
// Extensions reached it first, and Skills, Recipes and the scheduled runs on
// Chats all hold one too.
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
    /// Asks whose answer never reached goose, and what became of the round
    /// they belonged to.
    ///
    /// Written when an ask ARRIVES, not when it is lost. See
    /// [`crate::ask_journal`] for why that is the whole design, and
    /// `docs/permission-durability.md` section 0 for the measurement it is
    /// built against: the round is destroyed on the server, nothing here
    /// recovers it, and this exists so the loss is stated rather than silent.
    ///
    /// The one signal in this struct backed by a real file. Its own storage
    /// key, never merged into `settings`: the backing rewrites a key's whole
    /// file with no fsync and no atomic rename, so the blast radius of a torn
    /// write has to be the journal.
    pub lost_asks: Signal<Vec<crate::ask_journal::AskRecord>>,
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

    /// WHETHER THE WINDOW'S RIGHT-HAND INSPECTOR IS SHOWING.
    ///
    /// FS-BACKED, unlike `settings` and `code_cache`: `use_persistent` builds
    /// over `SessionStorage`, which on every non-wasm target this app builds
    /// for is an in-memory `HashMap` — the trap written up further down this
    /// file — so a flag stored through it is forgotten by the next launch and
    /// nothing says so. `crate::ask_journal::Backing` is `LocalStorage`, and
    /// `the_journals_storage_backing_really_reaches_the_disk` is the existing
    /// proof that it writes a file.
    ///
    /// ON `AppCtx` AND NOT LOCAL TO `AppShell`, which is where `nav_open` and
    /// `library_open` correctly live. Those two are deliberately not persisted
    /// and so owe no test; this one is, and `AppShell` cannot be mounted — it
    /// calls `dioxus::desktop::window()`. A persisted flag no test can reach
    /// is a flag that silently stops persisting.
    ///
    /// DEFAULT OPEN. The nav's opposite rule is not a precedent against it:
    /// that comment argues a remembered-shut NAV opens one day into a window
    /// with no navigation in it, and losing the inspector loses commentary,
    /// not the ability to get anywhere.
    #[cfg_attr(
        any(target_os = "ios", target_os = "android"),
        expect(
            dead_code,
            reason = "the inspector is the desktop shell's until the phone \
                      grows a third column, at which point this expectation \
                      fails and takes itself out"
        )
    )]
    pub inspector_open: Signal<bool>,
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
    /// The first message a session that does not exist yet is being started
    /// with, handed from one screen to the one that can act on it.
    ///
    /// ITS OWN SIGNAL, BESIDE `new_attachments`, AND NOT `code_draft`. The
    /// desktop's Code home screen has a composer, and pressing its arrow cannot
    /// create anything: a working tree needs a repo and a base branch first,
    /// which is what `views::code::CodeNewView` is for. So the sentence has to
    /// survive one screen change with no conversation to belong to — exactly
    /// the situation the tray above is in, and it is answered the same way.
    ///
    /// `code_draft` was the wrong carrier twice over. It is the code CHAT's
    /// composer, so `CodeNewView` never reads it (measured: `grep -n code_draft
    /// src/views/code.rs` returns one line, three inside `CodeChatView`) and
    /// the sentence was written to a signal with no reader and then blanked by
    /// `open_code_chat`. Made to work, it would have leaked the other way: a
    /// half-typed correction left in a chat would turn up in the field of a new
    /// session pointed at a different repo, which is the line `open_code_chat`
    /// and `new_attachments` both already draw.
    ///
    /// TAKEN, NOT READ. `CodeNewView` empties it as it seeds its field, so a
    /// second visit to that screen does not resurrect a sentence already sent.
    pub new_task: Signal<String>,
    // ---- one field per feature, each a Copy struct from its own module ----
    //
    // A feature's state is a struct it defines and this holds, rather than a
    // handful of loose signals: five branches adding five fields to one
    // struct merge, five branches adding thirty do not.
    pub recipes: crate::recipes::Ctx,
    pub skills: crate::skills::Ctx,
    pub scheduler: crate::scheduler::Ctx,
    pub extensions: crate::extensions::Ctx,
    // Session history (PR 7) contributes no struct: its state is the chats
    // list, which was already here as `sessions*` above. A second home for
    // the same three signals would be the merge hazard this region exists to
    // avoid rather than an instance of the pattern.
}

pub(crate) fn use_app_ctx_provider() -> AppCtx {
    let settings = dioxus_sdk_storage::use_persistent("settings", Settings::default);
    let code_cache =
        dioxus_sdk_storage::use_persistent("code_cache", crate::code::CodeCache::default);
    // `use_synced_storage::<LocalStorage, _>`, and NOT `use_persistent` like
    // the two above, because `use_persistent` does not persist on this app's
    // targets. It builds over `SessionStorage`, which on every non-wasm
    // target is an in-memory `HashMap` hung off the root context
    // (dioxus-sdk-storage `persistence.rs:34`, `client_storage/mod.rs:32-41`,
    // `client_storage/memory.rs:13-28`); `LocalStorage` is the fs-backed one.
    // Corroborated rather than merely read: `set_dir!()` in `main` points the
    // backing at `~/Library/Application Support/goose-mobile` and `fs::set`
    // does `create_dir_all` on its first write, and on a machine where this
    // app has run — `~/Library/WebKit/goose-mobile` and
    // `~/Library/Caches/goose-mobile` both exist — that directory does not.
    //
    // Which means `settings` and `code_cache` do not survive a restart
    // either. That is a separate bug with separate consequences (a saved
    // secret would start being written to disk), so it is deliberately NOT
    // fixed in this change.
    // The same fs-backed store the journal uses, and for the same reason —
    // see the field's own note.
    let inspector_open = dioxus_sdk_storage::use_synced_storage::<crate::ask_journal::Backing, bool>(
        "inspector_open".to_owned(),
        || true,
    );
    let lost_asks = dioxus_sdk_storage::use_synced_storage::<
        crate::ask_journal::Backing,
        Vec<crate::ask_journal::AskRecord>,
    >("lost_asks".to_owned(), Vec::new);
    // An entry still `Open` here was written by a process that never got to
    // say what happened to it — the app was killed, so the `Disconnected` arm
    // below never ran. That is the measured case, and this is the only thing
    // that catches it.
    use_hook(move || {
        let mut entries = lost_asks.peek().clone();
        if crate::ask_journal::reconcile_at_startup(&mut entries, now_secs()) {
            lost_asks.clone().set(entries);
        }
    });
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
        lost_asks,
        inspector_open,
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
        new_task: use_signal(String::new),

        extensions: crate::extensions::use_ctx(),
        // One line per feature, each calling its own module's hook. There was
        // no placeholder here to replace — the struct above has them and this
        // literal does not — so a sibling branch adding its own field lands
        // in this same hunk and resolves by keeping both lines.
        skills: crate::skills::use_ctx(),
        recipes: crate::recipes::use_recipes(),
        scheduler: crate::scheduler::use_ctx(),
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
                // Written down BEFORE it is queued, and that ordering is the
                // whole design: the case this journal exists for is the app
                // being killed, and nothing in this process runs at that
                // moment. See `crate::ask_journal`.
                let record = ask_record(ctx, &request);
                let mut journal = ctx.lost_asks;
                crate::ask_journal::note(&mut journal.write(), record, now_secs());
                // Queue, never replace: every request must be answered or the
                // agent's turn hangs.
                permission.write().push(request);
            }
            AcpEvent::RequestCancelled { request_id } => {
                // The agent took its own question back, so there is nothing
                // left to tell anyone about it. Read before the retain, so
                // the entry can be found by the id the journal is keyed on.
                let withdrawn: Vec<String> = permission
                    .peek()
                    .iter()
                    .filter(|p| p.request_id == request_id)
                    .map(|p| p.tool_call.tool_call_id.clone())
                    .collect();
                permission.write().retain(|p| p.request_id != request_id);
                let mut journal = ctx.lost_asks;
                let mut entries = journal.write();
                for id in &withdrawn {
                    crate::ask_journal::resolve(&mut entries, id);
                }
            }
            AcpEvent::Disconnected { reason, cause } => {
                client_slot.set(None);
                chat.write().running = false;
                running_sessions.write().clear();

                // MEASURED (docs/permission-durability.md section 0): the
                // round is DISCARDED on the server, within 75 seconds, even
                // when the socket was never closed. There is no declined
                // tool, no Failed status, no note in the transcript — the
                // user's prompt and the generated title survive and nothing
                // else does. The comment that used to stand here said the
                // server "resolves its own pending permission requests via
                // the transport-error path", which is the account that run
                // falsified.
                //
                // So clearing the queue is still right — those asks are
                // unanswerable and a modal over them would be a lie — but it
                // is local cleanup, not a mirror of anything the server does.
                // What the journal records, on the other hand, is the loss.
                //
                // The journal is marked rather than the queue read, because
                // by the time this runs the queue is usually already empty:
                // the transport drains its pending replies before it sends
                // this event, so `send_prompt`'s post-turn sweep is woken
                // first and clears the entries for the session it was
                // running. Reading the queue here would report nothing at all
                // in the single-session, turn-in-flight case, which is the
                // whole bug.
                permission.write().clear();
                {
                    let mut journal = ctx.lost_asks;
                    let mut entries = journal.write();
                    match cause {
                        // The user pressed Disconnect, or pressed Connect over
                        // a live connection. They chose it; nothing to narrate.
                        DisconnectCause::Local => {
                            crate::ask_journal::forget_open(&mut entries);
                        }
                        DisconnectCause::Transport => crate::ask_journal::lose_open(
                            &mut entries,
                            crate::ask_journal::LostCause::Connection,
                            now_secs(),
                        ),
                    }
                }
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

/// What the journal keeps about an ask, resolved at the moment it arrives.
///
/// Both strings are computed here rather than looked up later, because later
/// is after a reconnect that rebuilt the chat and re-fetched the list. The
/// title uses the same fallback chain as [`crate::views::chat::PermissionModal`]
/// so the card names the ask the way the modal named it, and the session name
/// is resolved the way the modal resolves a foreign session.
fn ask_record(ctx: &AppCtx, request: &PermissionRequest) -> crate::ask_journal::AskRecord {
    let title = request
        .tool_call
        .title
        .clone()
        .or_else(|| request.tool_call.tool_name().map(str::to_string))
        .unwrap_or_else(|| "Run a tool".to_string());
    let chat = ctx.chat.peek();
    let session_title = if chat.session_id.as_deref() == Some(request.session_id.as_str()) {
        chat.title.clone()
    } else {
        ctx.sessions
            .peek()
            .iter()
            .find(|s| s.session_id == request.session_id)
            .map_or_else(
                || request.session_id.clone(),
                goose_acp_client::SessionInfo::display_title,
            )
    };
    crate::ask_journal::AskRecord::open(
        request.session_id.clone(),
        session_title,
        request.tool_call.tool_call_id.clone(),
        title,
        now_secs(),
    )
}

/// Stop reporting a loss the reader has read.
pub(crate) fn dismiss_lost_ask(ctx: &AppCtx, tool_call_id: &str) {
    let mut journal = ctx.lost_asks;
    crate::ask_journal::acknowledge(&mut journal.write(), tool_call_id, now_secs());
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
            // Reloaded whatever screen is showing, and the guard that used to
            // be here — only when `Screen::Chat` — is gone deliberately. A
            // phone coming back is usually locked on the Chats list, and
            // `session/load` is the ONE channel through which goose could
            // ever re-raise an ask it still holds
            // (`resend_pending_tool_permissions`). Skipping it there closed
            // the only door that could ever let a parked round be recovered.
            // It buys nothing today, because today the round is already gone
            // (docs/permission-durability.md section 0), and it costs one
            // request; the day an upstream fix makes goose hold the ask, it
            // is the difference between recovering the turn and not.
            if let Some(session_id) = session_id {
                reload_chat(ctx, session_id, cwd).await;
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

    // An attachment belongs to the message it was picked for, and the tray
    // lives on the context (the picker has to be able to reach it from the app
    // root), so it has to be told.
    ctx.attachments.clone().set(Vec::new());

    // Walking out of a chat and back into it replays it from scratch, and the
    // replay cannot say what a photo was called or what it looked like. Only
    // from the same session: two conversations' attachments have nothing to
    // say about each other.
    let (same_session, carry) = {
        let current = ctx.chat.peek();
        if current.session_id.as_deref() == Some(info.session_id.as_str()) {
            (true, crate::attach::sent_attachments(&current.items))
        } else {
            (false, Vec::new())
        }
    };
    // The draft belongs to the conversation for the same reason. It used to be
    // a `use_signal` that died with the screen; hoisting it onto the context
    // (so a recipe can fill it in before its chat exists) means half-typed
    // text would otherwise follow you out of one conversation and into the
    // next with the send button lit. Cleared only when the conversation
    // actually changes, so leaving a chat and coming back keeps what you were
    // writing — exactly what `open_code_chat` already does for `code_draft`.
    if !same_session {
        ctx.chat_draft.clone().set(String::new());
    }
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
        // The turn is over; deal with any permission prompt it left behind.
        // Which of the two ways depends on whether there is still a socket:
        // `AcpError::Closed` is exactly what the transport's own drain sends
        // when it is shutting down, and answering into that is theatre.
        // Everything else — an RPC error, a timeout, an unsupported method —
        // leaves a live connection, and those keep today's behaviour.
        match &result {
            Err(AcpError::Closed) => abandon_pending_permissions(&ctx, &session_id),
            _ => answer_pending_permissions(&ctx, &client, &session_id),
        }

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
///
/// A deliberate answer, so the journal forgets these: the ask got a reply and
/// the round can finish. Only reached over a live socket — see
/// [`abandon_pending_permissions`] for the other case.
fn answer_pending_permissions(ctx: &AppCtx, client: &AcpClient, session_id: &str) {
    let mut permission = ctx.permission;
    let mut journal = ctx.lost_asks;
    permission.write().retain(|req| {
        if req.session_id == session_id {
            client.respond_permission(req.request_id.clone(), None);
            crate::ask_journal::resolve(&mut journal.write(), &req.tool_call.tool_call_id);
            false
        } else {
            true
        }
    });
}

/// Drop `session_id`'s queued asks without answering them, because there is
/// nothing to answer *into*.
///
/// The distinction from [`answer_pending_permissions`] is not cosmetic.
/// `respond_permission` on a dead transport pushes a command into a channel
/// whose receiver is gone; it looks like an answer and is not one. Worse, the
/// old code path cleared the queue on this route with no record of what had
/// been in it, which is the silence this branch exists to end.
///
/// The journal is deliberately NOT marked here. This runs on whichever of two
/// tasks the runtime wakes first and it cannot tell a dropped tailnet from a
/// user pressing Disconnect; the pump's `Disconnected` arm can, because the
/// transport tells it, so that arm is the single place the decision is made.
fn abandon_pending_permissions(ctx: &AppCtx, session_id: &str) {
    let mut permission = ctx.permission;
    permission
        .write()
        .retain(|req| req.session_id != session_id);
}

/// Answer the front-of-queue permission request and remove it.
pub(crate) fn answer_permission(ctx: &AppCtx, request_id: &Value, option_id: Option<String>) {
    if let Some(client) = ctx.client.peek().clone() {
        client.respond_permission(request_id.clone(), option_id);
    }
    // Read before the retain: the journal is keyed on the tool call, not on
    // the JSON-RPC id, because the id belongs to a socket that is gone by the
    // time any of this matters.
    let answered: Vec<String> = ctx
        .permission
        .peek()
        .iter()
        .filter(|req| req.request_id == *request_id)
        .map(|req| req.tool_call.tool_call_id.clone())
        .collect();
    let mut permission = ctx.permission;
    permission
        .write()
        .retain(|req| req.request_id != *request_id);
    let mut journal = ctx.lost_asks;
    let mut entries = journal.write();
    for id in &answered {
        crate::ask_journal::resolve(&mut entries, id);
    }
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
            // Walk back from the epoch day to a civil date for the label: the
            // exact inverse of `days_from_civil` above, Hinnant's
            // `civil_from_days`. The `yoe` line is the load-bearing one and it
            // has to be his: the obvious `days / 365` estimate overshoots by a
            // year on the last few days before a March, and this used to say
            // "Mar 0" for the 29th of February.
            let mut days = epoch.div_euclid(86_400) + 719_468;
            let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
            days -= era * 146_097;
            let yoe = (days - days / 1_460 + days / 36_524 - days / 146_096) / 365;
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
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions: a failing unwrap, expect or wrong-variant panic is the \
              failing check"
)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "a mounted `App` holds the lock that keeps two of them from sharing the ask \
              journal's process-global storage; dropping it early is exactly what must \
              not happen, and the test's last line is where its turn ends"
)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex, PoisonError};

    use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
    use goose_acp_client::Feature;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

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

    // ------------------------------------------------------------- harness
    //
    // `src/testkit.rs` mounts a VIEW and hands back markup, which is the right
    // shape for `src/views/` and the wrong one for this file: almost nothing
    // here draws anything. These are functions that move signals around, and
    // the thing worth asserting is the signal.
    //
    // So the same idea is taken one step further down. A component whose only
    // job is to run the real `use_app_ctx_provider` publishes the context it
    // built; the test then calls the code under test inside that dom's
    // runtime, exactly as the app does, and reads the signals back directly.

    thread_local! {
        /// Where `Probe` leaves the context it built, for `App::mount` to
        /// collect. A thread-local and not a static: `cargo test` runs these
        /// in parallel and two mounts on two threads must not see each other's.
        static MOUNTED: std::cell::Cell<Option<AppCtx>> = const { std::cell::Cell::new(None) };
    }

    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component, not like a fn"
    )]
    fn Probe() -> Element {
        let ctx = use_app_ctx_provider();
        MOUNTED.with(|slot| slot.set(Some(ctx)));
        rsx! { div {} }
    }

    /// A tokio runtime for the timers this module arms, and for the sockets
    /// the second half of this file talks over.
    ///
    /// `show_toast` spawns a task that sleeps for four seconds, and
    /// `tokio::time::sleep` panics on construction when no runtime is in
    /// scope. *Entering* one is what keeps polling a spawned task from
    /// exploding, which is the difference between testing the half of a
    /// function after its `spawn_forever` and not; *driving* one — with
    /// [`App::settle`] or [`App::drive`], both of which `block_on` — is what
    /// makes a timer fire and a WebSocket frame move.
    ///
    /// One runtime for the whole binary, and current-thread because
    /// `rt-multi-thread` is not among the dev-dependency's features. Only the
    /// test holding [`ONE_AT_A_TIME`] ever touches it, so nothing here calls
    /// `block_on` from two threads at once.
    fn tokio_rt() -> &'static tokio::runtime::Runtime {
        static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread tokio runtime for the toast timers and the sockets")
        })
    }

    /// Hold one receiver on the ask journal's storage channel open for as long
    /// as the test binary lives.
    ///
    /// `dioxus-sdk-storage` keeps ONE process-global sender per storage key
    /// (`client_storage/fs.rs`'s `SUBSCRIPTIONS`) and `.unwrap()`s the result
    /// of sending on it, while every *receiver* belongs to the `VirtualDom`
    /// that subscribed. So the moment the last mounted dom has been dropped
    /// the sender has no receivers left, and the next mount panics inside
    /// `use_app_ctx_provider` — before its own subscription exists to save it.
    ///
    /// Dioxus swallows a panic thrown during render, so the symptom is not a
    /// stack trace: it is a provider that publishes nothing, in whichever test
    /// happened to mount first after a gap. Measured before this existed: 4 of
    /// 25 full-suite runs failed, in six different tests, none of them twice.
    fn keep_the_journals_channel_open() {
        use dioxus_sdk_storage::{LocalStorage, StorageSubscriber as _};
        static ANCHOR: std::sync::OnceLock<
            tokio::sync::watch::Receiver<dioxus_sdk_storage::StorageChannelPayload>,
        > = std::sync::OnceLock::new();
        let _ = ANCHOR.get_or_init(|| {
            LocalStorage::subscribe::<Vec<crate::ask_journal::AskRecord>>(&"lost_asks".to_owned())
        });
    }

    /// One mounted app at a time.
    ///
    /// The ask journal's storage is process-global by construction — one file,
    /// one sender, one subscription map — so two mounts alive at once can
    /// broadcast into each other's signals. Serialising is what makes a
    /// failure here mean something about the code rather than about the order
    /// `cargo test` happened to schedule its threads in.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct App {
        /// Held for the test's whole life; see [`ONE_AT_A_TIME`]. A poisoned
        /// lock is taken anyway: one test panicking must not turn every later
        /// one into a second, unrelated failure.
        _turn: std::sync::MutexGuard<'static, ()>,
        /// Kept alive for the whole test: every signal in `ctx` is owned by a
        /// scope of this dom, and dropping it invalidates all of them.
        dom: VirtualDom,
        ctx: AppCtx,
    }

    impl App {
        fn mount() -> Self {
            Self::mount_over(&Vec::new())
        }

        /// Launch over a journal already on disk — which is the only way to
        /// reach the startup reconcile, and the only way to write a test about
        /// a process that is no longer running.
        fn mount_over(journal: &Vec<crate::ask_journal::AskRecord>) -> Self {
            use dioxus_sdk_storage::StorageBacking as _;

            let turn = ONE_AT_A_TIME
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The provider reaches filesystem-backed storage for the ask
            // journal and panics without a directory; `testkit` owns the one
            // the whole test binary uses.
            //
            // Laid down and then CHECKED, because the directory is not this
            // module's to keep: `ask_journal`'s own disk test deletes it when
            // it is done, and that test is not serialised against this lock.
            // It can land between the write and the provider reading it back,
            // and the symptom is a seeded journal that arrives empty.
            for _ in 0..20 {
                let _ = std::fs::create_dir_all(crate::testkit::storage_dir());
                keep_the_journals_channel_open();
                // The journal is the one signal backed by a real file, and the
                // file outlives the test that wrote it, so every test here says
                // what it is launching over rather than inheriting it.
                dioxus_sdk_storage::LocalStorage::set("lost_asks".to_owned(), journal);

                let mut dom = VirtualDom::new(Probe);
                dom.rebuild_in_place();
                let ctx = MOUNTED.with(std::cell::Cell::take).expect(
                    "the probe component never published a context — Dioxus swallows a \
                     panic thrown during render, so the provider itself failed",
                );
                if dom.in_runtime(|| ctx.lost_asks.peek().len()) == journal.len() {
                    return Self {
                        _turn: turn,
                        dom,
                        ctx,
                    };
                }
                drop(dom);
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("the seeded ask journal never survived long enough to be read back")
        }

        /// Call into this module the way the app calls it: inside the Dioxus
        /// runtime that owns the signals, and inside a tokio runtime so a
        /// timer can be armed.
        fn run<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
            let _tokio = tokio_rt().enter();
            self.dom.in_runtime(|| f(&self.ctx))
        }

        /// Poll whatever `spawn_forever` queued. The app's own event loop does
        /// this; a test that skips it never reaches the body of a task.
        fn drain(&mut self) {
            let _tokio = tokio_rt().enter();
            self.dom.process_events();
        }

        /// Run one of this module's async functions to completion, inside both
        /// runtimes: the Dioxus one owns every signal it touches, the tokio one
        /// carries the socket underneath it.
        fn drive<F: std::future::Future>(&self, f: impl FnOnce(AppCtx) -> F) -> F::Output {
            let ctx = self.ctx;
            self.dom.in_runtime(|| tokio_rt().block_on(f(ctx)))
        }

        /// Poll the dom — and the sockets under it — until `done`, or until the
        /// budget runs out.
        ///
        /// Dioxus polls a spawned task from its own executor, so nothing a tap
        /// started happens without this; the timeout is what lets the tokio
        /// runtime carrying the WebSocket actor make progress while the virtual
        /// DOM has nothing to do.
        fn settle_until(&mut self, budget: Duration, done: fn(&AppCtx) -> bool) {
            let ctx = self.ctx;
            let dom = &mut self.dom;
            tokio_rt().block_on(async {
                let deadline = tokio::time::Instant::now() + budget;
                loop {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(5), dom.wait_for_work()).await;
                    dom.render_immediate_to_vec();
                    if dom.in_runtime(|| done(&ctx)) || tokio::time::Instant::now() >= deadline {
                        return;
                    }
                }
            });
        }

        /// Let whatever is in flight finish, when there is no one flag that
        /// says it has. Two hundred milliseconds of *consecutive* idle — an
        /// iteration with work in it resets the count — against loopback round
        /// trips measured in microseconds.
        fn settle(&mut self) {
            let dom = &mut self.dom;
            tokio_rt().block_on(async {
                let mut idle = 0;
                while idle < 20 {
                    if tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work())
                        .await
                        .is_err()
                    {
                        idle += 1;
                    } else {
                        idle = 0;
                    }
                    dom.render_immediate_to_vec();
                }
            });
        }

        /// Point the app at `server` and press Connect, the way the Settings
        /// screen does.
        fn dial(&self, server: &Server) -> bool {
            let base = server.base_url.clone();
            self.run(|ctx| {
                ctx.settings.clone().set(Settings {
                    server_url: base,
                    ..Settings::default()
                });
            });
            self.drive(|ctx| async move { establish(&ctx).await })
        }

        /// A live client with no `establish` behind it, for the tests that are
        /// about what a request does rather than about how the connection was
        /// opened.
        ///
        /// The event stream comes back to the caller and has to be kept: the
        /// transport gives up the socket when the last receiver goes away, so a
        /// test that dropped it would have a connection that died between the
        /// handshake and the first request.
        fn attach(&self, server: &Server) -> mpsc::Receiver<AcpEvent> {
            let cfg = ConnectConfig {
                base_url: server.base_url.clone(),
                secret: String::new(),
                fingerprint: None,
            };
            let (client, events, _info) = tokio_rt()
                .block_on(AcpClient::connect(&cfg))
                .expect("the mock server accepted the handshake");
            self.run(|ctx| ctx.client.clone().set(Some(client)));
            events
        }

        /// Put a chat on screen, the way `open_session` would have.
        fn open_chat(&self, session_id: &str, title: &str) {
            self.run(|ctx| {
                ctx.chat.clone().set(ChatState {
                    session_id: Some(session_id.to_owned()),
                    title: title.to_owned(),
                    ..ChatState::default()
                });
            });
        }

        fn items(&self) -> Vec<String> {
            self.run(|ctx| transcript(&ctx.chat.peek().items))
        }

        fn toast(&self) -> Option<String> {
            self.run(|ctx| ctx.toast.peek().clone())
        }

        fn journal(&self) -> Vec<crate::ask_journal::AskRecord> {
            self.run(|ctx| ctx.lost_asks.peek().clone())
        }
    }

    /// A compact rendering of the transcript, so a failing assertion names
    /// what was actually folded rather than printing a length.
    fn transcript(items: &[ChatItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                ChatItem::User { text, attachments } => {
                    let names: Vec<&str> = attachments.iter().map(|a| a.name.as_str()).collect();
                    format!("user:{text}:[{}]", names.join(","))
                }
                ChatItem::Assistant { message_id, text } => {
                    format!("assistant:{}:{text}", message_id.as_deref().unwrap_or("-"))
                }
                ChatItem::Thought { message_id, text } => {
                    format!("thought:{}:{text}", message_id.as_deref().unwrap_or("-"))
                }
                ChatItem::Tool {
                    id,
                    title,
                    kind,
                    status,
                    output,
                } => format!("tool:{id}:{title}:{kind}:{status}:{output}"),
            })
            .collect()
    }

    /// Run the event pump over a fixed list of events and let it finish.
    ///
    /// The sender is dropped before the pump starts, so `recv` answers from
    /// the buffer and then reports the stream closed: the whole loop completes
    /// on the first poll, with no executor to drive it.
    fn pump_events(app: &App, events: Vec<AcpEvent>) {
        app.run(|ctx| {
            let (tx, rx) = mpsc::channel(64);
            for event in events {
                tx.try_send(event).expect("the test channel holds 64");
            }
            drop(tx);
            pump(ctx, rx)
                .now_or_never()
                .expect("the pump parked on an event it should have taken from the buffer");
        });
    }

    fn session(id: &str, title: Option<&str>) -> SessionInfo {
        serde_json::from_value(json!({"sessionId": id, "title": title})).unwrap()
    }

    /// A `session/update` notification, parsed the way a real one is so a test
    /// cannot invent a tag or a field name the server never sends.
    fn notify(session_id: &str, raw: Value) -> AcpEvent {
        AcpEvent::Update {
            session_id: session_id.to_owned(),
            update: SessionUpdate::from_value(raw),
        }
    }

    fn ask(request_id: i64, session_id: &str, tool_call: Value) -> AcpEvent {
        AcpEvent::Permission(PermissionRequest {
            request_id: json!(request_id),
            session_id: session_id.to_owned(),
            tool_call: serde_json::from_value(tool_call).unwrap(),
            options: Vec::new(),
        })
    }

    // -------------------------------------------------------- pure helpers

    /// The `serde(default)` on `Settings` is the difference between an upgrade
    /// that keeps your server and one that silently forgets it: the storage
    /// layer falls back to `Default` on a parse error, so a settings blob
    /// written before the Code tab existed would take the goose URL, the
    /// secret and the pin down with it.
    #[test]
    fn settings_saved_before_the_code_tab_existed_keep_their_server() {
        let old = json!({
            "server_url": "https://brain.tailnet.ts.net",
            "secret_key": "hunter2",
            "fingerprint": "AA:BB",
            "working_dir": "/srv/work"
        });
        let parsed: Settings = serde_json::from_value(old).expect(
            "settings written by a build without the code-agent fields no longer parse, so \
             every upgrading user is handed a blank Settings screen",
        );
        assert_eq!(parsed.server_url, "https://brain.tailnet.ts.net");
        assert_eq!(parsed.secret_key, "hunter2");
        assert_eq!(parsed.working_dir, "/srv/work");
        // The absent fields fall back to the same values a first launch has —
        // compared against `Default` rather than against "" because a debug
        // build may have been seeded with `GOOSE_DEV_*`.
        let fresh = Settings::default();
        assert_eq!(parsed.code_server_url, fresh.code_server_url);
        assert_eq!(parsed.code_password, fresh.code_password);

        // And the degenerate case the storage layer can hand back.
        assert!(serde_json::from_str::<Settings>("{}").unwrap() == fresh);
    }

    /// The same rule one level down, for the Code tab's on-device transcript
    /// cache. Without the default on `attachments`, one cached chat written by
    /// an older build fails to parse and the whole cache goes with it — which
    /// is the offline-readable transcript, gone.
    #[test]
    fn a_cached_transcript_from_before_attachments_still_parses() {
        let old = json!([
            {"User": {"text": "have a look at this"}},
            {"Assistant": {"message_id": "m1", "text": "looking"}}
        ]);
        let items: Vec<ChatItem> = serde_json::from_value(old).expect(
            "a transcript cached before messages carried attachments no longer parses, so the \
             whole on-device cache is discarded",
        );
        assert_eq!(
            transcript(&items),
            ["user:have a look at this:[]", "assistant:m1:looking"]
        );
    }

    /// The pin travels or the connection is refused; it never quietly becomes
    /// "no pin". A `?` dropped from the fingerprint line would connect to a
    /// server whose certificate nobody checked.
    #[test]
    fn a_malformed_fingerprint_refuses_the_connection_rather_than_dropping_the_pin() {
        let pinned = Settings {
            server_url: "https://brain.tailnet.ts.net".to_owned(),
            secret_key: "hunter2".to_owned(),
            fingerprint: "AA:".to_owned() + &"BB:".repeat(30) + "CC",
            ..Settings::default()
        };
        let cfg = connect_config(&pinned).expect("a 32-byte colon-separated fingerprint parses");
        assert_eq!(cfg.base_url, "https://brain.tailnet.ts.net");
        assert_eq!(cfg.secret, "hunter2");
        assert_eq!(
            cfg.fingerprint.map(|fp| fp[0]),
            Some(0xAA),
            "the pin reached the connect config"
        );

        let unpinned = Settings {
            fingerprint: "   ".to_owned(),
            ..Settings::default()
        };
        assert_eq!(
            connect_config(&unpinned).unwrap().fingerprint,
            None,
            "a blank fingerprint box means no pin, not a broken one"
        );

        let broken = Settings {
            fingerprint: "not-a-fingerprint".to_owned(),
            ..Settings::default()
        };
        let err = connect_config(&broken)
            .expect_err("a fingerprint that is not 32 hex bytes was accepted and thrown away");
        assert!(
            err.contains("32 hex bytes"),
            "the error has to name the format the box wants: {err}"
        );
    }

    /// A list nobody has fetched yet is not a list that is loading. Derived
    /// `Default` cannot be used here (it would demand `T: Default`), so this
    /// is hand-written and could disagree with `new` — a `loading: true` would
    /// put a spinner on every feature screen before its first request.
    #[test]
    fn an_unfetched_list_shows_neither_a_spinner_nor_a_failure() {
        let fresh = Remote::<u8>::default();
        assert!(fresh.items.is_empty());
        assert!(
            !fresh.loading,
            "a screen that has not asked yet is not busy"
        );
        assert!(!fresh.unsupported);
        assert!(fresh.sticky.is_none());
    }

    /// The rule that draws a time separator into the transcript. Too eager and
    /// every pause for thought becomes a rule across the chat; too slow and
    /// yesterday's conversation runs into today's with nothing between them.
    #[test]
    fn only_a_pause_of_minutes_puts_a_rule_in_the_transcript() {
        let now = now_secs();

        // The very first append has nothing to measure against.
        let (mut marks, mut last_at) = (Vec::new(), 0);
        mark_gap(0, &mut marks, &mut last_at);
        assert!(marks.is_empty(), "a first message is not a resumption");
        assert!(last_at >= now, "the append was stamped");

        // A reply moments later belongs to the same conversation.
        last_at = now - 30;
        mark_gap(1, &mut marks, &mut last_at);
        assert!(marks.is_empty(), "a 30-second pause is thinking, not a gap");

        // Under the ten-minute threshold, still nothing.
        last_at = now - 570;
        mark_gap(1, &mut marks, &mut last_at);
        assert!(marks.is_empty(), "nine and a half minutes is not a gap");

        // Over it, and the mark points at the item about to be pushed.
        last_at = now - 630;
        mark_gap(7, &mut marks, &mut last_at);
        assert_eq!(marks.len(), 1, "a ten-minute pause went unmarked");
        assert_eq!(marks[0].0, 7, "the rule sits before the resuming item");
        assert!(
            marks[0].1 >= now,
            "the rule is stamped with when things resumed, not when they stopped"
        );
    }

    /// The badge on a list row. Each unit is what the reader is actually
    /// tracking, and an off-by-one in a threshold reads as a chat that
    /// happened "60m" ago rather than "1h".
    #[test]
    fn a_recent_row_is_counted_in_the_unit_the_reader_thinks_in() {
        let now = now_secs();
        assert_eq!(relative_time(now - 5), "now");
        assert_eq!(relative_time(now - 30), "now");
        assert_eq!(relative_time(now - 130), "2m");
        assert_eq!(relative_time(now - 3_500), "58m");
        assert_eq!(relative_time(now - 3_700), "1h");
        assert_eq!(relative_time(now - 40_000), "11h");
        assert_eq!(relative_time(now - 90_000), "1d");
        assert_eq!(relative_time(now - 500_000), "5d");
        // A clock that has gone backwards (or a server stamp from the near
        // future) must still produce a badge rather than a negative one.
        assert_eq!(relative_time(now + 3_600), "now");
    }

    /// Past a week the badge becomes a date, and the civil-date arithmetic
    /// behind it is written out by hand here rather than pulled from a crate.
    /// A month index off by one puts every row in the wrong month.
    #[test]
    fn an_old_row_is_dated_rather_than_counted() {
        let on = |ts: &str| relative_time(rfc3339_to_epoch(ts).expect("a valid RFC3339 stamp"));
        assert_eq!(on("2021-01-01T00:00:00Z"), "Jan 1");
        assert_eq!(on("2020-02-29T12:00:00Z"), "Feb 29", "a leap day");
        assert_eq!(on("2020-03-01T12:00:00Z"), "Mar 1", "the day after one");
        assert_eq!(on("2021-03-01T12:00:00Z"), "Mar 1", "and a non-leap year");
        assert_eq!(on("2019-07-04T23:59:59Z"), "Jul 4");
        assert_eq!(on("2018-12-31T00:00:00Z"), "Dec 31");
        assert_eq!(on("1999-11-15T06:30:00Z"), "Nov 15", "before the epoch era");
        assert_eq!(on("1968-05-20T00:00:00Z"), "May 20", "before the epoch");
    }

    /// The `OpenCode` API reports timestamps as floats. The truncating cast is
    /// what lets one badge function serve both backends; if it rounded the
    /// wrong way the two tabs would disagree about the same instant.
    #[test]
    fn a_fractional_timestamp_reads_the_same_as_a_whole_one() {
        let epoch = rfc3339_to_epoch("2019-07-04T23:59:59Z").unwrap();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a 2019 epoch second is far inside f64's exact-integer range"
        )]
        let as_float = epoch as f64;
        assert_eq!(relative_time_secs(as_float + 0.75), "Jul 4");
        assert_eq!(relative_time_secs(as_float), relative_time(epoch));
    }

    // ------------------------------------------- lifecycle, toasts, loading

    /// The state the window opens in. "Connecting is an explicit user action"
    /// is a promise about a phone on somebody else's network: a launch that
    /// landed on Chats, or one that dialled out on its own, would reach for
    /// the tailnet before anyone asked it to.
    #[test]
    fn the_app_opens_disconnected_on_settings() {
        let app = App::mount();
        app.run(|ctx| {
            assert!(matches!(*ctx.screen.peek(), Screen::Settings));
            assert!(matches!(*ctx.tab.peek(), Tab::Home));
            assert!(matches!(*ctx.conn.peek(), ConnState::Disconnected));
            assert!(!ctx.conn.peek().is_connected());
            assert!(!*ctx.want_connected.peek(), "nothing is dialling out yet");
            assert!(ctx.client.peek().is_none());
            assert!(!*ctx.drawer_open.peek());
            assert!(
                *ctx.code_diff_wrap.peek(),
                "the review screen soft-wraps until somebody turns it off"
            );
        });
    }

    /// A toast says the newest thing, not the first thing. Two failures in
    /// quick succession that left the older sentence up would report a problem
    /// the reader has already moved past.
    #[test]
    fn a_toast_says_the_most_recent_thing() {
        let app = App::mount();
        assert_eq!(app.toast(), None, "nothing has gone wrong yet");
        app.run(|ctx| show_toast(ctx, "Failed to list sessions"));
        assert_eq!(app.toast().as_deref(), Some("Failed to list sessions"));
        app.run(|ctx| show_toast(ctx, format!("Rename failed: {}", "timed out")));
        assert_eq!(app.toast().as_deref(), Some("Rename failed: timed out"));
    }

    /// Disconnect has to take the *intent* down with the socket. The reconnect
    /// loop and the pump's `Disconnected` arm both read `want_connected`, so a
    /// disconnect that left it set would have the phone dialling back out
    /// every thirty seconds after the user asked it to stop.
    #[test]
    fn disconnecting_stops_the_app_wanting_to_be_connected() {
        let app = App::mount();
        app.run(|ctx| {
            ctx.want_connected.clone().set(true);
            disconnect(ctx);
            assert!(
                !*ctx.want_connected.peek(),
                "Disconnect left the app still trying to reconnect"
            );
        });
    }

    /// A server address that cannot be dialled is reported on the Settings
    /// screen rather than swallowed, and it must not leave the app believing
    /// it is connected — `want_connected` is what arms the reconnect loop, and
    /// arming it here would retry a URL that can never work.
    #[test]
    fn an_unusable_server_address_fails_the_connection_rather_than_hanging() {
        let app = App::mount();
        app.run(|ctx| {
            ctx.settings.clone().set(Settings {
                server_url: String::new(),
                ..Settings::default()
            });
            let connected = establish(ctx)
                .now_or_never()
                .expect("a blank address is refused before any socket is opened");
            assert!(!connected);
            match &*ctx.conn.peek() {
                ConnState::Failed(message) => assert!(
                    message.contains("empty"),
                    "the Settings screen has to say what is wrong with the address: {message}"
                ),
                _ => panic!("a blank server address did not fail the connection"),
            }
            assert!(!*ctx.want_connected.peek(), "nothing to reconnect to");

            // And the other refusal, one step earlier: the pin is unreadable,
            // so the connection is not attempted at all.
            ctx.settings.clone().set(Settings {
                server_url: "https://brain.tailnet.ts.net".to_owned(),
                fingerprint: "nonsense".to_owned(),
                ..Settings::default()
            });
            assert!(!establish(ctx)
                .now_or_never()
                .expect("a bad pin is refused without opening a socket"));
            match &*ctx.conn.peek() {
                ConnState::Failed(message) => assert!(message.contains("32 hex bytes")),
                _ => panic!("a malformed pin did not fail the connection"),
            }
        });
    }

    /// The shared fetch-into-a-list helper, driven through all three of its
    /// endings. Every feature screen goes through it, so a mistake here is the
    /// same mistake on five screens at once.
    #[test]
    fn a_feature_load_keeps_its_spinner_and_its_failure_in_step() {
        let app = App::mount();
        app.run(|ctx| {
            // A failure over nothing stays on screen: a toast would fade and
            // leave a blank page with no explanation on it. So it must not
            // toast — checked while nothing has toasted yet, which is the only
            // moment the absence can be told apart from a stale sentence.
            let empty: Signal<Remote<u8>> = Signal::new_in_scope(Remote::new(), ScopeId::ROOT);
            load_remote(ctx, empty, std::future::ready(Err(AcpError::Timeout)))
                .now_or_never()
                .expect("a ready fetch settles without an executor");
            assert_eq!(empty.peek().sticky.as_deref(), Some("timed out"));
            assert!(!empty.peek().loading, "the spinner outlived the failure");
            assert_eq!(
                *ctx.toast.peek(),
                None,
                "a failure kept on screen was ALSO toasted, so the reader is told twice \
                 and one of the two disappears"
            );

            let slot: Signal<Remote<u8>> = Signal::new_in_scope(Remote::new(), ScopeId::ROOT);
            load_remote(ctx, slot, std::future::ready(Ok(vec![1, 2, 3])))
                .now_or_never()
                .unwrap();
            assert_eq!(slot.peek().items, vec![1, 2, 3]);
            assert!(!slot.peek().loading, "the spinner outlived the response");
            assert_eq!(*ctx.toast.peek(), None, "a success is not worth a sentence");

            // A failure over a list you can still read is a toast, and the
            // rows stay put underneath it.
            load_remote(ctx, slot, std::future::ready(Err(AcpError::Timeout)))
                .now_or_never()
                .unwrap();
            assert_eq!(slot.peek().items, vec![1, 2, 3], "the list is still there");
            assert!(slot.peek().sticky.is_none());
            assert_eq!(ctx.toast.peek().as_deref(), Some("timed out"));
        });
    }

    // ------------------------------------------- the pump: folding a stream

    /// The stream carries every session the agent is running, not just the one
    /// on screen. Folding another chat's words into this transcript would put
    /// a stranger's conversation in front of the reader.
    #[test]
    fn a_chunk_for_another_session_never_reaches_the_open_chat() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                notify(
                    "s2",
                    json!({"sessionUpdate": "agent_message_chunk",
                           "content": {"type": "text", "text": "not yours"}}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "agent_message_chunk",
                           "content": {"type": "text", "text": "yours"}}),
                ),
            ],
        );
        assert_eq!(app.items(), ["assistant:-:yours"]);
    }

    /// goose streams a reply a token at a time. Each chunk becoming a bubble
    /// of its own is the difference between a paragraph and forty stacked
    /// cards; a new `messageId` starting a bubble is what keeps two answers
    /// apart.
    #[test]
    fn streamed_chunks_of_one_message_are_one_bubble() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        let chunk = |id: Option<&str>, text: &str| {
            notify(
                "s1",
                json!({"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "text", "text": text},
                       "messageId": id}),
            )
        };
        pump_events(
            &app,
            vec![
                chunk(Some("m1"), "The deploy "),
                chunk(Some("m1"), "is green."),
                // An empty chunk is not a bubble and not a break in one.
                chunk(Some("m1"), ""),
                chunk(Some("m2"), "Anything else?"),
                // No id at all: goose's older shape, which appends to whatever
                // is being written rather than starting a new bubble.
                chunk(None, " (I'll wait.)"),
            ],
        );
        assert_eq!(
            app.items(),
            [
                "assistant:m1:The deploy is green.",
                "assistant:m2:Anything else? (I'll wait.)"
            ]
        );
    }

    /// Reasoning and reply are two different things on screen — one is folded
    /// away, the other is the answer — so a thought must never land in the
    /// bubble beside it even when the two share a message id.
    #[test]
    fn a_thought_and_a_reply_do_not_share_a_bubble() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        let of = |tag: &str, text: &str| {
            notify(
                "s1",
                json!({"sessionUpdate": tag,
                       "content": {"type": "text", "text": text},
                       "messageId": "m1"}),
            )
        };
        pump_events(
            &app,
            vec![
                of("agent_thought_chunk", "check the logs "),
                of("agent_thought_chunk", "first"),
                of("agent_message_chunk", "Logs look fine."),
                of("agent_thought_chunk", "done"),
            ],
        );
        assert_eq!(
            app.items(),
            [
                "thought:m1:check the logs first",
                "assistant:m1:Logs look fine.",
                "thought:m1:done"
            ]
        );
    }

    /// Replaying a session hands back the reader's own turns. A photo comes
    /// back as bytes and a mime type, and hanging it off the message is what
    /// keeps the transcript from reading "[image: image/jpeg]" where a picture
    /// used to be — with the name and the thumbnail this device still holds.
    #[test]
    fn a_replayed_photo_hangs_off_the_message_rather_than_becoming_its_text() {
        let app = App::mount();
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState {
                session_id: Some("s1".to_owned()),
                attach_replay: vec![crate::attach::Attachment {
                    name: "roof.jpg".to_owned(),
                    mime: "image/jpeg".to_owned(),
                    size: 3,
                    thumb: "THUMB".to_owned(),
                }],
                ..ChatState::default()
            });
        });
        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "user_message_chunk",
                           "content": {"type": "text", "text": "what is "}}),
                ),
                // goose splits a replayed message into blocks, and an
                // untagged second one belongs to the turn already open — a
                // bubble per block would break one sentence into two.
                notify(
                    "s1",
                    json!({"sessionUpdate": "user_message_chunk",
                           "content": {"type": "text", "text": "this?"}}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "user_message_chunk",
                           "content": {"type": "image", "data": "QUJD", "mimeType": "image/jpeg"}}),
                ),
            ],
        );
        assert_eq!(
            app.items(),
            ["user:what is this?:[roof.jpg]"],
            "the photo either became text of its own or opened a second turn"
        );
        assert_eq!(
            app.run(|ctx| ctx.chat.peek().items.len()),
            1,
            "one message with a photo is one turn, not two"
        );

        // A message that is nothing but a photo has no bubble to hang off, so
        // it opens one — with an empty text, not a placeholder.
        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "agent_message_chunk",
                           "content": {"type": "text", "text": "a roof"}}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "user_message_chunk",
                           "content": {"type": "image", "data": "REVG", "mimeType": "image/png"}}),
                ),
            ],
        );
        assert_eq!(
            app.items(),
            [
                "user:what is this?:[roof.jpg]",
                "assistant:-:a roof",
                "user::[Image]"
            ]
        );
    }

    /// A tool call and everything that happens to it are one row that changes,
    /// not a row per notification. goose sends the title, the kind, the status
    /// and the output in separate updates, and a row that failed to find its
    /// call would leave the transcript showing "pending" forever.
    #[test]
    fn a_tool_call_and_its_updates_are_one_row() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                // Only the id: everything else falls back.
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "call_1"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "call_2",
                           "title": "shell: uname -a", "kind": "execute", "status": "pending"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "call_2",
                           "status": "completed",
                           "content": [{"type": "content",
                                        "content": {"type": "text", "text": "Linux brain"}}]}),
                ),
                // A second batch of output is appended, on its own line.
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "call_2",
                           "title": "shell", "kind": "other",
                           "content": [{"type": "content",
                                        "content": {"type": "text", "text": "6.1.0"}}]}),
                ),
                // And an update for a call that never arrived changes nothing.
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "ghost",
                           "status": "failed"}),
                ),
            ],
        );
        assert_eq!(
            app.items(),
            [
                "tool:call_1:tool:other:pending:",
                "tool:call_2:shell:other:completed:Linux brain\n6.1.0"
            ]
        );
    }

    /// When a tool reports no readable content, the raw result is better than
    /// a blank row — it is the only thing on screen that says what happened.
    /// And a later real output must not be replaced by it.
    #[test]
    fn a_tool_with_no_readable_content_falls_back_to_its_raw_result() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "a", "title": "read"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
                           "rawOutput": "just a string"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "b", "title": "list"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "b",
                           "rawOutput": {"count": 2}}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "c", "title": "grep"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "c",
                           "rawOutput": true}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "d", "title": "find"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "d",
                           "rawOutput": ["a", "b"]}),
                ),
            ],
        );
        assert_eq!(
            app.items(),
            [
                "tool:a:read:other:pending:just a string",
                "tool:b:list:other:pending:{\n  \"count\": 2\n}",
                // Neither a string nor a container: there is nothing worth
                // putting in a transcript, so the row stays empty.
                "tool:c:grep:other:pending:",
                "tool:d:find:other:pending:[\n  \"a\",\n  \"b\"\n]"
            ]
        );
    }

    /// goose names a session from its first message and then changes its mind.
    /// The name has to reach both places it is shown at once — the bar over the
    /// open chat and the row in the list — because a refetch of the list would
    /// take the reader's scroll position with it.
    #[test]
    fn a_generated_title_reaches_both_the_chat_and_its_row() {
        let app = App::mount();
        app.open_chat("s1", "quick question");
        app.run(|ctx| {
            ctx.sessions.clone().set(vec![
                session("s1", Some("quick question")),
                session("s2", None),
            ]);
        });
        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "session_info_update", "title": "Tailscale cert rotation"}),
                ),
                // Another session's rename reaches its row and stops there.
                notify(
                    "s2",
                    json!({"sessionUpdate": "session_info_update", "title": "Nightly standup"}),
                ),
                // An update with no title at all leaves both alone.
                notify(
                    "s1",
                    json!({"sessionUpdate": "session_info_update", "updatedAt": "2026-08-29T09:00:00Z"}),
                ),
            ],
        );
        app.run(|ctx| {
            assert_eq!(ctx.chat.peek().title, "Tailscale cert rotation");
            let titles: Vec<Option<String>> = ctx
                .sessions
                .peek()
                .iter()
                .map(|s| s.title.clone())
                .collect();
            assert_eq!(
                titles,
                vec![
                    Some("Tailscale cert rotation".to_owned()),
                    Some("Nightly standup".to_owned())
                ],
                "a rename reached the bar but not the list, or the wrong row"
            );
        });
    }

    /// The context meter. It is the only warning a reader gets before a long
    /// chat starts dropping its own history, so a usage update from a chat
    /// they are not looking at must not move it.
    #[test]
    fn the_context_meter_follows_the_open_chat_only() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        let goose = |session_id: &str, update: Value| AcpEvent::GooseUpdate {
            session_id: session_id.to_owned(),
            update,
        };
        pump_events(
            &app,
            vec![goose(
                "s1",
                json!({"sessionUpdate": "usage_update", "used": 12_000, "contextLimit": 200_000}),
            )],
        );
        assert_eq!(app.run(|ctx| *ctx.usage.peek()), Some((12_000, 200_000)));

        pump_events(
            &app,
            vec![
                goose(
                    "s2",
                    json!({"sessionUpdate": "usage_update", "used": 1, "contextLimit": 2}),
                ),
                // The right session, but a different kind of goose update.
                goose("s1", json!({"sessionUpdate": "status", "text": "thinking"})),
                // The right kind, but only half the numbers: a meter with no
                // limit has nothing to draw.
                goose("s1", json!({"sessionUpdate": "usage_update", "used": 99})),
            ],
        );
        assert_eq!(
            app.run(|ctx| *ctx.usage.peek()),
            Some((12_000, 200_000)),
            "the meter moved for something that was not this chat's usage"
        );
    }

    /// The agent pushes the option set after every change, including changes
    /// made from another client — but an empty push is goose saying nothing,
    /// not goose saying "no options", and taking it literally would empty the
    /// model picker mid-conversation.
    #[test]
    fn a_pushed_option_set_replaces_the_picker_and_an_empty_one_does_not() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![notify(
                "s1",
                json!({"sessionUpdate": "config_option_update", "configOptions": [
                    {"id": "model", "name": "Model", "type": "select", "currentValue": "sonnet",
                     "options": [{"value": "sonnet", "name": "Sonnet"},
                                 {"value": "opus", "name": "Opus"}]}
                ]}),
            )],
        );
        app.run(|ctx| {
            let opts = ctx.config_options.peek();
            assert_eq!(opts.len(), 1);
            assert_eq!(opts[0].config_id, "model");
            assert_eq!(opts[0].current_label(), Some("Sonnet"));
        });

        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "config_option_update", "configOptions": []}),
                ),
                // And one for a session that is not open, which the picker
                // still takes: the options belong to the agent, not the screen.
                notify(
                    "s2",
                    json!({"sessionUpdate": "config_option_update", "configOptions": [
                        {"id": "mode", "name": "Mode", "type": "select"}]}),
                ),
            ],
        );
        app.run(|ctx| {
            let opts = ctx.config_options.peek();
            assert_eq!(opts.len(), 1);
            assert_eq!(
                opts[0].config_id, "mode",
                "an empty push emptied the picker, or a push from another \
                 session was ignored"
            );
        });
    }

    // ------------------------------------------- asks, and how they are lost

    /// An ask is written to the journal BEFORE it is queued, and what is
    /// written has to be readable months later by someone whose app was killed
    /// mid-turn. Both strings are resolved now because "now" is the only time
    /// they are knowable: a reconnect rebuilds the chat and refetches the list.
    #[test]
    fn an_ask_is_written_down_under_a_name_the_reader_will_recognise() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        app.run(|ctx| {
            ctx.sessions
                .clone()
                .set(vec![session("s2", Some("Nightly standup"))]);
        });
        pump_events(
            &app,
            vec![
                ask(
                    1,
                    "s1",
                    json!({"toolCallId": "t1", "title": "shell: uname -a"}),
                ),
                ask(2, "s2", json!({"toolCallId": "t2"})),
                ask(3, "s3", json!({"toolCallId": "t3"})),
            ],
        );

        let named: Vec<(String, String)> = app
            .journal()
            .into_iter()
            .map(|r| (r.tool_call_id, r.session_title))
            .collect();
        assert_eq!(
            named,
            vec![
                ("t1".to_owned(), "Deploy".to_owned()),
                ("t2".to_owned(), "Nightly standup".to_owned()),
                // Nothing knows this one's name, so the id is the honest
                // answer rather than a blank card.
                ("t3".to_owned(), "s3".to_owned()),
            ]
        );
        assert!(
            app.journal()
                .iter()
                .all(crate::ask_journal::AskRecord::is_open),
            "an ask that has just arrived is open, not already lost"
        );

        // Queued in arrival order and never replaced: each one blocks a turn
        // until it is answered, so dropping one hangs the agent.
        let queued: Vec<String> = app.run(|ctx| {
            ctx.permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect()
        });
        assert_eq!(queued, ["t1", "t2", "t3"]);
    }

    /// The measured case the whole journal exists for (docs/permission-
    /// durability.md section 0): the app is killed with an ask on screen, so
    /// nothing in that process ever got to say what became of it. The
    /// reconcile at the next launch is the only thing that can, and it has to
    /// write its verdict back — a launch that only decided in memory would
    /// call the same ask a fresh loss on every launch after this one.
    #[test]
    fn an_ask_the_app_was_killed_on_is_reported_at_the_next_launch() {
        let stranded = crate::ask_journal::AskRecord::open(
            "s1".to_owned(),
            "Deploy".to_owned(),
            "t1".to_owned(),
            "shell: uname -a".to_owned(),
            now_secs() - 60,
        );
        let app = App::mount_over(&vec![stranded]);
        let journal = app.journal();
        assert_eq!(journal.len(), 1, "the stranded ask was dropped at launch");
        assert_eq!(journal[0].title, "shell: uname -a");
        assert!(
            matches!(
                journal[0].state,
                crate::ask_journal::AskState::Lost {
                    cause: crate::ask_journal::LostCause::AppEnded,
                    ..
                }
            ),
            "an ask left open by a killed process is still being called open, so the \
             round it belonged to is lost in silence"
        );

        // A launch with nothing to reconcile changes nothing — and costs no
        // write, which is what the return value of the reconcile is for.
        let settled = app.journal();
        drop(app);
        let again = App::mount_over(&settled);
        assert_eq!(
            again.journal(),
            settled,
            "a second launch re-dated the loss"
        );
    }

    /// The card names the ask the way the modal named it. The fallback chain
    /// has to be the same one, or a loss is reported for a tool the reader
    /// never saw under that name.
    #[test]
    fn an_ask_with_no_title_falls_back_to_its_tool_and_then_to_a_sentence() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(
                    1,
                    "s1",
                    json!({"toolCallId": "t1", "title": "shell: uname -a"}),
                ),
                ask(
                    2,
                    "s1",
                    json!({"toolCallId": "t2",
                           "_meta": {"goose": {"toolCall": {"toolName": "developer__shell"}}}}),
                ),
                ask(3, "s1", json!({"toolCallId": "t3"})),
            ],
        );
        let titles: Vec<String> = app.journal().into_iter().map(|r| r.title).collect();
        assert_eq!(
            titles,
            ["shell: uname -a", "developer__shell", "Run a tool"],
            "a card with no name on it says nothing about what was lost"
        );
    }

    /// The agent can take its own question back. There is then nothing left to
    /// tell anyone, so the entry goes — a journal that kept it would report a
    /// lost round that was never lost.
    #[test]
    fn an_ask_the_agent_takes_back_leaves_nothing_behind() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "one"})),
                ask(2, "s1", json!({"toolCallId": "t2", "title": "two"})),
                AcpEvent::RequestCancelled {
                    request_id: json!(1),
                },
            ],
        );
        let left: Vec<String> = app.journal().into_iter().map(|r| r.tool_call_id).collect();
        assert_eq!(left, ["t2"], "the withdrawn ask is still being reported");
        let queued: Vec<String> = app.run(|ctx| {
            ctx.permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect()
        });
        assert_eq!(queued, ["t2"], "a withdrawn ask is still on screen");
    }

    /// The measured case (docs/permission-durability.md section 0): the socket
    /// goes, the round is destroyed on the server, and nothing in the
    /// transcript says so. This arm is the only thing that records the loss —
    /// and it must clear the queue too, because a modal offering Allow over a
    /// dead socket is a lie.
    #[test]
    fn a_dropped_connection_is_recorded_as_a_lost_round() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        app.run(|ctx| {
            ctx.want_connected.clone().set(true);
            ctx.running_sessions
                .clone()
                .set(std::iter::once("s1".to_owned()).collect());
            ctx.chat.clone().write().running = true;
        });
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                AcpEvent::Disconnected {
                    reason: "socket closed".to_owned(),
                    cause: DisconnectCause::Transport,
                },
            ],
        );

        let journal = app.journal();
        assert_eq!(journal.len(), 1);
        assert!(
            journal[0].is_unreported_loss(),
            "the round was destroyed and the journal still calls the ask open"
        );
        assert!(matches!(
            journal[0].state,
            crate::ask_journal::AskState::Lost {
                cause: crate::ask_journal::LostCause::Connection,
                ..
            }
        ));
        app.run(|ctx| {
            assert!(
                ctx.permission.peek().is_empty(),
                "an unanswerable ask is still on screen"
            );
            assert!(ctx.client.peek().is_none());
            assert!(!ctx.chat.peek().running, "the spinner outlived the socket");
            assert!(ctx.running_sessions.peek().is_empty());
            match &*ctx.conn.peek() {
                ConnState::Failed(message) => {
                    assert_eq!(message, "Connection lost: socket closed");
                }
                _ => panic!("a dropped connection did not report itself"),
            }
        });
    }

    /// The other cause, and the reason the transport reports one at all. The
    /// user pressed Disconnect: they chose it, so there is nothing to narrate,
    /// and the app must not come back to a card telling them a round was lost.
    #[test]
    fn a_disconnect_the_user_asked_for_narrates_nothing() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                AcpEvent::Disconnected {
                    reason: "closed by client".to_owned(),
                    cause: DisconnectCause::Local,
                },
            ],
        );
        assert!(
            app.journal().is_empty(),
            "pressing Disconnect left a lost-round card behind it"
        );
        app.run(|ctx| {
            assert!(matches!(*ctx.conn.peek(), ConnState::Disconnected));
        });
    }

    /// Answering an ask over a dead socket is theatre, so `send_prompt`'s
    /// `Closed` arm drops the queue instead. What it must NOT do is decide the
    /// round was lost: it cannot tell a dropped tailnet from a press of
    /// Disconnect, and the pump's arm — which can — is the only place that
    /// decides. An entry marked here would narrate a loss to a user who
    /// pressed the button themselves.
    #[test]
    fn abandoning_a_dead_sessions_asks_leaves_the_verdict_to_the_pump() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                ask(2, "s2", json!({"toolCallId": "t2", "title": "write"})),
            ],
        );
        app.run(|ctx| abandon_pending_permissions(ctx, "s1"));

        let queued: Vec<String> = app.run(|ctx| {
            ctx.permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect()
        });
        assert_eq!(queued, ["t2"], "another session's ask was dropped with it");
        assert!(
            app.journal()
                .iter()
                .all(crate::ask_journal::AskRecord::is_open),
            "the abandon path decided a round was lost, which is not its call"
        );
    }

    /// Answering is keyed on the JSON-RPC id — one reply can settle several
    /// queued asks — while the journal is keyed on the tool call, because the
    /// id belongs to a socket that is gone by the time any of this matters.
    /// Resolving the wrong one leaves a card reporting an ask that was
    /// answered.
    #[test]
    fn answering_an_ask_clears_it_from_the_queue_and_the_journal() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                ask(1, "s1", json!({"toolCallId": "t2", "title": "shell again"})),
                ask(2, "s1", json!({"toolCallId": "t3", "title": "write"})),
            ],
        );
        app.run(|ctx| answer_permission(ctx, &json!(1), Some("allow_once".to_owned())));

        let queued: Vec<String> = app.run(|ctx| {
            ctx.permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect()
        });
        assert_eq!(queued, ["t3"]);
        let left: Vec<String> = app.journal().into_iter().map(|r| r.tool_call_id).collect();
        assert_eq!(left, ["t3"], "an answered ask is still reported as pending");
    }

    /// Dismissal has to stick. The entry is kept rather than deleted so a
    /// second sighting of the same ask cannot undo it, but it stops being a
    /// loss the moment the reader has read it.
    #[test]
    fn a_dismissed_loss_stops_being_reported() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                AcpEvent::Disconnected {
                    reason: "socket closed".to_owned(),
                    cause: DisconnectCause::Transport,
                },
            ],
        );
        assert_eq!(
            crate::ask_journal::loss_count(&app.journal(), "s1"),
            1,
            "the loss was never reported in the first place"
        );

        app.run(|ctx| dismiss_lost_ask(ctx, "t1"));
        assert_eq!(
            crate::ask_journal::loss_count(&app.journal(), "s1"),
            0,
            "a dismissed card came straight back"
        );
        assert_eq!(
            app.journal().len(),
            1,
            "the entry was deleted, so the next sighting of this ask would \
             undo the dismissal"
        );
    }

    // ---------------------------------------- opening, sending, and offline

    /// Walking into a different conversation. The draft, the attachment tray
    /// and the transcript all belong to the chat that was open, and carrying
    /// any of them across is how a photo picked for one chat ends up in
    /// another chat's next message.
    #[test]
    fn opening_a_different_chat_leaves_the_last_ones_draft_and_tray_behind() {
        let app = App::mount();
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState {
                session_id: Some("s1".to_owned()),
                title: "Deploy".to_owned(),
                items: vec![ChatItem::User {
                    text: "look".to_owned(),
                    attachments: vec![crate::attach::Attachment {
                        name: "roof.jpg".to_owned(),
                        ..crate::attach::Attachment::default()
                    }],
                }],
                ..ChatState::default()
            });
            ctx.chat_draft.clone().set("half typed".to_owned());
            ctx.attachments
                .clone()
                .set(vec![crate::attach::PendingAttachment {
                    record: crate::attach::Attachment::default(),
                    data: "QUJD".to_owned(),
                    text: None,
                }]);
            ctx.running_sessions
                .clone()
                .set(std::iter::once("s2".to_owned()).collect());
            ctx.usage.clone().set(Some((10, 100)));

            open_session(ctx, session("s2", None));

            let chat = ctx.chat.peek();
            assert!(matches!(*ctx.screen.peek(), Screen::Chat));
            assert_eq!(chat.session_id.as_deref(), Some("s2"));
            assert_eq!(
                chat.title, "s2",
                "a session goose never named is shown by its id, not blank"
            );
            assert_eq!(chat.cwd, "/", "a session with no cwd still has one");
            assert!(
                chat.items.is_empty(),
                "the last chat's transcript is still on screen"
            );
            assert!(chat.loading, "the replay is under way and nothing says so");
            assert!(chat.running, "a session with a turn in flight opened idle");
            assert!(
                chat.attach_replay.is_empty(),
                "another conversation's photos are waiting to be adopted here"
            );
            assert_eq!(
                *ctx.chat_draft.peek(),
                "",
                "half a message followed the reader out"
            );
            assert!(ctx.attachments.peek().is_empty(), "the tray followed too");
            assert_eq!(
                *ctx.usage.peek(),
                None,
                "the old chat's context meter is still up"
            );
        });
    }

    /// Backing out of a chat and going straight back in replays it from
    /// scratch, and the replay cannot say what a photo was called or what it
    /// looked like. So what the transcript knew is carried across — and so is
    /// what was being typed, because the conversation did not change.
    #[test]
    fn reopening_the_same_chat_keeps_what_was_typed_and_what_was_sent() {
        let app = App::mount();
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState {
                session_id: Some("s3".to_owned()),
                items: vec![ChatItem::User {
                    text: "look".to_owned(),
                    attachments: vec![crate::attach::Attachment {
                        name: "roof.jpg".to_owned(),
                        mime: "image/jpeg".to_owned(),
                        size: 3,
                        thumb: "THUMB".to_owned(),
                    }],
                }],
                ..ChatState::default()
            });
            ctx.chat_draft.clone().set("keep me".to_owned());

            let info: SessionInfo = serde_json::from_value(
                json!({"sessionId": "s3", "title": "Roof", "cwd": "/srv/app"}),
            )
            .unwrap();
            open_session(ctx, info);

            let chat = ctx.chat.peek();
            assert_eq!(chat.title, "Roof");
            assert_eq!(chat.cwd, "/srv/app");
            let carried: Vec<&str> = chat.attach_replay.iter().map(|a| a.name.as_str()).collect();
            assert_eq!(
                carried,
                ["roof.jpg"],
                "the replay has nothing to give the photo its name back with"
            );
            assert_eq!(
                *ctx.chat_draft.peek(),
                "keep me",
                "leaving a chat and coming back ate what was being written"
            );
        });
    }

    /// Opening a chat with no connection has to end the spinner and say why.
    /// The load runs on a task of its own, so the arm that answers this is
    /// reached only after the scheduler gets a turn — which is exactly the
    /// half of the function a test that never drains would miss.
    #[test]
    fn opening_a_chat_offline_stops_the_spinner_and_says_so() {
        let mut app = App::mount();
        app.run(|ctx| open_session(ctx, session("s2", Some("Nightly standup"))));
        assert!(app.run(|ctx| ctx.chat.peek().loading));
        assert_eq!(app.toast(), None, "nothing has been tried yet");

        app.drain();
        assert_eq!(
            app.toast().as_deref(),
            Some("Not connected — reconnect in Settings")
        );
        assert!(
            !app.run(|ctx| ctx.chat.peek().loading),
            "the chat is stuck showing a replay that will never arrive"
        );
    }

    /// The working directory is a path on the *server*, and goose refuses a
    /// relative one. Catching it here is the difference between a sentence
    /// naming the Settings field and an RPC error nobody can act on.
    #[test]
    fn a_new_chat_needs_an_absolute_working_directory_before_anything_is_sent() {
        let mut app = App::mount();
        for bad in ["", "   ", "work/goose", "~/work"] {
            app.run(|ctx| {
                ctx.toast.clone().set(None);
                ctx.settings.clone().set(Settings {
                    working_dir: bad.to_owned(),
                    ..Settings::default()
                });
                new_session(ctx);
            });
            let toast = app.toast();
            assert_eq!(
                toast.as_deref(),
                Some("Set an absolute working directory (a path on the server) in Settings first"),
                "`{bad}` was accepted as a working directory"
            );
            assert!(
                matches!(app.run(|ctx| *ctx.screen.peek()), Screen::Settings),
                "a refused new chat navigated anyway"
            );
        }

        // An absolute one gets as far as the transport, and stops there.
        app.run(|ctx| {
            ctx.toast.clone().set(None);
            ctx.settings.clone().set(Settings {
                working_dir: "  /srv/work  ".to_owned(),
                ..Settings::default()
            });
            new_session_with(ctx, json!({"recipeId": "standup"}));
        });
        assert_eq!(app.toast(), None, "the request has not been made yet");
        app.drain();
        assert_eq!(
            app.toast().as_deref(),
            Some("Not connected — reconnect in Settings")
        );
    }

    /// A send that could not be handed to the transport must report false and
    /// leave the composer alone — the caller clears the draft and the tray on
    /// a `true`, so a wrong answer here is a message and its photos deleted
    /// without ever being sent.
    #[test]
    fn a_send_that_cannot_reach_the_transport_keeps_the_message() {
        let app = App::mount();
        app.run(|ctx| {
            // No chat open at all: nothing to send into, and nothing to say
            // about it either.
            assert!(!send_prompt(ctx, "hello".to_owned(), &[]));
            assert_eq!(*ctx.toast.peek(), None);
            assert!(ctx.chat.peek().items.is_empty());
        });

        app.open_chat("s1", "Deploy");
        app.run(|ctx| {
            assert!(!send_prompt(ctx, "hello".to_owned(), &[]));
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Not connected — reconnect in Settings")
            );
            assert!(
                ctx.chat.peek().items.is_empty(),
                "the message was drawn into the transcript as though it had been sent"
            );
            assert!(
                !ctx.chat.peek().running,
                "a turn that never started is spinning"
            );
        });
    }

    /// Stop answers the open asks "cancelled" first, over a live socket. With
    /// no socket there is nothing to answer into, so it does nothing — and
    /// doing nothing has to include not quietly emptying the queue, or an ask
    /// still blocking the agent would vanish off the screen.
    #[test]
    fn stopping_a_turn_with_no_socket_does_not_swallow_the_open_asks() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"}))],
        );
        app.run(|ctx| {
            stop_turn(ctx);
            assert_eq!(
                ctx.permission.peek().len(),
                1,
                "the ask was dropped unanswered"
            );
        });

        // And with no chat open there is not even a session to cancel.
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState::default());
            stop_turn(ctx);
            assert_eq!(ctx.permission.peek().len(), 1);
        });
    }

    /// Every route out of this module that needs the transport has to fail
    /// closed: no spinner left up, no row rewritten, no picker changed, and
    /// above all no transcript emptied for a replay that is never coming.
    #[test]
    fn nothing_pretends_to_have_reached_a_server_that_is_not_there() {
        let app = App::mount();
        app.run(|ctx| {
            ctx.sessions
                .clone()
                .set(vec![session("s1", Some("old name"))]);
            ctx.chat.clone().set(ChatState {
                session_id: Some("s1".to_owned()),
                title: "old name".to_owned(),
                items: vec![ChatItem::Assistant {
                    message_id: None,
                    text: "yesterday's answer".to_owned(),
                }],
                ..ChatState::default()
            });
            ctx.config_options
                .clone()
                .set(serde_json::from_value(json!([{"id": "mode", "name": "Mode"}])).unwrap());

            // A list refresh with no client must not arm the spinner it will
            // never take back down.
            refresh_sessions(ctx, false).now_or_never().unwrap();
            assert!(
                !*ctx.sessions_loading.peek(),
                "an offline refresh left a spinner up"
            );
            assert_eq!(*ctx.sessions_epoch.peek(), 0, "it claimed the list anyway");

            // A rename that never reached the server must not rewrite the row.
            rename_session(ctx, "s1", "  ").now_or_never().unwrap();
            rename_session(ctx, "s1", "new name")
                .now_or_never()
                .unwrap();
            assert_eq!(
                ctx.sessions.peek()[0].title.as_deref(),
                Some("old name"),
                "the row shows a name the server never agreed to"
            );
            assert_eq!(ctx.chat.peek().title, "old name");

            // A reload must not clear the transcript it cannot replace.
            reload_chat(ctx, "s1".to_owned(), "/srv".to_owned())
                .now_or_never()
                .unwrap();
            assert_eq!(
                transcript(&ctx.chat.peek().items),
                ["assistant:-:yesterday's answer"],
                "an offline reload blanked the chat and left it blank"
            );
            assert!(!ctx.chat.peek().loading);

            // And a tap on the mode chip must not redraw it as though it took.
            set_config_option(ctx, "mode", "chat");
            assert_eq!(ctx.config_options.peek()[0].config_id, "mode");
            assert_eq!(ctx.config_options.peek().len(), 1);
        });
    }

    /// Typing in the search box retires the page after this one the instant
    /// the query changes: goose ties a cursor to the filters it was minted
    /// under and rejects a mismatched pair outright, so a "Load more" left
    /// armed across a new search is an `invalid_params` waiting to happen.
    /// A search that is not actually different must leave it alone, or every
    /// space bar throws away the list you are reading.
    #[test]
    fn changing_the_search_retires_the_page_that_belonged_to_the_last_one() {
        let app = App::mount();
        app.run(|ctx| {
            let armed = SessionQuery::new(&SessionKind::ALL, Some("deploy"))
                .next_page(&page(&["chat_1"], Some("cursor-of-deploy")));
            assert!(armed.is_some(), "the fixture never armed the button");

            ctx.sessions_query.clone().set("deploy".to_owned());
            ctx.sessions_next.clone().set(armed.clone());

            search_sessions(ctx, "  deploy ".to_owned())
                .now_or_never()
                .unwrap();
            assert_eq!(
                *ctx.sessions_next.peek(),
                armed,
                "a trailing space threw away the page the reader was about to ask for"
            );

            search_sessions(ctx, "rollback".to_owned())
                .now_or_never()
                .unwrap();
            assert_eq!(*ctx.sessions_query.peek(), "rollback");
            assert_eq!(
                *ctx.sessions_next.peek(),
                None,
                "\"Load more\" is still armed with the previous search's cursor"
            );
        });
    }

    // ------------------------------------------------ a goose on a loopback port
    //
    // Everything above this line runs with no socket, and that is only half of
    // this file. The other half — the successful arm of `establish`, the
    // reconnect loop, and the body of every request — is reachable only with a
    // live `AcpClient`, which has no constructor that is not `connect`. So the
    // tests below put a server in front of it: a plain-`ws://` JSON-RPC
    // listener on a loopback port that logs every frame it is sent and answers
    // from a script. `ws_url` only reaches for TLS on an `https://` base, so
    // `http://` here means no certificate and no pin.
    //
    // The technique is `src/scheduler.rs`'s. What is different is that these
    // tests mount the REAL provider through `App`, so the pump reading the
    // socket is the app's own and the ask journal is a real file.

    /// The reply to one request: how long the server sits on it, then a
    /// JSON-RPC `result` or `error`.
    ///
    /// The delay is not decoration. Two of the rules in this file are about
    /// *ordering* — a page that answers after a newer search has claimed the
    /// list must write nowhere, and must not take that newer fetch's spinner
    /// down on its way past — and neither can be provoked on a server that
    /// answers in the order it was asked.
    type Reply = (Duration, Result<Value, Value>);

    /// What a server answers, per method. A plain `fn` and never a closure, so
    /// a test's whole script is one readable `match`.
    type Script = fn(&str, &Value) -> Reply;

    fn ok(result: Value) -> Reply {
        (Duration::ZERO, Ok(result))
    }

    fn rpc_error(code: i64, message: &str) -> Reply {
        (
            Duration::ZERO,
            Err(json!({ "code": code, "message": message })),
        )
    }

    /// Not a reply at all: the server hangs up instead of answering. The only
    /// way to reach [`AcpError::Closed`], which `send_prompt` treats
    /// differently from every other failure.
    fn hang_up() -> Reply {
        (Duration::ZERO, Err(Value::Null))
    }

    /// One `configOptions` entry in goose's own spelling — `id`, not
    /// `configId` — so a test cannot assert against a shape no server sends.
    fn wire_option(id: &str, current: &str) -> Value {
        json!({"id": id, "name": id, "type": "select", "currentValue": current,
               "options": [{"value": current, "name": current}]})
    }

    /// A goose that answers everything.
    ///
    /// The stop reason it ends a turn with is whatever the message said, so
    /// one script can drive every arm of `send_prompt`'s post-turn match.
    fn happy(method: &str, params: &Value) -> Reply {
        match method {
            "initialize" => ok(json!({
                "protocolVersion": 1,
                "agentInfo": {"name": "goose", "version": "1.46.0"},
            })),
            "session/list" => ok(json!({"sessions": [{"sessionId": "chat_1"}]})),
            "session/load" => ok(json!({"configOptions": [wire_option("model", "sonnet")]})),
            "session/new" => ok(json!({
                "sessionId": "s_new",
                "configOptions": [wire_option("mode", "auto")],
            })),
            "session/prompt" => ok(json!({"stopReason": prompt_text(params)})),
            "session/set_config_option" => {
                ok(json!({"configOptions": [wire_option("mode", "chat")]}))
            }
            _ => ok(json!({})),
        }
    }

    /// The first text block of a `session/prompt`, which the scripts read as
    /// the test's instruction to the server.
    fn prompt_text(params: &Value) -> String {
        params
            .pointer("/prompt/0/text")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
            .to_owned()
    }

    struct Server {
        base_url: String,
        /// Every frame the app sent, in order: requests, notifications, and the
        /// answers it gave to the agent's own asks.
        frames: Arc<Mutex<Vec<Value>>>,
    }

    impl Server {
        fn log(&self) -> Vec<Value> {
            self.frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// What the app sent, in order, one word each: the method it called, or
        /// `answer` for a reply to one of the agent's own requests. The
        /// handshake is left out — it is the harness's, not the screen's.
        fn traffic(&self) -> Vec<String> {
            self.log()
                .iter()
                .map(|frame| {
                    frame
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or("answer")
                        .to_owned()
                })
                .filter(|word| word != "initialize")
                .collect()
        }

        fn methods(&self) -> Vec<String> {
            self.traffic()
                .into_iter()
                .filter(|w| w != "answer")
                .collect()
        }

        fn count(&self, method: &str) -> usize {
            self.methods().iter().filter(|m| *m == method).count()
        }

        /// The params of the `n`th call to `method`.
        fn params(&self, method: &str, n: usize) -> Value {
            self.log()
                .iter()
                .filter(|frame| frame.get("method").and_then(Value::as_str) == Some(method))
                .nth(n)
                .and_then(|frame| frame.get("params").cloned())
                .expect("the call the assertion is about was never made")
        }

        /// The outcome of every permission reply the app sent, in order.
        fn answers(&self) -> Vec<Value> {
            self.log()
                .iter()
                .filter(|frame| frame.get("method").is_none())
                .filter_map(|frame| frame.get("result").cloned())
                .collect()
        }
    }

    /// Stand a goose up on a loopback port, answering from `script`.
    fn serve(script: Script) -> Server {
        let listener = tokio_rt()
            .block_on(TcpListener::bind("127.0.0.1:0"))
            .expect("a loopback port for the mock server");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        let frames: Arc<Mutex<Vec<Value>>> = Arc::default();
        let log = Arc::clone(&frames);
        tokio_rt().spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                tokio::spawn(session_loop(socket, log, script));
            }
        });
        Server {
            base_url: format!("http://127.0.0.1:{port}"),
            frames,
        }
    }

    async fn session_loop(
        socket: tokio::net::TcpStream,
        log: Arc<Mutex<Vec<Value>>>,
        script: Script,
    ) {
        let Ok(ws) = tokio_tungstenite::accept_async(socket).await else {
            return;
        };
        let (mut sink, mut stream) = ws.split();
        let (out, mut outbox) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(text) = outbox.recv().await {
                if text.is_empty() {
                    // The hang-up: a close frame and no answer, which is what
                    // reaches the client as `AcpError::Closed`.
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(frame.clone());
            // A frame with no method is the app ANSWERING one of the agent's
            // own asks, and one with no id is a notification. Neither is a
            // question, so neither gets a reply.
            let (Some(method), Some(id)) = (
                frame
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                frame.get("id").cloned(),
            ) else {
                continue;
            };
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            let out = out.clone();
            // Answered on a task of its own, so a scripted delay holds up one
            // reply rather than the whole socket.
            tokio::spawn(async move {
                let (delay, body) = script(&method, &params);
                tokio::time::sleep(delay).await;
                let reply = match body {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err(Value::Null) => {
                        let _ = out.send(String::new());
                        return;
                    }
                    Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
                };
                let _ = out.send(reply.to_string());
            });
        }
    }

    /// Wait for a connection's event stream to say it is over, and report who
    /// ended it.
    fn ended(events: &mut mpsc::Receiver<AcpEvent>) -> Option<DisconnectCause> {
        tokio_rt().block_on(async {
            loop {
                match tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
                    Ok(Some(AcpEvent::Disconnected { cause, .. })) => return Some(cause),
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => return None,
                }
            }
        })
    }

    /// A loopback address with nothing listening on it: bound to learn a free
    /// port and then let go. A connection to it is refused at once, which is
    /// what a tailnet that is not up yet does — and is not the 20-second
    /// connect timeout a blackholed address would cost.
    fn dead_port() -> String {
        let listener = tokio_rt()
            .block_on(TcpListener::bind("127.0.0.1:0"))
            .expect("a loopback port to let go of again");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    /// A photo in the composer, picked and read and not yet sent.
    fn pending(name: &str) -> crate::attach::PendingAttachment {
        crate::attach::PendingAttachment {
            record: crate::attach::Attachment {
                name: name.to_owned(),
                mime: "image/jpeg".to_owned(),
                size: 3,
                thumb: "THUMB".to_owned(),
            },
            data: "QUJD".to_owned(),
            text: None,
        }
    }

    // ------------------------------------------------------- the connection

    /// The successful half of Connect, which the Settings button and every
    /// reconnect run through. The agent's name is what the connection badge
    /// shows — the only thing on screen that says which server this is — and
    /// `want_connected` is what arms the reconnect loop, so a connection that
    /// came up without setting it would never come back from a tailnet blink.
    #[test]
    fn connecting_names_the_agent_and_arms_the_reconnect_loop() {
        let app = App::mount();
        let server = serve(happy);
        assert!(
            app.dial(&server),
            "the handshake failed against a server that answered it"
        );
        app.run(|ctx| {
            match &*ctx.conn.peek() {
                ConnState::Connected { agent } => assert_eq!(
                    agent, "goose 1.46.0",
                    "the badge names the agent and the version it reported"
                ),
                _ => panic!("a handshake that succeeded did not leave the app connected"),
            }
            assert!(
                ctx.client.peek().is_some(),
                "connected with nothing to make a request over"
            );
            assert!(
                *ctx.want_connected.peek(),
                "a connection that came up did not arm the reconnect loop"
            );
        });
        assert!(
            server.methods().is_empty(),
            "connecting asked the server for something else: {:?}",
            server.methods()
        );
    }

    /// The other half of the same line. goose's `agentInfo` may carry no
    /// version, and joining on it unconditionally would put a trailing space
    /// in the badge.
    #[test]
    fn an_agent_that_reports_no_version_is_named_by_itself() {
        fn versionless(method: &str, params: &Value) -> Reply {
            if method == "initialize" {
                return ok(json!({"protocolVersion": 1, "agentInfo": {"name": "goose"}}));
            }
            happy(method, params)
        }
        let app = App::mount();
        let server = serve(versionless);
        assert!(app.dial(&server));
        app.run(|ctx| match &*ctx.conn.peek() {
            ConnState::Connected { agent } => assert_eq!(agent, "goose"),
            _ => panic!("a handshake that succeeded did not leave the app connected"),
        });
    }

    /// Pressing Connect over a live connection has to hang the old socket up
    /// before opening another. Left running, that connection goes on answering
    /// the agent's asks from a screen nobody is looking at — and holds a
    /// session open on the server for as long as the app lives.
    #[test]
    fn connecting_over_a_live_connection_hangs_the_old_socket_up() {
        let app = App::mount();
        let old = serve(happy);
        let mut old_events = app.attach(&old);
        // A second handle on that connection, held here so that *replacing*
        // `ctx.client` cannot be what ends it. Only `close` can.
        let held = app
            .run(|ctx| ctx.client.peek().clone())
            .expect("the client the harness just attached");

        let fresh = serve(happy);
        assert!(app.dial(&fresh));
        assert!(
            matches!(ended(&mut old_events), Some(DisconnectCause::Local)),
            "the previous connection is still up after Connect opened another"
        );
        drop(held);
    }

    /// Disconnect has to reach the socket and not just the flag. The other
    /// half of it — that the app stops wanting to be connected — is checked
    /// above; this is the half that lets go of the connection.
    #[test]
    fn disconnecting_hangs_the_socket_up() {
        let app = App::mount();
        let server = serve(happy);
        let mut events = app.attach(&server);
        let held = app
            .run(|ctx| ctx.client.peek().clone())
            .expect("the client the harness just attached");
        app.run(disconnect);
        assert!(
            matches!(ended(&mut events), Some(DisconnectCause::Local)),
            "Disconnect left the socket open"
        );
        drop(held);
    }

    /// The whole recovery path, end to end: the tailnet drops mid-chat, the
    /// pump records it and arms a retry, the retry dials back out, and the open
    /// chat is replayed.
    ///
    /// `session/load` is the ONE channel through which goose could ever
    /// re-raise an ask it still holds, so a reconnect that skipped it would
    /// close the only door a parked round can come back through.
    #[test]
    fn a_dropped_tailnet_dials_back_out_and_replays_the_open_chat() {
        let mut app = App::mount();
        let server = serve(happy);
        app.run(|ctx| {
            ctx.settings.clone().set(Settings {
                server_url: server.base_url.clone(),
                ..Settings::default()
            });
            ctx.want_connected.clone().set(true);
            ctx.chat.clone().set(ChatState {
                session_id: Some("s1".to_owned()),
                cwd: "/srv/work".to_owned(),
                title: "Deploy".to_owned(),
                items: vec![ChatItem::Assistant {
                    message_id: None,
                    text: "before the drop".to_owned(),
                }],
                ..ChatState::default()
            });
        });
        pump_events(
            &app,
            vec![AcpEvent::Disconnected {
                reason: "socket closed".to_owned(),
                cause: DisconnectCause::Transport,
            }],
        );

        // The first rung of the ramp is two seconds; the budget is the slack.
        app.settle_until(Duration::from_secs(15), |ctx| {
            !ctx.config_options.peek().is_empty()
        });

        app.run(|ctx| {
            assert!(
                ctx.conn.peek().is_connected(),
                "the phone never dialled back out"
            );
            assert!(ctx.client.peek().is_some());
            let chat = ctx.chat.peek();
            assert!(
                chat.items.is_empty(),
                "the transcript survived the reconnect, so the replay lands underneath \
                 a copy of itself"
            );
            assert!(!chat.loading, "the replay's spinner was never taken down");
            assert_eq!(
                ctx.config_options.peek()[0].config_id,
                "model",
                "the options the reload answered with were dropped, so the model \
                 picker stays empty until the agent happens to push another set"
            );
        });
        assert_eq!(
            server.params("session/load", 0),
            json!({"sessionId": "s1", "cwd": "/srv/work", "mcpServers": []}),
            "the replay went out for the wrong session or against the wrong directory"
        );
    }

    /// The reconnect loop sleeps first and asks afterwards, so both of its
    /// questions are asked at the moment they matter. A loop that asked before
    /// sleeping would dial out for a user who pressed Disconnect during the
    /// ramp, and would hang a second connection off a phone that had already
    /// come back some other way.
    #[test]
    fn a_reconnect_gives_up_when_there_is_nothing_left_to_reconnect() {
        let app = App::mount();
        let server = serve(happy);
        app.run(|ctx| {
            ctx.settings.clone().set(Settings {
                server_url: server.base_url.clone(),
                ..Settings::default()
            });
        });

        // The user pressed Disconnect while the first rung was sleeping.
        app.drive(|ctx| async move { reconnect_loop(&ctx).await });
        app.run(|ctx| {
            assert!(
                ctx.client.peek().is_none(),
                "the phone dialled back out after the user pressed Disconnect"
            );
            assert!(matches!(*ctx.conn.peek(), ConnState::Disconnected));
        });

        // And a connection that came back another way ends the loop rather
        // than being closed and replaced by one of its own.
        app.run(|ctx| {
            ctx.want_connected.clone().set(true);
            ctx.conn.clone().set(ConnState::Connected {
                agent: "goose".to_owned(),
            });
        });
        app.drive(|ctx| async move { reconnect_loop(&ctx).await });
        app.run(|ctx| {
            assert!(
                ctx.client.peek().is_none(),
                "a second connection was opened over a live one"
            );
        });
        assert!(
            server.traffic().is_empty(),
            "the loop reached the server: {:?}",
            server.traffic()
        );
    }

    /// The loop is a *loop*. A phone coming out of a tunnel finds a tailnet
    /// that is not up yet, and the attempt that finally works is rarely the
    /// first one — a loop that gave up on a refused connection would leave the
    /// app disconnected with nothing left to bring it back but the user
    /// noticing. And a phone with no chat open has nothing to replay, so it
    /// must not ask the server to load one.
    #[test]
    fn a_reconnect_keeps_trying_until_the_server_is_there() {
        let mut app = App::mount();
        let server = serve(happy);
        app.run(|ctx| {
            ctx.settings.clone().set(Settings {
                server_url: dead_port(),
                ..Settings::default()
            });
            ctx.want_connected.clone().set(true);
            let ctx = *ctx;
            spawn_forever(async move { reconnect_loop(&ctx).await });
        });

        // The first rung of the ramp is two seconds, and the attempt at the end
        // of it is refused.
        app.settle_until(Duration::from_secs(15), |ctx| {
            matches!(*ctx.conn.peek(), ConnState::Failed(_))
        });
        app.run(|ctx| {
            assert!(
                ctx.client.peek().is_none(),
                "a connection that was refused was kept anyway"
            );
        });

        // The tailnet comes up while the second rung is sleeping.
        app.run(|ctx| {
            ctx.settings.clone().set(Settings {
                server_url: server.base_url.clone(),
                ..Settings::default()
            });
        });
        app.settle_until(Duration::from_secs(20), |ctx| {
            ctx.conn.peek().is_connected()
        });
        app.run(|ctx| {
            assert!(
                ctx.conn.peek().is_connected(),
                "the loop gave up after one refused connection"
            );
        });
        assert!(
            server.methods().is_empty(),
            "a reconnect with no chat open asked the server to replay one: {:?}",
            server.methods()
        );
    }

    /// A `session/load` takes as long as the transcript is — the server
    /// replays the whole thing before it answers — and the reader can walk
    /// into another chat while it does. An answer that lands in a chat nobody
    /// is looking at has to write nothing at all: not the spinner, not the
    /// options, and above all not a failure about a session that is no longer
    /// on screen.
    #[test]
    fn a_replay_that_answers_after_the_reader_moved_on_writes_nothing() {
        fn dawdles(method: &str, params: &Value) -> Reply {
            if method == "session/load"
                && params.get("sessionId").and_then(Value::as_str) == Some("slow")
            {
                return (
                    Duration::from_millis(400),
                    Err(json!({"code": -32000, "message": "too late to matter"})),
                );
            }
            happy(method, params)
        }
        let mut app = App::mount();
        let server = serve(dawdles);
        let _events = app.attach(&server);

        let slow: SessionInfo =
            serde_json::from_value(json!({"sessionId": "slow", "cwd": "/srv"})).unwrap();
        app.run(|ctx| open_session(ctx, slow));
        app.settle_until(Duration::from_millis(80), |_| false);
        let quick: SessionInfo =
            serde_json::from_value(json!({"sessionId": "quick", "cwd": "/srv"})).unwrap();
        app.run(|ctx| open_session(ctx, quick));
        app.settle_until(Duration::from_millis(900), |_| false);

        assert_eq!(
            app.toast(),
            None,
            "a chat the reader had already left reported its own failure over the \
             one they are now reading"
        );
        app.run(|ctx| {
            assert_eq!(ctx.chat.peek().session_id.as_deref(), Some("quick"));
            assert!(!ctx.chat.peek().loading);
        });
        assert_eq!(server.count("session/load"), 2);

        // The same rule one function over, for the reload a reconnect runs.
        app.run(|ctx| {
            let ctx = *ctx;
            spawn_forever(async move {
                reload_chat(&ctx, "slow".to_owned(), "/srv".to_owned()).await;
            });
        });
        app.settle_until(Duration::from_millis(80), |_| false);
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState {
                session_id: Some("quick".to_owned()),
                loading: true,
                ..ChatState::default()
            });
        });
        app.settle_until(Duration::from_millis(900), |_| false);
        assert_eq!(app.toast(), None, "the same failure, one function over");
        app.run(|ctx| {
            assert!(
                ctx.chat.peek().loading,
                "a reload belonging to a chat the reader had left took down the \
                 spinner of the chat that replaced it"
            );
        });
    }

    /// A reload that the server refuses still empties the transcript — the
    /// replay is what refills it and there is no half measure — so the failure
    /// has to be said out loud, and what this device knows about the photos in
    /// it has to be kept for the replay that will bring them back.
    #[test]
    fn a_reload_the_server_refuses_says_so_and_keeps_the_photos_for_the_replay() {
        fn refuses(method: &str, params: &Value) -> Reply {
            if method == "session/load" {
                return rpc_error(-32602, "unknown session id");
            }
            happy(method, params)
        }
        let app = App::mount();
        let server = serve(refuses);
        let _events = app.attach(&server);
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState {
                session_id: Some("s1".to_owned()),
                cwd: "/srv".to_owned(),
                items: vec![ChatItem::User {
                    text: "look".to_owned(),
                    attachments: vec![crate::attach::Attachment {
                        name: "roof.jpg".to_owned(),
                        mime: "image/jpeg".to_owned(),
                        size: 3,
                        thumb: "THUMB".to_owned(),
                    }],
                }],
                ..ChatState::default()
            });
        });
        app.drive(|ctx| async move { reload_chat(&ctx, "s1".to_owned(), "/srv".to_owned()).await });

        assert_eq!(
            app.toast().as_deref(),
            Some("Failed to reload session: unknown session id"),
            "a replay that never happened left the chat blank and said nothing"
        );
        app.run(|ctx| {
            let chat = ctx.chat.peek();
            assert!(
                !chat.loading,
                "the chat is stuck showing a replay that is not coming"
            );
            let carried: Vec<&str> = chat.attach_replay.iter().map(|a| a.name.as_str()).collect();
            assert_eq!(
                carried,
                ["roof.jpg"],
                "the replay has nothing left to give the photo its name back with"
            );
        });
    }

    // -------------------------------------------------------- the chats list

    /// Paging. goose binds a cursor to the filters it was minted under, so the
    /// only cursor that may go out is the one the last page answered with —
    /// and "Load more" over a list that is already whole must ask for nothing,
    /// rather than silently re-fetching page one on top of itself.
    #[test]
    fn the_chats_list_pages_from_the_cursor_the_server_minted() {
        fn paged(method: &str, params: &Value) -> Reply {
            if method == "session/list" {
                return match params.get("cursor").and_then(Value::as_str) {
                    None => {
                        ok(json!({"sessions": [{"sessionId": "chat_1"}], "nextCursor": "cursor-1"}))
                    }
                    Some("cursor-1") => {
                        ok(json!({"sessions": [{"sessionId": "chat_2"}], "nextCursor": null}))
                    }
                    Some(_) => rpc_error(-32602, "session list cursor does not match filters"),
                };
            }
            happy(method, params)
        }
        let app = App::mount();
        let server = serve(paged);
        let _events = app.attach(&server);

        app.drive(|ctx| async move { refresh_sessions(&ctx, false).await });
        app.run(|ctx| {
            assert_eq!(ids(&ctx.sessions.peek()), ["chat_1"]);
            assert!(
                ctx.sessions_next.peek().is_some(),
                "the server offered another page and \"Load more\" is not armed"
            );
            assert!(
                !*ctx.sessions_loading.peek(),
                "the spinner outlived the page"
            );
            assert_eq!(
                *ctx.sessions_epoch.peek(),
                1,
                "the fetch never claimed the list"
            );
        });
        assert_eq!(
            server.params("session/list", 0),
            json!({"_meta": {
                "types": ["user", "scheduled", "acp"],
                "goose": {"includeLastMessageSnippet": true},
            }}),
            "page one went out with a cursor on it, or asking for the wrong kinds"
        );

        app.drive(|ctx| async move { refresh_sessions(&ctx, true).await });
        app.run(|ctx| {
            assert_eq!(
                ids(&ctx.sessions.peek()),
                ["chat_1", "chat_2"],
                "\"Load more\" replaced the list instead of adding to it"
            );
            assert_eq!(
                *ctx.sessions_next.peek(),
                None,
                "the last page still offers one after it"
            );
        });
        assert_eq!(
            server.params("session/list", 1).get("cursor"),
            Some(&json!("cursor-1")),
            "the next page was asked for without the cursor that names it"
        );

        // The list is whole, so there is nothing left to ask for.
        app.drive(|ctx| async move { refresh_sessions(&ctx, true).await });
        assert_eq!(
            server.count("session/list"),
            2,
            "\"Load more\" over a complete list re-fetched page one"
        );
    }

    /// The three ways a `session/list` ends other than well, and what each one
    /// is allowed to touch.
    #[test]
    fn a_list_that_fails_or_answers_late_leaves_the_chats_on_screen_alone() {
        /// Keyed on the search box: `boom` fails at once, `late` fails slowly,
        /// and the empty search answers slowly.
        fn contrary(method: &str, params: &Value) -> Reply {
            if method != "session/list" {
                return happy(method, params);
            }
            match params.pointer("/_meta/query").and_then(Value::as_str) {
                Some("boom") => rpc_error(-32000, "the session store is unavailable"),
                Some(_) => (
                    Duration::from_millis(120),
                    Err(json!({"code": -32000, "message": "too late to matter"})),
                ),
                None => (
                    Duration::from_millis(120),
                    Ok(json!({"sessions": [{"sessionId": "stale"}],
                              "nextCursor": "cursor-of-a-search-nobody-is-running"})),
                ),
            }
        }
        let mut app = App::mount();
        let server = serve(contrary);
        let _events = app.attach(&server);
        app.run(|ctx| {
            ctx.sessions
                .clone()
                .set(vec![session("chat_1", Some("Deploy"))]);
            ctx.sessions_query.clone().set("boom".to_owned());
        });

        // A failure over a list you can still read is a toast, and the rows
        // stay put underneath it. `-32601` would be too: `session/list` is base
        // ACP, so a server that cannot answer it is broken rather than one with
        // a feature switched off.
        app.drive(|ctx| async move { refresh_sessions(&ctx, false).await });
        assert_eq!(
            app.toast().as_deref(),
            Some("Failed to list sessions: the session store is unavailable")
        );
        app.run(|ctx| {
            assert_eq!(
                ids(&ctx.sessions.peek()),
                ["chat_1"],
                "a failed refresh emptied the list the reader was looking at"
            );
            assert!(!*ctx.sessions_loading.peek());
        });

        // A page that answers after a newer fetch has claimed the list writes
        // nowhere: not the rows, not the cursor, and not the spinner — that one
        // belongs to the fetch which superseded it and is still running.
        app.run(|ctx| {
            ctx.sessions_query.clone().set(String::new());
            ctx.toast.clone().set(None);
            let ctx = *ctx;
            spawn_forever(async move { refresh_sessions(&ctx, false).await });
        });
        app.settle_until(Duration::from_secs(2), |ctx| *ctx.sessions_loading.peek());
        app.run(|ctx| ctx.sessions_epoch.clone().set(99));
        app.settle_until(Duration::from_millis(500), |_| false);
        app.run(|ctx| {
            assert_eq!(
                ids(&ctx.sessions.peek()),
                ["chat_1"],
                "a superseded page landed in the list under another search's rows"
            );
            assert_eq!(
                *ctx.sessions_next.peek(),
                None,
                "and re-armed \"Load more\" with a cursor minted under filters \
                 nobody is looking at any more"
            );
            assert!(
                *ctx.sessions_loading.peek(),
                "the superseded fetch took down a spinner belonging to the fetch \
                 that replaced it"
            );
        });

        // And the same for a failure that arrives late: the fetch that
        // superseded it will report its own, and two sentences about one list
        // is one more than there is news.
        app.run(|ctx| {
            ctx.sessions_query.clone().set("late".to_owned());
            ctx.sessions_epoch.clone().set(0);
            let ctx = *ctx;
            spawn_forever(async move { refresh_sessions(&ctx, false).await });
        });
        app.settle_until(Duration::from_secs(2), |ctx| {
            *ctx.sessions_epoch.peek() == 1
        });
        app.run(|ctx| ctx.sessions_epoch.clone().set(99));
        app.settle_until(Duration::from_millis(500), |_| false);
        assert_eq!(
            app.toast(),
            None,
            "a superseded failure was reported over a list it no longer says anything about"
        );
    }

    /// A rename the server agreed to has to reach both places the name is
    /// shown — the bar over the open chat and the row in the list — because a
    /// refetch of the list would take the reader's scroll position with it. And
    /// the title goes out trimmed: the sheet leaves whatever was typed.
    #[test]
    fn a_rename_the_server_agreed_to_reaches_both_the_row_and_the_bar() {
        fn picky(method: &str, params: &Value) -> Reply {
            if method == "_goose/unstable/session/rename"
                && params.get("title").and_then(Value::as_str) == Some("no")
            {
                return rpc_error(-32000, "titles are immutable here");
            }
            happy(method, params)
        }
        let app = App::mount();
        let server = serve(picky);
        let _events = app.attach(&server);
        app.run(|ctx| {
            ctx.sessions.clone().set(vec![
                session("s1", Some("quick question")),
                session("s2", None),
            ]);
        });
        app.open_chat("s1", "quick question");

        app.drive(
            |ctx| async move { rename_session(&ctx, "s1", "  Tailscale cert rotation  ").await },
        );
        assert_eq!(
            server.params("_goose/unstable/session/rename", 0),
            json!({"sessionId": "s1", "title": "Tailscale cert rotation"}),
            "the title went out with the whitespace the sheet left on it"
        );
        app.run(|ctx| {
            let titles: Vec<Option<String>> = ctx
                .sessions
                .peek()
                .iter()
                .map(|s| s.title.clone())
                .collect();
            assert_eq!(
                titles,
                vec![Some("Tailscale cert rotation".to_owned()), None],
                "the rename reached the wrong row, or every row"
            );
            assert_eq!(
                ctx.chat.peek().title,
                "Tailscale cert rotation",
                "the bar over the chat being renamed still shows the old name"
            );
        });

        // A rename the server refused changes nothing on screen.
        app.drive(|ctx| async move { rename_session(&ctx, "s1", "no").await });
        assert_eq!(
            app.toast().as_deref(),
            Some("Rename failed: titles are immutable here")
        );
        app.run(|ctx| {
            assert_eq!(
                ctx.sessions.peek()[0].title.as_deref(),
                Some("Tailscale cert rotation"),
                "the row shows a name the server refused"
            );
        });
    }

    // -------------------------------------------------- opening and creating

    /// Opening a chat replays its history and takes the agent's option set off
    /// the reply, which is the only place a `session/load` carries one. A
    /// replay the server cannot do has to end the spinner and say why, or the
    /// chat sits under "Loading…" for the rest of the session.
    #[test]
    fn opening_a_chat_replays_it_and_takes_the_agents_options_off_the_reply() {
        fn picky(method: &str, params: &Value) -> Reply {
            if method == "session/load"
                && params.get("sessionId").and_then(Value::as_str) == Some("gone")
            {
                return rpc_error(-32602, "no such session");
            }
            happy(method, params)
        }
        let mut app = App::mount();
        let server = serve(picky);
        let _events = app.attach(&server);

        let info: SessionInfo = serde_json::from_value(
            json!({"sessionId": "s2", "title": "Nightly standup", "cwd": "/srv/work"}),
        )
        .unwrap();
        app.run(|ctx| open_session(ctx, info));
        app.settle_until(Duration::from_secs(5), |ctx| !ctx.chat.peek().loading);
        assert_eq!(
            server.params("session/load", 0),
            json!({"sessionId": "s2", "cwd": "/srv/work", "mcpServers": []}),
            "the replay went out against the wrong directory"
        );
        assert_eq!(app.toast(), None, "a replay that worked reported itself");
        app.run(|ctx| {
            assert_eq!(
                ctx.config_options.peek()[0].config_id,
                "model",
                "the picker is empty over a session whose load listed its models"
            );
        });

        let missing: SessionInfo =
            serde_json::from_value(json!({"sessionId": "gone", "cwd": "/srv/work"})).unwrap();
        app.run(|ctx| open_session(ctx, missing));
        app.settle_until(Duration::from_secs(5), |ctx| ctx.toast.peek().is_some());
        assert_eq!(
            app.toast().as_deref(),
            Some("Failed to load session: no such session")
        );
        app.run(|ctx| {
            assert!(
                !ctx.chat.peek().loading,
                "the chat is stuck on a replay that will never arrive"
            );
        });
    }

    /// `_meta` is how goose is told *why* a session exists — a recipe launch is
    /// a session with `_meta.recipeId` on it — and it is the server that acts
    /// on it, so the app cannot fake it afterwards by prompting the session
    /// into character. Everything belonging to the last conversation goes with
    /// the new one opening.
    #[test]
    fn a_new_chat_carries_its_reason_to_the_server_and_opens_what_comes_back() {
        fn picky(method: &str, params: &Value) -> Reply {
            if method == "session/new"
                && params.pointer("/_meta/recipeId").and_then(Value::as_str) == Some("boom")
            {
                return rpc_error(-32000, "the working directory does not exist");
            }
            happy(method, params)
        }
        let mut app = App::mount();
        let server = serve(picky);
        let _events = app.attach(&server);
        app.run(|ctx| {
            ctx.settings.clone().set(Settings {
                working_dir: "  /srv/work  ".to_owned(),
                ..Settings::default()
            });
        });

        // The refusal first, so that "the screen stayed put" means something.
        app.run(|ctx| new_session_with(ctx, json!({"recipeId": "boom"})));
        app.settle_until(Duration::from_secs(5), |ctx| ctx.toast.peek().is_some());
        assert_eq!(
            app.toast().as_deref(),
            Some("Failed to create session: the working directory does not exist")
        );
        app.run(|ctx| {
            assert!(
                matches!(*ctx.screen.peek(), Screen::Settings),
                "a chat that was never created was navigated into"
            );
            assert!(ctx.chat.peek().session_id.is_none());
        });

        app.run(|ctx| {
            ctx.usage.clone().set(Some((10, 100)));
            ctx.attachments.clone().set(vec![pending("roof.jpg")]);
            new_session_with(ctx, json!({"recipeId": "standup"}));
        });
        app.settle_until(Duration::from_secs(5), |ctx| {
            matches!(*ctx.screen.peek(), Screen::Chat)
        });
        assert_eq!(
            server.params("session/new", 1),
            json!({"cwd": "/srv/work", "mcpServers": [],
                   "_meta": {"client": "goose-mobile", "recipeId": "standup"}}),
            "the reason the session exists did not travel with the request that created it"
        );
        app.run(|ctx| {
            let chat = ctx.chat.peek();
            assert_eq!(chat.session_id.as_deref(), Some("s_new"));
            assert_eq!(
                chat.cwd, "/srv/work",
                "the chat runs in the untrimmed directory the box held"
            );
            assert_eq!(chat.title, "New chat");
            assert!(!chat.loading && !chat.running);
            assert_eq!(
                ctx.config_options.peek()[0].config_id,
                "mode",
                "the options `session/new` answered with were thrown away"
            );
            assert!(
                ctx.attachments.peek().is_empty(),
                "a photo picked for the last chat is sitting in the new one's tray"
            );
            assert_eq!(
                *ctx.usage.peek(),
                None,
                "the last chat's context meter is still on screen"
            );
        });
    }

    // ---------------------------------------------------------- sending, and
    // what a turn leaves behind

    /// What the reader is told when a turn ends. An ordinary ending is silence
    /// — the transcript is the answer — and every other ending is a sentence,
    /// because a turn that stopped for a reason nobody named looks exactly like
    /// one that finished.
    #[test]
    fn how_a_turn_ended_is_reported_only_when_it_was_not_ordinary() {
        let mut app = App::mount();
        let server = serve(happy);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");

        // An empty composer sends nothing. goose answers `invalid_params` to a
        // turn with no content in it, and the transport refuses it one step
        // earlier, so a message drawn into the transcript here would sit there
        // permanently over a turn that was never started.
        app.run(|ctx| {
            assert!(
                !send_prompt(ctx, String::new(), &[]),
                "an empty message was handed to the transport"
            );
            assert!(ctx.chat.peek().items.is_empty());
        });

        for (stop, said) in [
            ("end_turn", None),
            ("cancelled", None),
            ("max_tokens", Some("Stopped: max tokens reached")),
            ("refusal", Some("The agent declined to continue")),
            ("recursion_limit", Some("Turn ended: recursion_limit")),
        ] {
            app.run(|ctx| {
                ctx.toast.clone().set(None);
                assert!(
                    send_prompt(ctx, stop.to_owned(), &[]),
                    "the send never reached the transport"
                );
                assert!(
                    ctx.chat.peek().running,
                    "the composer never showed the turn as running"
                );
                assert!(ctx.running_sessions.peek().contains("s1"));
            });
            app.settle_until(Duration::from_secs(5), |ctx| !ctx.chat.peek().running);
            assert_eq!(
                app.toast().as_deref(),
                said,
                "a turn that ended `{stop}` was reported wrongly"
            );
            app.run(|ctx| {
                assert!(
                    ctx.running_sessions.peek().is_empty(),
                    "the chats list still shows a turn in flight for `{stop}`"
                );
            });
        }
        assert_eq!(server.count("session/prompt"), 5);
    }

    /// A send that died on the wire. The message did reach the transcript, so
    /// it stays — it may well have arrived, and a bubble the reader can see and
    /// re-send beats a photo that is simply gone — and the files go back into
    /// the tray they were picked into, which is the only place they can come
    /// back to.
    #[test]
    fn a_send_that_died_on_the_wire_puts_the_photos_back_in_the_composer() {
        fn broken(method: &str, params: &Value) -> Reply {
            if method == "session/prompt" {
                return rpc_error(-32000, "provider is unreachable");
            }
            happy(method, params)
        }
        let mut app = App::mount();
        let server = serve(broken);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");

        app.run(|ctx| {
            assert!(send_prompt(
                ctx,
                "look at this".to_owned(),
                &[pending("roof.jpg")]
            ));
        });
        app.settle_until(Duration::from_secs(5), |ctx| ctx.toast.peek().is_some());
        assert_eq!(
            app.toast().as_deref(),
            Some(
                "Prompt failed: provider is unreachable — its attachments are back in the composer"
            ),
            "the toast has to say what went wrong AND where the files went"
        );
        app.run(|ctx| {
            let back: Vec<String> = ctx
                .attachments
                .peek()
                .iter()
                .map(|f| f.record.name.clone())
                .collect();
            assert_eq!(
                back,
                ["roof.jpg"],
                "a send that never landed ate the photo it was carrying"
            );
            assert!(
                !ctx.chat.peek().running,
                "the turn that failed is still spinning"
            );
        });
        assert_eq!(
            app.items(),
            ["user:look at this:[roof.jpg]"],
            "the message was taken back out of the transcript"
        );
        assert_eq!(server.count("session/prompt"), 1);
    }

    /// The turn is over and the socket is still there, so every ask this
    /// session left open is answered "cancelled" — on the wire, not merely off
    /// the screen — and the journal stops reporting them, because they got a
    /// reply. Another chat's ask is another turn's, and is left alone.
    #[test]
    fn a_turn_that_ended_cleanly_answers_the_asks_it_left_behind() {
        let mut app = App::mount();
        let server = serve(happy);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                ask(2, "s1", json!({"toolCallId": "t2", "title": "write"})),
                ask(
                    3,
                    "s2",
                    json!({"toolCallId": "t3", "title": "another chat"}),
                ),
            ],
        );

        app.run(|ctx| assert!(send_prompt(ctx, "end_turn".to_owned(), &[])));
        app.settle_until(Duration::from_secs(5), |ctx| {
            ctx.permission.peek().len() == 1
        });
        app.settle();
        assert_eq!(
            server.answers(),
            vec![
                json!({"outcome": {"outcome": "cancelled"}}),
                json!({"outcome": {"outcome": "cancelled"}})
            ],
            "the asks the turn left open went off the screen without an answer \
             reaching goose, so the agent is still parked on them"
        );
        app.run(|ctx| {
            let queued: Vec<String> = ctx
                .permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect();
            assert_eq!(
                queued,
                ["t3"],
                "another chat's ask was answered with this turn's"
            );
        });
        let open: Vec<String> = app
            .journal()
            .into_iter()
            .filter(crate::ask_journal::AskRecord::is_open)
            .map(|r| r.tool_call_id)
            .collect();
        assert_eq!(
            open,
            ["t3"],
            "an ask that got a reply is still being reported as pending"
        );
    }

    /// The other side of that fork. `respond_permission` over a dead transport
    /// pushes a command into a channel whose receiver is gone: it looks like an
    /// answer and is not one. So the queue is dropped instead — this session's
    /// only — and the verdict is deliberately left to the pump, which is the
    /// one place that can tell a dropped tailnet from a press of Disconnect.
    #[test]
    fn a_turn_that_lost_its_socket_drops_its_asks_without_pretending_to_answer() {
        fn dies(method: &str, params: &Value) -> Reply {
            if method == "session/prompt" {
                return hang_up();
            }
            happy(method, params)
        }
        let mut app = App::mount();
        let server = serve(dies);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                ask(2, "s2", json!({"toolCallId": "t2", "title": "write"})),
            ],
        );

        app.run(|ctx| assert!(send_prompt(ctx, "go".to_owned(), &[])));
        app.settle_until(Duration::from_secs(5), |ctx| ctx.toast.peek().is_some());
        assert_eq!(
            app.toast().as_deref(),
            Some("Prompt failed: connection closed")
        );
        app.run(|ctx| {
            let queued: Vec<String> = ctx
                .permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect();
            assert_eq!(
                queued,
                ["t2"],
                "another chat's ask was dropped along with this one's"
            );
        });
        assert!(
            app.journal()
                .iter()
                .all(crate::ask_journal::AskRecord::is_open),
            "the abandon path called a round lost, which is the pump's decision \
             and not its own"
        );
        assert_eq!(server.count("session/prompt"), 1);
    }

    /// Stop answers the open asks BEFORE it cancels. The frames travel in
    /// order, so the parked run unparks and then observes the cancel; the other
    /// way round the cancel lands on a run that is still waiting to be told
    /// what to do, and the turn never ends at all.
    #[test]
    fn stopping_a_turn_answers_the_open_asks_before_it_cancels() {
        let mut app = App::mount();
        let server = serve(happy);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                ask(
                    2,
                    "s2",
                    json!({"toolCallId": "t2", "title": "another chat"}),
                ),
            ],
        );

        app.run(stop_turn);
        app.settle();
        assert_eq!(
            server.traffic(),
            ["answer", "session/cancel"],
            "the cancel went out first, so it reached a run still parked on an ask"
        );
        assert_eq!(
            server.params("session/cancel", 0),
            json!({"sessionId": "s1"})
        );
        app.run(|ctx| {
            let queued: Vec<String> = ctx
                .permission
                .peek()
                .iter()
                .map(|p| p.tool_call.tool_call_id.clone())
                .collect();
            assert_eq!(queued, ["t2"], "Stop swallowed another chat's ask");
        });
    }

    /// Allow and Deny have to reach goose, not just the screen. The modal
    /// coming down is the whole of the feedback, so an answer that never left
    /// the phone looks exactly like one that worked — and the agent stays
    /// parked forever.
    #[test]
    fn answering_an_ask_reaches_goose_and_not_just_the_screen() {
        let mut app = App::mount();
        let server = serve(happy);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                ask(1, "s1", json!({"toolCallId": "t1", "title": "shell"})),
                ask(2, "s1", json!({"toolCallId": "t2", "title": "write"})),
            ],
        );

        app.run(|ctx| answer_permission(ctx, &json!(1), Some("allow_once".to_owned())));
        app.settle();
        assert_eq!(
            server.answers(),
            vec![json!({"outcome": {"outcome": "selected", "optionId": "allow_once"}})],
            "Allow took the modal off the screen without telling goose"
        );
    }

    /// The mode and model chips. The agent applies the change and answers with
    /// the full option set, so what comes back is what the picker shows — but
    /// an answer with nothing in it is goose saying nothing, not goose saying
    /// "no options", and emptying the sheet on it would leave a control with no
    /// values mid-conversation.
    #[test]
    fn switching_a_session_option_takes_the_answer_the_agent_gives_back() {
        fn picky(method: &str, params: &Value) -> Reply {
            if method == "session/set_config_option" {
                return match params.get("value").and_then(Value::as_str) {
                    Some("boom") => rpc_error(-32602, "no such mode"),
                    Some("quiet") => ok(json!({})),
                    _ => happy(method, params),
                };
            }
            happy(method, params)
        }
        let mut app = App::mount();
        let server = serve(picky);
        let _events = app.attach(&server);
        app.open_chat("s1", "Deploy");
        app.run(|ctx| {
            ctx.config_options
                .clone()
                .set(serde_json::from_value(json!([wire_option("mode", "auto")])).unwrap());
        });

        app.run(|ctx| set_config_option(ctx, "mode", "chat"));
        app.settle();
        assert_eq!(
            server.params("session/set_config_option", 0),
            json!({"sessionId": "s1", "configId": "mode", "type": "id", "value": "chat"}),
            "goose routes the option by id and takes the value flattened beside it"
        );
        app.run(|ctx| {
            assert_eq!(
                ctx.config_options.peek()[0].current_value.as_deref(),
                Some("chat"),
                "the picker still shows the value the tap replaced"
            );
        });

        app.run(|ctx| set_config_option(ctx, "mode", "quiet"));
        app.settle();
        app.run(|ctx| {
            assert_eq!(
                ctx.config_options.peek().len(),
                1,
                "an answer with no options in it emptied the picker"
            );
        });

        app.run(|ctx| set_config_option(ctx, "mode", "boom"));
        app.settle_until(Duration::from_secs(5), |ctx| ctx.toast.peek().is_some());
        assert_eq!(
            app.toast().as_deref(),
            Some("Could not switch: no such mode")
        );
        app.run(|ctx| {
            assert_eq!(
                ctx.config_options.peek()[0].current_value.as_deref(),
                Some("chat"),
                "a switch the agent refused was drawn as though it had taken"
            );
        });

        // The sheet can be reached with no chat open — the option set outlives
        // the conversation it arrived with — and there is then no session for
        // the agent to apply anything to.
        let before = server.count("session/set_config_option");
        app.run(|ctx| {
            ctx.chat.clone().set(ChatState::default());
            set_config_option(ctx, "mode", "chat");
        });
        app.settle();
        assert_eq!(
            server.count("session/set_config_option"),
            before,
            "a config change went out naming no session at all"
        );
    }

    // ------------------------------------------------------- the leftover bits

    /// A toast takes itself off the screen, and the timer that does it belongs
    /// to the toast that armed it. Without the sequence check, the older
    /// toast's timer wipes whatever is on screen when it fires — so a failure
    /// reported three seconds after another one is shown for one second and
    /// then silently withdrawn.
    ///
    /// Real seconds, because `tokio`'s clock cannot be paused without the
    /// `test-util` feature and this crate does not carry it.
    #[test]
    fn a_toast_clears_itself_and_the_older_ones_timer_does_not_clear_it() {
        let mut app = App::mount();
        app.run(|ctx| show_toast(ctx, "Failed to list sessions"));
        app.settle_until(Duration::from_secs(3), |_| false);
        assert_eq!(
            app.toast().as_deref(),
            Some("Failed to list sessions"),
            "the toast went away well before its four seconds"
        );

        // Armed a second before the first one's timer fires, and still on
        // screen a second after it.
        app.run(|ctx| show_toast(ctx, "Rename failed: timed out"));
        app.settle_until(Duration::from_secs(2), |_| false);
        assert_eq!(
            app.toast().as_deref(),
            Some("Rename failed: timed out"),
            "the previous toast's timer cleared the one that replaced it"
        );

        // And it does eventually go, which is the other half of the rule.
        app.settle_until(Duration::from_secs(10), |ctx| ctx.toast.peek().is_none());
        assert_eq!(app.toast(), None, "the toast never cleared itself");
    }

    /// A generated title has to reach the bar over the open chat even when the
    /// list behind it holds no row for that session — walking into a chat from
    /// a recipe or a scheduled run does exactly that, and a title that only
    /// ever arrived through the list would leave the bar reading "New chat"
    /// for the rest of the conversation.
    #[test]
    fn a_generated_title_reaches_the_bar_with_no_row_to_put_it_in() {
        let app = App::mount();
        app.open_chat("s1", "New chat");
        pump_events(
            &app,
            vec![notify(
                "s1",
                json!({"sessionUpdate": "session_info_update", "title": "Tailscale cert rotation"}),
            )],
        );
        app.run(|ctx| {
            assert_eq!(ctx.chat.peek().title, "Tailscale cert rotation");
            assert!(
                ctx.sessions.peek().is_empty(),
                "a row was invented in a list that has never been fetched"
            );
        });
    }

    /// goose's older shape sends a chunk with no message id at all, and it
    /// belongs to whatever is being written. The reply arm already relies on
    /// that; the reasoning arm has to agree, or a stream of untagged thought
    /// chunks becomes one folded card per token.
    #[test]
    fn an_untagged_thought_chunk_continues_the_one_being_written() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "agent_thought_chunk", "messageId": "m1",
                           "content": {"type": "text", "text": "check the logs"}}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "agent_thought_chunk",
                           "content": {"type": "text", "text": " first"}}),
                ),
            ],
        );
        assert_eq!(app.items(), ["thought:m1:check the logs first"]);
    }

    /// An update that carries no output must leave the row's output alone.
    /// goose sends one of these the moment a call starts running, and again
    /// when it finishes with a raw result beside output that is already there —
    /// a row that took either as "here is the output" would blank what the call
    /// had printed.
    #[test]
    fn a_tool_update_carrying_no_output_leaves_the_row_as_it_was() {
        let app = App::mount();
        app.open_chat("s1", "Deploy");
        pump_events(
            &app,
            vec![
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call", "toolCallId": "a", "title": "read"}),
                ),
                // Nothing but a status, over a row with no output yet.
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
                           "status": "in_progress"}),
                ),
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
                           "content": [{"type": "content",
                                        "content": {"type": "text", "text": "Cargo.toml"}}]}),
                ),
                // And a raw result over a row that already has output.
                notify(
                    "s1",
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
                           "status": "completed", "rawOutput": {"lines": 3}}),
                ),
            ],
        );
        assert_eq!(app.items(), ["tool:a:read:other:completed:Cargo.toml"]);
    }

    /// A timestamp with a negative year. Nothing sends one, but a four-digit
    /// slice with a minus in it parses as one, and both halves of this file's
    /// date arithmetic have a branch that only a negative era reaches —
    /// Hinnant's `days_from_civil` and, in `relative_time`, its inverse. They
    /// are copied whole for exactly that reason, and a badge is what has to
    /// come out either way.
    #[test]
    fn a_timestamp_from_before_the_year_zero_still_produces_a_badge() {
        let epoch = rfc3339_to_epoch("-100-05-20T00:00:00Z")
            .expect("four characters, a minus among them, still parse as a year");
        assert!(epoch < 0, "the fixture is not before the epoch at all");
        assert_eq!(
            relative_time(epoch),
            "May 20",
            "the two halves of the date arithmetic disagree on the far side of \
             the era split"
        );
    }

    /// And the shapes that are not timestamps at all. Every list row's badge
    /// goes through this, so a `None` that turned into a `0` would date the row
    /// to January 1970 instead of saying nothing.
    #[test]
    fn a_string_that_is_not_a_timestamp_is_not_a_date() {
        assert_eq!(rfc3339_to_epoch(""), None);
        assert_eq!(rfc3339_to_epoch("nonsense"), None);
        assert_eq!(rfc3339_to_epoch("2026-08"), None, "a date with no day");
        assert_eq!(rfc3339_to_epoch("2026-XX-29T00:00:00Z"), None);
        // The time is the optional half: goose's schedule rows carry bare
        // dates, and midnight is the honest reading of one.
        assert_eq!(
            rfc3339_to_epoch("2026-08-29"),
            rfc3339_to_epoch("2026-08-29T00:00:00Z"),
            "a bare date stopped being a date"
        );
    }
}
