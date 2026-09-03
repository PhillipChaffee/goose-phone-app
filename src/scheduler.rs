//! The Scheduler: what the server runs on a timer, and what a phone does
//! about one that is misbehaving.
//!
//! State and the calls that change it. The rendering is
//! `src/views/scheduler.rs`, the split `code.rs`/`views/code.rs` established:
//! everything here is testable without a Dioxus runtime, and nothing here
//! writes markup.
//!
//! **Recipes is where a schedule is born; Scheduler is where it is watched.**
//! Nothing here creates a job — `schedules/create` needs a whole recipe body,
//! which this app does not author — so the empty state points at Recipes
//! rather than offering a button that could not work.
//!
//! # The two hard parts
//!
//! **Polling.** There is no push. Nothing announces a job starting, finishing
//! or being killed (the over-the-wire test asserts exactly that), so the only
//! way this screen can be true is to ask. It asks every [`POLL_BUSY`] while
//! anything is running and every [`POLL_IDLE`] otherwise, only while the
//! screen is mounted *and* connected, and never while an overlay is open. The
//! loop lives in `use_future` in the view — the one place in this app where
//! dying with the screen is correct — with a generation epoch behind it, so a
//! phone in a pocket holds no timer.
//!
//! **`run-now` blocks.** The server does not answer until the run is over,
//! which is a whole agent turn. So nothing waits on it: the toast and the busy
//! dot happen before the await, [`Ctx::started_here`] carries the dot until
//! the request resolves, and the poll is what makes it true. If the socket
//! dies the job keeps running on the server and the next list finds it —
//! **the poll is authoritative and the local flag is advisory.**

use std::collections::HashSet;
use std::time::Duration;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{
    AcpError, RunNowResponse, RunStatus, ScheduleState, ScheduledJob, SessionInfo,
};

use crate::cron::{self, Schedule};
use crate::extensions::humanize;
use crate::state::{
    load_remote, relative_time, rfc3339_to_epoch, show_toast, AppCtx, Remote, Screen as HomeScreen,
    Tab,
};
use crate::views::session_settings::SettingRow;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    List,
    Detail,
}

/// The overlays this feature can put over either screen.
///
/// On the shared context rather than in a view's `use_signal` for one reason:
/// the poll has to be able to ask. A list settling under an open sheet
/// re-renders and reorders the rows behind it, which on the native renderer is
/// a visible reflow under something you are in the middle of deciding.
///
/// Each destructive variant carries the id it is about, so the confirm survives
/// a re-list that moved the row it was opened from.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Sheet {
    Closed,
    /// The cadence sheet over the open job's detail.
    Cadence,
    ConfirmKill(String),
    ConfirmDelete(String),
}

impl Sheet {
    pub(crate) const fn is_open(&self) -> bool {
        !matches!(self, Self::Closed)
    }
}

/// This feature's whole state, held as one field of [`AppCtx`].
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub screen: Signal<Screen>,
    pub list: Signal<Remote<ScheduledJob>>,
    /// The job the detail screen is showing, **by id**.
    ///
    /// Not a clone of the row, unlike Skills and Recipes. This is the one
    /// screen with a live poll under it: the list is replaced every five to
    /// thirty seconds, and a clone taken at open time would go on showing a
    /// busy dot for a run that finished while the reader was looking at it.
    pub open: Signal<Option<String>>,
    /// The open job's run history.
    pub history: Signal<Remote<SessionInfo>>,
    /// Whose history [`Ctx::history`] holds, so a fast back-and-forth cannot
    /// show job A's runs under job B's title.
    pub history_of: Signal<Option<String>>,
    /// Jobs this device fired `run-now` on whose request has not resolved.
    ///
    /// The advisory half of "poll authoritative, local flag advisory": it
    /// flips the dot to busy the instant the button is pressed, and its life is
    /// exactly the life of that request — including the socket dying, which
    /// resolves it as `Closed`. A set rather than a slot because a run lasts
    /// minutes and two can overlap.
    pub started_here: Signal<HashSet<String>>,
    pub sheet: Signal<Sheet>,
    /// Generation of the poll loop, claimed on mount. One loop at a time, and
    /// the newest wins.
    pub poll: Signal<u64>,
}

pub(crate) fn use_ctx() -> Ctx {
    Ctx {
        screen: use_signal(|| Screen::List),
        list: use_signal(Remote::new),
        open: use_signal(|| None),
        history: use_signal(Remote::new),
        history_of: use_signal(|| None),
        started_here: use_signal(HashSet::new),
        sheet: use_signal(|| Sheet::Closed),
        poll: use_signal(|| 0),
    }
}

/// The dump key for each of this destination's screens.
///
/// A free function over the plain enum, so the mapping can be tested without a
/// Dioxus runtime — the same arrangement `skills::dump_key` has, kept here so
/// that adding this feature is one line in `nav.rs` rather than three.
pub(crate) const fn dump_key(screen: Screen) -> &'static str {
    match screen {
        Screen::List => "scheduler",
        Screen::Detail => "scheduler-detail",
    }
}

/// How many runs the history asks for. `limit` is required on the wire, so
/// there is no "all of them" to ask for and a number has to be chosen.
pub(crate) const RUN_HISTORY: u32 = 20;

/// The busy cadence: something is running, and the dot is the reason this
/// screen exists.
pub(crate) const POLL_BUSY: Duration = Duration::from_secs(5);
/// The idle cadence. Nothing is moving; the next thing to happen is a cron
/// firing, which is minutes away at best.
pub(crate) const POLL_IDLE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tick {
    /// This loop is over: a newer one took its place, or the screen it belongs
    /// to is gone.
    Retire,
    /// Nothing to ask for from here — sleep again.
    Idle,
    Fetch,
}

/// What a poll tick does when it wakes up.
///
/// Spelled out as a function so every input is visible in one signature and
/// the rules are testable without a Dioxus runtime — `code::poll_tick`'s
/// arrangement, for `code::poll_tick`'s reason.
///
/// Leaving the tab is [`Tick::Retire`] rather than [`Tick::Idle`], which is
/// the difference between this loop and the Code tab's: both scheduler screens
/// are unmounted the moment the tab changes, so there is nothing for a parked
/// loop to come back to — and a phone in a pocket must not hold a timer.
/// Re-entering the tab mounts a screen, which starts a fresh loop.
///
/// `unsupported` is deliberately **not** an input. A cached `-32601` costs no
/// socket round trip at all — `goose_request` short-circuits it — and the
/// whole point of keeping the tick alive is that a reconnect mints a fresh
/// client with an empty cache, so a user who restarts goose with
/// `--enable-scheduler` sees the screen fill in rather than having to leave
/// and come back.
///
/// `loading` is an input because the pull gesture fetches the same list this
/// does, and the list timeout is 30 s against a 5 s busy cadence — so over a
/// slow tailnet a gesture's fetch would have six ticks piled on top of it. The
/// loop cannot stack on *itself*: it awaits its own fetch before sleeping
/// again, which is what makes one flag enough.
pub(crate) const fn poll_tick(
    mine: u64,
    current: u64,
    tab: Tab,
    connected: bool,
    loading: bool,
    overlay: bool,
) -> Tick {
    if current != mine || !matches!(tab, Tab::Scheduler) {
        Tick::Retire
    } else if !connected || loading || overlay {
        Tick::Idle
    } else {
        Tick::Fetch
    }
}

pub(crate) const fn poll_interval(any_running: bool) -> Duration {
    if any_running {
        POLL_BUSY
    } else {
        POLL_IDLE
    }
}

/// Whether anything is running, server-side or as far as this device knows.
///
/// The local half is what tightens the cadence the instant Run now is pressed,
/// rather than up to thirty seconds later — which is exactly the window in
/// which somebody is watching to see whether their tap did anything.
pub(crate) fn any_running(jobs: &[ScheduledJob], started_here: &HashSet<String>) -> bool {
    jobs.iter()
        .any(|job| job.currently_running || started_here.contains(&job.id))
}

/// The state a row draws: the server's, promoted by this device's advisory
/// flag.
///
/// Promotion only. The flag can say "running" sooner than the server does and
/// never later, so the poll always wins in the end — which is what keeps a
/// request that died on a dead socket from pinning a dot on forever.
pub(crate) fn row_state(job: &ScheduledJob, started_here: &HashSet<String>) -> ScheduleState {
    if started_here.contains(&job.id) {
        ScheduleState::Running
    } else {
        job.state()
    }
}

/// The word beside the dot: what this job is doing, in one phrase.
pub(crate) fn state_label(job: &ScheduledJob, state: ScheduleState, now: i64) -> String {
    match state {
        ScheduleState::Running => running_for(job.job_start_time.as_deref(), now),
        ScheduleState::Paused => state.word().to_owned(),
        ScheduleState::Scheduled => cron::summary(&job.cron),
    }
}

/// How long the run in flight has been going.
///
/// Deliberately not `state::relative_time`, which answers "now" under a minute
/// — "running now" reads as a status that has not started — and falls back to
/// a date, which a run in flight can never want.
///
/// A missing start time is the honest common case rather than an error: goose
/// omits `jobStartTime` on a job it has only just marked running, and on one
/// this device started a second ago it has not been asked yet.
pub(crate) fn running_for(started: Option<&str>, now: i64) -> String {
    let Some(epoch) = started.and_then(rfc3339_to_epoch) else {
        return "running".to_owned();
    };
    match now.saturating_sub(epoch) {
        ..60 => "running".to_owned(),
        age @ 60..3_600 => format!("running {}m", age / 60),
        age @ 3_600..86_400 => format!("running {}h", age / 3_600),
        age => format!("running {}d", age / 86_400),
    }
}

/// When it last ran, or that it never has.
///
/// "never" is a fact worth stating; an em-dash is not.
pub(crate) fn last_run_label(job: &ScheduledJob) -> String {
    job.last_run
        .as_deref()
        .and_then(rfc3339_to_epoch)
        .map_or_else(|| "never".to_owned(), relative_time)
}

