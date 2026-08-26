//! The Scheduler's eight methods, as `tests/contract.rs` checks them.
//!
//! The literals below are the ones `src/goose/scheduler.rs`'s
//! `requests_use_goose_casing` and `kill_is_the_one_method_that_says_job_id`
//! pin its private builders to. Keep the two in step: the unit test says the
//! builder produces this, and the file above says goose accepts it.
//!
//! Two methods goose declares are deliberately absent, because this app never
//! sends them: `schedules/create` (it needs a whole recipe body — authoring,
//! which the phone does not do) and `schedules/running-job/inspect` (its
//! whole payload is already on the list row).

use serde_json::{json, Value};

use crate::Sample;

/// The id that stands in for a real one. Its value is irrelevant — only the
/// key it sits under is being checked — but it is spelled like a real job id
/// so a failure message reads like something that happened.
const ID: &str = "nightly-dependency-audit";

pub(crate) const SAMPLES: &[Sample] = &[
    Sample {
        method: "_goose/unstable/schedules/list",
        params: list,
    },
    Sample {
        method: "_goose/unstable/schedules/update",
        params: update,
    },
    Sample {
        method: "_goose/unstable/schedules/delete",
        params: schedule_id,
    },
    Sample {
        method: "_goose/unstable/schedules/pause",
        params: schedule_id,
    },
    Sample {
        method: "_goose/unstable/schedules/unpause",
        params: schedule_id,
    },
    Sample {
        method: "_goose/unstable/schedules/run-now",
        params: schedule_id,
    },
    Sample {
        method: "_goose/unstable/schedules/sessions/list",
        params: sessions,
    },
    // The one method in this namespace that spells the same value `jobId`.
    Sample {
        method: "_goose/unstable/schedules/running-job/kill",
        params: kill,
    },
];

fn list() -> Value {
    json!({})
}

fn schedule_id() -> Value {
    json!({ "scheduleId": ID })
}

fn update() -> Value {
    json!({ "scheduleId": ID, "cron": "0 0 7 * * 1" })
}

/// `limit` is required, not optional — omitting it is a `-32602`, which is
/// the sort of thing that is obvious in the schema and invisible in a call.
fn sessions() -> Value {
    json!({ "scheduleId": ID, "limit": 20 })
}

fn kill() -> Value {
    json!({ "jobId": ID })
}
