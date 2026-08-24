//! Building the JSON-RPC frames the mock sends, in the shapes goose sends
//! them.

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// Where a handler's frames go. Every send is fire-and-forget: the writer
/// task owns the sink, and a closed channel means the client is already gone.
pub(crate) type Out = mpsc::UnboundedSender<Message>;

pub(crate) fn notify(out: &Out, method: &str, params: &Value) {
    let frame = json!({"jsonrpc":"2.0","method":method,"params":params});
    let _ = out.send(Message::Text(frame.to_string().into()));
}

pub(crate) fn session_update(out: &Out, sid: &str, update: &Value) {
    notify(
        out,
        "session/update",
        &json!({"sessionId": sid, "update": update}),
    );
}

/// The JSON-RPC message for an error code, which is all goose ever puts in
/// `message`.
///
/// goose builds its failures as `Error::internal_error().data(reason)` and
/// friends, so `message` stays the canned string from the spec and the
/// sentence worth reading is in `data`. The mock used to put its reason in
/// `message`, a shape the real server never sends — which meant a client
/// change could be "verified" against the mock and still show nothing useful
/// against goose.
///
/// `-32002` is not in the JSON-RPC spec; it is ACP's resource-not-found,
/// which is what goose answers for an unknown session id.
const fn canned(code: i64) -> &'static str {
    match code {
        -32700 => "Parse error",
        -32600 => "Invalid Request",
        -32601 => "Method not found",
        -32602 => "Invalid params",
        -32002 => "Resource not found",
        _ => "Internal error",
    }
}

pub(crate) fn error_frame(id: &Value, code: i64, reason: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":canned(code),"data":reason}})
}

/// The frame answering a request: a result, or an error carrying its reason
/// in `data`.
pub(crate) fn response_frame(id: &Value, result: Result<Value, (i64, String)>) -> Value {
    match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err((code, reason)) => error_frame(id, code, &reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client reads `data` first and falls back to `message`, because
    /// that is how goose reports what actually went wrong. A mock that
    /// reversed the two would let a client bug pass.
    #[test]
    fn an_error_carries_its_reason_in_data() {
        let frame = error_frame(&json!(7), -32602, "cwd must be an absolute path");
        assert_eq!(frame["error"]["message"], "Invalid params");
        assert_eq!(frame["error"]["data"], "cwd must be an absolute path");
    }
}
