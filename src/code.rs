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
    ChatMeta, CodeClient, CodeConfig, CodeEvent, CodePermission, FileDiff, MessageWithParts, Part,
    PermissionReport,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::{DiffLine, Gap};
use crate::state::{show_toast, AppCtx, ChatItem, ConnState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeScreen {
    List,
    New,
    Chat,
    /// Reviewing the session's changes. Its own screen rather than a panel in
    /// the transcript: the thing being reviewed is a whole working tree, and
    /// a review has its own navigation, its own chrome and its own state.
    Diff,
}

/// Everything the code chat screen renders.
#[derive(Clone, PartialEq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these four are independent facts about the screen, not a state \
              machine to collapse into an enum: a chat can be waking AND \
              loading at once, running is orthogonal to both, and picked is \
              about where the model came from rather than about the chat's \
              lifecycle at all"
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
    // whatever it had, which for the open chat is the live event stream's.
    let Ok(report) = client.pending_permissions().await else {
        return;
    };
    let open = ctx.code_chat.peek().chat_id.clone();
    let mut queue = ctx.code_permissions;
    let mut answered = ctx.code_answered;
    merge_permission_report(
        &mut queue.write(),
        &mut answered.write(),
        &report,
        open.as_deref(),
    );
}

/// Reconcile a snapshot of what is pending server-side with what is queued.
///
/// The snapshot is authoritative for every chat it can speak for, which is
/// every chat except two. The chat that is **open** is left to its own event
/// stream: that stream is both faster and ordered, and a snapshot taken
/// before an ask arrived would otherwise blink it out of the modal for a
/// poll interval. A chat the manager lists as **unreachable** is one whose
/// container did not answer, which is not the same as a container with
/// nothing pending — dropping its ask on that would be inventing an answer.
fn merge_permission_report(
    queue: &mut Vec<(String, CodePermission)>,
    answered: &mut HashSet<(String, String)>,
    report: &PermissionReport,
    open_chat: Option<&str>,
) {
    let speaks_for =
        |chat: &str| open_chat != Some(chat) && !report.unreachable.iter().any(|c| c == chat);
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
        if chat.is_empty() || perm.id.is_empty() || open_chat == Some(chat.as_str()) {
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

/// Keep the chat list fresh while the Code tab is visible. One loop per
/// epoch; a tab switch away lets it park (cheap no-op ticks).
///
/// The pending-ask aggregate rides the same tick rather than getting a timer
/// of its own: it is the other half of "what is this list doing right now",
/// and two loops on the same cadence would only mean two ways for the list to
/// be internally inconsistent.
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
            refresh_code_permissions(&ctx).await;
        }
    });
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

/// Whether the open chat's repo is flagged a public throwaway — the one case
/// where a model that trains on its input is allowed to see the code.
pub(crate) fn open_chat_allows_free_models(ctx: &AppCtx) -> bool {
    let repo = ctx.code_chat.peek().repo.clone();
    ctx.code_repos
        .peek()
        .iter()
        .any(|r| r.name == repo && r.public_throwaway)
}

/// Fetch the chat server's model catalogue, once, on first need.
///
/// Deliberately not part of opening a chat: it is every model of every
/// provider and nothing outside the settings sheet reads it, so it is paid
/// for when that sheet is opened and not before.
pub(crate) fn ensure_code_models(ctx: &AppCtx) {
    if !ctx.code_models.peek().is_empty() || *ctx.code_models_loading.peek() {
        return;
    }
    let Some(chat_id) = ctx.code_chat.peek().chat_id.clone() else {
        return;
    };
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
    // follow you into a different conversation.
    if ctx.code_chat.peek().chat_id.as_deref() != Some(meta.id.as_str()) {
        ctx.code_draft.clone().set(String::new());
    }

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
        picked: false,
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
    if let Some(model) = list
        .iter()
        .find(|s| Some(&s.id) == session_id.as_ref())
        .and_then(|s| s.model.as_ref())
    {
        let mut c = chat.write();
        if !c.picked {
            if let Some(reference) = model.reference() {
                c.model = Some(reference);
            }
            c.effort = model.effort().map(str::to_owned);
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
    refresh_diff_counts(ctx);

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
            // The prompt goes into the transcript the moment it is sent, so
            // the bubble does not wait on a round trip. The server then
            // streams the same message back as a part of its own, and with
            // nothing matching the two up every code session opened with the
            // task written out twice. Adopt the optimistic bubble instead:
            // it is always the last item, and its text is exactly what was
            // sent.
            if role == "user" {
                if let Some(idx) = items.len().checked_sub(1) {
                    let same =
                        matches!(items.get(idx), Some(ChatItem::User { text: t }) if *t == text);
                    if same && !part_index.values().any(|&i| i == idx) {
                        part_index.insert(part.id.clone(), idx);
                        return;
                    }
                }
            }
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
        // Model and effort ride on the turn: OpenCode has no "set the
        // session's model" call, it copies whatever the turn asked for onto
        // the session record. That is why the sheet says "from your next
        // message" and means it.
        let (model, effort) = {
            let c = ctx.code_chat.peek();
            (c.model.clone(), c.effort.clone())
        };
        if let Err(e) = client
            .prompt_async(&chat_id, &sid, &text, model.as_deref(), effort.as_deref())
            .await
        {
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

/// Whether the chat that is open has an ask outstanding.
///
/// That one belongs to the modal, which is how a permission has always been
/// answered from inside a conversation. Every other chat's belongs to its
/// card in the list — a modal thrown over the screen about a chat you are not
/// in is the aggregate shouting rather than reporting.
pub(crate) fn open_chat_has_ask(ctx: &AppCtx) -> bool {
    // peek on the chat, read on the queue: this decides whether the root
    // renders the modal, and subscribing to `code_chat` would re-run the
    // whole app on every streamed token.
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
pub(crate) fn status_label(meta: &ChatMeta, running_turn: bool) -> (&'static str, String) {
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
        fold_part_into, merge_permission_report, ChatItem, CodePermission, GapSink, HashMap,
        HashSet, PermissionReport,
    };
    use opencode_client::{Part, PendingAsk};

    fn text_part(id: &str, message_id: &str, text: &str) -> Part {
        Part {
            id: id.to_owned(),
            message_id: message_id.to_owned(),
            session_id: "ses_1".to_owned(),
            kind: "text".to_owned(),
            text: Some(text.to_owned()),
            tool: None,
            call_id: None,
            state: None,
        }
    }

    /// The prompt is shown the instant it is sent, and the server streams the
    /// same message back a moment later. Both have to end up as one bubble.
    #[test]
    fn server_echo_adopts_the_optimistic_prompt() {
        let mut items = vec![ChatItem::User {
            text: "refactor the folding".to_owned(),
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

    /// The open chat is the event stream's. A snapshot older than an ask that
    /// has just streamed in would otherwise blink it out of the modal, and a
    /// snapshot older than a reply would double it up.
    #[test]
    fn the_open_chat_is_left_to_its_event_stream() {
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
}
