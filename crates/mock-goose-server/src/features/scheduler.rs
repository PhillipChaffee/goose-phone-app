//! The scheduler: `_goose/unstable/schedules/*`, backed by four canned jobs
//! that never touch a `schedule.json`.
//!
//! # Exact spellings only
//!
//! Seven of the eight methods read `scheduleId`; `running-job/kill` reads
//! `jobId`, for the same value. That asymmetry is goose's, and a mock generous
//! enough to answer to either would sign off a client that then fails against
//! the real server — which is the entire bug this crate exists to prevent.
//! Unknown keys are *ignored* rather than rejected, because that is what serde
//! does on the real server: a request whose only id is spelled the other way
//! looks, to goose, like a request with no id at all.
//!
//! # What is not answered
//!
//! `schedules/create` and `schedules/running-job/inspect` return `None` and
//! fall through to [`super::dispatch`]'s `-32601`. The app sends neither —
//! `create` needs a whole recipe body, and `inspect` returns nothing that is
//! not already on the list row — and answering a method nothing sends would be
//! inventing a contract.
//!
//! # Absent, not null
//!
//! goose puts `skip_serializing_if = "Option::is_none"` on `lastRun`,
//! `currentSessionId` and `jobStartTime`, so a job that has never run arrives
//! with those keys *missing*. This omits them for the same reason the skills
//! mock omits `supportingFiles`: it is the common shape, and it is the one
//! shape the client's round-trip fixtures structurally cannot cover.
//!
//! # Time, because the mock cannot block
//!
//! `schedules/run-now` does not answer on the real server until the run has
//! finished. [`super::dispatch`] runs inside the connection's read loop — only
//! `session/prompt` gets a `tokio::spawn` — so a handler that slept would
//! block the whole socket, and every other request behind it.
//!
//! So run-now answers immediately and stamps a *wall clock*: the job reports
//! `currentlyRunning: true` until it passes. That is not a lie about the
//! server so much as a real transient of it — the scheduler's own bookkeeping
//! clears `currently_running` after the run resolves, so a run that finished
//! quickly does list as running for a moment. Stretching that moment to
//! [`RUN_WINDOW`] is what makes the busy dot, the "running 4m" copy and the
//! app's five-second polling cadence reachable on a Mac.
//!
//! # State
//!
//! The store is a process-wide [`LazyLock`], like the recipes one and for the
//! same reason: schedules are a file on the server's disk, so a pause set over
//! one socket is visible over the next. Keeping it here also keeps the whole
//! feature to one file.

use std::sync::{LazyLock, Mutex};

use serde_json::{json, Value};

use crate::rpc::Out;
use crate::state::{now_epoch, stamp, Fixtures, Kind, Shared, State};

use super::{scheduler_disabled, Handled};

/// Cheap gate so the locks below are only taken for a method that could be
/// ours: `handle` sits on the path of every request `core` did not answer.
const PREFIX: &str = "_goose/unstable/schedules/";

const LIST: &str = "_goose/unstable/schedules/list";
const UPDATE: &str = "_goose/unstable/schedules/update";
const DELETE: &str = "_goose/unstable/schedules/delete";
const PAUSE: &str = "_goose/unstable/schedules/pause";
const UNPAUSE: &str = "_goose/unstable/schedules/unpause";
const RUN_NOW: &str = "_goose/unstable/schedules/run-now";
const SESSIONS: &str = "_goose/unstable/schedules/sessions/list";
const KILL: &str = "_goose/unstable/schedules/running-job/kill";

/// The eight methods this mock owns. Anything else under [`PREFIX`] falls
/// through to the `-32601`, which is what the app reads as "not offered".
const OWNED: [&str; 8] = [
    LIST, UPDATE, DELETE, PAUSE, UNPAUSE, RUN_NOW, SESSIONS, KILL,
];

/// How long a job keeps reporting itself as running after `run-now`.
///
/// Long enough to see the dot go amber, watch a poll or two land, and press
/// Kill; short enough that a fixture run does not sit busy forever. The app's
/// busy cadence is five seconds, so this is four ticks.
const RUN_WINDOW: i64 = 20;

