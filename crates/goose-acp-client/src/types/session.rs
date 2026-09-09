//! Session identity, listing and creation.

use serde::Deserialize;
use serde_json::Value;

use super::config::ConfigOption;

/// Update to session metadata (`session_info_update`), e.g. auto-generated titles.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionInfoUpdate {
    /// The session's name as the server now holds it — chiefly the
    /// auto-generated title it minted after a turn; absent when this update
    /// carries none.
    pub title: Option<String>,
    /// When the session was last touched (wire `updatedAt`, an ISO 8601
    /// string); absent when the update does not stamp one.
    pub updated_at: Option<String>,
    /// Unmodeled goose keys riding the standard `_meta` object beside the
    /// typed fields, kept whole; `None` when the update carries none.
    #[serde(rename = "_meta")]
    pub meta: Option<Value>,
}

/// One entry from `session/list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// The server's identifier for the session (wire `sessionId`); required
    /// on every row, and the label [`SessionInfo::display_title`] falls back
    /// to when there is no title.
    pub session_id: String,
    /// The server directory the session runs in (wire `cwd`); `None` when
    /// goose sends none.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The session's name — auto-titled or set by a person — or `None` for a
    /// session that has none.
    #[serde(default)]
    pub title: Option<String>,
    /// When the session was last touched (wire `updatedAt`, an ISO 8601
    /// string); `None` when goose sends none.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Unmodeled goose keys riding the standard `_meta` object — among them
    /// `messageCount`, `sessionType` and `lastMessageSnippet` — kept whole
    /// and read out by the typed accessors below; `None` when the row carries
    /// no object.
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

impl SessionInfo {
    fn meta_field(&self, key: &str) -> Option<&Value> {
        self.meta.as_ref()?.get(key)
    }

    /// The title this session shows: its own title when one is set and not
    /// all whitespace, its session id otherwise.
    #[must_use]
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.session_id.clone())
    }

    /// How many messages the session holds, from `_meta.messageCount`;
    /// `None` when goose sent no usable count.
    #[must_use]
    pub fn message_count(&self) -> Option<u64> {
        self.meta_field("messageCount")?.as_u64()
    }

    /// The session's last user-visible message, from
    /// `_meta.lastMessageSnippet`; goose sends it only when the list request
    /// opted in with `goose.includeLastMessageSnippet`, so `None` otherwise.
    #[must_use]
    pub fn last_message_snippet(&self) -> Option<String> {
        Some(self.meta_field("lastMessageSnippet")?.as_str()?.to_string())
    }

    /// Which kind of session this is, from `_meta.sessionType`.
    ///
    /// `None` covers both an absent field and a spelling this crate does not
    /// model. Neither is reachable through `session/list`, which returns only
    /// the three kinds it accepts as a filter, so treating an unknown as "no
    /// badge" is the right failure: a session still lists, unlabelled.
    #[must_use]
    pub fn kind(&self) -> Option<SessionKind> {
        SessionKind::from_wire(self.meta_field("sessionType")?.as_str()?)
    }

    /// The badge a list row shows for this session, or `None` for an ordinary
    /// user chat, which needs no label — every row in the app would carry it.
    ///
    /// The backend spelling never reaches the screen: "acp" means a session
    /// some *other* agent client opened, which reads to a person as "Agent".
    #[must_use]
    pub fn kind_label(&self) -> Option<&'static str> {
        self.kind()?.label()
    }
}

/// Which sessions `session/list` should return.
///
/// goose keeps three kinds side by side and filters on `_meta.types`. It
/// accepts these three strings and nothing else: anything unrecognised is an
/// `invalid_params` error rather than an ignored filter, so the set is a
/// closed enum here instead of free-form strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionKind {
    /// Started by a person, in the app or the CLI.
    User,
    /// Started by the scheduler running a recipe.
    Scheduled,
    /// Started by an ACP client other than a person — a sub-agent, say.
    Acp,
}

impl SessionKind {
    /// Every kind goose lists, in the order it declares them.
    pub const ALL: [Self; 3] = [Self::User, Self::Scheduled, Self::Acp];

    /// The wire string goose matches on.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Scheduled => "scheduled",
            Self::Acp => "acp",
        }
    }

    /// The inverse of [`SessionKind::as_wire`]. `None` for anything else —
    /// goose has seven session types internally and lists only these three.
    #[must_use]
    pub fn from_wire(wire: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_wire() == wire)
    }

    /// UI copy for this kind, `None` where the kind is the unremarkable one.
    #[must_use]
    pub const fn label(self) -> Option<&'static str> {
        match self {
            Self::User => None,
            Self::Scheduled => Some("Scheduled"),
            Self::Acp => Some("Agent"),
        }
    }
}

/// One `session/list` request: the filters, plus — if this is not the first
/// page — the cursor that was minted *under those same filters*.
///
/// The two travel together because the server ties them together. goose
/// hashes the effective filter set into every cursor it hands out
/// (`session_list_filter_hash` in its `list_sessions.rs`) and refuses a
/// cursor whose hash does not match the filters it arrives beside:
/// `invalid_params`, "session list cursor does not match filters". A
/// three-argument `session_list(kinds, query, cursor)` invites exactly that
/// bug — type in the search box, scroll to the bottom, and the next page is
/// fetched with yesterday's cursor and today's query.
///
/// So the fields are private and no constructor takes a cursor.
/// [`SessionQuery::new`] always describes page one, and the only thing that
/// produces a cursor-bearing query is [`SessionQuery::next_page`], which
/// carries over the filters of the query whose response it was given.
/// Changing filters means calling `new` again, which drops the cursor — which
/// is what you want anyway, since new filters mean a new first page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionQuery {
    kinds: Vec<SessionKind>,
    query: Option<String>,
    cursor: Option<String>,
}

