//! The two Scheduler screens: what the server runs on a timer, and one job in
//! full.
//!
//! Components only: every decision this file renders — which sentence an empty
//! list gets, which buttons a job earns, whether its cadence is editable, what
//! the word beside the dot says — is a free function in `crate::scheduler`,
//! `crate::cron` or the client crate, taken over plain data and tested there.
//! What is left here is markup.
//!
//! The best thing on either of them is the run history, and it is the part
//! this file writes least of: `schedules/sessions/list` returns the same
//! `SessionInfo` the Chats list renders, so a run is a [`ListRow`] that calls
//! `state::open_session` and replays through the path that already exists.

use std::collections::HashSet;

use dioxus::prelude::*;
use goose_acp_client::{ScheduleState, ScheduledJob, SessionInfo};

use crate::icons::Icon;
use crate::nav::Crumb;
use crate::recipes::{list_state, ListState};
use crate::scheduler::{
    self, cadence, close, detail_actions, facts, last_run_label, open, row_state, state_label,
    title_for, watch_target, Cadence, Sheet,
};
use crate::shell::Shell;
use crate::state::{now_secs, relative_time, rfc3339_to_epoch, use_app_ctx, AppCtx, Remote, Tab};
use crate::views::chrome::{ListRow, RowAction, RowFace, TopBar};
use crate::views::recipes::{fact_row, CronSheet};
use crate::views::session_settings::SettingRow;
use crate::views::ConfirmDelete;

/// Which of the Scheduler's two screens a [`use_poll`] call is on.
///
/// It exists because the answer to "does this call site start a loop" is a
/// question about the SHELL, and the two screens are not interchangeable there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollSite {
    List,
    Detail,
}

/// Whether a call site claims the poll epoch and runs the loop.
///
/// On the phone both screens do, and that is the hand-over the epoch was
/// written for: one screen is mounted at a time, so the newer claim retires
/// the older loop exactly as the older screen leaves.
///
/// On the desktop the two are mounted TOGETHER — `src/shell/desktop.rs` puts
/// the list in one column and what it opened in the next — and hand-over
/// becomes a bug. The later claimant (the detail) retires the list's loop; then
/// closing the detail unmounts the only surviving loop, `use_future` does not
/// re-run for a component that never unmounted, and the Scheduler simply stops
/// polling until you leave the destination and come back. So on the desktop the
/// list owns the loop outright: it is the column that cannot be closed while the
/// destination is up, and `poll_once` already refreshes the open job's history,
/// so the detail loses nothing by not having a timer of its own.
///
/// A `const fn` over the shell rather than a `cfg`, following
/// `views::chrome::row_action_words`: `cargo test` runs the desktop arm on a
/// host, so a `cfg` here would leave the phone's answer asserted by nothing.
const fn claims_the_poll(site: PollSite, shell: Shell) -> bool {
    !matches!((shell, site), (Shell::Desktop, PollSite::Detail))
}

