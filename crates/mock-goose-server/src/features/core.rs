//! Base ACP: the methods every agent must implement, and the session config
//! options goose builds on top of them.

use serde_json::{json, Value};

use crate::rpc::{session_update, Out};
use crate::state::{SessionConfig, SessionData, Shared};

use super::Handled;

pub(crate) fn handle(method: &str, params: &Value, state: &Shared, out: &Out) -> Handled {
    let result = match method {
        "initialize" => Ok(initialize()),
        "session/new" => session_new(params, state),
        "session/load" => session_load(params, state, out),
        "session/set_config_option" => set_config_option(params, state, out),
        "session/list" => Ok(json!({"sessions": list_sessions(state), "nextCursor": null})),
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

/// The `session/list` payload: every session that has messages, newest
/// session id first.
fn list_sessions(state: &Shared) -> Vec<Value> {
    let mut listed: Vec<Listed> = {
        let s = state.lock().unwrap();
        s.sessions
            .iter()
            .filter(|(_, d)| d.message_count > 0)
            .map(|(id, d)| Listed {
                id: id.clone(),
                cwd: d.cwd.clone(),
                title: d.title.clone(),
                message_count: d.message_count,
                snippet: d.snippet.clone(),
            })
            .collect()
    };
    listed.sort_by(|a, b| b.id.cmp(&a.id));
    listed
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
                    "sessionType": "user",
                    "hasRecipe": false,
                    "lastMessageSnippet": d.snippet,
                }
            })
        })
        .collect()
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
}