impl SessionQuery {
    /// The first page of the sessions matching `kinds` and `query`.
    ///
    /// An empty `kinds` means all of them: that is the server's own reading
    /// of an absent or empty `types` filter, not a special case invented
    /// here. `query` is trimmed and a blank one dropped, again mirroring the
    /// server, so a half-typed search box and an empty one are one request.
    #[must_use]
    pub fn new(kinds: &[SessionKind], query: Option<&str>) -> Self {
        Self {
            kinds: kinds.to_vec(),
            query: query
                .map(str::trim)
                .filter(|q| !q.is_empty())
                .map(ToString::to_string),
            cursor: None,
        }
    }

    /// The query for the page after `page`, or `None` when `page` was the
    /// last one — which is also the "is there more?" the UI needs.
    ///
    /// `page` must be this query's own response. That is the whole reason
    /// this is a method rather than a setter: it is the only way a cursor
    /// gets in, and it can only pair one with the filters that produced it.
    #[must_use]
    pub fn next_page(&self, page: &SessionListResponse) -> Option<Self> {
        let cursor = page.next_cursor.clone()?;
        Some(Self {
            kinds: self.kinds.clone(),
            query: self.query.clone(),
            cursor: Some(cursor),
        })
    }

    /// The kinds this query restricts `session/list` to; an empty slice
    /// means every kind.
    #[must_use]
    pub fn kinds(&self) -> &[SessionKind] {
        &self.kinds
    }

    /// The keyword this query searches with, already trimmed and non-empty;
    /// `None` when it performs no search.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// Deliberately not public: a cursor that can be read out is a cursor
    /// that can be stored beside a *different* filter set and sent back.
    pub(crate) fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}

/// Reply to `session/list`: one page of sessions plus the cursor to the page
/// after them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResponse {
    /// This page's sessions, newest first as goose orders them; a missing
    /// key deserializes as empty.
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    /// The cursor for the page after this one, minted under the same filters
    /// the request carried; `None` when this was the last page.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Reply to `session/new`: the session's identity, the configuration the
/// agent offers, and the `session/new`-specific `_meta` keys.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    /// The server's identifier for the new session (wire `sessionId`), which
    /// every later request about that session needs.
    pub session_id: String,
    /// Session configuration the agent offers — provider, model, mode and
    /// thinking effort. This is where the list of available models arrives:
    /// no separate call is needed, and it was previously parsed away.
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
    /// Unmodeled goose keys riding the standard `_meta` object of the reply,
    /// kept whole; `None` when the server sends no object.
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_session_info() {
        let raw = json!({
            "sessionId": "20260821_1",
            "cwd": "/home/me/project",
            "title": "Fix the build",
            "updatedAt": "2026-08-21T09:00:00Z",
            "additionalDirectories": [],
            "_meta": {
                "messageCount": 12,
                "createdAt": "2026-08-20T18:00:00Z",
                "userSetName": false,
                "sessionType": "user",
                "hasRecipe": false,
                "lastMessageSnippet": "Done — the build is green."
            }
        });
        let info: SessionInfo = serde_json::from_value(raw).unwrap();
        assert_eq!(info.session_id, "20260821_1");
        assert_eq!(info.display_title(), "Fix the build");
        assert_eq!(info.message_count(), Some(12));
        assert_eq!(
            info.last_message_snippet().as_deref(),
            Some("Done — the build is green.")
        );
    }

    /// The three strings goose accepts in `_meta.types`; anything else is an
    /// `invalid_params` error, so these have to match exactly.
    #[test]
    fn session_kinds_use_the_wire_spelling() {
        assert_eq!(SessionKind::User.as_wire(), "user");
        assert_eq!(SessionKind::Scheduled.as_wire(), "scheduled");
        assert_eq!(SessionKind::Acp.as_wire(), "acp");
        for kind in SessionKind::ALL {
            assert_eq!(SessionKind::from_wire(kind.as_wire()), Some(kind));
        }
        // goose has seven session types; four of them never list.
        assert_eq!(SessionKind::from_wire("sub_agent"), None);
    }

    fn with_meta(meta: &Value) -> SessionInfo {
        serde_json::from_value(json!({"sessionId": "s1", "_meta": meta})).unwrap()
    }

    /// The badge each kind earns. A user chat gets none — it is every other
    /// row in the list — and so does a session whose type never arrived.
    #[test]
    fn kind_label_names_only_the_unusual_kinds() {
        let scheduled = with_meta(&json!({"sessionType": "scheduled"}));
        assert_eq!(scheduled.kind(), Some(SessionKind::Scheduled));
        assert_eq!(scheduled.kind_label(), Some("Scheduled"));

        let acp = with_meta(&json!({"sessionType": "acp"}));
        assert_eq!(acp.kind(), Some(SessionKind::Acp));
        assert_eq!(acp.kind_label(), Some("Agent"));

        let user = with_meta(&json!({"sessionType": "user"}));
        assert_eq!(user.kind(), Some(SessionKind::User));
        assert_eq!(user.kind_label(), None);

        // `_meta` without the field at all, and no `_meta` at all: an
        // unlabelled row either way, never a panic and never a wrong badge.
        let untyped = with_meta(&json!({"messageCount": 3}));
        assert_eq!(untyped.kind(), None);
        assert_eq!(untyped.kind_label(), None);
        let bare: SessionInfo = serde_json::from_value(json!({"sessionId": "s1"})).unwrap();
        assert_eq!(bare.kind_label(), None);
    }
}
