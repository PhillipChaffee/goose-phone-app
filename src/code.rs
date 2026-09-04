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

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use opencode_client::{
    ChatMeta, Checks, CodeClient, CodeConfig, CodeEvent, CodePermission, FileDiff,
    MessageWithParts, Part, PermissionReport, PullRequest, PullState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::{DiffLine, Gap};
use crate::state::{show_toast, AppCtx, ChatItem, ConnState, Tab};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeScreen {
    List,
    New,
    Chat,
    /// Reviewing the session's changes. Its own screen rather than a panel in
    /// the transcript: the thing being reviewed is a whole working tree, and
    /// a review has its own navigation, its own chrome and its own state.
    Diff,
    /// The pull requests this chat's branch has produced.
    Pulls,
}

/// The conversation key the new-session composer's attachments belong to.
///
/// `conversation_key` normally answers with the open chat's id, and on this
/// screen there is no open chat — the last one you visited is still in
/// `code_chat`, and its id would make a photo picked here land in it. It also
/// selects the tray those picks sit in (`crate::attach::tray_of`), which is
/// the half that keeps the two composers' held files apart rather than only
/// their arriving ones.
///
/// It cannot collide with a real chat: the manager names one
/// `{repo}-{6 hex}`, so every id it issues carries a suffix.
pub(crate) const NEW_CONVERSATION: &str = "new";

/// A repo's branches, for the new-session screen's base-branch pill.
///
/// It carries the repo it was fetched for. A read takes a round trip and the
/// repo pill is one tap away, so an answer that arrives after the reader has
/// moved on is for a different repo — and showing it would offer branches
/// that do not exist on the one now selected.
#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) struct BranchList {
    pub repo: String,
    /// The repo's own default, marked `Default` in the picker and what the
    /// pill seeds itself from. `None` when GitHub would not say, which the
    /// manager reports rather than failing the whole list over.
    pub default: Option<String>,
    pub names: Vec<String>,
    /// The manager stopped paging before the end (500 branches). Carried so
    /// the picker can say the list is short rather than let it look complete —
    /// with a filter over it, "Nothing matches" about a branch that exists is
    /// the wrong answer to give silently.
    pub truncated: bool,
    pub loading: bool,
}

/// Everything the code chat screen renders.
#[derive(Clone, PartialEq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these five are independent facts about the screen, not a state \
              machine to collapse into an enum: a chat can be waking AND \
              loading at once, running is orthogonal to both, and picked and \
              agent_picked are about where the model and the mode came from \
              rather than about the chat's lifecycle at all. Neither of the \
              two flags can be folded into its own Option either — see picked \
              below for why doing so makes the server's record permanently \
              unadoptable"
)]
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
    /// What the next turn will run on, as `provider/model`. Seeded from the
    /// chat's own record, replaced by the server's session record once the
    /// container is awake, and by the settings sheet when the user picks.
    pub model: Option<String>,
    /// Thinking-effort tier for the next turn (`OpenCode` calls it a
    /// variant). `None` means the model's own default.
    pub effort: Option<String>,
    /// The agent the next turn runs as — what the composer calls the mode.
    ///
    /// Either the reader's pick or the agent the session record reports;
    /// `agent_picked` beside it says which. `None` means neither has spoken
    /// yet, and the composer resolves one out of the server's own agent list
    /// rather than showing the name of the control
    /// ([`opencode_client::resolve_agent`]).
    ///
    /// This used to say a flag would be owed "the day `GET /session` starts
    /// reporting the agent". That day is here: `Session.to_wire()` emits
    /// `agent` beside `model`, written on every turn that names one.
    pub agent: Option<String>,
    /// The reader picked `agent` in the mode sheet, as opposed to it being
    /// adopted from the session record.
    ///
    /// Exactly `picked`'s job for the mode. `attach_chat` also runs on an SSE
    /// reconnect, so without this a mode chosen seconds ago and not yet sent
    /// would be overwritten by whatever the last turn ran as.
    pub agent_picked: bool,
    /// The reader picked `model`/`effort` in the settings sheet, as opposed
    /// to them being seeded from the chat record.
    ///
    /// This cannot be recovered from `Option`: a chat created with a named
    /// model arrives with `model` already `Some`, so "has a value" and "the
    /// reader chose it" are different questions. Answering the second with
    /// the first makes the server's session record permanently unadoptable —
    /// the sheet then shows the create-time model forever, and the next turn
    /// writes it back over whatever the reader actually chose.
    pub picked: bool,
}

/// The review screen's state for the open chat.
///
/// A signal of its own rather than a field on `CodeChatState`, because the
/// chat screen re-renders on every keystroke in the composer and reads its
/// state by cloning it. A whole-file patch parsed into lines is by far the
/// largest thing this tab holds, and the transcript has no use for it.
#[derive(Clone, PartialEq, Default)]
pub(crate) struct DiffState {
    pub files: Vec<DiffFile>,
    pub loading: bool,
    pub error: Option<String>,
    /// Per-file review state, keyed by path.
    pub view: HashMap<String, FileView>,
}

/// One file, parsed once at fetch time rather than on every render — the
/// patch carries the whole file, so this is the expensive part.
#[derive(Clone, PartialEq)]
pub(crate) struct DiffFile {
    pub info: FileDiff,
    /// Fingerprint of `info.patch`; what "reviewed" is pinned to.
    pub fingerprint: u64,
    pub lines: Vec<DiffLine>,
    pub gaps: Vec<Gap>,
}

impl From<FileDiff> for DiffFile {
    fn from(info: FileDiff) -> Self {
        let lines = crate::diff::parse(&info.patch);
        Self {
            fingerprint: crate::diff::fingerprint(&info.patch),
            gaps: crate::diff::gaps(&lines),
            lines,
            info,
        }
    }
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct FileView {
    /// Card expanded. `None` means "follow `seen`" — marking a file read
    /// folds it away, which is what stops a long diff making you scroll past
    /// work you have finished with, while an explicit value keeps the two
    /// decoupled so a read file can be reopened without unmarking it.
    pub open: Option<bool>,
    /// The patch fingerprint that was marked reviewed, if any.
    pub seen: Option<u64>,
    /// Gap `start` -> lines revealed out of it.
    pub expanded: HashMap<usize, usize>,
    /// A deleted file's removed lines have been asked for.
    pub show_removed: bool,
}

impl DiffState {
    pub(crate) fn is_seen(&self, file: &DiffFile) -> bool {
        self.view.get(&file.info.file).and_then(|v| v.seen) == Some(file.fingerprint)
    }

    pub(crate) fn is_open(&self, file: &DiffFile) -> bool {
        self.view
            .get(&file.info.file)
            .and_then(|v| v.open)
            .unwrap_or_else(|| !self.is_seen(file))
    }

    pub(crate) fn reviewed(&self) -> usize {
        self.files.iter().filter(|f| self.is_seen(f)).count()
    }

    /// (added, removed) across every file in the diff.
    pub(crate) fn totals(&self) -> (u32, u32) {
        self.files.iter().fold((0, 0), |(a, d), f| {
            (
                a.saturating_add(f.info.additions),
                d.saturating_add(f.info.deletions),
            )
        })
    }

    /// path -> the fingerprint marked reviewed, for persistence.
    fn marks(&self) -> HashMap<String, u64> {
        self.view
            .iter()
            .filter_map(|(path, v)| v.seen.map(|hash| (path.clone(), hash)))
            .collect()
    }
}

/// Rebuild per-file review state from marks alone.
///
/// Everything in `FileView` except `seen` is positional — an expanded gap is
/// an index into the patch that was parsed — so a fresh payload has to start
/// from the marks and nothing else. `seen` survives because it carries the
/// fingerprint it was taken against and can check itself.
fn marks_to_view(marks: &HashMap<String, u64>) -> HashMap<String, FileView> {
    marks
        .iter()
        .map(|(path, hash)| {
            (
                path.clone(),
                FileView {
                    seen: Some(*hash),
                    ..FileView::default()
                },
            )
        })
        .collect()
}

/// The pull-request screen's state for the open chat, plus the plane-wide
/// index every row in the list reads (issue #84).
///
/// A signal of its own, for the same reason `DiffState` is one: the chat
/// screen re-renders on every keystroke and reads its state by cloning it,
/// and this list belongs to a different plane entirely — the manager's
/// GitHub calls, not the chat's container.
///
/// Two halves with two lifetimes, and they must not be cleared together:
/// `pulls`/`loading`/`loaded`/`error`/`merging` belong to whichever chat is
/// open and are dropped when a different one is, while `by_chat` and `swept`
/// belong to the plane and survive every open. `open_code_chat` says so where
/// it does the reset.
#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) struct PullsState {
    /// Only the pull requests off this chat's branch; see `CodeClient::pulls`.
    pub pulls: Vec<PullRequest>,
    pub loading: bool,
    /// An answer has landed for this chat. What tells "none" apart from "not
    /// asked yet" — without it the chip would print a count it cannot back,
    /// and `0` is a claim like any other.
    pub loaded: bool,
    pub error: Option<String>,
    /// The number whose merge is in flight.
    pub merging: Option<u64>,
    /// chat id -> that chat's pull requests, for every chat
    /// [`refresh_plane_pulls`] has reached. A chat that is absent has not been
    /// asked about; a chat mapped to an empty list has, and has none. The two
    /// must stay distinguishable for the same reason `loaded` exists.
    pub by_chat: HashMap<String, Vec<PullRequest>>,
    /// Unix seconds at which the last sweep started. The floor that keeps the
    /// ten-second poll off GitHub's hourly budget — see [`SWEEP_FLOOR_SECS`].
    pub swept: u64,
}

