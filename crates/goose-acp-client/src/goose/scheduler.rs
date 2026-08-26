//! Scheduler: `_goose/unstable/schedules/*` — the recipes goose runs on a
//! cron, and what a phone does about one that is misbehaving.
//!
//! # Casing
//!
//! Uniformly camelCase, requests included — which is not the same statement
//! as `rename_all`, and this module's ban on the blanket attribute still
//! holds. goose puts `#[serde(rename_all = "camelCase")]` on every type in
//! `goose-sdk-types/src/custom_requests/schedule.rs`, so each two-word field
//! here carries its own rename and the single-word ones carry none.
//!
//! Worth seeing beside its neighbour: `RecipeListEntry` is `snake_case` on the
//! same server, so `recipes/schedule` takes `cron_schedule` while
//! `schedules/update` takes `cron`. Two spellings of one idea, one method
//! apart, and neither server rejects the other's.
//!
//! # What is not here
//!
//! `schedules/create` needs a whole `RecipeDto` — that is authoring a recipe,
//! which this app does not do. Schedules are born on the Recipes detail via
//! `recipes/schedule`, which produces a real job in this same list.
//!
//! `schedules/running-job/inspect` returns nothing that is not already in
//! [`ScheduledJob`] or derivable from [`ScheduledJob::job_start_time`]. A
//! button whose result repeats the row it was pressed on is design rule 11's
//! exact target.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{LIST_TIMEOUT, MUTATE_TIMEOUT};
use crate::client::AcpClient;
use crate::error::AcpError;
use crate::types::SessionInfo;

const LIST: &str = "_goose/unstable/schedules/list";
const UPDATE: &str = "_goose/unstable/schedules/update";
const DELETE: &str = "_goose/unstable/schedules/delete";
const PAUSE: &str = "_goose/unstable/schedules/pause";
const UNPAUSE: &str = "_goose/unstable/schedules/unpause";
const RUN_NOW: &str = "_goose/unstable/schedules/run-now";
const SESSIONS: &str = "_goose/unstable/schedules/sessions/list";
const KILL: &str = "_goose/unstable/schedules/running-job/kill";

/// How long a `run-now` may stay pending.
///
/// `schedules/run-now` does not answer until the run is **over**:
/// `on_run_schedule_now` awaits `scheduler.run_now(&id)` and only then
/// resolves `Completed` or `Cancelled`, so the response time is a whole agent
/// turn. Six hours is therefore not a deadline anybody is expected to reach —
/// it exists so a socket that dies quietly cannot leak a pending request for
/// the life of the process.
///
/// It is safe to be wrong about, which is why it is a plain number rather
/// than `Duration::MAX` and a paragraph about how `tokio::time::timeout`
/// handles an overflowing deadline. Nothing waits on this call: the screen
/// polls `schedules/list`, which is authoritative, and
/// [`AcpError::Timeout`] here means "we stopped listening", not "the run
/// failed" — so the caller says nothing rather than something untrue.
const RUN_TIMEOUT: Duration = Duration::from_hours(6);

/// One job on the scheduler's list.
///
/// `currently_running` and `paused` carry no `default`: `acp-schema.json`
/// marks both required, and inventing `false` for a field the server stopped
/// sending would paint a running job as idle. The three `Option`s are the
/// three the schema declares nullable, and nothing here is
/// `skip_serializing_if`-ed — see this module's parent for why the round-trip
/// fixtures need the `null`s spelled out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledJob {
    /// The recipe's file stem — `nightly-dependency-audit`. Both the
    /// identifier every mutation takes as `scheduleId` and the only name a
    /// job has, which is why a screen humanises it rather than printing it.
    pub id: String,
    /// Absolute path of the recipe YAML on the server.
    pub source: String,
    pub cron: String,
    #[serde(rename = "lastRun")]
    pub last_run: Option<String>,
    #[serde(rename = "currentlyRunning")]
    pub currently_running: bool,
    pub paused: bool,
    /// The session the run in flight is writing into — what "Watch it run"
    /// opens.
    #[serde(rename = "currentSessionId")]
    pub current_session_id: Option<String>,
    /// RFC3339 start of the run in flight. This is the only source for
    /// "running 4m", which is why `running-job/inspect` is not worth a call.
    #[serde(rename = "jobStartTime")]
    pub job_start_time: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What a job is doing, in the order a reader needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleState {
    Running,
    Paused,
    Scheduled,
}