/// The poll, scoped to whichever screen owns it.
///
/// `use_future` rather than `spawn_forever` — the one documented exception in
/// this app — because the loop should die with the screen: a phone in a pocket
/// must not hold a timer for a list nobody is looking at. The generation epoch
/// behind it is the belt to that pair of braces, and on the phone it is also
/// what stops the two screens' loops from overlapping as one hands over to the
/// other.
///
/// Both screens call it, and [`claims_the_poll`] decides which of them starts a
/// loop. The detail's dot is the same fact as the row's, so on the phone — where
/// the list is not mounted behind it — the detail runs the loop itself rather
/// than showing a run that finished a minute ago.
fn use_poll(ctx: &AppCtx, site: PollSite) {
    let ctx = *ctx;
    use_future(move || async move {
        if !claims_the_poll(site, Shell::CURRENT) {
            return;
        }
        // Claimed inside the future, not in the component body: a signal
        // written during a render is a render that asks for another one.
        let mut generation = ctx.scheduler.poll;
        let mine = generation.peek().wrapping_add(1);
        generation.set(mine);

        loop {
            let interval = {
                let list = ctx.scheduler.list.peek();
                let started = ctx.scheduler.started_here.peek();
                scheduler::poll_interval(scheduler::any_running(&list.items, &started))
            };
            tokio::time::sleep(interval).await;
            // Every overlay over this list, not just this feature's own. The
            // reason written on `Sheet` — a list settling underneath re-renders
            // and reorders the rows behind it, which on the native renderer is
            // a visible reflow under something you are in the middle of
            // deciding — is about being covered, and the drawer (300px of a
            // 402pt screen, so the rows show past its edge) and the permission
            // modal cover this list exactly as a sheet does. `app.rs` renders
            // both over the view. The code plane's modal is not consulted
            // because `open_chat_has_ask` is false off the Code tab, and this
            // loop retires when the tab changes.
            let overlay = ctx.scheduler.sheet.peek().is_open()
                || *ctx.drawer_open.peek()
                || !ctx.permission.peek().is_empty();
            let tick = scheduler::poll_tick(
                mine,
                *ctx.scheduler.poll.peek(),
                *ctx.tab.peek(),
                ctx.client.peek().is_some(),
                ctx.scheduler.list.peek().loading,
                overlay,
            );
            match tick {
                scheduler::Tick::Retire => return,
                scheduler::Tick::Idle => continue,
                scheduler::Tick::Fetch => {}
            }
            scheduler::poll_once(&ctx).await;
        }
    });
}

/// The jobs the server is holding, whatever they are doing right now.
///
/// No FAB. Nothing here can create a schedule — `schedules/create` wants a
/// whole recipe body, which this app does not author — so the empty state
/// names the screen that can instead of offering a button that could not work.
#[component]
pub fn SchedulerView() -> Element {
    let ctx = use_app_ctx();
    let remote = (ctx.scheduler.list)();
    let connected = (ctx.conn)().is_connected();
    let state = list_state(&remote, connected);
    use_poll(&ctx, PollSite::List);

    // Reactive rather than a one-shot mount hook: the list is worth fetching
    // the moment there is a connection to fetch it over, including one that
    // arrives after this screen is already up.
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            scheduler::ensure_loaded(&ctx);
        }
    });

    let started = (ctx.scheduler.started_here)();
    let now = now_secs();

    rsx! {
        TopBar { title: "Scheduler", conn: true }
        main {
            class: "scroll",
            // On the phone: pull, not a button. The list refreshes itself on
            // a timer while you are looking at it; the gesture is for the
            // moment you do not want to wait for the next tick. The desktop
            // has the same timer — `claims_the_poll` above says which screen
            // holds it — and ⌘R for that moment instead of a gesture.
            "data-refresh": "scheduler",
            "data-refreshing": "{remote.loading}",

            match &state {
                ListState::Unsupported => rsx! {
                    // Not an error: no tint, no Retry, because retrying is not
                    // a thing that could work. The flag is on the machine
                    // running goose, which is where the sentence points.
                    p { class: "empty",
                        "Scheduling is off on this server. Restart goose with --enable-scheduler to use it."
                    }
                },
                ListState::Offline => rsx! {
                    // Deliberately not an empty state, which would say there
                    // are none — the app has no idea, and that is the point.
                    p { class: "error-box", "Not connected. Schedules live on your goose server." }
                },
                ListState::Failed(error) => rsx! {
                    p { class: "error-box", "{error}" }
                },
                ListState::Loading => rsx! {
                    p { class: "empty", "Loading schedules…" }
                },
                ListState::Empty => rsx! {
                    div { class: "empty job-empty",
                        p {
                            "Nothing is on a timer. A schedule starts life as a recipe: open one and set how often it should run."
                        }
                        button {
                            class: "btn secondary",
                            onclick: move |_| {
                                let mut tab = ctx.tab;
                                tab.set(Tab::Recipes);
                            },
                            Icon { name: "book" }
                            "Open Recipes"
                        }
                    }
                },
                ListState::Rows => rsx! {
                    ul { class: "session-list",
                        for job in remote.items.iter() {
                            {row(&ctx, job, &started, now)}
                        }
                    }
                },
            }
        }

        {overlays(&ctx)}
    }
}

