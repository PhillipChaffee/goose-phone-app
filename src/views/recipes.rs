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

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;

    use std::any::Any;
    use std::cell::Cell;
    use std::rc::Rc;

    use dioxus::dioxus_core::{AttributeValue, ElementId, Event, Mutation};
    use dioxus::html::{PlatformEventData, SerializedHtmlEventConverter, SerializedMouseData};
    use serde_json::{json, Value};

    use crate::cron::Repeat;
    use crate::icons::path_for;
    use crate::recipes::Screen;
    use crate::state::{ConnState, Tab};
    use crate::testkit::{render, render_seeded};

    // ------------------------------------------------- pressing, not seeding

    /// A mounted view that can be PRESSED, and re-read afterwards.
    ///
    /// `testkit::render_seeded` renders once and drops the dom, which reaches
    /// every state the context holds. Four of this file's states are not in
    /// the context: `confirm_delete`, `confirm_run`, `sheet_open` and the
    /// sheet's own `open_row` are `use_signal`s a component keeps to itself,
    /// and the only thing that writes one is a press. Without this they would
    /// be four overlays no check ever sees — including both of the confirms
    /// that stand between a thumb and an agent running on someone's machine.
    ///
    /// It is here rather than in `testkit.rs` because it is one file's need so
    /// far; if a second screen wants it, that is when it moves.
    struct Mounted {
        dom: VirtualDom,
        /// Every mutation the dom has emitted since it was mounted, in order.
        /// A re-render describes only what CHANGED, so they accumulate — and
        /// every lookup below starts at the END, which is what makes the
        /// newest thing on screen the one a press finds.
        edits: Vec<Mutation>,
        /// Where in `edits` the most recent render starts. The two positional
        /// presses below are about what THAT render put down — the confirm
        /// that just opened, the value list just drilled into — rather than
        /// about the screen as a whole.
        latest: usize,
        /// A timer for the handlers to arm. Pressing Delete or Run with no
        /// server behind it ends in `show_toast`, which calls
        /// `tokio::time::sleep` to take the toast away again — and that panics
        /// with "there is no reactor running" the moment the dom polls it.
        /// The app never builds one of these; Dioxus's own executor is running
        /// inside a tokio runtime on a device.
        timer: tokio::runtime::Runtime,
    }

    impl Mounted {
        fn mount(seed: fn(&AppCtx), view: fn() -> Element) -> Self {
            // The listener `dioxus-html` installs converts the platform's own
            // event into a `MouseData` before the handler sees it, through a
            // process-global converter that panics when it is missing. Setting
            // it twice is setting it once.
            dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
            let _ = crate::testkit::storage_dir();
            MOUNT.set(Some((seed, view)));
            let timer = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("a current-thread tokio runtime");
            let mut dom = VirtualDom::new(Probe);
            let edits = dom.rebuild_to_vec().edits;
            Self {
                dom,
                edits,
                latest: 0,
                timer,
            }
        }

        fn html(&self) -> String {
            dioxus_ssr::render(&self.dom)
        }

        /// The control that owns `label`, whichever way the label reaches the
        /// screen.
        ///
        /// Dioxus hands a renderer an `ElementId` per element and a
        /// `NewEventListener` per handler, in the order it builds them, and it
        /// builds an element by writing its attributes, then its listeners,
        /// then filling in its dynamic children. So the two kinds of label sit
        /// on opposite sides of the listener that owns them:
        ///
        ///   - a **rendered attribute** — a row action's `title` — is written
        ///     just BEFORE its element's listener, so the owner is the first
        ///     listener after it;
        ///   - a **rendered text child** — `{row.value}`, `{choice.label}`,
        ///     a confirm's `{confirm_label}` — is created after it, so the
        ///     owner is the last listener before it.
        ///
        /// Only rendered labels are in this stream at all: a literal word
        /// lives in the compiled template and is never a mutation. That is why
        /// the FAB and Save are pressed by [`Mounted::press_last`] instead.
        fn control_for(&self, label: &str) -> ElementId {
            let is_click = |edit: &Mutation| match edit {
                Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                _ => None,
            };
            let Some(at) = self.edits.iter().rposition(|edit| match edit {
                Mutation::CreateTextNode { value, .. }
                | Mutation::SetAttribute {
                    value: AttributeValue::Text(value),
                    ..
                } => value == label,
                _ => false,
            }) else {
                panic!("nothing on screen says {label:?}, so there is nothing to press")
            };
            let owner = if matches!(self.edits[at], Mutation::SetAttribute { .. }) {
                self.edits[at..].iter().find_map(is_click)
            } else {
                self.edits[..at].iter().rev().find_map(is_click)
            };
            owner.unwrap_or_else(|| {
                panic!("{label:?} is on screen but nothing around it is pressable")
            })
        }

        fn press(&mut self, label: &str) {
            let id = self.control_for(label);
            self.press_id(id);
        }

        /// Press the LAST control the most recent render built.
        ///
        /// For the ones whose word is a literal rather than a rendered one, so
        /// [`Mounted::control_for`] cannot see them: a screen's Run and a
        /// sheet's Save are both the last thing their render puts down. A
        /// control appearing after them is a change worth failing on.
        fn press_last(&mut self) {
            let id = self.clicks().next_back();
            self.press_id(id.unwrap_or_else(|| panic!("that render built nothing pressable")));
        }

        /// Press the FIRST control the most recent render built: a screen's
        /// leading chevron, a sheet's backdrop, a confirm's Cancel.
        fn press_first(&mut self) {
            let id = self.clicks().next();
            self.press_id(id.unwrap_or_else(|| panic!("that render built nothing pressable")));
        }

        fn clicks(&self) -> impl DoubleEndedIterator<Item = ElementId> + '_ {
            self.edits[self.latest..]
                .iter()
                .filter_map(|edit| match edit {
                    Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                    _ => None,
                })
        }

        fn press_id(&mut self, id: ElementId) {
            let data: Rc<dyn Any> = Rc::new(PlatformEventData::new(Box::new(
                SerializedMouseData::default(),
            )));
            let _timer = self.timer.enter();
            self.dom
                .runtime()
                .handle_event("click", Event::new(data, true), id);
            self.latest = self.edits.len();
            self.edits.extend(self.dom.render_immediate_to_vec().edits);
        }
    }

    /// What to put in the context, and what to render over it.
    type Mount = (fn(&AppCtx), fn() -> Element);

    thread_local! {
        /// What [`Probe`] is about to mount. A thread-local rather than props
        /// because `cargo test` gives every test its own thread, and this way
        /// the component needs no props type of its own.
        static MOUNT: Cell<Option<Mount>> = const { Cell::new(None) };
    }

    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component, not like a fn"
    )]
    fn Probe() -> Element {
        let ctx = crate::state::use_app_ctx_provider();
        let (seed, view) = MOUNT.with(Cell::get).expect("nothing was mounted");
        use_hook(|| seed(&ctx));
        view()
    }

    // ------------------------------------------------------------- fixtures

    /// A list row as goose sends one. JSON rather than a struct literal so the
    /// fixture says what is on the wire, which is what these screens read.
    ///
    /// `last_modified` is deliberately years old: `relative_time` prints a
    /// duration for anything inside a week and a calendar date beyond it, and
    /// only the date is a value a test can pin.
    fn entry(recipe: &Value, schedule_cron: &Value) -> RecipeListEntry {
        serde_json::from_value(json!({
            "id": "9f2c41ab6d3e0517",
            "recipe": recipe,
            "file_path": "/home/me/.config/goose/recipes/standup.yaml",
            "last_modified": "2020-01-05T09:41:00+00:00",
            "schedule_cron": schedule_cron,
            "slash_command": null,
        }))
        .unwrap()
    }

    /// A recipe that pins nothing, takes nothing and runs on no timer. The
    /// description carries markdown so the screens have to prove which of them
    /// renders it and which prints it.
    fn plain() -> Value {
        json!({"title": "Morning standup", "description": "What happened **yesterday**."})
    }

    fn param(requirement: &str) -> Value {
        json!({
            "key": "since",
            "input_type": "string",
            "requirement": requirement,
            "description": "",
        })
    }

    /// The stroke data an [`Icon`] renders, which is the only thing in the
    /// markup that says *which* icon it is.
    fn icon(name: &str) -> &'static str {
        path_for(name).unwrap()
    }

    // ---------------------------------------------------------------- seeds

    fn connect(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connected {
            agent: "goose 1.10".to_owned(),
        });
    }

    fn unsupported_server(ctx: &AppCtx) {
        connect(ctx);
        let mut list = ctx.recipes.list;
        list.write().unsupported = true;
    }

    fn failed_fetch(ctx: &AppCtx) {
        connect(ctx);
        let mut list = ctx.recipes.list;
        list.write().sticky = Some("recipes/list timed out after 15s".to_owned());
    }

    fn first_fetch_in_flight(ctx: &AppCtx) {
        connect(ctx);
        let mut list = ctx.recipes.list;
        list.write().begin();
    }

    fn one_plain_row(ctx: &AppCtx) {
        connect(ctx);
        let mut list = ctx.recipes.list;
        list.write().settle(vec![entry(&plain(), &Value::Null)]);
    }

    fn one_busy_row(ctx: &AppCtx) {
        connect(ctx);
        let mut list = ctx.recipes.list;
        list.write().settle(vec![entry(
            &json!({
                "title": "Nightly green build",
                "description": "Runs the gate.",
                "parameters": [param("optional"), param("optional")],
            }),
            &json!("0 30 8 * * 1-5"),
        )]);
    }

    fn one_row_with_an_unreadable_date(ctx: &AppCtx) {
        connect(ctx);
        let mut row = entry(&plain(), &Value::Null);
        row.last_modified = "whenever".to_owned();
        let mut list = ctx.recipes.list;
        list.write().settle(vec![row]);
    }

    /// The desktop arrangement: a list beside a detail column that has one of
    /// its own rows open.
    fn a_row_and_its_detail_open(ctx: &AppCtx) {
        one_plain_row(ctx);
        let (mut tab, mut screen, mut open) = (ctx.tab, ctx.recipes.screen, ctx.recipes.open);
        tab.set(Tab::Recipes);
        screen.set(Screen::Detail);
        open.set(Some(entry(&plain(), &Value::Null)));
    }

    fn open_plain(ctx: &AppCtx) {
        connect(ctx);
        let mut row = entry(&plain(), &Value::Null);
        row.slash_command = Some("standup".to_owned());
        let mut open = ctx.recipes.open;
        open.set(Some(row));
    }

    fn open_and_flagged(ctx: &AppCtx) {
        open_plain(ctx);
        let mut scan = ctx.recipes.scan;
        scan.set(ScanState::Warned);
    }

    fn open_and_asking_for_values(ctx: &AppCtx) {
        connect(ctx);
        let mut open = ctx.recipes.open;
        open.set(Some(entry(
            &json!({
                "title": "Draft release notes",
                "description": "Notes since the last tag.",
                "parameters": [param("required")],
            }),
            &Value::Null,
        )));
    }

    fn open_with_everything_pinned(ctx: &AppCtx) {
        connect(ctx);
        let mut open = ctx.recipes.open;
        open.set(Some(entry(
            &json!({
                "title": "Nightly green build",
                "description": "Runs the gate.",
                "instructions": "Run `cargo clippy` and report.",
                "prompt": "Start with the workspace lint gate.",
                "settings": {"goose_provider": "anthropic", "goose_model": "claude-sonnet-4-5",
                             "max_turns": 12},
                "extensions": [{"type": "stdio", "name": "github"}],
            }),
            &json!("0 30 8 * * 1-5"),
        )));
    }

    fn open_on_a_scheduler_less_server(ctx: &AppCtx) {
        open_plain(ctx);
        let mut off = ctx.recipes.scheduler_off;
        off.set(true);
    }

    fn open_with_a_cron_this_phone_cannot_hold(ctx: &AppCtx) {
        connect(ctx);
        let mut open = ctx.recipes.open;
        open.set(Some(entry(&plain(), &json!("*/15 9-17 * * 1-5"))));
    }

    // ------------------------------------------------------------- the list

    /// A server built without the recipes methods is not a broken one, and
    /// rule 11 says the app must not offer a Retry for something retrying
    /// cannot fix. If this arm ever picked up `error-box` the screen would be
    /// tinted red and read as a fault the user could clear.
    #[test]
    fn a_server_without_recipes_is_stated_rather_than_blamed() {
        let html = render_seeded(unsupported_server, || rsx! { RecipesView {} });
        assert!(
            html.contains("This goose server does not have recipes."),
            "the unsupported arm did not name the server as the reason: {html}"
        );
        assert!(
            !html.contains("error-box"),
            "a server that never had recipes is drawn as a failure, so the \
             screen reads as something a retry could fix: {html}"
        );
    }

    /// Offline and empty draw the same blank screen and mean opposite things.
    /// The wrong sentence here tells someone their recipes are gone when the
    /// phone simply cannot see the server.
    #[test]
    fn an_unreachable_server_is_not_reported_as_an_empty_shelf() {
        let html = render(|| rsx! { RecipesView {} });
        assert!(
            html.contains("Not connected. Recipes live on your goose server."),
            "a disconnected list did not say so: {html}"
        );
        assert!(
            !html.contains("No recipes yet"),
            "the offline list claimed the server has no recipes on it"
        );
    }

    /// `Remote::fail` keeps a first-load failure on screen instead of toasting
    /// it away, and this is the only place that decision becomes visible: a
    /// swallowed `sticky` leaves a blank screen with no reason on it.
    #[test]
    fn a_failed_first_load_keeps_its_reason_on_the_screen() {
        let html = render_seeded(failed_fetch, || rsx! { RecipesView {} });
        assert!(
            html.contains("recipes/list timed out after 15s"),
            "the failure the fetch recorded never reached the screen: {html}"
        );
        assert!(
            html.contains("error-box"),
            "a real failure is drawn as an ordinary empty state, so nothing \
             marks it as wrong: {html}"
        );
    }

    /// The first fetch of a connected session and a genuinely empty shelf are
    /// the same markup a moment apart. Saying "no recipes yet" while the list
    /// is still in flight invites someone to go and make one.
    #[test]
    fn a_fetch_in_flight_does_not_announce_an_empty_shelf() {
        let html = render_seeded(first_fetch_in_flight, || rsx! { RecipesView {} });
        assert!(
            html.contains("Loading recipes…"),
            "a list still loading did not say so: {html}"
        );
        assert!(
            !html.contains("No recipes yet"),
            "the loading list already claimed the shelf is empty"
        );
        assert!(
            html.contains(r#"data-refreshing="true""#),
            "the scroller does not report that a fetch is in flight, so \
             pull-to-refresh has nothing to drive its spinner from: {html}"
        );
    }

    /// The empty state is the one place that says where a recipe comes from.
    /// This app cannot author one, so an empty screen with no instructions on
    /// it is a dead end.
    #[test]
    fn an_empty_shelf_says_where_recipes_come_from() {
        let html = render_seeded(connect, || rsx! { RecipesView {} });
        assert!(
            html.contains(
                "No recipes yet. Ask goose in a chat to save one, or add one from your desktop."
            ),
            "the empty list does not say how to get a recipe onto it: {html}"
        );
    }

    /// Everything a row is for: which recipe it is, what it is about, and how
    /// old the file is.
    #[test]
    fn a_row_names_its_recipe_and_dates_its_file() {
        let html = render_seeded(one_plain_row, || rsx! { RecipesView {} });
        assert!(
            html.contains("Morning standup"),
            "the row is missing the recipe's title: {html}"
        );
        assert!(
            html.contains("What happened **yesterday**."),
            "the row's second line is not the recipe's description: {html}"
        );
        assert!(
            html.contains(r#"<span class="session-age">Jan 5</span>"#),
            "the row does not date the recipe file, so a list of them cannot \
             be read newest-first: {html}"
        );
        assert!(
            html.contains(icon("book")),
            "the row's tile is not the recipes icon: {html}"
        );
    }

    /// Design rule 8: a dot is a *state*, and "not on a timer" is the absence
    /// of one. A row that always drew the dot would say every recipe is
    /// scheduled.
    #[test]
    fn only_a_scheduled_row_wears_the_dot() {
        let quiet = render_seeded(one_plain_row, || rsx! { RecipesView {} });
        assert!(
            !quiet.contains("recipe-meta"),
            "an unscheduled recipe that takes no inputs still drew a meta \
             line, so the row is claiming to have something to say: {quiet}"
        );

        let busy = render_seeded(one_busy_row, || rsx! { RecipesView {} });
        assert!(
            busy.contains(r#"<span class="dot on">"#),
            "a scheduled recipe lost the dot that is the only thing marking \
             it as running on a timer: {busy}"
        );
        assert!(
            busy.contains("Runs every weekday at 8:30 AM"),
            "the row printed no schedule sentence, so the cron either did not \
             reach it or reached it raw: {busy}"
        );
        assert!(
            busy.contains("2 inputs"),
            "the row does not say the recipe takes values: {busy}"
        );
    }

    /// goose reports `last_modified` as the file's mtime, and a server that
    /// hands back something unparseable must cost the row its age, not the
    /// whole row.
    #[test]
    fn an_unreadable_timestamp_costs_the_row_its_age_and_nothing_else() {
        let html = render_seeded(one_row_with_an_unreadable_date, || {
            rsx! { RecipesView {} }
        });
        assert!(
            html.contains("Morning standup"),
            "an unreadable date took the whole row with it: {html}"
        );
        assert!(
            !html.contains("session-age"),
            "the row printed an age it could not compute: {html}"
        );
    }

    /// The desktop shows the list beside the detail column, and the highlight
    /// is the only thing saying which row the pane is showing. It is keyed off
    /// the open recipe's id rather than off a click, so a row opened from
    /// anywhere still marks itself — and a list with nothing open beside it
    /// marks nothing.
    #[test]
    fn the_open_recipes_row_is_the_marked_one() {
        let marked = render_seeded(a_row_and_its_detail_open, || rsx! { RecipesView {} });
        assert!(
            marked.contains(r#"class="session-item on""#),
            "the row whose recipe the detail column has open is not marked, \
             so the two columns say nothing about each other: {marked}"
        );

        let unmarked = render_seeded(one_plain_row, || rsx! { RecipesView {} });
        assert!(
            !unmarked.contains("session-item on"),
            "a row painted itself as the open one with nothing open beside \
             it: {unmarked}"
        );
    }

    // ----------------------------------------------------------- the detail

    /// The open recipe and the Detail screen are set and cleared together, so
    /// this arm should be unreachable — but rendering nothing would be a
    /// screen with no way off it, which is worse than a sentence nobody reads.
    #[test]
    fn a_detail_with_no_recipe_still_has_a_way_back() {
        let html = render(|| rsx! { RecipeDetailView {} });
        assert!(
            html.contains("That recipe is no longer open."),
            "the empty detail says nothing at all: {html}"
        );
        assert!(
            html.contains(icon("chevron-left")),
            "the dead-end detail has no back control, so the screen is a trap: \
             {html}"
        );
        assert!(
            html.contains(">Recipe<"),
            "the bar names nothing, so the desktop window's own chrome would \
             name nothing either: {html}"
        );
    }

    /// The bar's two lines both come from [`crumb`], which the desktop
    /// window's chrome reads as well — so a recipe cannot be called one thing
    /// in the pane and another in the title bar.
    #[test]
    fn the_bar_names_the_recipe_and_the_slash_command_it_answers_to() {
        let html = render_seeded(open_plain, || rsx! { RecipeDetailView {} });
        assert!(
            html.contains(r#"<h1 class="title ellipsis">Morning standup</h1>"#),
            "the open recipe's title is not the bar's title: {html}"
        );
        assert!(
            html.contains(r#"<span class="subtitle ellipsis">/standup</span>"#),
            "the slash command is missing its slash or missing entirely, so \
             the subtitle does not name something you could type: {html}"
        );
    }

    /// Two taps is the whole flow and this is the second one. The `has-fab`
    /// class is the room the button sits in: without it the card ends
    /// underneath it.
    #[test]
    fn a_clean_recipe_offers_one_button_and_the_room_for_it() {
        let html = render_seeded(open_plain, || rsx! { RecipeDetailView {} });
        assert!(
            html.contains(r#"<button class="fab">"#),
            "the recipe screen has no Run button on it: {html}"
        );
        assert!(
            html.contains(icon("play")),
            "the Run button carries no play icon: {html}"
        );
        assert!(
            html.contains(r#"class="scroll has-fab""#),
            "the page did not reserve room for the button, so the card runs \
             under it: {html}"
        );
        assert!(
            !html.contains("hidden characters"),
            "an unflagged recipe is carrying the scan warning: {html}"
        );
    }

    /// The banner is the reason the button below asks twice, so it has to be
    /// on the page rather than in a toast that is gone by the time a thumb
    /// gets there.
    #[test]
    fn a_flagged_recipe_carries_its_warning_next_to_the_button() {
        let html = render_seeded(open_and_flagged, || rsx! { RecipeDetailView {} });
        assert!(
            html.contains("This recipe contains hidden characters. Read it before running it."),
            "goose's scan verdict never reached the screen: {html}"
        );
        assert!(
            html.contains("error-box"),
            "the warning is set as ordinary prose rather than as a warning: {html}"
        );
        assert!(
            html.contains(r#"<button class="fab">"#),
            "a flagged recipe lost its Run button entirely — it belongs behind \
             a confirm, not gone: {html}"
        );
    }

    /// The rule the whole feature turns on. This client answers `-32601` to
    /// the parameter callback, so a run it could start would hang; rule 11
    /// says the answer is no button rather than a disabled one, and the facts
    /// card is where the missing button gets explained.
    #[test]
    fn a_recipe_that_would_ask_for_values_has_no_button_and_no_room_for_one() {
        let html = render_seeded(open_and_asking_for_values, || {
            rsx! { RecipeDetailView {} }
        });
        assert!(
            !html.contains(r#"class="fab""#),
            "a recipe this client cannot launch is offering to launch it: {html}"
        );
        assert!(
            !html.contains("has-fab"),
            "the page reserved room for a button it does not draw: {html}"
        );
        assert!(
            html.contains("1 input · asked for at launch"),
            "the Inputs fact is missing, so the button is absent with no \
             explanation anywhere on the screen: {html}"
        );
        assert!(
            html.contains("has to be started from goose on your desktop"),
            "the note does not say where the recipe can be run instead: {html}"
        );
    }

    /// A recipe's three pieces of prose are three different things — what it
    /// is, what it tells goose, and what will be sitting in the composer — and
    /// only the last is labelled, because only it is about to become a message
    /// you send.
    #[test]
    fn the_detail_prints_the_recipe_in_its_own_words() {
        let html = render_seeded(open_with_everything_pinned, || {
            rsx! { RecipeDetailView {} }
        });
        assert!(
            html.contains("<p>Runs the gate.</p>"),
            "the description did not go through the markdown renderer: {html}"
        );
        assert!(
            html.contains("<code>cargo clippy</code>"),
            "the instructions are missing, or reached the page as raw \
             markdown: {html}"
        );
        assert!(
            html.contains("Opens with"),
            "the prompt is on the page with nothing saying it is what the \
             composer will hold: {html}"
        );
        assert!(
            html.contains("<p>Start with the workspace lint gate.</p>"),
            "the recipe's prompt never reached the screen: {html}"
        );
    }

    /// A recipe with no prompt of its own opens an empty composer, and
    /// labelling nothing would promise text that is not coming.
    #[test]
    fn a_recipe_with_no_prompt_carries_no_label_for_one() {
        let html = render_seeded(open_plain, || rsx! { RecipeDetailView {} });
        assert!(
            !html.contains("Opens with"),
            "a recipe with no prompt still labelled one: {html}"
        );
    }

    /// The card is what you read before pressing Run: which model the recipe
    /// pins, how long it may go on for, and how many extra tool servers it
    /// switches on. Every one is read-only here, so every one is a fact row
    /// rather than a control (rule 11).
    #[test]
    fn the_facts_card_states_what_a_run_would_bring_with_it() {
        let html = render_seeded(open_with_everything_pinned, || {
            rsx! { RecipeDetailView {} }
        });
        for fact in [
            "claude-sonnet-4-5",
            "anthropic · pinned by the recipe, not by the session.",
            "Max turns",
            "1 extension",
            "Tool servers the recipe switches on for its run.",
        ] {
            assert!(
                html.contains(fact),
                "the facts card is missing {fact:?}, so a run brings something \
                 with it that the screen never mentioned: {html}"
            );
        }
        assert!(
            html.contains(r#"class="setting-row fact""#),
            "the facts are drawn as controls, so they look pressable: {html}"
        );
    }

    /// The one thing on this screen that can be changed, and so the only row
    /// on the card that earns a chevron.
    #[test]
    fn the_schedule_row_is_the_one_control_on_the_card() {
        let html = render_seeded(open_plain, || rsx! { RecipeDetailView {} });
        assert!(
            html.contains(r#"<button class="setting-row">"#),
            "the Schedule row is not pressable, so nothing on this screen can \
             put a recipe on a timer: {html}"
        );
        assert!(
            html.contains("Not scheduled"),
            "a recipe on no timer does not say so: {html}"
        );
        assert_eq!(
            html.matches(icon("chevron-right")).count(),
            1,
            "exactly one row on this card drills in, and it is Schedule; a \
             second chevron means a fact is pretending to be a control: {html}"
        );
    }

    /// goose only says the scheduler is off when it is asked, so the first
    /// refusal is what teaches the app. After that, rule 11 does not let the
    /// row go on offering something that has already been refused.
    #[test]
    fn a_scheduler_less_server_turns_the_schedule_row_into_a_fact() {
        let html = render_seeded(open_on_a_scheduler_less_server, || {
            rsx! { RecipeDetailView {} }
        });
        assert!(
            html.contains("--enable-scheduler"),
            "the row does not say why it cannot be pressed: {html}"
        );
        assert!(
            !html.contains(icon("chevron-right")),
            "a server that answered \"scheduler not enabled\" is still being \
             offered the schedule sheet: {html}"
        );
    }

    /// A cron this grammar cannot hold is evidence, not state. Rewriting it
    /// into the nearest thing the sheet can express would lose a schedule the
    /// app never understood; showing it raw is the only way to recognise it on
    /// the machine that can edit it.
    #[test]
    fn a_cron_this_phone_cannot_edit_is_shown_as_itself_and_left_alone() {
        let html = render_seeded(open_with_a_cron_this_phone_cannot_hold, || {
            rsx! { RecipeDetailView {} }
        });
        assert!(
            html.contains("*/15 9-17 * * 1-5"),
            "the schedule set from another client is not on the screen, so \
             there is no way to recognise it elsewhere: {html}"
        );
        assert!(
            html.contains("change it there"),
            "the row does not say where the schedule can be changed: {html}"
        );
        assert!(
            !html.contains(icon("chevron-right")),
            "a cron this sheet cannot express is still offering the sheet, \
             which could only overwrite it: {html}"
        );
    }

    // ------------------------------------------------------------ the sheet

    fn weekly_sheet() -> Element {
        rsx! {
            CronSheet {
                title: "Nightly green build".to_owned(),
                current: Some(Schedule {
                    repeat: Repeat::Weekly,
                    weekday: 5,
                    day: 1,
                    hour: 18,
                    minute: 30,
                }),
                on_save: move |_| {},
                on_close: move |()| {},
            }
        }
    }

    fn unscheduled_sheet() -> Element {
        rsx! {
            CronSheet {
                title: "Morning standup".to_owned(),
                current: None,
                on_save: move |_| {},
                on_close: move |()| {},
            }
        }
    }

    /// The sheet's whole claim is that nobody has to imagine the cron their
    /// taps add up to: every row reads as words and the Result row says the
    /// sentence. A row that printed its wire value — `5`, `18`, `30` — would
    /// be the backend value design rule 8 keeps off the screen.
    #[test]
    fn the_sheet_reads_as_words_and_says_what_they_add_up_to() {
        let html = render(weekly_sheet);
        for words in ["Every week", "Friday", "6 PM", ":30"] {
            assert!(
                html.contains(words),
                "the sheet is missing {words:?}, so one of its rows is showing \
                 a cron field instead of a choice: {html}"
            );
        }
        assert!(
            html.contains("Runs every Friday at 6:30 PM"),
            "the Result row does not say what the choices mean, so the only \
             way to find out is to save and look: {html}"
        );
        assert!(
            html.contains("Nightly green build"),
            "the sheet does not name the recipe it is about to reschedule: {html}"
        );
    }

    /// Off is a value of the Repeat row rather than a switch of its own, and
    /// with it chosen the sheet asks nothing else — a Day and an Hour that
    /// could not affect the result are not rendered disabled, they are not
    /// rendered (rule 11).
    #[test]
    fn a_recipe_on_no_timer_is_asked_one_question_and_no_more() {
        let html = render(unscheduled_sheet);
        assert_eq!(
            html.matches(r#"<button class="setting-row">"#).count(),
            1,
            "the sheet asked more than the one question an unscheduled recipe \
             has an answer for: {html}"
        );
        assert!(
            html.contains("Never"),
            "the Repeat row does not read as Off, so nothing says the recipe \
             is on no timer: {html}"
        );
        assert!(
            html.contains("Not scheduled"),
            "the Result row invented a schedule for a recipe that has none: {html}"
        );
    }

    /// A backdrop that is not pressable makes the sheet modal in the worst
    /// sense, and Save has to be the primary of the pair — Cancel taking the
    /// filled button would make the default action the one that does nothing.
    #[test]
    fn the_sheet_can_be_left_by_both_of_its_exits() {
        let html = render(weekly_sheet);
        assert!(
            html.contains("modal-backdrop"),
            "the sheet has no backdrop to dismiss it with: {html}"
        );
        assert!(
            html.contains(r#"<button class="btn secondary">Cancel</button>"#),
            "the sheet has no Cancel: {html}"
        );
        assert!(
            html.contains(r#"<button class="btn primary">Save</button>"#),
            "Save is not the primary action of the sheet: {html}"
        );
    }

    /// Stated only where it is true: cron skips a month with no such day
    /// rather than clamping to its last one, so a schedule set for the 31st
    /// quietly misses February and six other months.
    #[test]
    fn a_day_most_months_do_not_have_is_flagged_on_the_row_that_picks_it() {
        let html = render(late_monthly_sheet);
        assert!(
            html.contains("31st"),
            "the day-of-month row is not reading as a date: {html}"
        );
        assert!(
            html.contains("Months without this day are skipped, February most of all."),
            "a schedule that will miss seven months a year says nothing about \
             it: {html}"
        );
    }

    fn late_monthly_sheet() -> Element {
        rsx! {
            CronSheet {
                title: "Invoice sweep".to_owned(),
                current: Some(Schedule {
                    repeat: Repeat::Monthly,
                    weekday: 1,
                    day: 31,
                    hour: 9,
                    minute: 0,
                }),
                on_save: move |_| {},
                on_close: move |()| {},
            }
        }
    }

    // ------------------------------------------------- what a press produces

    thread_local! {
        /// Every cron the sheet has handed back. A thread-local because a
        /// mounted view is a plain `fn` pointer and can capture nothing, and
        /// `cargo test` gives each test a thread of its own.
        static SAVED: std::cell::RefCell<Vec<Option<String>>> =
            const { std::cell::RefCell::new(Vec::new()) };
        /// How many times the sheet has asked to be closed.
        static CLOSED: Cell<u32> = const { Cell::new(0) };
    }

    fn recording_weekly_sheet() -> Element {
        rsx! {
            CronSheet {
                title: "Nightly green build".to_owned(),
                current: Some(Schedule {
                    repeat: Repeat::Weekly,
                    weekday: 5,
                    day: 1,
                    hour: 18,
                    minute: 30,
                }),
                on_save: move |cron: Option<String>| SAVED.with_borrow_mut(|log| log.push(cron)),
                on_close: move |()| CLOSED.with(|closed| closed.set(closed.get() + 1)),
            }
        }
    }

    fn job_sheet() -> Element {
        rsx! {
            CronSheet {
                title: "Nightly green build".to_owned(),
                current: Some(Schedule {
                    repeat: Repeat::Daily,
                    weekday: 1,
                    day: 1,
                    hour: 7,
                    minute: 0,
                }),
                allow_never: false,
                on_save: move |_| {},
                on_close: move |()| {},
            }
        }
    }

    /// The second level of the sheet's grammar, which nothing else in this
    /// file can reach: the row's values, with the one in force ticked. A list
    /// that ticked nothing would leave the reader unable to tell what the
    /// schedule currently is from the screen that changes it.
    #[test]
    fn drilling_into_a_row_shows_its_values_with_the_one_in_force_ticked() {
        let mut screen = Mounted::mount(|_| {}, unscheduled_sheet);
        screen.press("Never");
        let html = screen.html();

        assert!(
            html.contains("<h2>Repeat</h2>"),
            "the drilled list is not headed with the row it came from: {html}"
        );
        assert!(
            html.contains(icon("chevron-left")),
            "the drilled list has no way back to the sheet, so it is a dead \
             end with the recipe's schedule behind it: {html}"
        );
        for cadence in [
            "Never",
            "Every hour",
            "Every day",
            "Every weekday",
            "Every week",
            "Every month",
        ] {
            assert!(
                html.contains(cadence),
                "the Repeat list is missing {cadence:?}: {html}"
            );
        }
        assert!(
            html.contains(
                r#"<button class="choice selected"><span class="choice-name">Never</span>"#
            ),
            "nothing in the list is marked as the current cadence: {html}"
        );
        assert_eq!(
            html.matches(icon("check")).count(),
            1,
            "a value list ticks exactly one value, and this one ticked a \
             different number of them: {html}"
        );
    }

    /// Which rows exist follows from the repeat, and the only way to see that
    /// is to change it: an hourly schedule has no hour to pick, a monthly one
    /// has a date. The Result row is the live sentence, so it has to move with
    /// the choice rather than with the save.
    #[test]
    fn choosing_a_repeat_changes_which_questions_the_sheet_asks() {
        let mut screen = Mounted::mount(|_| {}, unscheduled_sheet);
        screen.press("Never");
        screen.press("Every month");
        let html = screen.html();

        assert!(
            html.contains("Day of month") && html.contains("Hour") && html.contains("Minute"),
            "turning a schedule on did not bring out the questions it opens: {html}"
        );
        assert!(
            html.contains("Runs on the 1st of every month at 9:00 AM"),
            "the Result row did not follow the choice, so the sentence on \
             screen describes a schedule that is no longer selected: {html}"
        );
        assert!(
            !html.contains("Not scheduled"),
            "the sheet still says the recipe is on no timer after a cadence \
             was chosen: {html}"
        );
    }

    /// Five taps to say "weekly, Monday, 9:30" is ONE schedule, not five round
    /// trips — so the sheet keeps a draft and calls back exactly once, with
    /// the cron the whole conversation adds up to.
    #[test]
    fn the_sheet_saves_once_with_the_cron_the_choices_add_up_to() {
        SAVED.with_borrow_mut(Vec::clear);
        let mut screen = Mounted::mount(|_| {}, recording_weekly_sheet);

        screen.press("Friday");
        screen.press("Monday");
        assert!(
            SAVED.with_borrow(Vec::is_empty),
            "the sheet wrote to the server on a tap, so setting a schedule is \
             a round trip per choice and half-made ones reach goose"
        );
        assert!(
            screen.html().contains("Runs every Monday at 6:30 PM"),
            "the chosen day did not reach the draft: {}",
            screen.html()
        );

        screen.press_last();
        assert_eq!(
            SAVED.with_borrow(Clone::clone),
            vec![Some("0 30 18 * * 1".to_owned())],
            "Save handed back something other than the one cron the sheet was \
             showing — six fields, seconds pinned to zero, Monday as 1"
        );
    }

    /// A tap outside the sheet is the exit everything else in this app has,
    /// and it is the one that must not write: dismissing a sheet is not
    /// agreeing to what was half-chosen in it.
    #[test]
    fn a_tap_outside_the_sheet_leaves_it_without_saving() {
        SAVED.with_borrow_mut(Vec::clear);
        CLOSED.with(|closed| closed.set(0));
        let mut screen = Mounted::mount(|_| {}, recording_weekly_sheet);

        screen.press_first();
        assert_eq!(
            CLOSED.with(Cell::get),
            1,
            "the backdrop is inert, so the sheet can only be left by one of \
             its own buttons"
        );
        assert!(
            SAVED.with_borrow(Vec::is_empty),
            "dismissing the sheet wrote a schedule to the server"
        );
    }

    /// Backing out of a value list is not choosing from it. The row it came
    /// from has to be exactly as it was, or a look at the choices costs you
    /// the one already made.
    #[test]
    fn backing_out_of_a_value_list_changes_nothing() {
        let mut screen = Mounted::mount(|_| {}, unscheduled_sheet);
        screen.press("Never");
        screen.press_first();
        let html = screen.html();

        assert!(
            html.contains(r#"<button class="setting-row">"#),
            "the back chevron did not return to the sheet's questions: {html}"
        );
        assert!(
            html.contains("Not scheduled"),
            "coming back from the value list left the recipe on a schedule it \
             was never given: {html}"
        );
    }

    /// `schedules/update` takes a required cron and there is no null to send,
    /// so "Never" on a job's sheet would be a control with no method behind it
    /// (rule 11). Taking a job off its timer is Delete, which is a different
    /// question with a confirm of its own.
    #[test]
    fn a_jobs_cadence_sheet_does_not_offer_a_cadence_it_cannot_send() {
        let mut screen = Mounted::mount(|_| {}, job_sheet);
        screen.press("Every day");
        let html = screen.html();

        assert!(
            !html.contains("Never"),
            "the cadence list offered to remove a schedule the update call \
             has no way to remove: {html}"
        );
        assert!(
            html.contains("Every hour") && html.contains("Every month"),
            "the cadence list lost more than its Never: {html}"
        );
    }

    /// The schedule sheet is the one overlay on the detail screen, and nothing
    /// in the context opens it — a press does. It has to arrive already
    /// carrying the recipe it will reschedule.
    #[test]
    fn pressing_the_schedule_row_opens_the_sheet_on_this_recipe() {
        let mut screen = Mounted::mount(open_plain, || rsx! { RecipeDetailView {} });
        assert!(
            !screen.html().contains("modal sheet"),
            "the sheet is on screen before anything asked for it"
        );

        screen.press("Not scheduled");
        let html = screen.html();
        assert!(
            html.contains("modal sheet"),
            "pressing the Schedule row opened nothing: {html}"
        );
        assert!(
            html.contains(r#"<p class="modal-session">Morning standup</p>"#),
            "the sheet does not name the recipe it is about to put on a \
             timer: {html}"
        );

        screen.press_last();
        assert!(
            !screen.html().contains("modal sheet"),
            "Save left the sheet up, so the schedule looks unsaved and the \
             next press sends it again: {}",
            screen.html()
        );
    }

    /// The same exit, from the sheet the detail owns rather than one mounted
    /// on its own: a tap outside puts the screen back with the recipe's
    /// schedule untouched.
    #[test]
    fn a_tap_outside_the_detail_sheet_leaves_the_schedule_alone() {
        let mut screen = Mounted::mount(open_plain, || rsx! { RecipeDetailView {} });
        screen.press("Not scheduled");
        assert!(
            screen.html().contains("modal sheet"),
            "the sheet never opened, so there is nothing to dismiss"
        );

        screen.press_first();
        let html = screen.html();
        assert!(
            !html.contains("modal sheet"),
            "a tap outside the sheet did not close it: {html}"
        );
        assert!(
            html.contains("Not scheduled"),
            "dismissing the sheet put the recipe on a timer anyway: {html}"
        );
    }

    /// The back chevron leaves the recipe as well as the screen: the open
    /// recipe is what the list marks a row from and what the desktop window's
    /// bar is named after, and a screen that went back without clearing it
    /// would leave both pointing at something nobody is looking at.
    #[test]
    fn the_back_chevron_closes_the_recipe_it_came_from() {
        let mut screen = Mounted::mount(open_plain, || rsx! { RecipeDetailView {} });
        assert!(
            screen.html().contains("Morning standup"),
            "the recipe was not open to begin with"
        );

        screen.press_first();
        assert!(
            screen.html().contains("That recipe is no longer open."),
            "the back control left the recipe open behind it: {}",
            screen.html()
        );
    }

    /// goose's scan is advice, not a gate — it runs a flagged recipe from any
    /// client — so the app's answer is a second question rather than a
    /// refusal. The words have to say what a run costs: this recipe gets the
    /// user's own tools on the user's own machine.
    #[test]
    fn a_flagged_recipe_asks_a_second_time_before_it_runs() {
        let mut screen = Mounted::mount(open_and_flagged, || rsx! { RecipeDetailView {} });
        screen.press_last();
        let html = screen.html();

        assert!(
            html.contains("Run this recipe anyway?"),
            "the Run button on a flagged recipe fired straight into a run: {html}"
        );
        assert!(
            html.contains("A recipe runs with your tools, on your machine."),
            "the confirm does not say what running it costs: {html}"
        );
        assert!(
            html.contains(">Run anyway<"),
            "the confirm's button says something other than what it does: {html}"
        );

        screen.press("Run anyway");
        let after = screen.html();
        assert!(
            !after.contains("Run this recipe anyway?"),
            "the confirm stayed up after it was answered, so the screen is \
             stuck behind a modal: {after}"
        );
        assert!(
            after.contains(r#"<button class="fab">"#),
            "the screen did not come back after the confirm: {after}"
        );
    }

    /// A confirm that cannot be declined is not a confirm. Cancel has to put
    /// the screen back exactly as it was, with the recipe unrun.
    #[test]
    fn the_run_confirm_can_be_declined() {
        let mut screen = Mounted::mount(open_and_flagged, || rsx! { RecipeDetailView {} });
        screen.press_last();
        assert!(
            screen.html().contains("Run this recipe anyway?"),
            "the confirm never opened, so there is nothing to decline"
        );

        screen.press_first();
        let html = screen.html();
        assert!(
            !html.contains("Run this recipe anyway?"),
            "Cancel left the question on screen: {html}"
        );
        assert!(
            html.contains(r#"<button class="fab">"#),
            "declining the run cost the screen its button: {html}"
        );
    }

    /// The other half of the same rule: a recipe goose found nothing in runs
    /// on the first press. A confirm on every run would train the tap that
    /// dismisses it.
    #[test]
    fn an_unflagged_recipe_runs_without_being_asked_twice() {
        let mut screen = Mounted::mount(open_plain, || rsx! { RecipeDetailView {} });
        screen.press_last();
        let html = screen.html();
        assert!(
            !html.contains("modal"),
            "an unflagged recipe put a question between the button and the \
             run: {html}"
        );
    }

    // ------------------------------------------- what a press does to a list

    fn a_list_the_nav_is_looking_at(ctx: &AppCtx) {
        one_plain_row(ctx);
        let mut tab = ctx.tab;
        tab.set(Tab::Recipes);
    }

    /// A tap on a row OPENS the recipe; it does not run it. Running one starts
    /// an agent with the user's tools on the user's machine, and a thumb
    /// brushing a list while scrolling must not be able to do that.
    ///
    /// Pressed by the row's own class rather than by its title, because the
    /// title is a child of the element that carries the tap and the class is
    /// on the element itself. There is one row in the fixture, so there is one
    /// of them.
    #[test]
    fn a_tap_on_a_row_opens_the_recipe_rather_than_running_it() {
        let mut screen = Mounted::mount(a_list_the_nav_is_looking_at, || rsx! { RecipesView {} });
        assert!(
            !screen.html().contains("session-item on"),
            "a row is already marked as open before anything was pressed"
        );

        screen.press("session-item");
        let html = screen.html();
        assert!(
            html.contains(r#"class="session-item on""#),
            "a tap on the row left nothing open, so either the row does not \
             open a recipe or it opened one the list cannot recognise: {html}"
        );
    }

    /// goose's delete unlinks the file and there is no undo, so a drag or a
    /// click on an always-visible icon is not consent on its own. The sentence
    /// has to say the file goes from the server, because the recipe is not on
    /// the phone in the first place.
    #[test]
    fn deleting_a_recipe_is_asked_before_it_is_done() {
        let mut screen = Mounted::mount(one_plain_row, || rsx! { RecipesView {} });
        assert!(
            !screen.html().contains("Delete this recipe?"),
            "the delete confirm is up before anything asked for it"
        );

        screen.press("Delete");
        let html = screen.html();
        assert!(
            html.contains("Delete this recipe?"),
            "the row's delete went straight to the server with no question \
             in between: {html}"
        );
        assert!(
            html.contains("The recipe file goes from the goose server. This cannot be undone."),
            "the confirm does not say what is about to be destroyed or where: {html}"
        );

        screen.press("Delete");
        let after = screen.html();
        assert!(
            !after.contains("Delete this recipe?"),
            "the confirm stayed up after it was answered: {after}"
        );
        assert!(
            after.contains("Morning standup"),
            "the row left the list before the server said the file had gone, \
             so a failed delete would show a recipe that is still there as \
             deleted: {after}"
        );
    }

    /// The other half: a delete confirm that cannot be backed out of is a
    /// destructive control with no way off it.
    #[test]
    fn the_delete_confirm_can_be_declined() {
        let mut screen = Mounted::mount(one_plain_row, || rsx! { RecipesView {} });
        screen.press("Delete");
        assert!(
            screen.html().contains("Delete this recipe?"),
            "the confirm never opened, so there is nothing to decline"
        );

        screen.press_first();
        let html = screen.html();
        assert!(
            !html.contains("Delete this recipe?"),
            "Cancel left the question on screen: {html}"
        );
        assert!(
            html.contains("Morning standup"),
            "declining the delete took the row anyway: {html}"
        );
    }
}
