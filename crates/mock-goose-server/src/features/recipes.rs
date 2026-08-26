//! Recipes: `_goose/unstable/recipes/*`, backed by three canned YAML files
//! that never touch a disk.
//!
//! The entries are held as [`Value`], not as Rust structs, and that is the
//! point of this file. goose sends fields this repo's client deliberately does
//! not model — `sub_recipes`, `retry`, `extensions` — and a mock that parsed
//! them into a struct of its own would re-serialize whatever that struct
//! happened to know about, quietly answering with a *different* recipe than
//! the fixture says. Keeping the JSON as JSON means a `list` -> edit ->
//! `schedule` round trip through the client's `extra` map has something real
//! to fail against.
//!
//! # Exact spellings only
//!
//! Every request field is read by the one key goose spells it with —
//! `file_path`, `cron_schedule`, `id` — and no alternative is accepted. A mock
//! that also answered to `filePath` would let a client that mis-spells a key
//! pass here and then fail against the real server, which is the entire bug
//! this design exists to prevent. Unknown keys are *ignored* rather than
//! rejected, because that is what serde does on the real server: a client
//! sending `cronSchedule` gets a success that scheduled nothing, here and
//! there alike.
//!
//! # State
//!
//! The store is a process-wide [`LazyLock`] rather than a field on
//! [`crate::state::State`]. Recipes are files on the server's disk: every
//! connection sees the same ones, and a schedule set over one socket is
//! visible over the next. Keeping it here also keeps the whole feature to one
//! file, so the five feature branches landing in parallel do not all edit
//! `state.rs`.

use std::sync::{LazyLock, Mutex};

use serde_json::{json, Value};

use crate::rpc::Out;
use crate::state::{Fixtures, Shared};

// `scheduler_disabled` is shared with `features::scheduler`: `recipes/schedule`
// and every `schedules/*` method go through the same `require_scheduler` on
// the real server, so the sentence they answer with is one contract string and
// not two copies.
use super::{scheduler_disabled, Handled};

/// Cheap gate so the two locks below are only taken for a method that could
/// possibly be ours: `handle` sits on the path of every request `core` did not
/// answer.
const PREFIX: &str = "_goose/unstable/recipes/";

const LIST: &str = "_goose/unstable/recipes/list";
const DELETE: &str = "_goose/unstable/recipes/delete";
const SCAN: &str = "_goose/unstable/recipes/scan";
const SCHEDULE: &str = "_goose/unstable/recipes/schedule";
const ENCODE: &str = "_goose/unstable/recipes/encode";

static RECIPES: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::seeded()));

pub(crate) fn handle(method: &str, params: &Value, state: &Shared, _out: &Out) -> Handled {
    if !method.starts_with(PREFIX) {
        return None;
    }
    let (fixtures, no_scheduler) = {
        let state = state.lock().unwrap();
        (state.fixtures, state.no_scheduler)
    };
    answer(
        method,
        params,
        fixtures,
        no_scheduler,
        &mut RECIPES.lock().unwrap(),
    )
}

/// The dispatch, over plain data so the tests can drive it without a socket or
/// the process-wide store.
///
/// `None` for a `recipes/*` method this mock does not implement — `save`,
/// `parse`, `to-yaml`, `slash-command`, `decode` — so it falls through to the
/// `-32601` in [`super::dispatch`]. That is not a gap being papered over: the
/// client wraps exactly the five below, and answering a method it never sends
/// would be inventing a contract.
fn answer(
    method: &str,
    params: &Value,
    fixtures: Fixtures,
    no_scheduler: bool,
    store: &mut Store,
) -> Handled {
    let result = match method {
        LIST => store.list(fixtures),
        DELETE => required_str(params, "id").and_then(|id| store.delete(id)),
        SCHEDULE => match (required_str(params, "id"), cron_param(params)) {
            // After the params and before the id, which is where goose checks
            // it: the request is deserialized by the dispatch layer, then
            // `on_schedule_recipe` calls `require_scheduler` before resolving
            // the id. So a scheduler-less server refuses a schedule for a
            // recipe that does not exist, and rejects a malformed cron even
            // though it could not have run it.
            (Ok(_), Ok(_)) if no_scheduler => Err(scheduler_disabled()),
            (Ok(id), Ok(cron)) => store.schedule(id, cron),
            (Err(bad), _) | (_, Err(bad)) => Err(bad),
        },
        SCAN => recipe_param(params)
            .map(|recipe| json!({"has_security_warnings": has_security_warnings(recipe)})),
        ENCODE => recipe_param(params).map(|recipe| json!({"deeplink": deeplink(recipe)})),
        _ => return None,
    };
    Some(result)
}

