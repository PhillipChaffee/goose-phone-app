//! Base ACP: the methods every agent must implement, and the session config
//! options goose builds on top of them.

use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{json, Value};

use crate::rpc::{session_update, Out};
use crate::state::{now_epoch, stamp, Fixtures, Kind, SessionConfig, SessionData, Shared};

use super::Handled;

pub(crate) fn handle(method: &str, params: &Value, state: &Shared, out: &Out) -> Handled {
    let result = match method {
        "initialize" => Ok(initialize()),
        "session/new" => session_new(params, state),
        "session/load" => session_load(params, state, out),
        "session/set_config_option" => set_config_option(params, state, out),
        "session/list" => list_sessions(params, state),
        "_goose/unstable/session/rename" => rename_session(params, state),
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
        let at = stamp(now_epoch());
        let mut s = state.lock().unwrap();
        let sid = s.mint_session_id(&at.day);
        s.sessions.insert(
            sid.clone(),
            SessionData {
                cwd: cwd.to_string(),
                created_at: at.rfc3339.clone(),
                updated_at: at.rfc3339.clone(),
                sort_at: at.rfc3339,
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

/// Give a session the title a person typed.
///
/// goose renames through its session store, so the two things that happen are
/// the ones a store does: the name changes and `updated_at` moves to now. The
/// list *order* does not follow, because it sorts on the last message rather
/// than on `updated_at` — a renamed session stays exactly where it was, and
/// the app has to be able to see that here.
fn rename_session(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "sessionId must be a string".to_string()))?;
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "title must be a string".to_string()))?;

    let updated_at = stamp(now_epoch()).rfc3339;
    let renamed = {
        let mut s = state.lock().unwrap();
        s.sessions.get_mut(sid).map(|data| {
            data.title = title.to_string();
            data.user_set_name = true;
            data.updated_at = updated_at;
        })
    };
    match renamed {
        Some(()) => Ok(json!({})),
        // The real rename is a bare UPDATE with no existence check in front
        // of it, so an id nobody has surfaces as the store's own failure.
        None => Err((
            -32603,
            format!("failed to rename session: no session {sid}"),
        )),
    }
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
    user_set_name: bool,
    created_at: String,
    updated_at: String,
    sort_at: String,
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

/// Whether the caller asked for a last-message snippet on every row.
///
/// goose leaves it off unless `_meta.goose.includeLastMessageSnippet` says
/// otherwise — it is an extra join — and a mock that handed the snippet out
/// unasked would let a client that forgot to ask look correct right up until
/// the rows go quiet against a real server.
fn wants_snippet(meta: Option<&Value>) -> Result<bool, (i64, String)> {
    let Some(goose) = meta.and_then(|meta| meta.get("goose")) else {
        return Ok(false);
    };
    if goose.is_null() {
        return Ok(false);
    }
    if !goose.is_object() {
        return Err((-32602, "goose must be an object".to_string()));
    }
    match goose.get("includeLastMessageSnippet") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(wanted)) => Ok(*wanted),
        Some(_) => Err((
            -32602,
            "goose.includeLastMessageSnippet must be a boolean".to_string(),
        )),
    }
}

