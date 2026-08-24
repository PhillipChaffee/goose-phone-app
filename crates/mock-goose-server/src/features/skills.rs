//! Skills, which goose serves as *sources*: one method, `sources/list`,
//! narrowed by the `type` in the request.
//!
//! The two types the app asks for are answered from separate arms on purpose.
//! The client makes two calls and merges them, and the whole reason it does
//! that instead of a `Promise.all` is that one of them can fail while the
//! other succeeds — so [`Fixtures::Broken`] here fails exactly one of them,
//! and the partial path is something a test can actually walk. It fails that
//! half *once*: a mock that stayed broken would leave the screen's recovery —
//! the pull gesture — with nothing to prove.
//!
//! # What the fixtures are for
//!
//! Each one is a state the app draws and nothing else here would reach:
//!
//!   - a filesystem skill marked read-only, which is the only shape that makes
//!     `SourceEntry::is_editable` false without the entry being built in, and
//!     therefore the only way to see the detail screen's "Read-only" fact;
//!   - one supporting file on that skill and two on `deploy`, because "1 file"
//!     and "2 files" are different sentences;
//!   - a name and a description at the length a real skill reaches, which is
//!     what `docs/audit.js` stress-substitutes into a row before it measures;
//!   - two built-ins, so the built-in half is a list rather than a row;
//!   - and all of them in *discovery* order rather than name order, so the
//!     ordering the app shows is the client's `sort_entries` doing its job and
//!     not the fixtures happening to be alphabetical.
//!
//! # Two things the schema decides for us
//!
//! `writable` carries `#[serde(default)]` and no `skip_serializing_if`
//! (`goose-sdk-types` `custom_requests.rs`), so goose sends it on every entry
//! including the false ones. `supportingFiles` and `properties` do carry
//! `skip_serializing_if`, so an entry with neither arrives with those keys
//! *absent* rather than as `[]` and `{}` — which is the common case, and one
//! the client only exercises if the mock leaves them out too.

use serde_json::{json, Value};

use crate::rpc::Out;
use crate::state::{Fixtures, Shared};

use super::Handled;

const LIST: &str = "_goose/unstable/sources/list";

/// The one other project in the mock's registry: somewhere `includeProjectSources`
/// can reach that the client's own `projectDir` never points at.
const OTHER_PROJECT: &str = "/Users/me/work/legacy";

pub(crate) fn handle(method: &str, params: &Value, state: &Shared, _out: &Out) -> Handled {
    if method != LIST {
        return None;
    }
    let (source_type, project_dir, include_project_sources) = read_request(params);
    let answer = answer_for(state, source_type);
    Some(list(
        source_type,
        project_dir,
        include_project_sources,
        answer,
    ))
}