// ------------------------------------------------------------------ requests

/// A string field goose marks `required` in `acp-schema.json`.
fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, (i64, String)> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, format!("`{key}` is required and must be a string")))
}

/// `cron_schedule` on `recipes/schedule`, which is `["string", "null"]`.
///
/// Absent and explicit `null` are the same request — unschedule — because
/// that is how `Option<String>` deserializes on the real server. Anything
/// that is neither is a client bug worth naming rather than coercing.
fn cron_param(params: &Value) -> Result<Option<&str>, (i64, String)> {
    match params.get("cron_schedule") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cron)) => Ok(Some(cron)),
        Some(other) => Err((
            -32602,
            format!("`cron_schedule` must be a string or null, got {other}"),
        )),
    }
}

/// The `recipe` body `scan` and `encode` share.
fn recipe_param(params: &Value) -> Result<&Value, (i64, String)> {
    match params.get("recipe") {
        Some(recipe @ Value::Object(_)) => Ok(recipe),
        _ => Err((
            -32602,
            "`recipe` is required and must be an object".to_string(),
        )),
    }
}

// ------------------------------------------------------------------- the work

/// goose's own scan hunts for Unicode tag characters — an invisible
/// prompt-injection carrier that cannot be typed into a fixture, pasted into a
/// terminal, or seen in a diff. The mock triggers on `rm -rf` in
/// `instructions` instead: the same yes/no on the wire, reachable by a person
/// driving the mock by hand, and visible in the seeded recipe that carries it.
fn has_security_warnings(recipe: &Value) -> bool {
    recipe
        .get("instructions")
        .and_then(Value::as_str)
        .is_some_and(|instructions| instructions.contains("rm -rf"))
}

/// What `recipes/encode` returns: the recipe's JSON in URL-safe base64, no
/// padding, and *no* `goose://` wrapper — `recipe_deeplink::encode` hands back
/// the bare payload and the surrounding link is the caller's to build.
fn deeplink(recipe: &Value) -> String {
    base64_url_no_pad(recipe.to_string().as_bytes())
}

/// Twelve lines instead of a dependency: the mock has four, and pulling in a
/// base64 crate for one call in a test double is not a trade worth making.
fn base64_url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut group = [0u8; 3];
        group[..chunk.len()].copy_from_slice(chunk);
        let bits =
            (usize::from(group[0]) << 16) | (usize::from(group[1]) << 8) | usize::from(group[2]);
        // No padding, so a 3/2/1-byte chunk emits 4/3/2 characters: exactly
        // the ones the remaining bits actually fill.
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(bits >> (18 - 6 * i)) & 63]));
        }
    }
    out
}

// ---------------------------------------------------------------- the store

#[derive(Debug)]
struct Store {
    entries: Vec<Value>,
    /// Whether the `broken` fixture set has already spent its one failure.
    /// The point of that switch is the app's error-then-retry path, and a
    /// server that failed forever would only ever prove the first half of it.
    broken_list_served: bool,
}

impl Store {
    fn seeded() -> Self {
        Self {
            entries: seed(),
            broken_list_served: false,
        }
    }

    fn list(&mut self, fixtures: Fixtures) -> Result<Value, (i64, String)> {
        match fixtures {
            Fixtures::Empty => return Ok(json!({"recipes": []})),
            Fixtures::Broken if !self.broken_list_served => {
                self.broken_list_served = true;
                return Err((
                    -32603,
                    "Failed to list recipes: permission denied reading \
                     /home/demo/.config/goose/recipes"
                        .to_string(),
                ));
            }
            Fixtures::Full | Fixtures::Broken => {}
        }
        let mut entries = self.entries.clone();
        // goose returns the manifests already sorted by file mtime, newest
        // first, and the app renders whatever order it is given. Sorted here
        // rather than in `seed` so the seed can be stored out of order and
        // this stays a behaviour a test can catch losing.
        //
        // The timestamps are RFC 3339 with a fixed `+00:00` offset, which
        // makes lexicographic order chronological order.
        entries.sort_by(|a, b| last_modified(b).cmp(last_modified(a)));
        Ok(json!({ "recipes": entries }))
    }