/// One job as a row: what it is called, when it last ran, and what it is doing.
fn row(ctx: &AppCtx, job: &ScheduledJob, started: &HashSet<String>, now: i64) -> Element {
    let ctx = *ctx;
    let state = row_state(job, started);
    let label = state_label(job, state, now);
    let dot = format!("dot {}", state.dot());
    let (id, tray_id) = (job.id.clone(), job.id.clone());
    let paused = job.paused;
    let mut sheet = ctx.scheduler.sheet;
    // Which row the desktop's detail column came from. Ignored on the phone,
    // where the list is not on screen beside it (`views::chrome::row_is_marked`).
    let selected = ctx.scheduler.open.read().as_deref() == Some(job.id.as_str());

    rsx! {
        ListRow {
            key: "{job.id}",
            icon: "clock",
            title: title_for(&job.id),
            trailing: last_run_label(job),
            selected,
            // Pause before Delete: the tray is a scroller, so a short drag
            // reveals the first button and a full one reaches the last, and
            // the destructive action should be the deeper pull.
            actions: vec![
                RowAction::new(
                    if paused {
                        RowFace::plain("Resume", "play")
                    } else {
                        RowFace::plain("Pause", "pause")
                    },
                    EventHandler::new({
                        let id = tray_id.clone();
                        move |()| scheduler::set_paused(&ctx, &id, !paused)
                    }),
                ),
                RowAction::delete(EventHandler::new(move |()| {
                    sheet.set(Sheet::ConfirmDelete(tray_id.clone()));
                })),
            ],
            on_open: move |()| open(&ctx, &id),

            div { class: "session-meta job-meta",
                span {
                    span { class: "{dot}" }
                    " {label}"
                }
            }
        }
    }
}

/// What the open job is called, once.
///
/// Read by two things that are never on screen together: the header below,
/// and — on the desktop — the window's own bar, which takes the heading out of
/// the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop.rs`, `assets/desktop.css`).
///
/// The `None` arm is the view's own dead-end fallback and it is the one on
/// this table that is genuinely reachable: the poll replaces the list under
/// the screen, so a job deleted from another client disappears while its
/// detail is up.
pub(crate) fn crumb(ctx: &AppCtx) -> Crumb {
    let list = (ctx.scheduler.list)();
    let open_id = (ctx.scheduler.open)();
    let Some(job) = open_id
        .as_ref()
        .and_then(|id| list.items.iter().find(|job| &job.id == id))
    else {
        return Crumb::plain("Scheduled job");
    };
    let state = row_state(job, &(ctx.scheduler.started_here)());
    Crumb::detailed(
        title_for(&job.id),
        Some(state_label(job, state, now_secs())),
    )
}

