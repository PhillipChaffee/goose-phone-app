//! The two recipe screens and the sheet that schedules one.
//!
//! Components only: every decision this file renders — which sentence an
//! empty list gets, whether there is a Run button, which rows the schedule
//! sheet holds — is a free function in `crate::recipes` or `crate::cron`,
//! taken over plain data and tested there. What is left here is markup.

use dioxus::prelude::*;
use goose_acp_client::RecipeListEntry;

use crate::cron::{self, Schedule};
use crate::icons::Icon;
use crate::nav::Crumb;
use crate::recipes::{
    close, facts, list_state, open, refresh, row_meta, run, run_offer, schedule_rows, set_schedule,
    ListState, RunOffer, ScanState,
};
use crate::state::{relative_time, rfc3339_to_epoch, use_app_ctx, AppCtx};
use crate::views::chrome::{ListRow, RowAction, TopBar};
use crate::views::session_settings::SettingRow;
use crate::views::ConfirmDelete;

/// The recipe list: what the server has saved, newest file first.
///
/// No refresh control anywhere on it. The scroller names itself with
/// `data-refresh` and the list comes back by pull — a button in the bar is a
/// permanent piece of chrome for something a gesture already does.
#[component]
pub fn RecipesView() -> Element {
    let ctx = use_app_ctx();
    let remote = (ctx.recipes.list)();
    let connected = (ctx.conn)().is_connected();
    let state = list_state(&remote, connected);
    let mut confirm_delete = use_signal(|| None::<RecipeListEntry>);

    // Reactive rather than a one-shot mount hook: the list is worth fetching
    // the moment there is a connection to fetch it over, including one that
    // arrives after this screen is already up.
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            refresh(&ctx);
        }
    });

    rsx! {
        TopBar { title: "Recipes", conn: true }
        main {
            class: "scroll",
            "data-refresh": "recipes",
            "data-refreshing": "{remote.loading}",

            match &state {
                ListState::Unsupported => rsx! {
                    // Not an error: no tint, no Retry, because retrying is not
                    // a thing that could work.
                    p { class: "empty", "This goose server does not have recipes." }
                },
                ListState::Offline => rsx! {
                    // Deliberately not an empty state, which would say there
                    // are none — the app has no idea, and that is the point.
                    p { class: "error-box", "Not connected. Recipes live on your goose server." }
                },
                ListState::Failed(error) => rsx! {
                    p { class: "error-box", "{error}" }
                },
                ListState::Loading => rsx! {
                    p { class: "empty", "Loading recipes…" }
                },
                ListState::Empty => rsx! {
                    p { class: "empty",
                        "No recipes yet. Ask goose in a chat to save one, or add one from your desktop."
                    }
                },
                ListState::Rows => rsx! {
                    ul { class: "session-list",
                        for entry in remote.items.iter() {
                            {row(&ctx, entry, confirm_delete)}
                        }
                    }
                },
            }
        }

        if let Some(entry) = confirm_delete() {
            ConfirmDelete {
                title: "Delete this recipe?",
                body: "The recipe file goes from the goose server. This cannot be undone.",
                on_cancel: move |()| confirm_delete.set(None),
                on_confirm: move |()| {
                    confirm_delete.set(None);
                    crate::recipes::delete(&ctx, &entry.id);
                },
            }
        }
    }
}

/// One recipe as a row: the title, how long ago the file changed, what is
/// true about it, and what it is for.
fn row(
    ctx: &AppCtx,
    entry: &RecipeListEntry,
    mut confirm_delete: Signal<Option<RecipeListEntry>>,
) -> Element {
    // Which row the desktop's detail column came from. Ignored on the phone,
    // where the list is not on screen beside it (`views::chrome::row_is_marked`).
    let selected = ctx
        .recipes
        .open
        .read()
        .as_ref()
        .is_some_and(|open| open.id == entry.id);
    let ctx = *ctx;
    let meta = row_meta(entry);
    let age = rfc3339_to_epoch(&entry.last_modified).map(relative_time);
    let (open_entry, delete_entry) = (entry.clone(), entry.clone());

    rsx! {
        ListRow {
            key: "{entry.id}",
            icon: "book",
            title: "{entry.recipe.title}",
            trailing: age,
            selected,
            actions: vec![RowAction::delete(EventHandler::new(move |()| {
                confirm_delete.set(Some(delete_entry.clone()));
            }))],
            on_open: move |()| open(&ctx, open_entry.clone()),

            if meta.schedule.is_some() || meta.inputs.is_some() {
                div { class: "session-meta recipe-meta",
                    if let Some(schedule) = meta.schedule {
                        // The dot is the state; an unscheduled recipe has no
                        // state and therefore no dot (design rule 8).
                        span {
                            span { class: "dot on" }
                            " {schedule}"
                        }
                    }
                    if let Some(inputs) = meta.inputs {
                        span { "{inputs}" }
                    }
                }
            }
            div { class: "session-quote", "{entry.recipe.description}" }
        }
    }
}

