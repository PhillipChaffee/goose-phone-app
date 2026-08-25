//! Recipes: the saved, parameterised prompts goose keeps as YAML files.
//!
//! The phone is a *consumer* of recipes, not an author of them. It lists them,
//! reads what a run would ask for, deletes one, puts one on a cron, and hands
//! one to another device as a deeplink. It does not wrap `recipes/save`,
//! `recipes/parse` or `recipes/to-yaml` — authoring a recipe is a text-editor
//! job and a phone is the wrong editor — nor `recipes/slash-command`, because
//! this app's composer has no `/` autocomplete, so setting one would configure
//! a feature that cannot be reached from the device that set it.
//!
//! [`AcpClient::recipes_schedule`] is the only way this program creates a
//! schedule at all. goose also offers `schedules/create`, but that takes a
//! whole [`Recipe`] body and re-writes it to disk; `recipes/schedule` takes an
//! id and a cron string, which is what a row in a list actually has. Both go
//! through `require_scheduler`, so both answer `-32601` — mapped here to
//! [`AcpError::Unsupported`] — on a server started without
//! `--enable-scheduler`.
//!
//! Everything in this file is `snake_case` on the wire, so per the module
//! rules in [`super`] not one field carries a rename.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{LIST_TIMEOUT, MUTATE_TIMEOUT};
use crate::client::AcpClient;
use crate::error::AcpError;

const LIST: &str = "_goose/unstable/recipes/list";
const DELETE: &str = "_goose/unstable/recipes/delete";
const SCAN: &str = "_goose/unstable/recipes/scan";
const SCHEDULE: &str = "_goose/unstable/recipes/schedule";
const ENCODE: &str = "_goose/unstable/recipes/encode";

// ---------------------------------------------------------------- wire types

/// The `recipes/list` reply.
///
/// Its own type, with its own `extra`, rather than a `result["recipes"]` dig:
/// the dig would turn a wrong key into an empty list and an empty screen,
/// while a missed key here lands in `extra` where the round-trip check names
/// it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeListResponse {
    #[serde(default)]
    pub recipes: Vec<RecipeListEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One row of `recipes/list`: a recipe plus the facts about the *file* it
/// lives in and the things goose has attached to that file.
///
/// Fully modelled — all six fields — because every one of them is something a
/// list row shows or acts on. `id` is a hash of the path, not a name, and it
/// is what `delete` and `schedule` take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeListEntry {
    pub id: String,
    pub recipe: Recipe,
    pub file_path: String,
    /// RFC 3339, from the file's mtime. goose returns the list already sorted
    /// by this, newest first.
    pub last_modified: String,
    #[serde(default)]
    pub schedule_cron: Option<String>,
    /// The `/name` this recipe answers to in goose's own CLI and desktop
    /// composer. Reported, never set from here: see the module docs.
    #[serde(default)]
    pub slash_command: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A recipe, half-modelled on purpose.
///
/// Modelled: what the phone renders or edits. Left to `extra`: `extensions`,
/// `sub_recipes`, `retry`, `response`, `activities`, `author`. That is not
/// laziness, it is the whole reason `extra` exists — a save that round-trips
/// through this struct must not drop the retry policy or the sub-recipe list
/// the user never saw on a 402px screen. `extensions` in particular is a
/// four-variant internally-tagged union whose stdio arm alone has ten fields;
/// modelling it would be ten more chances to mis-spell a key in service of
/// something no screen draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    /// goose defaults this to `"1.0.0"` when a file omits it, so in practice
    /// it is always on the wire — but it is optional here rather than
    /// defaulted, so a server that stops sending it does not have a version
    /// invented on its behalf.
    ///
    /// The one `skip_serializing_if` in this module, against the rule in
    /// [`super`], because this is the one field whose goose-side type is not an
    /// `Option`: `RecipeDto::version` is a `String` with `#[serde(default)]`,
    /// and a serde `default` fires on a *missing* key and never on an explicit
    /// `null`. `scan` and `encode` send this body back, so serializing `None`
    /// as `null` would hand goose a `-32602` on a recipe it had just sent. The
    /// evidence the rule exists for is not lost with it: the fixture carries a
    /// version and `models_every_field_of_a_list_entry` reads it, so a
    /// mis-spelling here still fails a test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Absent and empty are different states on the wire and stay different
    /// here: goose omits the key for a recipe that takes no parameters and
    /// writes `[]` for one whose list was emptied.
    #[serde(default)]
    pub parameters: Option<Vec<RecipeParameter>>,
    #[serde(default)]
    pub settings: Option<RecipeSettings>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One input a recipe asks for before it runs.