/// One job, in full: what it runs, when it last ran, what it is doing about it
/// now, and every run it has ever produced.
#[component]
pub fn ScheduledJobView() -> Element {
    let ctx = use_app_ctx();
    use_poll(&ctx, PollSite::Detail);

    // The same reactive fetch `SchedulerView` has, and for a sharper version of
    // the same reason. `list_state` keeps a list that loaded before the socket
    // died tappable — deliberately — so this screen opens fine while offline,
    // and a fetch fired from `open` would have found no client and returned
    // with nothing marked, nothing failed and nothing ever firing again. The
    // history's only unprompted refetch is `poll_once`'s running→idle
    // transition, which a job that is not running never makes, so the screen
    // would have gone on saying "No runs yet" about a job with runs until
    // somebody happened to pull. Hanging the fetch on the connection instead
    // gives both cases one path: it does nothing while there is no client, and
    // runs the moment there is one.
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            if let Some(id) = ctx.scheduler.open.peek().clone() {
                scheduler::load_history(&ctx, &id);
            }
        }
    });

    let connected = (ctx.conn)().is_connected();
    let list = (ctx.scheduler.list)();
    let open_id = (ctx.scheduler.open)();
    let job = open_id
        .as_ref()
        .and_then(|id| list.items.iter().find(|job| &job.id == id).cloned());

    // The one expression the window's bar also reads, so a job cannot be
    // called one thing in the pane and another in the chrome. Both arms
    // hand `TopBar` a `String` equal to the one the expression they replace
    // produced, into the same prop of the same component — so the phone's
    // captured markup does not move.
    let bar = crumb(&ctx);

    let Some(job) = job else {
        // Reachable, unlike its siblings: the poll replaces the list under this
        // screen, so a job deleted from another client disappears while its
        // detail is up. A dead end would be the one failure this app has
        // already shipped once, so this keeps the way back.
        return rsx! {
            TopBar { title: bar.title, on_back: move |()| close(&ctx), conn: true }
            main { class: "scroll",
                p { class: "empty", "That schedule is no longer on the server." }
            }
        };
    };

    let started = (ctx.scheduler.started_here)();
    let now = now_secs();
    let state = row_state(&job, &started);
    let history = (ctx.scheduler.history)();
    let watching = watch_target(&history.items, job.current_session_id.as_deref());
    let id = job.id.clone();

    rsx! {
        TopBar {
            title: bar.title,
            subtitle: bar.subtitle,
            on_back: move |()| close(&ctx),
            conn: true,
        }
        main {
            class: "scroll",
            // One name for both scrollers: pulling here refreshes this job's
            // runs and the list behind it, which are the same fact.
            "data-refresh": "scheduler",
            "data-refreshing": "{list.loading || history.loading}",

            if matches!(state, ScheduleState::Paused) {
                // A banner and not an error box: a pause is something somebody
                // chose, not something that went wrong (design rule 7).
                div { class: "banner",
                    "Paused. It keeps its cadence and will not fire until you resume it."
                }
            }

            section { class: "card",
                div { class: "setting-list",
                    for row in facts(&job, now).iter() {
                        {fact_row(row)}
                    }
                    {cadence_row(&ctx, &job)}
                }
            }

            div { class: "btn-row",
                for action in detail_actions(state, watching.is_some()) {
                    button {
                        key: "{action.id}",
                        class: "{action.class}",
                        onclick: {
                            let (id, target) = (id.clone(), watching.clone());
                            move |_| act(&ctx, action.id, &id, target.clone())
                        },
                        Icon { name: action.icon }
                        "{action.label}"
                    }
                }
            }

            label { class: "field-label job-runs", "Recent runs" }
            {runs(&ctx, &history, connected)}
        }

        {overlays(&ctx)}
    }
}

/// What a detail button does. One place, so the ids `detail_actions` hands out
/// cannot drift from the calls behind them.
fn act(ctx: &AppCtx, action: &str, id: &str, watching: Option<SessionInfo>) {
    let mut sheet = ctx.scheduler.sheet;
    match action {
        "watch" => {
            if let Some(info) = watching {
                scheduler::watch(ctx, info);
            }
        }
        "run" => scheduler::run_now(ctx, id),
        "pause" => scheduler::set_paused(ctx, id, true),
        "resume" => scheduler::set_paused(ctx, id, false),
        "kill" => sheet.set(Sheet::ConfirmKill(id.to_owned())),
        _ => {}
    }
}