impl ScheduledJob {
    /// Running outranks paused outranks scheduled.
    ///
    /// The two flags are independent on the server — goose pauses the
    /// *schedule*, not the process, so a job can be flagged paused while a
    /// run it started is still going. What is happening now is what a reader
    /// needs first; a pause is a statement about the next fire.
    #[must_use]
    pub const fn state(&self) -> ScheduleState {
        if self.currently_running {
            ScheduleState::Running
        } else if self.paused {
            ScheduleState::Paused
        } else {
            ScheduleState::Scheduled
        }
    }
}

impl ScheduleState {
    /// The `.dot` modifier this state wears, so no screen maps a state to a
    /// colour of its own.
    ///
    /// Mapping lives here for the reason [`SessionInfo::kind_label`] does:
    /// the crate that knows what the protocol means is the one that should
    /// decide what it looks like, and it is the only place a test can hold
    /// the mapping without mounting anything.
    #[must_use]
    pub const fn dot(self) -> &'static str {
        match self {
            Self::Running => "busy",
            Self::Paused => "off",
            Self::Scheduled => "on",
        }
    }

    /// The bare word for this state. Callers with more to say — how long a
    /// run has been going, what cadence a scheduled job is on — say it around
    /// this rather than instead of it.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Scheduled => "scheduled",
        }
    }
}

/// How a `run-now` ended.
///
/// goose declares a closed two-variant enum under a blanket
/// `rename_all = "camelCase"` whose variants happen to be one word each —
/// the [`crate::SourceType`] accident again, so each variant names its own
/// wire string here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