/// The seeded job that arrives already running, so the busy state exists
/// without anybody having to press anything.
const SEEDED_RUN_WINDOW: i64 = 30 * 60;

static SCHEDULES: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::seeded()));

pub(crate) fn handle(method: &str, params: &Value, state: &Shared, _out: &Out) -> Handled {
    if !method.starts_with(PREFIX) {
        return None;
    }
    let (fixtures, no_scheduler, runs) = {
        let state = state.lock().unwrap();
        // Every scheduled session the mock holds, newest first — the pool a
        // job's history is drawn from. Read here, under the one lock, so
        // `answer` below is over plain data and a unit test can drive it.
        (state.fixtures, state.no_scheduler, scheduled_runs(&state))
    };
    answer(
        method,
        params,
        fixtures,
        no_scheduler,
        &runs,
        &mut SCHEDULES.lock().unwrap(),
    )
}

/// The dispatch, over plain data so the tests can drive it without a socket or
/// the process-wide store.
fn answer(
    method: &str,
    params: &Value,
    fixtures: Fixtures,
    no_scheduler: bool,
    runs: &[Run],
    store: &mut Store,
) -> Handled {
    if !OWNED.contains(&method) {
        return None;
    }
    // `require_scheduler` fires before the request's id is resolved and before
    // its cron is parsed, so a scheduler-less server refuses every method in
    // the namespace whatever the params say.
    if no_scheduler {
        return Some(Err(scheduler_disabled()));
    }
    let now = now_epoch();
    store.reap(now);

    let result = match method {
        LIST => store.list(fixtures, runs, now),
        UPDATE => match (
            required_str(params, "scheduleId"),
            required_str(params, "cron"),
        ) {
            (Ok(id), Ok(cron)) => store.update(id, cron, runs, now),
            (Err(bad), _) | (_, Err(bad)) => Err(bad),
        },
        DELETE => required_str(params, "scheduleId").and_then(|id| store.delete(id)),
        PAUSE => required_str(params, "scheduleId").and_then(|id| store.set_paused(id, true)),
        UNPAUSE => required_str(params, "scheduleId").and_then(|id| store.set_paused(id, false)),
        RUN_NOW => required_str(params, "scheduleId").and_then(|id| store.run_now(id, runs, now)),
        // `jobId`, and only `jobId` — see the module doc.
        KILL => required_str(params, "jobId").and_then(|id| store.kill(id)),
        SESSIONS => match (
            required_str(params, "scheduleId"),
            required_u64(params, "limit"),
        ) {
            (Ok(id), Ok(limit)) => store.sessions(id, limit, runs),
            (Err(bad), _) | (_, Err(bad)) => Err(bad),
        },
        _ => return None,
    };
    Some(result)
}

// ------------------------------------------------------------------ requests

/// A string field goose marks `required` in `acp-schema.json`.
fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, (i64, String)> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, format!("`{key}` is required and must be a string")))
}

/// `limit` on `schedules/sessions/list`, which goose marks required — not an
/// optional page size with a default. A client that omits it gets a `-32602`
/// here for the same reason it would there.
fn required_u64(params: &Value, key: &str) -> Result<usize, (i64, String)> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| {
            (
                -32602,
                format!("`{key}` is required and must be an integer"),
            )
        })
}

// -------------------------------------------------------------------- errors

/// What goose answers for a schedule id nothing matches: ACP's
/// resource-not-found, from `schedule_not_found_or_internal`.
///
/// The real server puts `{"uri": id}` in `data` and this frame builder carries
/// only strings, so the *code* is the part the two can agree on — which is why
/// the wire test asserts on -32002 and never on the sentence.
fn not_found(id: &str) -> (i64, String) {
    (-32002, format!("schedule not found: {id}"))
}

// ---------------------------------------------------------------- the store

/// One scheduled job. A struct rather than the recipes mock's raw [`Value`]
/// because every field of `ScheduledJobDto` is modelled by the client, so
/// there is no unmodelled half to preserve — and because half of them are
/// mutated by the seven methods that are not `list`.
#[derive(Debug, Clone)]
struct Job {
    id: String,
    source: String,
    cron: String,
    last_run: Option<String>,
    paused: bool,
    /// Which seeded session is this job's history, by title. `None` for a job
    /// that has never produced one.
    session_title: Option<&'static str>,
    /// Epoch second at which the scheduler's bookkeeping clears the run, or
    /// `None` when nothing is running.
    running_until: Option<i64>,
    started_at: Option<String>,
}

