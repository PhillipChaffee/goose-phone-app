//! Per-session configuration options (`configOptions`).

use serde::Deserialize;
use serde_json::Value;

/// One configurable knob on a session.
///
/// Wire shape per ACP schema 1.5 (`SessionConfigOption`): `configId`, `name`,
/// an optional `description`, and a flattened kind payload tagged by `type`.
/// For `type: "select"` the payload is `currentValue` plus `options`, each of
/// which keys on `value` (not `id`).
///
/// EXCEPT that goose renamed it. Measured against a real `goose serve` 1.46.0:
/// every entry arrives keyed `id`, not `configId`, and because this field is
/// the one non-`default` member of the struct, the whole of `session/new`
/// failed to deserialize, reporting a missing `configId` field. Not a
/// degraded session: no session at all, so the app could not open a chat
/// against that server. The rest of the shape is unchanged, including
/// `ConfigChoice` keying on `value`, which was checked at the same time.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOption {
    /// `alias`, not `rename`, so BOTH spellings parse. Which server a user
    /// points at is not ours to choose — the phone reaches whatever is on
    /// their tailnet — so a client that understands only the newer name would
    /// simply break in the other direction on the next person's older server.
    #[serde(alias = "id")]
    pub config_id: String,
    #[serde(default)]
    pub name: String,
    /// The agent's own words about what this option does. goose sends one
    /// for `thinking_effort`, which is exactly the option a user is most
    /// likely to find stuck on a single value.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub current_value: Option<String>,
    #[serde(default)]
    pub options: Vec<ConfigChoice>,
}

impl ConfigOption {
    /// The label for the current value, falling back to the raw id.
    #[must_use]
    pub fn current_label(&self) -> Option<&str> {
        let current = self.current_value.as_deref()?;
        Some(
            self.options
                .iter()
                .find(|o| o.value == current)
                .map_or(current, |o| o.name.as_str()),
        )
    }

    /// Whether choosing between the values would change anything.
    ///
    /// An option with one value is a fact, not a control: goose ships
    /// `thinking_effort` as a select whose only value is `off` whenever the
    /// session's model is not a reasoning model. Offering that as a menu
    /// would be a control that does nothing (design rule 11); reporting it
    /// tells the user *why* effort is not adjustable here.
    #[must_use]
    pub const fn is_adjustable(&self) -> bool {
        self.options.len() > 1
    }
}

/// One selectable value of a `select` config option.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChoice {
    pub value: String,
    #[serde(default)]
    pub name: String,
    /// What choosing this one does, per `SessionConfigSelectOption`. goose
    /// sends one for each of its modes, which is the option whose values a
    /// reader is least able to guess at from the name alone.
    #[serde(default)]
    pub description: Option<String>,
}

/// Pull a `configOptions` array out of any response that carries one.
///
/// `session/new` types it; `session/load` and `session/set_config_option`
/// come back as raw JSON, and all three carry the same array.
#[must_use]
pub fn config_options_from(raw: &Value) -> Vec<ConfigOption> {
    raw.get("configOptions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The four options goose builds in `acp::response_builder`, verbatim.
    #[test]
    fn parses_every_config_option_goose_sends() {
        let raw = json!([
            {"configId": "provider", "name": "Provider", "type": "select",
             "currentValue": "anthropic",
             "options": [{"value": "anthropic", "name": "Anthropic"},
                         {"value": "openai", "name": "OpenAI"}]},
            {"configId": "mode", "name": "Mode", "category": "mode", "type": "select",
             "currentValue": "auto",
             "options": [{"value": "auto", "name": "Auto"},
                         {"value": "approve", "name": "Manual approval"}]},
            {"configId": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": "claude-opus-5",
             "options": [{"value": "claude-opus-5", "name": "Claude Opus 5"}]},
            {"configId": "thinking_effort", "name": "Thinking effort",
             "category": "thought_level", "type": "select",
             "description": "Controls reasoning effort for models that support extended thinking.",
             "currentValue": "off",
             "options": [{"value": "off", "name": "off"}]}
        ]);
        let opts: Vec<ConfigOption> = serde_json::from_value(raw).unwrap();
        let ids: Vec<&str> = opts.iter().map(|o| o.config_id.as_str()).collect();
        assert_eq!(ids, ["provider", "mode", "model", "thinking_effort"]);

        assert!(opts[0].is_adjustable());
        assert!(opts[1].is_adjustable());
        // One value is a fact, not a control — see `is_adjustable`.
        assert!(!opts[2].is_adjustable());
        assert!(!opts[3].is_adjustable());
        assert_eq!(opts[2].current_label(), Some("Claude Opus 5"));
        assert!(opts[3]
            .description
            .as_deref()
            .is_some_and(|d| d.starts_with("Controls reasoning effort")));
    }

    /// `configOptions` is absent whenever the session has no provider/model
    /// yet, so the sheet has to survive an empty set rather than assume one.
    /// goose 1.46.0 keys these on `id`, not `configId`. Measured against a
    /// real `goose serve`, not imagined: the payload below is the shape that
    /// server actually answered `session/new` with, trimmed to one entry.
    ///
    /// Before the alias this did not degrade — `config_id` is the struct's
    /// only required field, so the whole response failed to deserialize and
    /// the app could not open a session at all.
    #[test]
    fn a_newer_goose_keys_config_options_on_id() {
        let raw = json!({"configOptions": [
            {"id": "mode", "name": "Mode", "category": "mode", "type": "select",
             "currentValue": "auto",
             "options": [{"value": "auto", "name": "auto",
                          "description": "Automatically approve tool calls"},
                         {"value": "approve", "name": "approve",
                          "description": "Ask before every tool call"}]}
        ]});
        let opts = config_options_from(&raw);
        assert_eq!(opts.len(), 1, "the `id` spelling must parse");
        assert_eq!(opts[0].config_id, "mode");
        assert_eq!(opts[0].current_value.as_deref(), Some("auto"));
        assert_eq!(opts[0].options.len(), 2);
    }

    /// And the older spelling has to keep working, because which server a
    /// phone reaches is not this crate's choice.
    #[test]
    fn the_older_config_id_spelling_still_parses() {
        let raw = json!({"configOptions": [
            {"configId": "mode", "name": "Mode", "type": "select", "currentValue": "auto"}
        ]});
        let opts = config_options_from(&raw);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].config_id, "mode");
    }

    #[test]
    fn missing_config_options_is_an_empty_set() {
        assert!(config_options_from(&json!({"sessionId": "x"})).is_empty());
    }
}