impl PullsState {
    /// The pull request a row should speak for, or `None` when the sweep has
    /// not reached this chat or the chat's branch has none.
    ///
    /// The newest, which is the manager's first: `chat_pulls` asks GitHub with
    /// `sort=created&direction=desc`. A branch with two pull requests is a
    /// branch someone reopened, and the row is about what is happening now.
    pub(crate) fn plane_pull(&self, chat_id: &str) -> Option<&PullRequest> {
        self.by_chat.get(chat_id)?.first()
    }
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
    /// path -> the patch fingerprint that was marked reviewed. Persisted
    /// because a review is a task you leave and come back to, and the app
    /// being backgrounded while a container wakes is the normal case here.
    pub diff_seen: HashMap<String, u64>,
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
            // Before the first poll tick, not ten seconds after it: an ask
            // raised while the app was closed is the case this whole path
            // exists for, and it is already waiting when the list first paints.
            refresh_code_permissions(ctx).await;
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
    // Kicked, not awaited: the sweep is up to twenty-four round trips to
    // GitHub and nothing on the screen should hold a spinner for them. It is
    // called from here rather than from the poll so that the pull gesture and
    // ⌘R drive it too — `viewport::refresh_named`'s "code" arm is this
    // function — and its own floor decides whether it does anything.
    refresh_plane_pulls(ctx);
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

/// Fold the manager's aggregate of pending asks into the queue, so a chat you
/// have not opened can say that it is blocked waiting on you.
///
/// One request to the manager, never one per chat: `/chat/<id>/…` goes
/// through the transparent proxy and wakes a stopped container, so walking
/// the list would keep every container alive and defeat the idle spin-down.
/// The manager asks only the containers that are already running, which is
/// not a compromise — a container that is down has no live turn to park.
pub(crate) async fn refresh_code_permissions(ctx: &AppCtx) {
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    // Silent on failure, deliberately: this runs on a timer, and a manager
    // too old to have the route would otherwise raise a toast every ten
    // seconds about a feature the reader cannot act on. The queue keeps
    // whatever it had — which is also why a body the client cannot parse is
    // an error rather than an empty report: read as one, it would mean
    // "nothing is waiting on you anywhere" and clear every card on the list.
    let Ok(report) = client.pending_permissions().await else {
        return;
    };
    // The chat with a live event stream, which is not the same as the chat
    // that was opened last — see `stream_events`.
    let streaming = ctx.code_stream.peek().clone();
    let mut queue = ctx.code_permissions;
    let mut answered = ctx.code_answered;
    merge_permission_report(
        &mut queue.write(),
        &mut answered.write(),
        &report,
        streaming.as_deref(),
    );
}

/// Reconcile a snapshot of what is pending server-side with what is queued.
///
/// The snapshot is authoritative for every chat it can speak for, which is
/// every chat except two. The chat with a **live event stream** is left to
/// it: that stream is both faster and ordered, and a snapshot taken before an
/// ask arrived would otherwise blink it out of the modal for a poll interval.
/// *Live* is the load-bearing word. This used to read "the chat that is
/// open", taken from `code_chat`, which is never cleared — so the last chat
/// you visited was excluded for the rest of the session, including after its
/// stream had died and left nothing else to speak for it. The chat most
/// likely to be blocked was the one chat the aggregate was forbidden to
/// report. A chat the manager lists as **unreachable** is one whose container
/// did not answer, which is not the same as a container with nothing pending
/// — dropping its ask on that would be inventing an answer.
fn merge_permission_report(
    queue: &mut Vec<(String, CodePermission)>,
    answered: &mut HashSet<(String, String)>,
    report: &PermissionReport,
    streaming: Option<&str>,
) {
    let speaks_for =
        |chat: &str| streaming != Some(chat) && !report.unreachable.iter().any(|c| c == chat);
    let listed = |chat: &str, id: &str| {
        report
            .permissions
            .iter()
            .any(|a| a.chat_id == chat && a.permission.id == id)
    };

    queue.retain(|(chat, p)| !speaks_for(chat) || listed(chat, &p.id));
    // A tombstone outlives its usefulness the moment the server stops
    // reporting the ask it was hiding; keeping it would be a slow leak, and
    // an id reused by a rebuilt container would be silently swallowed.
    answered.retain(|(chat, id)| !speaks_for(chat) || listed(chat, id));

    for ask in &report.permissions {
        let (chat, perm) = (&ask.chat_id, &ask.permission);
        // An ask with no chat cannot be answered — replying needs the chat in
        // the path — and one with no id cannot be told apart from the next.
        if chat.is_empty() || perm.id.is_empty() || streaming == Some(chat.as_str()) {
            continue;
        }
        if answered.contains(&(chat.clone(), perm.id.clone())) {
            continue;
        }
        if queue.iter().any(|(c, p)| c == chat && p.id == perm.id) {
            continue;
        }
        queue.push((chat.clone(), perm.clone()));
    }
}

/// Keep the chat list fresh while the Code tab is visible. One loop at a
/// time; a tab switch away lets it park (cheap no-op ticks).
///
/// The pending-ask aggregate rides the same tick rather than getting a timer
/// of its own: it is the other half of "what is this list doing right now",
/// and two loops on the same cadence would only mean two ways for the list to
/// be internally inconsistent.
pub(crate) fn start_code_poll(ctx: &AppCtx) {
    let mut generation = ctx.code_poll;
    let mine = generation.peek().wrapping_add(1);
    generation.set(mine);
    let ctx = *ctx;
    spawn_forever(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            match poll_tick(
                mine,
                *ctx.code_poll.peek(),
                *ctx.tab.peek(),
                ctx.code_client.peek().is_some(),
            ) {
                Tick::Retire => return,
                Tick::Idle => continue,
                Tick::Fetch => {}
            }
            refresh_code_chats(&ctx).await;
            refresh_code_permissions(&ctx).await;
        }
    });
}

/// What a poll tick does when it wakes up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tick {
    /// A newer loop has taken this one's place.
    Retire,
    /// Nothing to ask for from here — sleep again.
    Idle,
    /// Refresh the rows and the asks.
    Fetch,
}

/// A poll loop retires for a newer poll loop, and for nothing else.
///
/// Spelled out as a function so the one input it must not have is visibly
/// absent from the signature. It used to read `code_epoch`, which counts chat
/// *opens* — so the first tap on any row retired the loop, and neither caller
/// of `start_code_poll` can fire twice (both are gated on a connection that
/// is made once). From that tap on, the aggregate was fetched only by a pull
/// or a restart, and a chat that blocked on a permission afterwards was back
/// to looking exactly like a chat with nothing to do.
const fn poll_tick(mine: u64, current: u64, tab: Tab, connected: bool) -> Tick {
    if current != mine {
        Tick::Retire
    } else if !matches!(tab, Tab::Code) || !connected {
        Tick::Idle
    } else {
        Tick::Fetch
    }
}

// ------------------------------------------------------- session settings

/// Zen's free models train on their input (personal-ai-setup
/// `docs/privacy.md`, hard rule 1). The manager refuses them when a chat is
/// *created* against a repo that is not `public_throwaway` — but a per-turn
/// model rides through its transparent `/chat/<id>/…` proxy with no such
/// check, so the picker must not offer them either. Same rule the manager
/// applies, "free" substring net included.
pub(crate) fn is_free_model(reference: &str) -> bool {
    let bare = reference
        .rsplit('/')
        .next()
        .unwrap_or(reference)
        .to_ascii_lowercase();
    matches!(bare.as_str(), "big-pickle" | "muse-spark-contributor") || bare.contains("free")
}

/// Whether `repo` is flagged a public throwaway — the one case where a model
/// that trains on its input is allowed to see the code.
///
/// Asked about a repo name rather than about the open chat, because the
/// new-session screen has no chat: it applies this rule to whichever repo is
/// selected on it, which is the same rule the settings sheet applies to the
/// one a chat is on.
pub(crate) fn repo_allows_free_models(ctx: &AppCtx, repo: &str) -> bool {
    ctx.code_repos
        .peek()
        .iter()
        .any(|r| r.name == repo && r.public_throwaway)
}

/// The same question about the chat that is open.
pub(crate) fn open_chat_allows_free_models(ctx: &AppCtx) -> bool {
    let repo = ctx.code_chat.peek().repo.clone();
    repo_allows_free_models(ctx, &repo)
}

/// The chat whose container is asked for a catalogue the app has no chat of
/// its own to ask for.
///
/// A *running* one first, whichever it is: `/config/providers` and `/agent`
/// reach `OpenCode` through the manager's transparent proxy and any request to
/// a stopped chat wakes it, so borrowing one that is already awake costs
/// nothing the idle spin-down cares about. Only when none is running does this
/// fall back to the chat you last opened, and then to the first the manager
/// listed — and that one does wake, which is the price of the catalogue living
/// where it lives.
fn catalogue_donor(ctx: &AppCtx) -> Option<String> {
    let chats = ctx.code_chats.peek();
    if let Some(running) = chats.iter().find(|c| c.is_running()) {
        return Some(running.id.clone());
    }
    ctx.code_chat
        .peek()
        .chat_id
        .clone()
        .or_else(|| chats.first().map(|c| c.id.clone()))
}

/// Fetch the chat server's model catalogue, once, on first need.
///
/// Deliberately not part of opening a chat: it is every model of every
/// provider and nothing outside the pickers reads it, so it is paid for when
/// one of them is opened and not before. The answer is app-wide and kept for
/// the session, so the cost is paid once per app run.
pub(crate) fn ensure_code_models(ctx: &AppCtx) {
    let Some(chat_id) = ctx.code_chat.peek().chat_id.clone() else {
        return;
    };
    fetch_models(ctx, chat_id);
}

/// The same catalogue, from whichever chat can answer — the new-session
/// screen's path, where there is no chat of its own yet.
pub(crate) fn ensure_code_catalogue(ctx: &AppCtx) {
    let Some(chat_id) = catalogue_donor(ctx) else {
        return;
    };
    fetch_models(ctx, chat_id);
}

fn fetch_models(ctx: &AppCtx, chat_id: String) {
    if !ctx.code_models.peek().is_empty() || *ctx.code_models_loading.peek() {
        return;
    }
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let mut loading = ctx.code_models_loading;
    loading.set(true);
    let ctx = *ctx;
    spawn_forever(async move {
        match client.models(&chat_id).await {
            Ok(models) => ctx.code_models.clone().set(models),
            Err(e) => show_toast(&ctx, format!("Model list unavailable: {e}")),
        }
        ctx.code_models_loading.clone().set(false);
    });
}

/// Fetch the open chat's agents — the composer's modes — loudly, on the tap
/// that opens the picker.
///
/// After [`refresh_code_agents`] has run on chat open this is a free no-op;
/// after a *failed* open it retries and says why, which is what the loud half
/// is for.
pub(crate) fn ensure_code_agents(ctx: &AppCtx) {
    let Some(chat_id) = ctx.code_chat.peek().chat_id.clone() else {
        return;
    };
    fetch_agents(ctx, chat_id, true);
}

/// The same list, quietly, as part of opening a chat.
///
/// This used to be paid for on the chip tap alone, to save a request per open.
/// What that bought was a chip naming itself — `Mode` — instead of the mode
/// the next turn actually runs in, which is the whole reason the chip exists.
/// The request is not the cost it was: `attach_chat` reaches this point with
/// the container already awake and five requests already spent on it, so
/// `GET /agent` adds one round trip and no wake. That is the identical trade
/// [`refresh_diff_counts`] makes two lines above it.
///
/// Quiet is load-bearing: an `OpenCode` build predating the route answers 404,
/// and a loud fetch here would toast on every single chat open.
pub(crate) fn refresh_code_agents(ctx: &AppCtx) {
    let Some(chat_id) = ctx.code_chat.peek().chat_id.clone() else {
        return;
    };
    fetch_agents(ctx, chat_id, false);
}

/// The agents of a borrowed chat — the new-session screen's path.
///
/// The donor is chosen for `repo` rather than taken from
/// [`catalogue_donor`], because agents are not the catalogue: models come from
/// the server's own config and are the same in every container, while agents
/// can be defined by the repository (`.opencode/agent/`). A chat on this repo
/// is therefore the only donor whose answer is certainly right for it, and one
/// that is already awake is preferred over one that has to be woken.
pub(crate) fn ensure_code_agent_list(ctx: &AppCtx, repo: &str) {
    let Some(chat_id) = agent_donor(ctx, repo) else {
        return;
    };
    fetch_agents(ctx, chat_id, true);
}

/// Whichever chat is asked what agents `repo` has: one of its own first,
/// running before stopped, and only then anybody's.
fn agent_donor(ctx: &AppCtx, repo: &str) -> Option<String> {
    let own = {
        let chats = ctx.code_chats.peek();
        chats
            .iter()
            .find(|c| c.repo == repo && c.is_running())
            .or_else(|| chats.iter().find(|c| c.repo == repo))
            .map(|c| c.id.clone())
    };
    own.or_else(|| catalogue_donor(ctx))
}

/// The repo whose container answered the agent list the app is holding, when
/// that is not `repo`.
///
/// The new-session screen states this in the picker rather than passing a
/// borrowed list off as the selected repo's. `None` covers both "it is this
/// repo's" and "the donor is no longer in the list", which want the same
/// silence.
pub(crate) fn borrowed_agents_from(ctx: &AppCtx, repo: &str) -> Option<String> {
    let from = ctx.code_agents_from.peek().clone();
    if from.is_empty() {
        return None;
    }
    ctx.code_chats
        .peek()
        .iter()
        .find(|c| c.id == from)
        .map(|c| c.repo.clone())
        .filter(|donor| donor != repo && !donor.is_empty())
}