    fn delete(&mut self, id: &str) -> Result<Value, (i64, String)> {
        let before = self.entries.len();
        self.entries.retain(|entry| entry_id(entry) != Some(id));
        if self.entries.len() == before {
            return Err(not_found(id));
        }
        Ok(json!({}))
    }

    fn schedule(&mut self, id: &str, cron: Option<&str>) -> Result<Value, (i64, String)> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry_id(entry) == Some(id))
            .ok_or_else(|| not_found(id))?;
        // Indexing a `Value` inserts into an object and panics on anything
        // else. Every seeded entry is an object; one that was not would be a
        // broken fixture, and failing on it beats writing the cron nowhere and
        // reporting success.
        entry["schedule_cron"] = cron.map_or(Value::Null, |cron| Value::String(cron.to_string()));
        Ok(json!({}))
    }
}

/// What goose answers for an id that resolves to no file: `invalid_params`
/// with the id in `data`, from `resolve_recipe_path_by_id`. Not `-32602` with
/// an empty reason and not a silent success — the row the user swiped is gone
/// either way, and only one of those tells them why.
fn not_found(id: &str) -> (i64, String) {
    (-32602, format!("recipe not found: {id}"))
}

fn entry_id(entry: &Value) -> Option<&str> {
    entry.get("id").and_then(Value::as_str)
}