/// The row's title.
///
/// A job id is a recipe file stem — `nightly-dependency-audit` — and a column
/// of stems reads as a directory listing rather than as a list of jobs.
pub(crate) fn title_for(id: &str) -> String {
    humanize(id)
}

/// The recipe file this job runs, without the path or the extension.
pub(crate) fn recipe_name(source: &str) -> &str {
    source
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(source)
        .trim_end_matches(".yaml")
        .trim_end_matches(".yml")
}

/// Whether the cadence row is a control or a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cadence {
    Editable(Schedule),
    /// A cron this app's grammar cannot hold, shown as itself and left alone.
    ///
    /// The raw string is evidence rather than state — it is the only way to
    /// recognise the schedule on the machine that *can* edit it — and opening
    /// a sheet on it could only rewrite it into something else, which is
    /// design rule 11's target. The same call `views/recipes.rs` already makes
    /// for a recipe.
    Fixed(String),
}

pub(crate) fn cadence(job: &ScheduledJob) -> Cadence {
    cron::parse(&job.cron).map_or_else(|| Cadence::Fixed(job.cron.clone()), Cadence::Editable)
}

/// The cadence sheet's rows: the Recipes sheet without its "Never".
///
/// A job that exists always has a cron — `schedules/update` takes
/// `cron: String`, not an `Option`, and there is no null to send — so "Never"
/// would be a choice with no method behind it, which is design rule 11's exact
/// target. Taking a job off its timer is Delete: a different question, with a
/// confirm of its own, already on this screen.
pub(crate) fn cadence_rows(schedule: Schedule, on: bool) -> Vec<SettingRow> {
    let mut rows = crate::recipes::schedule_rows(schedule, on);
    if let Some(repeat) = rows.first_mut() {
        repeat
            .choices
            .retain(|choice| choice.value != crate::recipes::SCHEDULE_OFF);
    }
    rows
}

/// The facts card: what is true about this job that no control here changes.
pub(crate) fn facts(job: &ScheduledJob, now: i64) -> Vec<SettingRow> {
    let mut rows = vec![SettingRow::fact(
        "recipe",
        "Recipe",
        recipe_name(&job.source),
        job.source.clone(),
    )];
    rows.push(SettingRow::fact(
        "last_run",
        "Last run",
        last_run_label(job),
        "goose runs this on the server, whether or not the phone is on.",
    ));
    if job.currently_running {
        rows.push(SettingRow::fact(
            "started",
            "Started",
            running_for(job.job_start_time.as_deref(), now),
            "A run already in flight. Pausing the schedule does not stop it.",
        ));
    }
    rows
}

/// One button in the detail's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DetailAction {
    /// Which handler the view wires this to.
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub class: &'static str,
}

/// The buttons the detail offers, in the order it offers them.
///
/// A free function so the rules — no Kill on something that is not running, no
/// "Watch it run" without a session to open, exactly one primary — are pinned
/// without mounting anything.
///
/// While a run is in flight, watching it *is* the thing to do: the streaming
/// transcript shows the tool calls arriving, which is worth more than any
/// refresh cadence. So it leads, and takes the primary fill.
pub(crate) fn detail_actions(state: ScheduleState, can_watch: bool) -> Vec<DetailAction> {
    let running = matches!(state, ScheduleState::Running);
    let mut actions = Vec::new();
    if running && can_watch {
        actions.push(DetailAction {
            id: "watch",
            label: "Watch it run",
            icon: "message",
            class: "btn primary",
        });
    }
    actions.push(DetailAction {
        id: "run",
        label: "Run now",
        icon: "play",
        // One primary per screen. While something is running the star is the
        // transcript, not another start.
        class: if actions.is_empty() {
            "btn primary"
        } else {
            "btn secondary"
        },
    });
    actions.push(if matches!(state, ScheduleState::Paused) {
        DetailAction {
            id: "resume",
            label: "Resume",
            icon: "play",
            class: "btn secondary",
        }
    } else {
        DetailAction {
            id: "pause",
            label: "Pause",
            icon: "pause",
            class: "btn secondary",
        }
    });
    if running {
        // Last, because `.btn-row` fills in reading order and the destructive
        // one should be the furthest thing from a thumb landing on the screen.
        actions.push(DetailAction {
            id: "kill",
            label: "Kill",
            icon: "stop",
            class: "btn danger-outline",
        });
    }
    actions
}

/// The history row that is the run currently in flight, if the history in hand
/// holds it.
///
/// "Watch it run" needs a whole [`SessionInfo`] — `session/load` wants the
/// session's `cwd`, and a job's `currentSessionId` is only an id. The history
/// is where the rest of it is, so the button exists exactly when the answer
/// does. Offering it otherwise would be a control that opens a transcript
/// against the wrong working directory, which is worse than no button (design
/// rule 11).
pub(crate) fn watch_target(
    history: &[SessionInfo],
    session_id: Option<&str>,
) -> Option<SessionInfo> {
    let wanted = session_id?;
    history
        .iter()
        .find(|info| info.session_id == wanted && info.cwd.is_some())
        .cloned()
}

/// Whether the open job's run history is now out of date.
///
/// A run that just finished has written a session the history does not have,
/// and the history is the best thing on this screen — so the poll that noticed
/// the dot go out is the right moment to fetch it, rather than making somebody
/// pull. Only for the job that is actually open: a background job finishing is
/// not a reason to fetch anything.
pub(crate) fn history_is_stale(
    before: &[ScheduledJob],
    after: &[ScheduledJob],
    open: Option<&str>,
) -> bool {
    let Some(open) = open else {
        return false;
    };
    let running = |jobs: &[ScheduledJob]| {
        jobs.iter()
            .find(|job| job.id == open)
            .is_some_and(|job| job.currently_running)
    };
    running(before) && !running(after)
}

// ------------------------------------------------------------------ actions

/// Load the list if nothing is in it yet. The first visit's fetch.
pub(crate) fn ensure_loaded(ctx: &AppCtx) {
    let idle = {
        let remote = ctx.scheduler.list.peek();
        remote.items.is_empty() && !remote.loading
    };
    if idle {
        refresh(ctx);
    }
}

/// Fetch the list loudly: the spinner arms, a failure sticks or toasts.
///
/// The first load and the pull gesture, and nothing else. Every other fetch on
/// this screen is [`poll_once`], which is quiet.
pub(crate) fn refresh(ctx: &AppCtx) {
    let ctx = *ctx;
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            // Not an error to report: the bar's dot already says the phone is
            // offline, and a toast on top of that sentence says it twice.
            return;
        };
        load_remote(&ctx, ctx.scheduler.list, async move {
            client.schedules_list().await
        })
        .await;
    });
}

/// The pull gesture. One name for both scrollers, because the dot on a row and
/// the buttons on that row's detail are the same fact.
pub(crate) fn pull_refresh(ctx: &AppCtx) {
    refresh(ctx);
    if let Some(id) = ctx.scheduler.open.peek().clone() {
        load_history(ctx, &id);
    }
}

/// One poll tick's fetch: quiet.
///
/// It does not go through `load_remote`, and that is the whole point. Every
/// tick would otherwise call `Remote::begin`, which arms the pull spinner and
/// puts the screen into its "Loading…" state — every five seconds, forever.
/// So: settle on success, latch `unsupported` (which is a fact about the
/// server and worth keeping), and say nothing at all about a transient
/// failure. A list you can still read beats the reason the last refresh
/// failed, which the connection badge is already reporting.
pub(crate) async fn poll_once(ctx: &AppCtx) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let mut list = ctx.scheduler.list;
    match client.schedules_list().await {
        Ok(jobs) => {
            let open = ctx.scheduler.open.peek().clone();
            let stale = history_is_stale(&list.peek().items, &jobs, open.as_deref());
            list.write().settle(jobs);
            if let Some(id) = open.filter(|_| stale) {
                load_history(ctx, &id);
            }
        }
        Err(e) if e.is_unsupported() => {
            list.write().fail(&e);
        }
        Err(_) => {}
    }
}

/// Push the detail screen for one job.
///
/// It does **not** fetch. The detail's own connection-reactive effect owns
/// that, exactly as the list's effect owns [`ensure_loaded`] — which is what
/// makes the history recover on its own. `list_state` keeps a loaded list
/// tappable after the socket dies, so a job can be opened while offline; a
/// fetch fired from here would find no client and return, and nothing would
/// ever fire a second one. One fetch path, driven by the connection, has both
/// cases: it does nothing while there is no client and runs the moment there
/// is one.
///
/// It does arm the spinner, though, and that half has to happen here. An effect
/// runs *after* the render that mounted it, so a slot left settled and empty
/// would paint one committed frame of "No runs yet" — the exact sentence this
/// arrangement exists to stop the screen saying — before the fetch it is about
/// to make marks it. Claiming the slot synchronously, before the screen paints,
/// is the difference between "we have not asked yet" and "there are none".
/// Nothing can strand it: while there is no connection `list_state` answers
/// `Offline` without consulting `loading` at all.
pub(crate) fn open(ctx: &AppCtx, id: &str) {
    let (mut open, mut screen) = (ctx.scheduler.open, ctx.scheduler.screen);
    open.set(Some(id.to_owned()));
    screen.set(Screen::Detail);
    ctx.scheduler.history.clone().write().begin();
}

/// Back to the list. The history goes with it: it belongs to a job that is no
/// longer on screen, and the next open fetches its own.
pub(crate) fn close(ctx: &AppCtx) {
    let (mut open, mut screen, mut sheet) = (
        ctx.scheduler.open,
        ctx.scheduler.screen,
        ctx.scheduler.sheet,
    );
    screen.set(Screen::List);
    open.set(None);
    sheet.set(Sheet::Closed);
    ctx.scheduler.history.clone().write().settle(Vec::new());
    ctx.scheduler.history_of.clone().set(None);
}

