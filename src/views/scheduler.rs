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
/// On the desktop the two are mounted TOGETHER — `src/shell/desktop/mod.rs` puts
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
/// (`src/shell/desktop/mod.rs`, `assets/desktop.css`).
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
    //! The markup half of this file is asserted by rendering it, not by
    //! reading it: `crate::testkit` puts a real `AppCtx` under a view and
    //! hands back the HTML, so every "which sentence does this arm draw"
    //! below is answered by the same `rsx!` the phone runs.
    //! `src/selfscan.rs` is why that distinction is worth a paragraph.
    //!
    //! `Shell::CURRENT` is `Desktop` in a host `cargo test`, which shows up
    //! in two places and only two: a row's tray buttons carry their word as
    //! a `title` attribute rather than as text
    //! (`views::chrome::row_action_words`), and a row can be marked as the
    //! one the detail column has open. Assertions below stay on the word
    //! itself wherever the shell would otherwise decide where it lives.

    use dioxus::prelude::*;
    use goose_acp_client::{ScheduledJob, SessionInfo};
    use serde_json::{json, Map};

    use super::{act, claims_the_poll, crumb, PollSite};
    use crate::scheduler::{dump_key, Screen, Sheet};
    use crate::shell::Shell;
    use crate::state::{AppCtx, ConnState, Remote, Tab};
    use crate::testkit::{render, render_seeded, render_settled};

    const JOB: &str = "nightly-dependency-audit";
    const OTHER: &str = "weekly-changelog-digest";
    /// Every day at 3am, in the six-field form goose stores.
    const DAILY_3AM: &str = "0 0 3 * * *";
    /// What `DAILY_3AM` reads as. Spelled out rather than computed, so a
    /// change to `cron::describe` shows up here as a failure rather than as
    /// two sides agreeing with each other.
    const DAILY_3AM_WORDS: &str = "Runs every day at 3:00 AM";
    /// A cron `crate::cron` deliberately cannot hold — a stepped minute — so
    /// the cadence is a fact rather than a control.
    const UNREADABLE: &str = "*/15 * * * *";

    fn a_job(id: &str, cron: &str) -> ScheduledJob {
        ScheduledJob {
            id: id.to_owned(),
            source: format!("/home/demo/.config/goose/scheduled-recipes/{id}.yaml"),
            cron: cron.to_owned(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            job_start_time: None,
            extra: Map::new(),
        }
    }

    fn a_run(session_id: &str, cwd: Option<&str>) -> SessionInfo {
        SessionInfo {
            session_id: session_id.to_owned(),
            cwd: cwd.map(ToOwned::to_owned),
            title: Some("Dependency audit".to_owned()),
            // Old enough that `relative_time` answers with a date, so the
            // assertion does not age.
            updated_at: Some("2024-03-14T09:00:00Z".to_owned()),
            meta: Some(json!({
                "messageCount": 12,
                "lastMessageSnippet": "4 crates behind",
            })),
        }
    }

    fn settled<T>(items: Vec<T>) -> Remote<T> {
        Remote {
            items,
            loading: false,
            unsupported: false,
            sticky: None,
        }
    }

    /// A socket up and the Scheduler destination on screen — the state every
    /// arm below except the offline ones is about.
    fn go_online(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connected {
            agent: "goose 1.9".to_owned(),
        });
        let mut tab = ctx.tab;
        tab.set(Tab::Scheduler);
    }

    fn hold(ctx: &AppCtx, jobs: Vec<ScheduledJob>) {
        let mut list = ctx.scheduler.list;
        list.set(settled(jobs));
    }

    fn opened(ctx: &AppCtx, id: &str) {
        let mut open = ctx.scheduler.open;
        open.set(Some(id.to_owned()));
        let mut screen = ctx.scheduler.screen;
        screen.set(Screen::Detail);
    }

    fn list_view() -> Element {
        rsx! {
            super::SchedulerView {}
        }
    }

    fn detail_view() -> Element {
        rsx! {
            super::ScheduledJobView {}
        }
    }

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

    // ------------------------------------------------- the list's six arms
    //
    // Three of them draw a screen with nothing on it, and they are three
    // different facts: "this goose cannot schedule", "this phone cannot see
    // it" and "there is nothing on a timer". Collapsing any pair into the
    // other is a sentence that is simply untrue, which is the failure this
    // whole `match` exists to prevent.

    /// A goose started without `--enable-scheduler` is not a broken one. The
    /// sentence has to name the flag, because the fix is on the machine
    /// running goose and no amount of tapping on the phone reaches it — and
    /// it must not be dressed as an error, or it invites a retry that could
    /// never work.
    #[test]
    fn a_server_with_scheduling_off_is_told_which_flag_turns_it_on() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut list = ctx.scheduler.list;
                list.set(Remote {
                    unsupported: true,
                    ..settled(Vec::new())
                });
            },
            list_view,
        );
        assert!(
            html.contains("--enable-scheduler"),
            "the unsupported arm no longer names the flag, so a reader is \
             told scheduling does not work and not what to do about it: {html}"
        );
        assert!(
            !html.contains("error-box"),
            "a goose built without the scheduler is being painted as a \
             failure, which offers a retry that cannot succeed: {html}"
        );
    }

    /// Offline is deliberately not the empty state. The app has no idea
    /// whether anything is on a timer, and "Nothing is on a timer" would be
    /// the app stating something it cannot know.
    #[test]
    fn a_disconnected_phone_says_it_cannot_see_rather_than_that_there_is_nothing() {
        let html = render(list_view);
        assert!(
            html.contains("Not connected. Schedules live on your goose server."),
            "a cold launch is not showing the offline sentence: {html}"
        );
        assert!(
            !html.contains("Nothing is on a timer"),
            "an unreachable server is being reported as a server with no \
             schedules, which is a claim this app cannot make: {html}"
        );
    }

    /// A failed `schedules/list` gets said out loud. The alternative — an
    /// empty list — is the same screen a working server with no jobs draws,
    /// and there would be nothing on it to explain the difference.
    #[test]
    fn a_failed_fetch_shows_the_servers_own_words() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut list = ctx.scheduler.list;
                list.set(Remote {
                    sticky: Some("schedules/list timed out".to_owned()),
                    ..settled(Vec::new())
                });
            },
            list_view,
        );
        assert!(
            html.contains("schedules/list timed out"),
            "the failure the server reported is not on screen, so the list \
             reads as empty when it is actually broken: {html}"
        );
        assert!(
            html.contains("error-box"),
            "the failure is being shown as ordinary copy rather than as a \
             failure: {html}"
        );
    }

    /// The gap between asking and being answered is its own sentence.
    /// Without it the first frame of every visit claims the server has
    /// nothing scheduled.
    #[test]
    fn a_fetch_in_flight_says_so_instead_of_claiming_emptiness() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut list = ctx.scheduler.list;
                list.set(Remote {
                    loading: true,
                    ..settled(Vec::new())
                });
            },
            list_view,
        );
        assert!(
            html.contains("Loading schedules…"),
            "a fetch in flight is drawing something other than the loading \
             sentence: {html}"
        );
        assert!(
            html.contains(r#"data-refreshing="true""#),
            "the scroller is not advertising that it is refreshing, so the \
             pull gesture's spinner never arms: {html}"
        );
    }

    /// Nothing here can create a schedule — `schedules/create` wants a whole
    /// recipe body this app does not author — so the empty state has to point
    /// at the screen that can. A bare "no schedules" would be a dead end.
    #[test]
    fn an_empty_scheduler_sends_the_reader_to_recipes() {
        let html = render_seeded(go_online, list_view);
        assert!(
            html.contains("Nothing is on a timer."),
            "the empty state is not the empty state: {html}"
        );
        assert!(
            html.contains("Open Recipes"),
            "the empty Scheduler no longer offers the way to make a \
             schedule, which leaves the screen a dead end: {html}"
        );
    }

    /// The row is the whole list. Each of these four is a separate decision
    /// taken in `crate::scheduler` and rendered here, and a row that lost any
    /// of them still looks like a row.
    #[test]
    fn a_row_carries_the_jobs_name_its_age_its_state_and_its_actions() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
            },
            list_view,
        );
        assert!(
            html.contains("Nightly dependency audit"),
            "the row is printing the raw job id instead of a name, so the \
             list reads as a directory listing: {html}"
        );
        assert!(
            html.contains(r#"<span class="session-age">never</span>"#),
            "a job that has never run is not saying so: {html}"
        );
        assert!(
            html.contains(&format!(
                r#"<span class="dot on"></span> {DAILY_3AM_WORDS}"#
            )),
            "the row's dot and its cadence have come apart: {html}"
        );
        assert!(
            html.contains("Pause") && html.contains("Delete"),
            "the row's tray has lost an action: {html}"
        );
    }

    /// Pause and Resume are the same button wearing the job's own state. Get
    /// this backwards and a tap on a paused job pauses it again — the request
    /// succeeds, and nothing on screen changes.
    #[test]
    fn a_paused_job_offers_resume_and_not_pause() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut job = a_job(JOB, DAILY_3AM);
                job.paused = true;
                hold(ctx, vec![job]);
            },
            list_view,
        );
        assert!(
            html.contains("Resume"),
            "a paused job is not offering the way to un-pause it: {html}"
        );
        assert!(
            !html.contains("Pause"),
            "a paused job is offering Pause, which sends the request it is \
             already the result of: {html}"
        );
        assert!(
            html.contains(r#"<span class="dot off"></span> paused"#),
            "a paused job is not wearing the paused dot: {html}"
        );
    }

    /// The advisory flag is the whole reason a tap on Run now feels like
    /// anything: the server does not report the run for up to thirty seconds,
    /// and this device already knows. Promotion only — a job the server calls
    /// paused shows as running while a run this phone started is in flight.
    #[test]
    fn a_run_this_device_started_shows_busy_before_the_server_agrees() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut job = a_job(JOB, DAILY_3AM);
                job.paused = true;
                hold(ctx, vec![job]);
                let mut started = ctx.scheduler.started_here;
                started.set(std::iter::once(JOB.to_owned()).collect());
            },
            list_view,
        );
        assert!(
            html.contains(r#"<span class="dot busy"></span> running"#),
            "a run this device just started is not showing as running, so \
             pressing Run now looks like it did nothing for up to thirty \
             seconds: {html}"
        );
        assert!(
            html.contains("Resume"),
            "the tray has followed the advisory flag instead of the server's \
             own paused flag, which is the one Resume acts on: {html}"
        );
    }

    /// Which row the detail column came from. Only on the desktop, and only
    /// while something is actually open: the id outlives the screen that set
    /// it, so without the second half a row comes back from a detail still
    /// highlighted beside a pane that says nothing is open.
    #[test]
    fn exactly_the_open_row_is_marked_and_only_while_one_is_open() {
        let marked = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM), a_job(OTHER, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            list_view,
        );
        let count = marked.matches(r#"class="session-item on""#).count();
        assert_eq!(
            count, 1,
            "the desktop list marks {count} of its two rows as the one the \
             detail column is showing, and there is exactly one detail \
             column: {marked}"
        );

        let at_root = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM), a_job(OTHER, DAILY_3AM)]);
                let mut open = ctx.scheduler.open;
                open.set(Some(JOB.to_owned()));
            },
            list_view,
        );
        assert!(
            !at_root.contains(r#"class="session-item on""#),
            "a row is painted as the one being shown while the detail column \
             is closed, so the two columns say opposite things: {at_root}"
        );
    }

    // --------------------------------------------------------- the overlays

    /// With nothing open the view must render no overlay at all. A backdrop
    /// left in the tree covers the list with an invisible sheet.
    #[test]
    fn no_sheet_means_no_backdrop() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
            },
            list_view,
        );
        assert!(
            !html.contains("modal-backdrop"),
            "an overlay is being rendered over a list nobody opened one on: \
             {html}"
        );
    }

    /// Delete is for good, and the confirm has to say what survives it — the
    /// recipe stays, so this is undoable by scheduling it again rather than by
    /// an undo that does not exist.
    #[test]
    fn deleting_a_schedule_says_the_recipe_outlives_it() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                let mut sheet = ctx.scheduler.sheet;
                sheet.set(Sheet::ConfirmDelete(JOB.to_owned()));
            },
            list_view,
        );
        assert!(
            html.contains("Delete this schedule?"),
            "the delete confirmation is not on screen, so a swipe deletes \
             with no confirm at all: {html}"
        );
        assert!(
            html.contains("so you can schedule it again from Recipes"),
            "the confirm no longer says what survives the delete: {html}"
        );
    }

    /// Killing a run is not deleting a schedule, and the button is the last
    /// chance to say so. The same modal wearing "Delete" would read as the
    /// job being removed — which is the neighbouring sheet, on the same
    /// screen.
    #[test]
    fn killing_a_run_is_not_worded_as_a_deletion() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut job = a_job(JOB, DAILY_3AM);
                job.currently_running = true;
                hold(ctx, vec![job]);
                opened(ctx, JOB);
                let mut sheet = ctx.scheduler.sheet;
                sheet.set(Sheet::ConfirmKill(JOB.to_owned()));
            },
            detail_view,
        );
        assert!(
            html.contains("Kill this run?") && html.contains(">Kill</button>"),
            "the kill confirmation is missing or unlabelled: {html}"
        );
        assert!(
            html.contains("The schedule stays on"),
            "the kill confirm no longer says the schedule survives, which is \
             the whole difference from the sheet beside it: {html}"
        );
        assert!(
            !html.contains("Delete"),
            "stopping a run is being offered with the word for removing the \
             job: {html}"
        );
    }

    /// The cadence sheet opens on the cadence the job already has. Dropping
    /// `current` would open it on the default schedule, so tapping Save
    /// without touching anything would silently move the job.
    #[test]
    fn the_cadence_sheet_opens_on_the_cadence_the_job_already_has() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
                let mut sheet = ctx.scheduler.sheet;
                sheet.set(Sheet::Cadence);
            },
            detail_view,
        );
        assert!(
            html.contains(r#"<p class="modal-session">Nightly dependency audit</p>"#),
            "the cadence sheet does not name the job it is about: {html}"
        );
        assert!(
            html.contains(&format!(
                r#"<span class="setting-name">Result</span><span class="setting-value">{DAILY_3AM_WORDS}</span>"#
            )),
            "the sheet opened on a schedule that is not this job's, so Save \
             without a change would move it: {html}"
        );
    }

    /// The poll replaces the list under the open detail, so a job deleted from
    /// another client can vanish with its cadence sheet up. The sheet has
    /// nothing to edit then, and guessing would rewrite a cron nobody chose.
    ///
    /// Rendered through the LIST, not the detail: the detail's own
    /// job-is-gone arm returns before it reaches `overlays`, so a detail here
    /// would draw no sheet for a reason that has nothing to do with this one.
    #[test]
    fn the_cadence_sheet_closes_itself_when_the_job_it_edits_is_gone() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, Vec::new());
                opened(ctx, JOB);
                let mut sheet = ctx.scheduler.sheet;
                sheet.set(Sheet::Cadence);
            },
            list_view,
        );
        assert!(
            !html.contains("modal-backdrop"),
            "a cadence sheet is still up over a job that is no longer on the \
             server, and Save would write a cron to an id that is gone: \
             {html}"
        );
    }

    /// A cron this app's grammar cannot hold is shown and left alone. Opening
    /// the sheet on one could only rewrite it into something else, so the
    /// sheet declines rather than offering choices that would replace it.
    #[test]
    fn the_cadence_sheet_refuses_a_cron_this_app_would_have_to_rewrite() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, UNREADABLE)]);
                opened(ctx, JOB);
                let mut sheet = ctx.scheduler.sheet;
                sheet.set(Sheet::Cadence);
            },
            detail_view,
        );
        assert!(
            !html.contains("modal-backdrop"),
            "the sheet opened on a cron it cannot express, so saving would \
             replace a schedule set elsewhere with this app's nearest guess: \
             {html}"
        );
    }

    // ------------------------------------------------------ the job's detail

    /// The one dead end this app has already shipped once. The poll replaces
    /// the list under this screen, so a job deleted from another client
    /// disappears while its detail is up — and the back chevron has to
    /// survive it.
    #[test]
    fn a_job_that_vanished_still_leaves_the_way_back() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                opened(ctx, JOB);
            },
            detail_view,
        );
        assert!(
            html.contains("That schedule is no longer on the server."),
            "the detail of a job that has gone says nothing about why it is \
             blank: {html}"
        );
        assert!(
            html.contains(r#"<h1 class="title ellipsis">Scheduled job</h1>"#),
            "the fallback header lost its name, so the window bar and the \
             pane disagree about what is open: {html}"
        );
        assert!(
            html.contains(r#"class="icon-btn back""#),
            "the back chevron is gone from a screen with nothing on it, \
             which is the dead end this arm exists to prevent: {html}"
        );
    }

    /// What the detail is made of: the recipe behind the job, when it last
    /// ran, its cadence as a control, and the two buttons a scheduled job
    /// earns. Each is a separate decision, and a screen missing any one of
    /// them still renders.
    #[test]
    fn a_scheduled_jobs_detail_names_its_recipe_its_age_and_its_cadence() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            detail_view,
        );
        assert!(
            html.contains(r#"<span class="setting-value">nightly-dependency-audit</span>"#),
            "the Recipe row is not naming the recipe file this job runs: \
             {html}"
        );
        assert!(
            html.contains(
                "/home/demo/.config/goose/scheduled-recipes/nightly-dependency-audit.yaml"
            ),
            "the Recipe row dropped the path, which is the only way to find \
             the file on the server: {html}"
        );
        assert!(
            html.contains(&format!(
                r#"<span class="setting-name">Cadence</span><span class="setting-value">{DAILY_3AM_WORDS}</span>"#
            )),
            "the Cadence row is not saying what the cadence is: {html}"
        );
        assert!(
            html.contains("Run now") && html.contains("Pause"),
            "a scheduled job has lost one of the two buttons it earns: {html}"
        );
        assert!(
            !html.contains("Kill"),
            "Kill is offered on a job with nothing running, where it can only \
             fail: {html}"
        );
    }

    /// A pause is something somebody chose, so it gets a banner rather than
    /// an error box (design rule 7) — and it has to say what a pause does and
    /// does not do, because the cadence is still there.
    #[test]
    fn a_paused_job_explains_the_pause_and_offers_the_way_out() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut job = a_job(JOB, DAILY_3AM);
                job.paused = true;
                hold(ctx, vec![job]);
                opened(ctx, JOB);
            },
            detail_view,
        );
        assert!(
            html.contains(r#"<div class="banner">"#) && html.contains("will not fire until you"),
            "the paused banner is gone, so the only sign a job is paused is \
             the word beside a dot: {html}"
        );
        assert!(
            !html.contains("error-box"),
            "a pause somebody chose is being drawn as a failure: {html}"
        );
        assert!(
            html.contains("Resume") && !html.contains(">Pause</button>"),
            "the paused detail is offering Pause instead of Resume: {html}"
        );
    }

    /// While a run is in flight, watching it is the thing to do — and it needs
    /// a whole `SessionInfo`, which only the history has. The Kill button and
    /// the Started fact belong to the same moment.
    #[test]
    fn a_running_job_offers_the_transcript_the_kill_and_how_long_it_has_been() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut job = a_job(JOB, DAILY_3AM);
                job.currently_running = true;
                job.current_session_id = Some("sess-42".to_owned());
                hold(ctx, vec![job]);
                opened(ctx, JOB);
                let mut history = ctx.scheduler.history;
                history.set(settled(vec![a_run("sess-42", Some("/repo"))]));
            },
            detail_view,
        );
        assert!(
            html.contains(r#"<button class="btn primary">"#) && html.contains("Watch it run"),
            "a run in flight is not offering its transcript as the primary \
             action, which is the best thing on this screen: {html}"
        );
        assert!(
            html.contains("Kill"),
            "a run in flight cannot be stopped from its own screen: {html}"
        );
        assert!(
            html.contains(r#"<span class="setting-name">Started</span>"#),
            "the Started fact is missing, so nothing says how long the run \
             has been going: {html}"
        );
    }

    /// "Watch it run" opens a transcript, and `session/load` needs the
    /// session's `cwd`. A job whose current session is not in the history —
    /// or is there without a working directory — has no answer, and a button
    /// that opens the wrong directory is worse than no button.
    #[test]
    fn watch_is_not_offered_without_a_session_it_could_actually_open() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                let mut job = a_job(JOB, DAILY_3AM);
                job.currently_running = true;
                job.current_session_id = Some("sess-42".to_owned());
                hold(ctx, vec![job]);
                opened(ctx, JOB);
                let mut history = ctx.scheduler.history;
                history.set(settled(vec![a_run("sess-42", None)]));
            },
            detail_view,
        );
        assert!(
            !html.contains("Watch it run"),
            "the transcript is offered for a session with no working \
             directory, so the tap opens a chat against the wrong one: {html}"
        );
        assert!(
            html.contains(r#"<button class="btn primary">"#) && html.contains("Run now"),
            "with Watch gone, Run now should have taken the one primary this \
             screen gets: {html}"
        );
    }

    /// A cadence this app's grammar cannot hold is evidence, not state: it is
    /// the only way to recognise the schedule on the machine that can edit it.
    /// So it renders as a fact that says where to change it, and not as a
    /// control that would rewrite it.
    #[test]
    fn a_cron_this_app_cannot_express_is_shown_as_itself_and_not_as_a_control() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, UNREADABLE)]);
                opened(ctx, JOB);
            },
            detail_view,
        );
        assert!(
            html.contains(&format!(
                r#"<span class="setting-name">Cadence</span><span class="setting-value">{UNREADABLE}</span>"#
            )),
            "the raw cron is not on screen, so there is no way to recognise \
             this schedule on the client that set it: {html}"
        );
        assert!(
            html.contains("in a form this phone cannot edit"),
            "nothing says why the cadence is not editable here: {html}"
        );
        assert!(
            !html.contains(r#"<span class="setting-name">Cadence</span></span>"#),
            "the unreadable cadence is still a button, and tapping it can \
             only replace a schedule this app did not write: {html}"
        );
    }

    // ------------------------------------------------------- the run history
    //
    // `runs` is the six-way answer again, and this is where an empty-looking
    // screen lies loudest: "No runs yet" is a statement about the JOB, and
    // it is true only when the server was asked and said none.

    /// A goose old enough to schedule but not to list a job's sessions is a
    /// real server, not a failure — the method cache is per-method. It gets a
    /// hint pointing at the screen that does have the sessions.
    #[test]
    fn a_goose_without_the_sessions_method_is_pointed_at_chats() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
                let mut history = ctx.scheduler.history;
                history.set(Remote {
                    unsupported: true,
                    ..settled(Vec::new())
                });
            },
            detail_view,
        );
        assert!(
            html.contains("Its sessions are on the Chats screen."),
            "a goose that cannot list a job's runs is not saying where the \
             runs are instead: {html}"
        );
        assert!(
            !html.contains("No runs yet"),
            "a method this server does not have is being reported as a job \
             that has never run: {html}"
        );
    }

    /// A socket that died before the ask is an absence of an answer, not an
    /// answer of absence.
    #[test]
    fn an_offline_phone_does_not_claim_the_job_has_never_run() {
        let html = render_seeded(
            |ctx| {
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            detail_view,
        );
        assert!(
            // Matched without the apostrophe, which `dioxus-ssr` writes as
            // `&#39;`: the sentence is what is being asserted, not the entity.
            html.contains("runs live on your goose server."),
            "the offline history is not saying why it is empty: {html}"
        );
        assert!(
            !html.contains("No runs yet"),
            "an unreachable server is being reported as a job that has never \
             run — the exact false statement this arm exists to stop: {html}"
        );
    }

    /// A call that failed gets said. Dressed as an absence it becomes a wrong
    /// statement about the job rather than a missing one.
    #[test]
    fn a_failed_history_fetch_is_reported_rather_than_shown_as_no_runs() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
                let mut history = ctx.scheduler.history;
                history.set(Remote {
                    sticky: Some("sessions/list refused the cursor".to_owned()),
                    ..settled(Vec::new())
                });
            },
            detail_view,
        );
        assert!(
            html.contains("sessions/list refused the cursor"),
            "the history's failure is not on screen: {html}"
        );
        assert!(
            !html.contains("No runs yet"),
            "a failed fetch is being reported as a job that has never run: \
             {html}"
        );
    }

    /// `open` arms the history slot before the screen paints, precisely so
    /// this frame says "asking" rather than "none". Losing the loading arm
    /// puts "No runs yet" on screen for the length of every fetch.
    #[test]
    fn a_history_still_loading_says_so() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
                let mut history = ctx.scheduler.history;
                history.set(Remote {
                    loading: true,
                    ..settled(Vec::new())
                });
            },
            detail_view,
        );
        assert!(
            // The apostrophe is `&#39;` in the rendered markup; the sentence
            // is what matters here, not the entity.
            html.contains("Loading this job"),
            "a history fetch in flight is drawing something else: {html}"
        );
        assert!(
            !html.contains("No runs yet"),
            "the screen claims the job has never run while it is still \
             finding out: {html}"
        );
    }

    /// The one arm entitled to say it: asked, and answered with none.
    #[test]
    fn a_job_the_server_says_has_never_run_is_the_only_one_told_so() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            detail_view,
        );
        assert!(
            html.contains("No runs yet. When it fires, the session it writes shows up here."),
            "the one honest empty state is missing: {html}"
        );
    }

    /// A run is a chat, so it is the Chats row unchanged — down to the
    /// message count and the snippet. The one difference is that it carries
    /// no actions: deleting a session belongs where sessions are deleted.
    #[test]
    fn a_run_renders_as_the_chat_row_it_is_and_offers_no_delete() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
                let mut history = ctx.scheduler.history;
                history.set(settled(vec![a_run("sess-42", Some("/repo"))]));
            },
            detail_view,
        );
        assert!(
            html.contains(r#"<div class="session-title">Dependency audit</div>"#),
            "a run is not showing the session's own title: {html}"
        );
        assert!(
            html.contains(r#"<span class="session-age">Mar 14</span>"#),
            "a run is not showing when it happened: {html}"
        );
        assert!(
            html.contains("12 msgs") && html.contains("4 crates behind"),
            "the run row lost the count and the snippet that make it worth \
             scanning: {html}"
        );
        assert!(
            !html.contains("session-actions"),
            "a run row grew a swipe tray, which would delete a chat from the \
             one screen that is not the chats list: {html}"
        );
    }

    // ------------------------------------------------- the name, and the acts

    /// The window bar and the pane read the same expression, so a job cannot
    /// be called one thing in one and something else in the other. The
    /// subtitle is the row's own state label — the same fact the dot carries.
    #[test]
    fn the_crumb_names_the_open_job_and_what_it_is_doing() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            CrumbProbe,
        );
        assert!(
            html.contains("<p>Nightly dependency audit</p>"),
            "the crumb is not naming the open job, so the desktop window bar \
             says something other than the pane: {html}"
        );
        assert!(
            html.contains(&format!("<p>{DAILY_3AM_WORDS}</p>")),
            "the crumb lost the state line, so the bar names the job without \
             saying what it is doing: {html}"
        );
    }

    /// The reachable fallback: the poll replaces the list under the screen, so
    /// the crumb has to answer for a job that is no longer there rather than
    /// leaving the window bar holding the previous job's name.
    #[test]
    fn the_crumb_still_answers_for_a_job_that_is_gone() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                opened(ctx, JOB);
            },
            CrumbProbe,
        );
        assert!(
            html.contains("<p>Scheduled job</p><p>—</p>"),
            "a job deleted from another client leaves the crumb with no name \
             or a stale one: {html}"
        );
    }

    /// [`crumb`] renders nothing of its own, so the only way to see it is to
    /// run it inside a scope that has an `AppCtx` and print what it returned.
    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component"
    )]
    fn CrumbProbe() -> Element {
        let ctx = crate::state::use_app_ctx();
        let crumb = crumb(&ctx);
        let subtitle = crumb.subtitle.unwrap_or_else(|| "—".to_owned());
        rsx! {
            p { "{crumb.title}" }
            p { "{subtitle}" }
        }
    }

    /// The ids `detail_actions` hands out and the calls behind them are two
    /// lists that have to agree, and nothing but this checks that they do: a
    /// button whose id has no arm renders, highlights and does nothing at all.
    #[test]
    fn every_detail_button_reaches_the_call_it_is_labelled_with() {
        let html = render_seeded(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
            },
            ActProbe,
        );
        assert!(
            html.contains("<p>watch: home/chat/sess-42</p>"),
            "\"Watch it run\" no longer opens the run's transcript: {html}"
        );
        assert!(
            html.contains("<p>watch with nothing to watch: scheduler</p>"),
            "\"Watch it run\" fired without a session to open, which lands \
             on a chat built from nothing: {html}"
        );
        assert!(
            html.contains("<p>kill: kill nightly-dependency-audit</p>"),
            "Kill no longer opens its confirmation, so either nothing \
             happens or a run is stopped without being asked about: {html}"
        );
        for left in [
            "closed",
            "cadence",
            "delete nightly-dependency-audit",
            "kill weekly-changelog-digest",
        ] {
            assert!(
                html.contains(&format!("<p>an id from nowhere leaves: {left}</p>")),
                "an id this table has no arm for changed what was on screen \
                 — `{left}` should have survived it untouched: {html}"
            );
        }
        assert!(
            html.contains("<p>run: Not connected — reconnect in Settings</p>"),
            "Run now is not reaching `run_now`, which is the one detail \
             button whose failure is silent: {html}"
        );
    }

    /// Which sheet is open, as a word a probe can print. `Sheet` carries the
    /// id each confirm is about, and that is half of what is being asserted:
    /// a confirm has to survive a re-list that moved the row it came from.
    fn sheet_word(sheet: &Sheet) -> String {
        match sheet {
            Sheet::Closed => "closed".to_owned(),
            Sheet::Cadence => "cadence".to_owned(),
            Sheet::ConfirmKill(id) => format!("kill {id}"),
            Sheet::ConfirmDelete(id) => format!("delete {id}"),
        }
    }

    /// Where the app ended up: the chat it opened, or that it did not move.
    fn place_word(ctx: &AppCtx) -> String {
        if (ctx.tab)() == Tab::Home && (ctx.screen)() == crate::state::Screen::Chat {
            let chat = ctx.chat.peek();
            return format!(
                "home/chat/{}",
                chat.session_id.as_deref().unwrap_or("nothing")
            );
        }
        "scheduler".to_owned()
    }

    /// [`act`] writes to the context and returns nothing, so each arm is run
    /// here and what it changed is rendered as a line to assert on.
    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component"
    )]
    fn ActProbe() -> Element {
        let ctx = crate::state::use_app_ctx();
        let lines = use_hook(move || {
            let mut lines = Vec::new();

            act(&ctx, "watch", JOB, Some(a_run("sess-42", Some("/repo"))));
            lines.push(format!("watch: {}", place_word(&ctx)));

            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
            let mut screen = ctx.screen;
            screen.set(crate::state::Screen::Sessions);
            act(&ctx, "watch", JOB, None);
            lines.push(format!("watch with nothing to watch: {}", place_word(&ctx)));

            act(&ctx, "kill", JOB, None);
            lines.push(format!("kill: {}", sheet_word(&ctx.scheduler.sheet.peek())));

            // An id the table has no arm for must leave the screen exactly as
            // it found it — whatever was open, still open, untouched.
            for state in [
                Sheet::Closed,
                Sheet::Cadence,
                Sheet::ConfirmDelete(JOB.to_owned()),
                Sheet::ConfirmKill(OTHER.to_owned()),
            ] {
                let mut sheet = ctx.scheduler.sheet;
                sheet.set(state);
                act(&ctx, "definitely-not-an-action", JOB, None);
                lines.push(format!(
                    "an id from nowhere leaves: {}",
                    sheet_word(&ctx.scheduler.sheet.peek())
                ));
            }
            let mut sheet = ctx.scheduler.sheet;
            sheet.set(Sheet::Closed);

            // No client, which is what makes `run_now`'s own report the
            // observable half: everything else it does is behind an await.
            act(&ctx, "run", JOB, None);
            let toast = ctx.toast.peek().clone();
            lines.push(format!(
                "run: {}",
                toast.unwrap_or_else(|| "said nothing".to_owned())
            ));

            lines
        });
        rsx! {
            for line in lines.iter() {
                p { "{line}" }
            }
        }
    }

    // ----------------------------------------- what happens after the render
    //
    // `render` sees one pass. Everything below needs the scope's queued work
    // to run, which is what `render_settled` is for — and what it can reach
    // is what a task does before its first await.

    /// The claim itself, through the real components rather than through the
    /// `const fn` behind them.
    ///
    /// [`claims_the_poll`] is asserted twice above as plain data, and neither
    /// assertion would notice the call site moving, losing its argument, or
    /// being deleted: the list would simply stop polling, and the Scheduler
    /// would go on showing whatever it had when you arrived — no error, no
    /// warning, and a run history that never changes.
    #[test]
    fn the_list_takes_the_poll_epoch_and_the_desktops_detail_does_not() {
        let list = render_settled(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
            },
            PollProbe,
        );
        assert!(
            list.contains("<p>poll=1</p>"),
            "the Scheduler list mounted without claiming the poll epoch, so \
             nothing is polling: the dots and the run history are frozen at \
             whatever they were when the screen opened: {list}"
        );

        let detail = render_settled(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            DetailPollProbe,
        );
        assert!(
            detail.contains("<p>poll=0</p>"),
            "the detail claimed the epoch on a shell that mounts it BESIDE \
             the list, which retires the list's loop — and closing the detail \
             then leaves no loop at all: {detail}"
        );
    }

    /// The detail's fetch hangs off the connection rather than off the tap
    /// that opened the screen, so that a job opened while offline still gets
    /// its runs the moment the socket comes back. Claiming the slot is the
    /// first thing that fetch does, and it is the half that is observable
    /// without a server.
    #[test]
    fn a_connection_is_what_makes_the_open_job_fetch_its_runs() {
        let online = render_settled(
            |ctx| {
                go_online(ctx);
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            DetailPollProbe,
        );
        assert!(
            online.contains("<p>history-of=nightly-dependency-audit</p>"),
            "opening a job over a live connection never asked for its runs, \
             so the screen says \"No runs yet\" about a job with runs until \
             somebody happens to pull: {online}"
        );

        let offline = render_settled(
            |ctx| {
                hold(ctx, vec![a_job(JOB, DAILY_3AM)]);
                opened(ctx, JOB);
            },
            DetailPollProbe,
        );
        assert!(
            offline.contains("<p>history-of=nothing</p>"),
            "a fetch was dispatched with no client to make it on, which \
             claims the history slot for a request that can never answer: \
             {offline}"
        );
    }

    /// The list, with the poll epoch it claimed printed beside it.
    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component"
    )]
    fn PollProbe() -> Element {
        let ctx = crate::state::use_app_ctx();
        // Read, not peeked: the claim happens in a task after this render, so
        // the probe has to be subscribed to see the second one.
        let generation = (ctx.scheduler.poll)();
        rsx! {
            super::SchedulerView {}
            p { "poll={generation}" }
        }
    }

    /// The detail, with the epoch and the history slot it claimed.
    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component"
    )]
    fn DetailPollProbe() -> Element {
        let ctx = crate::state::use_app_ctx();
        let generation = (ctx.scheduler.poll)();
        let claimed = (ctx.scheduler.history_of)().unwrap_or_else(|| "nothing".to_owned());
        rsx! {
            super::ScheduledJobView {}
            p { "poll={generation}" }
            p { "history-of={claimed}" }
        }
    }
}
