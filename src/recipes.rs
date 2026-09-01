//! Recipes: the saved prompts on the goose server, and what a phone does with
//! one.
//!
//! State and the calls that change it. The rendering is
//! `src/views/recipes.rs`, the split `code.rs`/`views/code.rs` established:
//! everything here is testable without a Dioxus runtime, and nothing here
//! writes markup.
//!
//! The screen this feature exists for is two taps — open a recipe, press Run —
//! and the two rules that shape it are both about not firing an agent by
//! accident:
//!
//!   - **A tap on a row opens the recipe, it does not run it.** Running one
//!     starts an agent on the user's own machine with the user's own tools;
//!     a thumb brushing a list while scrolling must not be able to do that.
//!   - **A recipe that would stop and ask for values offers no Run at all**
//!     (design rule 11). The parameter callback is an agent→client *request*
//!     that blocks `session/new` until it is answered, and this client answers
//!     `-32601` to every agent request except permissions — so opting in
//!     without implementing the form would hang every parameterised launch
//!     instead of failing it. The recipe still opens, and its detail says why
//!     there is no button.

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::RecipeListEntry;
use serde_json::{json, Value};

use crate::cron::{self, Repeat, Schedule};
use crate::state::{load_remote, new_session_with, show_toast, AppCtx, Remote, Tab};
use crate::views::session_settings::{SettingChoice, SettingRow};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    List,
    Detail,
}

/// What goose's own security scan made of the open recipe.
///
/// [`ScanState::Unknown`] is a verdict this build could not get — an older
/// server, a scheduler-less one, a call that failed — and it deliberately
/// reads the same as clean. The scan is goose warning you about a recipe that
/// arrived from elsewhere, not a gate goose itself enforces: it runs a flagged
/// recipe from any client. A phone that refused to run anything it could not
/// scan would simply be broken against every server that lacks the method.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScanState {
    Pending,
    Clean,
    Warned,
    Unknown,
}

/// Everything the two recipe screens read.
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub screen: Signal<Screen>,
    pub list: Signal<Remote<RecipeListEntry>>,
    /// The recipe the detail screen is showing. A clone of the row rather
    /// than an id into the list: the list is refetched on every return to it,
    /// and an id would leave the open screen holding a row that moved.
    pub open: Signal<Option<RecipeListEntry>>,
    pub scan: Signal<ScanState>,
    /// This server answered a schedule call with "not enabled".
    ///
    /// Remembered so the Schedule row can stop being a control the second
    /// time: rule 11 does not let the app keep offering something it has been
    /// told does nothing. It cannot be known before the first call — goose
    /// only says so when asked — so the first refusal is what teaches it.
    pub scheduler_off: Signal<bool>,
}

/// Build the recipes state. A hook, so it can only be called where every
/// other `use_signal` in `AppCtx` is.
pub(crate) fn use_recipes() -> Ctx {
    Ctx {
        screen: use_signal(|| Screen::List),
        list: use_signal(Remote::new),
        open: use_signal(|| None),
        scan: use_signal(|| ScanState::Unknown),
        scheduler_off: use_signal(|| false),
    }
}

// ------------------------------------------------------------------ actions

/// Fetch the list, unless there is no connection to fetch it over.
///
/// The early return leaves whatever was last loaded on screen: a list you can
/// still read beats an empty one, and the bar's dot already says the phone is
/// offline.
pub(crate) async fn load(ctx: &AppCtx) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    load_remote(
        ctx,
        ctx.recipes.list,
        async move { client.recipes_list().await },
    )
    .await;
}

/// Load the list from a place that cannot await — a mount, a pull, a tap.
pub(crate) fn refresh(ctx: &AppCtx) {
    let ctx = *ctx;
    spawn_forever(async move { load(&ctx).await });
}

/// Open a recipe's detail screen and ask goose what it makes of it.
///
/// One RPC per open. The verdict is per *recipe body*, so it cannot be
/// cached against anything the list knows — a recipe edited on the desktop
/// keeps its id.
pub(crate) fn open(ctx: &AppCtx, entry: RecipeListEntry) {
    let (mut screen, mut open, mut scan) = (ctx.recipes.screen, ctx.recipes.open, ctx.recipes.scan);
    let recipe = entry.recipe.clone();
    let id = entry.id.clone();
    open.set(Some(entry));
    scan.set(ScanState::Pending);
    screen.set(Screen::Detail);

    let ctx = *ctx;
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            scan.set(ScanState::Unknown);
            return;
        };
        let verdict = client.recipes_scan(&recipe).await;
        // The reader may have gone back and opened another recipe while this
        // was in flight; a late verdict must not land on a different one.
        if ctx.recipes.open.peek().as_ref().map(|e| e.id.as_str()) != Some(id.as_str()) {
            return;
        }
        scan.set(match verdict {
            Ok(true) => ScanState::Warned,
            Ok(false) => ScanState::Clean,
            Err(_) => ScanState::Unknown,
        });
    });
}

/// Back to the list.
pub(crate) fn close(ctx: &AppCtx) {
    let (mut screen, mut open) = (ctx.recipes.screen, ctx.recipes.open);
    screen.set(Screen::List);
    open.set(None);
}

/// Delete a recipe, and drop its row on success.
///
/// The row goes locally rather than by refetching: goose has already unlinked
/// the file, and a list call to learn what the app just did is a second round
/// trip to confirm the first.
pub(crate) fn delete(ctx: &AppCtx, id: &str) {
    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.recipes_delete(&id).await {
            Ok(()) => {
                let mut list = ctx.recipes.list;
                list.write().items.retain(|entry| entry.id != id);
            }
            Err(e) => show_toast(&ctx, format!("Delete failed: {e}")),
        }
    });
}

/// Put the open recipe on a cron, or take it off one.
///
/// `cron` is already built by `crate::cron`, so there is nothing here to
/// validate — that is the point of the sheet producing choices instead of
/// text.
pub(crate) fn set_schedule(ctx: &AppCtx, id: &str, cron: Option<String>) {
    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.recipes_schedule(&id, cron.as_deref()).await {
            Ok(()) => {
                apply_schedule(&ctx, &id, cron.as_deref());
                show_toast(
                    &ctx,
                    cron.as_deref()
                        .map_or_else(|| "Schedule removed".to_owned(), cron::summary),
                );
            }
            Err(e) => {
                if e.is_unsupported() {
                    let mut off = ctx.recipes.scheduler_off;
                    off.set(true);
                }
                show_toast(&ctx, format!("Schedule not saved: {e}"));
            }
        }
    });
}

/// Write an accepted schedule into both copies of the row.
///
/// The open recipe and its row in the list are separate clones, and the
/// server pushes nothing when a schedule changes, so a screen that updated
/// only one of them would show the new schedule until the reader went back.
fn apply_schedule(ctx: &AppCtx, id: &str, cron: Option<&str>) {
    let (mut list, mut open) = (ctx.recipes.list, ctx.recipes.open);
    for entry in &mut list.write().items {
        if entry.id == id {
            entry.schedule_cron = cron.map(str::to_owned);
        }
    }
    let mut guard = open.write();
    if let Some(entry) = guard.as_mut() {
        if entry.id == id {
            entry.schedule_cron = cron.map(str::to_owned);
        }
    }
}

/// Start a chat that is this recipe's, and put its prompt in the composer.
///
/// `_meta.recipeId` is the whole mechanism: goose loads the recipe, extends
/// the session's system prompt with its instructions and applies its response
/// schema. What it does *not* do is send the first message — the recipe's
/// `prompt` is the client's to send, which is why it lands in the draft.
///
/// **Pre-filled, never auto-sent.** The prompt arrives exactly as the recipe
/// stores it, template markers and all, and the reader gets to read it before
/// an agent acts on it. Auto-sending would make the second tap of a two-tap
/// flow the point of no return.
/// Set unconditionally, not only when there is a prompt. The draft lives on
/// the context so this function can reach it, which also means it holds
/// whatever was last typed anywhere — and a recipe with no prompt of its own
/// would otherwise open its session with a stranger's half-written message
/// already in the composer, looking exactly like something the recipe put
/// there. A recipe with no prompt means an empty composer.
pub(crate) fn run(ctx: &AppCtx, entry: &RecipeListEntry) {
    let mut draft = ctx.chat_draft;
    draft.set(entry.recipe.prompt.clone().unwrap_or_default());
    new_session_with(ctx, run_meta(&entry.id));
    // The chat lives in the Home stack, so the drawer's destination has to
    // change with it. `new_session_with` navigates that stack on success and
    // toasts on failure, landing on the chat list — the same place the
    // failure would leave you if you had started the chat from there.
    let mut tab = ctx.tab;
    tab.set(Tab::Home);
}

/// The `_meta` that turns a plain `session/new` into this recipe's session.
///
/// A free function over plain data, like the client crate's `*_params`
/// builders and for the same reason: goose reads `recipeId` out of `_meta` in
/// `resolve_recipe_from_meta` and ignores every key it does not know, so
/// `recipe_id` here would open a perfectly ordinary chat with no recipe behind
/// it — no error, on either side. A unit test is the only place that spelling
/// can be caught without a server.
fn run_meta(id: &str) -> Value {
    json!({ "recipeId": id })
}

