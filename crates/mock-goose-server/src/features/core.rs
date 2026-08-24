//! Base ACP: the methods every agent must implement, and the session config
//! options goose builds on top of them.

use serde_json::{json, Value};

use crate::rpc::{session_update, Out};
use crate::state::{Kind, SessionConfig, SessionData, Shared};

use super::Handled;

pub(crate) fn handle(method: &str, params: &Value, state: &Shared, out: &Out) -> Handled {
    let result = match method {
        "initialize" => Ok(initialize()),
        "session/new" => session_new(params, state),
        "session/load" => session_load(params, state, out),
        "session/set_config_option" => set_config_option(params, state, out),
        "session/list" => list_sessions(params, state),
        "session/delete" => {
            let sid = params
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("");
            state.lock().unwrap().sessions.remove(sid);
            Ok(json!({}))
        }
        "session/close" => Ok(json!({})),
        _ => return None,
    };
    Some(result)
}

fn initialize() -> Value {
    json!({
        "protocolVersion": 1,
        "agentInfo": {"name": "goose-mock", "version": "1.47.0"},
        "agentCapabilities": {
            "loadSession": true,
            "sessionCapabilities": {"list": {}, "delete": {}, "close": {}},
            "promptCapabilities": {"image": true, "embeddedContext": true}
        },
        "authMethods": []
    })
}

fn session_new(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let cwd = params
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !cwd.starts_with('/') {
        return Err((-32602, format!("cwd must be an absolute path, got `{cwd}`")));
    }
    let sid = {
        let mut s = state.lock().unwrap();
        let n = s.next_session;
        s.next_session += 1;
        let sid = format!("20260821_{n}");
        s.sessions.insert(
            sid.clone(),
            SessionData {
                cwd: cwd.to_string(),
                ..Default::default()
            },
        );
        sid
    };
    let config = state.lock().unwrap().config.clone();
    Ok(json!({"sessionId": sid, "modes": null, "configOptions": config_options(&config)}))
}

fn session_load(params: &Value, state: &Shared, out: &Out) -> Result<Value, (i64, String)> {
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let data = state.lock().unwrap().sessions.get(sid).cloned();
    match data {
        Some(data) => {
            for update in &data.conversation {
                session_update(out, sid, update);
            }
            let config = state.lock().unwrap().config.clone();
            Ok(json!({"modes": null, "configOptions": config_options(&config)}))
        }
        None => Err((-32002, format!("session not found: {sid}"))),
    }
}

fn set_config_option(params: &Value, state: &Shared, out: &Out) -> Result<Value, (i64, String)> {
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let config_id = params.get("configId").and_then(Value::as_str).unwrap_or("");
    let value = params.get("value").and_then(Value::as_str).unwrap_or("");
    let config = {
        let mut s = state.lock().unwrap();
        match config_id {
            "provider" => s.config.provider = value.to_string(),
            "mode" => s.config.mode = value.to_string(),
            "model" => {
                s.config.model = value.to_string();
                // Effort is a property of the model: switching to one
                // that cannot reason drops the session back to `off`,
                // exactly as goose's response builder does.
                if !is_reasoning_model(value) {
                    s.config.thinking_effort = "off".to_string();
                }
            }
            "thinking_effort" => s.config.thinking_effort = value.to_string(),
            other => return Err((-32602, format!("Unsupported config option: {other}"))),
        }
        s.config.clone()
    };
    let opts = config_options(&config);
    // The real agent pushes this after every change so a second
    // client watching the same session stays in step.
    session_update(
        out,
        sid,
        &json!({"sessionUpdate": "config_option_update", "configOptions": opts}),
    );
    Ok(json!({"configOptions": opts}))
}

/// The fields `session/list` reports, copied out under the lock so the JSON
/// is built without holding it.
struct Listed {
    id: String,
    cwd: String,
    title: String,
    message_count: u64,
    snippet: String,
    kind: Kind,
}

/// Whether the model takes an extended-thinking effort at all.
///
/// The distinction is the point: goose offers the five effort tiers only for
/// a reasoning model and collapses to a lone `off` otherwise, which is what
/// makes the app's fact-row path reachable without a real provider.
fn is_reasoning_model(model: &str) -> bool {
    model != "qwen3-coder-480b"
}

