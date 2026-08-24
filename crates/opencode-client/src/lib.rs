//! Client for the brain's code-agent plane (personal-ai-setup issue #17,
//! this repo issue #2): the session manager's API plus the per-chat `OpenCode`
//! servers it fronts, all behind one TLS + Basic-auth gateway on the tailnet.
//!
//! Two layers, one base URL:
//! - manager: `/api/...` — chat lifecycle + the metadata index
//! - per chat: `/chat/<id>/...` — that chat's own `opencode serve` HTTP
//!   API (sessions, messages, prompts,
//!   permissions, diff) + SSE at `/event`
//!
//! The gateway wakes a stopped chat on any request to it, so this client
//! never needs explicit wake logic beyond tolerating slow first responses.
//!
//! UI-framework agnostic: reqwest (rustls) + tokio, same stack as
//! `goose-acp-client`. The gateway's certificate is a real Let's Encrypt
//! cert on the tailnet name, so standard verification applies — no pinning.

use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("server said {status}: {body}")]
    Status { status: u16, body: String },
    #[error("{0}")]
    Other(String),
}

impl CodeError {
    /// The sentence to put in front of a reader.
    ///
    /// The manager answers a refused request with `{"error": "..."}` and that
    /// sentence is the only part worth showing — `Display` wraps it in the
    /// status code and the braces it arrived in, which is a stack trace in a
    /// toast. Falls back to the status when the body is not one of those.
    #[must_use]
    pub fn message(&self) -> String {
        let Self::Status { status, body } = self else {
            return self.to_string();
        };
        serde_json::from_str::<Value>(body)
            .ok()
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map_or_else(|| format!("server said {status}"), str::to_owned)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CodeConfig {
    /// Gateway base, e.g. `https://brain.tailnet.ts.net:4300`.
    pub base_url: String,
    /// `OPENCODE_SERVER_PASSWORD` (username is free-form; we send `opencode`).
    pub password: String,
}

/// One repo from the manager's allowlist.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub edit_only: bool,
    #[serde(default)]
    pub allow_push: bool,
    #[serde(default)]
    pub public_throwaway: bool,
}

/// One chat from the manager's metadata index.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ChatMeta {
    pub id: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub branch: String,
    /// `running` | `stopped` | `absent` (absent = recreated on next wake).
    #[serde(default)]
    pub status: String,
    /// `provider/model` the chat was created with, if one was named. The
    /// manager has always sent this; it is what the settings sheet can show
    /// before the chat's container is awake enough to have a session.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub last_active: f64,
}

impl ChatMeta {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
}

/// What happened to a file in a session's diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FileStatus {
    Added,
    Deleted,
    #[default]
    Modified,
}

/// One file's entry in `GET /session/:id/diff` — `OpenCode`'s
/// `SnapshotFileDiff` (`packages/schema/src/file-diff.ts`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
pub struct FileDiff {
    /// Repo-relative path. Upstream names the field `file`; the mock
    /// `OpenCode` server in personal-ai-setup names it `path`, hence the
    /// alias.
    #[serde(default, alias = "path")]
    pub file: String,
    /// Unified patch, as `jsdiff`'s `formatPatch` writes it: a four-line
    /// `Index:` preamble and then **one** `@@` hunk carrying the *whole*
    /// file, because `Snapshot.diffFull` asks for
    /// `context: Number.MAX_SAFE_INTEGER`. So a three-line change in a
    /// 1200-line file arrives as 1204 lines, and anything rendering this has
    /// to re-hunk it (`src/diff.rs` in the app). Empty for a binary file.
    #[serde(default)]
    pub patch: String,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    /// Absent or unrecognised reads as [`FileStatus::Modified`] rather than
    /// failing the whole entry — a status this client has not heard of is
    /// still a file worth showing.
    #[serde(default, deserialize_with = "de_file_status")]
    pub status: FileStatus,
}

fn de_file_status<'de, D: serde::Deserializer<'de>>(d: D) -> Result<FileStatus, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    Ok(match raw.as_deref() {
        Some("added") => FileStatus::Added,
        Some("deleted") => FileStatus::Deleted,
        _ => FileStatus::Modified,
    })
}

impl FileDiff {
    /// A binary file: the server sends no patch and no counts for one.
    #[must_use]
    pub fn is_binary(&self) -> bool {
        self.patch.trim().is_empty()
    }
}

/// Where a pull request stands. `draft` is a flag on an open pull request
/// rather than a fourth state, because a draft that gets closed is both.
///
/// [`PullState::Unknown`] is this client's, not the wire's: a state string it
/// has not heard of must not read as open, because "open" is half of what
/// decides whether the app offers to merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PullState {
    Open,
    Merged,
    Closed,
    #[default]
    Unknown,
}

/// What the head commit's checks add up to.
///
/// [`Checks::Unknown`] is a real answer, not a parse failure. The manager's
/// GitHub credential is a fine-grained PAT with Contents and Pull requests
/// only (personal-ai-setup `docs/setup/70-code-agents.md`), and check runs
/// need `Checks: read` while commit statuses need `Commit statuses: read` —
/// so on a private repo the manager is told 403 and says so rather than
/// guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Checks {
    Passing,
    Failing,
    Pending,
    /// Nothing runs checks on this repo.
    None,
    #[default]
    Unknown,
}

/// One pull request from `GET /api/chats/<id>/pulls`.
///
/// The manager answers with the pull requests whose head branch is **this
/// chat's** branch (`agent/<chat-id>`) and no others, which is what makes a
/// count on a chip meaningful — a repo's other pull requests have nothing to
/// do with this conversation.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(deserialize_with = "de_pull_state")]
    pub state: PullState,
    pub draft: bool,
    /// GitHub computes mergeability asynchronously and answers `null` until it
    /// has. That is not the same as "cannot be merged", and the two must not
    /// collapse into one bool: one is a wait, the other is a refusal.
    pub mergeable: Option<bool>,
    #[serde(deserialize_with = "de_checks")]
    pub checks: Checks,
    /// The `html_url` — where the system browser is sent.
    pub url: String,
    /// Head branch, always this chat's `agent/<chat-id>`.
    pub head: String,
    /// Base branch the merge would land on.
    pub base: String,
    pub created_at: String,
    pub updated_at: String,
}

fn de_pull_state<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PullState, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    Ok(match raw.as_deref() {
        Some("open") => PullState::Open,
        Some("merged") => PullState::Merged,
        Some("closed") => PullState::Closed,
        _ => PullState::Unknown,
    })
}

fn de_checks<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Checks, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    Ok(match raw.as_deref() {
        Some("passing") => Checks::Passing,
        Some("failing") => Checks::Failing,
        Some("pending") => Checks::Pending,
        Some("none") => Checks::None,
        _ => Checks::Unknown,
    })
}

impl PullRequest {
    /// Whether merging should be offered: GitHub says it can be merged, it is
    /// open, it is not a draft, and its checks are not failing.
    ///
    /// Pending checks are not a refusal — they are checks that have not
    /// answered — so this says yes and the confirm says they are still
    /// running. Unknown ones are the credential case above; GitHub's own
    /// branch protection is the backstop, and it refuses the merge itself.
    #[must_use]
    pub const fn is_mergeable(&self) -> bool {
        matches!(self.state, PullState::Open)
            && !self.draft
            && matches!(self.mergeable, Some(true))
            && !matches!(self.checks, Checks::Failing)
    }
}

/// What `POST /api/chats/<id>/pulls/<n>/merge` answers on success.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct MergeOutcome {
    pub merged: bool,
    /// The merge commit GitHub made.
    pub sha: String,
    /// The pull request re-read after the merge. Present in the contract, and
    /// optional here so that a manager which omits it cannot turn a merge that
    /// happened into an error — the caller refetches instead.
    pub pull: Option<PullRequest>,
}

/// Decode `{"pulls": [...]}`, entry by entry.
///
/// An entry this client cannot make sense of is dropped on its own rather
/// than collapsing the answer to "no pull requests" — and it would be
/// unrenderable anyway, having neither a number nor a URL.
fn parse_pulls(body: &Value) -> Vec<PullRequest> {
    body.get("pulls")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// A pending permission ask from a chat's `OpenCode` server. Answer with
/// [`CodeClient::reply_permission`] using `once` / `always` / `reject`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodePermission {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub metadata: Value,
}