// -------------------------------------------------------------- the screens
//
// Everything below is a decision about what a screen shows, taken over plain
// data so it can be tested without mounting anything.

/// What the list has to say for itself.
///
/// One value rather than a pile of `if`s in the view, because the states are
/// exclusive and the interesting ones are the empty-looking ones: "no recipes
/// here", "this server has no recipes at all" and "this phone cannot see the
/// server" are three different sentences that all draw an empty screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListState {
    /// `-32601`: not a failure, so no Retry and no red.
    Unsupported,
    Offline,
    Failed(String),
    Loading,
    Empty,
    Rows,
}

pub(crate) fn list_state<T>(remote: &Remote<T>, connected: bool) -> ListState {
    if remote.unsupported {
        return ListState::Unsupported;
    }
    // Rows first: a list loaded before the connection dropped is still worth
    // reading, and the bar's dot already says the phone is offline.
    if !remote.items.is_empty() {
        return ListState::Rows;
    }
    if !connected {
        return ListState::Offline;
    }
    if let Some(error) = &remote.sticky {
        return ListState::Failed(error.clone());
    }
    if remote.loading {
        return ListState::Loading;
    }
    ListState::Empty
}

/// The second line of a list row: what is true about this recipe, in words.
///
/// Both halves are optional and neither is invented. An unscheduled recipe
/// has no state, so it gets no dot — design rule 8 puts a dot on a *state*,
/// and "not on a timer" is the absence of one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RowMeta {
    /// Present only when the recipe is on a cron. This is what the row's dot
    /// hangs off.
    pub schedule: Option<String>,
    pub inputs: Option<String>,
}

pub(crate) fn row_meta(entry: &RecipeListEntry) -> RowMeta {
    RowMeta {
        schedule: entry
            .is_scheduled()
            .then(|| crate::cron::summary(entry.schedule_cron.as_deref().unwrap_or_default())),
        inputs: (entry.input_count() > 0).then(|| counted(entry.input_count(), "input")),
    }
}

/// Whether the detail screen offers to run this, and how hard it makes you
/// press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunOffer {
    Run,
    /// Behind a confirm: goose's scan found something hidden in the recipe.
    Confirm,
    /// No button at all. The recipe would stop and ask for values, which this
    /// client cannot answer.
    Blocked,
}

pub(crate) const fn run_offer(needs_input: bool, scan: ScanState) -> RunOffer {
    if needs_input {
        return RunOffer::Blocked;
    }
    match scan {
        ScanState::Warned => RunOffer::Confirm,
        // Pending included: the verdict is advice, and a button that is not
        // there yet reads as a button that will not come.
        _ => RunOffer::Run,
    }
}

/// The Repeat row's value for a recipe that is not on a timer.
///
/// "Off" is a choice in the Repeat list rather than a switch of its own: a
/// separate toggle would be a second control for the same fact, and this way
/// the sheet has exactly one row that decides whether the others exist.
pub(crate) const SCHEDULE_OFF: &str = "off";

/// The schedule sheet, as rows.
///
/// Which rows exist follows from the repeat: an hourly schedule has no hour
/// to pick, a weekly one has a day and a monthly one has a date. A row that
/// could not affect the result is not rendered disabled, it is not rendered
/// (design rule 11).
pub(crate) fn schedule_rows(schedule: Schedule, on: bool) -> Vec<SettingRow> {
    let mut repeats = vec![SettingChoice::new(SCHEDULE_OFF, "Never")];
    repeats.extend(
        Repeat::ALL
            .into_iter()
            .map(|repeat| SettingChoice::new(repeat.id(), repeat.label())),
    );
    let current = if on {
        schedule.repeat.id()
    } else {
        SCHEDULE_OFF
    };
    let mut rows = vec![SettingRow::select(
        "repeat",
        "Repeat",
        Some(current),
        repeats,
        None,
    )];
    if !on {
        return rows;
    }

    match schedule.repeat {
        Repeat::Weekly => rows.push(SettingRow::select(
            "weekday",
            "Day",
            Some(&schedule.weekday.to_string()),
            (0..7)
                .map(|day| SettingChoice::new(day.to_string(), cron::weekday_name(day)))
                .collect(),
            None,
        )),
        Repeat::Monthly => rows.push(SettingRow::select(
            "day",
            "Day of month",
            Some(&schedule.day.to_string()),
            (1..=31)
                .map(|day| SettingChoice::new(day.to_string(), cron::ordinal(day)))
                .collect(),
            // Stated only where it is true. cron skips a month that has no
            // such day rather than clamping to its last one, and a schedule
            // that quietly misses February is worth one line.
            (schedule.day > 28)
                .then(|| "Months without this day are skipped, February most of all.".to_owned()),
        )),
        Repeat::Hourly | Repeat::Daily | Repeat::Weekdays => {}
    }
    if schedule.repeat != Repeat::Hourly {
        rows.push(SettingRow::select(
            "hour",
            "Hour",
            Some(&schedule.hour.to_string()),
            (0..24)
                .map(|hour| SettingChoice::new(hour.to_string(), cron::hour_label(hour)))
                .collect(),
            None,
        ));
    }
    rows.push(SettingRow::select(
        "minute",
        "Minute",
        Some(&schedule.minute.to_string()),
        cron::MINUTES
            .into_iter()
            .map(|minute| SettingChoice::new(minute.to_string(), cron::minute_label(minute)))
            .collect(),
        None,
    ));
    rows
}

/// Fold one tap in the sheet into the draft schedule.
///
/// Every value it can be handed came out of [`schedule_rows`], so an
/// unreadable one is not a user error to report — it is impossible, and
/// ignoring it is how this stays a function with no failure mode. Values the
/// current repeat does not use are still written: picking Monday, switching
/// to Daily and switching back must land on Monday again.
pub(crate) fn choose(schedule: &mut Schedule, on: &mut bool, row: &str, value: &str) {
    match row {
        "repeat" => {
            *on = value != SCHEDULE_OFF;
            if let Some(repeat) = Repeat::from_id(value) {
                schedule.repeat = repeat;
            }
        }
        "weekday" => set(&mut schedule.weekday, value),
        "day" => set(&mut schedule.day, value),
        "hour" => set(&mut schedule.hour, value),
        "minute" => set(&mut schedule.minute, value),
        _ => {}
    }
}

fn set(field: &mut u8, value: &str) {
    if let Ok(parsed) = value.parse() {
        *field = parsed;
    }
}

/// The facts card: what is true about this recipe that no control here can
/// change.
///
/// A row per thing worth stating and nothing per thing that is absent — a
/// recipe that pins no model has no Model row, rather than a Model row
/// reading "—". Every one of these is read-only on a phone, so every one of
/// them is a fact rather than a disabled control (rule 11).
pub(crate) fn facts(entry: &RecipeListEntry) -> Vec<SettingRow> {
    let mut rows = Vec::new();
    let settings = entry.recipe.settings.as_ref();

    if let Some(model) = settings.and_then(|s| s.goose_model.as_deref()) {
        rows.push(SettingRow::fact(
            "model",
            "Model",
            model,
            settings
                .and_then(|s| s.goose_provider.as_deref())
                .map_or_else(
                    || "The recipe pins this for its own runs.".to_owned(),
                    |provider| format!("{provider} · pinned by the recipe, not by the session."),
                ),
        ));
    }
    if let Some(turns) = settings.and_then(|s| s.max_turns) {
        rows.push(SettingRow::fact(
            "max_turns",
            "Max turns",
            turns.to_string(),
            "The run stops itself after this many agent turns.",
        ));
    }
    // `extensions` and `sub_recipes` are the fields the client crate leaves in
    // `extra` on purpose: nothing on a 402px screen draws a stdio command
    // line, but "this turns on two more tool servers" is exactly the kind of
    // thing you want to know before pressing Run.
    if let Some(count) = listed(entry, "extensions") {
        rows.push(SettingRow::fact(
            "extensions",
            "Extensions",
            counted(count, "extension"),
            "Tool servers the recipe switches on for its run.",
        ));
    }
    if let Some(count) = listed(entry, "sub_recipes") {
        rows.push(SettingRow::fact(
            "sub_recipes",
            "Sub-recipes",
            counted(count, "sub-recipe"),
            "Recipes this one runs as part of itself.",
        ));
    }
    if let Some((value, note)) = inputs_fact(entry.input_count(), entry.needs_input()) {
        rows.push(SettingRow::fact("inputs", "Inputs", value, note));
    }
    rows
}

/// The Inputs row: how many values the recipe takes, and what that means
/// here.
///
/// The two cases are the whole reason `input_count` and `needs_input` are
/// separate questions. Values with defaults are a fact about the recipe;
/// values it would stop and ask for are the reason the Run button is missing,
/// and a missing button with no explanation is design rule 11 failed rather
/// than followed.
pub(crate) fn inputs_fact(count: usize, needs_input: bool) -> Option<(String, String)> {
    if count == 0 {
        return None;
    }
    let value = counted(count, "input");
    Some(if needs_input {
        (
            format!("{value} · asked for at launch"),
            "Answering these is not supported on the phone yet, so this recipe \
             has to be started from goose on your desktop."
                .to_owned(),
        )
    } else {
        (
            value,
            "All optional — goose fills them from the recipe's own defaults.".to_owned(),
        )
    })
}