/// The Cadence row, as a control or as a fact.
fn cadence_row(ctx: &AppCtx, job: &ScheduledJob) -> Element {
    let ctx = *ctx;
    let mut sheet = ctx.scheduler.sheet;

    match cadence(job) {
        Cadence::Fixed(cron) => fact_row(&SettingRow::fact(
            "cadence",
            "Cadence",
            cron,
            "Set on another client, in a form this phone cannot edit — change it there.",
        )),
        Cadence::Editable(schedule) => rsx! {
            button {
                class: "setting-row",
                onclick: move |_| sheet.set(Sheet::Cadence),
                span { class: "setting-main",
                    span { class: "setting-name", "Cadence" }
                    span { class: "setting-value", "{crate::cron::describe(schedule)}" }
                }
                Icon { name: "chevron-right" }
            }
        },
    }
}

/// The job's runs: the Chats list, unchanged, over the sessions the scheduler
/// produced.
///
/// Through `list_state`, the same six-way answer every other list on this
/// screen and the four beside it give — because the empty-looking states are
/// where a run history can lie. "No runs yet" is a statement about the job, and
/// it is true only when the server was asked and said none: a `-32601` on
/// `schedules/sessions/list` (the cache is per *method*, so a goose with
/// `schedules/list` and without this one is a real server) and a socket that
/// died before the ask are both absences of an answer, not answers of
/// absence. Writing them as one `if history.items.is_empty()` is exactly how
/// this screen ended up asserting a job had never run.
fn runs(ctx: &AppCtx, history: &Remote<SessionInfo>, connected: bool) -> Element {
    let ctx = *ctx;
    match list_state(history, connected) {
        ListState::Unsupported => {
            return rsx! {
                // Not an error and no Retry, for `SchedulerView`'s reason: the
                // list works, so scheduling is on — this goose is simply older
                // than the method that lists a job's sessions.
                p { class: "hint",
                    "This goose does not report a job's runs. Its sessions are on the Chats screen."
                }
            };
        }
        ListState::Offline => {
            return rsx! {
                p { class: "error-box", "Not connected. This job's runs live on your goose server." }
            }
        }
        // A failure gets said rather than dressed as an absence: "No runs yet"
        // over a call that failed is a wrong statement about the job, not a
        // missing one.
        ListState::Failed(error) => return rsx! { p { class: "error-box", "{error}" } },
        ListState::Loading => return rsx! { p { class: "hint", "Loading this job's runs…" } },
        ListState::Empty => {
            return rsx! {
                p { class: "hint", "No runs yet. When it fires, the session it writes shows up here." }
            }
        }
        ListState::Rows => {}
    }

    rsx! {
        ul { class: "session-list",
            for info in history.items.iter() {
                ListRow {
                    key: "{info.session_id}",
                    icon: "clock",
                    title: info.display_title(),
                    trailing: info.updated_at.as_deref().and_then(rfc3339_to_epoch).map(relative_time),
                    // Nothing on this list writes: a run is a chat, and
                    // deleting one belongs where chats are deleted.
                    actions: vec![],
                    on_open: {
                        let info = info.clone();
                        move |()| scheduler::watch(&ctx, info.clone())
                    },

                    if let Some(count) = info.message_count() {
                        div { class: "session-meta",
                            span { "{count} msgs" }
                        }
                    }
                    if let Some(snippet) = info.last_message_snippet() {
                        div { class: "session-quote", "{snippet}" }
                    }
                }
            }
        }
    }
}

