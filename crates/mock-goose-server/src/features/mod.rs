//! Method dispatch, one module per feature area.
//!
//! A handler answers the methods it owns and returns `None` for everything
//! else, so adding a feature area is a new file plus one line in
//! [`HANDLERS`] — not another arm in a match that five branches all want to
//! edit at once.

pub(crate) mod core;
pub(crate) mod recipes;

use serde_json::Value;

use crate::rpc::Out;
use crate::state::Shared;

/// `None` means "not mine": the next handler in the chain gets the method.
pub(crate) type Handled = Option<Result<Value, (i64, String)>>;
pub(crate) type Handler = fn(&str, &Value, &Shared, &Out) -> Handled;

/// Alphabetical, so five branches appending here merge deterministically.
const HANDLERS: [Handler; 2] = [core::handle, recipes::handle];

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
