//! Timing the three round trips the reader waits on, so an argument about
//! where the wait goes can be settled with a number.
//!
//! Before this, `Instant::now` appeared twice in the whole workspace and both
//! were a test harness's deadline (`settle_until` in `src/state.rs`); the
//! `SystemTime::now` calls are all Unix seconds off the wall clock — a row's
//! age, a transcript gap mark, a cache key. One-second resolution, about
//! *when* something happened. **Nothing measured how long anything took**, so
//! a running build could not answer "where did that go" for any of the three
//! waits that have a spinner attached: `establish`, `session/list` and
//! `session/load`. The repo carries exactly one real-server latency figure and
//! it is a doc comment on `learn_default_model` — 635 ms for three round
//! trips, deliberately off the critical path, and silent about loading a
//! conversation.
//!
//! # A load wants three marks, not two
//!
//! For `session/load` the quantity that decides how the app feels is **time to
//! the first replayed frame**, not time to resolve. goose re-activates the
//! agent and replays the conversation as `session/update` notifications, which
//! arrive on a separate task and are folded by `pump` → `apply_update` — so
//! they are not sequenced with the RPC's reply by anything the client can see.
//! `docs/where-state-lives.md` records the claim that they land *before* the
//! call resolves as a source read rather than a measurement, and it is the
//! only unverified item in that list a client can settle on its own.
//!
//! So a load is marked three times: request sent, first fold for that session,
//! resolve. The first gap is how long the pane has nothing in it. The gap to
//! resolve is what the composer waits on, because `can_send` is gated on
//! `chat.loading` (`src/views/chat.rs`) and that flag is cleared where the
//! call returns. If the two are the same number the replay is not early and
//! a transcript cache buys the whole wait; if the first is much smaller, the
//! pane fills early and only the composer is blocked.
//!
//! The watch is deliberately NOT closed when the call resolves, because a
//! first frame that lands *after* the reply is precisely the outcome the
//! unverified claim rules out — closing it would guarantee the answer this is
//! being built to find. The cost is that a conversation which replays nothing
//! at all leaves the watch armed until the next load replaces it, so a
//! `first-frame` line whose number dwarfs its own `resolve` line reads as
//! "nothing was replayed", not "the replay was slow".
//!
//! # What it prints, and what it deliberately does not
//!
//! One greppable line per mark — `@@TIME@@{label}@@{ms}` — read off the
//! console the way `scripts/capture-gallery.py` reads `@@DOM@@` lines, so
//! there is no file to manage and no consumer to build. Milliseconds with
//! three decimals, because against `crates/mock-goose-server/` every one of
//! these is microseconds and a number that rounds to `0.0` would look like a
//! broken instrument rather than a fake answering from memory.
//!
//! **Labels and durations only.** No titles, no message text, no session ids.
//! The console this prints to belongs to whoever is running the debug build,
//! and a transcript is the one thing in this app that is nobody else's
//! business; a timing line that carried a conversation's id would turn a
//! pasted log into a disclosure. The replay watch below does hold a session id
//! to match against, and never prints it.
//!
//! # The shape is `src/domdump.rs`'s, for its reasons
//!
//! Declared `#[cfg(debug_assertions)]` in `src/main.rs` and `cfg`'d again
//! here, so a release build does not carry an instrument as dead code — the
//! rule `src/shell/mod.rs` states for `DUMP_PREFIX`. The `print_stdout`
//! exception is scoped to this module and carries its reason, because the lint
//! is on workspace-wide (`Cargo.toml`) and CI runs `-D warnings`: an
//! instrument whose output *is* the console is the narrow case that earns one,
//! and it earns it only where the module cannot reach a shipped binary.

#![cfg(debug_assertions)]
#![allow(clippy::print_stdout, reason = "the console is this module's output")]

use std::sync::Mutex;
use std::time::Instant;

/// Open an interval.
pub(crate) fn start() -> Instant {
    Instant::now()
}

/// Close one, and print it.
pub(crate) fn done(label: &str, t: Instant) {
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("@@TIME@@{label}@@{ms:.3}");
}

/// The load whose first replayed frame has not arrived yet.
///
/// One slot rather than a map: opening a conversation replaces whatever was
/// open, so at most one load is ever the one being waited on, and a load that
/// gets abandoned mid-flight should stop being reported rather than print late
/// against a pane nobody is looking at.
struct Replay {
    /// Matched against, never printed. See the module note.
    session_id: String,
    label: &'static str,
    at: Instant,
}

static REPLAY: Mutex<Option<Replay>> = Mutex::new(None);

/// Arm the first-frame watch for a session whose `session/load` is going out.
pub(crate) fn watch_replay(label: &'static str, session_id: &str, at: Instant) {
    if let Ok(mut slot) = REPLAY.lock() {
        *slot = Some(Replay {
            session_id: session_id.to_string(),
            label,
            at,
        });
    }
}

/// Report the first fold that lands for a watched session, once.
///
/// Called from the fold for every update of every session, so it has to be
/// cheap and silent in the overwhelmingly common case: a streamed token
/// against no armed watch is one uncontended lock and a `None`. A poisoned
/// lock gives up instead of unwinding — an instrument that panics the app it
/// is measuring has answered a different question.
pub(crate) fn replay_frame(session_id: &str) {
    let hit = REPLAY.lock().ok().and_then(|mut slot| {
        if slot.as_ref().is_some_and(|r| r.session_id == session_id) {
            slot.take()
        } else {
            None
        }
    });
    if let Some(r) = hit {
        done(r.label, r.at);
    }
}