/// The mock's stand-in for goose's `session_list_filter_hash`.
///
/// goose SHA-256s the effective filter set, embeds the digest in every cursor
/// it hands out, and refuses a cursor whose digest does not match the filters
/// it arrives beside. Which digest is not the part a client can get wrong —
/// carrying a cursor across a filter change is — so this uses the standard
/// library's hasher and spends the fidelity where it counts: same coupling,
/// same error, same message. A cursor never outlives the process that minted
/// it, which is exactly as long as `DefaultHasher` promises to agree with
/// itself.
fn filter_hash(kinds: &[Kind], keyword: Option<&str>) -> String {
    let mut names: Vec<&str> = kinds.iter().copied().map(Kind::as_wire).collect();
    names.sort_unstable();
    let mut hasher = DefaultHasher::new();
    (names, keyword).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// goose pages 50 at a time. Three is enough to put two more pages behind the
/// seeded sessions, which is the only reason the mock pages at all.
const SESSION_PAGE_SIZE: usize = 3;

/// The `session/list` payload: sessions that have messages, filtered by kind
/// and keyword, most recently spoken in first, one page at a time.
fn list_sessions(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let meta = params.get("_meta");
    let kinds = requested_kinds(meta)?;
    let keyword = requested_keyword(meta);
    let snippets = wants_snippet(meta)?;
    let hash = filter_hash(&kinds, keyword.as_deref());

    let after = match params.get("cursor").and_then(Value::as_str) {
        None => None,
        Some(cursor) => {
            let (id, minted_under) = cursor
                .rsplit_once('.')
                .ok_or_else(|| (-32602, "malformed session list cursor".to_string()))?;
            if minted_under != hash {
                return Err((
                    -32602,
                    "session list cursor does not match filters".to_string(),
                ));
            }
            Some(id.to_string())
        }
    };

    let mut listed: Vec<Listed> = {
        let mut s = state.lock().unwrap();
        // One failure, then the truth: a list that answered `-32603` forever
        // would be a screen the app could never be driven past, and the error
        // state worth testing is the one a pull-to-refresh clears.
        if matches!(s.fixtures, Fixtures::Broken) && !s.session_list_failed {
            s.session_list_failed = true;
            return Err((
                -32603,
                "failed to read the session store: database is locked".to_string(),
            ));
        }
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
                user_set_name: d.user_set_name,
                created_at: d.created_at.clone(),
                updated_at: d.updated_at.clone(),
                sort_at: d.sort_at.clone(),
            })
            .collect()
    };
    // goose's `ORDER BY sort_timestamp DESC, s.id DESC`, where the sort
    // timestamp is the last message rather than `updated_at`. The id breaks
    // the tie, and it has to: two sessions written in the same second would
    // otherwise be ordered differently on either side of a page boundary, and
    // the cursor would skip one.
    listed.sort_by(|a, b| (&b.sort_at, &b.id).cmp(&(&a.sort_at, &a.id)));

    // The cursor names the last session of the previous page, so the next one
    // starts after it.
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
        Some(last) if more => Value::String(format!("{}.{hash}", last.id)),
        _ => Value::Null,
    };

    let sessions: Vec<Value> = listed
        .into_iter()
        .map(|d| {
            let mut meta = json!({
                "messageCount": d.message_count,
                "createdAt": d.created_at,
                "lastMessageAt": d.sort_at,
                "userSetName": d.user_set_name,
                "sessionType": d.kind.as_wire(),
                "hasRecipe": false,
            });
            if snippets {
                meta["lastMessageSnippet"] = Value::String(d.snippet);
            }
            json!({
                "sessionId": d.id,
                "cwd": d.cwd,
                "additionalDirectories": [],
                "title": if d.title.is_empty() { Value::Null } else { Value::String(d.title) },
                "updatedAt": d.updated_at,
                "_meta": meta,
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

    /// Ids are minted from the clock, so the fixtures are named by their
    /// titles here — which is also what a failure has to print to be worth
    /// reading.
    fn titles(page: &Value) -> Vec<String> {
        page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap_or("<untitled>").to_string())
            .collect()
    }

    fn session_ids(page: &Value) -> Vec<String> {
        page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["sessionId"].as_str().unwrap().to_string())
            .collect()
    }

    /// Every page of a filter, walked the way a client walks them: the cursor
    /// from the page before, and the same `_meta` every time.
    fn walk(state: &Shared, meta: &Value) -> Vec<Value> {
        let mut params = json!({ "_meta": meta });
        let mut pages = Vec::new();
        loop {
            let page = list_sessions(&params, state).unwrap();
            let cursor = page["nextCursor"].clone();
            pages.push(page);
            let Some(cursor) = cursor.as_str() else {
                return pages;
            };
            params["cursor"] = Value::String(cursor.to_string());
            assert!(pages.len() < 20, "the cursor never ran out");
        }
    }

    fn walked_titles(state: &Shared, meta: &Value) -> Vec<String> {
        walk(state, meta).iter().flat_map(titles).collect()
    }

    /// The filter the app spent its life sending, and the sessions it hid.
    #[test]
    fn types_selects_the_kinds_asked_for() {
        let state = seeded();

        let users = walked_titles(&state, &json!({"types": ["user"]}));
        assert!(
            users.contains(&"Seeded example chat".to_string()),
            "{users:?}"
        );
        assert!(!users.contains(&"Nightly dependency audit".to_string()));

        let scheduled = walk(&state, &json!({"types": ["scheduled"]}));
        assert_eq!(
            titles(&scheduled[0]),
            ["Nightly dependency audit", "Weekly changelog digest"]
        );
        assert_eq!(
            scheduled[0]["sessions"][0]["_meta"]["sessionType"],
            "scheduled"
        );

        let agents = walked_titles(&state, &json!({"types": ["acp"]}));
        assert_eq!(agents, ["Sub-agent: summarise the audit"]);

        // No filter at all is all three, which is goose's reading of it.
        let all = walked_titles(&state, &json!({}));
        assert_eq!(
            all.len(),
            users.len() + scheduled[0]["sessions"].as_array().unwrap().len() + 1
        );
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

        let audit = walked_titles(&state, &json!({"query": "AUDIT"}));
        assert_eq!(
            audit,
            ["Nightly dependency audit", "Sub-agent: summarise the audit"]
        );

        // In the messages and nowhere near the title, which is the half of
        // the search a title-matching mock would quietly lose.
        let deep = walked_titles(&state, &json!({"query": "cargo.toml"}));
        assert_eq!(deep, ["Seeded example chat"]);

        // Two words are ORed, not ANDed: this is two different sessions, one
        // per term.
        let either = walked_titles(&state, &json!({"query": "certificate scrolled"}));
        assert_eq!(either.len(), 2, "{either:?}");

        let nothing = walked_titles(&state, &json!({"query": "zzz"}));
        assert!(nothing.is_empty(), "{nothing:?}");
    }

    /// A title matches nothing on its own: the search reads messages, and the
    /// long-title fixture's words appear in both, so the *other* direction is
    /// the one worth pinning.
    #[test]
    fn a_title_only_word_finds_nothing() {
        let state = seeded();
        // "Weekly changelog digest" says "changelog" in its title and in its
        // message; "digest" is title-only.
        assert!(walked_titles(&state, &json!({"query": "digest"})).is_empty());
        assert_eq!(
            walked_titles(&state, &json!({"query": "changelog"})),
            ["Weekly changelog digest"]
        );
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
        let cursor = first["nextCursor"].as_str().unwrap().to_string();

        let second = list_sessions(&json!({"cursor": cursor, "_meta": all}), &state).unwrap();
        assert_eq!(
            second["sessions"].as_array().unwrap().len(),
            SESSION_PAGE_SIZE
        );

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

        // And a cursor from nowhere is refused before it is even compared.
        let err = list_sessions(&json!({"cursor": "nonsense", "_meta": all}), &state).unwrap_err();
        assert_eq!(err.0, -32602);
        assert!(err.1.contains("malformed"), "{}", err.1);
    }

    /// A blank search is no search, so the cursor it mints is the unfiltered
    /// one — the client trims the query before it hashes, and the two sides
    /// have to agree about that or every second page 500s.
    #[test]
    fn a_blank_query_hashes_as_no_query() {
        let state = seeded();
        let first = list_sessions(&json!({"_meta": {"query": "   "}}), &state).unwrap();
        let cursor = first["nextCursor"].as_str().unwrap().to_string();
        assert!(list_sessions(&json!({"cursor": cursor, "_meta": {}}), &state).is_ok());
    }

    /// The pages tile the list: every session once, in order, and the last
    /// page ends the chain rather than handing out a cursor to nothing.
    #[test]
    fn pages_tile_the_list_without_repeating_or_skipping() {
        let state = seeded();
        let pages = walk(&state, &json!({}));
        assert!(pages.len() > 2, "the fixtures should need three pages");

        let mut seen: Vec<String> = Vec::new();
        for (index, page) in pages.iter().enumerate() {
            let ids = session_ids(page);
            let last = index + 1 == pages.len();
            assert!(
                if last {
                    !ids.is_empty()
                } else {
                    ids.len() == SESSION_PAGE_SIZE
                },
                "page {index} has {} sessions",
                ids.len()
            );
            assert_eq!(last, page["nextCursor"].is_null());
            seen.extend(ids);
        }

        let total = state.lock().unwrap().sessions.len();
        assert_eq!(seen.len(), total);
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "a session was listed twice");
    }

    /// The snippet is opt-in, exactly as goose has it: asking is what a
    /// client has to remember to do, so not asking has to be visible.
    #[test]
    fn the_snippet_is_only_sent_when_asked_for() {
        let state = seeded();
        let quiet = list_sessions(&json!({"_meta": {}}), &state).unwrap();
        assert!(quiet["sessions"][0]["_meta"]["lastMessageSnippet"].is_null());

        let asked = list_sessions(
            &json!({"_meta": {"goose": {"includeLastMessageSnippet": true}}}),
            &state,
        )
        .unwrap();
        assert!(asked["sessions"][0]["_meta"]["lastMessageSnippet"].is_string());

        let err = list_sessions(
            &json!({"_meta": {"goose": {"includeLastMessageSnippet": "yes"}}}),
            &state,
        )
        .unwrap_err();
        assert_eq!(err.0, -32602);
    }

    /// Rename is only real if the list agrees afterwards.
    #[test]
    fn a_rename_is_the_title_the_next_list_shows() {
        let state = seeded();
        let before = list_sessions(&json!({"_meta": {}}), &state).unwrap();
        let sid = session_ids(&before)[1].clone();
        assert_eq!(before["sessions"][1]["_meta"]["userSetName"], false);

        rename_session(
            &json!({"sessionId": sid, "title": "Cert rotation, again"}),
            &state,
        )
        .unwrap();

        let after = list_sessions(&json!({"_meta": {}}), &state).unwrap();
        assert_eq!(after["sessions"][1]["title"], "Cert rotation, again");
        assert_eq!(after["sessions"][1]["_meta"]["userSetName"], true);
        // Renaming touches `updated_at` but not the last message, and the
        // list sorts on the latter — so the row keeps its place. A UI that
        // assumed otherwise would scroll out from under the person typing.
        assert_eq!(session_ids(&after), session_ids(&before));
    }

    /// Renaming a session nobody has is the store's failure, not a silent
    /// success — a client that renamed the wrong id would otherwise never
    /// find out.
    #[test]
    fn renaming_an_unknown_session_fails() {
        let err = rename_session(
            &json!({"sessionId": "19700101_1", "title": "Nowhere"}),
            &seeded(),
        )
        .unwrap_err();
        assert_eq!(err.0, -32603);
        assert!(err.1.contains("19700101_1"), "{}", err.1);

        let err = rename_session(&json!({"sessionId": "19700101_1"}), &seeded()).unwrap_err();
        assert_eq!(err.0, -32602);
    }

    /// `broken` fixtures fail the first list and then behave, which is the
    /// shape the app's error path needs: something to show, and a refresh
    /// that clears it.
    #[test]
    fn broken_fixtures_fail_the_first_list_only() {
        let state: Shared = std::sync::Arc::new(std::sync::Mutex::new(State {
            fixtures: Fixtures::Broken,
            ..State::default()
        }));
        crate::state::seed(&state);

        let err = list_sessions(&json!({"_meta": {}}), &state).unwrap_err();
        assert_eq!(err.0, -32603);
        assert!(err.1.contains("session store"), "{}", err.1);

        assert!(!titles(&list_sessions(&json!({"_meta": {}}), &state).unwrap()).is_empty());
    }
}