/// The `configOptions` array a real agent returns, in the shape ACP schema
/// 1.5 defines: a flattened kind tagged by `type`, an optional `description`,
/// and select options keyed on `value`.
///
/// All four options goose builds, in its order — `session/set_config_option`
/// routes exactly these ids, so a fifth here would be a control the real
/// agent rejects.
fn config_options(config: &SessionConfig) -> Value {
    let efforts: Value = if is_reasoning_model(&config.model) {
        json!([
            {"value": "off", "name": "off"},
            {"value": "low", "name": "low"},
            {"value": "medium", "name": "medium"},
            {"value": "high", "name": "high"},
            {"value": "max", "name": "max"},
        ])
    } else {
        json!([{"value": "off", "name": "off"}])
    };
    json!([
        {
            "configId": "provider",
            "name": "Provider",
            "type": "select",
            "currentValue": config.provider,
            "options": [
                {"value": "anthropic", "name": "Anthropic"},
                {"value": "openai", "name": "OpenAI"},
            ]
        },
        {
            "configId": "mode",
            "name": "Mode",
            "category": "mode",
            "type": "select",
            "currentValue": config.mode,
            "options": [
                {"value": "auto", "name": "Auto", "description": "Run tools without asking."},
                {"value": "approve", "name": "Manual approval",
                 "description": "Ask before every tool call."},
                {"value": "chat", "name": "Chat only", "description": "No tools at all."},
            ]
        },
        {
            "configId": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": config.model,
            "options": [
                {"value": "claude-opus-5", "name": "Claude Opus 5"},
                {"value": "claude-sonnet-5", "name": "Claude Sonnet 5"},
                {"value": "gpt-5.2", "name": "GPT-5.2"},
                {"value": "qwen3-coder-480b", "name": "Qwen3 Coder 480B"},
            ]
        },
        {
            "configId": "thinking_effort",
            "name": "Thinking effort",
            "category": "thought_level",
            "type": "select",
            "description":
                "Controls reasoning effort for models that support extended thinking.",
            "currentValue": config.thinking_effort,
            "options": efforts
        }
    ])
}

/// The kinds asked for in `_meta.types`. Absent, null or empty means all of
/// them; anything outside the three is `invalid_params`, which is goose's
/// behaviour and not a nicety — a filter it silently ignored would list
/// sessions the caller asked it not to.
fn requested_kinds(meta: Option<&Value>) -> Result<Vec<Kind>, (i64, String)> {
    let Some(value) = meta.and_then(|meta| meta.get("types")) else {
        return Ok(Kind::ALL.to_vec());
    };
    if value.is_null() {
        return Ok(Kind::ALL.to_vec());
    }
    let Some(items) = value.as_array() else {
        return Err((
            -32602,
            "types must be an array of session type strings".to_string(),
        ));
    };
    if items.is_empty() {
        return Ok(Kind::ALL.to_vec());
    }
    items
        .iter()
        .map(|item| {
            item.as_str().and_then(Kind::from_wire).ok_or_else(|| {
                (
                    -32602,
                    "types may only include user, scheduled, or acp".into(),
                )
            })
        })
        .collect()
}

/// `_meta.query`, trimmed and lower-cased; a blank one is no search at all.
fn requested_keyword(meta: Option<&Value>) -> Option<String> {
    let keyword = meta?.get("query")?.as_str()?.trim();
    (!keyword.is_empty()).then(|| keyword.to_lowercase())
}

/// Whether a session's *messages* contain any of the search words.
///
/// goose splits the query on whitespace and ORs the words, and looks only at
/// the text parts of user-visible messages — the title is not searched at
/// all. Worth copying exactly: a mock that matched titles would make a
/// title-shaped search box look right and then find nothing against goose.
fn matches_keyword(conversation: &[Value], keyword: &str) -> bool {
    let terms: Vec<&str> = keyword.split_whitespace().collect();
    conversation
        .iter()
        .filter_map(|update| update.pointer("/content/text").and_then(Value::as_str))
        .any(|text| {
            let text = text.to_lowercase();
            terms.iter().any(|term| text.contains(term))
        })
}

