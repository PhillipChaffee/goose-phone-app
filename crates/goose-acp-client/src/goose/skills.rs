//! Skills: `_goose/unstable/sources/list`, read-only.
//!
//! goose has no `skills/*` namespace. Skills are one kind of *source* —
//! `crates/goose/src/skills/mod.rs` says so in its module doc, and points at
//! `crate::sources` for the user-facing CRUD because that generalises across
//! recipes, subrecipes, agents and projects too. So the method this file
//! wraps is the generic one, narrowed by a `type` in the request.
//!
//! # Why only `list`
//!
//! `sources/create`, `/update`, `/delete`, `/export` and `/import` all exist
//! on the wire and none of them is wrapped here. That is settled by the
//! reference client, not by taste: goose Desktop's own "Add Skill" button
//! ships `hidden` with `title="Coming soon"`
//! (`ui/desktop/src/components/skills/SkillsView.tsx`), and a phone that
//! authored skills would put a secondary client ahead of the primary one.
//! Delete is cut for a second reason — [`SourceEntry::is_editable`] is false
//! for every built-in and for anything the server marked read-only, which is
//! a large share of a real list, so the control would be dead half the time
//! it was on screen.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::LIST_TIMEOUT;
use crate::client::AcpClient;
use crate::error::AcpError;

/// The one method this feature uses. Both skill kinds come through it.
const SOURCES_LIST: &str = "_goose/unstable/sources/list";

/// The kinds of source goose discovers.
///
/// The wire spellings are *not* uniform: `builtinSkill` is camelCase and the
/// other five are single lowercase words. goose gets that from a blanket
/// `rename_all = "camelCase"` on an enum whose other variants happen to be
/// one word each — which is exactly the kind of accident this module's
/// no-`rename_all` rule exists to make visible, so every variant names its
/// own wire string here.
///
/// Closed on purpose. goose's schema declares a closed enum, and this crate
/// only ever *asks* for `skill` and `builtinSkill`, so a variant a future
/// goose adds cannot appear in a response to a request made here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SourceType {
    #[serde(rename = "skill")]
    Skill,
    #[serde(rename = "builtinSkill")]
    BuiltinSkill,
    #[serde(rename = "recipe")]
    Recipe,
    #[serde(rename = "subrecipe")]
    Subrecipe,
    #[serde(rename = "agent")]
    Agent,
    #[serde(rename = "project")]
    Project,
}

impl SourceType {
    /// The wire string for this variant.
    ///
    /// Exists so a request can be built with `json!` instead of a fallible
    /// `serde_json::to_value`, and so the spellings sit in one place a
    /// reviewer can diff against `crates/goose/acp-schema.json`. A test holds
    /// it to what serde does.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::BuiltinSkill => "builtinSkill",
            Self::Recipe => "recipe",
            Self::Subrecipe => "subrecipe",
            Self::Agent => "agent",
            Self::Project => "project",
        }
    }
}

