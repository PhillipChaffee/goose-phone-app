//! The one error type this crate returns, and the classification of goose's
//! custom methods that gives a missing feature a sentence a user can act on.

use serde_json::Value;

/// Which area of goose a custom method belongs to.
///
/// goose gates whole feature areas at startup — the scheduler needs
/// `--enable-scheduler`, and older builds simply lack the newer namespaces —
/// so "this method is not there" is nearly always "this *feature* is not
/// there". Classifying the method is what lets one screen say "Scheduler is
/// not available on this goose server" instead of surfacing `-32601`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    Recipes,
    Skills,
    Scheduler,
    Extensions,
    SessionHistory,
    /// Anything not in goose's custom namespace: base ACP, which every server
    /// must implement.
    Other,
}

impl Feature {
    /// Classify a JSON-RPC method by its namespace. Total: an unrecognised
    /// method is [`Feature::Other`], never a panic.
    #[must_use]
    pub fn of_method(method: &str) -> Self {
        // Everything goose adds lives under one prefix. Base ACP methods are
        // mandatory, so a `-32601` on `session/prompt` means a broken server,
        // not a feature that is switched off — hence `Other` for those.
        let Some(rest) = method.strip_prefix(crate::goose::GOOSE_NAMESPACE) else {
            return Self::Other;
        };

        // `recipes/schedule` is gated on `--enable-scheduler`, not on recipe
        // support: goose calls `require_scheduler` before it even resolves the
        // recipe. It therefore has to be matched ahead of the `recipes/`
        // prefix it otherwise belongs to.
        if rest == "recipes/schedule" || rest.starts_with("schedules/") {
            return Self::Scheduler;
        }
        // Extensions arrive on three separate paths: the global config list,
        // the per-session list, and the catalogue of installable ones.
        if rest.starts_with("extensions/")
            || rest.starts_with("config/extensions/")
            || rest.starts_with("session/extensions/")
        {
            return Self::Extensions;
        }
        if rest.starts_with("recipes/") {
            return Self::Recipes;
        }
        // A skill is a "source" on the wire — goose's own name for the
        // directory of markdown files it loads skills from.
        if rest.starts_with("sources/") {
            return Self::Skills;
        }
        if rest.starts_with("session/") {
            return Self::SessionHistory;
        }
        Self::Other
    }

    /// UI copy for this feature, written to slot into a sentence.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Recipes => "Recipes",
            Self::Skills => "Skills",
            Self::Scheduler => "Scheduler",
            Self::Extensions => "Extensions",
            Self::SessionHistory => "Session history",
            Self::Other => "This feature",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("connection closed")]
    Closed,
    #[error("timed out")]
    Timeout,
    /// A JSON-RPC error object from the agent.
    ///
    /// Rendered from `data` when that is a string, falling back to `message`:
    /// goose builds nearly every failure as
    /// `Error::internal_error().data(e.to_string())`, so `message` is the
    /// canned "Internal error" and the sentence worth reading is in `data`.
    /// goose's own desktop client reads `data` first for the same reason.
    #[error("{}", rpc_reason(.message, .data.as_ref()))]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    /// The server does not offer this method.
    ///
    /// `reason` is carried rather than flattened to a bool because the two
    /// cases need different sentences. `Some` is goose answering "this server
    /// can do it, it just isn't switched on" — `require_scheduler` returns
    /// `method_not_found().data("Scheduled recipe execution is not enabled")`
    /// when `--enable-scheduler` is off — which points the user at a flag on
    /// their own machine. `None` means the method is simply absent: the
    /// server is older than the feature, and there is nothing to switch on.
    #[error("{}", unsupported_message(*.feature, .reason.as_deref()))]
    Unsupported {
        feature: Feature,
        method: String,
        reason: Option<String>,
    },
    /// A write appeared to succeed but reading it back did not match.
    ///
    /// Its own variant so it can never be mistaken for a transient RPC
    /// hiccup and retried: the server took the call and did something else.
    /// The caller composes the whole sentence, because only it knows what was
    /// being written.
    #[error("{0}")]
    Verification(String),
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl AcpError {
    /// Whether this is a feature the server does not offer — the one error a
    /// screen answers by hiding itself rather than by showing a failure.
    #[must_use]
    pub const fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }
}