impl Job {
    /// The wire shape, with the three nullable keys omitted rather than sent
    /// as `null` — see the module doc.
    fn dto(&self, runs: &[Run]) -> Value {
        let running = self.running_until.is_some();
        let mut dto = json!({
            "id": self.id,
            "source": self.source,
            "cron": self.cron,
            "currentlyRunning": running,
            "paused": self.paused,
        });
        if let Some(last_run) = &self.last_run {
            dto["lastRun"] = Value::String(last_run.clone());
        }
        if let Some(started) = &self.started_at {
            dto["jobStartTime"] = Value::String(started.clone());
        }
        if running {
            if let Some(session) = self.newest_run(runs) {
                dto["currentSessionId"] = Value::String(session.id.clone());
            }
        }
        dto
    }

    fn newest_run<'a>(&self, runs: &'a [Run]) -> Option<&'a Run> {
        let title = self.session_title?;
        runs.iter().find(|run| run.title == title)
    }
}

#[derive(Debug)]
struct Store {
    jobs: Vec<Job>,
    /// Whether the `broken` fixture set has already spent its one failure.
    /// The point of that switch is the app's error-then-retry path, and a
    /// server that failed forever would only ever prove the first half of it.
    broken_list_served: bool,
}

impl Store {
    fn seeded() -> Self {
        Self {
            jobs: seed(now_epoch()),
            broken_list_served: false,
        }
    }

    /// Clear the runs whose window has passed, exactly as the scheduler's own
    /// bookkeeping would, and stamp their `lastRun` on the way out.
    fn reap(&mut self, now: i64) {
        for job in &mut self.jobs {
            if job.running_until.is_some_and(|until| now >= until) {
                job.running_until = None;
                job.started_at = None;
                job.last_run = Some(stamp(now).rfc3339);
            }
        }
    }

    fn find(&mut self, id: &str) -> Result<&mut Job, (i64, String)> {
        self.jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| not_found(id))
    }

    fn list(
        &mut self,
        fixtures: Fixtures,
        runs: &[Run],
        _now: i64,
    ) -> Result<Value, (i64, String)> {
        match fixtures {
            Fixtures::Empty => return Ok(json!({"jobs": []})),
            Fixtures::Broken if !self.broken_list_served => {
                self.broken_list_served = true;
                return Err((
                    -32603,
                    "Failed to list schedules: could not read \
                     /home/demo/.local/share/goose/schedule.json"
                        .to_string(),
                ));
            }
            Fixtures::Full | Fixtures::Broken => {}
        }
        let jobs: Vec<Value> = self.jobs.iter().map(|job| job.dto(runs)).collect();
        Ok(json!({ "jobs": jobs }))
    }

    /// goose re-lists internally and hands back the stored job, so a client
    /// can prove the cadence actually changed without a second call.
    fn update(
        &mut self,
        id: &str,
        cron: &str,
        runs: &[Run],
        _now: i64,
    ) -> Result<Value, (i64, String)> {
        let job = self.find(id)?;
        job.cron = cron.to_string();
        Ok(json!({ "job": job.dto(runs) }))
    }

    fn delete(&mut self, id: &str) -> Result<Value, (i64, String)> {
        let before = self.jobs.len();
        self.jobs.retain(|job| job.id != id);
        if self.jobs.len() == before {
            return Err(not_found(id));
        }
        Ok(json!({}))
    }

    fn set_paused(&mut self, id: &str, paused: bool) -> Result<Value, (i64, String)> {
        self.find(id)?.paused = paused;
        Ok(json!({}))
    }

    fn run_now(&mut self, id: &str, runs: &[Run], now: i64) -> Result<Value, (i64, String)> {
        let job = self.find(id)?;
        job.running_until = Some(now + RUN_WINDOW);
        job.started_at = Some(stamp(now).rfc3339);
        let session = job.newest_run(runs).map(|run| run.id.clone());
        // `completed` immediately, with the job still listing as running: the
        // module doc says why the mock cannot do it the other way round.
        let mut reply = json!({ "status": "completed" });
        if let Some(session) = session {
            reply["sessionId"] = Value::String(session);
        }
        Ok(reply)
    }

    fn kill(&mut self, id: &str) -> Result<Value, (i64, String)> {
        let job = self.find(id)?;
        if job.running_until.is_none() {
            // goose's `schedule_state_error` maps this to invalid_params, not
            // to a not-found: the job exists, it simply is not running.
            return Err((-32602, format!("job '{id}' is not running")));
        }
        job.running_until = None;
        job.started_at = None;
        Ok(json!({"message": format!("Successfully killed running job '{id}'")}))
    }

    fn sessions(&mut self, id: &str, limit: usize, runs: &[Run]) -> Result<Value, (i64, String)> {
        let job = self.find(id)?;
        let title = job.session_title;
        let sessions: Vec<Value> = runs
            .iter()
            .filter(|run| Some(run.title.as_str()) == title)
            .take(limit)
            .map(|run| run.info.clone())
            .collect();
        Ok(json!({ "sessions": sessions }))
    }
}

