//! goose's own `_goose/unstable/*` namespace: the methods that are not part of
//! the Agent Client Protocol at all.
//!
//! One file per feature area lives here (recipes, skills, scheduler,
//! extensions, session history), and every one of them goes out through
//! [`AcpClient::goose_request`] so that a server which does not offer the
//! feature produces [`AcpError::Unsupported`] rather than a raw `-32601`.
//!
//! # Casing
//!
//! **`#[serde(rename_all = ...)]` is banned on every type in this module.**
//! goose's custom namespace has no single casing convention — it is
//! inconsistent *per type*: `RecipeListEntryDto` is `snake_case`
//! (`file_path`, `last_modified`), `ScheduledJobDto` is `camelCase` (`lastRun`,
//! `currentlyRunning`), and `SourceEntry` mixes the two (`supportingFiles`
//! next to `global`). A blanket `rename_all` on a type whose wire shape is
//! mixed silently renames the fields it should not have touched.
//!
//! So: a `snake_case` field carries no attribute at all — the Rust name *is*
//! the wire name — and a `camelCase` field carries its own
//! `#[serde(rename = "lastRun")]`, on the line a reviewer diffs against
//! `crates/goose/acp-schema.json`. Every renamed field is visible; nothing is
//! renamed by a rule stated once at the top of the type.
//!
//! [`crate::types`] keeps its blanket `rename_all = "camelCase"` and is out of
//! scope for this rule: base ACP is uniformly `camelCase` *by specification*, so
//! one attribute per type there is a statement of the spec, not a guess. The
//! difference between the two modules is a decision, not rot.
//!
//! # Unknown fields
//!
//! Every DTO here carries exactly one `#[serde(flatten)] extra: Map<String,
//! Value>`. It does two jobs. It round-trips fields the phone does not model,
//! so a value the app reads and writes back does not lose whatever goose put
//! there. And it makes [`crate::assert_round_trip`] able to name a field this
//! crate spells wrong: goose sets no `deny_unknown_fields`, so a mis-spelled
//! field is silently dropped to `None` and the correctly-spelled wire key
//! lands in `extra` — the two together are what the round-trip check sees.
//!
//! For that to work an `Option` field on a DTO here must *not* be
//! `skip_serializing_if`-ed away: the `null` it serializes beside the key in
//! `extra` is the evidence. Test fixtures are therefore complete server
//! responses, not minimal ones.

use std::collections::HashSet;
use std::sync::{MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::client::AcpClient;
use crate::error::{string_data, AcpError, Feature};

// One module per feature area, alphabetical, each re-exported flat so the
// public path is `goose_acp_client::Thing` wherever the thing lives.
mod skills;

pub use skills::*;

/// The prefix on every method goose adds to ACP.
///
/// `unstable` is goose's word, not ours: the shapes behind it change between
/// releases with no deprecation window, which is why a `-32601` here means a
/// feature is missing rather than a server is broken.
pub const GOOSE_NAMESPACE: &str = "_goose/unstable/";

/// Timeout for a read of a list the server holds on disk (recipes, skills,
/// schedules). Generous: goose walks directories to build these.
pub(crate) const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for a write. Same 30 s: these are local file or database writes,
/// and anything slower than a list is already wrong.
pub(crate) const MUTATE_TIMEOUT: Duration = Duration::from_secs(30);

impl AcpClient {
    /// The funnel every `_goose/unstable/*` wrapper goes through.
    ///
    /// Sends `method` and, on a `-32601`, converts goose's "no such method"
    /// into [`AcpError::Unsupported`] and remembers it. `-32601` is goose's
    /// own signal for "this feature is absent", not merely "this server is
    /// old": `require_scheduler` answers every schedule method with
    /// `method_not_found().data("Scheduled recipe execution is not enabled")`
    /// when the server was started without `--enable-scheduler`. Whatever
    /// sentence goose put in `data` becomes the error's `reason`.
    ///
    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if the server does not offer the method —
    /// without touching the socket once that is known. Otherwise as
    /// [`AcpClient::request_with_timeout`]: [`AcpError::Timeout`],
    /// [`AcpError::Closed`], or [`AcpError::Rpc`].
    pub(crate) async fn goose_request(
        &self,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AcpError> {
        // Bound the guard to this statement: nothing may hold the lock across
        // the await below.
        let known_missing = self.unsupported_methods().contains(method);
        if known_missing {
            return Err(AcpError::Unsupported {
                feature: Feature::of_method(method),
                method: method.to_string(),
                reason: None,
            });
        }

        match self.request_with_timeout(method, params, timeout).await {
            Err(AcpError::Rpc {
                code: -32601, data, ..
            }) => {
                self.unsupported_methods().insert(method);
                Err(AcpError::Unsupported {
                    feature: Feature::of_method(method),
                    method: method.to_string(),
                    reason: string_data(data.as_ref()).map(str::to_string),
                })
            }
            other => other,
        }
    }

    /// The per-connection set of methods this server has already refused.
    ///
    /// A poisoned lock means some other task panicked mid-update; the set is
    /// a cache of refusals, so the worst a recovered guard can hold is a
    /// half-inserted entry — recovering beats taking the app down with it.
    fn unsupported_methods(&self) -> MutexGuard<'_, HashSet<&'static str>> {
        self.unsupported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