/// One source goose found on disk, or one it ships.
///
/// Mixed casing, so per the module rule: `supportingFiles` carries its own
/// rename and its neighbours — all single lowercase words — carry none.
/// `type` is renamed only because it is a Rust keyword.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    #[serde(rename = "type")]
    pub source_type: SourceType,
    pub name: String,
    pub description: String,
    /// The body of `SKILL.md`, frontmatter included.
    pub content: String,
    /// Stable identity: the directory holding `SKILL.md` for a filesystem
    /// skill, and a synthetic `builtin://skills/<name>` for a built-in.
    pub path: String,
    /// True in the user's global sources directory, false inside a project.
    pub global: bool,
    /// The server's own read-only marking. Do not read it alone — see
    /// [`SourceEntry::is_editable`], which built-ins lie about.
    pub writable: Option<bool>,
    /// Absolute paths of files living beside the skill. Only skills populate
    /// it, and it is the one camelCase field on this type.
    #[serde(rename = "supportingFiles")]
    pub supporting_files: Option<Vec<String>>,
    /// Type-specific frontmatter metadata. Free-form by declaration — the
    /// schema is an open object with no named members — so modelling it would
    /// be inventing a shape goose does not promise.
    pub properties: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SourceEntry {
    /// Where this skill comes from, in the words the screen uses.
    ///
    /// The backend spells this as an enum variant plus a bare bool, neither
    /// of which is English: `builtinSkill` is a wire token and `global: false`
    /// is a double negative about a project. Mapping happens here, in the
    /// crate that knows the protocol, so no screen ever has to.
    #[must_use]
    pub const fn scope_label(&self) -> &'static str {
        match self.source_type {
            SourceType::BuiltinSkill => "Built in",
            _ if self.global => "Global",
            _ => "This project",
        }
    }

    /// Whether goose would accept an edit to this source.
    ///
    /// Checks the type *before* the flag, because the flag is wrong for
    /// built-ins: `builtin_skill_entry` in `crates/goose/src/sources.rs`
    /// rewrites the type and the path but leaves `writable` at the `true` its
    /// parser set, so a built-in arrives claiming to be editable while every
    /// mutating method rejects it.
    #[must_use]
    pub const fn is_editable(&self) -> bool {
        match self.source_type {
            SourceType::BuiltinSkill => false,
            // `writable` is absent on a server old enough to predate it, and
            // the schema's default there is false — the cautious reading, and
            // the one that matches "only offer controls that do something".
            _ => matches!(self.writable, Some(true)),
        }
    }

    /// How many files ship alongside the skill.
    #[must_use]
    pub fn supporting_file_count(&self) -> usize {
        self.supporting_files.as_ref().map_or(0, Vec::len)
    }
}

/// The `sources/list` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSourcesResponse {
    pub sources: Vec<SourceEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Build the request body for one `sources/list` call.
///
/// Split out from the call so the casing — `projectDir`,
/// `includeProjectSources` — is checkable without a socket.
fn list_params(
    source_type: SourceType,
    project_dir: Option<&str>,
    include_project_sources: bool,
) -> Value {
    json!({
        "type": source_type.wire(),
        "projectDir": project_dir,
        "includeProjectSources": include_project_sources,
    })
}

/// Order a merged list the way goose Desktop orders it: by name, ignoring
/// case, and by path where two skills share a name — which they can, since a
/// project skill and a global one are different entries.
fn sort_entries(entries: &mut [SourceEntry]) {
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.path.clone()));
}

impl AcpClient {
    /// List sources of a single type.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if the server has no `sources/list`;
    /// otherwise as [`AcpClient::request_with_timeout`].
    pub async fn sources_list(
        &self,
        source_type: SourceType,
        project_dir: Option<&str>,
        include_project_sources: bool,
    ) -> Result<Vec<SourceEntry>, AcpError> {
        let params = list_params(source_type, project_dir, include_project_sources);
        let raw = self
            .goose_request(SOURCES_LIST, params, LIST_TIMEOUT)
            .await?;
        // A response this crate cannot parse is the server breaking the
        // contract, not a feature being off — `Transport` says that without
        // inventing an error variant for a case the round-trip tests exist to
        // keep from happening.
        let parsed: ListSourcesResponse = serde_json::from_value(raw)
            .map_err(|e| AcpError::Transport(format!("{SOURCES_LIST} returned {e}")))?;
        Ok(parsed.sources)
    }