// ------------------------------------------------------------- run history

/// One session the scheduler produced, ready to hand back.
#[derive(Debug, Clone)]
struct Run {
    title: String,
    id: String,
    /// The `SessionInfo` shape, built once so `sessions/list` is a filter.
    info: Value,
}

/// Every `Kind::Scheduled` session the mock holds, newest first.
///
/// Resolved against the live store at request time rather than baked into the
/// fixtures, so a history row opens onto a transcript `session/load` will
/// actually replay — and so nothing in `state.rs` has to move, which several
/// `core.rs` pagination tests count on.
fn scheduled_runs(state: &State) -> Vec<Run> {
    let mut runs: Vec<Run> = state
        .sessions
        .iter()
        .filter(|(_, data)| data.kind == Kind::Scheduled)
        .map(|(id, data)| Run {
            title: data.title.clone(),
            id: id.clone(),
            // `build_session_info` on the real server always attaches the full
            // `session_meta`, snippet included — unlike `session/list`, which
            // gates the snippet behind `includeLastMessageSnippet`. So this
            // listing carries more than a chats page does, and the app renders
            // both with the same row.
            info: json!({
                "sessionId": id,
                "cwd": data.cwd,
                "additionalDirectories": [],
                "title": data.title,
                "updatedAt": data.updated_at,
                "_meta": {
                    "messageCount": data.message_count,
                    "createdAt": data.created_at,
                    "lastMessageAt": data.sort_at,
                    "userSetName": data.user_set_name,
                    "sessionType": data.kind.as_wire(),
                    "hasRecipe": true,
                    "lastMessageSnippet": data.snippet,
                },
            }),
        })
        .collect();
    // A HashMap has no order, and "newest first" is what the history claims.
    // The timestamps are RFC 3339 with a fixed offset, so lexicographic order
    // is chronological order.
    runs.sort_by(|a, b| {
        b.info["updatedAt"]
            .as_str()
            .cmp(&a.info["updatedAt"].as_str())
    });
    runs
}

// -------------------------------------------------------------------- fixtures

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

const SCHEDULED_DIR: &str = "/home/demo/.config/goose/scheduled-recipes";

/// The long one: a job id is a recipe file stem, and the row humanises it, so
/// this is the width the list row and the detail's top bar have to survive.
/// `docs/audit.js` stress-substitutes the longest text it finds, and short
/// fixtures give it nothing to bite on.
const LONG_ID: &str =
    "quarterly-dependency-licence-and-security-audit-across-every-workspace-crate";