/// Read the three fields of a `sources/list` request, in the only spellings
/// goose reads them in.
///
/// goose's request type is `rename_all = "camelCase"` and sets no
/// `deny_unknown_fields`, so `project_dir` is not a synonym for `projectDir` —
/// it is a key the real server reads straight past, leaving the field's
/// default behind. A mock generous enough to accept both would sign off a
/// client that then comes back empty against goose, which is the one failure
/// this crate exists to make loud.
fn read_request(params: &Value) -> (&str, Option<&str>, bool) {
    (
        params.get("type").and_then(Value::as_str).unwrap_or(""),
        params.get("projectDir").and_then(Value::as_str),
        params
            .get("includeProjectSources")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

/// What one call answers with, once the fixture switch has had its say.
#[derive(Clone, Copy, Debug)]
enum Answer {
    /// Everything the mock has of that type.
    Populated,
    /// `[]` — the zero state each screen draws.
    Empty,
    /// The filesystem half is down. Only ever handed to `skill`: the built-in
    /// half is served from memory on a real server, so a directory read is not
    /// a way for it to fail.
    Failed,
}

/// Decide what this call answers with, spending [`Fixtures::Broken`]'s one
/// failure if this is the call it belongs to.
///
/// The failure is a directory read, so it belongs to the half that reads
/// directories, and it is spent rather than permanent so the screen's
/// error-then-retry path is walkable: the first list is missing its filesystem
/// skills, the pull gesture brings them back.
fn answer_for(state: &Shared, source_type: &str) -> Answer {
    let mut state = state.lock().unwrap();
    match state.fixtures {
        Fixtures::Empty => Answer::Empty,
        Fixtures::Broken if source_type == "skill" => {
            if std::mem::replace(&mut state.skills_broken_spent, true) {
                Answer::Populated
            } else {
                Answer::Failed
            }
        }
        Fixtures::Broken | Fixtures::Full => Answer::Populated,
    }
}

fn list(
    source_type: &str,
    project_dir: Option<&str>,
    include_project_sources: bool,
    answer: Answer,
) -> Result<Value, (i64, String)> {
    // goose rejects a type it cannot list rather than answering an empty
    // array, and the client's `SourceType` is closed — so a request for
    // something else is a bug worth surfacing, not a quiet `[]`. The reason
    // rides in `data`, where goose puts its reasons and the client looks.
    if !matches!(
        source_type,
        "skill" | "builtinSkill" | "recipe" | "subrecipe" | "agent" | "project"
    ) {
        return Err((-32602, format!("unknown source type: {source_type}")));
    }
    let sources = match answer {
        Answer::Failed => {
            return Err((
                -32603,
                "failed to read /Users/me/.agents/skills: Permission denied (os error 13)"
                    .to_string(),
            ))
        }
        Answer::Empty => Vec::new(),
        Answer::Populated => populated(source_type, project_dir, include_project_sources),
    };
    Ok(json!({ "sources": sources }))
}

/// Everything the mock has of one type, in the order a walk would have found
/// it.
fn populated(
    source_type: &str,
    project_dir: Option<&str>,
    include_project_sources: bool,
) -> Vec<Value> {
    match source_type {
        // Unsorted, and the flag below is ignored here on purpose: goose only
        // honours `includeProjectSources` on the filesystem arm, because
        // built-ins do not live in a project.
        "builtinSkill" => vec![subagents(), doc_guide()],
        "skill" => {
            // Discovery order: a directory walk returns what it finds, not an
            // alphabetised list. Fixtures that happened to be sorted would let
            // the client's `sort_entries` be deleted with every test still
            // green.
            let mut sources = vec![incident_postmortem(), code_review(), pdf_form_filling()];
            // goose's `discover_skills` walks the global directories whatever
            // it is given and only adds the project's when it has a path —
            // which is what the app's degraded state depends on.
            let working_dir = project_dir.map(str::trim).filter(|dir| !dir.is_empty());
            if let Some(dir) = working_dir {
                sources.push(deploy(dir));
            }
            // `includeProjectSources` sends goose round the *other* projects in
            // its registry, skipping the one it has already walked and tagging
            // what it finds with where it came from
            // (`list_sources_with_roots`, `crates/goose/src/sources.rs`).
            if include_project_sources && working_dir != Some(OTHER_PROJECT) {
                sources.push(release_notes());
            }
            sources
        }
        // The other four types exist on the wire and this feature never asks
        // for them. Empty is honest: the mock has no recipes.
        _ => Vec::new(),
    }
}

/// A writable skill in the user's global directory.
fn code_review() -> Value {
    json!({
        "type": "skill",
        "name": "code-review",
        "description": "Review a diff the way this team reviews diffs.",
        "content": "---\nname: code-review\n\
                    description: Review a diff the way this team reviews diffs.\n---\n\n\
                    # Code review\n\nStart with the tests. If a change has no \
                    test, ask what would have caught it.\n",
        "path": "/Users/me/.agents/skills/code-review",
        "global": true,
        "writable": true
    })
}

/// A filesystem skill the server marked read-only.
///
/// Bundled with the client rather than written by the user, which is the case
/// the schema names in as many words: "Client-provided bundled sources are
/// returned as read-only". It is the only entry here that is neither editable
/// nor built in, so without it that branch of `SourceEntry::is_editable` — and
/// the "Read-only" fact the detail screen draws from it — cannot be seen in
/// the running app at all. Its single supporting file is the other reason it
/// exists: `deploy` only ever exercises the plural.
///
/// `global: true` because the flag's only other meaning is "inside a project",
/// and a skill shipped in the desktop bundle is not in one.
fn pdf_form_filling() -> Value {
    json!({
        "type": "skill",
        "name": "pdf-form-filling",
        "description": "Fill a PDF form from a table of answers, then flatten it so nothing can be typed over.",
        "content": "---\nname: pdf-form-filling\n\
                    description: Fill a PDF form from a table of answers, then flatten it so \
                    nothing can be typed over.\n---\n\n\
                    # Filling a PDF form\n\nMatch the columns to field names with `field-map.json` \
                    before writing anything; a form filled against the wrong map looks right and \
                    says something else.\n",
        "path": "/Applications/Goose.app/Contents/Resources/skills/pdf-form-filling",
        "global": true,
        "writable": false,
        "supportingFiles": [
            "/Applications/Goose.app/Contents/Resources/skills/pdf-form-filling/field-map.json"
        ]
    })
}

/// The stress entry: a name and a description at the length a real one
/// reaches.
///
/// `docs/audit.js` substitutes the longest text the server could plausibly
/// send into a list row and re-measures it. With only short fixtures it
/// reports clean over markup nothing has ever pushed on — so 68 characters of
/// name and ~300 of description are not a joke value but what a team that
/// names a skill after the document it produces actually writes. A test below
/// holds the lengths against a well-meaning tidy.
fn incident_postmortem() -> Value {
    json!({
        "type": "skill",
        "name": "incident-postmortem-timeline-reconstruction-and-follow-up-assignment",
        "description": "Reconstruct the timeline of an incident from the alert history, the \
                        deploy log and the chat transcript, name the moment each signal was \
                        first visible to a human, and draft the follow-up actions with an owner \
                        against every one of them. Ask before writing anything back to the \
                        incident record itself.",
        "content": "---\nname: incident-postmortem-timeline-reconstruction-and-follow-up-assignment\n\
                    description: Reconstruct the timeline of an incident from the alert history, \
                    the deploy log and the chat transcript, name the moment each signal was first \
                    visible to a human, and draft the follow-up actions with an owner against \
                    every one of them. Ask before writing anything back to the incident record \
                    itself.\n---\n\n\
                    # Postmortem\n\nBuild the timeline before you build the story. Every line \
                    gets a source and a timestamp, and anything you cannot source stays out of \
                    it.\n",
        "path": "/Users/me/.agents/skills/incident-postmortem-timeline-reconstruction-and-follow-up-assignment",
        "global": true,
        "writable": true
    })
}

/// A project skill with files beside it — the entry that exercises the row's
/// file count and the detail screen's second fact.
///
/// `workingDir` sits in the frontmatter as well as in `properties` because
/// that is where goose gets `properties` from: every frontmatter key beyond
/// `name` and `description` lands there verbatim.
fn deploy(dir: &str) -> Value {
    json!({
        "type": "skill",
        "name": "deploy",
        "description": "Ship the pilot service, including the rollback.",
        "content": format!(
            "---\nname: deploy\ndescription: Ship the pilot service, including the rollback.\n\
             workingDir: {dir}\n---\n\n\
             # Deploy\n\n1. Run the migration.\n2. Watch the error rate for ten minutes.\n"
        ),
        "path": format!("{dir}/.agents/skills/deploy"),
        "global": false,
        "writable": true,
        "supportingFiles": [
            format!("{dir}/.agents/skills/deploy/runbook.md"),
            format!("{dir}/.agents/skills/deploy/rollback.sh"),
        ],
        "properties": {"workingDir": dir}
    })
}

/// A skill from *another* project in goose's registry, tagged with where it
/// came from — what `includeProjectSources` is asking for.
///
/// `projectName` and `projectDir` are the one pair in `properties` that is not
/// frontmatter: goose inserts them as it walks each registered project, and
/// they are how a client tells a skill belonging to the project it is pointed
/// at from one it merely knows about.
fn release_notes() -> Value {
    json!({
        "type": "skill",
        "name": "release-notes",
        "description": "Turn a range of merged pull requests into notes a customer would read.",
        "content": "---\nname: release-notes\n\
                    description: Turn a range of merged pull requests into notes a customer \
                    would read.\n---\n\n\
                    # Release notes\n\nGroup by what changed for the user, not by which service \
                    changed. A line nobody outside the team can parse is a line to cut.\n",
        "path": format!("{OTHER_PROJECT}/.agents/skills/release-notes"),
        "global": false,
        "writable": true,
        "properties": {"projectName": "Legacy billing", "projectDir": OTHER_PROJECT}
    })
}

/// A built-in, `writable: true` and all — which is what goose really sends
/// (`builtin_skill_entry` rewrites the type and the path and leaves the flag
/// alone), and the reason `SourceEntry::is_editable` reads the type first.
///
/// No `supportingFiles`, and not because it happens to have none:
/// `builtin_skill_entry` clears them on its way out.
fn doc_guide() -> Value {
    json!({
        "type": "builtinSkill",
        "name": "goose-doc-guide",
        "description": "Write documentation the way the goose docs are written.",
        "content": "---\nname: goose-doc-guide\n\
                    description: Write documentation the way the goose docs are written.\n---\n\n\
                    # Writing docs\n\nSay what it does before you say how.\n",
        "path": "builtin://skills/goose-doc-guide",
        "global": true,
        "writable": true
    })
}

/// The second built-in, so "built-ins" is a list rather than a row. A screen
/// that has only ever rendered one of something hides whatever it does with
/// two, and the merge that interleaves them with filesystem skills has nothing
/// to interleave.
fn subagents() -> Value {
    json!({
        "type": "builtinSkill",
        "name": "subagents",
        "description": "Hand a piece of work to a subagent, and know when not to.",
        "content": "---\nname: subagents\n\
                    description: Hand a piece of work to a subagent, and know when not to.\n---\n\n\
                    # Subagents\n\nDelegate a search, not a decision. A subagent returns what it \
                    found; what to do about it is still yours.\n",
        "path": "builtin://skills/subagents",
        "global": true,
        "writable": true
    })
}

#[cfg(test)]
#[expect(
    clippy::panic,
    reason = "test assertions: a panic naming the offending fixture is the check"
)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::sync::mpsc;

    use super::*;
    use crate::rpc::response_frame;
    use crate::state::State;

    const PILOT: &str = "/Users/me/work/pilot";
    const LONG: &str = "incident-postmortem-timeline-reconstruction-and-follow-up-assignment";

    /// The over-the-wire tests only ever run the default fixtures, so every
    /// branch that answers something *other* than the happy list is reachable
    /// in the app and nowhere in the suite. `list` is a free function over
    /// plain data precisely so that gap can be closed without a socket.
    fn entries(source_type: &str, project_dir: Option<&str>, include: bool) -> Vec<Value> {
        list(source_type, project_dir, include, Answer::Populated).unwrap()["sources"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn names(source_type: &str, project_dir: Option<&str>, include: bool) -> Vec<String> {
        entries(source_type, project_dir, include)
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect()
    }

    fn lists(source_type: &str, project_dir: Option<&str>, include: bool, name: &str) -> bool {
        names(source_type, project_dir, include)
            .iter()
            .any(|n| n == name)
    }

    /// Every entry the mock can serve, both halves, everything switched on.
    fn all_entries() -> Vec<Value> {
        let mut all = entries("skill", Some(PILOT), true);
        all.extend(entries("builtinSkill", Some(PILOT), true));
        all
    }

    fn named(name: &str) -> Value {
        all_entries()
            .into_iter()
            .find(|entry| entry["name"] == json!(name))
            .unwrap_or_else(|| panic!("no fixture called {name}"))
    }

    fn shared(fixtures: Fixtures) -> Shared {
        Arc::new(Mutex::new(State {
            fixtures,
            ..State::default()
        }))
    }

    /// One request through the real entry point, fixture switch and all.
    fn call(state: &Shared, params: &Value) -> Result<Value, (i64, String)> {
        let (out, _rx) = mpsc::unbounded_channel();
        handle(LIST, params, state, &out).unwrap_or_else(|| panic!("{LIST} went unanswered"))
    }

    fn call_names(state: &Shared, params: &Value) -> Vec<String> {
        call(state, params).unwrap()["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect()
    }

    // ---- what a request selects ----

    #[test]
    fn a_project_dir_is_what_adds_the_project_skill() {
        assert!(!lists("skill", None, false, "deploy"));
        // Whitespace is not a path: goose trims and filters before it decides
        // whether to walk a project, so a mock that did not would answer a
        // question the real server refuses.
        assert!(!lists("skill", Some("   "), false, "deploy"));
        assert!(lists("skill", Some(PILOT), false, "deploy"));
        assert_eq!(
            named("deploy")["path"],
            json!("/Users/me/work/pilot/.agents/skills/deploy")
        );
    }

    /// The request field the client sends and the mock used to ignore.
    #[test]
    fn include_project_sources_is_what_adds_the_other_projects_skills() {
        assert!(!lists("skill", Some(PILOT), false, "release-notes"));
        assert!(lists("skill", Some(PILOT), true, "release-notes"));
        // A project already walked is not walked twice: goose skips a
        // registered project whose working directory is the one it was given,
        // and a mock that did not would send the same skill under two paths.
        assert!(!lists("skill", Some(OTHER_PROJECT), true, "release-notes"));
        // The built-in half ignores the flag, because goose's does.
        assert_eq!(
            names("builtinSkill", Some(PILOT), true),
            names("builtinSkill", Some(PILOT), false)
        );
    }

    /// The tag is the whole point of the flag: without it a client cannot tell
    /// a skill from the project it is pointed at from one it merely knows of.
    #[test]
    fn a_skill_from_another_project_says_which_project() {
        let props = &named("release-notes")["properties"];
        assert_eq!(props["projectDir"], json!(OTHER_PROJECT));
        assert_eq!(props["projectName"], json!("Legacy billing"));
        assert_eq!(named("release-notes")["global"], json!(false));
    }

    /// Discovery order, not name order. If this ever passes with the names
    /// sorted, the client's `sort_entries` could be deleted and every test in
    /// the suite would still be green — which is how it got deleted.
    #[test]
    fn the_fixtures_arrive_unsorted_so_the_client_has_to_sort_them() {
        for (kind, dir) in [("skill", Some(PILOT)), ("builtinSkill", None)] {
            let served = names(kind, dir, true);
            let mut sorted = served.clone();
            sorted.sort();
            assert_ne!(served, sorted, "{kind} fixtures are in name order");
        }
        // And across the merge: the built-ins have to land *between*
        // filesystem skills, not after them, or concatenation alone would look
        // sorted.
        let mut merged = names("skill", Some(PILOT), true);
        merged.extend(names("builtinSkill", Some(PILOT), true));
        let mut sorted = merged.clone();
        sorted.sort();
        assert_ne!(
            merged, sorted,
            "the two halves concatenate into sorted order"
        );
    }

    // ---- the fixture switches ----

    /// The partial-failure path, which is the whole reason the client makes
    /// two calls instead of one: Broken must take down exactly one half.
    #[test]
    fn broken_fails_the_filesystem_half_and_leaves_the_builtins() {
        let state = shared(Fixtures::Broken);
        assert_eq!(
            call_names(&state, &json!({"type": "builtinSkill"})),
            ["subagents", "goose-doc-guide"]
        );
        // The built-in call did not spend the failure: whichever order the
        // client makes its two calls in, the filesystem half is the one that
        // breaks.
        let err = call(&state, &json!({"type": "skill", "projectDir": PILOT})).unwrap_err();
        assert_eq!(err.0, -32603);
    }

    /// Once, not forever. The screen's recovery is the pull gesture, and a
    /// mock that stayed broken would leave that path with nothing to prove.
    #[test]
    fn broken_fails_the_first_listing_and_then_answers_normally() {
        let state = shared(Fixtures::Broken);
        let params = json!({"type": "skill", "projectDir": PILOT});
        assert!(call(&state, &params).is_err());
        assert!(call_names(&state, &params).contains(&"deploy".to_string()));
        assert!(call_names(&state, &params).contains(&"code-review".to_string()));
    }

    #[test]
    fn empty_empties_both_halves_rather_than_failing_them() {
        let state = shared(Fixtures::Empty);
        for kind in ["skill", "builtinSkill"] {
            let params = json!({"type": kind, "projectDir": PILOT, "includeProjectSources": true});
            assert!(call_names(&state, &params).is_empty(), "{kind} answered");
        }
    }

    // ---- spelling ----

    /// The mis-spelling this mock exists to catch, in both directions.
    ///
    /// goose declares its request and its entries `rename_all = "camelCase"`
    /// and sets no `deny_unknown_fields`, so `project_dir` is not a synonym
    /// for `projectDir`: it is a key the real server reads straight past,
    /// leaving the default in its place. A mock that generously accepted both
    /// spellings would sign off a client that then comes back empty against
    /// goose. The same holds in reverse for `supportingFiles` — a mock that
    /// emitted `supporting_files` would teach a client to read a key goose
    /// never sends.
    #[test]
    fn only_the_camel_case_spellings_are_read() {
        let camel = json!({"type": "skill", "projectDir": PILOT, "includeProjectSources": true});
        assert_eq!(read_request(&camel), ("skill", Some(PILOT), true));

        // Every field spelled the way a Rust-shaped guess spells it, and not
        // one of them is read: the project directory is gone and the flag is
        // back to its default.
        let snake = json!({
            "type": "skill",
            "project_dir": PILOT,
            "include_project_sources": true,
        });
        assert_eq!(read_request(&snake), ("skill", None, false));

        // Which is what the user would see: a list that looks fine with the
        // project's own skills quietly missing from it.
        let state = shared(Fixtures::Full);
        let served = call_names(&state, &snake);
        assert!(!served.contains(&"deploy".to_string()));
        assert!(!served.contains(&"release-notes".to_string()));

        // The response side of the same rule. `supportingFiles` is the one
        // camelCase key on an entry, and `builtinSkill` the one camelCase
        // token in the type enum — both surrounded by single lowercase words,
        // which is what makes either easy to lose.
        assert_eq!(
            named("deploy")["supportingFiles"].as_array().unwrap().len(),
            2
        );
        for entry in all_entries() {
            assert!(
                entry.get("supporting_files").is_none(),
                "{} sends the snake_case spelling",
                entry["name"]
            );
            let kind = entry["type"].as_str().unwrap();
            assert!(
                matches!(kind, "skill" | "builtinSkill"),
                "{} claims type {kind}",
                entry["name"]
            );
        }
    }

    /// A misspelt *type* is the same bug from the client's side, so it has to
    /// be loud. `builtin_skill` — the `snake_case` guess — is the one that
    /// would otherwise look like "this server has no built-in skills".
    ///
    /// The empty string is left out on purpose: this handler reads a missing
    /// `type` as `""` and refuses it, where goose defaults a missing `type` to
    /// `skill`. Nothing here sends one, so the divergence is unreachable —
    /// pinning it with a test would make it look intended.
    #[test]
    fn a_type_the_schema_does_not_name_is_refused_not_emptied() {
        for wrong in ["builtin_skill", "skills", "Skill", "builtinskill"] {
            let err = list(wrong, None, false, Answer::Populated).unwrap_err();
            assert_eq!(err.0, -32602, "{wrong} should be invalid params");
            assert!(
                err.1.contains(wrong),
                "the reason should name it: {}",
                err.1
            );
        }
        // goose builds its failures as `Error::invalid_params().data(reason)`,
        // so `message` stays the canned string from the spec and the sentence
        // worth reading is in `data` — which is where the client looks first.
        let refused = list("skills", None, false, Answer::Populated);
        let frame = response_frame(&json!(4), refused);
        assert_eq!(frame["error"]["code"], json!(-32602));
        assert_eq!(frame["error"]["message"], json!("Invalid params"));
        assert_eq!(frame["error"]["data"], json!("unknown source type: skills"));
    }

    // ---- the shapes the app draws ----

    /// Read-only *and* not built in: the combination `SourceEntry::is_editable`
    /// has a branch for and the detail screen has a fact for, and the one the
    /// app could not reach until this fixture existed.
    #[test]
    fn one_filesystem_skill_is_read_only() {
        let bundled = named("pdf-form-filling");
        assert_eq!(bundled["type"], json!("skill"));
        assert_eq!(bundled["writable"], json!(false));
        // And exactly one file beside it, which is the singular the row's
        // "1 file" needs and `deploy` never gives it.
        assert_eq!(bundled["supportingFiles"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn there_is_more_than_one_builtin() {
        let builtins = names("builtinSkill", None, false);
        assert!(
            builtins.len() > 1,
            "built-ins should be plural: {builtins:?}"
        );
    }

    /// `docs/audit.js` stress-substitutes long text; the fixtures have to give
    /// it something to bite on, so guard the lengths against a tidy.
    #[test]
    fn the_fixtures_carry_an_overlong_entry() {
        let long = named(LONG);
        assert!(long["name"].as_str().unwrap().len() > 60);
        assert!(long["description"].as_str().unwrap().len() > 240);
    }

    /// goose omits `supportingFiles` and `properties` when they are empty
    /// (both carry `skip_serializing_if`) and always sends `writable` (it does
    /// not). An entry that spelled the empty cases out as `[]` and `{}` would
    /// be a shape the real server never sends, and the client's handling of
    /// the absent ones would go unexercised over the wire.
    #[test]
    fn the_empty_optional_keys_are_absent_rather_than_spelled_out() {
        for entry in all_entries() {
            let name = &entry["name"];
            assert!(entry.get("writable").is_some(), "{name} omits writable");
            if let Some(files) = entry.get("supportingFiles") {
                assert!(!files.as_array().unwrap().is_empty(), "{name}: empty list");
            }
            if let Some(props) = entry.get("properties") {
                assert!(!props.as_object().unwrap().is_empty(), "{name}: empty map");
            }
        }
    }

    /// Every fixture body is a `SKILL.md`, which means closed frontmatter —
    /// the app strips it before rendering, and a body without it would make
    /// `strip_frontmatter` look correct on input it never sees.
    #[test]
    fn every_body_opens_with_closed_frontmatter() {
        for entry in all_entries() {
            let content = entry["content"].as_str().unwrap();
            let rest = content
                .strip_prefix("---\n")
                .unwrap_or_else(|| panic!("{} does not open a frontmatter fence", entry["name"]));
            assert!(
                rest.contains("\n---\n"),
                "{} never closes its fence",
                entry["name"]
            );
        }
    }

    /// goose parses `name` and `description` *out of* the frontmatter, so an
    /// entry whose fields disagree with its own document is a shape the real
    /// server cannot produce — and the detail screen shows both at once.
    #[test]
    fn the_frontmatter_says_what_the_entry_says() {
        for entry in all_entries() {
            let content = entry["content"].as_str().unwrap();
            let front = content
                .strip_prefix("---\n")
                .and_then(|rest| rest.split_once("\n---\n"))
                .unwrap_or_else(|| panic!("{} has no frontmatter", entry["name"]))
                .0
                .replace('\n', " ");
            for key in ["name", "description"] {
                let value = entry[key].as_str().unwrap();
                assert!(
                    front.contains(&format!("{key}: {value}")),
                    "{}'s frontmatter disagrees about {key}",
                    entry["name"]
                );
            }
        }
    }
}