impl Default for CodePermission {
    fn default() -> Self {
        Self {
            id: String::new(),
            session_id: String::new(),
            title: String::new(),
            kind: String::new(),
            metadata: Value::Null,
        }
    }
}

/// One entry of the manager's pending-ask aggregate: a [`CodePermission`]
/// with the chat it belongs to spliced in beside it.
///
/// Flattened rather than nested because that is what the manager produces —
/// a chat's own `/permission` payload with one field added — and because an
/// entry is useless without the chat: answering needs `/chat/<id>/…`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct PendingAsk {
    #[serde(rename = "chatId")]
    pub chat_id: String,
    #[serde(flatten)]
    pub permission: CodePermission,
}

/// The manager's answer to "which chats are blocked waiting on me".
///
/// Only chats whose container is **already running** are in it — a stopped
/// container has no live turn, so it cannot be waiting on anything, and
/// asking it would wake it (see [`CodeClient::pending_permissions`]).
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct PermissionReport {
    pub permissions: Vec<PendingAsk>,
    /// Running chats that did not answer in time. The aggregate still
    /// succeeds without them, and a caller must read their absence from
    /// `permissions` as "unknown" rather than as "nothing pending" — which is
    /// the whole reason this list is on the wire instead of being swallowed.
    pub unreachable: Vec<String>,
}

/// One message part as `OpenCode`'s API ships it. Kept lenient: only the
/// fields the transcript fold needs are typed, everything else stays raw.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Part {
    pub id: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    /// The server wrote this part on the reader's behalf: the scaffolding it
    /// wraps a `text/plain` attachment in, the note it leaves when it
    /// compacts a session, the line it records when the user ran a tool.
    /// They are the model's context rather than anything anybody said, and
    /// `OpenCode`'s own UI does not draw them either.
    pub synthetic: bool,
    pub tool: Option<String>,
    #[serde(rename = "callID")]
    pub call_id: Option<String>,
    pub state: Option<Value>,
    // `FilePart`: what an attachment looks like coming back out of history.
    // `url` is whatever the client sent — for this app, a `data:` URI, which
    // is how a re-opened chat gets its thumbnails back without a second
    // fetch.
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub url: Option<String>,
}

impl Part {
    /// The base64 payload of a `data:` URL, if that is what `url` is.
    ///
    /// Deliberately narrow: a `file:` URL names a path inside the chat's
    /// container, which this device cannot read, and an `http:` one would be
    /// a fetch the transcript has no business making.
    #[must_use]
    pub fn data_url_base64(&self) -> Option<&str> {
        let url = self.url.as_deref()?;
        let rest = url.strip_prefix("data:")?;
        let (meta, payload) = rest.split_once(',')?;
        meta.ends_with(";base64").then_some(payload)
    }
}

/// One entry of the `parts` array a prompt body carries — `OpenCode`'s
/// `TextPartInput | FilePartInput` (`packages/sdk/js/src/gen/types.gen.ts`).
///
/// `FilePartInput` is `{id?, type: "file", mime, filename?, url, source?}`,
/// and `url` really is a URL rather than a payload field: the server switches
/// on its protocol. A `data:` URI is the form a phone can use, because
/// `file:` would name a path on the container rather than on this device.
/// `source` is for a citation into a file the agent already has and has no
/// meaning for something uploaded, so it is not modelled here.
///
/// One consequence worth knowing about the server side: a `data:` part whose
/// mime is exactly `text/plain` is decoded and inlined into the conversation
/// as text, while any other mime is passed through to the model as an
/// attachment. That is why [`Self::text_file`] exists — a `.md` sent as
/// `text/markdown` reaches a model that cannot read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PromptPart {
    Text {
        text: String,
    },
    File {
        mime: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        url: String,
    },
}

impl PromptPart {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// A file part carrying its bytes inline. `data` is base64.
    #[must_use]
    pub fn file(mime: &str, filename: &str, data: &str) -> Self {
        Self::File {
            mime: mime.to_owned(),
            filename: Some(filename.to_owned()),
            url: format!("data:{mime};base64,{data}"),
        }
    }

    /// A text file, declared as `text/plain` whatever it is really called, so
    /// the server inlines its contents instead of handing the model a blob it
    /// has no decoder for. `data` is base64.
    ///
    /// The inlining is not free and not invisible: the server rewrites this
    /// one part into three persisted parts — a `Called the Read tool…` line,
    /// the whole decoded file, and the file part itself — with the first two
    /// flagged [`Part::synthetic`]. A client that renders every part it is
    /// given will therefore print the attachment's entire contents back at
    /// the reader as if they had typed it.
    #[must_use]
    pub fn text_file(filename: &str, data: &str) -> Self {
        Self::file("text/plain", filename, data)
    }
}

/// `{info, parts}` from `GET /session/:id/message`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MessageWithParts {
    pub info: MessageInfo,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
}

/// A session on a chat's server (`GET /session`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub directory: String,
    /// What the session is currently set to. `OpenCode` writes this on every
    /// turn whose model or variant differs from the record, so it is the
    /// server's own answer to "what will the next message use".
    pub model: Option<SessionModel>,
}

/// The model a session is set to, as `Session.model` ships it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SessionModel {
    /// Model id within its provider, e.g. `deepseek-v4-flash`.
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    /// Thinking-effort tier, `OpenCode`'s "variant". Recorded as the literal
    /// string `default` when the turn asked for none.
    pub variant: Option<String>,
}

impl SessionModel {
    /// `provider/model` — the one form the manager, the composer and the
    /// prompt body all speak.
    #[must_use]
    pub fn reference(&self) -> Option<String> {
        (!self.id.is_empty() && !self.provider_id.is_empty())
            .then(|| format!("{}/{}", self.provider_id, self.id))
    }

    /// The variant that is an actual choice. `default` is not one: it is how
    /// `OpenCode` records "no variant was asked for".
    #[must_use]
    pub fn effort(&self) -> Option<&str> {
        self.variant
            .as_deref()
            .filter(|v| !v.is_empty() && *v != "default")
    }
}

/// One model in a chat server's catalogue (`Provider.models[id]`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModelInfo {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    pub name: String,
    pub limit: ModelLimit,
    /// Thinking-effort tiers this model accepts, keyed by variant name. It is
    /// legitimately empty — `OpenCode` returns no variants at all for the
    /// minimax / qwen / glm / kimi / deepseek-v3 families, which includes the
    /// template's default small model.
    pub variants: std::collections::BTreeMap<String, Value>,
}

impl ModelInfo {
    /// `provider/model`, matching what `create_chat` and `prompt_async` take.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}/{}", self.provider_id, self.id)
    }

    /// Effort tiers, weakest first.
    ///
    /// The wire shape is a JSON object and `serde_json`'s map is sorted, so
    /// the server's own ordering is gone by the time this decodes — and
    /// alphabetical would put `high` before `low`. Ordering by the tier
    /// ladder `OpenCode` builds them from restores an order a reader can use;
    /// a name outside the ladder keeps its place at the end rather than
    /// disappearing.
    #[must_use]
    pub fn efforts(&self) -> Vec<&str> {
        const LADDER: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
        let mut names: Vec<&str> = self.variants.keys().map(String::as_str).collect();
        names.sort_by_key(|n| {
            (
                LADDER.iter().position(|l| l == n).unwrap_or(LADDER.len()),
                *n,
            )
        });
        names
    }
}

/// What an agent may be used for — `Agent.mode` in `OpenCode`'s SDK types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentMode {
    /// A main assistant you talk to directly.
    Primary,
    /// Invoked by another agent for a sub-task, never by a person.
    Subagent,
    /// Either way round. `OpenCode`'s default for a definition that does not
    /// say which it is.
    #[default]
    All,
}

/// `mode` is required by the wire schema, so a missing or unrecognised value
/// means a server this client does not fully understand. It reads as `all`
/// rather than as a subagent: only the explicit word `subagent` is a reason
/// to withhold an agent, and guessing the other way would make a new mode
/// name silently empty the picker.
fn de_agent_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AgentMode, D::Error> {
    let raw = Option::<String>::deserialize(d)?;
    Ok(match raw.as_deref() {
        Some("primary") => AgentMode::Primary,
        Some("subagent") => AgentMode::Subagent,
        _ => AgentMode::All,
    })
}