/// Fetch one job's run history.
///
/// The one fetch in this app that does **not** go through `load_remote`, and
/// the reason is the identity check: two opens in quick succession put two
/// requests in flight for one slot, and the check that decides whether an
/// answer may be written has to sit between the await and the write —
/// which is exactly the seam `load_remote` closes over. `Remote`'s three
/// flags are kept in step by hand here, and the arms below are the same three
/// it has.
///
/// A stale answer returns rather than clearing: settling an empty list would
/// replace the history the reader is looking at with "No runs yet", which is
/// a false statement rather than a missing one. The Chats list draws the same
/// line with `sessions_epoch`.
pub(crate) fn load_history(ctx: &AppCtx, id: &str) {
    // Claimed before the task starts, so the newest open owns the slot no
    // matter which request answers first.
    ctx.scheduler.history_of.clone().set(Some(id.to_owned()));
    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            return;
        };
        let mut history = ctx.scheduler.history;
        history.write().begin();
        let result = client.schedules_sessions(&id, RUN_HISTORY).await;
        if ctx.scheduler.history_of.peek().as_deref() != Some(id.as_str()) {
            return;
        }
        match result {
            Ok(runs) => history.write().settle(runs),
            Err(e) => {
                // The guard is dropped before the toast: `show_toast` reads the
                // context, and holding a write borrow across that is how a
                // re-entrant read turns into a panic.
                let toast = history.write().fail(&e);
                if let Some(message) = toast {
                    show_toast(&ctx, message);
                }
            }
        }
    });
}

/// Move a job to a different cadence.
///
/// `cron` is already built by `crate::cron`, so there is nothing here to
/// validate — that is the point of the sheet producing choices instead of
/// text.
pub(crate) fn set_cadence(ctx: &AppCtx, id: &str, cron: &str) {
    let (ctx, id, cron) = (*ctx, id.to_owned(), cron.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.schedules_update(&id, &cron).await {
            // Re-listed rather than patched, even though the reply carries the
            // updated job: what is shown is what the server holds. Here that is
            // nearly free — the poll is already doing it.
            Ok(_) => {
                show_toast(&ctx, cron::summary(&cron));
                poll_once(&ctx).await;
            }
            Err(e) => show_toast(&ctx, format!("Cadence not saved: {e}")),
        }
    });
}

/// Pause or resume.
pub(crate) fn set_paused(ctx: &AppCtx, id: &str, paused: bool) {
    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.schedules_set_paused(&id, paused).await {
            Ok(()) => poll_once(&ctx).await,
            Err(e) => show_toast(
                &ctx,
                format!("{} failed: {e}", if paused { "Pause" } else { "Resume" }),
            ),
        }
    });
}

/// Remove a job. The recipe file it points at stays where it is.
pub(crate) fn delete(ctx: &AppCtx, id: &str) {
    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.schedules_delete(&id).await {
            Ok(()) => {
                // The screen showing it has nothing left to show.
                if ctx.scheduler.open.peek().as_deref() == Some(id.as_str()) {
                    close(&ctx);
                }
                poll_once(&ctx).await;
            }
            Err(e) => show_toast(&ctx, format!("Delete failed: {e}")),
        }
    });
}

/// The re-list a mutation to `id` earns, and the history refetch that goes
/// with it — each under the guard that decides whether it is wanted.
///
/// One function so the two callers cannot disagree, which they did: `kill`
/// refetched unconditionally. Both guards matter.
///
/// **The tab**, because a mutation can resolve long after the screen is gone —
/// `run_now` answers when the run ends, minutes later — and a phone in a
/// pocket must not fetch because something finished in the background.
///
/// **The open job's identity**, because [`load_history`] claims `history_of`
/// as its first act. Firing it for a job whose detail has already been closed
/// undoes [`close`]'s `history_of = None` and settles a closed job's runs into
/// the shared slot — re-arming the identity check against an id that is not on
/// screen, which is the one thing that check exists to get right.
async fn refetch(ctx: &AppCtx, id: &str) {
    if *ctx.tab.peek() != Tab::Scheduler {
        return;
    }
    poll_once(ctx).await;
    if ctx.scheduler.open.peek().as_deref() == Some(id) {
        load_history(ctx, id);
    }
}

/// Stop the run in flight. The schedule stays, and it fires again at its next
/// time.
pub(crate) fn kill(ctx: &AppCtx, id: &str) {
    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            show_toast(&ctx, "Not connected — reconnect in Settings");
            return;
        };
        match client.schedules_kill(&id).await {
            Ok(()) => {
                // A run this device started and then killed: the advisory flag
                // outlives the request that set it, so it has to be told.
                ctx.scheduler.started_here.clone().write().remove(&id);
                show_toast(&ctx, "Stopped. The schedule is still on.");
                refetch(&ctx, &id).await;
            }
            Err(e) => show_toast(&ctx, format!("Could not stop it: {e}")),
        }
    });
}

/// Run a job now, out of band.
///
/// The request does not answer until the run is over — minutes — so nothing
/// here waits on it. The toast and the busy dot happen before the await; the
/// poll is what makes them true. If the socket dies the job keeps running on
/// the server and the next list finds it, which is why a dead socket says
/// nothing: the connection badge already reports that, and "Run failed" would
/// be a false statement about a job that is running fine.
pub(crate) fn run_now(ctx: &AppCtx, id: &str) {
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(ctx, "Not connected — reconnect in Settings");
        return;
    };
    let mut started = ctx.scheduler.started_here;
    if started.peek().contains(id) {
        // Already waiting on one from this device. A second tap would start a
        // second run — `run_now` is not idempotent on the server.
        return;
    }
    started.write().insert(id.to_owned());
    show_toast(ctx, "Started — it'll show up in this job's history");

    let (ctx, id) = (*ctx, id.to_owned());
    spawn_forever(async move {
        let outcome = client.schedules_run_now(&id).await;
        // The advisory flag's whole life is this request's, including the
        // socket dying: cleared here, on every arm below.
        ctx.scheduler.started_here.clone().write().remove(&id);
        match outcome {
            Ok(RunNowResponse {
                status: RunStatus::Cancelled,
                ..
            }) => show_toast(&ctx, "That run was cancelled"),
            // Two silences, for two different reasons, deliberately not merged.
            //
            // A clean finish says nothing because the toast would land minutes
            // later, quite possibly on another screen, to repeat what the new
            // history row already says. A dead socket says nothing because
            // this client stopped listening and the job did not stop running —
            // "Run failed" would be a false statement about a run that is
            // going fine, and the connection badge already reports the part
            // that did fail.
            Ok(RunNowResponse {
                status: RunStatus::Completed,
                ..
            })
            | Err(AcpError::Closed | AcpError::Timeout) => {}
            Err(e) => show_toast(&ctx, format!("Run failed: {e}")),
        }
        refetch(&ctx, &id).await;
    });
}

/// Open one of this job's runs as a chat.
///
/// The tab is set first, and that line is the whole function. `open_session`
/// sets the *Home* screen and never the tab, because every other caller is
/// already on Home — so from here the transcript would load into a stack
/// nothing is rendering, and the tap would look like a dead button.
pub(crate) fn watch(ctx: &AppCtx, info: SessionInfo) {
    let (mut tab, mut screen) = (ctx.tab, ctx.screen);
    tab.set(Tab::Home);
    screen.set(HomeScreen::Chat);
    crate::state::open_session(ctx, info);
}