/// Unlike the model catalogue this list is dropped when the chat that answered
/// it is no longer the one being asked about — on opening a chat, and on the
/// new-session screen when its repo has a container of its own to ask —
/// because agents can be defined by the repository (`.opencode/agent/`), so
/// one chat's are not necessarily another's.
///
/// `code_agents_from` is what makes that checkable. Without it the guard below
/// reads "some list is already here", which on the new-session screen made the
/// mode pill a no-op that showed the last chat you opened whatever repo you
/// had since chosen.
fn fetch_agents(ctx: &AppCtx, chat_id: String, loud: bool) {
    let held_from = ctx.code_agents_from.peek().clone();
    if held_from == chat_id
        && (!ctx.code_agents.peek().is_empty() || *ctx.code_agents_loading.peek())
    {
        return;
    }
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    ctx.code_agents_from.clone().set(chat_id.clone());
    ctx.code_agents.clone().set(Vec::new());
    let mut loading = ctx.code_agents_loading;
    loading.set(true);
    let ctx = *ctx;
    spawn_forever(async move {
        let answer = client.agents(&chat_id).await;
        // The reader may have opened a chat, or pointed the new-session screen
        // at another repo, while the container answered — either makes this
        // the wrong list to write. Same guard `ensure_code_branches` uses.
        if *ctx.code_agents_from.peek() != chat_id {
            return;
        }
        match answer {
            Ok(agents) => ctx.code_agents.clone().set(agents),
            Err(e) => {
                if loud {
                    show_toast(&ctx, format!("Mode list unavailable: {e}"));
                }
            }
        }
        ctx.code_agents_loading.clone().set(false);
    });
}

/// Fetch `repo`'s branches, unless they are already in hand.
///
/// Fired by choosing a repo rather than by tapping the branch pill: the pill
/// has to be able to say `main` before it is touched, because the field's
/// placeholder names the branch in a sentence. It costs nothing a container
/// pays for — the manager answers this from GitHub with its own credential,
/// exactly as it answers `pulls`.
pub(crate) fn ensure_code_branches(ctx: &AppCtx, repo: &str) {
    if repo.is_empty() {
        return;
    }
    {
        let held = ctx.code_branches.peek();
        if held.repo == repo && (held.loading || !held.names.is_empty()) {
            return;
        }
    }
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    ctx.code_branches.clone().set(BranchList {
        repo: repo.to_owned(),
        loading: true,
        ..BranchList::default()
    });
    let (ctx, repo) = (*ctx, repo.to_owned());
    spawn_forever(async move {
        let answer = client.branches(&repo).await;
        // Quiet on failure, like `refresh_code_permissions`: a manager too old
        // to have the route would otherwise toast on every repo change, about
        // a pill that already says what it will do without it.
        let mut slot = ctx.code_branches;
        if slot.peek().repo != repo {
            return;
        }
        slot.set(match answer {
            Ok(list) => BranchList {
                default: list.default_name().map(str::to_owned),
                names: list.names(),
                truncated: list.truncated,
                repo,
                loading: false,
            },
            Err(_) => BranchList {
                repo,
                loading: false,
                ..BranchList::default()
            },
        });
    });
}

/// Run the open chat's next turn as `agent` (an [`Agent::name`]).
///
/// Mirrors [`set_code_model`]: re-picking the name that is already there is
/// still a pick, because it means the reader looked and kept it — and the
/// server must not overrule it on the next reconnect.
///
/// [`Agent::name`]: opencode_client::Agent::name
pub(crate) fn set_code_agent(ctx: &AppCtx, agent: &str) {
    let mut chat = ctx.code_chat;
    let mut c = chat.write();
    c.agent_picked = true;
    c.agent = Some(agent.to_owned());
}

/// Point the open chat's next turn at `reference` (`provider/model`).
pub(crate) fn set_code_model(ctx: &AppCtx, reference: &str) {
    let mut chat = ctx.code_chat;
    let mut c = chat.write();
    if c.model.as_deref() == Some(reference) {
        // Still a choice, even when it names what was already there: it means
        // the reader looked and kept it, so the server must not overrule it.
        c.picked = true;
        return;
    }
    c.picked = true;
    c.model = Some(reference.to_owned());
    // Effort tiers belong to a model, not to a session: one model's `xhigh`
    // is a 400 on the next. Carrying the old tier across a switch would send
    // a value the new model never offered.
    c.effort = None;
}

/// Set the open chat's thinking-effort tier; `None` is the model's default.
pub(crate) fn set_code_effort(ctx: &AppCtx, effort: Option<&str>) {
    let mut chat = ctx.code_chat;
    let mut c = chat.write();
    c.picked = true;
    c.effort = effort.map(str::to_owned);
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

    // A draft belongs to the chat it was typed in. It survives the review
    // screen (that is the point of hoisting it out of the view) but must not
    // follow you into a different conversation — and neither must a photo
    // picked for it.
    if ctx.code_chat.peek().chat_id.as_deref() != Some(meta.id.as_str()) {
        ctx.code_draft.clone().set(String::new());
        ctx.code_attachments.clone().set(Vec::new());
    }

    // Agents can come from the repository (`.opencode/agent/`), so the list
    // is per chat in a way the model catalogue is not. Dropping it here — and
    // with it the note of which container answered — is what makes the next
    // `ensure_code_agents` ask this chat's server.
    ctx.code_agents.clone().set(Vec::new());
    ctx.code_agents_from.clone().set(String::new());

    let cached = ctx.code_cache.peek().chats.get(&meta.id).cloned();
    let waking = !meta.is_running();
    // The review starts empty (the diff is fetched on demand) but not
    // forgetful: the marks come back off the cache.
    ctx.code_diff.clone().set(DiffState {
        view: cached
            .as_ref()
            .map(|c| marks_to_view(&c.diff_seen))
            .unwrap_or_default(),
        ..DiffState::default()
    });
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
        model: meta.model.clone(),
        effort: None,
        agent: None,
        agent_picked: false,
        picked: false,
    });
    screen.set(CodeScreen::Chat);
    // Asked for now rather than from `attach_chat`, which is where the diff's
    // counts are fetched. The diff needs the container awake and a session to
    // exist; this route is the manager talking to GitHub with its own
    // credential and touches neither, so waiting for the wake would be paying
    // a cost the request does not have. The chip carries its number while the
    // container is still booting.
    // The screen's half is this chat's and goes; the plane's half is every
    // row's and stays. Clearing `by_chat` here would blank the build state on
    // the whole list on every open and then not refill it for five minutes,
    // which is the sweep floor working against the thing it protects.
    let (by_chat, swept) = {
        let p = ctx.code_pulls.peek();
        (p.by_chat.clone(), p.swept)
    };
    ctx.code_pulls.clone().set(PullsState {
        by_chat,
        swept,
        ..PullsState::default()
    });
    refresh_pulls(ctx);
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
    let list = match sessions {
        Ok(list) => list,
        Err(e) => {
            chat.write().loading = false;
            chat.write().waking = false;
            show_toast(ctx, format!("Chat unreachable: {e}"));
            return;
        }
    };
    let session_id = {
        let cached = chat.peek().session_id.clone();
        cached
            .filter(|id| list.iter().any(|s| &s.id == id))
            .or_else(|| list.first().map(|s| s.id.clone()))
    };
    // The session record is the server's answer to what the next turn will
    // use, and it outranks whatever the chat was created with — but only
    // while the reader has not said otherwise.
    //
    // OpenCode writes the model onto the session when a TURN IS SENT, not
    // when it is picked. This path also runs on an SSE reconnect, so adopting
    // the server's value unconditionally threw away a pick the user had made
    // and not yet sent, with no visible cause. A local choice is the more
    // recent intent and stands until the next turn makes the server agree.
    //
    // The agent is adopted the same way, under its own flag, for the same
    // reason on the same path: the record carries it beside the model, written
    // by the same turn.
    if let Some(session) = list.iter().find(|s| Some(&s.id) == session_id.as_ref()) {
        let mut c = chat.write();
        if !c.picked {
            if let Some(model) = session.model.as_ref() {
                if let Some(reference) = model.reference() {
                    c.model = Some(reference);
                }
                c.effort = model.effort().map(str::to_owned);
            }
        }
        if !c.agent_picked {
            // `agent()` filters the empty string, so a server that sends
            // `"agent": ""` does not overwrite a resolved value with nothing.
            if let Some(agent) = session.agent() {
                c.agent = Some(agent.to_owned());
            }
        }
    }
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

    // The Diff chip states what is there before you tap it, which means the
    // diff has to be fetched by opening the chat rather than by opening the
    // review screen. Free in practice: the container is already awake by this
    // point, which is what the request would otherwise have paid for.
    //
    // The other two ride the same argument, and all three are fire-and-forget
    // spawns, so none of them delays the transcript, the SSE attach, or each
    // other. A chip that names the mode and the model has to be told what they
    // are, and here is where asking is cheapest.
    refresh_diff_counts(ctx);
    refresh_code_agents(ctx);
    resolve_default_model(ctx);

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
            // Taken before the fold, applied after it: the server replays an
            // attachment at the size it was sent, which is too big to keep in
            // a transcript this tab clones on every keystroke. Without this
            // the photos on screen turn into chips a second after the chat
            // opens (`crate::attach`).
            let thumbs = crate::attach::thumbnail_index(&ctx.code_chat.peek().items);
            let (mut items, part_index, roles, running) = fold_history(&msgs);
            crate::attach::restore_thumbnails(&thumbs, &mut items);
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
///
/// Says on the way in that this chat has a stream speaking for it, and takes
/// that back however the pump ends. It is what stops the manager's aggregate
/// second-guessing the stream, and — the half that was missing — what hands
/// the chat back to the aggregate when the stream is gone. A stream can end
/// for good: a `Disconnected` raised while the Code tab is not showing
/// schedules no re-attach, and `attach_chat` gives up outright when the
/// session fetch fails. Both left a chat with nobody speaking for it and the
/// aggregate still barred from doing so.
async fn stream_events(ctx: &AppCtx, client: &CodeClient, chat_id: &str, epoch: u64) {
    let mut live = ctx.code_stream;
    live.set(Some(chat_id.to_owned()));
    pump_events(ctx, client, chat_id, epoch).await;
    // Not an unconditional clear: a re-attach that has already claimed the
    // marker owns it now, and this one is the ghost.
    let still_ours = live.peek().as_deref() == Some(chat_id);
    if still_ours {
        live.set(None);
    }
}

