//! All eight `schedules/*` methods, sent as strings by the client and matched
//! as strings by the mock.
//!
//! This is the one bug neither crate's unit tests can see: the client asks for
//! `_goose/unstable/schedules/running-job/kill` with `{"jobId": ...}` and the
//! mock reads `scheduleId`, both sides pass everything they test, and the
//! button in the app does nothing at all. `kill` is the live risk here — it is
//! the single method in this namespace that spells the id differently from the
//! other seven.

// Test code: a failing unwrap, or a panic on the wrong error variant, IS the
// failing check. `expect` rather than `allow`: if a use goes away, so should
// its exception.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: an unwrap or a wrong-variant panic is the assertion"
)]

mod common;

use goose_acp_client::{AcpError, RunStatus, ScheduleState, ScheduledJob, SessionKind};

const RUNNING: &str = "nightly-dependency-audit";
const PAUSED: &str = "weekly-changelog-digest";
const ODD_CRON: &str = "staging-smoke-tests-during-office-hours";
/// The stress fixture: a job id is a recipe file stem, and this is the width
/// the row and the top bar have to survive.
const LONG: &str = "quarterly-dependency-licence-and-security-audit-across-every-workspace-crate";

fn find<'a>(jobs: &'a [ScheduledJob], id: &str) -> &'a ScheduledJob {
    jobs.iter()
        .find(|job| job.id == id)
        .unwrap_or_else(|| panic!("no job {id} in {:?}", jobs.iter().map(|j| &j.id)))
}

/// One flow through every method the feature added, in the order a person
/// would reach them.
#[tokio::test]
async fn every_scheduler_method_reaches_the_mock_and_comes_back() {
    let (mut server, client) = common::spawn_mock().await;

    // ---- list ----
    let jobs = client.schedules_list().await.unwrap();
    assert_eq!(jobs.len(), 4);
    // The long id survives the trip intact: this is what the audit's stress
    // pass measures, and a truncation here would make it measure nothing.
    assert_eq!(find(&jobs, LONG).id.len(), LONG.len());
    // Absent keys, not nulls — goose `skip_serializing_if`s all three, and a
    // job that has never run is where that shows.
    let never = find(&jobs, LONG);
    assert_eq!(never.last_run, None);
    assert_eq!(never.job_start_time, None);
    assert!(never.extra.is_empty(), "unmodelled keys: {:?}", never.extra);

    let running = find(&jobs, RUNNING);
    assert_eq!(running.state(), ScheduleState::Running);
    assert!(
        running.job_start_time.is_some(),
        "a run in flight has a start"
    );
    let watch = running.current_session_id.clone().unwrap();

    assert_eq!(find(&jobs, PAUSED).state(), ScheduleState::Paused);
    assert_eq!(find(&jobs, ODD_CRON).state(), ScheduleState::Scheduled);

    // ---- sessions/list ----
    //
    // The claim the detail screen is built on: the same type Chats renders,
    // carrying an id `session/load` will actually replay.
    let runs = client.schedules_sessions(RUNNING, 20).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].session_id, watch);
    assert_eq!(runs[0].kind(), Some(SessionKind::Scheduled));
    assert_eq!(runs[0].kind_label(), Some("Scheduled"));
    assert!(runs[0].last_message_snippet().is_some());
    assert!(runs[0].message_count().is_some());
    let cwd = runs[0]
        .cwd
        .clone()
        .expect("a history row has to know its cwd to replay");
    client
        .session_load(&runs[0].session_id, &cwd)
        .await
        .unwrap();
    // `session/load` replays the transcript as notifications, which is base
    // ACP doing its job and not the scheduler doing anything. Drained here so
    // the assertion at the end of this test is about the scheduler alone.
    while server.events.try_recv().is_ok() {}

    // ---- kill ----
    //
    // `jobId` over the socket, with the real client building the body. This is
    // the only place the two spellings meet.
    client.schedules_kill(RUNNING).await.unwrap();
    let jobs = client.schedules_list().await.unwrap();
    assert_eq!(find(&jobs, RUNNING).state(), ScheduleState::Scheduled);
    // A killed run still stamps a last run: it happened, it was stopped.
    assert!(find(&jobs, RUNNING).last_run.is_some());

    // ---- pause / unpause ----
    client.schedules_set_paused(ODD_CRON, true).await.unwrap();
    let jobs = client.schedules_list().await.unwrap();
    assert!(find(&jobs, ODD_CRON).paused, "pause did not stick");

    client.schedules_set_paused(ODD_CRON, false).await.unwrap();
    let jobs = client.schedules_list().await.unwrap();
    assert!(!find(&jobs, ODD_CRON).paused, "unpause did not stick");

    // ---- update ----
    //
    // Asserted on the *cadence*, not on an EmptyResponse: goose re-lists to
    // build its reply, so this is the stored value coming back.
    let updated = client
        .schedules_update(PAUSED, "0 0 7 * * 2")
        .await
        .unwrap();
    assert_eq!(updated.cron, "0 0 7 * * 2");
    assert_eq!(updated.id, PAUSED);
    let jobs = client.schedules_list().await.unwrap();
    assert_eq!(find(&jobs, PAUSED).cron, "0 0 7 * * 2");

    // ---- run-now ----
    let reply = client.schedules_run_now(PAUSED).await.unwrap();
    assert_eq!(reply.status, RunStatus::Completed);
    assert!(
        reply.session_id.is_some(),
        "a completed run names its session"
    );
    assert!(reply.extra.is_empty(), "unmodelled keys: {:?}", reply.extra);
    let jobs = client.schedules_list().await.unwrap();
    let started = find(&jobs, PAUSED);
    assert!(started.currently_running, "run-now left the job idle");
    assert!(started.job_start_time.is_some());
    assert_eq!(started.current_session_id, reply.session_id);
    client.schedules_kill(PAUSED).await.unwrap();

    // ---- an id nothing answers to ----
    //
    // `-32002` is ACP's resource-not-found, reaching the client as an `Rpc`
    // error and not an `Unsupported` one: the feature is present, this call
    // was wrong. Asserted on the *code* — goose puts `{"uri": id}` in `data`
    // and this mock's frame builder carries only strings, so the sentence is
    // the one thing the two cannot agree on.
    match client.schedules_set_paused("no-such-job", true).await {
        Err(AcpError::Rpc { code, .. }) => assert_eq!(code, -32002),
        other => panic!("expected an RPC error, got {other:?}"),
    }

    // ---- delete ----
    client.schedules_delete(ODD_CRON).await.unwrap();
    let jobs = client.schedules_list().await.unwrap();
    assert_eq!(jobs.len(), 3);
    assert!(!jobs.iter().any(|job| job.id == ODD_CRON));

    // ---- and nothing was pushed ----
    //
    // There is no notification for any of this. Nothing announces a job
    // starting, finishing or being killed — which is the entire reason the
    // screen holds a poll rather than a subscription. If goose ever grows one,
    // this is the assertion that should fail, and the poll is what should go.
    assert!(
        server.events.try_recv().is_err(),
        "the scheduler pushed an event; the screen could stop polling"
    );
}