/// What the open recipe is called, once.
///
/// Read by two things that are never on screen together: the header below,
/// and — on the desktop — the window's own bar, which takes the heading out of
/// the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop/mod.rs`, `assets/desktop.css`). The `None` arm is the same
/// dead-end fallback the view renders, for the same reason: the open recipe
/// and the Detail screen are set and cleared together, so it is unreachable —
/// but a window bar naming nothing would be worse than one naming the kind of
/// thing that was there.
pub(crate) fn crumb(ctx: &AppCtx) -> Crumb {
    (ctx.recipes.open)().map_or_else(
        || Crumb::plain("Recipe"),
        |entry| {
            Crumb::detailed(
                entry.recipe.title.clone(),
                entry.slash_command.as_ref().map(|name| format!("/{name}")),
            )
        },
    )
}

/// One recipe, in full: what it says, what it pins, when it runs, and the one
/// button that starts it.
#[component]
pub fn RecipeDetailView() -> Element {
    let ctx = use_app_ctx();
    // Both hooks before the early return below: a hook that runs on one
    // render and not the next is how a Dioxus component starts reading
    // another one's state.
    let mut sheet_open = use_signal(|| false);
    let mut confirm_run = use_signal(|| false);

    // The one expression the window's bar also reads, so a recipe cannot be
    // called one thing in the pane and another in the chrome. Both arms
    // hand `TopBar` a `String` equal to the one the expression they replace
    // produced, into the same prop of the same component — so the phone's
    // captured markup does not move.
    let bar = crumb(&ctx);

    let Some(entry) = (ctx.recipes.open)() else {
        // Unreachable: the open recipe and the Detail screen are set and
        // cleared together. A dead end would not be, so this keeps a way
        // back rather than rendering nothing.
        return rsx! {
            TopBar { title: bar.title, on_back: move |()| close(&ctx), conn: true }
            main { class: "scroll", p { class: "empty", "That recipe is no longer open." } }
        };
    };
    let scan = (ctx.recipes.scan)();
    let offer = run_offer(entry.needs_input(), scan);

    rsx! {
        TopBar {
            title: bar.title,
            subtitle: bar.subtitle,
            on_back: move |()| close(&ctx),
            conn: true,
        }
        // `has-fab` is room for the button below, so it is spent only when
        // there is a button: a Blocked recipe renders no FAB, and the padding
        // would be 100px of nothing under a card that is often short.
        main { class: if offer == RunOffer::Blocked { "scroll" } else { "scroll has-fab" },
            if scan == ScanState::Warned {
                // Persistent, not a toast: it is the reason the button below
                // asks twice, and it has to still be there when you reach it.
                p { class: "error-box",
                    "This recipe contains hidden characters. Read it before running it."
                }
            }

            {prose(&entry)}

            {card(&ctx, &entry, sheet_open)}
        }

        match offer {
            RunOffer::Run => rsx! {
                button {
                    class: "fab",
                    onclick: {
                        let entry = entry.clone();
                        move |_| run(&ctx, &entry)
                    },
                    Icon { name: "play" }
                    "Run"
                }
            },
            RunOffer::Confirm => rsx! {
                button {
                    class: "fab",
                    onclick: move |_| confirm_run.set(true),
                    Icon { name: "play" }
                    "Run"
                }
            },
            // No button, and the facts card says why. A disabled one would be
            // a control that does nothing (design rule 11).
            RunOffer::Blocked => rsx! {},
        }

        if confirm_run() {
            ConfirmDelete {
                title: "Run this recipe anyway?",
                body: "goose found characters in this recipe that do not show on screen. \
                       A recipe runs with your tools, on your machine.",
                confirm_label: "Run anyway",
                on_cancel: move |()| confirm_run.set(false),
                on_confirm: {
                    let entry = entry.clone();
                    move |()| {
                        confirm_run.set(false);
                        run(&ctx, &entry);
                    }
                },
            }
        }

        if sheet_open() {
            CronSheet {
                title: entry.recipe.title.clone(),
                current: entry.schedule_cron.as_deref().and_then(cron::parse),
                on_close: move |()| sheet_open.set(false),
                on_save: {
                    // Moved, not cloned: the sheet is the last thing this
                    // screen builds, so nothing needs the entry afterwards.
                    let id = entry.id;
                    move |cron: Option<String>| {
                        sheet_open.set(false);
                        set_schedule(&ctx, &id, cron);
                    }
                },
            }
        }
    }
}