/// How long a list under one of `Recipe`'s unmodelled keys is, or `None` when
/// it is absent or empty.
fn listed(entry: &RecipeListEntry, key: &str) -> Option<usize> {
    let count = entry.recipe.extra.get(key).and_then(Value::as_array)?.len();
    (count > 0).then_some(count)
}

/// "1 input", "3 inputs". Every plural in this feature is a count of
/// something with a regular plural, so one helper covers all of them.
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;

    /// A list row as goose sends one. Written as JSON rather than as a struct
    /// literal so the test says what is on the wire, which is the thing the
    /// screens are reading.
    fn entry(recipe: &Value, schedule_cron: &Value) -> RecipeListEntry {
        entry_id("9f2c41ab6d3e0517", recipe, schedule_cron)
    }

    /// The same row with an id the caller chose, for the tests that need two
    /// rows and care which one a call reached.
    fn entry_id(id: &str, recipe: &Value, schedule_cron: &Value) -> RecipeListEntry {
        serde_json::from_value(json!({
            "id": id,
            "recipe": recipe,
            "file_path": "/home/me/.config/goose/recipes/x.yaml",
            "last_modified": "2026-08-23T18:42:11.508233+00:00",
            "schedule_cron": schedule_cron,
            "slash_command": null,
        }))
        .unwrap()
    }

    fn plain() -> Value {
        json!({"title": "Morning standup", "description": "What happened yesterday."})
    }

    fn with_parameters(requirements: &[&str]) -> Value {
        let parameters: Vec<Value> = requirements
            .iter()
            .enumerate()
            .map(|(i, requirement)| {
                json!({
                    "key": format!("p{i}"),
                    "input_type": "string",
                    "requirement": requirement,
                    "description": "",
                })
            })
            .collect();
        json!({
            "title": "Draft release notes",
            "description": "Notes since the last tag.",
            "parameters": parameters,
        })
    }

    /// The one wire key this file writes. `_meta.recipeId` is the whole
    /// mechanism — it is what makes the new session the recipe's — and goose
    /// drops an unknown `_meta` key silently, so nothing downstream of a
    /// mis-spelling would fail: the chat would simply open without the recipe.
    #[test]
    fn the_run_meta_carries_the_key_goose_reads() {
        assert_eq!(
            run_meta("9f2c41ab6d3e0517"),
            json!({"recipeId": "9f2c41ab6d3e0517"})
        );
    }

    /// The row says what is true and nothing else: no schedule line for a
    /// recipe that is not on a timer, no input line for one that takes none.
    #[test]
    fn a_row_with_nothing_to_say_says_nothing() {
        let meta = row_meta(&entry(&plain(), &Value::Null));
        assert_eq!(meta, RowMeta::default());
    }

    /// Rule 8: the cron string is mapped to copy before it reaches the row.
    #[test]
    fn a_scheduled_row_reads_as_a_sentence_not_as_a_cron() {
        let meta = row_meta(&entry(&plain(), &json!("30 8 * * 1-5")));
        assert_eq!(
            meta.schedule.as_deref(),
            Some("Runs every weekday at 8:30 AM")
        );
    }

    /// goose builds `schedule_cron` by joining scheduler jobs to recipe
    /// paths, so a blank one is reachable — and it would otherwise light a
    /// "scheduled" dot on a row with no schedule behind it.
    #[test]
    fn a_blank_cron_is_not_a_schedule() {
        assert_eq!(row_meta(&entry(&plain(), &json!("   "))).schedule, None);
    }

    #[test]
    fn the_input_count_is_every_parameter_not_just_the_blocking_ones() {
        let meta = row_meta(&entry(&with_parameters(&["optional"]), &Value::Null));
        assert_eq!(meta.inputs.as_deref(), Some("1 input"));
        let meta = row_meta(&entry(
            &with_parameters(&["required", "optional", "user_prompt"]),
            &Value::Null,
        ));
        assert_eq!(meta.inputs.as_deref(), Some("3 inputs"));
    }

    /// The rule the whole Run flow turns on: a recipe goose would stop and
    /// ask about gets no button, because this client cannot answer the ask.
    #[test]
    fn a_recipe_that_would_ask_for_values_offers_no_run() {
        for requirement in ["required", "user_prompt"] {
            let entry = entry(&with_parameters(&[requirement]), &Value::Null);
            assert!(entry.needs_input(), "{requirement} should block a run");
            assert_eq!(
                run_offer(entry.needs_input(), ScanState::Clean),
                RunOffer::Blocked
            );
        }
    }

    /// Optional parameters are not an ask: goose renders them from their
    /// defaults and runs.
    #[test]
    fn optional_values_still_leave_a_recipe_runnable() {
        let entry = entry(&with_parameters(&["optional"]), &Value::Null);
        assert!(!entry.needs_input());
        assert_eq!(
            run_offer(entry.needs_input(), ScanState::Clean),
            RunOffer::Run
        );
    }

    /// A scan that could not be got is not a scan that failed: it reads as
    /// clean, or the button disappears on every server without the method.
    #[test]
    fn only_a_positive_verdict_puts_run_behind_a_confirm() {
        assert_eq!(run_offer(false, ScanState::Warned), RunOffer::Confirm);
        assert_eq!(run_offer(false, ScanState::Pending), RunOffer::Run);
        assert_eq!(run_offer(false, ScanState::Unknown), RunOffer::Run);
        assert_eq!(run_offer(false, ScanState::Clean), RunOffer::Run);
        // Blocked wins over everything: there is no button to put behind a
        // confirm in the first place.
        assert_eq!(run_offer(true, ScanState::Warned), RunOffer::Blocked);
    }

    /// The missing button has to be explained where the reader is looking,
    /// and the explanation has to name the phone rather than the recipe.
    #[test]
    fn the_inputs_fact_says_why_there_is_no_button() {
        assert_eq!(inputs_fact(0, false), None);
        let (value, note) = inputs_fact(3, true).unwrap();
        assert_eq!(value, "3 inputs · asked for at launch");
        assert!(note.contains("desktop"), "{note}");
        let (value, note) = inputs_fact(1, false).unwrap();
        assert_eq!(value, "1 input");
        assert!(note.contains("defaults"), "{note}");
    }

    /// The unmodelled halves of a recipe still get stated, because "this
    /// turns on two more tool servers" is something you want before pressing
    /// Run — and they are exactly the fields the client crate keeps in
    /// `extra`.
    #[test]
    fn extensions_and_sub_recipes_are_read_out_of_extra() {
        let entry = entry(
            &json!({
                "title": "Nightly green build",
                "description": "Runs the gate.",
                "settings": {"goose_provider": "anthropic", "goose_model": "claude-sonnet-4-5",
                             "max_turns": 12},
                "extensions": [{"type": "stdio", "name": "github"}],
                "sub_recipes": [{"name": "lint", "path": "/x.yaml"}],
            }),
            &Value::Null,
        );
        let rows = facts(&entry);
        let named: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            named,
            [
                ("Model", "claude-sonnet-4-5"),
                ("Max turns", "12"),
                ("Extensions", "1 extension"),
                ("Sub-recipes", "1 sub-recipe"),
            ]
        );
    }

    /// A recipe with nothing pinned draws no card at all, rather than a card
    /// of em-dashes.
    #[test]
    fn a_recipe_that_pins_nothing_has_no_facts() {
        assert!(facts(&entry(&plain(), &Value::Null)).is_empty());
    }

    /// The Model row's note is the only place the provider is ever shown, so
    /// a recipe that names one has to say so and a recipe that names none
    /// must not invent one.
    ///
    /// Getting this backwards is silent: both branches produce a sentence,
    /// and the wrong one either drops the provider on a recipe that pinned
    /// `openai/gpt-5` — leaving a bare model name that could belong to any
    /// account — or prints `"None · pinned by the recipe"` where there is
    /// nothing to name.
    #[test]
    fn the_model_row_names_the_provider_only_when_the_recipe_did() {
        let with_provider = entry(
            &json!({
                "title": "T", "description": "D",
                "settings": {"goose_provider": "anthropic", "goose_model": "claude-sonnet-4-5"},
            }),
            &Value::Null,
        );
        let rows = facts(&with_provider);
        assert_eq!(
            rows[0].note.as_deref(),
            Some("anthropic · pinned by the recipe, not by the session.")
        );

        let bare_model = entry(
            &json!({
                "title": "T", "description": "D",
                "settings": {"goose_model": "claude-sonnet-4-5"},
            }),
            &Value::Null,
        );
        let rows = facts(&bare_model);
        assert_eq!(rows.len(), 1, "a model with no provider is still one row");
        assert_eq!(rows[0].value, "claude-sonnet-4-5");
        assert_eq!(
            rows[0].note.as_deref(),
            Some("The recipe pins this for its own runs."),
            "a recipe that named no provider had one printed for it"
        );
    }

    /// The facts card is where the missing Run button is explained, so the
    /// Inputs row has to be *in* it.
    ///
    /// `inputs_fact` composing the right sentence is worth nothing if `facts`
    /// never asks for it: the reader would open a parameterised recipe, find
    /// no Run button and no card, and have nothing on screen telling them to
    /// go to the desktop. That is design rule 11 failed rather than followed,
    /// and it fails with no error anywhere.
    #[test]
    fn the_facts_card_carries_the_reason_the_run_button_is_missing() {
        let blocked = entry(&with_parameters(&["required", "optional"]), &Value::Null);
        assert_eq!(
            run_offer(blocked.needs_input(), ScanState::Clean),
            RunOffer::Blocked,
            "the fixture has to be a recipe with no Run button, or this test \
             is about nothing"
        );
        let rows = facts(&blocked);
        let inputs = rows
            .iter()
            .find(|row| row.name == "Inputs")
            .expect("a recipe whose Run button is missing must say why on the card");
        assert_eq!(inputs.value, "2 inputs · asked for at launch");
        assert!(
            inputs
                .note
                .as_deref()
                .is_some_and(|n| n.contains("desktop")),
            "the note has to point somewhere the recipe can actually be run"
        );

        // A recipe whose inputs all have defaults still gets the row — it is
        // a fact about the recipe — but not the sentence about the desktop.
        let runnable = entry(&with_parameters(&["optional"]), &Value::Null);
        let rows = facts(&runnable);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "1 input");
    }

    /// The three empty-looking states are three different sentences, and
    /// telling them apart is the only thing this function does.
    #[test]
    fn an_empty_screen_still_says_which_kind_of_empty_it_is() {
        let mut remote = Remote::<u8>::new();
        assert_eq!(list_state(&remote, false), ListState::Offline);
        assert_eq!(list_state(&remote, true), ListState::Empty);
        remote.begin();
        assert_eq!(list_state(&remote, true), ListState::Loading);
        remote.settle(Vec::new());
        assert_eq!(list_state(&remote, true), ListState::Empty);
        remote.unsupported = true;
        assert_eq!(list_state(&remote, true), ListState::Unsupported);
        // Unsupported outranks offline: it is a fact about the server that
        // stays true whether or not the phone can reach it right now.
        assert_eq!(list_state(&remote, false), ListState::Unsupported);
    }

    /// A failure with nothing behind it is the one that has to stay on
    /// screen, and it has to stay *below* the two facts that outrank it.
    ///
    /// Without the `sticky` arm the reader who hit a broken server gets the
    /// same blank "No recipes yet" as the reader whose server has none — the
    /// toast that carried the reason has already faded, so there would be
    /// nothing left anywhere saying the list failed to load. And a phone that
    /// went offline after that failure must go back to saying so: the stale
    /// error names a request, and "this phone cannot reach the server" is the
    /// thing to fix first.
    #[test]
    fn a_failure_over_an_empty_list_is_kept_where_the_toast_would_have_faded() {
        let mut remote = Remote::<u8>::new();
        let toast = remote.fail(&goose_acp_client::AcpError::Transport(
            "no route".to_owned(),
        ));
        assert_eq!(
            toast, None,
            "a failure with an empty screen behind it must be kept, not \
             toasted away"
        );
        assert_eq!(
            list_state(&remote, true),
            ListState::Failed("transport error: no route".to_owned())
        );
        assert_eq!(
            list_state(&remote, false),
            ListState::Offline,
            "a disconnected phone reports the disconnection, not the request \
             that failed while it still had a connection"
        );
        remote.settle(vec![1_u8]);
        assert_eq!(
            list_state(&remote, true),
            ListState::Rows,
            "a successful refetch left the old failure on screen"
        );
    }

    /// A list you can still read is worth more than the reason the last
    /// refresh failed, which `Remote` has already toasted.
    #[test]
    fn rows_outrank_every_way_of_being_empty() {
        let mut remote = Remote::new();
        remote.settle(vec![1_u8, 2]);
        remote.loading = true;
        assert_eq!(list_state(&remote, false), ListState::Rows);
    }

    fn row_names(schedule: Schedule, on: bool) -> Vec<String> {
        schedule_rows(schedule, on)
            .into_iter()
            .map(|row| row.name)
            .collect()
    }

    /// The sheet holds exactly the questions its repeat has answers for. A
    /// weekday schedule has no day to pick and an hourly one has no hour, and
    /// neither gets a row that could not change the result.
    #[test]
    fn the_sheet_asks_only_what_the_repeat_leaves_open() {
        let base = Schedule::default();
        assert_eq!(row_names(base, false), ["Repeat"]);
        assert_eq!(
            row_names(
                Schedule {
                    repeat: Repeat::Hourly,
                    ..base
                },
                true
            ),
            ["Repeat", "Minute"]
        );
        assert_eq!(
            row_names(
                Schedule {
                    repeat: Repeat::Weekdays,
                    ..base
                },
                true
            ),
            ["Repeat", "Hour", "Minute"]
        );
        assert_eq!(
            row_names(
                Schedule {
                    repeat: Repeat::Weekly,
                    ..base
                },
                true
            ),
            ["Repeat", "Day", "Hour", "Minute"]
        );
        assert_eq!(
            row_names(
                Schedule {
                    repeat: Repeat::Monthly,
                    ..base
                },
                true
            ),
            ["Repeat", "Day of month", "Hour", "Minute"]
        );
    }

    /// Every row is a control with more than one value, so none of them
    /// degrades to a fact — and every value on screen is already UI copy, not
    /// the number that will go on the wire.
    #[test]
    fn every_row_is_pickable_and_reads_as_words() {
        let schedule = Schedule {
            repeat: Repeat::Weekly,
            weekday: 5,
            hour: 18,
            minute: 30,
            ..Schedule::default()
        };
        let values: Vec<(String, String)> = schedule_rows(schedule, true)
            .into_iter()
            .map(|row| {
                assert!(row.choices.len() > 1, "{} degraded to a fact", row.name);
                (row.name, row.value)
            })
            .collect();
        assert_eq!(
            values,
            [
                ("Repeat".to_owned(), "Every week".to_owned()),
                ("Day".to_owned(), "Friday".to_owned()),
                ("Hour".to_owned(), "6 PM".to_owned()),
                ("Minute".to_owned(), ":30".to_owned()),
            ]
        );
    }

    /// A schedule is edited, not re-entered: a value the current repeat does
    /// not use has to survive a trip through one that does, or picking
    /// "Monthly" by mistake costs you the day you already chose.
    #[test]
    fn switching_repeat_keeps_the_choices_the_other_repeats_made() {
        let mut schedule = Schedule::default();
        let mut on = true;
        choose(&mut schedule, &mut on, "repeat", "weekly");
        choose(&mut schedule, &mut on, "weekday", "4");
        choose(&mut schedule, &mut on, "hour", "17");
        choose(&mut schedule, &mut on, "minute", "45");
        assert_eq!(crate::cron::build(schedule), "0 45 17 * * 4");

        choose(&mut schedule, &mut on, "repeat", "daily");
        assert_eq!(crate::cron::build(schedule), "0 45 17 * * *");
        choose(&mut schedule, &mut on, "repeat", "weekly");
        assert_eq!(crate::cron::build(schedule), "0 45 17 * * 4");
    }

    /// Off is a value of the Repeat row rather than a control of its own, and
    /// it is the only thing in the sheet that produces no cron at all.
    #[test]
    fn off_is_a_repeat_choice_and_nothing_else_changes_it() {
        let mut schedule = Schedule::default();
        let mut on = true;
        choose(&mut schedule, &mut on, "repeat", SCHEDULE_OFF);
        assert!(!on);
        // The rest of the schedule is untouched, so turning it back on shows
        // what was there rather than a default.
        choose(&mut schedule, &mut on, "minute", "20");
        assert!(!on, "a value row must not switch the schedule back on");
        choose(&mut schedule, &mut on, "repeat", "daily");
        assert!(on);
        assert_eq!(crate::cron::build(schedule), "0 20 9 * * *");
    }

    /// The sheet's promise is that no tap in it can produce a cron the reader
    /// did not pick, and the two ways that could happen are a row name this
    /// file has no arm for and a value that is not a number.
    ///
    /// Both are the same class of bug and neither would report itself: a
    /// mistyped row id in `schedule_rows`, or an id whose values stop being
    /// integers, would leave the sheet visibly moving and the cron silently
    /// stuck on whatever it already said. The recipe would then run at a time
    /// nobody chose, which is a phone starting an agent unasked — the thing
    /// this whole file is careful about.
    #[test]
    fn a_tap_this_file_cannot_read_leaves_the_schedule_exactly_as_it_was() {
        let before = Schedule {
            repeat: Repeat::Weekly,
            weekday: 3,
            hour: 7,
            minute: 15,
            ..Schedule::default()
        };
        let mut schedule = before;
        let mut on = true;

        choose(&mut schedule, &mut on, "second", "30");
        choose(&mut schedule, &mut on, "", "");
        choose(&mut schedule, &mut on, "hour", "half past noon");
        choose(&mut schedule, &mut on, "weekday", "-1");
        choose(&mut schedule, &mut on, "minute", "999");

        assert!(on, "an unreadable tap turned the schedule off");
        assert_eq!(
            crate::cron::build(schedule),
            crate::cron::build(before),
            "an unreadable tap moved the schedule to a time nobody picked"
        );

        // Off is still reachable, and an unknown repeat id still means "not
        // off" — the row said something, it just was not a repeat this build
        // knows.
        choose(&mut schedule, &mut on, "repeat", SCHEDULE_OFF);
        assert!(!on);
        choose(&mut schedule, &mut on, "repeat", "fortnightly");
        assert!(
            on,
            "a repeat id this build does not know still means the schedule is on"
        );
        assert_eq!(
            crate::cron::build(schedule),
            crate::cron::build(before),
            "an unknown repeat id rewrote the schedule"
        );
    }

    // ------------------------------------------------------- the live actions
    //
    // Everything above this line is a decision taken over plain data. The
    // actions are not: they write signals, and four of them spawn. So this
    // half runs them against a real `AppCtx` inside a real `VirtualDom`, the
    // way `src/shell/desktop` drives its arrival effect.
    //
    // The context is built here rather than by
    // `crate::state::use_app_ctx_provider` deliberately. That hook opens the
    // permission journal's real storage backing, whose directory is a
    // process-wide `OnceLock` that `ask_journal`'s own test claims for the
    // whole test binary — `set_directory` unwraps, so a second claimant
    // panics whichever of the two loses the race. Nothing in this file reads
    // the journal, so the struct is assembled here out of plain signals and
    // touches no disk.
    //
    // The server's *answers* — a scan verdict, a delete that succeeded, a
    // schedule refused as unsupported — used to be out of reach here, because
    // `AcpClient` has no constructor but `connect` and connecting means
    // completing a WebSocket handshake against something. So there is
    // something: `crate::scheduler::tests::serve` puts a scripted JSON-RPC
    // server on a loopback port over plain `ws://`, and [`Harness::connected`]
    // points a real client at it. Everything below the "over the wire" rule
    // runs the code that only exists after an `await`.

    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::time::Duration;

    use goose_acp_client::{AcpClient, ConnectConfig};

    use crate::scheduler::tests::{ok, rpc_error, serve, Reply, Script, Server};
    use crate::state::AppCtx;

    thread_local! {
        /// The context the probe built, published for the test driving it.
        /// Per-thread, and the test harness gives every `#[test]` a thread of
        /// its own, so two harnesses never see each other's.
        static PROBE_CTX: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
    }

    /// The app's whole context and no app: no shell, no screen, no storage.
    #[component]
    fn CtxProbe() -> Element {
        let ctx = AppCtx {
            screen: use_signal(|| crate::state::Screen::Settings),
            settings: use_signal(crate::state::Settings::default),
            conn: use_signal(|| crate::state::ConnState::Disconnected),
            client: use_signal(|| None),
            want_connected: use_signal(|| false),
            sessions: use_signal(Vec::new),
            sessions_next: use_signal(|| None),
            sessions_loading: use_signal(|| false),
            sessions_query: use_signal(String::new),
            sessions_epoch: use_signal(|| 0),
            chat: use_signal(crate::state::ChatState::default),
            running_sessions: use_signal(HashSet::new),
            permission: use_signal(Vec::new),
            lost_asks: use_signal(Vec::new),
            usage: use_signal(|| None),
            config_options: use_signal(Vec::new),
            chat_draft: use_signal(String::new),
            toast: use_signal(|| None),
            attachments: use_signal(Vec::new),
            attach_reading: use_signal(Vec::new),
            tab: use_signal(|| Tab::Home),
            drawer_open: use_signal(|| false),
            code_screen: use_signal(|| crate::code::CodeScreen::List),
            code_client: use_signal(|| None),
            code_conn: use_signal(|| crate::state::ConnState::Disconnected),
            code_chats: use_signal(Vec::new),
            code_chats_loading: use_signal(|| false),
            code_repos: use_signal(Vec::new),
            code_models: use_signal(Vec::new),
            code_models_loading: use_signal(|| false),
            code_agents: use_signal(Vec::new),
            code_agents_from: use_signal(String::new),
            code_agents_loading: use_signal(|| false),
            code_branches: use_signal(crate::code::BranchList::default),
            code_chat: use_signal(crate::code::CodeChatState::default),
            code_permissions: use_signal(Vec::new),
            code_answered: use_signal(HashSet::new),
            code_cache: use_signal(crate::code::CodeCache::default),
            code_epoch: use_signal(|| 0),
            code_poll: use_signal(|| 0),
            code_stream: use_signal(|| None),
            code_diff: use_signal(crate::code::DiffState::default),
            code_pulls: use_signal(crate::code::PullsState::default),
            code_diff_wrap: use_signal(|| true),
            code_draft: use_signal(String::new),
            code_attachments: use_signal(Vec::new),
            new_attachments: use_signal(Vec::new),
            recipes: use_recipes(),
            skills: crate::skills::use_ctx(),
            scheduler: crate::scheduler::use_ctx(),
            extensions: crate::extensions::use_ctx(),
        };
        use_context_provider(|| ctx);
        PROBE_CTX.with(|slot| *slot.borrow_mut() = Some(ctx));
        rsx! { div {} }
    }

    struct Harness {
        rt: tokio::runtime::Runtime,
        dom: VirtualDom,
        ctx: AppCtx,
        /// The connection's event stream, parked for the harness's lifetime.
        /// The client's actor gives up the socket when this end goes away, so
        /// dropping it would leave a connection that died between the
        /// handshake and the first request.
        events: Option<tokio::sync::mpsc::Receiver<goose_acp_client::AcpEvent>>,
    }

    impl Harness {
        fn new() -> Self {
            // `enable_all` rather than `enable_time`: the timer is what the
            // spawned halves need, and the IO driver is what the socket in
            // `connected` needs.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread tokio runtime for the spawned halves");
            PROBE_CTX.with(|slot| *slot.borrow_mut() = None);
            let mut dom = VirtualDom::new(CtxProbe);
            dom.rebuild_in_place();
            let ctx = PROBE_CTX
                .with(|slot| *slot.borrow())
                .expect("the probe rendered, so it published its context");
            Self {
                rt,
                dom,
                ctx,
                events: None,
            }
        }

        /// The same, with a real [`AcpClient`] on the context talking to a
        /// goose that answers `script`.
        fn connected(script: Script) -> (Self, Server) {
            let mut harness = Self::new();
            let server = serve(&harness.rt, script);
            let cfg = ConnectConfig {
                base_url: server.base_url.clone(),
                secret: String::new(),
                fingerprint: None,
            };
            let (client, events, _info) = harness
                .rt
                .block_on(AcpClient::connect(&cfg))
                .expect("the mock server accepted the handshake");
            harness.events = Some(events);
            harness.with(|ctx| ctx.client.clone().set(Some(client)));
            (harness, server)
        }

        /// Touch signals inside the Dioxus runtime without letting anything
        /// the call spawned run yet — which is how the immediate half of an
        /// action is told apart from the half that waits on the server.
        fn with<R>(&self, body: impl FnOnce(&AppCtx) -> R) -> R {
            let ctx = self.ctx;
            self.dom.in_runtime(move || body(&ctx))
        }

        /// Let the spawned tasks run. Dioxus queues them and polls from an
        /// executor, so nothing spawned makes a step without this.
        ///
        /// The budget is 400 ms of *idle* — a slice with work in it returns at
        /// once — against a longest scripted delay of 60 ms, which is what
        /// gives a loaded machine room before a socket round trip becomes a
        /// flake.
        fn settle(&mut self) {
            let Self { rt, dom, .. } = self;
            rt.block_on(async {
                for _ in 0..40 {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
                    dom.render_immediate_to_vec();
                }
            });
        }

        fn act(&mut self, body: impl FnOnce(&AppCtx)) {
            self.with(body);
            self.settle();
        }
    }

    /// A recipes tab nobody has opened yet must not look like one that is
    /// mid-scan, or one whose server has already refused to schedule.
    ///
    /// Both of those are states the detail screen draws differently and
    /// neither is recoverable from the UI, because nothing in this file ever
    /// sets them back: a `Warned` default puts every recipe behind a confirm
    /// it never earned, and a `scheduler_off` default deletes the Schedule
    /// row outright (rule 11) on a server that would happily have taken one.
    #[test]
    fn a_recipes_tab_starts_with_nothing_assumed_about_the_server() {
        let h = Harness::new();
        h.with(|ctx| {
            assert!(
                *ctx.recipes.screen.peek() == Screen::List,
                "the tab opened on a detail screen with no recipe behind it"
            );
            assert!(
                ctx.recipes.open.peek().is_none(),
                "the tab opened holding a recipe nobody chose"
            );
            assert_eq!(
                *ctx.recipes.scan.peek(),
                ScanState::Unknown,
                "a fresh tab is claiming a verdict goose has not given"
            );
            assert!(
                !*ctx.recipes.scheduler_off.peek(),
                "the Schedule row is gone before any server has refused one"
            );
            assert_eq!(
                list_state(&ctx.recipes.list.peek(), true),
                ListState::Empty,
                "a tab that has never fetched must read as empty, not as \
                 loading or as a server without the feature"
            );
        });
    }

    /// Going back has to let go of the recipe, not just change screens.
    ///
    /// The detail screen renders from `open`, and `delete` drops rows from
    /// the list without touching it. Leave the recipe behind and the reader
    /// who deletes one, backs out, and opens another arrives at a detail
    /// screen still showing the deleted recipe — with a Run button on it.
    #[test]
    fn going_back_to_the_list_lets_go_of_the_recipe_it_was_showing() {
        let mut h = Harness::new();
        h.act(|ctx| open(ctx, entry(&plain(), &Value::Null)));
        h.with(|ctx| {
            close(ctx);
            assert!(
                *ctx.recipes.screen.peek() == Screen::List,
                "Back did not return to the list"
            );
            assert!(
                ctx.recipes.open.peek().is_none(),
                "the closed detail screen is still holding its recipe"
            );
        });
    }

    /// The first of this file's two rules, run rather than read: **a tap on a
    /// row opens the recipe, it does not run it.**
    ///
    /// Running one starts an agent on the reader's own machine with their own
    /// tools. The two things `run` does that `open` must not — replace the
    /// composer's draft and move the drawer to Home — are both silent and
    /// both destructive, so this seeds a draft and a tab that would visibly
    /// change if a tap ever started doing double duty.
    #[test]
    fn opening_a_recipe_shows_it_and_starts_nothing() {
        let h = Harness::new();
        h.with(|ctx| {
            let (mut draft, mut tab) = (ctx.chat_draft, ctx.tab);
            draft.set("half a message to someone else".to_owned());
            tab.set(Tab::Recipes);
        });
        h.with(|ctx| open(ctx, entry_id("abc123", &plain(), &Value::Null)));

        h.with(|ctx| {
            assert!(
                *ctx.recipes.screen.peek() == Screen::Detail,
                "the tap did not open the recipe"
            );
            assert_eq!(
                ctx.recipes.open.peek().as_ref().map(|e| e.id.clone()),
                Some("abc123".to_owned()),
                "the detail screen opened on a different recipe than the one \
                 tapped"
            );
            assert_eq!(
                *ctx.recipes.scan.peek(),
                ScanState::Pending,
                "the screen is claiming a verdict before the scan was asked \
                 for"
            );
            assert_eq!(
                ctx.chat_draft.peek().as_str(),
                "half a message to someone else",
                "opening a recipe overwrote the composer — that is what \
                 running one does"
            );
            assert!(
                *ctx.tab.peek() == Tab::Recipes,
                "opening a recipe navigated away from the recipes tab"
            );
        });
    }

    /// A phone that cannot ask for a verdict has to stop asking.
    ///
    /// `Pending` is the state the detail screen spins in. Drop the
    /// `ScanState::Unknown` on the no-client path and every recipe opened
    /// while offline — the common case, since this app reaches its server
    /// over a tailnet — spins forever on a screen whose Run button is
    /// perfectly usable.
    #[test]
    fn a_scan_that_cannot_be_asked_for_settles_instead_of_spinning() {
        let mut h = Harness::new();
        h.act(|ctx| open(ctx, entry(&plain(), &Value::Null)));
        h.with(|ctx| {
            assert_eq!(
                *ctx.recipes.scan.peek(),
                ScanState::Unknown,
                "the scan never resolved, so the detail screen is still \
                 spinning at a reader who is simply offline"
            );
            assert_eq!(
                run_offer(false, *ctx.recipes.scan.peek()),
                RunOffer::Run,
                "a verdict nobody could get must read as clean, or the Run \
                 button disappears on every server without the method"
            );
        });
    }

    /// A refresh with no connection has to be a no-op, and the flag that
    /// proves it is `loading`.
    ///
    /// `refresh` is called from the pull gesture and from an effect that runs
    /// whenever this screen is mounted, so it fires while offline all the
    /// time. `Remote::begin` is what `views/recipes.rs` hangs
    /// `data-refreshing` off, and it clears `sticky` on the way past. Start
    /// the fetch before checking for a client and the reader who walks out of
    /// Wi-Fi and pulls down gets a spinner that spins until the app is
    /// restarted, having first wiped the sentence that said why the last
    /// load failed.
    #[test]
    fn refreshing_with_no_connection_does_not_start_a_spinner_that_cannot_stop() {
        let mut h = Harness::new();
        h.with(|ctx| {
            let mut list = ctx.recipes.list;
            list.write().settle(vec![
                entry(&plain(), &Value::Null),
                entry(&plain(), &json!("0 9 * * *")),
            ]);
            list.write().sticky = Some("transport error: no route".to_owned());
        });
        h.act(refresh);
        h.with(|ctx| {
            let list = ctx.recipes.list.peek();
            assert_eq!(list.items.len(), 2, "the offline refresh emptied the list");
            assert!(
                !list.loading,
                "the offline refresh left the pull-to-refresh spinner running, \
                 and nothing is coming back to stop it"
            );
            assert_eq!(
                list.sticky.as_deref(),
                Some("transport error: no route"),
                "the offline refresh cleared the last failure without \
                 attempting one of its own"
            );
            assert_eq!(
                list_state(&list, false),
                ListState::Rows,
                "the list that was already readable stopped being readable"
            );
        });
    }

    /// Nothing is dropped from the list until goose says it dropped the file.
    ///
    /// The row goes locally on success rather than by refetching, which is
    /// only safe because the removal is on the *`Ok`* arm. Move it out and a
    /// delete tapped with no connection removes the recipe from the screen
    /// while the file is still on the server — and the list is refetched on
    /// every return to it, so it comes back, which reads as the app having
    /// undone the delete.
    #[test]
    fn a_delete_that_never_reached_the_server_keeps_the_row() {
        let mut h = Harness::new();
        h.with(|ctx| {
            let mut list = ctx.recipes.list;
            list.write().settle(vec![
                entry_id("keep-me", &plain(), &Value::Null),
                entry_id("delete-me", &plain(), &Value::Null),
            ]);
        });
        h.act(|ctx| delete(ctx, "delete-me"));
        h.with(|ctx| {
            let ids: Vec<String> = ctx
                .recipes
                .list
                .peek()
                .items
                .iter()
                .map(|entry| entry.id.clone())
                .collect();
            assert_eq!(
                ids,
                ["keep-me", "delete-me"],
                "a delete that never reached goose took the row off the \
                 screen anyway"
            );
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Not connected — reconnect in Settings"),
                "the delete did nothing and said nothing"
            );
        });
    }

    /// A schedule the server never accepted must not appear on the row.
    ///
    /// `apply_schedule` writes the cron into both copies of the recipe, and
    /// the row's dot and its second line are drawn from that. Call it before
    /// the RPC and a phone with no connection paints "Runs every day at 9:00
    /// AM" on a recipe goose has never heard of a schedule for — a claim that
    /// something will run unattended, made on the strength of nothing.
    #[test]
    fn a_schedule_that_never_reached_the_server_is_not_shown_as_set() {
        let mut h = Harness::new();
        h.with(|ctx| {
            let (mut list, mut open) = (ctx.recipes.list, ctx.recipes.open);
            list.write()
                .settle(vec![entry_id("r1", &plain(), &Value::Null)]);
            open.set(Some(entry_id("r1", &plain(), &Value::Null)));
        });
        h.act(|ctx| set_schedule(ctx, "r1", Some("0 0 9 * * *".to_owned())));
        h.with(|ctx| {
            assert_eq!(
                row_meta(&ctx.recipes.list.peek().items[0]).schedule,
                None,
                "the row is advertising a schedule the server never took"
            );
            assert_eq!(
                ctx.recipes
                    .open
                    .peek()
                    .as_ref()
                    .and_then(|e| e.schedule_cron.clone()),
                None,
                "the detail screen is showing a schedule the server never took"
            );
            assert!(
                !*ctx.recipes.scheduler_off.peek(),
                "a phone that was merely offline has concluded the server has \
                 no scheduler, and nothing ever sets that back"
            );
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Not connected — reconnect in Settings")
            );
        });
    }

    /// An accepted schedule has to land on both copies of the recipe, and on
    /// no others.
    ///
    /// The open recipe is a clone of its row, not a pointer into the list,
    /// and goose pushes nothing when a schedule changes. Write only the open
    /// copy and the sheet's confirmation is gone the moment the reader goes
    /// back; write only the list and the screen they are looking at never
    /// changes at all.
    #[test]
    fn an_accepted_schedule_lands_on_the_row_and_on_the_open_recipe() {
        let h = Harness::new();
        h.with(|ctx| {
            let (mut list, mut open) = (ctx.recipes.list, ctx.recipes.open);
            list.write().settle(vec![
                entry_id("r1", &plain(), &Value::Null),
                entry_id("r2", &plain(), &Value::Null),
            ]);
            open.set(Some(entry_id("r1", &plain(), &Value::Null)));
        });

        h.with(|ctx| {
            apply_schedule(ctx, "r1", Some("0 30 8 * * 1-5"));
            let list = ctx.recipes.list.peek();
            assert_eq!(
                row_meta(&list.items[0]).schedule.as_deref(),
                Some("Runs every weekday at 8:30 AM"),
                "the list row never learned about the schedule that was just \
                 saved, so backing out of the sheet loses it"
            );
            assert_eq!(
                row_meta(&list.items[1]).schedule,
                None,
                "scheduling one recipe scheduled another"
            );
            assert_eq!(
                ctx.recipes
                    .open
                    .peek()
                    .as_ref()
                    .and_then(|e| e.schedule_cron.clone())
                    .as_deref(),
                Some("0 30 8 * * 1-5"),
                "the screen the reader is looking at did not change"
            );
        });

        // Scheduling a recipe that is not the open one must leave the open
        // one alone: the two ids are compared for a reason.
        h.with(|ctx| {
            apply_schedule(ctx, "r2", Some("0 0 * * * *"));
            assert_eq!(
                ctx.recipes
                    .open
                    .peek()
                    .as_ref()
                    .and_then(|e| e.schedule_cron.clone())
                    .as_deref(),
                Some("0 30 8 * * 1-5"),
                "another recipe's new schedule was written onto the open one"
            );
        });

        // Off is the same path, and it has to clear both copies too.
        h.with(|ctx| {
            apply_schedule(ctx, "r1", None);
            assert_eq!(
                row_meta(&ctx.recipes.list.peek().items[0]).schedule,
                None,
                "the row is still showing a schedule that was turned off"
            );
            assert_eq!(
                ctx.recipes
                    .open
                    .peek()
                    .as_ref()
                    .and_then(|e| e.schedule_cron.clone()),
                None,
                "the detail screen is still showing a schedule that was \
                 turned off"
            );
        });

        // And the reply can land after the reader has already gone back:
        // `set_schedule` spawns, and `close` empties `open` while the call is
        // still in flight. The row is then the only copy left, so it is the
        // one that must not be missed.
        h.with(|ctx| {
            close(ctx);
            apply_schedule(ctx, "r1", Some("0 30 8 * * 1-5"));
            assert_eq!(
                row_meta(&ctx.recipes.list.peek().items[0])
                    .schedule
                    .as_deref(),
                Some("Runs every weekday at 8:30 AM"),
                "a schedule goose accepted after the reader went back was \
                 dropped, so the list says the recipe is on no timer while \
                 the server runs it every weekday"
            );
        });
    }

    /// Run puts the recipe's prompt in the composer, exactly as stored, and
    /// moves to the stack the chat will appear in.
    ///
    /// Verbatim is the point: the template markers are what tell the reader
    /// this prompt has holes in it, and the second tap of a two-tap flow is
    /// their last chance to read it before an agent acts. And the chat lives
    /// in the Home stack — drop the `tab.set` and launching a recipe leaves
    /// the drawer on Recipes while the chat opens out of sight behind it, so
    /// the reader sees the list they started from and assumes nothing
    /// happened.
    #[test]
    fn running_a_recipe_pre_fills_the_composer_and_follows_the_chat() {
        let mut h = Harness::new();
        let entry = entry(
            &json!({
                "title": "Draft release notes",
                "description": "Notes since the last tag.",
                "prompt": "Summarise {{ repo }} since {{ tag }}.",
            }),
            &Value::Null,
        );
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Recipes);
        });
        h.act(|ctx| run(ctx, &entry));
        h.with(|ctx| {
            assert_eq!(
                ctx.chat_draft.peek().as_str(),
                "Summarise {{ repo }} since {{ tag }}.",
                "the prompt reached the composer changed, so the reader is \
                 not reading what the recipe actually says"
            );
            assert!(
                *ctx.tab.peek() == Tab::Home,
                "the recipe's chat opens in the Home stack, and the drawer \
                 stayed on Recipes"
            );
        });
    }

    /// A recipe with no prompt of its own opens an **empty** composer.
    ///
    /// The draft lives on the context, so it holds whatever was last typed
    /// anywhere in the app. Set it only when the recipe has a prompt and a
    /// prompt-less recipe opens its session with a stranger's half-written
    /// message already in the box, looking exactly like something the recipe
    /// put there — one tap from being sent to an agent.
    #[test]
    fn a_recipe_with_no_prompt_does_not_inherit_the_last_thing_you_typed() {
        let mut h = Harness::new();
        let entry = entry(&plain(), &Value::Null);
        h.with(|ctx| {
            let mut draft = ctx.chat_draft;
            draft.set("rm -rf the staging bucket".to_owned());
        });
        h.act(|ctx| run(ctx, &entry));
        h.with(|ctx| {
            assert_eq!(
                ctx.chat_draft.peek().as_str(),
                "",
                "a recipe with no prompt opened its session with someone \
                 else's half-written message in the composer"
            );
        });
    }

    // ------------------------------------------------------- over the wire
    //
    // Everything above stops at the first `let Some(client)`. What follows is
    // the other side of every one of those: the request that went out, and
    // what this file does with the answer.

    /// The method name with goose's recipe namespace taken off.
    fn short(method: &str) -> &str {
        method.trim_start_matches("_goose/unstable/recipes/")
    }

    /// One `recipes/list` row as goose puts it on the wire.
    fn wire_entry(id: &str, title: &str, cron: &Value) -> Value {
        json!({
            "id": id,
            "recipe": { "title": title, "description": "What happened yesterday." },
            "file_path": format!("/home/me/.config/goose/recipes/{id}.yaml"),
            "last_modified": "2026-08-23T18:42:11.508233+00:00",
            "schedule_cron": cron,
            "slash_command": null,
        })
    }

    /// The ids in the list, in order.
    fn ids(ctx: &AppCtx) -> Vec<String> {
        ctx.recipes
            .list
            .peek()
            .items
            .iter()
            .map(|entry| entry.id.clone())
            .collect()
    }

    /// Every method this server was asked for, in order, without the
    /// namespace and without the handshake.
    fn asked(server: &Server) -> Vec<String> {
        server
            .log()
            .iter()
            .map(|(method, _)| short(method).to_owned())
            .filter(|method| method != "initialize")
            .collect()
    }

    /// A fetch that reaches goose replaces the list with what goose holds,
    /// stops the spinner, and brings each row's schedule with it.
    ///
    /// The offline half of this is already checked; this is the half where
    /// something comes back. Losing the settle would leave the pull gesture
    /// spinning over an empty screen forever — the same symptom as the offline
    /// bug, from the opposite cause — and losing the mapping would put rows on
    /// screen with no schedule line on the one that is on a timer, which is
    /// the app failing to mention that something runs unattended.
    #[test]
    fn a_list_that_arrives_replaces_the_screen_and_stops_the_spinner() {
        fn two_recipes(method: &str, _params: &Value) -> Reply {
            if short(method) == "list" {
                return ok(json!({
                    "recipes": [
                        wire_entry("r1", "Morning standup", &Value::Null),
                        wire_entry("r2", "Release notes", &json!("30 8 * * 1-5")),
                    ]
                }));
            }
            ok(json!({}))
        }
        let (mut h, server) = Harness::connected(two_recipes);
        h.act(refresh);
        h.with(|ctx| {
            assert_eq!(
                ids(ctx),
                ["r1", "r2"],
                "the list goose answered with never reached the screen"
            );
            let list = ctx.recipes.list.peek();
            assert!(
                !list.loading,
                "the fetch answered and the pull spinner is still going"
            );
            assert_eq!(list_state(&list, true), ListState::Rows);
            assert_eq!(
                row_meta(&list.items[1]).schedule.as_deref(),
                Some("Runs every weekday at 8:30 AM"),
                "a recipe goose says is on a timer arrived with no schedule \
                 line, so nothing on screen says it runs unattended"
            );
            assert_eq!(row_meta(&list.items[0]).schedule, None);
        });
        assert_eq!(asked(&server), ["list"]);
    }

    /// The verdict goose gives is the verdict the Run button wears.
    ///
    /// Three answers, three states, and the mapping between them is silent
    /// either way it breaks: read `true` as clean and a recipe goose flagged
    /// runs on one tap; read an error as `Warned` and every server without the
    /// method puts a confirm in front of every recipe it has.
    #[test]
    fn goose_s_own_verdict_is_what_decides_how_hard_run_is_to_press() {
        fn scan_by_title(method: &str, params: &Value) -> Reply {
            if short(method) == "scan" {
                return match params["recipe"]["title"].as_str().unwrap_or_default() {
                    "Flagged" => ok(json!({ "has_security_warnings": true })),
                    "Clean" => ok(json!({ "has_security_warnings": false })),
                    _ => rpc_error(-32602, "that recipe will not parse"),
                };
            }
            ok(json!({}))
        }
        let (mut h, server) = Harness::connected(scan_by_title);
        for (title, verdict, offer) in [
            ("Flagged", ScanState::Warned, RunOffer::Confirm),
            ("Clean", ScanState::Clean, RunOffer::Run),
            ("Unreadable", ScanState::Unknown, RunOffer::Run),
        ] {
            let body = json!({ "title": title, "description": "D" });
            h.act(|ctx| open(ctx, entry_id(title, &body, &Value::Null)));
            h.with(|ctx| {
                assert_eq!(
                    *ctx.recipes.scan.peek(),
                    verdict,
                    "goose answered for {title} and the screen is showing a \
                     different verdict"
                );
                assert_eq!(run_offer(false, *ctx.recipes.scan.peek()), offer);
            });
        }
        assert_eq!(
            asked(&server),
            ["scan", "scan", "scan"],
            "an open either asked twice or did not ask"
        );
    }

    /// A verdict that arrives after the reader has opened something else must
    /// be dropped, not shown.
    ///
    /// One RPC per open and no cancellation, so two opens in quick succession
    /// leave two scans in flight for one slot. Without the identity check the
    /// slower one wins simply by being slower — and the screen then puts a
    /// confirm in front of a recipe goose called clean, or takes one away from
    /// a recipe goose flagged. The reader has no way to tell either happened.
    #[test]
    fn a_verdict_for_a_recipe_the_reader_has_left_never_lands_on_the_open_one() {
        fn slow_for_the_flagged_one(method: &str, params: &Value) -> Reply {
            if short(method) == "scan" {
                if params["recipe"]["title"] == json!("Flagged") {
                    return (
                        Duration::from_millis(60),
                        Ok(json!({ "has_security_warnings": true })),
                    );
                }
                return ok(json!({ "has_security_warnings": false }));
            }
            ok(json!({}))
        }
        let (mut h, _server) = Harness::connected(slow_for_the_flagged_one);
        h.with(|ctx| {
            let flagged = json!({ "title": "Flagged", "description": "D" });
            let clean = json!({ "title": "Clean", "description": "D" });
            open(ctx, entry_id("flagged", &flagged, &Value::Null));
            open(ctx, entry_id("clean", &clean, &Value::Null));
        });
        h.settle();
        h.with(|ctx| {
            assert_eq!(
                ctx.recipes.open.peek().as_ref().map(|e| e.id.clone()),
                Some("clean".to_owned()),
                "the screen is not on the recipe this test is about"
            );
            assert_eq!(
                *ctx.recipes.scan.peek(),
                ScanState::Clean,
                "the late verdict for a recipe the reader has already left \
                 landed on the one in front of them, so the screen is warning \
                 about a recipe goose called clean"
            );
        });
    }

    /// The row goes when goose says the file is gone, and stays when it says
    /// anything else.
    ///
    /// Dropping the row locally is only safe because it is on the `Ok` arm.
    /// The list is refetched on every return to it, so a row removed over a
    /// refusal comes straight back — which reads as the app having undone the
    /// delete rather than as the delete never having happened.
    #[test]
    fn a_delete_goose_took_drops_the_row_and_one_it_refused_says_why() {
        fn only_r2_exists(method: &str, params: &Value) -> Reply {
            if short(method) == "delete" {
                if params["id"] == json!("r2") {
                    return ok(json!({}));
                }
                return rpc_error(-32602, "no recipe with that id");
            }
            ok(json!({}))
        }
        let (mut h, server) = Harness::connected(only_r2_exists);
        h.with(|ctx| {
            let mut list = ctx.recipes.list;
            list.write().settle(vec![
                entry_id("r1", &plain(), &Value::Null),
                entry_id("r2", &plain(), &Value::Null),
            ]);
        });

        h.act(|ctx| delete(ctx, "r2"));
        h.with(|ctx| {
            assert_eq!(
                ids(ctx),
                ["r1"],
                "goose unlinked the file and the row is still on screen, so \
                 the next tap on it deletes something that is already gone"
            );
            assert_eq!(
                ctx.toast.peek().as_deref(),
                None,
                "a delete that worked toasted over the row it had just removed"
            );
        });

        h.act(|ctx| delete(ctx, "r1"));
        h.with(|ctx| {
            assert_eq!(
                ids(ctx),
                ["r1"],
                "a delete goose refused took the row off the screen anyway — \
                 and the next list brings it back, which reads as an undo"
            );
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Delete failed: no recipe with that id")
            );
        });
        assert_eq!(asked(&server), ["delete", "delete"]);
        assert_eq!(server.log()[1].1, json!({ "id": "r2" }));
    }

    /// A schedule goose accepted is written into both copies of the recipe and
    /// said out loud in words.
    ///
    /// The toast is the sheet's only answer — nothing else on screen changes
    /// at the moment of saving — and it is the cron as a sentence rather than
    /// the cron, because the reader picked "every weekday at 8:30" and never
    /// typed `30 8 * * 1-5`.
    #[test]
    fn a_schedule_goose_accepted_is_shown_on_the_row_and_said_in_words() {
        fn takes_anything(method: &str, _params: &Value) -> Reply {
            let _ = short(method);
            ok(json!({}))
        }
        let (mut h, server) = Harness::connected(takes_anything);
        h.with(|ctx| {
            let (mut list, mut open) = (ctx.recipes.list, ctx.recipes.open);
            list.write()
                .settle(vec![entry_id("r1", &plain(), &Value::Null)]);
            open.set(Some(entry_id("r1", &plain(), &Value::Null)));
        });

        h.act(|ctx| set_schedule(ctx, "r1", Some("30 8 * * 1-5".to_owned())));
        h.with(|ctx| {
            assert_eq!(
                row_meta(&ctx.recipes.list.peek().items[0])
                    .schedule
                    .as_deref(),
                Some("Runs every weekday at 8:30 AM"),
                "goose took the schedule and the row does not show it, so the \
                 reader has no way to tell the save worked"
            );
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Runs every weekday at 8:30 AM"),
                "the sheet closed without saying what it saved, or said it as \
                 the cron nobody typed"
            );
            assert!(
                !*ctx.recipes.scheduler_off.peek(),
                "a schedule that was accepted concluded the server has no \
                 scheduler, which deletes the Schedule row for good"
            );
        });

        // Off is the same call with a null cron, and it gets its own sentence
        // rather than a summary of nothing.
        h.act(|ctx| set_schedule(ctx, "r1", None));
        h.with(|ctx| {
            assert_eq!(row_meta(&ctx.recipes.list.peek().items[0]).schedule, None);
            assert_eq!(ctx.toast.peek().as_deref(), Some("Schedule removed"));
        });
        assert_eq!(asked(&server), ["schedule", "schedule"]);
        assert_eq!(
            server.log()[2].1,
            json!({ "id": "r1", "cron_schedule": Value::Null }),
            "removing a schedule has to say so on the wire, not by leaving the \
             key out"
        );
    }

    /// "Not enabled" is a fact about the server and is remembered; anything
    /// else is one call that failed and is not.
    ///
    /// The difference is what the Schedule row does next time. Rule 11 does
    /// not let the app go on offering a control it has been told does nothing,
    /// and nothing in this file ever sets the flag back — so latching on an
    /// ordinary refusal would delete the row for the rest of the session over
    /// a cron that simply would not parse.
    #[test]
    fn only_a_server_without_a_scheduler_takes_the_schedule_row_away() {
        fn refuses(method: &str, params: &Value) -> Reply {
            if short(method) == "schedule" {
                if params["id"] == json!("no-scheduler") {
                    return (
                        Duration::ZERO,
                        // goose's own shape when `--enable-scheduler` is off:
                        // method-not-found with the reason in `data`.
                        Err(json!({
                            "code": -32601,
                            "message": "Method not found",
                            "data": "Scheduled recipe execution is not enabled",
                        })),
                    );
                }
                return rpc_error(-32602, "that cron will not parse");
            }
            ok(json!({}))
        }
        let (mut h, _server) = Harness::connected(refuses);

        h.act(|ctx| set_schedule(ctx, "bad-cron", Some("30 8 * * 1-5".to_owned())));
        h.with(|ctx| {
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Schedule not saved: that cron will not parse")
            );
            assert!(
                !*ctx.recipes.scheduler_off.peek(),
                "one call that failed convinced the app this server has no \
                 scheduler, and nothing ever sets that back — so the Schedule \
                 row is gone for the rest of the session"
            );
        });

        h.act(|ctx| set_schedule(ctx, "no-scheduler", Some("30 8 * * 1-5".to_owned())));
        h.with(|ctx| {
            assert!(
                *ctx.recipes.scheduler_off.peek(),
                "the server said scheduling is not enabled and the app went on \
                 offering the row that asks for it"
            );
            assert_eq!(
                ctx.toast.peek().as_deref(),
                Some("Schedule not saved: Scheduler: Scheduled recipe execution is not enabled"),
                "the refusal did not carry goose's own reason, which is the \
                 only thing that points at the flag on the server"
            );
        });
    }
}