/// A goose started without `--enable-scheduler`, end to end.
///
/// The `-32601` and goose's sentence are what the client turns into
/// `Unsupported` and the screen turns into prose with no Retry, so the whole
/// degraded state is proved here rather than asserted about a constant.
#[tokio::test]
async fn a_scheduler_less_server_says_so_in_goose_s_own_words() {
    let (_server, client) = common::spawn_mock_with(&[("MOCK_NO_SCHEDULER", "1")]).await;

    match client.schedules_list().await {
        Err(AcpError::Unsupported {
            feature,
            method,
            reason,
        }) => {
            assert_eq!(feature, goose_acp_client::Feature::Scheduler);
            assert_eq!(method, "_goose/unstable/schedules/list");
            assert_eq!(
                reason.as_deref(),
                Some("Scheduled recipe execution is not enabled")
            );
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }

    // The second call costs no socket round trip — `goose_request` remembers
    // the refusal per method — and still says the same thing, minus the reason
    // it did not ask for again.
    assert!(client.schedules_list().await.unwrap_err().is_unsupported());
}

/// The invariant that would otherwise be invisible until a screen stayed dark
/// until relaunch: **only a `-32601` is remembered.**
///
/// `goose_request` caches unsupported methods per connection, and a cache that
/// swallowed an ordinary failure would turn one bad list — a permissions blip
/// on the server's `schedule.json` — into "this server has no scheduler" for
/// the life of the socket, with no Retry offered because there is nothing to
/// retry. The `broken` fixture fails the first list with `-32603` and answers
/// normally after, so both halves are reachable in one process.
#[tokio::test]
async fn a_transient_failure_is_not_remembered_as_a_missing_feature() {
    let (_server, client) = common::spawn_mock_with(&[("MOCK_FIXTURES", "broken")]).await;

    let first = client.schedules_list().await.unwrap_err();
    assert!(
        !first.is_unsupported(),
        "a -32603 was filed as a missing feature: {first:?}"
    );
    match first {
        AcpError::Rpc { code, .. } => assert_eq!(code, -32603),
        other => panic!("expected an RPC error, got {other:?}"),
    }

    let recovered = client.schedules_list().await.unwrap();
    assert_eq!(recovered.len(), 4, "the retry did not reach the store");
}