/// The socket in here is `pub(crate)` from the waist down.
///
/// [`serve`] and the [`Script`] it takes are the only way anything in this
/// crate can drive a real [`AcpClient`]: the type has no constructor but
/// `connect`, so a request body, a reply and every arm behind an `await` were
/// unreachable until a server existed to put in front of it. `src/recipes.rs`
/// has the same four shapes of unreachable code — a verdict, a delete that
/// succeeded, a schedule refused as unsupported — and re-typing a WebSocket
/// JSON-RPC listener per module would be re-typing the one part of this that
/// is fiddly rather than the part that is interesting.
#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test scaffolding: a harness that cannot start is the failing check"
)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    use futures_util::{SinkExt as _, StreamExt as _};
    use goose_acp_client::{AcpClient, ConnectConfig};
    use serde_json::{json, Map, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;

    fn job(id: &str, cron: &str, running: bool, paused: bool) -> ScheduledJob {
        ScheduledJob {
            id: id.to_owned(),
            source: format!("/home/demo/.config/goose/scheduled-recipes/{id}.yaml"),
            cron: cron.to_owned(),
            last_run: None,
            currently_running: running,
            paused,
            current_session_id: None,
            job_start_time: None,
            extra: Map::new(),
        }
    }

    fn session(id: &str, cwd: Option<&str>) -> SessionInfo {
        SessionInfo {
            session_id: id.to_owned(),
            cwd: cwd.map(str::to_owned),
            title: None,
            updated_at: None,
            meta: None,
        }
    }

    /// Two screens under one dump key means the second overwrites the first in
    /// the gallery, and whatever it was showing sits outside everything
    /// `docs/audit.js` checks.
    #[test]
    fn each_screen_dumps_under_a_key_of_its_own() {
        assert_ne!(dump_key(Screen::List), dump_key(Screen::Detail));
        // The list's key is the destination id, which is what the shell falls
        // back to when a destination names no screen.
        assert_eq!(dump_key(Screen::List), "scheduler");
    }

    // ---- the poll ----

    /// A loop retires for a newer loop, and for the screen going away. Nothing
    /// else ends it — a disconnection is temporary, and a screen that stopped
    /// polling because the tailnet blinked would stay wrong until somebody
    /// pulled it.
    #[test]
    fn a_poll_loop_retires_for_a_newer_loop_and_for_leaving_the_tab() {
        let tick = |current, tab, connected, loading, overlay| {
            poll_tick(7, current, tab, connected, loading, overlay)
        };
        assert_eq!(tick(7, Tab::Scheduler, true, false, false), Tick::Fetch);
        assert_eq!(tick(8, Tab::Scheduler, true, false, false), Tick::Retire);
        assert_eq!(tick(7, Tab::Home, true, false, false), Tick::Retire);
        assert_eq!(tick(7, Tab::Recipes, true, false, false), Tick::Retire);
        // Temporary, so the loop stays alive and asks again.
        assert_eq!(tick(7, Tab::Scheduler, false, false, false), Tick::Idle);
    }

    /// The list timeout is 30 s and the busy cadence is 5 s, so a slow tailnet
    /// would otherwise stack six requests behind each other.
    #[test]
    fn a_tick_never_stacks_on_a_fetch_that_is_still_out() {
        assert_eq!(
            poll_tick(1, 1, Tab::Scheduler, true, true, false),
            Tick::Idle
        );
    }

    /// A re-list under an open sheet reorders the rows behind it, which on the
    /// native renderer is a visible reflow under something you are in the
    /// middle of deciding.
    #[test]
    fn an_open_overlay_suspends_the_poll() {
        assert_eq!(
            poll_tick(1, 1, Tab::Scheduler, true, false, true),
            Tick::Idle
        );
        assert!(!Sheet::Closed.is_open());
        for sheet in [
            Sheet::Cadence,
            Sheet::ConfirmKill("x".to_owned()),
            Sheet::ConfirmDelete("x".to_owned()),
        ] {
            assert!(sheet.is_open());
        }
    }

    /// The advisory flag's first job: tighten the cadence the moment Run now is
    /// pressed, rather than up to thirty seconds later — which is exactly the
    /// window in which somebody is watching to see whether their tap did
    /// anything.
    #[test]
    fn a_run_this_device_started_tightens_the_cadence_before_the_server_agrees() {
        let jobs = [job("nightly", "0 0 2 * * *", false, false)];
        let mut mine = HashSet::new();
        assert!(!any_running(&jobs, &mine));
        assert_eq!(poll_interval(false), POLL_IDLE);

        mine.insert("nightly".to_owned());
        assert!(any_running(&jobs, &mine));
        assert_eq!(poll_interval(true), POLL_BUSY);
    }

    /// Promotion only. The flag may say "running" sooner than the server does
    /// and never later, so a request that died on a dead socket cannot pin a
    /// dot on forever.
    #[test]
    fn the_local_flag_only_ever_promotes() {
        let paused = job("nightly", "0 0 2 * * *", false, true);
        let mut mine = HashSet::new();
        assert_eq!(row_state(&paused, &mine), ScheduleState::Paused);
        mine.insert("nightly".to_owned());
        assert_eq!(row_state(&paused, &mine), ScheduleState::Running);

        // And a job the server calls running stays running whatever this
        // device thinks.
        let running = job("other", "0 0 2 * * *", true, false);
        assert_eq!(row_state(&running, &HashSet::new()), ScheduleState::Running);
    }

    /// The history is the best thing on the screen, so the poll that notices a
    /// run end is the right moment to fetch it — and only for the job whose
    /// detail is actually open.
    #[test]
    fn a_run_ending_under_the_open_job_is_what_refetches_its_history() {
        let before = [job("nightly", "0 0 2 * * *", true, false)];
        let after = [job("nightly", "0 0 2 * * *", false, false)];
        assert!(history_is_stale(&before, &after, Some("nightly")));
        // Still running: nothing new to fetch.
        assert!(!history_is_stale(&before, &before, Some("nightly")));
        // A background job finishing is not a reason to fetch anything.
        assert!(!history_is_stale(&before, &after, Some("other")));
        assert!(!history_is_stale(&before, &after, None));
    }

    // ---- copy ----

    /// `relative_time` answers "now" under a minute, and "running now" reads as
    /// a status that has not started rather than as a duration.
    #[test]
    fn a_run_in_flight_says_running_rather_than_running_now() {
        let start = "2026-08-25T09:00:00Z";
        let epoch = rfc3339_to_epoch(start).unwrap_or_default();
        // Under a minute, where `relative_time` would answer "now" — and
        // "running now" reads as a status that has not started.
        assert_eq!(running_for(Some(start), epoch + 59), "running");
        // Absent is the honest common case, not an error: goose omits
        // `jobStartTime` on a job it has only just marked running.
        assert_eq!(running_for(None, epoch), "running");
        assert_eq!(running_for(Some("not a timestamp"), epoch), "running");
    }

    #[test]
    fn a_run_in_flight_is_measured_in_the_unit_it_has_reached() {
        let start = "2026-01-02T03:00:00Z";
        let epoch = rfc3339_to_epoch(start).unwrap_or_default();
        assert_eq!(running_for(Some(start), epoch + 30), "running");
        assert_eq!(running_for(Some(start), epoch + 4 * 60), "running 4m");
        assert_eq!(running_for(Some(start), epoch + 3 * 3_600), "running 3h");
        assert_eq!(running_for(Some(start), epoch + 2 * 86_400), "running 2d");
        // A clock that disagrees with the server's is not a negative duration.
        assert_eq!(running_for(Some(start), epoch - 500), "running");
    }

    #[test]
    fn a_job_that_never_ran_says_never() {
        let mut job = job("nightly", "0 0 2 * * *", false, false);
        assert_eq!(last_run_label(&job), "never");
        job.last_run = Some("nonsense".to_owned());
        assert_eq!(last_run_label(&job), "never");
    }

    /// Design rule 8, mechanically: whatever the server stored, what the row
    /// gets is a sentence.
    #[test]
    fn a_row_reads_as_words_and_never_as_a_cron() {
        let now = 1_800_000_000;
        let scheduled = job("nightly", "0 30 9 * * 1-5", false, false);
        assert_eq!(
            state_label(&scheduled, ScheduleState::Scheduled, now),
            "Runs every weekday at 9:30 AM"
        );

        // A cron this grammar cannot hold still gets words on the row; the
        // detail is where the expression itself is shown as evidence.
        let odd = job("smoke", "*/15 9-17 * * 1-5", false, false);
        let label = state_label(&odd, ScheduleState::Scheduled, now);
        assert_eq!(label, "Runs on a schedule");
        assert!(!label.contains('*'), "the cron leaked into the copy");

        assert_eq!(
            state_label(&scheduled, ScheduleState::Paused, now),
            "paused"
        );
    }

    /// A running row says how long the run has been going — not the bare word
    /// "running", and above all not the cadence. The cadence is what the job
    /// will do *next*, which is not what somebody watching a run in flight is
    /// looking at the row to find out.
    #[test]
    fn a_running_row_says_how_long_the_run_has_been_going() {
        let mut running = job("nightly", "0 30 9 * * 1-5", true, false);
        running.job_start_time = Some("2026-01-02T03:00:00Z".to_owned());
        let epoch = rfc3339_to_epoch("2026-01-02T03:00:00Z").unwrap_or_default();
        assert_eq!(
            state_label(&running, ScheduleState::Running, epoch + 2 * 3_600),
            "running 2h"
        );
    }

    #[test]
    fn a_job_title_is_a_name_and_not_a_file_stem() {
        assert_eq!(
            title_for("nightly-dependency-audit"),
            "Nightly dependency audit"
        );
        assert_eq!(
            title_for("weekly_changelog_digest"),
            "Weekly changelog digest"
        );
    }

    #[test]
    fn the_recipe_name_drops_the_path_and_the_extension() {
        assert_eq!(
            recipe_name("/home/demo/.config/goose/scheduled-recipes/nightly.yaml"),
            "nightly"
        );
        assert_eq!(recipe_name("/a/b/nightly.yml"), "nightly");
        assert_eq!(recipe_name("nightly.yaml"), "nightly");
        assert_eq!(recipe_name(""), "");
    }

    // ---- the sheet ----

    /// `schedules/update` takes a required cron, so "Never" would be a choice
    /// with no method behind it. Taking a job off its timer is Delete.
    #[test]
    fn the_cadence_sheet_offers_no_never() {
        let rows = cadence_rows(Schedule::default(), true);
        assert!(
            !rows
                .iter()
                .flat_map(|row| &row.choices)
                .any(|choice| choice.value == crate::recipes::SCHEDULE_OFF),
            "the cadence sheet still offers Never"
        );
        // And the rest of the sheet is untouched — this drops one choice, it
        // does not fork the grammar.
        let repeat = rows.first();
        assert_eq!(repeat.map(|row| row.name.as_str()), Some("Repeat"));
        assert_eq!(
            repeat.map(|row| row.choices.len()),
            Some(crate::cron::Repeat::ALL.len())
        );
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Repeat", "Hour", "Minute"]);
    }

    /// A cron nobody here can express is shown as itself and left alone —
    /// opening a sheet on it could only rewrite it into something else.
    #[test]
    fn a_cron_this_grammar_cannot_hold_is_a_fact_rather_than_a_control() {
        let odd = job("smoke", "*/15 9-17 * * 1-5", false, false);
        assert_eq!(
            cadence(&odd),
            Cadence::Fixed("*/15 9-17 * * 1-5".to_owned())
        );

        let plain = job("nightly", "0 30 9 * * 1-5", false, false);
        assert!(matches!(cadence(&plain), Cadence::Editable(_)));
    }

    // ---- the detail's buttons ----

    #[test]
    fn kill_is_only_offered_for_something_that_is_running() {
        for state in [ScheduleState::Paused, ScheduleState::Scheduled] {
            let ids: Vec<&str> = detail_actions(state, true)
                .iter()
                .map(|action| action.id)
                .collect();
            assert!(!ids.contains(&"kill"), "{ids:?}");
            assert!(!ids.contains(&"watch"), "{ids:?}");
        }
        let ids: Vec<&str> = detail_actions(ScheduleState::Running, true)
            .iter()
            .map(|action| action.id)
            .collect();
        assert_eq!(ids, ["watch", "run", "pause", "kill"]);
    }

    /// Without a session in hand there is nothing to open, and a button that
    /// loads a transcript against the wrong directory is worse than no button.
    #[test]
    fn watching_is_only_offered_when_there_is_something_to_watch() {
        let ids: Vec<&str> = detail_actions(ScheduleState::Running, false)
            .iter()
            .map(|action| action.id)
            .collect();
        assert_eq!(ids, ["run", "pause", "kill"]);
    }

    #[test]
    fn exactly_one_button_is_the_primary_one() {
        for (state, can_watch) in [
            (ScheduleState::Running, true),
            (ScheduleState::Running, false),
            (ScheduleState::Paused, true),
            (ScheduleState::Scheduled, false),
        ] {
            let actions = detail_actions(state, can_watch);
            let primaries = actions
                .iter()
                .filter(|action| action.class.contains("primary"))
                .count();
            assert_eq!(primaries, 1, "{state:?} / {can_watch}: {actions:?}");
        }
        // While something is running the star is the transcript, not another
        // start.
        assert_eq!(detail_actions(ScheduleState::Running, true)[0].id, "watch");
    }

    #[test]
    fn a_paused_job_offers_resume_and_a_running_one_offers_pause() {
        let paused = detail_actions(ScheduleState::Paused, false);
        assert_eq!(paused[1].id, "resume");
        assert_eq!(paused[1].label, "Resume");
        let scheduled = detail_actions(ScheduleState::Scheduled, false);
        assert_eq!(scheduled[1].id, "pause");
    }

    #[test]
    fn the_run_in_flight_is_found_in_the_history_or_not_offered_at_all() {
        let history = [
            session("20260825_9", Some("/home/demo")),
            session("20260824_1", Some("/home/demo")),
        ];
        let found = watch_target(&history, Some("20260825_9"));
        assert_eq!(
            found.map(|info| info.session_id).as_deref(),
            Some("20260825_9")
        );

        // Not in the history yet — a run started seconds ago.
        assert!(watch_target(&history, Some("20260826_1")).is_none());
        // Nothing running.
        assert!(watch_target(&history, None).is_none());
        // Present but with no cwd, so `session/load` has nowhere to load from.
        let cwdless = [session("20260825_9", None)];
        assert!(watch_target(&cwdless, Some("20260825_9")).is_none());
    }

    // ---- facts ----

    #[test]
    fn the_facts_state_a_started_time_only_while_something_is_started() {
        let now = 1_800_000_000;
        let idle = facts(&job("nightly", "0 0 2 * * *", false, false), now);
        let idle_rows: Vec<&str> = idle.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(idle_rows, ["Recipe", "Last run"]);

        let mut running = job("nightly", "0 0 2 * * *", true, false);
        running.job_start_time = Some("2026-01-02T03:00:00Z".to_owned());
        let epoch = rfc3339_to_epoch("2026-01-02T03:00:00Z").unwrap_or_default();
        let rows = facts(&running, epoch + 300);
        let running_rows: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.value.as_str()))
            .collect();
        assert_eq!(
            running_rows,
            [
                ("Recipe", "nightly"),
                ("Last run", "never"),
                ("Started", "running 5m"),
            ]
        );
    }

    // ------------------------------------------------------------- harness
    //
    // Everything under "actions" above needs three things none of the checks
    // so far needed: an `AppCtx` to write into, a Dioxus runtime to hold its
    // signals and poll the tasks `spawn_forever` starts, and — for anything
    // past the first `let Some(client)` — a live `AcpClient`.
    //
    // The third is why there is a socket in here. `AcpClient` has no
    // constructor that is not `connect`, so the only way to drive the half of
    // this module that talks to a server is to put a server in front of it:
    // a plain-`ws://` JSON-RPC listener on a loopback port, answering the
    // `_goose/unstable/schedules/*` methods and logging what it was asked.
    // `ws_url` only reaches for TLS on an `https://` base, so `http://` here
    // means no certificate and no fingerprint.

    thread_local! {
        /// The context `Probe` built, so a test can reach it.
        static PUBLISHED: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
    }

    /// One component holding one `AppCtx`, built field by field.
    ///
    /// Deliberately **not** `state::use_app_ctx_provider`. Two of its fields
    /// are persistent, and the filesystem backing behind them *panics* unless
    /// a process-wide `set_directory` has already run (`dioxus-sdk-storage`
    /// `client_storage/fs.rs:44`). That `OnceLock` is claimed — and can only
    /// be claimed once — by `ask_journal`'s
    /// `the_journals_storage_backing_really_reaches_the_disk`, so a harness
    /// that called the real provider would either panic or make *that* test
    /// panic, depending on which of the two the test runner started first.
    /// This one touches no disk at all.
    ///
    /// The cost is that a new field on `AppCtx` fails to compile here. That
    /// is the intended trade: the alternative is a harness that silently
    /// hands a screen a context the app never builds.
    #[component]
    fn Probe() -> Element {
        let ctx = AppCtx {
            screen: use_signal(|| HomeScreen::Settings),
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
            inspector_open: use_signal(|| true),
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
            extensions: crate::extensions::use_ctx(),
            skills: crate::skills::use_ctx(),
            recipes: crate::recipes::use_recipes(),
            scheduler: use_ctx(),
        };
        use_context_provider(|| ctx);
        PUBLISHED.with(|slot| *slot.borrow_mut() = Some(ctx));
        rsx! { div {} }
    }

    /// The reply the mock server sends for one request: how long it sits on
    /// it, then a JSON-RPC `result` or `error`.
    ///
    /// The delay is not decoration. Two of the rules in this module are about
    /// *ordering* — a stale history answer must not overwrite a newer one, and
    /// a second Run now while the first is still out must not start a second
    /// run — and neither can be provoked on a server that answers in the order
    /// it was asked.
    pub(crate) type Reply = (Duration, Result<Value, Value>);

    /// What a mock server answers, per method. A plain `fn` and never a
    /// closure, so the whole script of a test is one readable `match`.
    pub(crate) type Script = fn(&str, &Value) -> Reply;

    pub(crate) fn ok(result: Value) -> Reply {
        (Duration::ZERO, Ok(result))
    }

    pub(crate) fn rpc_error(code: i64, message: &str) -> Reply {
        (
            Duration::ZERO,
            Err(json!({ "code": code, "message": message })),
        )
    }

    /// Not a reply at all: the server hangs up instead of answering.
    ///
    /// The only way to reach [`AcpError::Closed`], which `run_now` treats
    /// differently from every other failure.
    fn hang_up() -> Reply {
        (Duration::ZERO, Err(Value::Null))
    }

    /// The method name with goose's namespace taken off, which is what the
    /// assertions below read.
    fn short(method: &str) -> &str {
        method.trim_start_matches("_goose/unstable/schedules/")
    }

    pub(crate) struct Server {
        pub(crate) base_url: String,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl Server {
        /// Every request this server was sent, in order, with its params and
        /// its method spelled in full. The one accessor that does not assume
        /// goose's scheduler namespace, so another feature's methods come back
        /// readable.
        pub(crate) fn log(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }

        /// Every request this server was sent, in order, by short name. The
        /// handshake is left out: it is the harness's, not the screen's.
        fn methods(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(method, _)| short(method).to_owned())
                .filter(|method| method != "initialize")
                .collect()
        }

        fn count(&self, method: &str) -> usize {
            self.methods().iter().filter(|m| *m == method).count()
        }

        /// The params of the `n`th call to `method`.
        fn params(&self, method: &str, n: usize) -> Value {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| short(m) == method)
                .nth(n)
                .map(|(_, params)| params.clone())
                .expect("the call the assertion is about was never made")
        }
    }

    /// A goose that answers everything happily, with one scheduled job.
    fn happy(method: &str, _params: &Value) -> Reply {
        match short(method) {
            "list" => ok(json!({ "jobs": [wire_job("nightly", false)] })),
            "sessions/list" => ok(json!({ "sessions": [wire_session("run-1")] })),
            "update" => ok(json!({ "job": wire_job("nightly", false) })),
            "running-job/kill" => ok(json!({ "message": "Successfully killed running job" })),
            "run-now" => ok(json!({ "status": "completed", "sessionId": "run-9" })),
            _ => ok(json!({})),
        }
    }

    fn wire_job(id: &str, running: bool) -> Value {
        json!({
            "id": id,
            "source": format!("/home/demo/.config/goose/scheduled-recipes/{id}.yaml"),
            "cron": "0 0 2 * * *",
            "lastRun": null,
            "currentlyRunning": running,
            "paused": false,
            "currentSessionId": null,
            "jobStartTime": null,
        })
    }

    fn wire_session(id: &str) -> Value {
        json!({ "sessionId": id, "cwd": "/home/demo", "title": null })
    }

    struct Harness {
        dom: VirtualDom,
        rt: tokio::runtime::Runtime,
        ctx: AppCtx,
        /// The connection's event stream, parked here for its lifetime. The
        /// client's actor gives up the socket when this end goes away, so a
        /// harness that dropped it would have a connection that died between
        /// the handshake and the first request.
        events: Option<tokio::sync::mpsc::Receiver<goose_acp_client::AcpEvent>>,
    }

    impl Harness {
        /// A mounted app context with no connection: the offline half of every
        /// action.
        fn offline() -> Self {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let mut dom = VirtualDom::new(Probe);
            dom.rebuild_in_place();
            let ctx = PUBLISHED
                .with(|slot| *slot.borrow())
                .expect("the probe rendered, so it published its context");
            Self {
                dom,
                rt,
                ctx,
                events: None,
            }
        }

        /// The same, plus a live client talking to a server running `script`.
        fn connected(script: Script) -> (Self, Server) {
            let mut harness = Self::offline();
            let server = harness.serve(script);
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

        fn serve(&self, script: Script) -> Server {
            serve(&self.rt, script)
        }

        /// Read or write the context. Signals belong to the virtual DOM's
        /// runtime and panic outside it, so every touch goes through here.
        fn with<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
            let ctx = self.ctx;
            self.dom.in_runtime(|| f(&ctx))
        }

        /// Let the queued Dioxus tasks — and the socket under them — run.
        ///
        /// Dioxus polls a spawned task from its own executor, so nothing an
        /// action started happens without this; the timeout is what lets the
        /// tokio runtime carrying the WebSocket actor make progress while the
        /// virtual DOM has nothing to do.
        ///
        /// The budget is 400 ms of *idle* — an iteration with work in it
        /// returns at once — against a longest scripted delay of 60 ms, so a
        /// loaded machine has room before this becomes a flake.
        fn settle(&mut self) {
            let dom = &mut self.dom;
            self.rt.block_on(async {
                for _ in 0..40 {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
                    dom.render_immediate_to_vec();
                }
            });
        }

        /// One poll tick's fetch, run to completion. `poll_once` is an `async
        /// fn` on the context, so it has to go on the same executor the
        /// screen's `use_future` would put it on.
        fn poll(&mut self) {
            self.with(|ctx| {
                let ctx = *ctx;
                spawn_forever(async move {
                    poll_once(&ctx).await;
                });
            });
            self.settle();
        }

        fn toast(&self) -> Option<String> {
            self.with(|ctx| ctx.toast.peek().clone())
        }

        fn runs(&self) -> Vec<String> {
            self.with(|ctx| {
                ctx.scheduler
                    .history
                    .peek()
                    .items
                    .iter()
                    .map(|info| info.session_id.clone())
                    .collect()
            })
        }

        fn jobs(&self) -> Vec<String> {
            self.with(|ctx| {
                ctx.scheduler
                    .list
                    .peek()
                    .items
                    .iter()
                    .map(|job| job.id.clone())
                    .collect()
            })
        }
    }

    /// A goose that answers `script`, on a loopback port, over plain `ws://`.
    ///
    /// `http://` in the base URL is what keeps this certificate-free: `ws_url`
    /// only reaches for TLS on an `https://` base, so there is no fingerprint
    /// to pin and nothing to sign.
    pub(crate) fn serve(rt: &tokio::runtime::Runtime, script: Script) -> Server {
        let listener = rt.block_on(async { TcpListener::bind("127.0.0.1:0").await.unwrap() });
        let port = listener.local_addr().unwrap().port();
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let log = Arc::clone(&calls);
        rt.spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                tokio::spawn(async move { session_loop(socket, log, script).await });
            }
        });
        Server {
            base_url: format!("http://127.0.0.1:{port}"),
            calls,
        }
    }

    async fn session_loop(
        socket: tokio::net::TcpStream,
        log: Arc<Mutex<Vec<(String, Value)>>>,
        script: Script,
    ) {
        let Ok(ws) = tokio_tungstenite::accept_async(socket).await else {
            return;
        };
        let (mut sink, mut stream) = ws.split();
        let (out, mut outbox) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(text) = outbox.recv().await {
                if text.is_empty() {
                    // The hang-up: a close frame and no answer, which is what
                    // reaches the client as `AcpError::Closed`.
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            let Some(id) = frame.get("id").cloned() else {
                continue;
            };
            let method = frame
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            log.lock().unwrap().push((method.clone(), params.clone()));
            let out = out.clone();
            // Answered on a task of its own, so a scripted delay holds up one
            // reply rather than the whole socket.
            tokio::spawn(async move {
                let (delay, body) = if method == "initialize" {
                    ok(json!({
                        "protocolVersion": 1,
                        "agentInfo": { "name": "mock", "version": "0" },
                    }))
                } else {
                    script(&method, &params)
                };
                tokio::time::sleep(delay).await;
                let frame = match body {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err(Value::Null) => {
                        let _ = out.send(String::new());
                        return;
                    }
                    Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
                };
                let _ = out.send(frame.to_string());
            });
        }
    }

    // -------------------------------------------------- navigation, offline

    /// A fresh Scheduler is on its list with nothing claimed. Every other
    /// field here is a claim some later action releases — the open job, the
    /// history's owner, the advisory flags — and one that started life
    /// non-empty would be a claim nothing ever made.
    #[test]
    fn the_scheduler_opens_on_the_list_with_nothing_claimed() {
        let h = Harness::offline();
        h.with(|ctx| {
            let s = ctx.scheduler;
            assert!(
                matches!(*s.screen.peek(), Screen::List),
                "the tab opened straight into a detail, which has no job to show"
            );
            assert_eq!(s.open.peek().as_deref(), None);
            assert_eq!(s.history_of.peek().as_deref(), None);
            assert!(!s.sheet.peek().is_open());
            assert_eq!(*s.poll.peek(), 0);
            assert!(s.started_here.peek().is_empty());
            let list = s.list.peek();
            assert!(
                list.items.is_empty() && !list.loading && !list.unsupported,
                "a list that starts loading has a spinner nothing will ever stop"
            );
        });
    }

    /// The history slot has to be claimed *synchronously*, in the same beat as
    /// the navigation. An effect runs after the render that mounted it, so a
    /// slot left settled and empty paints one committed frame of "No runs
    /// yet" — the exact sentence this arrangement exists to stop the screen
    /// saying about runs it has not asked for.
    #[test]
    fn opening_a_job_marks_its_history_as_asked_for_before_the_screen_paints() {
        let h = Harness::offline();
        h.with(|ctx| open(ctx, "nightly"));
        h.with(|ctx| {
            assert!(matches!(*ctx.scheduler.screen.peek(), Screen::Detail));
            assert_eq!(ctx.scheduler.open.peek().as_deref(), Some("nightly"));
            assert!(
                ctx.scheduler.history.peek().loading,
                "the detail paints one frame of \"No runs yet\" before its own \
                 fetch is even armed"
            );
        });
    }

    /// Backing out has to take the history with it. It belongs to a job that
    /// is no longer on screen, and a slot left holding job A's runs — still
    /// claimed by A's id — would show them under B's title the moment B is
    /// opened, until B's own fetch answered.
    #[test]
    fn closing_the_detail_lets_go_of_the_job_and_of_its_runs() {
        let h = Harness::offline();
        h.with(|ctx| {
            open(ctx, "nightly");
            let (mut sheet, mut history, mut of) = (
                ctx.scheduler.sheet,
                ctx.scheduler.history,
                ctx.scheduler.history_of,
            );
            sheet.set(Sheet::Cadence);
            history
                .write()
                .settle(vec![session("run-1", Some("/home/demo"))]);
            of.set(Some("nightly".to_owned()));
        });
        h.with(close);
        h.with(|ctx| {
            assert!(matches!(*ctx.scheduler.screen.peek(), Screen::List));
            assert_eq!(ctx.scheduler.open.peek().as_deref(), None);
            assert!(
                !ctx.scheduler.sheet.peek().is_open(),
                "a confirm left open over the list is a question about a job \
                 that is no longer on screen"
            );
            assert!(ctx.scheduler.history.peek().items.is_empty());
            assert_eq!(
                ctx.scheduler.history_of.peek().as_deref(),
                None,
                "the identity check is still armed against a job nobody has open"
            );
        });
    }

    /// `open_session` sets the *Home* screen and never the tab, because every
    /// other caller is already on Home. Without the tab line the transcript
    /// loads into a stack nothing is rendering, and "Watch it run" looks like
    /// a dead button.
    #[test]
    fn watching_a_run_moves_to_the_tab_that_draws_the_transcript() {
        let h = Harness::offline();
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
            watch(ctx, session("run-1", Some("/home/demo")));
        });
        h.with(|ctx| {
            assert!(
                matches!(*ctx.tab.peek(), Tab::Home),
                "the chat was opened on the Scheduler tab, which does not draw it"
            );
            assert!(matches!(*ctx.screen.peek(), HomeScreen::Chat));
            let chat = ctx.chat.peek();
            assert_eq!(chat.session_id.as_deref(), Some("run-1"));
            assert_eq!(
                chat.cwd, "/home/demo",
                "the transcript would replay against the wrong directory"
            );
        });
    }

    /// A tap has to be answered. Nothing else on this screen changes when
    /// there is no socket — the row stays exactly as it was — so a mutation
    /// that returned quietly would be indistinguishable from one that worked.
    #[test]
    fn every_mutation_says_so_when_there_is_no_connection() {
        const OFFLINE: &str = "Not connected — reconnect in Settings";
        let mut h = Harness::offline();

        h.with(|ctx| set_cadence(ctx, "nightly", "0 0 2 * * *"));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some(OFFLINE), "saving a cadence");
        h.with(|ctx| ctx.toast.clone().set(None));

        h.with(|ctx| set_paused(ctx, "nightly", true));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some(OFFLINE), "pausing");
        h.with(|ctx| ctx.toast.clone().set(None));

        h.with(|ctx| delete(ctx, "nightly"));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some(OFFLINE), "deleting");
        h.with(|ctx| ctx.toast.clone().set(None));

        h.with(|ctx| kill(ctx, "nightly"));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some(OFFLINE), "killing a run");
        h.with(|ctx| ctx.toast.clone().set(None));

        h.with(|ctx| run_now(ctx, "nightly"));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some(OFFLINE), "running now");
        h.with(|ctx| {
            assert!(
                ctx.scheduler.started_here.peek().is_empty(),
                "the busy dot was lit for a request that was never sent, and \
                 nothing will ever take it off: the flag's life is the \
                 request's, and there was no request"
            );
        });
    }

    /// The fetches are silent when there is no socket, and the mutations above
    /// are not. The difference is deliberate: the connection badge already
    /// says the phone is offline, so a toast on top of it says it twice — but
    /// a tap on a *button* has to be answered, because nothing else on the
    /// screen moves. What a fetch must not do either way is spoil what is
    /// already on screen: a job opened while offline is meant to stay
    /// readable, and a spinner armed here has no request behind it to stop it.
    #[test]
    fn the_fetches_say_nothing_when_there_is_no_socket_and_spoil_nothing() {
        let mut h = Harness::offline();
        h.with(|ctx| {
            let (mut list, mut history) = (ctx.scheduler.list, ctx.scheduler.history);
            list.write()
                .settle(vec![job("nightly", "0 0 2 * * *", false, false)]);
            open(ctx, "nightly");
            history
                .write()
                .settle(vec![session("run-1", Some("/home"))]);
            refresh(ctx);
            load_history(ctx, "nightly");
        });
        h.poll();

        assert_eq!(
            h.toast(),
            None,
            "a fetch with no socket said what the connection badge is already saying"
        );
        assert_eq!(
            h.jobs(),
            ["nightly"],
            "a fetch that never left the device emptied the list it could not replace"
        );
        assert_eq!(h.runs(), ["run-1"]);
        h.with(|ctx| {
            assert!(
                !ctx.scheduler.list.peek().loading && !ctx.scheduler.history.peek().loading,
                "the spinner is armed with no request behind it to ever stop it"
            );
        });
    }

    // ------------------------------------------------------------ the fetch

    /// The first visit fetches, and only the first. `ensure_loaded` runs from
    /// the list's connection-reactive effect, so one that asked again on a
    /// list it already had would put a request on the wire every time you
    /// walked back into the tab.
    #[test]
    fn the_first_visit_fetches_the_list_and_a_second_one_does_not() {
        let (mut h, server) = Harness::connected(happy);
        h.with(ensure_loaded);
        h.settle();
        assert_eq!(h.jobs(), ["nightly"], "the first visit fetched nothing");
        h.with(ensure_loaded);
        h.settle();
        assert_eq!(
            server.count("list"),
            1,
            "arriving at a list already in hand asked for it again"
        );
        // The pull gesture is the loud one, and it always asks.
        h.with(refresh);
        h.settle();
        assert_eq!(
            server.count("list"),
            2,
            "the pull gesture asked for nothing"
        );
    }

    /// One gesture for both scrollers: the dot on a row and the buttons on
    /// that row's detail are the same fact, so a pull has to ask for both
    /// halves of what is on screen.
    #[test]
    fn the_pull_gesture_asks_for_the_list_and_for_the_open_jobs_runs() {
        let (mut h, server) = Harness::connected(happy);
        h.with(pull_refresh);
        h.settle();
        assert_eq!(
            server.methods(),
            ["list"],
            "a pull on the list fetched a history nothing is showing"
        );

        h.with(|ctx| open(ctx, "nightly"));
        h.with(pull_refresh);
        h.settle();
        assert_eq!(
            server.count("sessions/list"),
            1,
            "the runs were not re-read"
        );
        assert_eq!(
            server.params("sessions/list", 0),
            json!({ "scheduleId": "nightly", "limit": RUN_HISTORY }),
        );
        assert_eq!(h.runs(), ["run-1"]);
    }

    /// The poll is quiet and the pull is loud, and that is the whole reason
    /// `poll_once` does not go through `load_remote`. A tick that armed the
    /// spinner would put the screen into "Loading…" every five seconds
    /// forever; a tick that reported a transient failure would stack a
    /// sentence on the screen every five seconds while a tailnet blinked —
    /// over a list that is still perfectly readable.
    #[test]
    fn a_poll_tick_keeps_a_readable_list_and_says_nothing_about_a_hiccup() {
        fn flaky(method: &str, params: &Value) -> Reply {
            if short(method) == "list" {
                return rpc_error(-32000, "the scheduler is having a moment");
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(flaky);
        h.with(|ctx| {
            let mut list = ctx.scheduler.list;
            list.write()
                .settle(vec![job("nightly", "0 0 2 * * *", false, false)]);
        });

        h.poll();
        assert_eq!(
            h.jobs(),
            ["nightly"],
            "a failed tick threw away the list the reader was looking at"
        );
        assert_eq!(
            h.toast(),
            None,
            "the tick reported a hiccup the connection badge is already reporting"
        );
        h.with(|ctx| {
            let list = ctx.scheduler.list.peek();
            assert!(!list.loading, "the tick armed the pull spinner");
            assert_eq!(list.sticky, None, "the tick left a failure on the screen");
        });

        // The pull gesture, over the same failure, does report it — that
        // difference is the point.
        h.with(refresh);
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("the scheduler is having a moment"),
            "a pull that failed said nothing, so the gesture looks broken"
        );
    }

    /// `-32601` is goose's own signal that the server was started without
    /// `--enable-scheduler`. Latched rather than shrugged off: it is a fact
    /// about the server, and it is what makes the screen explain itself
    /// instead of showing an empty list with a Retry that cannot work.
    #[test]
    fn a_server_with_the_scheduler_switched_off_is_remembered_by_the_poll() {
        fn switched_off(_method: &str, _params: &Value) -> Reply {
            rpc_error(-32601, "Scheduled recipe execution is not enabled")
        }
        let (mut h, _server) = Harness::connected(switched_off);
        h.with(|ctx| {
            let mut list = ctx.scheduler.list;
            list.write()
                .settle(vec![job("nightly", "0 0 2 * * *", false, false)]);
        });
        h.poll();
        h.with(|ctx| {
            let list = ctx.scheduler.list.peek();
            assert!(
                list.unsupported,
                "the screen goes on showing a list of jobs a server without \
                 --enable-scheduler cannot run"
            );
            assert!(list.items.is_empty());
        });
        assert_eq!(
            h.toast(),
            None,
            "a switched-off feature was toasted as a failure"
        );
    }

    /// A run that just finished has written a session the history does not
    /// have, and the history is the best thing on this screen — so the tick
    /// that noticed the dot go out is the right moment to fetch it, rather
    /// than making somebody pull.
    #[test]
    fn a_poll_that_sees_the_open_jobs_run_end_re_reads_its_history() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| {
            open(ctx, "nightly");
            let mut list = ctx.scheduler.list;
            list.write()
                .settle(vec![job("nightly", "0 0 2 * * *", true, false)]);
        });
        h.poll();
        assert_eq!(
            h.runs(),
            ["run-1"],
            "the run that just ended is missing from the history until the \
             reader pulls"
        );
        assert_eq!(server.count("sessions/list"), 1);
    }

    /// The other half of the same rule: a tick that changed nothing about the
    /// open job must not fetch its history. At the busy cadence that is a
    /// second request every five seconds, for a list that did not move.
    #[test]
    fn a_poll_that_changes_nothing_fetches_no_history() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| {
            open(ctx, "nightly");
            let mut list = ctx.scheduler.list;
            list.write()
                .settle(vec![job("nightly", "0 0 2 * * *", false, false)]);
        });
        h.poll();
        assert_eq!(
            server.count("sessions/list"),
            0,
            "every tick refetches the history, at up to one request per five seconds"
        );
    }

    /// Two opens in quick succession put two requests in flight for one slot,
    /// and they answer in whatever order the server chooses. The slot belongs
    /// to the newest open, so the older answer is dropped — settling it would
    /// put job A's runs under job B's title.
    #[test]
    fn a_history_answer_for_a_job_that_is_no_longer_open_is_dropped() {
        fn slow_for_nightly(method: &str, params: &Value) -> Reply {
            if short(method) == "sessions/list" {
                let id = params["scheduleId"].as_str().unwrap_or_default();
                let delay = if id == "nightly" {
                    Duration::from_millis(60)
                } else {
                    Duration::ZERO
                };
                return (
                    delay,
                    Ok(json!({ "sessions": [wire_session(&format!("run-of-{id}"))] })),
                );
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(slow_for_nightly);
        h.with(|ctx| {
            load_history(ctx, "nightly");
            load_history(ctx, "weekly");
        });
        h.settle();
        h.with(|ctx| {
            assert_eq!(ctx.scheduler.history_of.peek().as_deref(), Some("weekly"));
        });
        assert_eq!(
            h.runs(),
            ["run-of-weekly"],
            "the slower answer, for a job that is no longer open, overwrote the \
             open one's runs"
        );
    }

    /// A failure with nothing behind it stays on screen; a failure over a
    /// history you can still read is a toast that fades. Both come out of the
    /// same call, and getting them the wrong way round means either an empty
    /// screen that says nothing or a sentence you cannot dismiss.
    #[test]
    fn a_history_that_will_not_load_is_stated_where_there_is_room_for_it() {
        fn no_sessions(method: &str, params: &Value) -> Reply {
            if short(method) == "sessions/list" {
                return rpc_error(-32000, "history is unavailable");
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(no_sessions);

        h.with(|ctx| load_history(ctx, "nightly"));
        h.settle();
        h.with(|ctx| {
            assert_eq!(
                ctx.scheduler.history.peek().sticky.as_deref(),
                Some("history is unavailable"),
                "the detail says \"No runs yet\" about runs it failed to read"
            );
        });
        assert_eq!(
            h.toast(),
            None,
            "a failure with an empty screen behind it was toasted away"
        );

        // With runs already on screen the failure is a toast instead: the
        // list stays readable.
        h.with(|ctx| {
            let mut history = ctx.scheduler.history;
            history
                .write()
                .settle(vec![session("run-1", Some("/home"))]);
            load_history(ctx, "nightly");
        });
        h.settle();
        assert_eq!(h.toast().as_deref(), Some("history is unavailable"));
        assert_eq!(h.runs(), ["run-1"], "a failed refetch emptied the history");
    }

    // -------------------------------------------------------- the mutations

    /// What is shown is what the server holds: the cadence is re-listed
    /// rather than patched onto the row, and the toast is the sheet's answer
    /// in words rather than the cron it just sent.
    #[test]
    fn saving_a_cadence_says_what_it_is_now_and_re_reads_the_list() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| set_cadence(ctx, "nightly", "0 30 9 * * 1-5"));
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("Runs every weekday at 9:30 AM"),
            "the sheet closed without saying what it saved"
        );
        assert_eq!(
            server.params("update", 0),
            json!({ "scheduleId": "nightly", "cron": "0 30 9 * * 1-5" }),
        );
        assert_eq!(
            server.methods(),
            ["update", "list"],
            "the cadence was saved and the screen went on showing the old one"
        );
    }

    /// A write that failed must not be followed by a re-list dressed up as a
    /// success, and the sentence has to name what did not happen.
    #[test]
    fn a_cadence_that_would_not_save_says_so_and_re_reads_nothing() {
        fn refuses_update(method: &str, params: &Value) -> Reply {
            if short(method) == "update" {
                return rpc_error(-32002, "no such schedule");
            }
            happy(method, params)
        }
        let (mut h, server) = Harness::connected(refuses_update);
        h.with(|ctx| set_cadence(ctx, "nightly", "0 30 9 * * 1-5"));
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("Cadence not saved: no such schedule")
        );
        assert_eq!(server.methods(), ["update"]);
    }

    /// One control, two methods — and two sentences, because "Pause failed"
    /// under a button that says Resume names the wrong thing.
    #[test]
    fn a_pause_and_a_resume_fail_in_their_own_words() {
        fn refuses_both(method: &str, params: &Value) -> Reply {
            match short(method) {
                "pause" | "unpause" => rpc_error(-32002, "no such schedule"),
                _ => happy(method, params),
            }
        }
        let (mut h, server) = Harness::connected(refuses_both);
        h.with(|ctx| set_paused(ctx, "nightly", true));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some("Pause failed: no such schedule"));

        h.with(|ctx| set_paused(ctx, "nightly", false));
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("Resume failed: no such schedule")
        );
        assert_eq!(
            server.methods(),
            ["pause", "unpause"],
            "the toggle sent one method for both directions"
        );
    }

    /// A pause that worked says nothing and re-reads instead: the row's own
    /// dot is the answer, and a toast repeating it is a sentence the reader
    /// has to dismiss to see what they just did.
    #[test]
    fn pausing_re_reads_the_list_rather_than_flipping_the_row_in_place() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| set_paused(ctx, "nightly", true));
        h.settle();
        assert_eq!(server.methods(), ["pause", "list"]);
        assert_eq!(h.jobs(), ["nightly"]);
        assert_eq!(
            h.toast(),
            None,
            "a successful pause toasted over its own row"
        );
    }

    /// The detail of a job that no longer exists has nothing to show, so a
    /// delete that succeeded from it takes the screen back with it — and only
    /// when it is *that* job: deleting one row must not close the detail of
    /// another.
    #[test]
    fn deleting_the_open_job_takes_the_screen_back_to_the_list() {
        let (mut h, _server) = Harness::connected(happy);
        h.with(|ctx| {
            open(ctx, "nightly");
            delete(ctx, "weekly");
        });
        h.settle();
        h.with(|ctx| {
            assert_eq!(
                ctx.scheduler.open.peek().as_deref(),
                Some("nightly"),
                "deleting one job closed another job's detail"
            );
        });

        h.with(|ctx| delete(ctx, "nightly"));
        h.settle();
        h.with(|ctx| {
            assert!(
                matches!(*ctx.scheduler.screen.peek(), Screen::List),
                "the detail of a deleted job is still on screen, showing a job \
                 that is gone"
            );
            assert_eq!(ctx.scheduler.open.peek().as_deref(), None);
        });
    }

    #[test]
    fn a_delete_that_failed_says_so_and_leaves_the_detail_alone() {
        fn refuses_delete(method: &str, params: &Value) -> Reply {
            if short(method) == "delete" {
                return rpc_error(-32002, "no such schedule");
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(refuses_delete);
        h.with(|ctx| {
            open(ctx, "nightly");
            delete(ctx, "nightly");
        });
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("Delete failed: no such schedule")
        );
        h.with(|ctx| {
            assert_eq!(
                ctx.scheduler.open.peek().as_deref(),
                Some("nightly"),
                "the detail of a job that is still there was closed anyway"
            );
        });
    }

    /// The advisory flag outlives the request that set it only until that
    /// request resolves — but a kill resolves a *different* request, so it has
    /// to clear the flag itself. Otherwise the row keeps a busy dot on a job
    /// that is not running, and the poll cannot take it off: the flag only
    /// ever promotes.
    #[test]
    fn killing_a_run_this_device_started_takes_its_dot_back_off() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| {
            let (mut tab, mut started) = (ctx.tab, ctx.scheduler.started_here);
            tab.set(Tab::Scheduler);
            started.write().insert("nightly".to_owned());
            kill(ctx, "nightly");
        });
        h.settle();
        h.with(|ctx| {
            assert!(
                !ctx.scheduler.started_here.peek().contains("nightly"),
                "a killed run keeps a busy dot forever: the poll only ever \
                 promotes, so nothing can take this flag off"
            );
        });
        assert_eq!(
            h.toast().as_deref(),
            Some("Stopped. The schedule is still on.")
        );
        assert_eq!(
            server.params("running-job/kill", 0),
            json!({ "jobId": "nightly" }),
        );
        assert_eq!(server.methods(), ["running-job/kill", "list"]);
    }

    #[test]
    fn a_kill_that_failed_says_the_run_is_still_going() {
        fn refuses_kill(method: &str, params: &Value) -> Reply {
            if short(method) == "running-job/kill" {
                return rpc_error(-32602, "no job is running");
            }
            happy(method, params)
        }
        let (mut h, server) = Harness::connected(refuses_kill);
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
            kill(ctx, "nightly");
        });
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("Could not stop it: no job is running")
        );
        assert_eq!(
            server.methods(),
            ["running-job/kill"],
            "a kill that failed re-listed as though it had worked"
        );
    }

    /// The re-fetch a mutation earns, in full: the list, *and* the open job's
    /// runs. Killing a run is the moment the history gains a row, and the
    /// history is the best thing on this screen — leaving it to the poll means
    /// the screen the tap came from is the one screen that does not update.
    #[test]
    fn killing_the_open_jobs_run_re_reads_that_jobs_runs_too() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
            open(ctx, "nightly");
            kill(ctx, "nightly");
        });
        h.settle();
        assert_eq!(
            server.methods(),
            ["running-job/kill", "list", "sessions/list"]
        );
        assert_eq!(
            h.runs(),
            ["run-1"],
            "the run that was just stopped is missing from the history it wrote"
        );
    }

    /// A mutation can resolve long after the screen is gone — `run-now`
    /// answers when the run ends, minutes later — and a phone in a pocket
    /// must not put a request on the wire because something finished in the
    /// background.
    #[test]
    fn a_mutation_that_lands_after_you_left_the_tab_fetches_nothing() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Home);
            open(ctx, "nightly");
            kill(ctx, "nightly");
        });
        h.settle();
        assert_eq!(
            server.methods(),
            ["running-job/kill"],
            "a background mutation woke the socket for a screen nobody is looking at"
        );
    }

    /// The other guard on the same re-fetch: `load_history` claims
    /// `history_of` as its first act, so firing it for a job whose detail has
    /// already been closed re-arms the identity check against an id that is
    /// not on screen — which is the one thing that check exists to get right.
    #[test]
    fn a_mutation_on_a_job_nobody_has_open_re_reads_no_history() {
        let (mut h, server) = Harness::connected(happy);
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
            kill(ctx, "nightly");
        });
        h.settle();
        assert_eq!(server.count("sessions/list"), 0);
        h.with(|ctx| {
            assert_eq!(
                ctx.scheduler.history_of.peek().as_deref(),
                None,
                "the closed detail's identity check was re-armed against a job \
                 that is not on screen"
            );
        });
    }

    // ---------------------------------------------------------- the run-now

    /// The dot and the toast happen *before* the await, because the request
    /// does not answer until the run is over — a whole agent turn. And a
    /// second tap while the first is out must start nothing: `run-now` is not
    /// idempotent on the server, so it would be a second run.
    #[test]
    fn run_now_lights_the_dot_at_once_and_a_second_tap_starts_nothing() {
        fn slow_run(method: &str, params: &Value) -> Reply {
            if short(method) == "run-now" {
                return (
                    Duration::from_millis(60),
                    Ok(json!({ "status": "completed", "sessionId": "run-9" })),
                );
            }
            happy(method, params)
        }
        let (mut h, server) = Harness::connected(slow_run);
        h.with(|ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
            run_now(ctx, "nightly");
        });
        h.with(|ctx| {
            assert!(
                ctx.scheduler.started_here.peek().contains("nightly"),
                "the dot waits on a request that is minutes away, so the tap \
                 looks like it did nothing"
            );
        });
        assert_eq!(
            h.toast().as_deref(),
            Some("Started — it'll show up in this job's history")
        );

        h.with(|ctx| run_now(ctx, "nightly"));
        h.settle();
        assert_eq!(
            server.count("run-now"),
            1,
            "a second tap started a second run"
        );
        h.with(|ctx| {
            assert!(
                ctx.scheduler.started_here.peek().is_empty(),
                "the advisory flag outlived the request whose life it is"
            );
        });
    }

    /// A cancellation is the one `run-now` outcome worth a sentence: it is the
    /// only one where the thing that was asked for did not happen. A clean
    /// finish says nothing, because the toast would land minutes later, quite
    /// possibly on another screen, to repeat what the new history row says.
    #[test]
    fn only_a_cancelled_run_is_reported_when_it_ends() {
        fn cancels(method: &str, params: &Value) -> Reply {
            if short(method) == "run-now" {
                return ok(json!({ "status": "cancelled", "sessionId": null }));
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(cancels);
        h.with(|ctx| run_now(ctx, "nightly"));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some("That run was cancelled"));

        // A completed one leaves the "Started" toast as the last word.
        let (mut clean, _server) = Harness::connected(happy);
        clean.with(|ctx| run_now(ctx, "nightly"));
        clean.settle();
        assert_eq!(
            clean.toast().as_deref(),
            Some("Started — it'll show up in this job's history"),
            "a run that finished cleanly toasted minutes after the tap, quite \
             possibly on another screen"
        );
    }

    /// A dead socket says nothing, and that silence is deliberate: this client
    /// stopped listening and the job did not stop running, so "Run failed"
    /// would be a false statement about a run that is going fine. The
    /// connection badge already reports the part that did fail.
    #[test]
    fn a_socket_that_dies_under_a_run_is_not_reported_as_a_failed_run() {
        fn hangs_up(method: &str, params: &Value) -> Reply {
            if short(method) == "run-now" {
                return hang_up();
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(hangs_up);
        h.with(|ctx| run_now(ctx, "nightly"));
        h.settle();
        assert_eq!(
            h.toast().as_deref(),
            Some("Started — it'll show up in this job's history"),
            "a socket this client lost was reported as a run that failed"
        );
        h.with(|ctx| {
            assert!(
                ctx.scheduler.started_here.peek().is_empty(),
                "a request that died on a dead socket pinned the dot on forever"
            );
        });
    }

    /// The failure that *is* the run's: goose refused it. That one gets a
    /// sentence, because nothing else on the screen will ever show a run that
    /// never started.
    #[test]
    fn a_run_goose_refused_is_reported() {
        fn refuses_run(method: &str, params: &Value) -> Reply {
            if short(method) == "run-now" {
                return rpc_error(-32002, "no such schedule");
            }
            happy(method, params)
        }
        let (mut h, _server) = Harness::connected(refuses_run);
        h.with(|ctx| run_now(ctx, "nightly"));
        h.settle();
        assert_eq!(h.toast().as_deref(), Some("Run failed: no such schedule"));
    }
}
