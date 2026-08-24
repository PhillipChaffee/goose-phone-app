//! Session identity, listing and creation.

use serde::Deserialize;
use serde_json::Value;

use super::config::ConfigOption;

/// Update to session metadata (`session_info_update`), e.g. auto-generated titles.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionInfoUpdate {
    pub title: Option<String>,
    pub updated_at: Option<String>,
    #[serde(rename = "_meta")]
    pub meta: Option<Value>,
}

/// One entry from `session/list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

impl SessionInfo {
    fn meta_field(&self, key: &str) -> Option<&Value> {
        self.meta.as_ref()?.get(key)
    }

    #[must_use]
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.session_id.clone())
    }

    #[must_use]
    pub fn message_count(&self) -> Option<u64> {
        self.meta_field("messageCount")?.as_u64()
    }

    #[must_use]
    pub fn last_message_snippet(&self) -> Option<String> {
        Some(self.meta_field("lastMessageSnippet")?.as_str()?.to_string())
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
    /// The wire string goose matches on.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Scheduled => "scheduled",
            Self::Acp => "acp",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResponse {
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    pub session_id: String,
    /// Session configuration the agent offers — provider, model, mode and
    /// thinking effort. This is where the list of available models arrives:
    /// no separate call is needed, and it was previously parsed away.
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
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
    }
}