///
/// Fully modelled: this drives the "asks for 3 inputs" fact on a row, and the
/// form that collects them. `options` is only meaningful for
/// [`RecipeInputType::Select`], and `default` is a string whatever the input
/// type says — goose stores the literal from the YAML and coerces at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeParameter {
    pub key: String,
    pub input_type: RecipeInputType,
    pub requirement: RecipeRequirement,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The provider/model overrides a recipe pins for its own runs.
///
/// Fully modelled, and still carrying an `extra`: `scan` and `encode` send
/// this object straight back to the server, so a settings key goose adds after
/// this build would otherwise be dropped from the scan payload or the
/// deeplink — the exact loss `extra` exists to prevent, one level down from
/// [`Recipe`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeSettings {
    #[serde(default)]
    pub goose_provider: Option<String>,
    #[serde(default)]
    pub goose_model: Option<String>,
    /// `f64`, though goose types it `f32`. JSON has one number type and the
    /// narrowing is not free: `0.2` read as `f32` and written back as `f64`
    /// becomes `0.20000000298023224`, a value the server never sent and the
    /// round-trip check would rightly reject.
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What kind of value a parameter takes.
///
/// `Other` is the version skew hatch. goose's namespace is called `unstable`
/// for a reason, and a single input type this build has not heard of must not
/// blank the whole Recipes screen — so an unrecognised string is carried
/// verbatim and written back exactly as it arrived.
///
/// The wire strings live in [`RecipeInputType::as_wire`] rather than in a
/// `rename_all` (banned in this module) or in six `rename` attributes: one
/// `match` is the whole mapping, on the lines a reviewer diffs against
/// `RecipeParameterInputTypeDto` in `acp-schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum RecipeInputType {
    String,
    Number,
    Boolean,
    Date,
    File,
    Select,
    Other(String),
}

impl RecipeInputType {
    /// The string goose spells this with.
    #[must_use]
    pub fn as_wire(&self) -> &str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::File => "file",
            Self::Select => "select",
            Self::Other(raw) => raw,
        }
    }
}

impl From<String> for RecipeInputType {
    fn from(wire: String) -> Self {
        match wire.as_str() {
            "string" => Self::String,
            "number" => Self::Number,
            "boolean" => Self::Boolean,
            "date" => Self::Date,
            "file" => Self::File,
            "select" => Self::Select,
            _ => Self::Other(wire),
        }
    }
}

impl From<RecipeInputType> for String {
    fn from(value: RecipeInputType) -> Self {
        match value {
            RecipeInputType::Other(raw) => raw,
            other => other.as_wire().to_owned(),
        }
    }
}

/// Whether a run has to stop and ask for this parameter.
///
/// `UserPrompt` is goose's `user_prompt`: a value it will ask for
/// interactively even though the recipe could technically proceed without it.
/// It blocks a run the same way `Required` does, which is the distinction
/// [`RecipeListEntry::needs_input`] turns into a yes or no.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum RecipeRequirement {
    Required,
    Optional,
    UserPrompt,
    /// See [`RecipeInputType::Other`].
    Other(String),
}

impl RecipeRequirement {
    /// The string goose spells this with.
    #[must_use]
    pub fn as_wire(&self) -> &str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::UserPrompt => "user_prompt",
            Self::Other(raw) => raw,
        }
    }

    /// Whether a run stops on this parameter.
    ///
    /// `Other` counts as not blocking: an unrecognised requirement is a word
    /// this build cannot explain, and guessing "blocking" would put a warning
    /// on a row for a reason the screen could not name.
    #[must_use]
    pub const fn blocks_a_run(&self) -> bool {
        matches!(self, Self::Required | Self::UserPrompt)
    }
}