/// The mock's stand-in for goose's `session_list_filter_hash`: whatever it
/// is, a cursor carries it and is refused beside a different filter set. The
/// value being readable rather than a SHA-256 is the only difference, and it
/// makes a failure legible in a test.
fn filter_key(kinds: &[Kind], keyword: Option<&str>) -> String {
    let mut names: Vec<&str> = kinds.iter().map(|kind| kind.as_wire()).collect();
    names.sort_unstable();
    names.dedup();
    format!("{}|{}", names.join(","), keyword.unwrap_or_default())
}

/// goose pages 50 at a time. Two is enough to put a second page behind a
/// handful of seeded sessions, which is the only reason the mock pages at all.
const SESSION_PAGE_SIZE: usize = 2;

/// The `session/list` payload: sessions that have messages, filtered by kind
/// and keyword, newest session id first, one page at a time.
fn list_sessions(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let meta = params.get("_meta");
    let kinds = requested_kinds(meta)?;
    let keyword = requested_keyword(meta);
    let key = filter_key(&kinds, keyword.as_deref());

    let after = match params.get("cursor").and_then(Value::as_str) {
        None => None,
        Some(cursor) => {
            let (id, cursor_key) = cursor
                .split_once('#')
                .ok_or_else(|| (-32602, "malformed session list cursor".to_string()))?;
            if cursor_key != key {
                return Err((
                    -32602,
                    "session list cursor does not match filters".to_string(),
                ));
            }
            Some(id.to_string())
        }
    };

    let mut listed: Vec<Listed> = {
        let s = state.lock().unwrap();
        s.sessions
            .iter()
            .filter(|(_, d)| d.message_count > 0 && kinds.contains(&d.kind))
            .filter(|(_, d)| {
                keyword
                    .as_ref()
                    .is_none_or(|keyword| matches_keyword(&d.conversation, keyword))
            })
            .map(|(id, d)| Listed {
                id: id.clone(),
                cwd: d.cwd.clone(),
                title: d.title.clone(),
                message_count: d.message_count,
                snippet: d.snippet.clone(),
                kind: d.kind,
            })
            .collect()
    };
    listed.sort_by(|a, b| b.id.cmp(&a.id));

    // The cursor names the last session of the previous page, so the next one
    // starts after it — the same (sort key, id) walk goose does, minus the
    // timestamps the mock does not vary.
    if let Some(after) = after {
        let start = listed
            .iter()
            .position(|d| d.id == after)
            .map_or(listed.len(), |index| index + 1);
        listed.drain(..start);
    }
    let more = listed.len() > SESSION_PAGE_SIZE;
    listed.truncate(SESSION_PAGE_SIZE);
    let next_cursor = match listed.last() {
        Some(last) if more => Value::String(format!("{}#{key}", last.id)),
        _ => Value::Null,
    };

    let sessions: Vec<Value> = listed
        .into_iter()
        .map(|d| {
            json!({
                "sessionId": d.id,
                "cwd": d.cwd,
                "additionalDirectories": [],
                "title": if d.title.is_empty() { Value::Null } else { Value::String(d.title) },
                "updatedAt": "2026-08-21T12:00:00Z",
                "_meta": {
                    "messageCount": d.message_count,
                    "createdAt": "2026-08-21T09:00:00Z",
                    "userSetName": false,
                    "sessionType": d.kind.as_wire(),
                    "hasRecipe": false,
                    "lastMessageSnippet": d.snippet,
                }
            })
        })
        .collect();
    Ok(json!({"sessions": sessions, "nextCursor": next_cursor}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::State;

    fn ids(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|o| o["configId"].as_str().unwrap().to_string())
            .collect()
    }

    fn effort_values(v: &Value) -> usize {
        let effort = &v.as_array().unwrap()[3];
        assert_eq!(effort["configId"], "thinking_effort");
        effort["options"].as_array().unwrap().len()
    }

    /// Exactly the ids goose routes — no more, since the real agent answers
    /// `invalid_params` to anything else, and no fewer, since the app renders
    /// whatever arrives instead of naming ids of its own.
    #[test]
    fn offers_the_four_options_goose_routes() {
        let config = State::default().config;
        assert_eq!(
            ids(&config_options(&config)),
            ["provider", "mode", "model", "thinking_effort"]
        );
    }

    /// The edge case the app's fact row exists for: a model that cannot
    /// reason leaves exactly one effort to "choose" between.
    #[test]
    fn a_non_reasoning_model_collapses_effort_to_one_value() {
        let mut config = State::default().config;
        config.model = "qwen3-coder-480b".to_string();
        assert_eq!(effort_values(&config_options(&config)), 1);

        config.model = "claude-opus-5".to_string();
        assert_eq!(effort_values(&config_options(&config)), 5);
    }

    fn seeded() -> Shared {
        let state: Shared = std::sync::Arc::new(std::sync::Mutex::new(State::default()));
        crate::state::seed(&state);
        state
    }

    fn session_ids(page: &Value) -> Vec<String> {
        page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["sessionId"].as_str().unwrap().to_string())
            .collect()
    }

    /// The filter the app spent its life sending, and the two seeded sessions
    /// it hid.
    #[test]
    fn types_selects_the_kinds_asked_for() {
        let state = seeded();
        let users = list_sessions(&json!({"_meta": {"types": ["user"]}}), &state).unwrap();
        assert_eq!(session_ids(&users), ["20260820_1"]);

        let scheduled = list_sessions(&json!({"_meta": {"types": ["scheduled"]}}), &state).unwrap();
        assert_eq!(session_ids(&scheduled), ["20260819_1"]);
        assert_eq!(
            scheduled["sessions"][0]["_meta"]["sessionType"],
            "scheduled"
        );

        // No filter at all is all three, which is goose's reading of it.
        let all = list_sessions(&json!({}), &state).unwrap();
        assert_eq!(session_ids(&all), ["20260820_1", "20260819_1"]);
    }

    /// goose refuses a type outside the three rather than ignoring it, and a
    /// mock that ignored it would let a client ship a filter the real server
    /// rejects.
    #[test]
    fn an_unknown_type_is_invalid_params() {
        let err = list_sessions(&json!({"_meta": {"types": ["hidden"]}}), &seeded()).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("user, scheduled, or acp"), "{}", err.1);
    }

    /// The search is over message text, case-insensitively, across every
    /// kind — goose does not search titles, so neither does this.
    #[test]
    fn query_searches_message_text() {
        let state = seeded();
        let audit = list_sessions(&json!({"_meta": {"query": "AUDIT"}}), &state).unwrap();
        assert_eq!(session_ids(&audit), ["20260819_1", "20260818_1"]);

        let deep = list_sessions(&json!({"_meta": {"query": "cargo.toml"}}), &state).unwrap();
        assert_eq!(session_ids(&deep), ["20260820_1"]);

        let nothing = list_sessions(&json!({"_meta": {"query": "zzz"}}), &state).unwrap();
        assert!(session_ids(&nothing).is_empty());
    }

    /// The trap this feature is built around: a cursor is minted under a
    /// filter set and is refused beside any other one. Reproduced here so a
    /// client that carries a stale cursor fails against the mock exactly the
    /// way it fails against goose, rather than at the demo.
    #[test]
    fn a_cursor_is_refused_beside_different_filters() {
        let state = seeded();
        let all = json!({"types": ["user", "scheduled", "acp"]});
        let first = list_sessions(&json!({"_meta": all}), &state).unwrap();
        assert_eq!(session_ids(&first), ["20260820_1", "20260819_1"]);
        let cursor = first["nextCursor"].as_str().unwrap().to_string();

        let second = list_sessions(&json!({"cursor": cursor, "_meta": all}), &state).unwrap();
        assert_eq!(session_ids(&second), ["20260818_1"]);
        assert_eq!(second["nextCursor"], Value::Null);

        let err = list_sessions(
            &json!({"cursor": cursor, "_meta": {"types": ["user"]}}),
            &state,
        )
        .unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("does not match filters"), "{}", err.1);

        // Same kinds, a search added: still a different filter set.
        let err = list_sessions(
            &json!({"cursor": cursor, "_meta": {"types": ["user", "scheduled", "acp"],
                                               "query": "audit"}}),
            &state,
        )
        .unwrap_err();
        assert_eq!(err.0, -32602);
    }
}