/// The three overlays, rendered at the view's root on both screens.
///
/// At the root and never inside the bar or a card: the bar's controls carry
/// `backdrop-filter`, and a filtered element becomes the containing block for
/// every `position: fixed` descendant.
fn overlays(ctx: &AppCtx) -> Element {
    let ctx = *ctx;
    let mut sheet = ctx.scheduler.sheet;
    let open = (ctx.scheduler.sheet)();
    let list = (ctx.scheduler.list)();

    match open {
        Sheet::Closed => rsx! {},
        Sheet::Cadence => {
            let job = (ctx.scheduler.open)()
                .and_then(|id| list.items.iter().find(|job| job.id == id).cloned());
            let Some(job) = job else {
                return rsx! {};
            };
            let Cadence::Editable(schedule) = cadence(&job) else {
                // Unreachable: a fixed cadence renders a fact, which is not a
                // button. Closing beats opening a sheet on a schedule this
                // grammar would rewrite.
                return rsx! {};
            };
            let id = job.id.clone();
            rsx! {
                CronSheet {
                    title: title_for(&job.id),
                    current: Some(schedule),
                    // Without "Never": a job that exists always has a cron, and
                    // there is no null to send. Stopping one is Delete.
                    allow_never: false,
                    on_close: move |()| sheet.set(Sheet::Closed),
                    // `None` is the sheet saying "Never", which `allow_never`
                    // has already taken off the list — so this arm is the
                    // impossible one, and it does nothing rather than guessing
                    // that a cadence somebody was editing meant Delete.
                    on_save: move |cron: Option<String>| {
                        sheet.set(Sheet::Closed);
                        if let Some(cron) = cron {
                            scheduler::set_cadence(&ctx, &id, &cron);
                        }
                    },
                }
            }
        }
        Sheet::ConfirmKill(id) => rsx! {
            ConfirmDelete {
                title: "Kill this run?",
                body: "goose stops the process. The schedule stays on, and it \
                       runs again at its next time.",
                // Not "Delete": killing a run stops it, it does not remove
                // anything — which is the case `confirm_label` exists for.
                confirm_label: "Kill",
                on_cancel: move |()| sheet.set(Sheet::Closed),
                on_confirm: move |()| {
                    sheet.set(Sheet::Closed);
                    scheduler::kill(&ctx, &id);
                },
            }
        },
        Sheet::ConfirmDelete(id) => rsx! {
            ConfirmDelete {
                title: "Delete this schedule?",
                body: "goose stops running it. The recipe it runs stays where \
                       it is, so you can schedule it again from Recipes.",
                on_cancel: move |()| sheet.set(Sheet::Closed),
                on_confirm: move |()| {
                    sheet.set(Sheet::Closed);
                    scheduler::delete(&ctx, &id);
                },
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{claims_the_poll, PollSite};
    use crate::scheduler::{dump_key, Screen};
    use crate::shell::Shell;

    /// Exactly one loop, whatever is mounted.
    ///
    /// On the phone one screen is up at a time, so both claim and the epoch
    /// hands the loop over. On the desktop both are mounted at once and the
    /// hand-over would be a silent stop: the detail's claim retires the list's
    /// loop, and closing the detail then unmounts the only one left — after
    /// which nothing polls the Scheduler until you leave the destination and
    /// come back, because `use_future` does not re-run for a component that
    /// never unmounted.
    #[test]
    fn the_desktop_detail_does_not_take_the_poll_off_the_list() {
        assert!(claims_the_poll(PollSite::List, Shell::Desktop));
        assert!(
            !claims_the_poll(PollSite::Detail, Shell::Desktop),
            "the detail is mounted BESIDE the list on the desktop, so a second \
             claim retires the list's loop and closing the detail leaves none"
        );
    }

    /// The phone's arm is unchanged, and this is the assertion that says so:
    /// `cargo test` runs the desktop arm on a host, so a `cfg` in
    /// `claims_the_poll` would leave this claim checked by nothing at all.
    #[test]
    fn the_phone_still_polls_from_whichever_screen_is_up() {
        assert!(claims_the_poll(PollSite::List, Shell::Mobile));
        assert!(claims_the_poll(PollSite::Detail, Shell::Mobile));
    }

    /// The dump keys this destination files its states under. Stated here as
    /// well as in `crate::scheduler` because this is the file that has to be
    /// re-captured when one of them changes, and `scripts/capture-gallery.py`
    /// carries a label for each.
    #[test]
    fn the_two_screens_are_the_two_keys_the_gallery_expects() {
        assert_eq!(dump_key(Screen::List), "scheduler");
        assert_eq!(dump_key(Screen::Detail), "scheduler-detail");
    }
}