/// The `data` payload as a usable sentence, or `None` when it is absent,
/// blank, or not a string.
pub(crate) fn string_data(data: Option<&Value>) -> Option<&str> {
    let text = data?.as_str()?.trim();
    (!text.is_empty()).then_some(text)
}

/// What to show a user for a JSON-RPC error: `data` when it carries a
/// sentence, else the `message` field.
fn rpc_reason<'a>(message: &'a str, data: Option<&'a Value>) -> &'a str {
    string_data(data).unwrap_or(message)
}

fn unsupported_message(feature: Feature, reason: Option<&str>) -> String {
    reason.map_or_else(
        || format!("{} is not available on this goose server", feature.label()),
        |reason| format!("{}: {reason}", feature.label()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_every_goose_namespace() {
        let cases = [
            ("_goose/unstable/recipes/list", Feature::Recipes),
            ("_goose/unstable/recipes/save", Feature::Recipes),
            ("_goose/unstable/sources/list", Feature::Skills),
            ("_goose/unstable/sources/update", Feature::Skills),
            ("_goose/unstable/schedules/list", Feature::Scheduler),
            (
                "_goose/unstable/schedules/running-job/kill",
                Feature::Scheduler,
            ),
            // Scheduler-gated despite the recipe prefix.
            ("_goose/unstable/recipes/schedule", Feature::Scheduler),
            ("_goose/unstable/extensions/available", Feature::Extensions),
            (
                "_goose/unstable/config/extensions/list",
                Feature::Extensions,
            ),
            (
                "_goose/unstable/session/extensions/add",
                Feature::Extensions,
            ),
            ("_goose/unstable/session/rename", Feature::SessionHistory),
            ("_goose/unstable/session/export", Feature::SessionHistory),
            // Custom, but none of the five areas.
            ("_goose/unstable/providers/list", Feature::Other),
            ("_goose/unstable/tools/list", Feature::Other),
            // Base ACP.
            ("session/prompt", Feature::Other),
            ("session/list", Feature::Other),
            ("", Feature::Other),
        ];
        for (method, expected) in cases {
            assert_eq!(Feature::of_method(method), expected, "method: {method}");
        }
    }

    #[test]
    fn unsupported_reads_the_reason_when_there_is_one() {
        let switched_off = AcpError::Unsupported {
            feature: Feature::Scheduler,
            method: "_goose/unstable/schedules/list".into(),
            reason: Some("Scheduled recipe execution is not enabled".into()),
        };
        assert_eq!(
            switched_off.to_string(),
            "Scheduler: Scheduled recipe execution is not enabled"
        );
        assert!(switched_off.is_unsupported());

        let too_old = AcpError::Unsupported {
            feature: Feature::SessionHistory,
            method: "_goose/unstable/session/export".into(),
            reason: None,
        };
        assert_eq!(
            too_old.to_string(),
            "Session history is not available on this goose server"
        );

        assert_eq!(
            AcpError::Unsupported {
                feature: Feature::Other,
                method: "_goose/unstable/tools/list".into(),
                reason: None,
            }
            .to_string(),
            "This feature is not available on this goose server"
        );
    }

    /// goose puts the sentence in `data` and leaves `message` as the canned
    /// JSON-RPC text, so `data` wins whenever it is a non-blank string.
    #[test]
    fn rpc_prefers_a_string_data_over_the_message() {
        let with_data = |data: Value| AcpError::Rpc {
            code: -32603,
            message: "Internal error".into(),
            data: Some(data),
        };

        assert_eq!(
            with_data(json!("cwd must be an absolute path")).to_string(),
            "cwd must be an absolute path"
        );
        // Absent, blank, and non-string all fall back to `message`.
        assert_eq!(
            AcpError::Rpc {
                code: -32603,
                message: "Internal error".into(),
                data: None,
            }
            .to_string(),
            "Internal error"
        );
        assert_eq!(with_data(json!("   ")).to_string(), "Internal error");
        assert_eq!(
            with_data(json!({"detail": "x"})).to_string(),
            "Internal error"
        );
        assert_eq!(with_data(Value::Null).to_string(), "Internal error");
    }

    #[test]
    fn verification_is_not_an_rpc_error() {
        let err = AcpError::Verification("the server kept the old title".into());
        assert!(!err.is_unsupported());
        assert_eq!(err.to_string(), "the server kept the old title");
    }
}
