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
pub(crate) fn open(ctx: &AppCtx, id: &str) {
    let (mut open, mut screen) = (ctx.scheduler.open, ctx.scheduler.screen);
    open.set(Some(id.to_owned()));
    screen.set(Screen::Detail);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

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
}