    /// Every skill the server knows: filesystem skills and built-ins, merged
    /// and sorted.
    ///
    /// # Why the return type has two error channels
    ///
    /// This is two calls — `type: "skill"` and `type: "builtinSkill"` — and
    /// they are independent: different code paths on the server, different
    /// failure modes (a directory walk that fails to stat versus a decode of
    /// shipped content). goose Desktop `Promise.all`s them
    /// (`ui/desktop/src/acp/sources.ts`), so either one failing throws away
    /// the other's results and the user gets an empty screen when half the
    /// data was in hand. This does not.
    ///
    /// `Ok` therefore means *at least one half arrived*, and the
    /// `Option<AcpError>` beside the entries is the other half's failure —
    /// enough to show the list with a toast saying part of it is missing.
    /// `Err` is reserved for both halves failing, because the caller — the
    /// app's `load_remote`, where an error is what sets the "unsupported" or
    /// "retry" state — must be able to tell "nothing loaded" from "most of it
    /// did". A bare `(Vec, Option<AcpError>)` would have flattened those two
    /// into an empty list beside a warning, which reads as "you have no
    /// skills": a wrong statement rather than a missing one.
    ///
    /// The unsupported case stays clean despite the doubling: both calls use
    /// the same method, so [`AcpClient::goose_request`] caches the first
    /// `-32601` and the second returns [`AcpError::Unsupported`] without
    /// touching the socket. A server without the feature costs one round
    /// trip, not two, and both halves fail — so this returns `Err`, which is
    /// the state the screen wants.
    ///
    /// # Errors
    ///
    /// The `skill` call's error, if *both* calls failed. Typically
    /// [`AcpError::Unsupported`] on a server without the feature.
    pub async fn skills_list(
        &self,
        project_dir: Option<&str>,
        include_project_sources: bool,
    ) -> Result<(Vec<SourceEntry>, Option<AcpError>), AcpError> {
        // Sequential, not joined: the second call is what teaches
        // `goose_request` the method is missing, and a server that is going
        // to refuse both should be asked once.
        let filesystem = self
            .sources_list(SourceType::Skill, project_dir, include_project_sources)
            .await;
        let builtin = self
            .sources_list(
                SourceType::BuiltinSkill,
                project_dir,
                include_project_sources,
            )
            .await;
        merge_skills(filesystem, builtin)
    }
}

/// Fold the two list results into one, keeping whichever arrived.
///
/// A free function over plain data so the partial-failure rules are testable
/// without a server: both ok, either one failing, and both failing.
fn merge_skills(
    filesystem: Result<Vec<SourceEntry>, AcpError>,
    builtin: Result<Vec<SourceEntry>, AcpError>,
) -> Result<(Vec<SourceEntry>, Option<AcpError>), AcpError> {
    let (mut entries, partial) = match (filesystem, builtin) {
        (Ok(mut a), Ok(b)) => {
            a.extend(b);
            (a, None)
        }
        (Ok(a), Err(e)) => (a, Some(e)),
        (Err(e), Ok(b)) => (b, Some(e)),
        // Both gone. The filesystem call's error is the one to report: it is
        // the one the user's skills are actually in.
        (Err(e), Err(_)) => return Err(e),
    };
    sort_entries(&mut entries);
    Ok((entries, partial))
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the check"
)]
mod tests {
    use super::*;
    use crate::assert_round_trip;
    use crate::error::Feature;

    /// A complete `sources/list` response, in the shape goose sends one.
    const FIXTURE: &str = include_str!("../../tests/fixtures/skills.json");

    fn response() -> Value {
        serde_json::from_str(FIXTURE).unwrap()
    }

    fn entries() -> Vec<SourceEntry> {
        assert_round_trip::<ListSourcesResponse>(&response()).sources
    }

    fn named(name_fragment: &str) -> SourceEntry {
        entries()
            .into_iter()
            .find(|entry| entry.name.contains(name_fragment))
            .unwrap_or_else(|| panic!("fixture has no entry matching {name_fragment}"))
    }

    // ---- wire shape ----

    #[test]
    fn every_entry_round_trips_with_nothing_left_over() {
        let raw = response();
        for source in raw["sources"].as_array().unwrap() {
            let entry: SourceEntry = assert_round_trip(source);
            assert!(
                entry.extra.is_empty(),
                "unmodelled keys on {}: {:?}",
                entry.name,
                entry.extra.keys().collect::<Vec<_>>()
            );
        }
    }