fn last_modified(entry: &Value) -> &str {
    entry
        .get("last_modified")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

// -------------------------------------------------------------------- fixtures

/// Three recipes, deliberately out of date order, chosen so that every shape
/// the Recipes screen has to draw is reachable without editing this file:
///
/// - `daily-standup` takes no parameters and is already on a cron with a
///   slash command, so the scheduled dot and the "runs on one tap" path both
///   have a subject.
/// - `review-pr` asks for a `required` string and an `optional` select with
///   options, which is the input form.
/// - `deep-research` carries `sub_recipes` and `retry`, which the client does
///   not model, plus the longest title and description here — so it is both
///   the round-trip proof and the row the UI stress pass substitutes into. Its
///   instructions contain `rm -rf`, which is what makes `recipes/scan` return
///   a warning for something a person can actually see.
fn seed() -> Vec<Value> {
    vec![
        json!({
            "id": "1c9d4f2a6b083e57",
            "recipe": {
                "version": "1.0.0",
                "title": "Daily standup",
                "description": "Summarises yesterday's commits and the reviews waiting on me.",
                "instructions": null,
                "prompt": "Summarise what landed on main since yesterday morning, then list the pull requests waiting on me.",
                "parameters": null,
                "settings": {
                    "goose_provider": "anthropic",
                    "goose_model": "claude-sonnet-4-5",
                    "temperature": 0.2,
                    "max_turns": 12
                }
            },
            "file_path": "/home/demo/.config/goose/recipes/daily-standup.yaml",
            "last_modified": "2026-08-22T06:58:03.114927+00:00",
            "schedule_cron": "30 8 * * 1-5",
            "slash_command": "standup"
        }),
        json!({
            "id": "8e35b0a7d914c26f",
            "recipe": {
                "version": "1.0.0",
                "title": "Review a pull request",
                "description": "Reads a pull request end to end and writes the review at the depth asked for.",
                "instructions": "Read the diff before the description, and say what you could not check as well as what you did.",
                "prompt": "Review pull request {{ pr_url }} at {{ depth }} depth.",
                "parameters": [
                    {
                        "key": "pr_url",
                        "input_type": "string",
                        "requirement": "required",
                        "description": "The pull request to review.",
                        "default": null,
                        "options": null
                    },
                    {
                        "key": "depth",
                        "input_type": "select",
                        "requirement": "optional",
                        "description": "How closely to read the diff.",
                        "default": "normal",
                        "options": ["skim", "normal", "line-by-line"]
                    }
                ],
                "settings": null
            },
            "file_path": "/home/demo/.config/goose/recipes/review-pr.yaml",
            "last_modified": "2026-08-18T11:27:40.663210+00:00",
            "schedule_cron": null,
            "slash_command": null
        }),
        json!({
            "id": "4a70f6c318d2be95",
            "recipe": {
                "version": "1.0.0",
                "title": "Deep research across every source I can reach, with citations and a written brief",
                "description": "Takes a question, breaks it into sub-questions, researches each one against the web, the local repository and any documentation it can find, resolves the contradictions between sources, and writes a brief with a citation against every claim it makes.",
                "instructions": "Work one sub-question at a time and keep the sources beside the claims. Clear the scratch directory with `rm -rf ./.research-cache` before writing the final brief.",
                "prompt": "Research {{ question }} and write the brief.",
                "parameters": [
                    {
                        "key": "question",
                        "input_type": "string",
                        "requirement": "required",
                        "description": "What to research.",
                        "default": null,
                        "options": null
                    }
                ],
                "settings": null,
                "sub_recipes": [
                    {
                        "name": "gather",
                        "path": "/home/demo/.config/goose/recipes/gather-sources.yaml",
                        "values": { "depth": "3" },
                        "sequential_when_repeated": true,
                        "description": "Collect and de-duplicate the sources."
                    }
                ],
                "retry": {
                    "max_retries": 2,
                    "checks": [
                        { "type": "shell", "command": "test -s ./research-brief.md" }
                    ],
                    "on_failure": "echo 'brief was empty, retrying'",
                    "timeout_seconds": 900
                }
            },
            "file_path": "/home/demo/.config/goose/recipes/deep-research.yaml",
            "last_modified": "2026-08-23T21:14:52.771043+00:00",
            "schedule_cron": null,
            "slash_command": null
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const STANDUP: &str = "1c9d4f2a6b083e57";
    const REVIEW: &str = "8e35b0a7d914c26f";
    const RESEARCH: &str = "4a70f6c318d2be95";

    fn store() -> Store {
        Store::seeded()
    }

    /// Every handler test goes through `answer`, so the method strings are
    /// exercised rather than the functions behind them.
    fn call(store: &mut Store, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        answer(method, params, Fixtures::Full, false, store).unwrap()
    }

    fn listed(store: &mut Store) -> Vec<Value> {
        call(store, LIST, &json!({}))
            .unwrap()
            .get("recipes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap()
    }

    fn ids(store: &mut Store) -> Vec<String> {
        listed(store)
            .iter()
            .filter_map(|entry| entry_id(entry).map(str::to_string))
            .collect()
    }

    fn cron_of(store: &mut Store, id: &str) -> Value {
        listed(store)
            .into_iter()
            .find(|entry| entry_id(entry) == Some(id))
            .unwrap()["schedule_cron"]
            .clone()
    }

    /// The seed is stored out of date order on purpose: this asserts the sort,
    /// not the literal.
    #[test]
    fn the_list_is_newest_file_first() {
        assert_eq!(ids(&mut store()), [RESEARCH, STANDUP, REVIEW]);
    }

    /// The keys of a list entry, spelled out against `RecipeListEntryDto`.
    /// A fixture that drifted to `filePath` would still deserialize on the
    /// client — into a `None` and an `extra` entry nobody looks at — so the
    /// spelling has to be asserted somewhere, and this is the somewhere.
    #[test]
    fn a_list_entry_carries_exactly_the_six_fields_goose_sends() {
        for entry in listed(&mut store()) {
            let mut keys: Vec<&str> = entry
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                [
                    "file_path",
                    "id",
                    "last_modified",
                    "recipe",
                    "schedule_cron",
                    "slash_command"
                ]
            );
        }
    }

    #[test]
    fn a_parameter_carries_the_snake_case_keys() {
        let review = listed(&mut store())
            .into_iter()
            .find(|entry| entry_id(entry) == Some(REVIEW))
            .unwrap();
        let parameters = review["recipe"]["parameters"].as_array().unwrap().clone();
        assert_eq!(parameters.len(), 2);
        assert_eq!(parameters[0]["input_type"], json!("string"));
        assert_eq!(parameters[0]["requirement"], json!("required"));
        assert_eq!(parameters[1]["input_type"], json!("select"));
        assert_eq!(parameters[1]["options"][2], json!("line-by-line"));
    }

    /// The whole reason `schedule` is stateful: the app sets a cron and then
    /// re-lists, and it has to see what it just set.
    #[test]
    fn a_scheduled_recipe_stays_scheduled() {
        let mut store = store();
        assert_eq!(cron_of(&mut store, REVIEW), Value::Null);

        let params = json!({"id": REVIEW, "cron_schedule": "0 7 * * 1-5"});
        assert_eq!(call(&mut store, SCHEDULE, &params).unwrap(), json!({}));

        assert_eq!(cron_of(&mut store, REVIEW), json!("0 7 * * 1-5"));
        // and the one that arrived scheduled is untouched
        assert_eq!(cron_of(&mut store, STANDUP), json!("30 8 * * 1-5"));
    }

    #[test]
    fn a_null_cron_clears_the_schedule() {
        let mut store = store();
        let params = json!({"id": STANDUP, "cron_schedule": null});
        assert_eq!(call(&mut store, SCHEDULE, &params).unwrap(), json!({}));
        assert_eq!(cron_of(&mut store, STANDUP), Value::Null);
    }

    #[test]
    fn delete_removes_it_from_the_list() {
        let mut store = store();
        assert_eq!(
            call(&mut store, DELETE, &json!({"id": REVIEW})).unwrap(),
            json!({})
        );
        assert_eq!(ids(&mut store), [RESEARCH, STANDUP]);
    }

    #[test]
    fn deleting_an_unknown_id_is_invalid_params_with_a_reason() {
        let mut store = store();
        let (code, reason) = call(&mut store, DELETE, &json!({"id": "nope"})).unwrap_err();
        assert_eq!(code, -32602);
        assert!(reason.contains("nope"), "unhelpful reason: {reason}");
        assert_eq!(
            ids(&mut store).len(),
            3,
            "a failed delete removed something"
        );
    }

    /// The rule this file exists to enforce. `recipeId` is not `id` and
    /// `cronSchedule` is not `cron_schedule`, and being generous about either
    /// would mean a client could pass here and fail against goose.
    #[test]
    fn only_the_exact_wire_spellings_are_read() {
        let mut store = store();

        let (code, _) = call(&mut store, DELETE, &json!({"recipeId": REVIEW})).unwrap_err();
        assert_eq!(code, -32602);
        assert_eq!(ids(&mut store).len(), 3);

        // The camelCase spelling is ignored, not rejected — serde on the real
        // server drops an unknown key, so this schedules nothing and says it
        // succeeded, exactly as goose would.
        let params = json!({"id": STANDUP, "cronSchedule": "0 7 * * 1-5"});
        assert_eq!(call(&mut store, SCHEDULE, &params).unwrap(), json!({}));
        assert_eq!(cron_of(&mut store, STANDUP), Value::Null);
    }

    /// `MOCK_NO_SCHEDULER=1`: the one thing a goose without
    /// `--enable-scheduler` does differently, and the only way to reach the
    /// app's `scheduler_off` branch without one. `-32601` with goose's own
    /// sentence in the reason, because that pair is what the client turns into
    /// `Unsupported` and the detail screen turns into a fact row.
    #[test]
    fn a_scheduler_less_server_refuses_to_schedule() {
        let mut store = store();
        let params = json!({"id": REVIEW, "cron_schedule": "0 7 * * 1-5"});
        let (code, reason) = answer(SCHEDULE, &params, Fixtures::Full, true, &mut store)
            .unwrap()
            .unwrap_err();
        assert_eq!(code, -32601);
        assert_eq!(reason, "Scheduled recipe execution is not enabled");
        assert_eq!(
            cron_of(&mut store, REVIEW),
            Value::Null,
            "a refused schedule was written anyway"
        );

        // Unscheduling is refused too: it is the same method, and a server
        // that cannot run a timer cannot clear one either.
        let params = json!({"id": STANDUP, "cron_schedule": null});
        let (code, _) = answer(SCHEDULE, &params, Fixtures::Full, true, &mut store)
            .unwrap()
            .unwrap_err();
        assert_eq!(code, -32601);
        assert_eq!(cron_of(&mut store, STANDUP), json!("30 8 * * 1-5"));
    }

    /// The switch is the scheduler's alone. Everything else on the Recipes
    /// screen has to keep working, or the flag would be testing "no recipes"
    /// rather than "no scheduler".
    #[test]
    fn no_scheduler_leaves_the_rest_of_the_feature_alone() {
        let mut store = store();
        for (method, params) in [
            (LIST, json!({})),
            (SCAN, json!({"recipe": {"title": "ok"}})),
            (ENCODE, json!({"recipe": {"title": "ok"}})),
            (DELETE, json!({"id": REVIEW})),
        ] {
            assert!(
                answer(method, &params, Fixtures::Full, true, &mut store)
                    .unwrap()
                    .is_ok(),
                "{method} failed with the scheduler off"
            );
        }
    }

    #[test]
    fn a_cron_that_is_neither_a_string_nor_null_is_rejected() {
        let mut store = store();
        let params = json!({"id": STANDUP, "cron_schedule": 30});
        let (code, reason) = call(&mut store, SCHEDULE, &params).unwrap_err();
        assert_eq!(code, -32602);
        assert!(reason.contains("cron_schedule"), "{reason}");
    }

    #[test]
    fn scan_flags_the_recipe_that_shells_out_and_nothing_else() {
        let mut store = store();
        let entries = listed(&mut store);
        for entry in entries {
            let flagged = entry_id(&entry) == Some(RESEARCH);
            let params = json!({"recipe": entry["recipe"]});
            assert_eq!(
                call(&mut store, SCAN, &params).unwrap(),
                json!({"has_security_warnings": flagged}),
                "{:?}",
                entry_id(&entry)
            );
        }
    }

    #[test]
    fn scan_of_a_missing_recipe_body_is_invalid_params() {
        let mut store = store();
        let (code, _) = call(&mut store, SCAN, &json!({})).unwrap_err();
        assert_eq!(code, -32602);
    }

    /// The deeplink is bare URL-safe base64 of the recipe JSON, and this is
    /// the one assertion that pins the alphabet rather than trusting the
    /// encoder to agree with itself.
    #[test]
    fn encode_returns_url_safe_base64_of_the_recipe() {
        let mut store = store();
        let params = json!({"recipe": {"title": "ok"}});
        assert_eq!(
            call(&mut store, ENCODE, &params).unwrap(),
            json!({"deeplink": "eyJ0aXRsZSI6Im9rIn0"})
        );
    }

    /// Every tail length, and the two characters the URL-safe alphabet moves.
    #[test]
    fn base64_covers_both_partial_chunks_and_the_url_safe_characters() {
        assert_eq!(base64_url_no_pad(b""), "");
        assert_eq!(base64_url_no_pad(b"f"), "Zg");
        assert_eq!(base64_url_no_pad(b"fo"), "Zm8");
        assert_eq!(base64_url_no_pad(b"foo"), "Zm9v");
        assert_eq!(base64_url_no_pad(b"foob"), "Zm9vYg");
        assert_eq!(base64_url_no_pad(&[0xff, 0xef, 0xbf]), "_--_");
    }

    #[test]
    fn empty_fixtures_serve_no_recipes() {
        let mut store = store();
        let result = answer(LIST, &json!({}), Fixtures::Empty, false, &mut store).unwrap();
        assert_eq!(result.unwrap(), json!({"recipes": []}));
    }

    /// The error-then-retry path needs both halves reachable from one process,
    /// so the failure is spent on the first call and the second one works.
    #[test]
    fn broken_fixtures_fail_once_and_then_behave() {
        let mut store = store();

        let (code, reason) = answer(LIST, &json!({}), Fixtures::Broken, false, &mut store)
            .unwrap()
            .unwrap_err();
        assert_eq!(code, -32603);
        assert!(reason.contains("recipes"), "unhelpful reason: {reason}");

        let recovered = answer(LIST, &json!({}), Fixtures::Broken, false, &mut store)
            .unwrap()
            .unwrap();
        assert_eq!(recovered["recipes"].as_array().unwrap().len(), 3);
    }

    /// A `recipes/*` method this mock does not implement has to fall through
    /// to `dispatch`'s `-32601`, which is how the client learns to hide a
    /// control rather than show a failure.
    #[test]
    fn an_unimplemented_recipe_method_is_left_to_the_method_not_found() {
        let mut store = store();
        for method in [
            "_goose/unstable/recipes/save",
            "_goose/unstable/recipes/parse",
            "_goose/unstable/recipes/decode",
            "_goose/unstable/recipes/slash-command",
        ] {
            assert!(
                answer(method, &json!({}), Fixtures::Full, false, &mut store).is_none(),
                "{method} was answered"
            );
        }
    }
}