/// The recipe in its own words: description, instructions, and the message
/// that will be waiting in the composer.
///
/// Prose on the page rather than in a panel, in the serif — this is the same
/// voice an agent reply is set in, because it is the same kind of text
/// (design rules 1 and 6).
fn prose(entry: &RecipeListEntry) -> Element {
    let description = crate::markdown::to_html(&entry.recipe.description);
    let instructions = entry
        .recipe
        .instructions
        .as_deref()
        .map(crate::markdown::to_html);
    let prompt = entry.recipe.prompt.as_deref().map(crate::markdown::to_html);

    rsx! {
        div { class: "md recipe-prose", dangerous_inner_html: "{description}" }
        if let Some(instructions) = instructions {
            div { class: "md recipe-prose", dangerous_inner_html: "{instructions}" }
        }
        if let Some(prompt) = prompt {
            label { class: "field-label", "Opens with" }
            div { class: "md recipe-prose", dangerous_inner_html: "{prompt}" }
        }
    }
}

/// The facts card, with the schedule row at the bottom of it.
///
/// One card and one list, because the settings sheet's grammar already holds
/// both shapes: the facts are facts, the schedule is the one thing on this
/// screen that can be changed, and the chevron is the only difference between
/// them (design rule 11).
fn card(ctx: &AppCtx, entry: &RecipeListEntry, sheet_open: Signal<bool>) -> Element {
    let facts = facts(entry);
    let schedule = schedule_row(ctx, entry, sheet_open);

    rsx! {
        section { class: "card",
            div { class: "setting-list",
                for row in facts.iter() {
                    {fact_row(row)}
                }
                {schedule}
            }
        }
    }
}

/// The Schedule row, in whichever of the three shapes this server and this
/// cron string have earned.
fn schedule_row(ctx: &AppCtx, entry: &RecipeListEntry, mut sheet_open: Signal<bool>) -> Element {
    let cron = entry
        .is_scheduled()
        .then(|| entry.schedule_cron.clone().unwrap_or_default());
    let value = cron
        .as_deref()
        .map_or_else(|| "Not scheduled".to_owned(), cron::summary);

    if (ctx.recipes.scheduler_off)() {
        return fact_row(&SettingRow::fact(
            "schedule",
            "Schedule",
            value,
            "This goose server was started without --enable-scheduler, so it \
             cannot run anything on a timer.",
        ));
    }

    // A cron nobody here can express is shown as itself and left alone. The
    // raw string is evidence rather than state — it is the only way to
    // recognise the schedule on the machine that can edit it — and the row
    // stops being pressable, because pressing it could only rewrite it into
    // something else (design rule 11).
    if let Some(cron) = cron.as_deref() {
        if cron::parse(cron).is_none() {
            return fact_row(&SettingRow::fact(
                "schedule",
                "Schedule",
                cron,
                "Set on another client, in a form this phone cannot edit — change it there.",
            ));
        }
    }

    rsx! {
        button {
            class: "setting-row",
            onclick: move |_| sheet_open.set(true),
            span { class: "setting-main",
                span { class: "setting-name", "Schedule" }
                span { class: "setting-value", "{value}" }
            }
            Icon { name: "chevron-right" }
        }
    }
}

/// A fact in the settings sheet's row grammar.
///
/// Spelled here rather than borrowed from `session_settings`, whose renderer
/// is private to the sheet it drives and whose control rows push a drill-in
/// signal rather than opening an overlay. Same classes, same two shapes, so
/// the card reads as the sheet does.
///
/// `pub(crate)` because the Scheduler's detail draws the same card: a job's
/// recipe, its last run and a cadence it cannot edit are facts in exactly this
/// grammar, and a second copy of eight lines of markup is a second thing to
/// keep in step with the stylesheet.
pub(crate) fn fact_row(row: &SettingRow) -> Element {
    rsx! {
        div { key: "{row.id}", class: "setting-row fact",
            span { class: "setting-main",
                span { class: "setting-name", "{row.name}" }
                span { class: "setting-value", "{row.value}" }
                if let Some(note) = row.note.as_ref() {
                    span { class: "setting-note", "{note}" }
                }
            }
        }
    }
}