    /// `extra` is the whole point of the flatten, and the fixture cannot prove
    /// it: every key in there is modelled, so an entry that round-trips proves
    /// only that nothing was *renamed*. A field goose grows tomorrow has to
    /// come back out the way it went in — otherwise this crate silently
    /// truncates a response it re-sends, and `extra.is_empty()` elsewhere
    /// stops meaning "we model everything" and starts meaning nothing at all.
    #[test]
    fn a_field_this_crate_does_not_model_survives_the_round_trip() {
        let mut raw = response();
        raw["sources"][0]["mtime"] = json!("2026-08-24T09:00:00Z");
        raw["sources"][0]["tags"] = json!(["review", "ci"]);
        // Top-level too: `ListSourcesResponse` carries its own flatten.
        raw["nextCursor"] = json!("page-2");

        let parsed: ListSourcesResponse = assert_round_trip(&raw);
        assert_eq!(parsed.extra["nextCursor"], json!("page-2"));
        let first = &parsed.sources[0];
        assert_eq!(first.extra["mtime"], json!("2026-08-24T09:00:00Z"));
        assert_eq!(first.extra["tags"], json!(["review", "ci"]));
        assert_eq!(
            first.extra.len(),
            2,
            "a modelled field leaked into extra: {:?}",
            first.extra.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn source_type_spellings_match_serde() {
        for kind in [
            SourceType::Skill,
            SourceType::BuiltinSkill,
            SourceType::Recipe,
            SourceType::Subrecipe,
            SourceType::Agent,
            SourceType::Project,
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.wire()));
        }
        assert_eq!(SourceType::BuiltinSkill.wire(), "builtinSkill");
    }

    /// The one camelCase field on `SourceEntry`, and therefore the one a
    /// blanket casing rule would have silently dropped to `None`.
    #[test]
    fn supporting_files_survives_the_wire() {
        let deploy = named("deploy");
        assert_eq!(
            deploy.supporting_files.as_deref(),
            Some(
                [
                    "/Users/me/work/pilot/.agents/skills/deploy/runbook.md".to_string(),
                    "/Users/me/work/pilot/.agents/skills/deploy/rollback.sh".to_string(),
                ]
                .as_slice()
            )
        );
        assert!(deploy.extra.is_empty(), "supportingFiles landed in extra");
    }

    /// The shape a real goose actually sends for a bare skill.
    ///
    /// `writable`, `supportingFiles` and `properties` all carry `default` and
    /// `skip_serializing_if` on the server (`goose-sdk-types`
    /// `custom_requests.rs`), so an entry with no files and no frontmatter
    /// metadata arrives with those three keys *absent*, not empty. The fixture
    /// spells them out because `assert_round_trip` compares both directions
    /// and this crate re-emits `None` as `null` — so absence is the one wire
    /// shape a round-trip test structurally cannot cover, and it is the common
    /// one.
    #[test]
    fn the_optional_keys_may_be_absent_entirely() {
        let bare = json!({
            "type": "skill",
            "name": "bare",
            "description": "No files, no properties, no writable flag.",
            "content": "Do the thing.",
            "path": "/Users/me/.agents/skills/bare",
            "global": true,
        });
        let entry: SourceEntry = serde_json::from_value(bare).unwrap();
        assert_eq!(entry.writable, None);
        assert_eq!(entry.supporting_files, None);
        assert_eq!(entry.properties, None);
        assert_eq!(entry.supporting_file_count(), 0);
        // Absent means read-only, which is the server's own default for the
        // flag — not "unknown, so offer the control anyway".
        assert!(!entry.is_editable());
        assert!(entry.extra.is_empty());
    }

    #[test]
    fn typed_fields_are_read_not_merely_accepted() {
        let builtin = named("goose-doc-guide");
        assert_eq!(builtin.source_type, SourceType::BuiltinSkill);
        assert_eq!(builtin.path, "builtin://skills/goose-doc-guide");
        assert_eq!(builtin.writable, Some(false));
        assert!(builtin.global);

        let review = named("code-review");
        assert_eq!(review.source_type, SourceType::Skill);
        assert_eq!(review.writable, Some(true));
        assert!(review.description.starts_with("Review a diff"));
        assert!(review.content.contains("---"), "frontmatter is kept intact");

        let deploy = named("deploy");
        assert!(!deploy.global);
        assert_eq!(
            deploy.properties.as_ref().unwrap()["workingDir"],
            json!("/Users/me/work/pilot")
        );
    }

