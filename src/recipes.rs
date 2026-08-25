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
    reason = "test fixtures: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;

    /// A list row as goose sends one. Written as JSON rather than as a struct
    /// literal so the test says what is on the wire, which is the thing the
    /// screens are reading.
    fn entry(recipe: &Value, schedule_cron: &Value) -> RecipeListEntry {
        serde_json::from_value(json!({
            "id": "9f2c41ab6d3e0517",
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
}