/// A schedule, as choices.
///
/// The settings sheet's two-level grammar — a list of rows, each drilling
/// into a list of values — so this needs no overlay type of its own and
/// `domdump` files it under the same `-sheet` and `-choices` keys everything
/// else uses.
///
/// **There is no validation state anywhere in it**, because every combination
/// it can produce is a legal cron: the rows are the alphabet, `crate::cron`
/// spells the string, and nothing here can be typed. That is also why the
/// sheet keeps a draft and saves once — five taps to say "weekly, Monday,
/// 9:30" is one schedule, not five round trips.
///
/// Shared with the Scheduler's detail screen, which edits an existing job's
/// cadence through the same rows and the same `crate::cron` grammar. The only
/// difference is which method Save calls — and which questions get asked, for
/// which see `rows`.
#[component]
pub(crate) fn CronSheet(
    title: String,
    /// The schedule as it stands, or `None` when the recipe has none.
    current: Option<Schedule>,
    /// Whether "Never" is one of the answers.
    ///
    /// It is here on a recipe, whose schedule can be removed with a null cron,
    /// and not on a scheduled job, where `schedules/update` takes a required
    /// cron and there is no null to send. A job with no cadence is not a job
    /// with a blank one; it is a deleted one, which is a different question
    /// with a confirm of its own. Offering the choice anyway would be a
    /// control with no method behind it — design rule 11.
    ///
    /// A prop rather than two sheets, because the difference is one entry in
    /// one list and everything else about the two is identical.
    #[props(default = true)]
    allow_never: bool,
    on_save: EventHandler<Option<String>>,
    on_close: EventHandler<()>,
) -> Element {
    let mut schedule = use_signal(|| current.unwrap_or_default());
    let mut on = use_signal(|| current.is_some());
    let mut open_row = use_signal(|| None::<String>);

    let rows = if allow_never {
        schedule_rows(schedule(), on())
    } else {
        crate::scheduler::cadence_rows(schedule(), on())
    };
    let drilled = open_row().and_then(|id| rows.iter().find(|row| row.id == id).cloned());

    let body = match drilled {
        Some(row) => rsx! {
            div { class: "sheet-head",
                button {
                    class: "icon-btn back",
                    title: "Back",
                    onclick: move |_| open_row.set(None),
                    Icon { name: "chevron-left" }
                }
                h2 { "{row.name}" }
            }
            div { class: "choice-list",
                for choice in row.choices.iter() {
                    button {
                        key: "{choice.value}",
                        class: if row.current.as_deref() == Some(choice.value.as_str()) {
                            "choice selected"
                        } else {
                            "choice"
                        },
                        onclick: {
                            let (id, value) = (row.id.clone(), choice.value.clone());
                            move |_| {
                                crate::recipes::choose(
                                    &mut schedule.write(),
                                    &mut on.write(),
                                    &id,
                                    &value,
                                );
                                open_row.set(None);
                            }
                        },
                        span { class: "choice-name", "{choice.label}" }
                        if row.current.as_deref() == Some(choice.value.as_str()) {
                            Icon { name: "check" }
                        }
                    }
                }
            }
        },
        None => rsx! {
            h2 { "Schedule" }
            p { class: "modal-session", "{title}" }
            div { class: "setting-list",
                for row in rows.iter() {
                    {control_row(row, open_row)}
                }
                // The live sentence: what these choices mean, updated as they
                // are made, so nobody has to imagine the cron they add up to.
                {fact_row(&SettingRow::fact(
                    "summary",
                    "Result",
                    if on() { cron::describe(schedule()) } else { "Not scheduled".to_owned() },
                    "goose runs this on the server, whether or not the phone is on.",
                ))}
            }
            div { class: "modal-actions",
                button {
                    class: "btn secondary",
                    onclick: move |_| on_close.call(()),
                    "Cancel"
                }
                button {
                    class: "btn primary",
                    onclick: move |_| {
                        on_save.call(on().then(|| cron::build(schedule())));
                    },
                    "Save"
                }
            }
        },
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
            div {
                class: "modal sheet",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                {body}
            }
        }
    }
}

/// A control in the sheet's row grammar: name, current value, chevron.
fn control_row(row: &SettingRow, mut open_row: Signal<Option<String>>) -> Element {
    let id = row.id.clone();
    rsx! {
        button {
            key: "{row.id}",
            class: "setting-row",
            onclick: move |_| open_row.set(Some(id.clone())),
            span { class: "setting-main",
                span { class: "setting-name", "{row.name}" }
                span { class: "setting-value", "{row.value}" }
                if let Some(note) = row.note.as_ref() {
                    span { class: "setting-note", "{note}" }
                }
            }
            Icon { name: "chevron-right" }
        }
    }
}
