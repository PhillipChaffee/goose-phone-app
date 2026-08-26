//! Method dispatch, one module per feature area.
//!
//! A handler answers the methods it owns and returns `None` for everything
//! else, so adding a feature area is a new file plus one line in
//! [`HANDLERS`] — not another arm in a match that five branches all want to
//! edit at once.

pub(crate) mod core;
pub(crate) mod extensions;
pub(crate) mod recipes;
pub(crate) mod scheduler;
pub(crate) mod skills;

use serde_json::Value;

use crate::rpc::Out;
use crate::state::Shared;

/// `None` means "not mine": the next handler in the chain gets the method.
pub(crate) type Handled = Option<Result<Value, (i64, String)>>;
pub(crate) type Handler = fn(&str, &Value, &Shared, &Out) -> Handled;

/// Alphabetical, so five branches appending here merge deterministically.
const HANDLERS: [Handler; 5] = [
    core::handle,
    extensions::handle,
    recipes::handle,
    scheduler::handle,
    skills::handle,
];

/// What a goose started without `--enable-scheduler` answers, string for
/// string: `require_scheduler` returns
/// `method_not_found().data("Scheduled recipe execution is not enabled")`.
///
/// `-32601` and not an error code of its own, because that is the whole point
/// of the branch it exists to exercise: the client turns a `-32601` into
/// `AcpError::Unsupported`, carrying goose's sentence, and the app states the
/// fact rather than showing a failure. A mock that said `-32603` here would
/// test the failure path instead.
///
/// It lives up here rather than in either handler because **two** features go
/// through that one gate: `recipes/schedule` and the whole `schedules/*`
/// namespace both call `require_scheduler` on the real server. Two private
/// copies of a contract string is two things that can drift apart, and the
/// test that would catch it is in neither file.
pub(crate) fn scheduler_disabled() -> (i64, String) {
    (
        -32601,
        "Scheduled recipe execution is not enabled".to_string(),
    )
}

/// Run a request past every handler, in order.
pub(crate) fn dispatch(
    method: &str,
    params: &Value,
    state: &Shared,
    out: &Out,
) -> Result<Value, (i64, String)> {
    for handler in HANDLERS {
        if let Some(result) = handler(method, params, state, out) {
            return result;
        }
    }
    // Load-bearing, not a default: the client's feature detection *is* this
    // code. `-32601` is how it learns a method is absent, and how it decides
    // to hide a screen rather than show a failure — so an unknown method has
    // to arrive here and produce exactly that, never a shrug or a silence.
    Err((-32601, format!("method not found: {method}")))
}