    /// The audit stress-substitutes long text; the fixture has to give it
    /// something to bite on, so guard the lengths against a well-meaning tidy.
    #[test]
    fn the_fixture_carries_an_overlong_entry() {
        let long = entries()
            .into_iter()
            .max_by_key(|entry| entry.name.len())
            .unwrap();
        assert!(
            long.name.len() > 60,
            "longest fixture name is only {} chars",
            long.name.len()
        );
        assert!(
            long.description.len() > 240,
            "longest fixture description is only {} chars",
            long.description.len()
        );
    }

    // ---- domain ----

    #[test]
    fn scope_label_is_english() {
        assert_eq!(named("goose-doc-guide").scope_label(), "Built in");
        assert_eq!(named("code-review").scope_label(), "Global");
        assert_eq!(named("deploy").scope_label(), "This project");
    }

    /// A built-in that goose *claims* is writable is still not editable —
    /// which is the live case: `builtin_skill_entry` leaves the flag at true.
    #[test]
    fn builtins_are_never_editable_however_they_are_flagged() {
        let mut builtin = named("goose-doc-guide");
        assert!(!builtin.is_editable());
        builtin.writable = Some(true);
        assert!(!builtin.is_editable());
    }

    #[test]
    fn is_editable_follows_the_writable_flag_for_filesystem_skills() {
        let mut review = named("code-review");
        assert!(review.is_editable());
        review.writable = Some(false);
        assert!(!review.is_editable());
        // Absent on a server predating the field: the schema's default is
        // false, so treat it as read-only rather than offering a dead control.
        review.writable = None;
        assert!(!review.is_editable());
    }

    #[test]
    fn supporting_file_count_counts() {
        assert_eq!(named("deploy").supporting_file_count(), 2);
        assert_eq!(named("code-review").supporting_file_count(), 0);
        let mut absent = named("code-review");
        absent.supporting_files = None;
        assert_eq!(absent.supporting_file_count(), 0);
    }

    // ---- request ----

    #[test]
    fn request_uses_goose_casing() {
        assert_eq!(
            list_params(SourceType::BuiltinSkill, Some("/Users/me/work/pilot"), true),
            json!({
                "type": "builtinSkill",
                "projectDir": "/Users/me/work/pilot",
                "includeProjectSources": true,
            })
        );
        assert_eq!(
            list_params(SourceType::Skill, None, false),
            json!({
                "type": "skill",
                "projectDir": Value::Null,
                "includeProjectSources": false,
            })
        );
    }

    // ---- merge ----

    fn stub(name: &str, path: &str) -> SourceEntry {
        SourceEntry {
            source_type: SourceType::Skill,
            name: name.to_string(),
            description: String::new(),
            content: String::new(),
            path: path.to_string(),
            global: true,
            writable: Some(true),
            supporting_files: None,
            properties: None,
            extra: Map::new(),
        }
    }

    fn missing() -> AcpError {
        AcpError::Unsupported {
            feature: Feature::Skills,
            method: SOURCES_LIST.to_string(),
            reason: None,
        }
    }

    #[test]
    fn merge_sorts_case_insensitively_then_by_path() {
        let (entries, partial) = merge_skills(
            Ok(vec![
                stub("zebra", "/z"),
                stub("Apple", "/a2"),
                stub("Apple", "/a1"),
            ]),
            Ok(vec![stub("beta", "/b")]),
        )
        .unwrap();
        let order: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.path.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("Apple", "/a1"),
                ("Apple", "/a2"),
                ("beta", "/b"),
                ("zebra", "/z")
            ]
        );
        assert!(partial.is_none());
    }

    /// The whole point of not `Promise.all`-ing: one half failing still shows
    /// the other half.
    #[test]
    fn one_half_failing_keeps_the_other() {
        let (entries, partial) =
            merge_skills(Ok(vec![stub("mine", "/m")]), Err(missing())).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(partial.is_some());

        let (entries, partial) =
            merge_skills(Err(missing()), Ok(vec![stub("shipped", "/s")])).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(partial.is_some());
    }

    #[test]
    fn both_halves_failing_is_an_error_not_an_empty_list() {
        let err = merge_skills(Err(missing()), Err(missing())).unwrap_err();
        assert!(
            matches!(err, AcpError::Unsupported { .. }),
            "both-failed should surface the failure, got {err:?}"
        );
    }
}