/// One agent a chat's server offers (`GET /agent`).
///
/// `OpenCode` calls this an agent; the app calls it a mode, because that is
/// what it is from the composer — Build and Plan are the two it ships, and
/// they differ in what the turn is allowed to do. It rides a turn exactly the
/// way the model does (`agent` in the prompt body), so switching costs no
/// config rewrite and no server restart.
///
/// Only what the picker renders is typed. Permissions, tools and options stay
/// where they are enforced, which is the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Agent {
    /// The id the prompt body's `agent` field takes, e.g. `build`.
    pub name: String,
    /// The agent's own one-line account of when to use it.
    pub description: Option<String>,
    #[serde(deserialize_with = "de_agent_mode")]
    pub mode: AgentMode,
    /// Shipped with `OpenCode` rather than defined by the repo.
    #[serde(rename = "builtIn")]
    pub built_in: bool,
}

impl Agent {
    /// Whether a session may run on this agent.
    ///
    /// A subagent may not: it exists to be called by another agent for a
    /// sub-task, and pointing a chat at one would be asking for a turn the
    /// server has no primary agent to run.
    #[must_use]
    pub const fn is_primary(&self) -> bool {
        matches!(self.mode, AgentMode::Primary | AgentMode::All)
    }
}

/// A model's declared limits. Read-only catalogue data: nothing in the
/// prompt API takes a context window, and the one route that rewrites it
/// (`PATCH /config`) restarts the chat's server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModelLimit {
    /// Context window in tokens. Typed `f64` because the server's schema
    /// types it as a finite number: a `200000.0` must not make the whole
    /// catalogue fail to decode.
    pub context: f64,
}

impl ModelLimit {
    /// The context window in whole tokens, or `None` if none was declared.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "guarded positive above, and a context window is many orders \
                  of magnitude inside u64"
    )]
    pub fn context_tokens(&self) -> Option<u64> {
        (self.context >= 1.0).then_some(self.context as u64)
    }
}

/// `GET /config/providers` and `GET /provider` differ only in the key their
/// provider array hangs off; both carry the same `Provider` objects.
#[derive(Debug, Deserialize)]
struct ProviderCatalog {
    #[serde(default)]
    providers: Vec<ProviderEntry>,
    #[serde(default)]
    all: Vec<ProviderEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProviderEntry {
    id: String,
    models: std::collections::BTreeMap<String, ModelInfo>,
}

impl ProviderCatalog {
    /// Every model, flattened, with each entry's `providerID` filled in from
    /// its provider when the server left it off.
    fn models(self) -> Vec<ModelInfo> {
        let mut out: Vec<ModelInfo> = self
            .providers
            .into_iter()
            .chain(self.all)
            .flat_map(|p| {
                let provider_id = p.id;
                p.models.into_iter().map(move |(key, mut m)| {
                    if m.id.is_empty() {
                        m.id = key;
                    }
                    if m.provider_id.is_empty() {
                        m.provider_id.clone_from(&provider_id);
                    }
                    if m.name.is_empty() {
                        m.name.clone_from(&m.id);
                    }
                    m
                })
            })
            .collect();
        // Both keys are read because a build answers on one route or the
        // other, but a build that fills in both would otherwise hand back
        // every model twice — and two entries with the same reference are
        // indistinguishable in a picker, so there is nothing downstream can
        // do about it.
        out.sort_by(|a, b| {
            a.provider_id
                .cmp(&b.provider_id)
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.name.cmp(&b.name))
        });
        out.dedup_by(|a, b| a.provider_id == b.provider_id && a.id == b.id);
        out.sort_by(|a, b| {
            a.provider_id
                .cmp(&b.provider_id)
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }
}

/// Events off a chat's SSE stream, pre-dispatched on the `type` tag.
/// Unknown kinds are preserved so new server events degrade gracefully.
#[derive(Clone, Debug)]
pub enum CodeEvent {
    MessageUpdated {
        info: MessageInfo,
    },
    PartUpdated {
        part: Part,
        delta: Option<String>,
    },
    PermissionAsked(CodePermission),
    PermissionReplied {
        id: String,
    },
    SessionIdle {
        session_id: String,
    },
    SessionStatus(Value),
    Connected,
    Unknown {
        tag: String,
        raw: Value,
    },
    /// The stream ended (network drop, chat spin-down, gateway restart).
    Disconnected {
        reason: String,
    },
}

fn dispatch_event(raw: Value) -> CodeEvent {
    // Gateway `/global/event` wraps payloads as {directory, payload}; the
    // per-chat `/event` sends them bare. Accept both.
    let evt = raw.get("payload").cloned().unwrap_or(raw);
    let tag = evt
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let props = evt.get("properties").cloned().unwrap_or(Value::Null);
    match tag.as_str() {
        "server.connected" => CodeEvent::Connected,
        "server.heartbeat" => CodeEvent::Unknown {
            tag,
            raw: Value::Null,
        },
        "message.updated" => {
            let info = props
                .get("info")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            CodeEvent::MessageUpdated { info }
        }
        "message.part.updated" => {
            let part = props
                .get("part")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let delta = props
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_string);
            CodeEvent::PartUpdated { part, delta }
        }
        "permission.updated" | "permission.asked" => {
            match serde_json::from_value::<CodePermission>(props.clone()) {
                Ok(p) if !p.id.is_empty() => CodeEvent::PermissionAsked(p),
                _ => CodeEvent::Unknown { tag, raw: props },
            }
        }
        "permission.replied" => CodeEvent::PermissionReplied {
            id: props
                .get("permissionID")
                .or_else(|| props.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "session.idle" => CodeEvent::SessionIdle {
            session_id: props
                .get("sessionID")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "session.status" => CodeEvent::SessionStatus(props),
        _ => CodeEvent::Unknown { tag, raw: evt },
    }
}

#[derive(Clone)]
pub struct CodeClient {
    http: reqwest::Client,
    base: String,
    password: String,
}

/// Hand-written so the gateway password never reaches a log line; the
/// derived form would print it verbatim.
impl std::fmt::Debug for CodeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeClient")
            .field("base", &self.base)
            .field("password", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl CodeClient {
    /// Build a client for the gateway described by `cfg`.
    ///
    /// # Errors
    ///
    /// [`CodeError::Other`] if `cfg.base_url` is blank once trimmed, and
    /// [`CodeError::Http`] if reqwest cannot build its TLS-backed client
    /// (a broken rustls or root-certificate setup on the device).
    pub fn new(cfg: &CodeConfig) -> Result<Self, CodeError> {
        let base = cfg.base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err(CodeError::Other("code server URL is empty".into()));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Generous default: any request may transparently wake a stopped
            // chat (container start + server boot ≈ up to 90s gateway-side).
            .timeout(Duration::from_secs(150))
            .build()?;
        Ok(Self {
            http,
            base,
            password: cfg.password.clone(),
        })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base, path))
            .basic_auth("opencode", Some(&self.password))
    }

    async fn json_of(resp: reqwest::Response) -> Result<Value, CodeError> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let body = body.chars().take(300).collect::<String>();
            return Err(CodeError::Status {
                status: status.as_u16(),
                body,
            });
        }
        if status.as_u16() == 204 {
            return Ok(Value::Null);
        }
        Ok(resp.json().await.unwrap_or(Value::Null))
    }

    // ------------------------------------------------------------ manager

    /// The manager's liveness probe (`GET /api/health`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer — 401 when `password` is wrong, 502 while the manager behind
    /// the gateway is restarting.
    pub async fn health(&self) -> Result<Value, CodeError> {
        Self::json_of(self.req(reqwest::Method::GET, "/api/health").send().await?).await
    }

    /// The manager's repo allowlist (`GET /api/repos`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer (401 for a wrong `password`). A 2xx body that does not decode
    /// as `{"repos": [...]}` yields an empty list rather than an error.
    pub async fn repos(&self) -> Result<Vec<RepoEntry>, CodeError> {
        let v = Self::json_of(self.req(reqwest::Method::GET, "/api/repos").send().await?).await?;
        Ok(serde_json::from_value(v.get("repos").cloned().unwrap_or_default()).unwrap_or_default())
    }