/// The `schedules/run-now` result, once the run has finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNowResponse {
    pub status: RunStatus,
    /// The session the run wrote into. Absent on a cancellation, which never
    /// got one.
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSchedulesResponse {
    pub jobs: Vec<ScheduledJob>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateScheduleResponse {
    /// The job as goose re-read it after the write. goose builds this by
    /// re-listing internally, so it is the stored value and not an echo of
    /// the request.
    pub job: ScheduledJob,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillRunningJobResponse {
    /// goose's own sentence — "Successfully killed running job 'x'". Parsed
    /// to prove the shape and then dropped: design rule 8 keeps a backend
    /// string off the screen, and the caller has better words.
    pub message: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `schedules/sessions/list` result.
///
/// The one response in this module that is **not** put through
/// [`crate::assert_round_trip`], and the reason is a type from another
/// namespace: its elements are base-ACP [`SessionInfo`], which derives
/// `Deserialize` only — nothing in this crate has ever needed to *send* a
/// session listing — so the round-trip helper cannot be pointed at it.
/// Adding `Serialize` would mean editing `types/`, whose rules are
/// deliberately not this module's.
///
/// `extra` still does its other job, and this type's test still asserts it is
/// empty: a sibling key of `sessions` that nothing here claims would show up
/// there.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListScheduleSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ---------------------------------------------------------------- requests
//
// Free functions over plain data, so every wire spelling is checkable without
// a socket. `tests/contract.rs` checks the same literals against goose's own
// declaration of each method; the unit tests below check that these builders
// produce those literals. Builder == literal here, literal ⊆ goose's keys
// there.

fn list_params() -> Value {
    json!({})
}

fn id_params(schedule_id: &str) -> Value {
    json!({ "scheduleId": schedule_id })
}

fn update_params(schedule_id: &str, cron: &str) -> Value {
    json!({ "scheduleId": schedule_id, "cron": cron })
}

fn sessions_params(schedule_id: &str, limit: u32) -> Value {
    json!({ "scheduleId": schedule_id, "limit": limit })
}

/// `kill` names its argument `jobId`, and it is the *schedule's* id all the
/// same: `on_kill_running_job` hands it straight to `kill_running_job`, and
/// its sibling `on_inspect_running_job` resolves the identical argument with
/// `.find(|job| job.id == req.job_id)`.
///
/// One method in this namespace spelling it differently from the other seven
/// is exactly the kind of thing that passes both sides' unit tests and fails
/// in the app, which is why it has a function and a test of its own.
fn kill_params(job_id: &str) -> Value {
    json!({ "jobId": job_id })
}

/// Which of the two methods a pause toggle sends.
///
/// A function rather than an `if` at the call site so the pair is stated once
/// and pinned by a test: two method strings differing by two letters is
/// precisely the typo the contract test exists for.
const fn pause_method(paused: bool) -> &'static str {
    if paused {
        PAUSE
    } else {
        UNPAUSE
    }
}

/// A response body this crate cannot parse is the server breaking the
/// contract, not a feature being off — `Transport` says that without
/// inventing an error variant for a case the round-trip tests exist to keep
/// from happening.
fn unreadable(method: &str, error: &serde_json::Error) -> AcpError {
    AcpError::Transport(format!("{method} returned {error}"))
}

impl AcpClient {
    /// Every job the scheduler holds.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] on a server started without
    /// `--enable-scheduler` — goose answers `-32601` with "Scheduled recipe
    /// execution is not enabled" in `data` — or one predating the namespace;
    /// otherwise as [`AcpClient::request_with_timeout`].
    pub async fn schedules_list(&self) -> Result<Vec<ScheduledJob>, AcpError> {
        let raw = self
            .goose_request(LIST, list_params(), LIST_TIMEOUT)
            .await?;
        let parsed: ListSchedulesResponse =
            serde_json::from_value(raw).map_err(|e| unreadable(LIST, &e))?;
        Ok(parsed.jobs)
    }

    /// Move a job to a different cadence. Returns the job as goose re-read it.
    ///
    /// Takes only `{scheduleId, cron}`, which is why editing a cadence works
    /// for **every** job — including ones the desktop created from a recipe
    /// this phone could not author.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] when the scheduler is off; [`AcpError::Rpc`]
    /// `-32002` for an id that is gone and `-32602` for a cron goose will not
    /// parse (which the sheet cannot produce); otherwise as
    /// [`AcpClient::request_with_timeout`].
    pub async fn schedules_update(
        &self,
        schedule_id: &str,
        cron: &str,
    ) -> Result<ScheduledJob, AcpError> {
        let raw = self
            .goose_request(UPDATE, update_params(schedule_id, cron), MUTATE_TIMEOUT)
            .await?;
        let parsed: UpdateScheduleResponse =
            serde_json::from_value(raw).map_err(|e| unreadable(UPDATE, &e))?;
        Ok(parsed.job)
    }

    /// Pause or resume, whichever the flag asks for.
    ///
    /// One wrapper for two methods because it is one control: a screen that
    /// picked between `pause` and `unpause` at the call site would be holding
    /// the pair in two places.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] when the scheduler is off; [`AcpError::Rpc`]
    /// `-32002` for an unknown id; otherwise as
    /// [`AcpClient::request_with_timeout`].
    pub async fn schedules_set_paused(
        &self,
        schedule_id: &str,
        paused: bool,
    ) -> Result<(), AcpError> {
        self.goose_request(pause_method(paused), id_params(schedule_id), MUTATE_TIMEOUT)
            .await?;
        Ok(())
    }

    /// Run a job now, out of band.
    ///
    /// **This does not return until the run is over.** Call it from a task
    /// nothing is waiting on: it resolves after a whole agent turn, and a
    /// socket that dies in the meantime leaves the job running on the server,
    /// where the next [`AcpClient::schedules_list`] finds it.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] when the scheduler is off;
    /// [`AcpError::Closed`] or [`AcpError::Timeout`] if this client stops
    /// listening (the run is unaffected either way); [`AcpError::Rpc`]
    /// `-32002` for an unknown id.
    pub async fn schedules_run_now(&self, schedule_id: &str) -> Result<RunNowResponse, AcpError> {
        let raw = self
            .goose_request(RUN_NOW, id_params(schedule_id), RUN_TIMEOUT)
            .await?;
        serde_json::from_value(raw).map_err(|e| unreadable(RUN_NOW, &e))
    }

    /// Stop the run in flight. `schedule_id` is the schedule's own id — see
    /// [`kill_params`] for why the wire key says `jobId`.
    ///
    /// goose's success message is parsed and discarded: the shape is worth
    /// checking, the sentence is not worth showing.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] when the scheduler is off; [`AcpError::Rpc`]
    /// `-32002` for an unknown id and `-32602` when nothing is running.
    pub async fn schedules_kill(&self, schedule_id: &str) -> Result<(), AcpError> {
        let raw = self
            .goose_request(KILL, kill_params(schedule_id), MUTATE_TIMEOUT)
            .await?;
        let _parsed: KillRunningJobResponse =
            serde_json::from_value(raw).map_err(|e| unreadable(KILL, &e))?;
        Ok(())
    }

    /// The job's run history, newest first.
    ///
    /// The entries are base-ACP [`SessionInfo`] — goose builds them with the
    /// same `build_session_info` that answers `session/list` — so the run
    /// history is the type the Chats list already renders and `session/load`
    /// already replays. `limit` is a parameter and never an `Option` because
    /// goose marks it required: a call that omitted it would be `-32602`.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] when the scheduler is off; otherwise as
    /// [`AcpClient::request_with_timeout`].
    pub async fn schedules_sessions(
        &self,
        schedule_id: &str,
        limit: u32,
    ) -> Result<Vec<SessionInfo>, AcpError> {
        let raw = self
            .goose_request(SESSIONS, sessions_params(schedule_id, limit), LIST_TIMEOUT)
            .await?;
        let parsed: ListScheduleSessionsResponse =
            serde_json::from_value(raw).map_err(|e| unreadable(SESSIONS, &e))?;
        Ok(parsed.sessions)
    }

    /// Remove a job outright. The recipe file it points at stays.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] when the scheduler is off; [`AcpError::Rpc`]
    /// `-32002` for an id that is already gone.
    pub async fn schedules_delete(&self, schedule_id: &str) -> Result<(), AcpError> {
        self.goose_request(DELETE, id_params(schedule_id), MUTATE_TIMEOUT)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the check"
)]
mod tests {
    use super::*;
    use crate::assert_round_trip;

    /// A complete `schedules/list` response, in the shape goose sends one.
    const FIXTURE: &str = include_str!("../../tests/fixtures/scheduler.json");

    const RUNNING: &str = "nightly-dependency-audit";
    const PAUSED: &str = "weekly-changelog-digest";
    const UNREADABLE_CRON: &str = "staging-smoke-tests-during-office-hours";

    fn response() -> Value {
        serde_json::from_str(FIXTURE).unwrap()
    }

    fn jobs() -> Vec<ScheduledJob> {
        assert_round_trip::<ListSchedulesResponse>(&response()).jobs
    }

    fn job(id: &str) -> ScheduledJob {
        jobs()
            .into_iter()
            .find(|job| job.id == id)
            .unwrap_or_else(|| panic!("fixture has no job {id}"))
    }

    // ---- wire shape ----

    #[test]
    fn every_job_round_trips_with_nothing_left_over() {
        let raw = response();
        for entry in raw["jobs"].as_array().unwrap() {
            let job: ScheduledJob = assert_round_trip(entry);
            assert!(
                job.extra.is_empty(),
                "unmodelled keys on {}: {:?}",
                job.id,
                job.extra.keys().collect::<Vec<_>>()
            );
        }
    }

    /// `extra` is the whole point of the flatten, and a fixture whose every
    /// key is modelled cannot prove it: a job that round-trips proves only
    /// that nothing was *renamed*. A field goose grows tomorrow has to come
    /// back out the way it went in, or `extra.is_empty()` elsewhere stops
    /// meaning "we model everything".
    #[test]
    fn a_field_this_crate_does_not_model_survives_the_round_trip() {
        let mut raw = response();
        raw["jobs"][0]["retryPolicy"] = json!({"maxRetries": 2});
        raw["jobs"][0]["nextRun"] = json!("2026-08-26T02:30:00+00:00");
        // Top-level too: `ListSchedulesResponse` carries its own flatten.
        raw["nextCursor"] = json!("page-2");

        let parsed: ListSchedulesResponse = assert_round_trip(&raw);
        assert_eq!(parsed.extra["nextCursor"], json!("page-2"));
        let first = &parsed.jobs[0];
        assert_eq!(first.extra["retryPolicy"], json!({"maxRetries": 2}));
        assert_eq!(first.extra["nextRun"], json!("2026-08-26T02:30:00+00:00"));
        assert_eq!(
            first.extra.len(),
            2,
            "a modelled field leaked into extra: {:?}",
            first.extra.keys().collect::<Vec<_>>()
        );
    }

    /// The shape a real goose sends for a job that has never run.
    ///
    /// `lastRun`, `currentSessionId` and `jobStartTime` all carry
    /// `skip_serializing_if = "Option::is_none"` on the server, so they arrive
    /// *absent* rather than as `null`. The fixture spells them out because
    /// `assert_round_trip` compares both directions and this crate re-emits
    /// `None` as `null` — so absence is the one wire shape a round-trip test
    /// structurally cannot cover, and it is the common one.
    #[test]
    fn the_optional_keys_may_be_absent_entirely() {
        let bare = json!({
            "id": "fresh-job",
            "source": "/home/demo/.config/goose/scheduled-recipes/fresh-job.yaml",
            "cron": "0 0 9 * * *",
            "currentlyRunning": false,
            "paused": false,
        });
        let job: ScheduledJob = serde_json::from_value(bare).unwrap();
        assert_eq!(job.last_run, None);
        assert_eq!(job.current_session_id, None);
        assert_eq!(job.job_start_time, None);
        assert_eq!(job.state(), ScheduleState::Scheduled);
        assert!(job.extra.is_empty());
    }

    #[test]
    fn typed_fields_are_read_not_merely_accepted() {
        let running = job(RUNNING);
        assert!(running.currently_running && !running.paused);
        assert_eq!(
            running.job_start_time.as_deref(),
            Some("2026-08-25T09:34:12.881204+00:00")
        );
        assert_eq!(running.current_session_id.as_deref(), Some("20260825_9"));
        assert_eq!(running.cron, "0 30 2 * * *");
        assert!(running.source.ends_with("nightly-dependency-audit.yaml"));

        let paused = job(PAUSED);
        assert!(paused.paused && !paused.currently_running);
        assert_eq!(paused.current_session_id, None);
        assert!(paused.last_run.is_some());
    }

    /// The audit's stress pass substitutes the longest text it finds; the
    /// fixture has to give it something to bite on, so guard the length
    /// against a well-meaning tidy.
    #[test]
    fn the_fixture_carries_an_overlong_id_and_an_unreadable_cron() {
        let longest = jobs()
            .into_iter()
            .max_by_key(|job| job.id.len())
            .unwrap()
            .id;
        assert!(
            longest.len() > 60,
            "longest fixture id is only {} chars",
            longest.len()
        );
        // A cron `src/cron.rs` cannot hold, so the app's "Runs on a schedule"
        // fallback and its read-only cadence row are both reachable from the
        // mock without editing a fixture.
        assert_eq!(job(UNREADABLE_CRON).cron, "*/15 9-17 * * 1-5");
    }

    #[test]
    fn a_schedule_session_listing_parses_into_the_type_chats_renders() {
        let raw = json!({
            "sessions": [{
                "sessionId": "20260825_9",
                "cwd": "/home/demo",
                "additionalDirectories": [],
                "title": "Nightly dependency audit",
                "updatedAt": "2026-08-25T09:34:12.881204+00:00",
                "_meta": {
                    "messageCount": 2,
                    "createdAt": "2026-08-25T09:34:12.881204+00:00",
                    "userSetName": false,
                    "sessionType": "scheduled",
                    "hasRecipe": true,
                    "lastMessageSnippet": "No new advisories since yesterday."
                }
            }]
        });
        // Not `assert_round_trip`: see the note on `ListScheduleSessionsResponse`
        // — `SessionInfo` is base ACP and deserialize-only, so `extra` plus
        // typed values is the whole check available here.
        let parsed: ListScheduleSessionsResponse = serde_json::from_value(raw).unwrap();
        assert!(parsed.extra.is_empty(), "{:?}", parsed.extra);
        let first = &parsed.sessions[0];
        assert_eq!(first.session_id, "20260825_9");
        assert_eq!(first.display_title(), "Nightly dependency audit");
        assert_eq!(first.kind_label(), Some("Scheduled"));
        assert_eq!(first.message_count(), Some(2));
        assert_eq!(first.cwd.as_deref(), Some("/home/demo"));
    }

    #[test]
    fn run_status_spellings_match_the_schema() {
        assert_eq!(
            serde_json::to_value(RunStatus::Completed).unwrap(),
            "completed"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::Cancelled).unwrap(),
            "cancelled"
        );
        let reply: RunNowResponse = assert_round_trip(&json!({
            "status": "completed",
            "sessionId": "20260825_9",
        }));
        assert_eq!(reply.status, RunStatus::Completed);
        assert_eq!(reply.session_id.as_deref(), Some("20260825_9"));
        assert!(reply.extra.is_empty());
    }

    /// A cancelled run never got a session, and goose omits the key rather
    /// than sending `null`.
    #[test]
    fn a_cancelled_run_carries_no_session() {
        let reply: RunNowResponse = serde_json::from_value(json!({"status": "cancelled"})).unwrap();
        assert_eq!(reply.status, RunStatus::Cancelled);
        assert_eq!(reply.session_id, None);
    }

    #[test]
    fn an_updated_job_comes_back_with_the_new_cadence() {
        let mut raw = response();
        let mut updated = raw["jobs"][1].take();
        updated["cron"] = json!("0 0 7 * * 1");
        let parsed: UpdateScheduleResponse = assert_round_trip(&json!({ "job": updated }));
        assert_eq!(parsed.job.cron, "0 0 7 * * 1");
        assert!(parsed.extra.is_empty());
    }

    #[test]
    fn a_kill_reply_is_a_sentence_and_nothing_else() {
        let parsed: KillRunningJobResponse = assert_round_trip(&json!({
            "message": "Successfully killed running job 'nightly-dependency-audit'",
        }));
        assert!(parsed.message.contains("nightly-dependency-audit"));
        assert!(parsed.extra.is_empty());
    }

    // ---- domain ----

    /// goose pauses the schedule, not the process, so both flags can be true
    /// at once. What is happening now wins.
    #[test]
    fn state_puts_a_live_run_ahead_of_a_pause() {
        let mut job = job(RUNNING);
        assert_eq!(job.state(), ScheduleState::Running);
        job.paused = true;
        assert_eq!(job.state(), ScheduleState::Running);
        job.currently_running = false;
        assert_eq!(job.state(), ScheduleState::Paused);
        job.paused = false;
        assert_eq!(job.state(), ScheduleState::Scheduled);
    }

    /// Three states, three dots, three words — a state that shared a dot with
    /// another would be a row that cannot be read at a glance, which is the
    /// only thing the dot is for.
    #[test]
    fn every_state_has_a_dot_and_a_word_of_its_own() {
        let states = [
            ScheduleState::Running,
            ScheduleState::Paused,
            ScheduleState::Scheduled,
        ];
        let mut dots: Vec<&str> = states.into_iter().map(ScheduleState::dot).collect();
        dots.sort_unstable();
        dots.dedup();
        assert_eq!(dots.len(), 3);
        let mut words: Vec<&str> = states.into_iter().map(ScheduleState::word).collect();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), 3);
        // The busy dot pulses, and a run in flight is the one thing on this
        // screen that is moving.
        assert_eq!(ScheduleState::Running.dot(), "busy");
    }

    // ---- requests ----

    /// Every wire key this feature sends, in one place. `tests/contract.rs`
    /// checks the same literals against goose's own declaration of each
    /// method; this checks that the builders produce them.
    #[test]
    fn requests_use_goose_casing() {
        assert_eq!(list_params(), json!({}));
        assert_eq!(id_params("nightly"), json!({"scheduleId": "nightly"}));
        assert_eq!(
            update_params("nightly", "0 0 7 * * 1"),
            json!({"scheduleId": "nightly", "cron": "0 0 7 * * 1"})
        );
        assert_eq!(
            sessions_params("nightly", 20),
            json!({"scheduleId": "nightly", "limit": 20})
        );
    }

    /// The asymmetry a reader will not believe: seven methods say
    /// `scheduleId` and this one says `jobId`, for the same value.
    #[test]
    fn kill_is_the_one_method_that_says_job_id() {
        assert_eq!(kill_params("nightly"), json!({"jobId": "nightly"}));
        assert!(kill_params("nightly").get("scheduleId").is_none());
    }

    #[test]
    fn pause_and_unpause_are_two_methods() {
        assert_eq!(pause_method(true), "_goose/unstable/schedules/pause");
        assert_eq!(pause_method(false), "_goose/unstable/schedules/unpause");
    }

    /// A run-now that hangs must not be reported as a failed run, so the
    /// ceiling is far past any agent turn and the caller treats reaching it
    /// as "we stopped listening".
    #[test]
    fn the_run_now_ceiling_is_not_a_deadline_anybody_reaches() {
        assert!(RUN_TIMEOUT >= Duration::from_hours(1));
        assert!(RUN_TIMEOUT > LIST_TIMEOUT && RUN_TIMEOUT > MUTATE_TIMEOUT);
    }
}