/// Four jobs, chosen so every state the two screens draw is reachable without
/// editing this file:
///
/// - `nightly-dependency-audit` arrives **already running**, with a start time
///   four minutes ago — the busy dot, the "running 4m" copy, the Kill button
///   and "Watch it run" all have a subject the moment the mock starts. Its
///   history is a real seeded session, so the button opens a transcript that
///   replays.
/// - `weekly-changelog-digest` is **paused**: the off dot, and the banner on
///   its detail. It has a history too.
/// - [`LONG_ID`] has **never run** — `lastRun` absent rather than null — and is
///   the stress fixture.
/// - `staging-smoke-tests-during-office-hours` carries a cron this app's
///   grammar cannot hold, so the row reads "Runs on a schedule" and the
///   cadence row degrades to a fact instead of opening a sheet that would
///   rewrite it.
///
/// Dated against the moment the process started rather than a date written
/// into the file: a hardcoded timestamp reads "20 Aug" three weeks later, and
/// the relative-age copy is then never exercised by the thing it exists for.
fn seed(now: i64) -> Vec<Job> {
    vec![
        Job {
            id: "nightly-dependency-audit".to_string(),
            source: format!("{SCHEDULED_DIR}/nightly-dependency-audit.yaml"),
            cron: "0 30 2 * * *".to_string(),
            last_run: Some(stamp(now - 4 * MINUTE).rfc3339),
            paused: false,
            session_title: Some("Nightly dependency audit"),
            running_until: Some(now + SEEDED_RUN_WINDOW),
            started_at: Some(stamp(now - 4 * MINUTE).rfc3339),
        },
        Job {
            id: "weekly-changelog-digest".to_string(),
            source: format!("{SCHEDULED_DIR}/weekly-changelog-digest.yaml"),
            cron: "0 0 9 * * 1".to_string(),
            last_run: Some(stamp(now - 6 * DAY).rfc3339),
            paused: true,
            session_title: Some("Weekly changelog digest"),
            running_until: None,
            started_at: None,
        },
        Job {
            id: LONG_ID.to_string(),
            source: format!("{SCHEDULED_DIR}/{LONG_ID}.yaml"),
            cron: "0 0 6 1 * *".to_string(),
            last_run: None,
            paused: false,
            session_title: None,
            running_until: None,
            started_at: None,
        },
        Job {
            id: "staging-smoke-tests-during-office-hours".to_string(),
            source: format!("{SCHEDULED_DIR}/staging-smoke-tests-during-office-hours.yaml"),
            // Five-field, and a step and a range at that: legal cron that
            // `src/cron.rs` refuses rather than approximating.
            cron: "*/15 9-17 * * 1-5".to_string(),
            last_run: Some(stamp(now - 40 * MINUTE).rfc3339),
            paused: false,
            session_title: None,
            running_until: None,
            started_at: None,
        },
    ]
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test fixtures: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;

    const RUNNING: &str = "nightly-dependency-audit";
    const PAUSED: &str = "weekly-changelog-digest";
    const ODD_CRON: &str = "staging-smoke-tests-during-office-hours";

    const NOW: i64 = 1_800_000_000;

    fn store() -> Store {
        Store {
            jobs: seed(NOW),
            broken_list_served: false,
        }
    }

    fn runs() -> Vec<Run> {
        vec![
            Run {
                title: "Nightly dependency audit".to_string(),
                id: "20260825_4".to_string(),
                info: json!({"sessionId": "20260825_4", "updatedAt": "2026-08-25T09:00:00Z"}),
            },
            Run {
                title: "Weekly changelog digest".to_string(),
                id: "20260819_1".to_string(),
                info: json!({"sessionId": "20260819_1", "updatedAt": "2026-08-19T09:00:00Z"}),
            },
        ]
    }

    /// Every handler test goes through `answer`, so the method strings are
    /// exercised rather than the functions behind them.
    fn call(store: &mut Store, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        answer(method, params, Fixtures::Full, false, &runs(), store).unwrap()
    }

    fn listed(store: &mut Store) -> Vec<Value> {
        call(store, LIST, &json!({}))
            .unwrap()
            .get("jobs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap()
    }

    fn job_of(store: &mut Store, id: &str) -> Value {
        listed(store)
            .into_iter()
            .find(|job| job["id"] == json!(id))
            .unwrap()
    }

    /// The keys of a list entry, spelled out against `ScheduledJobDto`. A
    /// fixture that drifted to `last_run` would still deserialize on the
    /// client — into a `None` and an `extra` entry nobody looks at — so the
    /// spelling has to be asserted somewhere, and this is the somewhere.
    #[test]
    fn a_job_carries_exactly_the_keys_goose_sends() {
        for job in listed(&mut store()) {
            let mut keys: Vec<&str> = job
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            for key in &keys {
                assert!(
                    [
                        "id",
                        "source",
                        "cron",
                        "lastRun",
                        "currentlyRunning",
                        "paused",
                        "currentSessionId",
                        "jobStartTime",
                    ]
                    .contains(key),
                    "unknown key `{key}` on {}",
                    job["id"]
                );
            }
            for required in ["id", "source", "cron", "currentlyRunning", "paused"] {
                assert!(keys.contains(&required), "{required} missing from {job}");
            }
        }
    }

    /// goose `skip_serializing_if`s the three nullable fields, so a job that
    /// has never run sends no `lastRun` key at all. The client's round-trip
    /// fixtures cannot cover that shape; this is where it is covered.
    #[test]
    fn a_job_that_never_ran_omits_the_key_rather_than_nulling_it() {
        let never = job_of(&mut store(), LONG_ID);
        assert!(never.get("lastRun").is_none(), "{never}");
        assert!(never.get("jobStartTime").is_none(), "{never}");
        assert!(never.get("currentSessionId").is_none(), "{never}");
        assert_eq!(never["currentlyRunning"], json!(false));
    }

    /// The state the whole screen is built around, present from startup so
    /// nobody has to press anything to see it.
    #[test]
    fn one_job_arrives_already_running_with_a_session_to_watch() {
        let running = job_of(&mut store(), RUNNING);
        assert_eq!(running["currentlyRunning"], json!(true));
        assert_eq!(running["currentSessionId"], json!("20260825_4"));
        assert!(running["jobStartTime"].is_string());

        let paused = job_of(&mut store(), PAUSED);
        assert_eq!(paused["paused"], json!(true));
        assert_eq!(paused["currentlyRunning"], json!(false));
        // Not running, so no session to watch even though it has a history.
        assert!(paused.get("currentSessionId").is_none());
    }

    #[test]
    fn pausing_and_resuming_change_what_the_next_list_says() {
        let mut store = store();
        assert_eq!(
            call(&mut store, PAUSE, &json!({"scheduleId": ODD_CRON})).unwrap(),
            json!({})
        );
        assert_eq!(job_of(&mut store, ODD_CRON)["paused"], json!(true));

        call(&mut store, UNPAUSE, &json!({"scheduleId": ODD_CRON})).unwrap();
        assert_eq!(job_of(&mut store, ODD_CRON)["paused"], json!(false));
    }

    /// The reply is the stored job, not an echo — goose re-lists to build it,
    /// so a client can prove the cadence changed without a second call.
    #[test]
    fn update_answers_with_the_job_as_it_now_stands() {
        let mut store = store();
        let params = json!({"scheduleId": PAUSED, "cron": "0 0 7 * * 2"});
        let reply = call(&mut store, UPDATE, &params).unwrap();
        assert_eq!(reply["job"]["cron"], json!("0 0 7 * * 2"));
        assert_eq!(reply["job"]["id"], json!(PAUSED));
        assert_eq!(job_of(&mut store, PAUSED)["cron"], json!("0 0 7 * * 2"));
    }

    #[test]
    fn update_without_a_cron_is_invalid_params() {
        let mut store = store();
        let (code, reason) = call(&mut store, UPDATE, &json!({"scheduleId": PAUSED})).unwrap_err();
        assert_eq!(code, -32602);
        assert!(reason.contains("cron"), "{reason}");
    }

    #[test]
    fn delete_removes_it_from_the_list() {
        let mut store = store();
        assert_eq!(
            call(&mut store, DELETE, &json!({"scheduleId": ODD_CRON})).unwrap(),
            json!({})
        );
        assert!(!listed(&mut store)
            .iter()
            .any(|job| job["id"] == json!(ODD_CRON)));
    }

    /// run-now marks the job running and hands back the session its history
    /// will show — the two halves of "started, and here is where to look".
    #[test]
    fn run_now_starts_the_job_and_names_a_session() {
        let mut store = store();
        // Not running to begin with, and no history of its own.
        assert_eq!(
            job_of(&mut store, ODD_CRON)["currentlyRunning"],
            json!(false)
        );

        let reply = call(&mut store, RUN_NOW, &json!({"scheduleId": PAUSED})).unwrap();
        assert_eq!(reply["status"], json!("completed"));
        assert_eq!(reply["sessionId"], json!("20260819_1"));
        let job = job_of(&mut store, PAUSED);
        assert_eq!(job["currentlyRunning"], json!(true));
        assert_eq!(job["currentSessionId"], json!("20260819_1"));
    }

    /// The scheduler's bookkeeping clears the run when the window passes,
    /// which is what makes the app's busy cadence drop back to idle on its own
    /// rather than needing a Kill.
    #[test]
    fn a_run_clears_itself_once_its_window_has_passed() {
        let mut store = store();
        store.run_now(PAUSED, &runs(), NOW).unwrap();
        assert!(store.find(PAUSED).unwrap().running_until.is_some());

        store.reap(NOW + RUN_WINDOW);
        let job = store.find(PAUSED).unwrap().clone();
        assert!(job.running_until.is_none());
        assert!(job.started_at.is_none());
        assert!(
            job.last_run.is_some(),
            "a finished run has to stamp lastRun"
        );
    }

    /// goose spells this one `jobId` while every other method in the namespace
    /// says `scheduleId`. A mock that also answered to `scheduleId` would let
    /// a client that sends the wrong key pass here and fail against the real
    /// server — which is the entire bug this design exists to prevent.
    #[test]
    fn kill_answers_to_job_id_and_to_nothing_else() {
        let mut store = store();
        let (code, _) = call(&mut store, KILL, &json!({"scheduleId": RUNNING})).unwrap_err();
        assert_eq!(code, -32602);
        assert_eq!(job_of(&mut store, RUNNING)["currentlyRunning"], json!(true));

        let reply = call(&mut store, KILL, &json!({"jobId": RUNNING})).unwrap();
        assert!(reply["message"].as_str().unwrap().contains(RUNNING));
        assert_eq!(
            job_of(&mut store, RUNNING)["currentlyRunning"],
            json!(false)
        );
    }

    /// The job exists, it simply is not running — goose's own mapping makes
    /// that `invalid_params` rather than a not-found, and the difference is what
    /// tells the app whether the row it swiped is gone.
    #[test]
    fn killing_something_that_is_not_running_is_invalid_params() {
        let mut store = store();
        let (code, _) = call(&mut store, KILL, &json!({"jobId": PAUSED})).unwrap_err();
        assert_eq!(code, -32602);
    }

    #[test]
    fn the_history_is_the_jobs_own_sessions_and_nobody_elses() {
        let mut store = store();
        let params = json!({"scheduleId": RUNNING, "limit": 20});
        let sessions = call(&mut store, SESSIONS, &params).unwrap();
        assert_eq!(sessions["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(sessions["sessions"][0]["sessionId"], json!("20260825_4"));

        // A job with no history gets an empty list, not somebody else's.
        let params = json!({"scheduleId": LONG_ID, "limit": 20});
        let sessions = call(&mut store, SESSIONS, &params).unwrap();
        assert_eq!(sessions["sessions"], json!([]));
    }

    #[test]
    fn the_history_honours_its_limit_and_demands_one() {
        let mut store = store();
        let params = json!({"scheduleId": RUNNING, "limit": 0});
        assert_eq!(
            call(&mut store, SESSIONS, &params).unwrap()["sessions"],
            json!([])
        );

        let (code, reason) =
            call(&mut store, SESSIONS, &json!({"scheduleId": RUNNING})).unwrap_err();
        assert_eq!(code, -32602);
        assert!(reason.contains("limit"), "{reason}");
    }

    /// -32002 is ACP's resource-not-found, which is how the app tells "this
    /// feature is off" (-32601) from "that particular job is gone".
    #[test]
    fn an_unknown_id_is_resource_not_found_on_every_method_that_takes_one() {
        let mut store = store();
        for (method, params) in [
            (UPDATE, json!({"scheduleId": "nope", "cron": "0 0 9 * * *"})),
            (DELETE, json!({"scheduleId": "nope"})),
            (PAUSE, json!({"scheduleId": "nope"})),
            (UNPAUSE, json!({"scheduleId": "nope"})),
            (RUN_NOW, json!({"scheduleId": "nope"})),
            (SESSIONS, json!({"scheduleId": "nope", "limit": 20})),
            (KILL, json!({"jobId": "nope"})),
        ] {
            let (code, _) = call(&mut store, method, &params).unwrap_err();
            assert_eq!(code, -32002, "{method} answered the wrong code");
        }
    }

    /// `MOCK_NO_SCHEDULER=1`: the whole namespace refuses, with goose's own
    /// sentence, before any id or cron is looked at. `-32601` and not an error
    /// code of its own, because that pair is what the client turns into
    /// `Unsupported` and the screen turns into a sentence with no Retry.
    #[test]
    fn a_scheduler_less_server_refuses_every_method_in_the_namespace() {
        let mut store = store();
        for method in OWNED {
            let (code, reason) = answer(
                method,
                &json!({}),
                Fixtures::Full,
                true,
                &runs(),
                &mut store,
            )
            .unwrap()
            .unwrap_err();
            assert_eq!(code, -32601, "{method}");
            assert_eq!(reason, "Scheduled recipe execution is not enabled");
        }
    }

    #[test]
    fn empty_fixtures_serve_no_jobs() {
        let mut store = store();
        let result = answer(
            LIST,
            &json!({}),
            Fixtures::Empty,
            false,
            &runs(),
            &mut store,
        )
        .unwrap();
        assert_eq!(result.unwrap(), json!({"jobs": []}));
    }

    /// The error-then-retry path needs both halves reachable from one process,
    /// so the failure is spent on the first call and the second one works.
    /// It is also the only place the client's per-method `-32601` cache can be
    /// shown *not* to swallow an ordinary failure.
    #[test]
    fn broken_fixtures_fail_once_and_then_behave() {
        let mut store = store();
        let (code, reason) = answer(
            LIST,
            &json!({}),
            Fixtures::Broken,
            false,
            &runs(),
            &mut store,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(code, -32603);
        assert!(reason.contains("schedule"), "unhelpful reason: {reason}");

        let recovered = answer(
            LIST,
            &json!({}),
            Fixtures::Broken,
            false,
            &runs(),
            &mut store,
        )
        .unwrap()
        .unwrap();
        assert_eq!(recovered["jobs"].as_array().unwrap().len(), 4);
    }

    /// A `schedules/*` method this mock does not implement has to fall through
    /// to `dispatch`'s `-32601`, which is how the client learns to hide a
    /// control rather than show a failure. Both of these are methods goose
    /// really has and this app really never sends.
    #[test]
    fn the_two_methods_nothing_sends_are_left_to_the_method_not_found() {
        let mut store = store();
        for method in [
            "_goose/unstable/schedules/create",
            "_goose/unstable/schedules/running-job/inspect",
        ] {
            assert!(
                answer(
                    method,
                    &json!({}),
                    Fixtures::Full,
                    false,
                    &runs(),
                    &mut store
                )
                .is_none(),
                "{method} was answered"
            );
            // And still unanswered with the scheduler switched off, so the two
            // modes cannot disagree about which methods exist.
            assert!(
                answer(
                    method,
                    &json!({}),
                    Fixtures::Full,
                    true,
                    &runs(),
                    &mut store
                )
                .is_none(),
                "{method} was answered with the scheduler off"
            );
        }
    }

    /// The history is drawn from the live session store, so it resolves to ids
    /// `session/load` will replay rather than to fixtures of its own.
    #[test]
    fn the_run_pool_is_the_scheduled_sessions_newest_first() {
        let state: Shared = std::sync::Arc::new(Mutex::new(State::default()));
        crate::state::seed(&state);
        let runs = scheduled_runs(&state.lock().unwrap());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].title, "Nightly dependency audit");
        assert_eq!(runs[1].title, "Weekly changelog digest");
        assert!(runs[0].info["_meta"]["lastMessageSnippet"].is_string());
        assert_eq!(runs[0].info["_meta"]["sessionType"], json!("scheduled"));
    }
}