    /// The metadata index of every chat the manager knows (`GET /api/chats`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer (401 for a wrong `password`). A 2xx body that does not decode
    /// as `{"chats": [...]}` yields an empty list rather than an error.
    pub async fn chats(&self) -> Result<Vec<ChatMeta>, CodeError> {
        let v = Self::json_of(self.req(reqwest::Method::GET, "/api/chats").send().await?).await?;
        Ok(serde_json::from_value(v.get("chats").cloned().unwrap_or_default()).unwrap_or_default())
    }

    /// Every pending permission ask across the chats that are **already
    /// running** (`GET /api/permissions`).
    ///
    /// This is a manager-side aggregate, and it has to be: the alternative is
    /// [`Self::permissions`] once per chat, and every `/chat/<id>/…` request
    /// goes through the transparent proxy, which wakes a stopped container.
    /// Polling the list that way would hold every container open and undo the
    /// idle spin-down the whole code plane is built on. Restricting the
    /// aggregate to running containers costs nothing real — a container that
    /// is down has no live turn, so it has nothing parked on an ask.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, [`CodeError::Status`] on a non-2xx
    /// answer — 401 for a wrong `password`, 404 from a manager too old to
    /// have this route — and [`CodeError::Other`] for a 2xx body that is not
    /// the contracted shape. A single container that fails to answer does
    /// **not** fail the aggregate: the manager names it in
    /// [`PermissionReport::unreachable`] and still returns 200.
    pub async fn pending_permissions(&self) -> Result<PermissionReport, CodeError> {
        Self::decode_permission_report(
            Self::json_of(
                self.req(reqwest::Method::GET, "/api/permissions")
                    .send()
                    .await?,
            )
            .await?,
        )
    }

    /// Read the aggregate's body, or say why it could not be read.
    ///
    /// The strictest decode in this client, and the only one that has to be.
    /// Everywhere else a 2xx that does not parse degrades to an empty list
    /// and the screen shows less than it could; here the report is read as
    /// authority over what is *not* pending — the merge clears every card the
    /// report does not list — so `unwrap_or_default()` on a body this client
    /// cannot read announces "nothing is waiting on you anywhere" in a voice
    /// indistinguishable from the truthful answer, and wipes the list. An
    /// error keeps whatever the app already had, which is the safe half of
    /// the ambiguity. `{"permissions": null}` from a manager that meant "none"
    /// lands here, and so does the `Value::Null` [`Self::json_of`] hands back
    /// for a body that stopped mid-read.
    ///
    /// A bare array is refused rather than tolerated: serde derives
    /// struct-from-sequence, so `[]` would otherwise decode as an empty
    /// report — an unrecognised shape that happens to be empty is precisely
    /// the false negative above.
    fn decode_permission_report(body: Value) -> Result<PermissionReport, CodeError> {
        if !body.is_object() {
            return Err(CodeError::Other(
                "bad permission aggregate: expected an object carrying `permissions`".into(),
            ));
        }
        serde_json::from_value(body)
            .map_err(|e| CodeError::Other(format!("bad permission aggregate: {e}")))
    }

    /// Create a chat on `repo` with `task` as its opening instruction.
    /// `model` is `provider/model`; `None` leaves the manager's default.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout,
    /// [`CodeError::Status`] on a non-2xx answer — 400 for a repo outside the
    /// allowlist, 401 for a wrong `password` — and [`CodeError::Other`] if
    /// the created chat's payload does not decode as a [`ChatMeta`].
    pub async fn create_chat(
        &self,
        repo: &str,
        task: &str,
        model: Option<&str>,
    ) -> Result<ChatMeta, CodeError> {
        let mut body = json!({"repo": repo, "task": task});
        if let Some(m) = model {
            body["model"] = json!(m);
        }
        let v = Self::json_of(
            self.req(reqwest::Method::POST, "/api/chats")
                .json(&body)
                .send()
                .await?,
        )
        .await?;
        serde_json::from_value(v).map_err(|e| CodeError::Other(format!("bad chat payload: {e}")))
    }