/// The stream pump itself: every event folded until one of the three exits.
async fn pump_events(ctx: &AppCtx, client: &CodeClient, chat_id: &str, epoch: u64) {
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
                if *ctx.code_epoch.peek() == epoch && *ctx.tab.peek() == Tab::Code {
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
    // Parts the server wrote on the reader's behalf are the model's context,
    // not the conversation. Attaching a text file is the case that made this
    // matter: `OpenCode` expands one `text/plain` part into a fake "Called
    // the Read tool…" line and the whole decoded file, both persisted on the
    // user's message, so a 40 kB note came back as two more bubbles under the
    // one that sent it — and went into the on-device cache that way too.
    if part.synthetic {
        return;
    }
    match part.kind.as_str() {
        "text" | "reasoning" => fold_text_part(items, part_index, roles, part, delta, gap),
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
        "file" => fold_file_part(items, part_index, roles, part, gap),
        // step-start / step-finish / snapshot etc. — nothing to render.
        _ => {}
    }
}

/// Prose — a reply, a thought, or the reader's own message — into the item it
/// belongs to.
fn fold_text_part(
    items: &mut Vec<ChatItem>,
    part_index: &mut HashMap<String, usize>,
    roles: &HashMap<String, String>,
    part: &Part,
    delta: Option<&str>,
    gap: &mut GapSink<'_>,
) {
    let role = roles
        .get(&part.message_id)
        .map_or("assistant", String::as_str);
    let full = part.text.clone().unwrap_or_default();
    if let Some(&idx) = part_index.get(&part.id) {
        if let Some(
            ChatItem::Assistant { text, .. }
            | ChatItem::Thought { text, .. }
            | ChatItem::User { text, .. },
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
    // The prompt goes into the transcript the moment it is sent, so the
    // bubble does not wait on a round trip. The server then streams the same
    // message back as a part of its own, and with nothing matching the two up
    // every code session opened with the task written out twice. Adopt the
    // optimistic bubble instead: it is always the last item, and its text is
    // exactly what was sent.
    if role == "user" {
        if let Some(idx) = items.len().checked_sub(1) {
            let bound = part_index.values().any(|&i| i == idx);
            match items.get_mut(idx) {
                Some(ChatItem::User { text: t, .. }) if *t == text && !bound => {
                    part_index.insert(part.id.clone(), idx);
                    return;
                }
                // A bubble that exists only because this message's
                // attachments were folded before its text: fill it in rather
                // than opening a second bubble underneath it.
                Some(ChatItem::User {
                    text: t,
                    attachments,
                }) if t.is_empty() && !attachments.is_empty() => {
                    *t = text;
                    part_index.insert(part.id.clone(), idx);
                    return;
                }
                _ => {}
            }
        }
    }
    let item = match (part.kind.as_str(), role) {
        ("reasoning", _) => ChatItem::Thought {
            message_id: Some(part.id.clone()),
            text,
        },
        (_, "user") => ChatItem::User {
            text,
            attachments: Vec::new(),
        },
        _ => ChatItem::Assistant {
            message_id: Some(part.id.clone()),
            text,
        },
    };
    part_index.insert(part.id.clone(), items.len());
    crate::state::mark_gap(items.len(), gap.marks, gap.last_at);
    items.push(item);
}

/// An attachment, onto the message it was sent with.
fn fold_file_part(
    items: &mut Vec<ChatItem>,
    part_index: &mut HashMap<String, usize>,
    roles: &HashMap<String, String>,
    part: &Part,
    gap: &mut GapSink<'_>,
) {
    // Only what the reader attached. The agent's own messages carry file
    // parts too — a screenshot a tool took, a resource it read — and those
    // belong to the tool card that produced them, not to a bubble the reader
    // never wrote.
    if roles
        .get(&part.message_id)
        .map_or("assistant", String::as_str)
        != "user"
    {
        return;
    }
    if part_index.contains_key(&part.id) {
        return;
    }
    let record = crate::attach::from_part(part);
    // The bubble is either the optimistic one this message was sent from, or
    // the one its own text part just made. Anything else and this is a
    // message that is nothing but attachments.
    let idx = match items.len().checked_sub(1) {
        Some(idx) if matches!(items.get(idx), Some(ChatItem::User { .. })) => idx,
        _ => {
            crate::state::mark_gap(items.len(), gap.marks, gap.last_at);
            items.push(ChatItem::User {
                text: String::new(),
                attachments: Vec::new(),
            });
            items.len() - 1
        }
    };
    if let Some(ChatItem::User { attachments, .. }) = items.get_mut(idx) {
        // The optimistic bubble already shows what was just sent, and the
        // server echoes the same parts back under ids of its own. Name and
        // weight, not mime: a text attachment goes out declared `text/plain`
        // whatever the picker called it (`crate::attach::code_parts` says
        // why), so a `.md` echoed back never matched the `text/markdown` the
        // record kept and every one of them was drawn twice.
        let known = attachments
            .iter()
            .any(|a| a.name == record.name && a.size == record.size);
        if !known {
            attachments.push(record);
        }
    }
    part_index.insert(part.id.clone(), idx);
}

// --------------------------------------------------------------- actions

/// Send a prompt into the open code chat (creating its `OpenCode` session on
/// first use). Returns false if the message could not be handed to the client
/// at all.
///
/// True does not mean delivered: the POST is answered on a task of its own,
/// and an unreachable gateway or a container that outruns the 150s wake
/// surfaces there. Those paths put the files back in the tray themselves,
/// because the composer has already emptied it by then.
///
/// `files` is the caller's, not the tray's: "Open PR" and a new chat's opening
/// task are messages the app composes, and neither should pick up a photo the
/// reader had queued for something else.
pub(crate) fn send_code_prompt(
    ctx: &AppCtx,
    text: String,
    files: &[crate::attach::PendingAttachment],
) -> bool {
    let mut chat = ctx.code_chat;
    let Some(chat_id) = chat.peek().chat_id.clone() else {
        return false;
    };
    let Some(client) = ctx.code_client.peek().clone() else {
        show_toast(ctx, "Code plane not connected — check Settings");
        return false;
    };
    let parts = crate::attach::code_parts(&text, files);
    if parts.is_empty() {
        return false;
    }
    {
        let mut c = chat.write();
        let CodeChatState {
            items,
            marks,
            last_at,
            ..
        } = &mut *c;
        crate::state::mark_gap(items.len(), marks, last_at);
        items.push(ChatItem::User {
            text,
            attachments: crate::attach::records(files),
        });
        c.running = true;
    }
    // Held for the length of the request, so a failure has something to give
    // back. It costs a second copy of the payload until the POST is answered,
    // which is the price of not making a sleeping container eat the photo.
    let carried = files.to_vec();
    let ctx = *ctx;
    spawn_forever(async move {
        // Neither of the two ways this can fail is a delivery: the turn never
        // started, so the tray gets its files back and the toast says so.
        // Named with the chat it was sent in: waking a container can take
        // most of a minute, which is long enough to have opened another one,
        // and the tray this empties into is whichever is on screen now.
        let failed = |reason: String, files| {
            ctx.code_chat.clone().write().running = false;
            let note = crate::attach::return_to_tray(
                &ctx,
                crate::attach::AttachTarget::Code,
                &chat_id,
                files,
            );
            show_toast(&ctx, format!("{reason}{note}"));
        };
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
                    failed(format!("Session create failed: {e}"), carried);
                    return;
                }
            },
        };
        // Model, effort and agent all ride on the turn: OpenCode has no "set
        // the session's model" call, it copies whatever the turn asked for
        // onto the session record. That is why the sheet and the mode picker
        // both say "from your next message" and mean it.
        //
        // The agent is *resolved* here rather than sent as whatever the state
        // happens to hold, so the chip's claim and the request agree by
        // construction: the chip does not predict which agent the server will
        // pick, the app tells it. `prompt_body` drops an absent or empty
        // agent, so a resolution of None keeps today's behaviour byte for
        // byte.
        let (model, effort, agent) = {
            let c = ctx.code_chat.peek();
            let agents = ctx.code_agents.peek();
            (
                c.model.clone(),
                c.effort.clone(),
                opencode_client::resolve_agent(c.agent.as_deref(), &agents).map(str::to_owned),
            )
        };
        if let Err(e) = client
            .prompt_async(
                &chat_id,
                &sid,
                &parts,
                model.as_deref(),
                effort.as_deref(),
                agent.as_deref(),
            )
            .await
        {
            failed(format!("Prompt failed: {e}"), carried);
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
    let key = (chat_id.clone(), perm.id.clone());
    ctx.code_answered.clone().write().insert(key.clone());
    let ctx = *ctx;
    spawn_forever(async move {
        if let Err(e) = client
            .reply_permission(&chat_id, &perm.session_id, &perm.id, &response)
            .await
        {
            // The ask is still parked server-side, so the tombstone would be
            // hiding something real. Drop it and let the next aggregate put
            // the card back; the toast says why it reappeared.
            ctx.code_answered.clone().write().remove(&key);
            show_toast(&ctx, format!("Permission reply failed: {e}"));
        }
    });
}

/// What an ask is called: its title, or the tool kind when it has none.
pub(crate) fn ask_label(perm: &CodePermission) -> String {
    if perm.title.is_empty() {
        perm.kind.clone()
    } else {
        perm.title.clone()
    }
}

/// Whether the chat you are reading has an ask outstanding.
///
/// That one belongs to the modal, which is how a permission has always been
/// answered from inside a conversation. Every other chat's belongs to its
/// card in the list — a modal thrown over the screen about a chat you are not
/// in is the aggregate shouting rather than reporting.
///
/// "Reading", not "opened last": `code_chat` keeps its chat id when you back
/// out to the list, and the aggregate now speaks for a chat whose stream has
/// ended, so asking `code_chat` alone would throw the modal over the very
/// list whose card is already showing that ask with the same two answers on
/// it.
pub(crate) fn open_chat_has_ask(ctx: &AppCtx) -> bool {
    // peek throughout, read on the queue: this decides whether the root
    // renders the modal, and subscribing to `code_chat` would re-run the
    // whole app on every streamed token. The screen and the tab are safe to
    // peek because the root already reads both to pick what to render.
    if *ctx.tab.peek() != Tab::Code {
        return false;
    }
    if !matches!(*ctx.code_screen.peek(), CodeScreen::Chat | CodeScreen::Diff) {
        return false;
    }
    let Some(open) = ctx.code_chat.peek().chat_id.clone() else {
        return false;
    };
    ctx.code_permissions
        .read()
        .iter()
        .any(|(cid, _)| *cid == open)
}

/// Open the review screen and fetch the session's cumulative diff into it.
///
/// Navigates first and fetches after: the request wakes a stopped container
/// and can take the better part of a minute, and a chip that does nothing
/// visible for that long reads as broken.
pub(crate) fn load_code_diff(ctx: &AppCtx) {
    if fetch_diff(ctx, true) {
        ctx.code_screen.clone().set(CodeScreen::Diff);
    }
}

/// Ask the chat's own container which model it is configured to run, when
/// nothing else in the app knows.
///
/// A chat's record carries a model only when one was named at creation, and
/// its session record only after a turn has been sent — so a chat created
/// without a model and not yet prompted has none anywhere the app can see,
/// while its container has had one all along: the one `render_chat_config`
/// wrote into the chat volume. `GET /chat/<id>/config` is that file, resolved.
///
/// Silent on every failure path. Not knowing which model runs is exactly where
/// the app already was, so there is no news in saying so.
pub(crate) fn resolve_default_model(ctx: &AppCtx) {
    // Both facts off one guard, and the guard dropped before anything else
    // touches the signal — the same shape `send_code_prompt` had to be taught.
    let (chat_id, known) = {
        let c = ctx.code_chat.peek();
        (c.chat_id.clone(), c.model.is_some())
    };
    if known {
        return;
    }
    let Some(chat_id) = chat_id else {
        return;
    };
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let ctx = *ctx;
    spawn_forever(async move {
        let Ok(Some(model)) = client.default_model(&chat_id).await else {
            return;
        };
        // The reader may have opened another chat, or picked a model, while
        // the container answered. Either makes this answer the wrong one to
        // write — same guard `fetch_diff` uses, for the same reason.
        let mut chat = ctx.code_chat;
        let mut c = chat.write();
        if c.chat_id.as_deref() == Some(chat_id.as_str()) && c.model.is_none() && !c.picked {
            c.model = Some(model);
        }
    });
}

/// Fetch the diff without going anywhere, so the Diff chip can carry its
/// `+N −M` before the review screen has ever been opened.
///
/// Quiet: a chat with no session yet is the normal state of a chat you have
/// not prompted, and saying so on open would be noise rather than news.
pub(crate) fn refresh_diff_counts(ctx: &AppCtx) {
    fetch_diff(ctx, false);
}

/// Returns false when there was nothing to fetch.
fn fetch_diff(ctx: &AppCtx, loud: bool) -> bool {
    let chat = ctx.code_chat.peek();
    let (Some(chat_id), Some(sid)) = (chat.chat_id.clone(), chat.session_id.clone()) else {
        if loud {
            show_toast(ctx, "No changes yet — the chat has no session");
        }
        return false;
    };
    drop(chat);
    let Some(client) = ctx.code_client.peek().clone() else {
        if loud {
            show_toast(ctx, "Code plane not connected — check Settings");
        }
        return false;
    };
    {
        let mut diff = ctx.code_diff;
        let mut d = diff.write();
        d.loading = true;
        d.error = None;
    }
    let ctx = *ctx;
    spawn_forever(async move {
        let result = client.diff(&chat_id, &sid).await;
        // The user may have walked back to the list and opened another chat
        // while the container woke; writing this chat's files into that one
        // would be a silent lie about what changed.
        if ctx.code_chat.peek().chat_id.as_deref() != Some(chat_id.as_str()) {
            return;
        }
        let mut diff = ctx.code_diff;
        let mut d = diff.write();
        d.loading = false;
        match result {
            Ok(files) => {
                d.view = marks_to_view(&d.marks());
                d.files = files.into_iter().map(DiffFile::from).collect();
                d.error = None;
            }
            Err(e) => d.error = Some(e.to_string()),
        }
    });
    true
}

/// Fold or unfold one file's card. Independent of whether it is marked
/// reviewed, and independent of every other file.
pub(crate) fn toggle_diff_file(ctx: &AppCtx, path: &str) {
    let mut diff = ctx.code_diff;
    let open = {
        let d = diff.peek();
        d.files
            .iter()
            .find(|f| f.info.file == path)
            .map(|f| d.is_open(f))
    };
    let Some(open) = open else { return };
    diff.write().view.entry(path.to_owned()).or_default().open = Some(!open);
}

/// Mark a file reviewed, or clear the mark. Marking also folds the card,
/// which is the point: a finished file stops occupying the scroll.
pub(crate) fn toggle_diff_seen(ctx: &AppCtx, path: &str, fingerprint: u64) {
    let mut diff = ctx.code_diff;
    {
        let mut d = diff.write();
        let entry = d.view.entry(path.to_owned()).or_default();
        if entry.seen == Some(fingerprint) {
            entry.seen = None;
            entry.open = Some(true);
        } else {
            entry.seen = Some(fingerprint);
            entry.open = Some(false);
        }
    }
    write_cache(ctx);
}

pub(crate) fn mark_all_diff_seen(ctx: &AppCtx) {
    let mut diff = ctx.code_diff;
    {
        let mut d = diff.write();
        let marks: Vec<(String, u64)> = d
            .files
            .iter()
            .map(|f| (f.info.file.clone(), f.fingerprint))
            .collect();
        for (path, fingerprint) in marks {
            let entry = d.view.entry(path).or_default();
            entry.seen = Some(fingerprint);
            entry.open = Some(false);
        }
    }
    write_cache(ctx);
}

/// Give back part of a collapsed band of unchanged lines.
pub(crate) fn expand_diff_gap(ctx: &AppCtx, path: &str, key: usize, hidden: usize) {
    let mut diff = ctx.code_diff;
    let mut d = diff.write();
    let entry = d.view.entry(path.to_owned()).or_default();
    let revealed = entry.expanded.entry(key).or_insert(0);
    *revealed = crate::diff::expand_to(*revealed, hidden);
}

/// Show the body of a deleted file, which is otherwise a count rather than
/// several hundred red rows nobody reads line by line.
pub(crate) fn reveal_removed_lines(ctx: &AppCtx, path: &str) {
    let mut diff = ctx.code_diff;
    diff.write()
        .view
        .entry(path.to_owned())
        .or_default()
        .show_removed = true;
}

// ---------------------------------------------------------- pull requests

/// Open the pull-request screen and refresh it.
pub(crate) fn open_code_pulls(ctx: &AppCtx) {
    ctx.code_screen.clone().set(CodeScreen::Pulls);
    refresh_pulls(ctx);
}

/// Ask the manager what this chat's branch has open on GitHub.
///
/// Costs the chat nothing: `/api/chats/<id>/pulls` is answered by the manager
/// from its own GitHub credential and never proxied to the container, so this
/// neither wakes a sleeping chat nor keeps a waking one busy. That is what
/// makes it safe to call on every chat open.
pub(crate) fn refresh_pulls(ctx: &AppCtx) {
    let Some(chat_id) = ctx.code_chat.peek().chat_id.clone() else {
        return;
    };
    let mut pulls = ctx.code_pulls;
    let Some(client) = ctx.code_client.peek().clone() else {
        // Written into the screen's state rather than toasted: nothing is
        // visible yet on a chat open, and by the time the reader gets here the
        // toast would be long gone.
        pulls.write().error = Some("Code plane not connected — check Settings".to_owned());
        return;
    };
    {
        let mut p = pulls.write();
        p.loading = true;
        p.error = None;
    }
    let ctx = *ctx;
    spawn_forever(async move {
        let result = client.pulls(&chat_id).await;
        // Another chat may have been opened while GitHub was answering; its
        // branch has its own pull requests and these are not them.
        if ctx.code_chat.peek().chat_id.as_deref() != Some(chat_id.as_str()) {
            return;
        }
        let mut pulls = ctx.code_pulls;
        let mut p = pulls.write();
        p.loading = false;
        match result {
            Ok(list) => {
                // The plane's copy too, so the row this chat came from is
                // never staler than the screen opened from it — and so a
                // merge answered here shows on the list without waiting out
                // the sweep floor.
                p.by_chat.insert(chat_id, list.clone());
                p.pulls = list;
                p.loaded = true;
                p.error = None;
            }
            Err(e) => p.error = Some(e.message()),
        }
    });
}

/// How long a plane-wide sweep's answer stands before another is allowed.
///
/// Five minutes, and the number is a rate limit rather than a taste. See
/// [`refresh_plane_pulls`] for the arithmetic; the short version is that this
/// sweep on the ten-second poll would spend seven times GitHub's whole hourly
/// budget, and at this floor it spends between 5% and 23% of it. A build that
/// turns red is on the rows within five minutes, which is inside the time the
/// build itself takes.
const SWEEP_FLOOR_SECS: u64 = 300;

/// How many chats one sweep will ask about, most recently active first.
///
/// The manager sends the index newest-first and this app renders it in wire
/// order, so the cap falls on the rows a reader has to scroll to reach. It is
/// a ceiling on the worst case, not a target: it exists so that a fleet that
/// grows to eighty trees cannot quietly turn a status dot into a
/// rate-limit outage.
const SWEEP_MAX_CHATS: usize = 24;

/// Whether a sweep is allowed to start, given when the last one did.
///
/// A function of its own for [`poll_tick`]'s reason: this is the whole of the
/// rate limit, and it is worth being able to say in a test that a sweep two
/// seconds after a sweep does not happen while one six minutes after one
/// does. `swept == 0` is "never swept", which is always due — the first list
/// to arrive should carry its build states, not wait five minutes for them.
///
/// `saturating_sub` is not decoration: `now_secs` reads the wall clock, and a
/// clock that steps backwards (a phone crossing a time-zone-less NTP
/// correction, a laptop waking) would otherwise wrap the subtraction and make
/// every sweep due forever.
const fn sweep_due(swept: u64, now: u64) -> bool {
    swept == 0 || now.saturating_sub(swept) >= SWEEP_FLOOR_SECS
}

/// Fill in every row's build state: one `/api/chats/<id>/pulls` per chat,
/// floored to one sweep per [`SWEEP_FLOOR_SECS`] and capped at
/// [`SWEEP_MAX_CHATS`] chats.
///
/// **Why a fan-out is allowed here and is not allowed for the diff.** This
/// route is the manager talking to GitHub with its own credential
/// (`chat_pulls`, personal-ai-setup `scripts/vps/code-agent-manager.py`); it
/// is never proxied to a container, so a sweep wakes nothing and keeps
/// nothing awake. The per-session diff is the opposite — `/chat/<id>/…` goes
/// through the transparent proxy — which is why issue #81's numbers are not
/// fetched this way and are absent instead.
///
/// **The cost, measured rather than asserted.** The manager spends `1 + 3P`
/// GitHub calls per chat: one list call, then per pull request on the branch
/// one detail call (the list form carries no `mergeable` — confirmed against
/// the real API, `GET /repos/…/pulls?per_page=1` answers without it) and two
/// in `summarise_checks` (check runs, then combined status). A fine-grained
/// PAT gets 5,000 REST requests an hour.
///
/// | policy | calls/sweep | calls/hour | share of 5,000 |
/// |---|---|---|---|
/// | 24 chats, 1 pull each, every 10s poll | 96 | 34,560 | 691% |
/// | 24 chats, 1 pull each, this floor | 96 | 1,152 | 23% |
/// | 10 chats, 4 with a pull, this floor | 22 | 264 | 5.3% |
///
/// The first row is why the sweep does not ride the poll, and 23% for a
/// status dot is still more than it is worth. **The fix is one aggregate
/// route on the manager**, the shape `/api/permissions` already has, which
/// would make the whole table one call — filed upstream as
/// `PhillipChaffee/personal-ai-setup#29` and linked from #84.
pub(crate) fn refresh_plane_pulls(ctx: &AppCtx) {
    let mut pulls = ctx.code_pulls;
    let Some(client) = ctx.code_client.peek().clone() else {
        return;
    };
    let ids: Vec<String> = ctx
        .code_chats
        .peek()
        .iter()
        .take(SWEEP_MAX_CHATS)
        .map(|c| c.id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    let now = now_secs();
    {
        let mut p = pulls.write();
        // `swept` is claimed here rather than when the sweep finishes, so the
        // poll tick that lands while a slow sweep is still walking the list
        // does not start a second one behind it.
        if !sweep_due(p.swept, now) {
            return;
        }
        p.swept = now;
    }
    let ctx = *ctx;
    spawn_forever(async move {
        for id in &ids {
            // Sequential, not joined: a burst of twenty-four TLS requests at
            // once is twenty-four threads on the manager's
            // `ThreadingHTTPServer`, each holding a 20-second GitHub timeout.
            // Nothing on screen is waiting for the last row.
            if ctx.code_client.peek().is_none() {
                return;
            }
            if let Ok(list) = client.pulls(id).await {
                // Only a success writes. A chat GitHub could not answer for
                // keeps the answer it had, because a row that drops its build
                // state on one flaky request reads as "this branch has no
                // pull request", which is a different claim.
                ctx.code_pulls
                    .clone()
                    .write()
                    .by_chat
                    .insert(id.clone(), list);
            }
        }
        // A deleted chat's answer would otherwise sit in the map for the life
        // of the process, and its id can be reused. Kept against the WHOLE
        // index and not against `ids`: a chat past the cap was never asked
        // about, which is not the same as one that is gone, and dropping it
        // would also throw away the open chat's own answer whenever it sits
        // below row twenty-four.
        let live: HashSet<String> = ctx.code_chats.peek().iter().map(|c| c.id.clone()).collect();
        ctx.code_pulls
            .clone()
            .write()
            .by_chat
            .retain(|id, _| live.contains(id));
    });
}

/// Merge one of this chat's pull requests. Confirmed before it gets here.
pub(crate) fn merge_pull(ctx: &AppCtx, number: u64) {
    let Some(chat_id) = ctx.code_chat.peek().chat_id.clone() else {
        return;
    };
    let Some(client) = ctx.code_client.peek().clone() else {
        show_toast(ctx, "Code plane not connected — check Settings");
        return;
    };
    ctx.code_pulls.clone().write().merging = Some(number);
    let ctx = *ctx;
    spawn_forever(async move {
        let result = client.merge_pull(&chat_id, number).await;
        if ctx.code_chat.peek().chat_id.as_deref() != Some(chat_id.as_str()) {
            return;
        }
        // The fresh row the manager sends back, folded in before anything else
        // runs — the write guard has to be gone before `show_toast` or
        // `refresh_pulls` touch a signal.
        let repainted = {
            let mut pulls = ctx.code_pulls;
            let mut p = pulls.write();
            p.merging = None;
            match result.as_ref().map(|outcome| outcome.pull.clone()) {
                Ok(Some(fresh)) => {
                    if let Some(row) = p.pulls.iter_mut().find(|row| row.number == number) {
                        *row = fresh;
                    }
                    true
                }
                Ok(None) | Err(_) => false,
            }
        };
        match result {
            Ok(_) => show_toast(&ctx, format!("Merged #{number}")),
            // Whatever the manager said: the pull request is already merged,
            // GitHub wants a review, GitHub is unreachable. All of it is more
            // useful than "merge failed".
            Err(e) => show_toast(&ctx, e.message()),
        }
        // A refusal means the row was describing a state that had already
        // moved on, so the list is re-read rather than left saying it again.
        if !repainted {
            refresh_pulls(&ctx);
        }
    });
}

/// Dot class and word for where a pull request stands, the way
/// `status_label` does it for a chat's container.
pub(crate) const fn pull_state_label(pull: &PullRequest) -> (&'static str, &'static str) {
    match pull.state {
        PullState::Merged => ("dot on", "merged"),
        PullState::Closed => ("dot off", "closed"),
        // Draft is a flag on an open pull request, and it is the more useful
        // of the two facts: "open" is what every pull request is until it is
        // not, "draft" is why nothing can be done with this one yet.
        PullState::Open if pull.draft => ("dot off", "draft"),
        PullState::Open => ("dot on", "open"),
        PullState::Unknown => ("dot err", "state unknown"),
    }
}

/// Dot class and words for the head commit's checks.
pub(crate) const fn checks_label(checks: Checks) -> (&'static str, &'static str) {
    match checks {
        Checks::Passing => ("dot on", "checks passing"),
        Checks::Failing => ("dot err", "checks failing"),
        Checks::Pending => ("dot busy", "checks running"),
        Checks::None => ("dot off", "no checks"),
        // Not a parse failure: the manager's PAT carries Contents and Pull
        // requests, and reading check runs needs a scope of its own.
        Checks::Unknown => ("dot off", "checks unknown"),
    }
}

/// Dot class and word for a mergeability the row is not otherwise saying.
///
/// The state and checks chips beside this one name three of the four reasons
/// merging is not offered: closed, merged, draft, failing. The fourth is
/// `mergeable` itself, and it is invisible — an open pull request with passing
/// checks that GitHub says conflicts renders exactly like one it says can
/// merge, minus the button. So this chip is that fact and only that fact.
///
/// `None` where the answer is already on the row: where merging *is* offered,
/// where the pull request is not open, and where one of the other two chips is
/// already carrying the reason. Draft outranks the rest for the reason it
/// always did — marking it ready is the first move whatever else is wrong —
/// and only one reason is ever shown.
pub(crate) const fn mergeability_label(pull: &PullRequest) -> Option<(&'static str, &'static str)> {
    if !matches!(pull.state, PullState::Open)
        || pull.draft
        || matches!(pull.checks, Checks::Failing)
    {
        return None;
    }
    match pull.mergeable {
        Some(true) => None,
        Some(false) => Some(("dot err", "conflicts")),
        // Not a refusal: GitHub computes mergeability asynchronously and has
        // not answered yet. Pulling the list again is what re-asks.
        None => Some(("dot busy", "mergeability pending")),
    }
}

/// What a LIST ROW says about the newest pull request off its branch.
///
/// The number is the useful half — it is the thing you type into a browser,
/// and it is the only identifier a working tree has outside this app — so it
/// leads. The state follows as a word, from [`pull_state_label`], which is the
/// same vocabulary the pull-request screen uses; two screens with two words
/// for `draft` would be two facts to a reader.
///
/// [`PullState::Unknown`] drops the word rather than printing "state unknown"
/// on a row: the number is still true, and a row is not where a reader can do
/// anything about a state string this client has not heard of. The pull
/// request screen still says it, with a red dot, where the reader is looking
/// at that one pull request.
pub(crate) fn row_pull_word(pull: &PullRequest) -> String {
    if matches!(pull.state, PullState::Unknown) {
        return format!("#{}", pull.number);
    }
    let (_, word) = pull_state_label(pull);
    format!("#{} {word}", pull.number)
}

/// Dot and word for the build behind a row, or `None` when there is no build
/// this row can usefully report (issue #84).
///
/// `None` in three cases, and each is a decision rather than a gap:
///
/// - [`Checks::None`] — nothing runs checks on this repo. Every row on that
///   repo would carry the same words forever.
/// - [`Checks::Unknown`] — the manager's PAT cannot read check runs
///   (`summarise_checks` says so in its own doc). Fifteen rows reading "checks
///   unknown" is a credential problem announced fifteen times, and the pull
///   request screen already draws that distinction with a chip of its own,
///   one tap away, where a reader is looking at the one branch they care
///   about.
/// - the pull request is not open. A closed branch's red build is not
///   something anybody is going to fix, and `merged` / `closed` from
///   [`row_pull_word`] is already the whole story of that row.
///
/// Rejected: showing the chip unconditionally, on the argument that the wire
/// said it so the app should say it. It is the argument the app usually
/// takes, and it loses here because this row exists to answer "which of these
/// wants me" — a chip that is present on every row answers nothing, and the
/// fact is one tap away rather than absent.
pub(crate) const fn row_checks_label(pull: &PullRequest) -> Option<(&'static str, &'static str)> {
    if !matches!(pull.state, PullState::Open) {
        return None;
    }
    match pull.checks {
        Checks::Passing | Checks::Failing | Checks::Pending => Some(checks_label(pull.checks)),
        Checks::None | Checks::Unknown => None,
    }
}

/// Everything the new-session composer collected.
///
/// A struct rather than five positional arguments because four of them are
/// `Option<String>`-shaped, and a call site with three `None`s in a row is a
/// bug waiting to be written.
pub(crate) struct NewSessionSpec {
    pub repo: String,
    pub task: String,
    /// `provider/model`. `None` means the manager's own default, which is the
    /// one case the model pill allows it.
    pub model: Option<String>,
    /// The agent the first turn runs as — what the composer calls the mode.
    pub agent: Option<String>,
    /// What the session's branch is cut from. `None` is the repo's default.
    pub base_branch: Option<String>,
}

/// Create a new code chat and open it, sending the task as its first prompt.
///
/// `files` is passed in rather than read off a tray: the new-session screen has
/// a tray of its own ([`crate::state::AppCtx::new_attachments`]), and these
/// have to be lifted out of it before the create, because by the time the
/// prompt goes out the screen has been replaced by the chat it made. The tray
/// is emptied here rather than by the composer, so a create that fails leaves
/// the photos where the reader can still see them.
pub(crate) fn new_code_chat(
    ctx: &AppCtx,
    spec: NewSessionSpec,
    files: Vec<crate::attach::PendingAttachment>,
) {
    let Some(client) = ctx.code_client.peek().clone() else {
        show_toast(ctx, "Code plane not connected — check Settings");
        return;
    };
    let ctx = *ctx;
    spawn_forever(async move {
        show_toast(&ctx, format!("Preparing workspace for {}…", spec.repo));
        let created = client
            .create_chat(
                &spec.repo,
                &spec.task,
                spec.model.as_deref(),
                spec.base_branch.as_deref(),
            )
            .await;
        match created {
            Ok(meta) => {
                refresh_code_chats(&ctx).await;
                open_code_chat(&ctx, meta);
                // They are this chat's now, and the screen that held them is
                // gone. Left behind they would be in the tray of the *next*
                // new session, whatever repo that one is pointed at.
                ctx.new_attachments.clone().set(Vec::new());
                // The reader's picks, written back over the state
                // `open_code_chat` seeds from the manager's record alone.
                // `picked` with them: OpenCode writes a model onto a session
                // only when a turn is sent, so without it the reconnect path
                // in `attach_chat` would adopt the server's value and throw
                // away a choice made seconds earlier.
                {
                    let mut chat = ctx.code_chat;
                    let mut c = chat.write();
                    if spec.agent.is_some() {
                        c.agent.clone_from(&spec.agent);
                        c.agent_picked = true;
                    }
                    if spec.model.is_some() {
                        c.model.clone_from(&spec.model);
                        c.picked = true;
                    }
                }
                // A first turn always carries text, because the task is also
                // the session's title — `can_start` is what makes that true,
                // and an attachment alone cannot start a session for the same
                // reason. So there is no files-only case to guard for here,
                // unlike in the chat composer where a photo is a message.
                if !spec.task.trim().is_empty() {
                    // First prompt after the open flow resolves the session.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    send_code_prompt(&ctx, spec.task, &files);
                }
            }
            Err(e) => show_toast(&ctx, format!("Create failed: {}", e.message())),
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
        diff_seen: ctx.code_diff.peek().marks(),
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
///
/// An outstanding ask outranks every container status, because it is both the
/// more specific fact and the more recent one. `running_turn` is only ever
/// true for the chat the app has open — nothing in the manager's index says
/// whether a container is mid-turn — so a chat parked on a permission fell to
/// `("running", false)` and reported itself **idle**, in the line directly
/// above its own "Approve or deny …" panel. A card cannot hold both those
/// statements at once, and the aggregate is the one that knows.
pub(crate) fn status_label(
    meta: &ChatMeta,
    running_turn: bool,
    waiting: bool,
) -> (&'static str, String) {
    if waiting {
        return ("dot wait", "waiting on you".to_string());
    }
    match (meta.status.as_str(), running_turn) {
        ("running", true) => ("dot busy", "working".to_string()),
        ("running", false) => ("dot on", "idle".to_string()),
        ("stopped" | "absent", _) => ("dot off", "asleep".to_string()),
        (other, _) => ("dot err", other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        checks_label, fold_part_into, merge_permission_report, mergeability_label, poll_tick,
        pull_state_label, row_checks_label, row_pull_word, status_label, sweep_due, ChatItem,
        ChatMeta, Checks, CodePermission, GapSink, HashMap, HashSet, PermissionReport, PullRequest,
        PullState, PullsState, Tab, Tick, SWEEP_FLOOR_SECS,
    };
    use opencode_client::{Part, PendingAsk};

    fn pull(state: PullState, draft: bool, mergeable: Option<bool>, checks: Checks) -> PullRequest {
        PullRequest {
            number: 12,
            state,
            draft,
            mergeable,
            checks,
            ..PullRequest::default()
        }
    }

    /// Draft is the more useful of the two facts about an open draft, and a
    /// state this client has not heard of must not read as anything reassuring.
    #[test]
    fn a_pulls_state_reads_as_the_thing_that_matters_about_it() {
        let open = pull(PullState::Open, false, Some(true), Checks::Passing);
        assert_eq!(pull_state_label(&open), ("dot on", "open"));

        let draft = pull(PullState::Open, true, Some(true), Checks::Passing);
        assert_eq!(pull_state_label(&draft), ("dot off", "draft"));

        let merged = pull(PullState::Merged, false, None, Checks::Passing);
        assert_eq!(pull_state_label(&merged), ("dot on", "merged"));

        let unknown = pull(PullState::Unknown, false, Some(true), Checks::Passing);
        assert_eq!(pull_state_label(&unknown), ("dot err", "state unknown"));
    }

    /// A row leads with the number, because that is the only identifier a
    /// working tree has outside this app — and it drops a state word this
    /// client has not heard of rather than printing "state unknown" in a list.
    #[test]
    fn a_row_names_the_pull_request_by_number_and_where_it_stands() {
        assert_eq!(
            row_pull_word(&pull(PullState::Open, false, Some(true), Checks::Passing)),
            "#12 open"
        );
        assert_eq!(
            row_pull_word(&pull(PullState::Open, true, None, Checks::Pending)),
            "#12 draft"
        );
        assert_eq!(
            row_pull_word(&pull(PullState::Merged, false, None, Checks::Passing)),
            "#12 merged"
        );
        assert_eq!(
            row_pull_word(&pull(PullState::Closed, false, None, Checks::None)),
            "#12 closed"
        );
        assert_eq!(
            row_pull_word(&pull(PullState::Unknown, false, None, Checks::Passing)),
            "#12",
            "a list row has nothing a reader can do about an unrecognised \
             state, and the number is still true"
        );
    }

    /// The build a row draws, and the three it deliberately does not: a repo
    /// that runs no checks, a credential that cannot read them, and a branch
    /// that is already closed or merged. Each would be the same words on every
    /// row of a list that exists to say which row wants you.
    #[test]
    fn a_row_draws_a_build_only_where_there_is_one_to_act_on() {
        assert_eq!(
            row_checks_label(&pull(PullState::Open, false, Some(true), Checks::Failing)),
            Some(("dot err", "checks failing"))
        );
        assert_eq!(
            row_checks_label(&pull(PullState::Open, true, None, Checks::Pending)),
            Some(("dot busy", "checks running")),
            "a draft is still a branch with a build running on it"
        );
        assert_eq!(
            row_checks_label(&pull(PullState::Open, false, Some(true), Checks::Passing)),
            Some(("dot on", "checks passing"))
        );
        assert_eq!(
            row_checks_label(&pull(PullState::Open, false, Some(true), Checks::None)),
            None
        );
        assert_eq!(
            row_checks_label(&pull(PullState::Open, false, Some(true), Checks::Unknown)),
            None
        );
        assert_eq!(
            row_checks_label(&pull(PullState::Merged, false, None, Checks::Failing)),
            None,
            "nobody is going to fix a red build on a branch that has landed"
        );
        assert_eq!(
            row_checks_label(&pull(PullState::Closed, false, None, Checks::Failing)),
            None
        );
    }

    /// Three answers, not two: never asked, asked and none, asked and some.
    /// A chat the sweep has not reached must not read as a branch with no
    /// pull request, for `loaded`'s reason one level down.
    #[test]
    fn the_plane_index_tells_unasked_apart_from_answered_empty() {
        let mut state = PullsState::default();
        assert!(state.plane_pull("c1").is_none());

        state.by_chat.insert("c1".to_owned(), Vec::new());
        assert!(state.plane_pull("c1").is_none());

        let newest = PullRequest {
            number: 30,
            ..pull(PullState::Open, false, Some(true), Checks::Passing)
        };
        let older = PullRequest {
            number: 29,
            ..pull(PullState::Closed, false, None, Checks::Passing)
        };
        state.by_chat.insert("c2".to_owned(), vec![newest, older]);
        assert_eq!(
            state.plane_pull("c2").map(|p| p.number),
            Some(30),
            "the manager sorts newest first and a reopened branch is about \
             what is happening now"
        );
    }

    /// The whole of the rate limit. A sweep two seconds after a sweep must not
    /// happen — that is the ten-second poll multiplying the cost by thirty —
    /// and the first list to arrive must not wait five minutes for its builds.
    #[test]
    fn a_sweep_waits_out_its_floor_and_survives_a_clock_that_steps_back() {
        assert!(sweep_due(0, 0), "never swept is always due");
        assert!(sweep_due(0, 9_999));
        assert!(!sweep_due(1_000, 1_002));
        assert!(!sweep_due(1_000, 1_000 + SWEEP_FLOOR_SECS - 1));
        assert!(sweep_due(1_000, 1_000 + SWEEP_FLOOR_SECS));
        assert!(
            !sweep_due(1_000, 500),
            "a clock that stepped backwards must not make every sweep due"
        );
    }

    /// "No checks" and "we could not read the checks" are different facts, and
    /// the second one is the normal answer for a private repo — the manager's
    /// PAT carries Contents and Pull requests, not Checks.
    #[test]
    fn absent_checks_and_unreadable_checks_are_told_apart() {
        assert_eq!(checks_label(Checks::None), ("dot off", "no checks"));
        assert_eq!(checks_label(Checks::Unknown), ("dot off", "checks unknown"));
        assert_eq!(
            checks_label(Checks::Pending),
            ("dot busy", "checks running")
        );
        assert_eq!(checks_label(Checks::Failing), ("dot err", "checks failing"));
    }

    /// The two merge-blocked states no other chip on the row can name — a
    /// conflict, and a mergeability GitHub has not worked out yet — each get
    /// one of their own. Without them an open pull request that cannot be
    /// merged reads exactly like one that can, minus the button.
    #[test]
    fn a_pull_that_only_conflicts_still_says_so() {
        assert_eq!(
            mergeability_label(&pull(PullState::Open, false, Some(false), Checks::Passing)),
            Some(("dot err", "conflicts"))
        );
        assert_eq!(
            mergeability_label(&pull(PullState::Open, false, None, Checks::Passing)),
            Some(("dot busy", "mergeability pending")),
            "mergeability GitHub has not computed is a wait, and says so"
        );
    }

    /// Silent wherever the row already answers the question: a Merge button is
    /// its own explanation, a merged or closed pull request raises no question,
    /// and draft and failing checks are already chips beside this one.
    #[test]
    fn a_row_that_already_says_why_says_it_once() {
        assert_eq!(
            mergeability_label(&pull(PullState::Open, false, Some(true), Checks::Passing)),
            None,
            "a row with a Merge button needs no explanation"
        );
        assert_eq!(
            mergeability_label(&pull(PullState::Merged, false, Some(false), Checks::None)),
            None,
            "nobody wonders why a merged pull request cannot be merged"
        );
        assert_eq!(
            mergeability_label(&pull(PullState::Open, true, Some(false), Checks::Failing)),
            None,
            "draft outranks the rest: marking it ready is the first move"
        );
        assert_eq!(
            mergeability_label(&pull(PullState::Open, false, None, Checks::Failing)),
            None,
            "the checks chip is already carrying this row's reason"
        );
    }

    fn text_part(id: &str, message_id: &str, text: &str) -> Part {
        Part {
            id: id.to_owned(),
            message_id: message_id.to_owned(),
            session_id: "ses_1".to_owned(),
            kind: "text".to_owned(),
            text: Some(text.to_owned()),
            ..Part::default()
        }
    }

    fn file_part(id: &str, message_id: &str, name: &str) -> Part {
        Part {
            id: id.to_owned(),
            message_id: message_id.to_owned(),
            session_id: "ses_1".to_owned(),
            kind: "file".to_owned(),
            mime: Some("image/jpeg".to_owned()),
            filename: Some(name.to_owned()),
            url: Some("data:image/jpeg;base64,QUJD".to_owned()),
            ..Part::default()
        }
    }

    /// A text attachment as it comes back: declared `text/plain` whatever the
    /// picker called it, because that is the only mime the server inlines.
    fn text_file_part(id: &str, message_id: &str, name: &str) -> Part {
        Part {
            mime: Some("text/plain".to_owned()),
            url: Some("data:text/plain;base64,QUJD".to_owned()),
            ..file_part(id, message_id, name)
        }
    }

    /// A part the server wrote on the reader's behalf.
    fn synthetic_part(id: &str, message_id: &str, text: &str) -> Part {
        Part {
            synthetic: true,
            ..text_part(id, message_id, text)
        }
    }

    /// The bubble a message with one text attachment is sent from.
    fn optimistic(text: &str, name: &str, mime: &str) -> ChatItem {
        ChatItem::User {
            text: text.to_owned(),
            attachments: vec![crate::attach::Attachment {
                name: name.to_owned(),
                mime: mime.to_owned(),
                size: 3,
                thumb: String::new(),
            }],
        }
    }

    fn fold(
        items: &mut Vec<ChatItem>,
        index: &mut HashMap<String, usize>,
        roles: &HashMap<String, String>,
        part: &Part,
    ) {
        let (mut marks, mut last_at) = (Vec::new(), 0);
        fold_part_into(
            items,
            index,
            roles,
            part,
            None,
            &mut GapSink {
                marks: &mut marks,
                last_at: &mut last_at,
            },
        );
    }

    fn roles_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(id, role)| ((*id).to_owned(), (*role).to_owned()))
            .collect()
    }

    fn attachments_of(item: &ChatItem) -> &[crate::attach::Attachment] {
        match item {
            ChatItem::User { attachments, .. } => attachments,
            _ => &[],
        }
    }

    /// A message the reader sent with a photo comes back as a text part and a
    /// file part. Both belong to the one bubble that is already on screen —
    /// the file part must not open a second turn under it.
    #[test]
    fn a_replayed_file_part_lands_on_the_message_it_belongs_to() {
        let mut items = Vec::new();
        let mut index = HashMap::new();
        let roles = roles_of(&[("msg_1", "user")]);

        fold(
            &mut items,
            &mut index,
            &roles,
            &text_part("p1", "msg_1", "look at this"),
        );
        fold(
            &mut items,
            &mut index,
            &roles,
            &file_part("p2", "msg_1", "shot.jpg"),
        );

        assert_eq!(items.len(), 1);
        assert_eq!(attachments_of(&items[0]).len(), 1);
        assert_eq!(attachments_of(&items[0])[0].name, "shot.jpg");
        assert_eq!(index.get("p2"), Some(&0));
    }

    /// The optimistic bubble already shows what was just sent, and the server
    /// echoes the same attachment back under an id of its own.
    #[test]
    fn the_echo_of_an_attachment_is_not_a_second_copy_of_it() {
        let mut items = vec![ChatItem::User {
            text: "look".to_owned(),
            attachments: vec![crate::attach::Attachment {
                name: "shot.jpg".to_owned(),
                mime: "image/jpeg".to_owned(),
                size: 3,
                thumb: "THUMB".to_owned(),
            }],
        }];
        let mut index = HashMap::new();
        let roles = roles_of(&[("msg_1", "user")]);

        fold(
            &mut items,
            &mut index,
            &roles,
            &text_part("p1", "msg_1", "look"),
        );
        fold(
            &mut items,
            &mut index,
            &roles,
            &file_part("p2", "msg_1", "shot.jpg"),
        );

        assert_eq!(items.len(), 1);
        assert_eq!(attachments_of(&items[0]).len(), 1);
        assert_eq!(
            attachments_of(&items[0])[0].thumb,
            "THUMB",
            "the phone's own thumbnail is the one that stays"
        );
    }

    /// Attaching a text file makes the server expand it: it persists a fake
    /// "Called the Read tool…" line and the entire decoded file as two more
    /// text parts on the reader's own message. Rendering those puts the whole
    /// attachment into the transcript — and into the cache on disk — as
    /// bubbles the reader never wrote.
    #[test]
    fn the_server_inlining_a_text_attachment_adds_no_bubbles() {
        let mut items = vec![optimistic("look at this", "notes.md", "text/markdown")];
        let mut index = HashMap::new();
        let roles = roles_of(&[("msg_1", "user")]);

        for part in [
            text_part("p1", "msg_1", "look at this"),
            synthetic_part(
                "p2",
                "msg_1",
                r#"Called the Read tool with the following input: {"filePath":"notes.md"}"#,
            ),
            synthetic_part("p3", "msg_1", "# notes\n\nthe whole file, decoded"),
            text_file_part("p4", "msg_1", "notes.md"),
        ] {
            fold(&mut items, &mut index, &roles, &part);
        }

        assert_eq!(items.len(), 1, "one message, one bubble");
        let ChatItem::User { text, .. } = &items[0] else {
            unreachable!()
        };
        assert_eq!(text, "look at this");
        assert_eq!(attachments_of(&items[0]).len(), 1);
    }

    /// The same file goes out declared `text/plain` and comes back that way,
    /// while the record in the transcript keeps the type the picker reported.
    /// Deciding they are different files drew every `.md`, `.csv` and `.json`
    /// attachment twice.
    #[test]
    fn the_echo_of_a_text_attachment_is_not_a_second_chip() {
        let mut items = vec![optimistic("read this", "notes.md", "text/markdown")];
        let mut index = HashMap::new();
        let roles = roles_of(&[("msg_1", "user")]);

        fold(
            &mut items,
            &mut index,
            &roles,
            &text_file_part("p1", "msg_1", "notes.md"),
        );

        let names: Vec<&str> = attachments_of(&items[0])
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["notes.md"]);
    }

    /// A file part on an assistant message belongs to whatever the agent was
    /// doing, not to a turn the reader never wrote.
    #[test]
    fn an_agents_own_file_part_makes_no_user_bubble() {
        let mut items = Vec::new();
        let mut index = HashMap::new();
        let roles = roles_of(&[("msg_1", "assistant")]);

        fold(
            &mut items,
            &mut index,
            &roles,
            &file_part("p1", "msg_1", "chart.png"),
        );

        assert!(items.is_empty());
    }

    /// The prompt is shown the instant it is sent, and the server streams the
    /// same message back a moment later. Both have to end up as one bubble.
    #[test]
    fn server_echo_adopts_the_optimistic_prompt() {
        let mut items = vec![ChatItem::User {
            text: "refactor the folding".to_owned(),
            attachments: Vec::new(),
        }];
        let mut part_index = HashMap::new();
        let mut roles = HashMap::new();
        roles.insert("msg_1".to_owned(), "user".to_owned());
        let (mut marks, mut last_at) = (Vec::new(), 0);

        fold_part_into(
            &mut items,
            &mut part_index,
            &roles,
            &text_part("prt_1", "msg_1", "refactor the folding"),
            None,
            &mut GapSink {
                marks: &mut marks,
                last_at: &mut last_at,
            },
        );

        assert_eq!(items.len(), 1, "the echo should not add a second bubble");
        assert_eq!(
            part_index.get("prt_1"),
            Some(&0),
            "echo bound to the bubble"
        );
    }

    /// Sending the same text twice is two prompts, not one echoed twice: the
    /// second optimistic bubble is the one the second echo may adopt, and the
    /// first must be left alone.
    #[test]
    fn the_same_prompt_twice_stays_two_bubbles() {
        let mut items = Vec::new();
        let mut part_index = HashMap::new();
        let mut roles = HashMap::new();
        roles.insert("msg_1".to_owned(), "user".to_owned());
        roles.insert("msg_2".to_owned(), "user".to_owned());
        let (mut marks, mut last_at) = (Vec::new(), 0);

        for (n, msg) in [("prt_1", "msg_1"), ("prt_2", "msg_2")] {
            items.push(ChatItem::User {
                text: "again".to_owned(),
                attachments: Vec::new(),
            });
            fold_part_into(
                &mut items,
                &mut part_index,
                &roles,
                &text_part(n, msg, "again"),
                None,
                &mut GapSink {
                    marks: &mut marks,
                    last_at: &mut last_at,
                },
            );
        }

        assert_eq!(items.len(), 2);
        assert_eq!(part_index.get("prt_1"), Some(&0));
        assert_eq!(part_index.get("prt_2"), Some(&1));
    }

    /// An assistant reply after the prompt is a new item, not an adoption —
    /// the guard keys on role, and must not swallow the answer.
    #[test]
    fn an_assistant_reply_is_still_its_own_item() {
        let mut items = vec![ChatItem::User {
            text: "hello".to_owned(),
            attachments: Vec::new(),
        }];
        let mut part_index = HashMap::new();
        let mut roles = HashMap::new();
        roles.insert("msg_2".to_owned(), "assistant".to_owned());
        let (mut marks, mut last_at) = (Vec::new(), 0);

        fold_part_into(
            &mut items,
            &mut part_index,
            &roles,
            &text_part("prt_2", "msg_2", "hello"),
            None,
            &mut GapSink {
                marks: &mut marks,
                last_at: &mut last_at,
            },
        );

        assert_eq!(items.len(), 2);
        assert!(matches!(items[1], ChatItem::Assistant { .. }));
    }

    // ------------------------------------------------- the pending aggregate

    fn ask(chat: &str, id: &str) -> PendingAsk {
        PendingAsk {
            chat_id: chat.to_owned(),
            permission: CodePermission {
                id: id.to_owned(),
                session_id: "ses_1".to_owned(),
                title: "Run git push".to_owned(),
                kind: "bash".to_owned(),
                ..CodePermission::default()
            },
        }
    }

    fn report(permissions: Vec<PendingAsk>, unreachable: &[&str]) -> PermissionReport {
        PermissionReport {
            permissions,
            unreachable: unreachable.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn ids(queue: &[(String, CodePermission)]) -> Vec<(&str, &str)> {
        queue
            .iter()
            .map(|(c, p)| (c.as_str(), p.id.as_str()))
            .collect()
    }

    /// The point of the whole aggregate: a chat you have never opened, whose
    /// container is up and parked on an ask, says so without being visited.
    #[test]
    fn an_ask_from_a_chat_you_have_not_opened_reaches_the_queue() {
        let mut queue = Vec::new();
        let mut answered = HashSet::new();
        merge_permission_report(
            &mut queue,
            &mut answered,
            &report(vec![ask("chat_a", "per_1")], &[]),
            None,
        );
        assert_eq!(ids(&queue), [("chat_a", "per_1")]);
    }

    /// An ask the server no longer reports has been answered somewhere else,
    /// or its turn died with it. Either way the card must not linger.
    #[test]
    fn an_ask_the_snapshot_drops_leaves_the_queue() {
        let mut queue = vec![("chat_a".to_owned(), ask("chat_a", "per_1").permission)];
        let mut answered = HashSet::new();
        merge_permission_report(&mut queue, &mut answered, &report(Vec::new(), &[]), None);
        assert!(queue.is_empty());
    }

    /// One container that will not answer must not be able to speak for the
    /// others — nor be spoken for. Its ask stays; the rest reconcile.
    #[test]
    fn a_container_that_did_not_answer_keeps_its_ask() {
        let mut queue = vec![
            ("chat_a".to_owned(), ask("chat_a", "per_1").permission),
            ("chat_b".to_owned(), ask("chat_b", "per_2").permission),
        ];
        let mut answered = HashSet::new();
        merge_permission_report(
            &mut queue,
            &mut answered,
            &report(Vec::new(), &["chat_b"]),
            None,
        );
        assert_eq!(ids(&queue), [("chat_b", "per_2")]);
    }

    /// A snapshot taken a moment before the reply landed still lists the ask.
    /// Merging that would put the panel back under the thumb that dismissed
    /// it, and the second tap would 404.
    #[test]
    fn an_ask_answered_here_is_not_re_added_by_a_stale_snapshot() {
        let mut queue = Vec::new();
        let mut answered = HashSet::from([("chat_a".to_owned(), "per_1".to_owned())]);
        merge_permission_report(
            &mut queue,
            &mut answered,
            &report(vec![ask("chat_a", "per_1")], &[]),
            None,
        );
        assert!(
            queue.is_empty(),
            "the tombstone holds while the server lags"
        );
        assert_eq!(answered.len(), 1, "and holds until the server agrees");
    }

    /// Once the server stops reporting it the tombstone has done its job.
    /// Keeping it would leak, and would swallow an id a rebuilt container
    /// happened to reuse.
    #[test]
    fn a_tombstone_clears_once_the_server_agrees() {
        let mut queue = Vec::new();
        let mut answered = HashSet::from([("chat_a".to_owned(), "per_1".to_owned())]);
        merge_permission_report(&mut queue, &mut answered, &report(Vec::new(), &[]), None);
        assert!(answered.is_empty());
    }

    /// A chat with a live stream is that stream's. A snapshot older than an
    /// ask that has just streamed in would otherwise blink it out of the
    /// modal, and a snapshot older than a reply would double it up.
    #[test]
    fn the_chat_with_a_live_stream_is_left_to_it() {
        let mut queue = vec![("chat_a".to_owned(), ask("chat_a", "per_live").permission)];
        let mut answered = HashSet::new();
        merge_permission_report(
            &mut queue,
            &mut answered,
            &report(vec![ask("chat_a", "per_old")], &[]),
            Some("chat_a"),
        );
        assert_eq!(
            ids(&queue),
            [("chat_a", "per_live")],
            "neither dropped nor added behind the stream's back"
        );
    }

    /// Exactly the streaming chat is exempt, and no other. The chat you last
    /// visited keeps its id in `code_chat` for the rest of the session, and
    /// excluding *that* left the chat most likely to be blocked as the one
    /// chat nothing was allowed to report: its stream can end for good, and
    /// the aggregate was still standing back for it.
    #[test]
    fn a_chat_with_no_stream_of_its_own_is_reported_by_the_aggregate() {
        let mut queue = Vec::new();
        let mut answered = HashSet::new();
        merge_permission_report(
            &mut queue,
            &mut answered,
            &report(vec![ask("chat_a", "per_1"), ask("chat_b", "per_2")], &[]),
            Some("chat_b"),
        );
        assert_eq!(ids(&queue), [("chat_a", "per_1")]);
    }

    /// Replying needs the chat in the path, so an entry without one is not an
    /// ask anybody can answer — showing it would be a dead button.
    #[test]
    fn an_entry_with_no_chat_is_ignored() {
        let mut queue = Vec::new();
        let mut answered = HashSet::new();
        merge_permission_report(
            &mut queue,
            &mut answered,
            &report(vec![ask("", "per_1"), ask("chat_a", "")], &[]),
            None,
        );
        assert!(queue.is_empty());
    }

    /// The aggregate runs every ten seconds against a queue the event stream
    /// is also writing to; a second sighting of the same ask is the normal
    /// case, not a second ask.
    #[test]
    fn a_repeated_snapshot_does_not_duplicate_a_card() {
        let mut queue = Vec::new();
        let mut answered = HashSet::new();
        let snapshot = report(vec![ask("chat_a", "per_1")], &[]);
        merge_permission_report(&mut queue, &mut answered, &snapshot, None);
        merge_permission_report(&mut queue, &mut answered, &snapshot, None);
        assert_eq!(ids(&queue), [("chat_a", "per_1")]);
    }

    // ------------------------------------------------------ the poll's life

    /// The loop that carries the aggregate retires for a newer loop and for
    /// nothing else. It used to retire on `code_epoch`, which counts chat
    /// opens — so the first tap on a row stopped the only thing fetching the
    /// asks, and neither caller of `start_code_poll` can fire a second time.
    #[test]
    fn a_poll_loop_is_retired_only_by_a_newer_poll_loop() {
        assert_eq!(poll_tick(1, 1, Tab::Code, true), Tick::Fetch);
        assert_eq!(poll_tick(1, 2, Tab::Code, true), Tick::Retire);
        assert_eq!(
            poll_tick(1, 1, Tab::Home, true),
            Tick::Idle,
            "another tab is a pause, not an exit — the list is still there"
        );
        assert_eq!(
            poll_tick(1, 1, Tab::Code, false),
            Tick::Idle,
            "nothing to ask until there is a client to ask with"
        );
    }

    // ------------------------------------------------------ what a row says

    fn chat_meta(status: &str) -> ChatMeta {
        ChatMeta {
            id: "chat_a".to_owned(),
            repo: "testrepo".to_owned(),
            title: "Wire up the aggregate".to_owned(),
            branch: "agent/testrepo-9f3403".to_owned(),
            base: String::new(),
            status: status.to_owned(),
            model: None,
            last_active: 0.0,
        }
    }

    /// A card cannot say "idle" on one line and "Approve or deny …" on the
    /// next. Nothing in the manager's index reports a live turn on a chat the
    /// app does not have open, so an ask is the only evidence there is that
    /// the container is parked rather than sitting about — and it is enough.
    #[test]
    fn a_chat_parked_on_an_ask_does_not_call_itself_idle() {
        assert_eq!(
            status_label(&chat_meta("running"), false, true),
            ("dot wait", "waiting on you".to_owned())
        );
        assert_eq!(
            status_label(&chat_meta("running"), false, false),
            ("dot on", "idle".to_owned()),
            "and a chat with nothing pending still reports its container"
        );
        // The two facts come from two fetches a tick apart, so the index can
        // still say "stopped" for a container that has just raised an ask.
        // The ask is the fresher of the two and the one that wants an answer.
        assert_eq!(
            status_label(&chat_meta("stopped"), false, true),
            ("dot wait", "waiting on you".to_owned())
        );
    }
}