impl From<String> for RecipeRequirement {
    fn from(wire: String) -> Self {
        match wire.as_str() {
            "required" => Self::Required,
            "optional" => Self::Optional,
            "user_prompt" => Self::UserPrompt,
            _ => Self::Other(wire),
        }
    }
}

impl From<RecipeRequirement> for String {
    fn from(value: RecipeRequirement) -> Self {
        match value {
            RecipeRequirement::Other(raw) => raw,
            other => other.as_wire().to_owned(),
        }
    }
}

/// The `recipes/scan` reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ScanResponse {
    #[serde(default)]
    has_security_warnings: bool,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The `recipes/encode` reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EncodeResponse {
    #[serde(default)]
    deeplink: String,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

// ------------------------------------------------------------ domain methods

impl RecipeListEntry {
    /// How many values running this recipe involves, blocking or not.
    ///
    /// The count a row states as a fact ("3 inputs"). It is deliberately not
    /// the same question as [`RecipeListEntry::needs_input`]: a recipe whose
    /// every parameter has a default still *has* parameters, it just will not
    /// stop to ask for them.
    #[must_use]
    pub fn input_count(&self) -> usize {
        self.recipe.parameters.as_ref().map_or(0, Vec::len)
    }

    /// Whether launching this will stop and ask for something first.
    ///
    /// This is what decides between a one-tap run and a form, so it counts
    /// only the requirements goose itself blocks on — `required` and
    /// `user_prompt`.
    #[must_use]
    pub fn needs_input(&self) -> bool {
        self.recipe.parameters.as_ref().is_some_and(|parameters| {
            parameters
                .iter()
                .any(|parameter| parameter.requirement.blocks_a_run())
        })
    }

    /// Whether the scheduler is running this on a cron.
    ///
    /// Blank is not scheduled. goose builds `schedule_cron` by joining the
    /// scheduler's jobs to recipe paths, and a job carrying an empty cron
    /// would otherwise light a "scheduled" dot on a row with nothing behind
    /// it.
    #[must_use]
    pub fn is_scheduled(&self) -> bool {
        self.schedule_cron
            .as_ref()
            .is_some_and(|cron| !cron.trim().is_empty())
    }
}

// ------------------------------------------------------------ request frames
//
// One builder per method, free functions over plain data so the exact keys
// each call puts on the wire are pinned by a unit test instead of by a live
// server. goose sets `deny_unknown_fields` on nothing, so a request key
// spelled wrong is accepted and ignored — `cronSchedule` would return success
// having scheduled nothing.

fn list_params() -> Value {
    json!({})
}

fn delete_params(id: &str) -> Value {
    json!({ "id": id })
}

fn schedule_params(id: &str, cron: Option<&str>) -> Value {
    // `null` is sent rather than the key omitted: both mean "unschedule" to
    // goose, but only the explicit null says on the wire what the user asked
    // for, which is the difference between reading a capture and guessing at
    // one.
    json!({ "id": id, "cron_schedule": cron })
}

/// The body `scan` and `encode` share.
fn recipe_params(recipe: &Recipe) -> Result<Value, AcpError> {
    let recipe = serde_json::to_value(recipe).map_err(|e| AcpError::Transport(e.to_string()))?;
    Ok(json!({ "recipe": recipe }))
}

// ----------------------------------------------------------------- the calls

impl AcpClient {
    /// Every recipe goose can see on disk, newest file first.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if this server has no recipe support,
    /// [`AcpError::Timeout`] after 30 s — goose walks the recipe directories
    /// to build this — [`AcpError::Closed`] if the connection drops, or
    /// [`AcpError::Transport`] if the reply is not a [`RecipeListResponse`].
    pub async fn recipes_list(&self) -> Result<Vec<RecipeListEntry>, AcpError> {
        let result = self
            .goose_request(LIST, list_params(), LIST_TIMEOUT)
            .await?;
        let parsed: RecipeListResponse =
            serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))?;
        Ok(parsed.recipes)
    }

    /// Delete a recipe by id. This unlinks the YAML file; there is no trash.
    ///
    /// `id` is the one from [`RecipeListEntry`] — goose resolves it against
    /// the directory listing, so it stays valid without a `list` immediately
    /// beforehand.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if this server has no recipe support,
    /// [`AcpError::Rpc`] if `id` names no recipe or the file cannot be
    /// removed, [`AcpError::Timeout`] after 30 s, or [`AcpError::Closed`].
    pub async fn recipes_delete(&self, id: &str) -> Result<(), AcpError> {
        self.goose_request(DELETE, delete_params(id), MUTATE_TIMEOUT)
            .await?;
        Ok(())
    }

    /// Whether goose's own security scan objects to this recipe.
    ///
    /// The check to run before handing a recipe that arrived from elsewhere to
    /// an agent with tools. It is a single yes/no by design: goose returns the
    /// verdict and keeps the findings.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if this server has no recipe support,
    /// [`AcpError::Rpc`] if goose rejects the recipe body,
    /// [`AcpError::Timeout`] after 30 s, [`AcpError::Closed`], or
    /// [`AcpError::Transport`] if the recipe will not serialize or the reply
    /// has no verdict in it.
    pub async fn recipes_scan(&self, recipe: &Recipe) -> Result<bool, AcpError> {
        let result = self
            .goose_request(SCAN, recipe_params(recipe)?, MUTATE_TIMEOUT)
            .await?;
        let parsed: ScanResponse =
            serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))?;
        Ok(parsed.has_security_warnings)
    }

    /// Put a recipe on a cron, or take it off one.
    ///
    /// `cron` is a five-field expression (`30 8 * * 1-5`); goose prepends the
    /// seconds field itself. `None` removes the schedule.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] — carrying goose's own "Scheduled recipe
    /// execution is not enabled" — if the server was started without
    /// `--enable-scheduler`, [`AcpError::Rpc`] if `id` names no recipe or the
    /// cron expression will not parse, [`AcpError::Timeout`] after 30 s, or
    /// [`AcpError::Closed`].
    pub async fn recipes_schedule(&self, id: &str, cron: Option<&str>) -> Result<(), AcpError> {
        self.goose_request(SCHEDULE, schedule_params(id, cron), MUTATE_TIMEOUT)
            .await?;
        Ok(())
    }

    /// Pack a recipe into a `goose://` deeplink — the shareable form.
    ///
    /// The whole recipe travels inside the link, which is why this takes the
    /// body rather than an id: the receiving machine does not need the file.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if this server has no recipe support,
    /// [`AcpError::Rpc`] if goose rejects the recipe body,
    /// [`AcpError::Timeout`] after 30 s, [`AcpError::Closed`], or
    /// [`AcpError::Transport`] if the recipe will not serialize or the reply
    /// has no link in it.
    pub async fn recipes_encode(&self, recipe: &Recipe) -> Result<String, AcpError> {
        let result = self
            .goose_request(ENCODE, recipe_params(recipe)?, MUTATE_TIMEOUT)
            .await?;
        let parsed: EncodeResponse =
            serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))?;
        Ok(parsed.deeplink)
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;
    use crate::assert_round_trip;

    /// A `recipes/list` reply in the shape `list_recipe_file_manifests` builds
    /// it: newest file first, four recipes, one of them carrying the
    /// `sub_recipes`/`retry`/`extensions` this crate does not model.
    const FIXTURE: &str = include_str!("../../tests/fixtures/recipes.json");

    fn reply() -> Value {
        serde_json::from_str(FIXTURE).unwrap()
    }

    fn entries() -> Vec<RecipeListEntry> {
        assert_round_trip::<RecipeListResponse>(&reply()).recipes
    }

    fn entry(index: usize) -> RecipeListEntry {
        entries().swap_remove(index)
    }

    /// Every entry, individually, so a failure names the recipe it came from
    /// rather than the whole reply.
    #[test]
    fn every_entry_round_trips() {
        let raw = reply();
        let raw = raw["recipes"].as_array().unwrap();
        assert_eq!(raw.len(), 4);
        for one in raw {
            let _: RecipeListEntry = assert_round_trip(one);
        }
    }

    #[test]
    fn models_every_field_of_a_list_entry() {
        let entry = entry(0);
        assert_eq!(entry.id, "9f2c41ab6d3e0517");
        assert_eq!(
            entry.file_path,
            "/home/me/.config/goose/recipes/quarterly-dependency-and-security-audit.yaml"
        );
        assert_eq!(entry.last_modified, "2026-08-23T18:42:11.508233+00:00");
        assert_eq!(entry.schedule_cron, None);
        assert_eq!(entry.slash_command, None);
        assert!(entry.recipe.title.starts_with("Quarterly dependency"));
        assert!(entry.recipe.instructions.is_some());
        assert_eq!(entry.recipe.prompt, None);
        assert_eq!(entry.recipe.version.as_deref(), Some("1.0.0"));
        // The point of the whole exercise: nothing on this recipe fell into
        // the catch-all, so every key goose sent is one this crate spells the
        // way goose spells it.
        assert!(entry.extra.is_empty(), "entry extra: {:?}", entry.extra);
        assert!(
            entry.recipe.extra.is_empty(),
            "recipe extra: {:?}",
            entry.recipe.extra
        );
    }

    /// The UI audit stress-substitutes the longest string it can find, so the
    /// fixture has to hold one worth substituting.
    #[test]
    fn one_recipe_has_text_long_enough_to_stress_a_row() {
        let entry = entry(0);
        assert!(
            entry.recipe.title.chars().count() > 60,
            "title is too short to stress a 402px row: {}",
            entry.recipe.title
        );
        assert!(entry.recipe.description.chars().count() > 200);
    }

    #[test]
    fn reads_a_scheduled_recipe_with_a_slash_command() {
        let entry = entry(1);
        assert_eq!(entry.schedule_cron.as_deref(), Some("30 8 * * 1-5"));
        assert_eq!(entry.slash_command.as_deref(), Some("standup"));
        assert!(entry.is_scheduled());
        let settings = entry.recipe.settings.clone().unwrap();
        assert_eq!(settings.goose_provider.as_deref(), Some("anthropic"));
        assert_eq!(settings.goose_model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(settings.temperature, Some(0.2));
        assert_eq!(settings.max_turns, Some(12));
        assert!(entry.recipe.extra.is_empty());
        assert!(
            settings.extra.is_empty(),
            "settings extra: {:?}",
            settings.extra
        );
    }

    #[test]
    fn an_unscheduled_recipe_is_not_scheduled() {
        assert!(!entry(0).is_scheduled());
        assert!(!entry(2).is_scheduled());
    }

    /// An empty cron reaching the phone is a job with nothing behind it, and
    /// it must not light the scheduled dot.
    #[test]
    fn a_blank_cron_is_not_a_schedule() {
        let mut entry = entry(0);
        entry.schedule_cron = Some("   ".to_owned());
        assert!(!entry.is_scheduled());
    }

    #[test]
    fn models_every_field_of_a_parameter() {
        let entry = entry(2);
        let parameters = entry.recipe.parameters.unwrap();
        assert_eq!(parameters.len(), 3);

        assert_eq!(parameters[0].key, "version");
        assert_eq!(parameters[0].input_type, RecipeInputType::String);
        assert_eq!(parameters[0].requirement, RecipeRequirement::Required);
        assert!(parameters[0].description.starts_with("The version"));
        assert_eq!(parameters[0].default, None);
        assert_eq!(parameters[0].options, None);

        assert_eq!(parameters[1].input_type, RecipeInputType::Select);
        assert_eq!(parameters[1].requirement, RecipeRequirement::Optional);
        assert_eq!(parameters[1].default.as_deref(), Some("stable"));
        assert_eq!(
            parameters[1].options.as_deref(),
            Some(["stable".to_owned(), "beta".to_owned(), "nightly".to_owned()].as_slice())
        );

        assert_eq!(parameters[2].input_type, RecipeInputType::Boolean);
        assert_eq!(parameters[2].requirement, RecipeRequirement::UserPrompt);

        for parameter in &parameters {
            assert!(parameter.extra.is_empty(), "{:?}", parameter.extra);
        }
    }

    #[test]
    fn a_required_parameter_makes_a_recipe_ask() {
        let entry = entry(2);
        assert_eq!(entry.input_count(), 3);
        assert!(entry.needs_input());
    }

    /// The distinction the two methods exist to draw: this recipe takes an
    /// input and still runs on one tap, because the input has a default and is
    /// only `optional`.
    #[test]
    fn optional_parameters_are_inputs_but_do_not_block() {
        let entry = entry(3);
        assert_eq!(entry.input_count(), 1);
        assert!(!entry.needs_input());
    }

    #[test]
    fn a_recipe_with_no_parameters_asks_for_nothing() {
        let entry = entry(0);
        assert_eq!(entry.input_count(), 0);
        assert!(!entry.needs_input());
    }

    /// The round-trip guarantee, tested on the case it exists for: the phone
    /// renders none of this and a save that went through this struct must
    /// still hand every byte of it back.
    #[test]
    fn unmodelled_recipe_fields_survive_in_extra() {
        let raw = reply();
        let raw = &raw["recipes"][3]["recipe"];
        let entry = entry(3);
        let recipe = &entry.recipe;

        for key in [
            "extensions",
            "sub_recipes",
            "retry",
            "activities",
            "author",
            "response",
        ] {
            assert!(recipe.extra.contains_key(key), "`{key}` was dropped");
        }
        assert_eq!(
            recipe.extra["sub_recipes"][0]["path"],
            json!("/home/me/.config/goose/recipes/lint.yaml")
        );
        assert_eq!(recipe.extra["retry"]["max_retries"], json!(2));

        assert_eq!(
            &serde_json::to_value(recipe).unwrap(),
            raw,
            "the recipe body did not come back the way it went in"
        );
    }

    /// A recipe body as `scan` and `encode` send it back, with a settings key
    /// this build has never heard of in it.
    fn recipe_with_settings(settings: &Value) -> Value {
        json!({
            "version": "1.0.0",
            "title": "Morning standup",
            "description": "What happened yesterday.",
            "instructions": null,
            "prompt": null,
            "parameters": null,
            "settings": settings,
        })
    }

    /// The same guarantee [`Recipe`] gets, one level down. `recipes/scan` and
    /// `recipes/encode` put this object back on the wire, so a settings field
    /// goose adds tomorrow must not be dropped from the payload that goes to
    /// its own scanner or baked out of a deeplink.
    #[test]
    fn an_unknown_settings_key_survives_in_extra() {
        let raw = recipe_with_settings(&json!({
            "goose_provider": "anthropic",
            "goose_model": "claude-sonnet-4-5",
            "temperature": 0.2,
            "max_turns": 12,
            "context_limit": 200_000,
        }));
        let recipe: Recipe = assert_round_trip(&raw);

        let settings = recipe.settings.clone().unwrap();
        assert_eq!(settings.goose_model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(settings.extra["context_limit"], json!(200_000));

        // Through the builder, because the frame is where losing it would
        // cost something.
        assert_eq!(recipe_params(&recipe).unwrap()["recipe"], raw);
    }

    /// goose types `version` as a `String` with a serde `default`, and a
    /// `default` fires on a missing key and never on an explicit `null` — so a
    /// body this crate built for a recipe that arrived without one has to omit
    /// the key rather than send `"version": null`, which goose answers with
    /// `-32602`.
    #[test]
    fn an_absent_version_is_omitted_rather_than_nulled() {
        let raw = json!({
            "title": "Morning standup",
            "description": "What happened yesterday.",
            "instructions": null,
            "prompt": null,
            "parameters": null,
            "settings": null,
        });
        let recipe: Recipe = assert_round_trip(&raw);
        assert_eq!(recipe.version, None);

        let params = recipe_params(&recipe).unwrap();
        assert!(
            params["recipe"].get("version").is_none(),
            "a null version would be rejected: {params}"
        );
    }

    /// goose's namespace is `unstable`: a value added on the server must cost
    /// one unreadable word on one row, not the whole screen.
    #[test]
    fn an_unknown_enum_value_is_carried_verbatim() {
        let raw = json!({
            "key": "region",
            "input_type": "geo_point",
            "requirement": "on_tuesdays",
            "description": "Where to run it.",
            "default": null,
            "options": null
        });
        let parameter: RecipeParameter = assert_round_trip(&raw);
        assert_eq!(
            parameter.input_type,
            RecipeInputType::Other("geo_point".to_owned())
        );
        assert_eq!(
            parameter.requirement,
            RecipeRequirement::Other("on_tuesdays".to_owned())
        );
        assert!(!parameter.requirement.blocks_a_run());
        assert_eq!(parameter.input_type.as_wire(), "geo_point");
    }

    /// The wire strings, spelled out against `acp-schema.json`'s two enums.
    #[test]
    fn enums_use_the_wire_spellings() {
        assert_eq!(RecipeInputType::String.as_wire(), "string");
        assert_eq!(RecipeInputType::Number.as_wire(), "number");
        assert_eq!(RecipeInputType::Boolean.as_wire(), "boolean");
        assert_eq!(RecipeInputType::Date.as_wire(), "date");
        assert_eq!(RecipeInputType::File.as_wire(), "file");
        assert_eq!(RecipeInputType::Select.as_wire(), "select");
        assert_eq!(RecipeRequirement::Required.as_wire(), "required");
        assert_eq!(RecipeRequirement::Optional.as_wire(), "optional");
        assert_eq!(RecipeRequirement::UserPrompt.as_wire(), "user_prompt");
    }

    #[test]
    fn the_scan_and_encode_replies_are_fully_modelled() {
        let scan: ScanResponse = assert_round_trip(&json!({"has_security_warnings": true}));
        assert!(scan.has_security_warnings);
        assert!(scan.extra.is_empty());

        let encode: EncodeResponse =
            assert_round_trip(&json!({"deeplink": "goose://recipe?config=eyJ0aXRsZSI6Im9rIn0"}));
        assert_eq!(encode.deeplink, "goose://recipe?config=eyJ0aXRsZSI6Im9rIn0");
        assert!(encode.extra.is_empty());
    }

    /// The exact frames these wrappers put on the wire. goose ignores a key it
    /// does not recognise, so `cronSchedule` here would succeed and schedule
    /// nothing; pinning the params is the only place that can be caught
    /// without a server.
    #[test]
    fn request_frames_use_the_wire_spellings() {
        assert_eq!(list_params(), json!({}));
        assert_eq!(
            delete_params("9f2c41ab6d3e0517"),
            json!({"id": "9f2c41ab6d3e0517"})
        );
        assert_eq!(
            schedule_params("9f2c41ab6d3e0517", Some("30 8 * * 1-5")),
            json!({"id": "9f2c41ab6d3e0517", "cron_schedule": "30 8 * * 1-5"})
        );
        assert_eq!(
            schedule_params("9f2c41ab6d3e0517", None),
            json!({"id": "9f2c41ab6d3e0517", "cron_schedule": null}),
            "unscheduling sends the null, it does not drop the key"
        );

        let recipe = entry(1).recipe;
        let params = recipe_params(&recipe).unwrap();
        assert_eq!(params["recipe"]["title"], json!("Morning standup"));
        assert_eq!(
            params.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["recipe"]
        );
    }

    /// The methods these wrap, spelled out once. `Feature::of_method` reads
    /// these strings to decide which screen a `-32601` darkens, and
    /// `recipes/schedule` is deliberately the scheduler's rather than the
    /// recipe list's.
    #[test]
    fn method_names_match_the_meta_file() {
        assert_eq!(LIST, "_goose/unstable/recipes/list");
        assert_eq!(DELETE, "_goose/unstable/recipes/delete");
        assert_eq!(SCAN, "_goose/unstable/recipes/scan");
        assert_eq!(SCHEDULE, "_goose/unstable/recipes/schedule");
        assert_eq!(ENCODE, "_goose/unstable/recipes/encode");
        assert_eq!(crate::Feature::of_method(LIST), crate::Feature::Recipes);
        assert_eq!(
            crate::Feature::of_method(SCHEDULE),
            crate::Feature::Scheduler
        );
    }
}