    /// Start a stopped chat's container. Any request to the chat wakes it
    /// anyway; this is the explicit form, useful to pay the start cost early.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure, or if the container start
    /// outruns the client's 150s request timeout; [`CodeError::Status`] on a
    /// non-2xx answer (404 for a `chat_id` the manager does not know).
    pub async fn wake_chat(&self, chat_id: &str) -> Result<(), CodeError> {
        Self::json_of(
            self.req(reqwest::Method::POST, &format!("/api/chats/{chat_id}/wake"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    /// Stop a running chat's container; its workspace and branch survive.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for a `chat_id` the
    /// manager does not know).
    pub async fn stop_chat(&self, chat_id: &str) -> Result<(), CodeError> {
        Self::json_of(
            self.req(reqwest::Method::POST, &format!("/api/chats/{chat_id}/stop"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    /// Delete a chat. With `purge`, its workspace is discarded too.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for a `chat_id` the
    /// manager does not know).
    pub async fn delete_chat(&self, chat_id: &str, purge: bool) -> Result<(), CodeError> {
        let q = if purge { "?purge=1" } else { "" };
        Self::json_of(
            self.req(reqwest::Method::DELETE, &format!("/api/chats/{chat_id}{q}"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    /// The pull requests **this chat's branch** has produced
    /// (`GET /api/chats/<id>/pulls`), newest first.
    ///
    /// A manager route, not a proxied one: the manager makes these calls to
    /// GitHub itself with its own credential, so asking costs nothing on the
    /// container and — the property the whole feature hangs on — does not wake
    /// a sleeping chat. That is why a chat can carry a pull-request count the
    /// moment it is opened, while the diff has to wait for a container.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer — 404 for a `chat_id` the manager does not know, 409 when the
    /// chat's repo has left the allowlist (there is no clone URL left to ask
    /// GitHub about), 502 when GitHub is unreachable or refuses the
    /// credential. An entry that does not decode is dropped on its own rather
    /// than collapsing the list, the same way [`CodeClient::diff`] does it.
    pub async fn pulls(&self, chat_id: &str) -> Result<Vec<PullRequest>, CodeError> {
        let body = Self::json_of(
            self.req(reqwest::Method::GET, &format!("/api/chats/{chat_id}/pulls"))
                .send()
                .await?,
        )
        .await?;
        Ok(parse_pulls(&body))
    }

    /// Merge one of this chat's pull requests
    /// (`POST /api/chats/<id>/pulls/<n>/merge`).
    ///
    /// The empty body takes the manager's merge method. The manager checks
    /// that `number` is really on this chat's branch before it does anything,
    /// so this cannot be used to merge a pull request that has nothing to do
    /// with the chat whose id is in the path.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer — 404 for an unknown chat or
    /// a pull request that is not on its branch, 409 when the pull request is
    /// not in a state that can be merged, 422 when GitHub itself refused
    /// (branch protection, a required review, a head that moved), 502 when
    /// GitHub is unreachable. Use [`CodeError::message`] for what to show.
    pub async fn merge_pull(&self, chat_id: &str, number: u64) -> Result<MergeOutcome, CodeError> {
        let body = Self::json_of(
            self.req(
                reqwest::Method::POST,
                &format!("/api/chats/{chat_id}/pulls/{number}/merge"),
            )
            .json(&json!({}))
            .send()
            .await?,
        )
        .await?;
        Ok(serde_json::from_value(body).unwrap_or_default())
    }

    // ----------------------------------------------------------- per chat

    fn chat_path(chat_id: &str, sub: &str) -> String {
        format!("/chat/{chat_id}{sub}")
    }

    /// The sessions on a chat's own `OpenCode` server (`GET /session`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`). A 2xx body that does not decode as a session list yields
    /// an empty list rather than an error.
    pub async fn sessions(&self, chat_id: &str) -> Result<Vec<SessionMeta>, CodeError> {
        let v = Self::json_of(
            self.req(reqwest::Method::GET, &Self::chat_path(chat_id, "/session"))
                .send()
                .await?,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Every model a chat's `OpenCode` server can route to, with its context
    /// window and its thinking-effort tiers.
    ///
    /// Two routes carry the same catalogue. `/config/providers` is the older
    /// and more widely deployed of the two, `/provider` the newer; the
    /// container tracks a rolling `:latest` tag, so this asks for the first
    /// and falls back to the second instead of betting on which build is
    /// installed.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] from the *fallback* route when neither answers —
    /// a build that has only one of the two still succeeds.
    pub async fn models(&self, chat_id: &str) -> Result<Vec<ModelInfo>, CodeError> {
        let mut last: Option<CodeError> = None;
        for path in ["/config/providers", "/provider"] {
            let attempt = self
                .req(reqwest::Method::GET, &Self::chat_path(chat_id, path))
                .send()
                .await;
            match attempt {
                Ok(resp) => match Self::json_of(resp).await {
                    Ok(v) => {
                        if let Ok(catalog) = serde_json::from_value::<ProviderCatalog>(v) {
                            let models = catalog.models();
                            if !models.is_empty() {
                                return Ok(models);
                            }
                        }
                    }
                    Err(e) => last = Some(e),
                },
                Err(e) => last = Some(e.into()),
            }
        }
        // Both routes answered but neither held a catalogue: an empty list is
        // the honest report, and the sheet renders it as "none offered".
        last.map_or_else(|| Ok(Vec::new()), Err)
    }

    /// The agents a chat's server offers (`GET /agent`) — what the composer
    /// calls modes.
    ///
    /// Every agent is returned, subagents included; which of them may hold a
    /// session is [`Agent::is_primary`], and that is the caller's filter to
    /// apply. Decoded entry by entry so one definition this client cannot
    /// read drops on its own rather than emptying the picker, and an agent
    /// with no name is discarded because the name *is* what the prompt body
    /// sends.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`, or for an `OpenCode` build predating the route).
    pub async fn agents(&self, chat_id: &str) -> Result<Vec<Agent>, CodeError> {
        let v = Self::json_of(
            self.req(reqwest::Method::GET, &Self::chat_path(chat_id, "/agent"))
                .send()
                .await?,
        )
        .await?;
        let Value::Array(items) = v else {
            return Ok(Vec::new());
        };
        Ok(items
            .into_iter()
            .filter_map(|item| serde_json::from_value::<Agent>(item).ok())
            .filter(|agent| !agent.name.is_empty())
            .collect())
    }

    /// Create the chat's `OpenCode` session in its workspace.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout,
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`), and [`CodeError::Other`] if the new session's payload does
    /// not decode as a [`SessionMeta`].
    pub async fn create_session(&self, chat_id: &str) -> Result<SessionMeta, CodeError> {
        let v = Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(chat_id, "/session?directory=/chat/workspace"),
            )
            .json(&json!({}))
            .send()
            .await?,
        )
        .await?;
        serde_json::from_value(v).map_err(|e| CodeError::Other(format!("bad session payload: {e}")))
    }

    /// Every message of a session with its parts (`GET .../message`), the
    /// transcript the UI folds.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id` or `session_id`). A 2xx body that does not decode as a
    /// message list yields an empty list rather than an error.
    pub async fn messages(
        &self,
        chat_id: &str,
        session_id: &str,
    ) -> Result<Vec<MessageWithParts>, CodeError> {
        let v = Self::json_of(
            self.req(
                reqwest::Method::GET,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/message")),
            )
            .send()
            .await?,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Fire-and-forget prompt: the turn runs server-side; progress arrives
    /// over the SSE stream. `model` is `provider/model`; one without a `/`
    /// is dropped and the session's own model is used. `variant` is the
    /// thinking-effort tier and `agent` is the [`Agent::name`] to run as.
    /// All three are properties of the turn in the same way — the server
    /// applies whatever the body asked for and leaves the rest alone.
    ///
    /// `parts` is the whole message: text and any attachments, in the order
    /// the model should see them. See [`PromptPart`].
    ///
    /// # Errors
    ///
    /// [`CodeError::Other`] if `parts` is empty — the server takes that as a
    /// message with no content and answers 400.
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer — 404 for an unknown
    /// `chat_id` or `session_id`, 400 for a model or agent the server
    /// rejects. A turn that fails *after* this returns surfaces on the event
    /// stream, not here.
    pub async fn prompt_async(
        &self,
        chat_id: &str,
        session_id: &str,
        parts: &[PromptPart],
        model: Option<&str>,
        variant: Option<&str>,
        agent: Option<&str>,
    ) -> Result<(), CodeError> {
        if parts.is_empty() {
            return Err(CodeError::Other("prompt has no content".into()));
        }
        let body = prompt_body(parts, model, variant, agent);
        Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/prompt_async")),
            )
            .json(&body)
            .send()
            .await?,
        )
        .await
        .map(|_| ())
    }

    /// Cancel the session's in-flight turn.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id` or `session_id`).
    pub async fn abort(&self, chat_id: &str, session_id: &str) -> Result<(), CodeError> {
        Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/abort")),
            )
            .send()
            .await?,
        )
        .await
        .map(|_| ())
    }

    /// The session's cumulative diff, one [`FileDiff`] per changed file.
    ///
    /// Decoded element by element: an entry this client cannot make sense of
    /// is dropped on its own rather than collapsing the whole answer to "no
    /// changes", which is what a whole-array `unwrap_or_default` would do the
    /// day upstream adds a field shape we do not expect.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id` or `session_id`).
    pub async fn diff(&self, chat_id: &str, session_id: &str) -> Result<Vec<FileDiff>, CodeError> {
        let body = Self::json_of(
            self.req(
                reqwest::Method::GET,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/diff")),
            )
            .send()
            .await?,
        )
        .await?;
        let Value::Array(items) = body else {
            return Ok(Vec::new());
        };
        Ok(items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect())
    }

    /// Pending permission asks (reconnect catch-up).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`). A 2xx body that does not decode as a permission list
    /// yields an empty list rather than an error.
    pub async fn permissions(&self, chat_id: &str) -> Result<Vec<CodePermission>, CodeError> {
        let v = Self::json_of(
            self.req(
                reqwest::Method::GET,
                &Self::chat_path(chat_id, "/permission"),
            )
            .send()
            .await?,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Answer a pending permission ask. `response`: `once` | `always` |
    /// `reject`.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer — 404 for an ask that has
    /// already been answered or expired, 400 for a `response` outside the
    /// three accepted words.
    pub async fn reply_permission(
        &self,
        chat_id: &str,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<(), CodeError> {
        Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(
                    chat_id,
                    &format!("/session/{session_id}/permissions/{permission_id}"),
                ),
            )
            .json(&json!({"response": response}))
            .send()
            .await?,
        )
        .await
        .map(|_| ())
    }

    /// Attach to a chat's SSE stream. Events (including a final
    /// [`CodeEvent::Disconnected`]) arrive on the returned receiver; drop it
    /// to detach. Reconnection policy belongs to the caller.
    #[must_use]
    pub fn events(&self, chat_id: &str) -> mpsc::Receiver<CodeEvent> {
        let (tx, rx) = mpsc::channel(256);
        let this = self.clone();
        let chat_id = chat_id.to_string();
        tokio::spawn(async move {
            let reason = match this.stream_events(&chat_id, &tx).await {
                Ok(()) => "stream ended".to_string(),
                Err(e) => e.to_string(),
            };
            let _ = tx.send(CodeEvent::Disconnected { reason }).await;
        });
        rx
    }

    async fn stream_events(
        &self,
        chat_id: &str,
        tx: &mpsc::Sender<CodeEvent>,
    ) -> Result<(), CodeError> {
        // No overall timeout on the SSE request: heartbeats every ~10s keep
        // it alive; a silent dead stream is caught by the read inactivity
        // window below.
        let resp = self
            .http
            .get(format!(
                "{}{}",
                self.base,
                Self::chat_path(chat_id, "/event")
            ))
            .basic_auth("opencode", Some(&self.password))
            .timeout(Duration::from_hours(24))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CodeError::Status { status, body });
        }
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(60), stream.next()).await;
            let chunk = match chunk {
                Err(_) => return Err(CodeError::Other("stream silent for 60s".into())),
                Ok(None) => return Ok(()),
                Ok(Some(c)) => c?,
            };
            buf.extend_from_slice(&chunk);
            // SSE frames are separated by a blank line.
            while let Some(pos) = find_frame_end(&buf) {
                let frame: Vec<u8> = buf.drain(..pos + 2).collect();
                if let Some(evt) = parse_sse_frame(&frame) {
                    if tx.send(evt).await.is_err() {
                        return Ok(()); // receiver dropped — detach
                    }
                }
            }
        }
    }
}

/// The body both prompt routes take: the message, plus the three things that
/// ride a turn rather than being set on the session beforehand.
///
/// A field is sent only when there is something to say. `OpenCode` keeps
/// whatever the session already has for anything the body omits, so an empty
/// string here would not mean "leave it" — it would mean "use the model, tier
/// or agent called nothing", which is a 400.
fn prompt_body(
    parts: &[PromptPart],
    model: Option<&str>,
    variant: Option<&str>,
    agent: Option<&str>,
) -> Value {
    let mut body = json!({ "parts": parts });
    if let Some(m) = model {
        if let Some((provider, model_id)) = m.split_once('/') {
            body["model"] = json!({"providerID": provider, "modelID": model_id});
        }
    }
    if let Some(v) = variant.filter(|v| !v.is_empty()) {
        body["variant"] = json!(v);
    }
    if let Some(a) = agent.filter(|a| !a.is_empty()) {
        body["agent"] = json!(a);
    }
    body
}

fn find_frame_end(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Parse one SSE frame: concatenated `data:` lines → JSON → [`CodeEvent`].
fn parse_sse_frame(frame: &[u8]) -> Option<CodeEvent> {
    let text = String::from_utf8_lossy(frame);
    let mut data = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    let raw: Value = serde_json::from_str(&data).ok()?;
    let evt = dispatch_event(raw);
    // Heartbeats parse as Unknown with a null payload — drop them here.
    if matches!(&evt, CodeEvent::Unknown { tag, raw } if tag == "server.heartbeat" && raw.is_null())
    {
        return None;
    }
    Some(evt)
}

#[cfg(test)]
#[expect(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test assertions: an unexpected event kind or a fixture that does not decode is a test failure, and the panic carries the offending value into the report"
)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Both routes carry the same providers under different keys, so a build
    /// that fills in both must not hand back every model twice: two entries
    /// with the same reference are indistinguishable in a picker.
    #[test]
    fn a_model_on_both_keys_is_returned_once() {
        let raw = json!({
            "providers": [{"id": "anthropic", "models": {
                "claude-sonnet-4-5": {"name": "Claude Sonnet 4.5"}
            }}],
            "all": [{"id": "anthropic", "models": {
                "claude-sonnet-4-5": {"name": "Claude Sonnet 4.5"}
            }}]
        });
        let models = serde_json::from_value::<ProviderCatalog>(raw)
            .unwrap()
            .models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].reference(), "anthropic/claude-sonnet-4-5");
    }

    /// The same model id offered by two providers is two different models —
    /// one direct, one proxied — and both have to survive the flattening.
    #[test]
    fn the_same_id_from_two_providers_is_two_models() {
        let raw = json!({
            "providers": [
                {"id": "anthropic", "models": {
                    "claude-sonnet-4-5": {"name": "Claude Sonnet 4.5"}
                }},
                {"id": "opencode", "models": {
                    "claude-sonnet-4-5": {"name": "Claude Sonnet 4.5"}
                }}
            ]
        });
        let models = serde_json::from_value::<ProviderCatalog>(raw)
            .unwrap()
            .models();
        let refs: Vec<String> = models.iter().map(ModelInfo::reference).collect();
        assert_eq!(
            refs,
            ["anthropic/claude-sonnet-4-5", "opencode/claude-sonnet-4-5"]
        );
    }

    #[test]
    fn dispatches_part_updated_with_delta() {
        let raw = json!({
            "type": "message.part.updated",
            "properties": {
                "part": {
                    "id": "prt_1", "messageID": "msg_1", "sessionID": "ses_1",
                    "type": "text", "text": "Hello wor"
                },
                "delta": "ld"
            }
        });
        match dispatch_event(raw) {
            CodeEvent::PartUpdated { part, delta } => {
                assert_eq!(part.id, "prt_1");
                assert_eq!(part.kind, "text");
                assert_eq!(delta.as_deref(), Some("ld"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    /// The server expands a `text/plain` attachment into two text parts of
    /// its own — one of them the whole file — and flags them. A part that
    /// arrives without the flag is something somebody actually said.
    #[test]
    fn a_parts_synthetic_flag_survives_the_wire() {
        let part = |raw: Value| match dispatch_event(json!({
            "type": "message.part.updated",
            "properties": {"part": raw}
        })) {
            CodeEvent::PartUpdated { part, .. } => part,
            other => panic!("wrong event: {other:?}"),
        };
        assert!(
            part(json!({
                "id": "prt_2", "messageID": "msg_1", "sessionID": "ses_1",
                "type": "text", "text": "# notes", "synthetic": true
            }))
            .synthetic
        );
        assert!(
            !part(json!({
                "id": "prt_1", "messageID": "msg_1", "sessionID": "ses_1",
                "type": "text", "text": "look at this"
            }))
            .synthetic,
            "an ordinary part carries no flag at all"
        );
    }

    #[test]
    fn dispatches_global_envelope() {
        let raw = json!({
            "directory": "/chat/workspace",
            "payload": {"type": "session.idle", "properties": {"sessionID": "ses_9"}}
        });
        match dispatch_event(raw) {
            CodeEvent::SessionIdle { session_id } => assert_eq!(session_id, "ses_9"),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn dispatches_permission() {
        let raw = json!({
            "type": "permission.updated",
            "properties": {
                "id": "perm_1", "sessionID": "ses_1", "type": "bash",
                "title": "Run git push", "metadata": {"command": "git push"}
            }
        });
        match dispatch_event(raw) {
            CodeEvent::PermissionAsked(p) => {
                assert_eq!(p.id, "perm_1");
                assert_eq!(p.title, "Run git push");
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn sse_frame_parsing_joins_data_lines() {
        let frame =
            b"event: message\ndata: {\"type\":\"server.connected\",\ndata: \"properties\":{}}\n\n";
        match parse_sse_frame(frame) {
            Some(CodeEvent::Connected) => {}
            other => panic!("wrong parse: {other:?}"),
        }
    }

    #[test]
    fn unknown_events_are_preserved() {
        let raw = json!({"type": "todo.updated", "properties": {"x": 1}});
        match dispatch_event(raw) {
            CodeEvent::Unknown { tag, .. } => assert_eq!(tag, "todo.updated"),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn session_carries_its_model_and_effort() {
        let raw = json!({
            "id": "ses_1", "title": "t", "directory": "/chat/workspace",
            "version": "1.0.0", "projectID": "p", "slug": "s",
            "time": {"created": 1, "updated": 2},
            "model": {"id": "deepseek-v4", "providerID": "opencode", "variant": "high"}
        });
        let s: SessionMeta = serde_json::from_value(raw).unwrap();
        let model = s.model.unwrap_or_default();
        assert_eq!(model.reference().as_deref(), Some("opencode/deepseek-v4"));
        assert_eq!(model.effort(), Some("high"));
    }

    /// `default` is how `OpenCode` records "the turn asked for no variant", so
    /// it must not show up as a chosen effort tier.
    #[test]
    fn default_variant_is_not_an_effort() {
        let model = SessionModel {
            id: "minimax-m2.7".into(),
            provider_id: "opencode".into(),
            variant: Some("default".into()),
        };
        assert_eq!(model.effort(), None);
    }

    #[test]
    fn catalog_decodes_from_either_route() {
        let models = json!({
            "gpt-5.2": {
                "id": "gpt-5.2", "providerID": "openai", "name": "GPT-5.2",
                "limit": {"context": 400_000.0, "output": 128_000.0},
                "variants": {"high": {}, "low": {}, "none": {}, "xhigh": {}}
            },
            "minimax-m2.7": {
                "id": "minimax-m2.7", "providerID": "opencode", "name": "MiniMax M2.7",
                "limit": {"context": 204_800}
            }
        });
        for body in [
            json!({"providers": [{"id": "x", "models": models}], "default": {}}),
            json!({"all": [{"id": "x", "models": models}], "default": {}, "connected": []}),
        ] {
            let catalog: ProviderCatalog = serde_json::from_value(body).unwrap();
            let all = catalog.models();
            assert_eq!(all.len(), 2);
            let gpt = all.iter().find(|m| m.id == "gpt-5.2").unwrap();
            assert_eq!(gpt.reference(), "openai/gpt-5.2");
            assert_eq!(gpt.limit.context_tokens(), Some(400_000));
            // Weakest first, not the alphabetical order the map decoded into.
            assert_eq!(gpt.efforts(), ["none", "low", "high", "xhigh"]);

            let mini = all.iter().find(|m| m.id == "minimax-m2.7").unwrap();
            assert!(mini.efforts().is_empty());
            assert_eq!(mini.limit.context_tokens(), Some(204_800));
        }
    }

    #[test]
    fn a_model_with_no_declared_window_reports_none() {
        assert_eq!(ModelLimit::default().context_tokens(), None);
    }

    /// `GET /agent` as the SDK types describe it, decoded down to the four
    /// fields the picker needs.
    #[test]
    fn an_agent_decodes_with_its_description_and_mode() {
        let raw = json!({
            "name": "plan",
            "description": "Read-only analysis. Cannot edit files.",
            "mode": "primary",
            "builtIn": true,
            "permission": {"edit": "deny", "bash": {"*": "ask"}},
            "tools": {"write": false},
            "options": {},
            "temperature": 0.2
        });
        let agent: Agent = serde_json::from_value(raw).unwrap();
        assert_eq!(agent.name, "plan");
        assert_eq!(
            agent.description.as_deref(),
            Some("Read-only analysis. Cannot edit files.")
        );
        assert_eq!(agent.mode, AgentMode::Primary);
        assert!(agent.built_in);
        assert!(agent.is_primary());
    }

    /// A subagent is invoked by another agent, never chosen by a person, so
    /// it must not be offered as something a chat can run on.
    #[test]
    fn a_subagent_is_not_selectable_but_all_is() {
        let subagent: Agent = serde_json::from_value(json!({
            "name": "reviewer", "mode": "subagent", "builtIn": false
        }))
        .unwrap();
        assert!(!subagent.is_primary());

        let both: Agent =
            serde_json::from_value(json!({"name": "general", "mode": "all"})).unwrap();
        assert!(both.is_primary());
    }

    /// A mode this client has not heard of, or none at all, still names an
    /// agent a person can pick — guessing "subagent" would make one word of
    /// server vocabulary empty the whole picker.
    #[test]
    fn an_unknown_mode_is_still_selectable() {
        for raw in [
            json!({"name": "build"}),
            json!({"name": "build", "mode": null}),
            json!({"name": "build", "mode": "supervisor"}),
        ] {
            let agent: Agent = serde_json::from_value(raw).unwrap();
            assert_eq!(agent.mode, AgentMode::All);
            assert!(agent.is_primary());
        }
    }

    /// The turn carries the agent beside the model and the tier, as a bare
    /// name at the top level of the body.
    #[test]
    fn the_prompt_body_carries_model_tier_and_agent() {
        let body = prompt_body(
            &[PromptPart::text("ship it")],
            Some("opencode/claude-sonnet-4-5"),
            Some("high"),
            Some("plan"),
        );
        assert_eq!(
            body["model"],
            json!({"providerID": "opencode", "modelID": "claude-sonnet-4-5"})
        );
        assert_eq!(body["variant"], json!("high"));
        assert_eq!(body["agent"], json!("plan"));
        assert_eq!(body["parts"][0]["text"], json!("ship it"));
    }

    /// Nothing chosen means nothing sent: the session keeps what it has.
    #[test]
    fn an_unchosen_field_is_left_out_of_the_body() {
        let body = prompt_body(&[PromptPart::text("hello")], None, None, None);
        assert!(body.get("model").is_none());
        assert!(body.get("variant").is_none());
        assert!(body.get("agent").is_none());

        // Empty is not a choice either — it would name an agent called "".
        let blank = prompt_body(
            &[PromptPart::text("hello")],
            Some("no-slash"),
            Some(""),
            Some(""),
        );
        assert!(blank.get("model").is_none());
        assert!(blank.get("variant").is_none());
        assert!(blank.get("agent").is_none());
    }

    // ---------------------------------------------------------- pull requests

    fn pull(extra: &Value) -> PullRequest {
        let mut raw = json!({
            "number": 12,
            "title": "Tighten the README quickstart",
            "state": "open",
            "draft": false,
            "mergeable": true,
            "checks": "passing",
            "url": "https://github.com/me/notes/pull/12",
            "head": "agent/notes-9f2c1a",
            "base": "main",
            "created_at": "2026-08-24T09:12:31Z",
            "updated_at": "2026-08-24T10:02:04Z"
        });
        if let (Some(base), Some(extra)) = (raw.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                base.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value(raw).unwrap()
    }

    #[test]
    fn a_pull_decodes_every_field_of_the_contract() {
        let list = parse_pulls(&json!({"pulls": [{
            "number": 12,
            "title": "Tighten the README quickstart",
            "state": "merged",
            "draft": false,
            "mergeable": null,
            "checks": "none",
            "url": "https://github.com/me/notes/pull/12",
            "head": "agent/notes-9f2c1a",
            "base": "main",
            "created_at": "2026-08-24T09:12:31Z",
            "updated_at": "2026-08-24T10:02:04Z"
        }]}));
        let [only] = list.as_slice() else {
            panic!("expected one pull, got {list:?}")
        };
        assert_eq!(only.number, 12);
        assert_eq!(only.state, PullState::Merged);
        assert_eq!(only.checks, Checks::None);
        assert_eq!(only.mergeable, None);
        assert_eq!(only.head, "agent/notes-9f2c1a");
        assert_eq!(only.base, "main");
    }

    /// A state or a check summary this client has not heard of must not read
    /// as "open" or "passing": between them those two decide whether the app
    /// offers to merge.
    #[test]
    fn words_this_client_does_not_know_read_as_unknown() {
        let odd = pull(&json!({"state": "locked", "checks": "flaky"}));
        assert_eq!(odd.state, PullState::Unknown);
        assert_eq!(odd.checks, Checks::Unknown);
        assert!(!odd.is_mergeable());
    }

    /// A garbled entry loses itself, not the rest of the list.
    #[test]
    fn one_bad_entry_does_not_take_the_list_with_it() {
        let list = parse_pulls(&json!({"pulls": [
            {"number": "twelve"},
            {"number": 13, "state": "open", "url": "https://example.invalid/13"}
        ]}));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].number, 13);
    }

    #[test]
    fn a_body_without_a_pulls_array_is_no_pulls() {
        assert!(parse_pulls(&json!({"error": "unknown chat"})).is_empty());
        assert!(parse_pulls(&Value::Null).is_empty());
    }

    /// The merge gate, case by case. Everything here is a state GitHub really
    /// reports, and each one of them is a reason not to offer the button.
    #[test]
    fn merging_is_offered_only_when_github_says_it_can_merge() {
        assert!(pull(&json!({})).is_mergeable());
        // Checks that have not answered yet are not a refusal.
        assert!(pull(&json!({"checks": "pending"})).is_mergeable());
        assert!(pull(&json!({"checks": "none"})).is_mergeable());

        assert!(!pull(&json!({"checks": "failing"})).is_mergeable());
        assert!(!pull(&json!({"draft": true})).is_mergeable());
        assert!(!pull(&json!({"mergeable": false})).is_mergeable());
        // Still being computed: a wait, not a yes.
        assert!(!pull(&json!({"mergeable": null})).is_mergeable());
        assert!(!pull(&json!({"state": "merged"})).is_mergeable());
        assert!(!pull(&json!({"state": "closed"})).is_mergeable());
    }

    #[test]
    fn a_merge_answer_carries_the_refreshed_row() {
        let outcome: MergeOutcome = serde_json::from_value(json!({
            "merged": true,
            "sha": "9f2c1ad",
            "pull": {"number": 12, "state": "merged", "mergeable": false}
        }))
        .unwrap();
        assert!(outcome.merged);
        assert_eq!(outcome.sha, "9f2c1ad");
        assert_eq!(outcome.pull.unwrap().state, PullState::Merged);
    }

    /// A manager that answers the merge without echoing the row must not make
    /// a merge that happened look like a failure.
    #[test]
    fn a_merge_answer_without_a_row_still_decodes() {
        let outcome: MergeOutcome =
            serde_json::from_value(json!({"merged": true, "sha": "9f2c1ad"})).unwrap();
        assert!(outcome.merged);
        assert_eq!(outcome.pull, None);
    }

    /// What the user is shown when the manager refuses. The status code and
    /// the JSON it arrived in are not part of the sentence.
    #[test]
    fn a_refusal_is_shown_as_the_servers_own_sentence() {
        let refused = CodeError::Status {
            status: 409,
            body: r##"{"error": "#12 conflicts with main — it needs a rebase."}"##.to_owned(),
        };
        assert_eq!(
            refused.message(),
            "#12 conflicts with main — it needs a rebase."
        );

        let github = CodeError::Status {
            status: 422,
            body: r#"{"error": "At least 1 approving review is required."}"#.to_owned(),
        };
        assert_eq!(github.message(), "At least 1 approving review is required.");
    }

    /// A body that is not the manager's error shape — an HTML 502 from the
    /// gateway, or nothing at all — still has to say something.
    #[test]
    fn an_answer_with_no_sentence_falls_back_to_the_status() {
        for body in ["", "<html>502 Bad Gateway</html>", r#"{"error": "  "}"#] {
            let e = CodeError::Status {
                status: 502,
                body: body.to_owned(),
            };
            assert_eq!(e.message(), "server said 502");
        }
        assert_eq!(CodeError::Other("no route".into()).message(), "no route");
    }

    /// The prompt body is the one place this client writes `OpenCode`'s own
    /// schema rather than reading it, so the shape is pinned against
    /// `TextPartInput | FilePartInput` verbatim.
    #[test]
    fn prompt_parts_serialize_to_the_sdk_shapes() {
        let parts = vec![
            PromptPart::text("what changed here"),
            PromptPart::file("image/jpeg", "IMG_0042.jpg", "QUJD"),
            PromptPart::text_file("notes.md", "QUJD"),
        ];
        assert_eq!(
            serde_json::to_value(&parts).unwrap(),
            json!([
                {"type": "text", "text": "what changed here"},
                {"type": "file", "mime": "image/jpeg", "filename": "IMG_0042.jpg",
                 "url": "data:image/jpeg;base64,QUJD"},
                // Declared text/plain whatever the file is called: that is the
                // one mime the server decodes and inlines.
                {"type": "file", "mime": "text/plain", "filename": "notes.md",
                 "url": "data:text/plain;base64,QUJD"},
            ])
        );
    }

    /// History gives an attachment back as a file part, and the transcript
    /// wants its bytes — but only from a `data:` URL. A `file:` URL names a
    /// path inside the container, which this device cannot read.
    #[test]
    fn only_a_data_url_yields_bytes_to_the_transcript() {
        let part = |url: &str| Part {
            url: Some(url.to_owned()),
            ..Part::default()
        };
        assert_eq!(
            part("data:image/png;base64,QUJD").data_url_base64(),
            Some("QUJD")
        );
        assert_eq!(part("file:///chat/workspace/a.png").data_url_base64(), None);
        assert_eq!(part("data:text/plain,hello").data_url_base64(), None);
        assert_eq!(Part::default().data_url_base64(), None);
    }

    /// A file part decodes out of the same `message.part.updated` envelope
    /// every other part arrives in, so the fold has something to work with.
    #[test]
    fn dispatches_a_file_part() {
        let raw = json!({
            "type": "message.part.updated",
            "properties": {"part": {
                "id": "prt_2", "messageID": "msg_1", "sessionID": "ses_1",
                "type": "file", "mime": "image/jpeg", "filename": "IMG_0042.jpg",
                "url": "data:image/jpeg;base64,QUJD"
            }}
        });
        match dispatch_event(raw) {
            CodeEvent::PartUpdated { part, .. } => {
                assert_eq!(part.kind, "file");
                assert_eq!(part.mime.as_deref(), Some("image/jpeg"));
                assert_eq!(part.filename.as_deref(), Some("IMG_0042.jpg"));
                assert_eq!(part.data_url_base64(), Some("QUJD"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    /// The aggregate's contract, verbatim: a chat's own permission payload
    /// with `chatId` spliced in beside it. Both halves have to survive the
    /// flatten, because an ask without its chat cannot be answered and a
    /// chat without its ask is nothing to show.
    #[test]
    fn an_aggregate_entry_carries_both_its_chat_and_its_ask() {
        let raw = json!({
            "permissions": [{
                "chatId": "chat_a1",
                "id": "per_1",
                "sessionID": "ses_1",
                "type": "bash",
                "title": "Run git push",
                "metadata": {"command": "git push"}
            }],
            "unreachable": ["chat_b2"]
        });
        let report: PermissionReport = serde_json::from_value(raw).unwrap();
        assert_eq!(report.permissions.len(), 1);
        let ask = &report.permissions[0];
        assert_eq!(ask.chat_id, "chat_a1");
        assert_eq!(ask.permission.id, "per_1");
        assert_eq!(ask.permission.session_id, "ses_1");
        assert_eq!(ask.permission.kind, "bash");
        assert_eq!(ask.permission.title, "Run git push");
        assert_eq!(
            ask.permission
                .metadata
                .get("command")
                .and_then(Value::as_str),
            Some("git push")
        );
        assert_eq!(report.unreachable, ["chat_b2"]);
    }

    /// "Nothing is waiting on you" is the common answer and must not need the
    /// optional half of the contract to be spelled out.
    #[test]
    fn an_empty_aggregate_needs_no_optional_keys() {
        let report: PermissionReport = serde_json::from_value(json!({"permissions": []})).unwrap();
        assert!(report.permissions.is_empty());
        assert!(report.unreachable.is_empty());
    }

    /// The contracted shape survives the strict decode, both halves of it.
    #[test]
    fn the_contracted_aggregate_decodes() {
        let report = CodeClient::decode_permission_report(json!({
            "permissions": [{
                "chatId": "chat_a1", "id": "per_1", "sessionID": "ses_1",
                "type": "bash", "title": "Run git push", "metadata": {}
            }],
            "unreachable": ["chat_b2"]
        }))
        .unwrap();
        assert_eq!(report.permissions.len(), 1);
        assert_eq!(report.unreachable, ["chat_b2"]);
    }

    /// A 2xx whose shape this client cannot read is not an answer, and above
    /// all it is not "nothing is waiting on you anywhere" — that reading is
    /// what clears every card on the list, which is the failure the aggregate
    /// was built to remove rather than to cause.
    #[test]
    fn a_body_the_client_cannot_read_is_an_error_rather_than_an_empty_report() {
        for body in [
            // A manager that serialised Python's None for "none pending".
            json!({"permissions": null}),
            // What json_of hands back for a body that stopped mid-read, and
            // for a 204.
            json!(null),
            // A map keyed by chat where the flat list was contracted.
            json!({"permissions": {"chat_a": []}}),
            // The bare-array shapes: serde would read the empty one as a
            // struct with every field defaulted.
            json!([]),
            json!([{"chatId": "chat_a", "id": "per_1"}]),
            // An auth or captive-portal page served with a 200 through the
            // tailnet front door.
            json!("<html>sign in</html>"),
        ] {
            assert!(
                CodeClient::decode_permission_report(body.clone()).is_err(),
                "{body} must not read as an empty report"
            );
        }
    }

    /// A manager that grows a field must not take the aggregate down with it:
    /// unknown keys are ignored, at both levels.
    #[test]
    fn an_aggregate_tolerates_fields_it_has_not_heard_of() {
        let raw = json!({
            "permissions": [{
                "chatId": "chat_a1", "id": "per_1", "sessionID": "ses_1",
                "type": "bash", "title": "Run git push", "metadata": {},
                "askedAt": 1_756_000_000.0
            }],
            "unreachable": [],
            "scannedAt": 1_756_000_001.0
        });
        let report: PermissionReport = serde_json::from_value(raw).unwrap();
        assert_eq!(report.permissions.len(), 1);
        assert_eq!(report.permissions[0].chat_id, "chat_a1");
    }
}
