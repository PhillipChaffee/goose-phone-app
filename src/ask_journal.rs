//! A durable note of every permission ask that never got its answer back to
//! goose.
//!
//! # Why this exists, and why it is written when it is
//!
//! MEASURED, not read (`docs/permission-durability.md` section 0): goose
//! 1.46.0, `mode = approve`, a client that receives an ask and never answers.
//! Seventy-five seconds later a fresh client called `session/load` and got
//! back four things — the user's message, two usage reports and the command
//! list. No tool call. No assistant message. No decline. **The round is
//! destroyed on the server, and nothing on this side can bring it back.** The
//! prompt survives, and so does the title goose generated from it, so the
//! session comes back named after work that has no trace of having happened.
//!
//! This file cannot fix that. What it can do is stop the loss being silent.
//!
//! The one design decision worth defending is *when* the note is written: at
//! the moment the ask **arrives**, not at the moment it is lost. The case
//! this exists for is the app being killed — iOS jetsam after a suspension is
//! the shape section 0 was built to imitate — and no Rust in this process
//! runs at that moment. A note taken on the way out is a note that is never
//! taken. So the last instant we are guaranteed to be alive is the one we
//! use, and an entry left `Open` by a process that did not come back is
//! reconciled into a loss at the next startup.
//!
//! Everything here is a plain function over a `Vec`, and that is deliberate
//! too: it is the half of this feature that can be tested without a renderer.
//! The signal, the storage backing and the four call sites are in
//! `crate::state`.

use serde::{Deserialize, Serialize};

/// Where the journal is kept, named here rather than written at the one call
/// site in `crate::state` so that a test in this file can hold it to account.
///
/// It has to be `LocalStorage`. `use_persistent`, which `settings` and
/// `code_cache` use, resolves to `SessionStorage` — an in-memory `HashMap`
/// hung off the root context on every non-wasm target (dioxus-sdk-storage
/// `persistence.rs:34`, `client_storage/mod.rs:32-41`, `memory.rs:13-28`) —
/// and a journal kept there would evaporate on exactly the event it exists to
/// survive. `the_journals_storage_backing_really_reaches_the_disk` is the
/// gate: swap this alias and that test fails.
pub(crate) type Backing = dioxus_sdk_storage::LocalStorage;

/// How many entries are kept. The storage backing rewrites a key's whole file
/// on every change, and that write is on the path of every permission ask, so
/// the journal's size is a cost paid per ask rather than per loss.
const MAX_ENTRIES: usize = 20;

/// How long a loss is worth reporting. A week: long enough to survive a phone
/// left in a drawer over a weekend, short enough that nothing here becomes an
/// archive. This is a notification queue, not an audit log.
const MAX_AGE_SECS: i64 = 7 * 86_400;

/// One permission ask this client received, and what became of it.
///
/// Answered and withdrawn asks are **removed**, not retained: an entry exists
/// only while it has something left to say.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AskRecord {
    pub session_id: String,
    /// What the session was called when the ask arrived.
    ///
    /// Kept rather than looked up later because the generated title is the
    /// one string the server is known to keep (section 0.2), so it is the
    /// only thing certain to still mean something when the reader comes back.
    pub session_title: String,
    /// The ACP `toolCallId`. The key for everything here: the JSON-RPC
    /// request id belongs to a socket that is, by the time this matters,
    /// gone.
    pub tool_call_id: String,
    /// `"shell · uname -a"` — resolved with the same fallback chain the modal
    /// uses, so the card names the ask the way the modal named it.
    pub title: String,
    pub asked_at: i64,
    pub state: AskState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AskState {
    /// On screen, unanswered, and the connection is believed live.
    Open,
    Lost {
        at: i64,
        cause: LostCause,
    },
    /// Reported and dismissed. Kept briefly rather than deleted so a second
    /// sighting of the same ask cannot undo a dismissal.
    Acknowledged {
        at: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LostCause {
    /// The socket died under it and this process was awake to see it.
    Connection,
    /// Found `Open` at startup: the app itself went away, which is the case
    /// section 0 measured and the one no drain-time hook can catch.
    AppEnded,
}

impl AskRecord {
    /// A fresh, unanswered ask.
    pub(crate) const fn open(
        session_id: String,
        session_title: String,
        tool_call_id: String,
        title: String,
        asked_at: i64,
    ) -> Self {
        Self {
            session_id,
            session_title,
            tool_call_id,
            title,
            asked_at,
            state: AskState::Open,
        }
    }

    pub(crate) const fn is_open(&self) -> bool {
        matches!(self.state, AskState::Open)
    }

    /// Whether this is a loss the reader has not yet been shown.
    pub(crate) const fn is_unreported_loss(&self) -> bool {
        matches!(self.state, AskState::Lost { .. })
    }

    /// When this entry last changed, for ageing it out.
    const fn touched_at(&self) -> i64 {
        match self.state {
            AskState::Open => self.asked_at,
            AskState::Lost { at, .. } | AskState::Acknowledged { at } => at,
        }
    }
}

/// Note an ask that has just arrived.
///
/// Replaces any entry for the same `tool_call_id` rather than adding a second
/// one. That is not tidiness: `session/load` makes goose re-raise the asks it
/// still holds, so an ask this journal already gave up for lost can arrive
/// again on the next connection, and the entry has to become `Open` again or
/// the reader is told a round was lost while looking at it.
pub(crate) fn note(journal: &mut Vec<AskRecord>, record: AskRecord, now: i64) {
    journal.retain(|entry| entry.tool_call_id != record.tool_call_id);
    journal.push(record);
    prune(journal, now);
}

/// The ask got its answer, or the agent took the question back. Either way
/// there is nothing left to tell anyone.
pub(crate) fn resolve(journal: &mut Vec<AskRecord>, tool_call_id: &str) {
    journal.retain(|entry| entry.tool_call_id != tool_call_id);
}

/// The connection died under us with asks still open.
///
/// Every `Open` entry, not just the ones still sitting in `ctx.permission`.
/// The queue is drained by whichever of two tasks the runtime wakes first
/// (`send_prompt`'s post-turn sweep, or the pump's `Disconnected` arm), and a
/// design that read the queue here would report nothing at all in the single
/// case that matters — a lone session with a turn in flight, which is the
/// whole bug.
pub(crate) fn lose_open(journal: &mut [AskRecord], cause: LostCause, now: i64) {
    for entry in journal.iter_mut().filter(|entry| entry.is_open()) {
        entry.state = AskState::Lost { at: now, cause };
    }
}

/// The user closed the connection themselves. Their asks go without a word:
/// they pressed the button, and there is nothing to narrate.
pub(crate) fn forget_open(journal: &mut Vec<AskRecord>) {
    journal.retain(|entry| !entry.is_open());
}

/// The reader has seen this one and dismissed it.
pub(crate) fn acknowledge(journal: &mut [AskRecord], tool_call_id: &str, now: i64) {
    for entry in journal
        .iter_mut()
        .filter(|entry| entry.tool_call_id == tool_call_id)
    {
        entry.state = AskState::Acknowledged { at: now };
    }
}

/// What the app makes of the journal it finds on disk at startup.
///
/// An entry still marked `Open` was written by a process that never got to
/// say what happened to it — which is precisely the measured case: the app
/// was killed, so the `Disconnected` arm never ran, so nothing marked it.
/// This is the only place [`LostCause::AppEnded`] is ever set, and it is the
/// step the whole "write it early" decision exists to make possible.
///
/// Returns whether anything changed, so a startup with a clean journal costs
/// no write.
pub(crate) fn reconcile_at_startup(journal: &mut Vec<AskRecord>, now: i64) -> bool {
    let before = journal.clone();
    lose_open(journal, LostCause::AppEnded, now);
    prune(journal, now);
    *journal != before
}

/// Keep the journal small and current.
///
/// Oldest `Acknowledged` first, then oldest `Lost`: a loss the reader has
/// already been shown is worth less than one they have not.
fn prune(journal: &mut Vec<AskRecord>, now: i64) {
    journal.retain(|entry| now - entry.touched_at() < MAX_AGE_SECS);
    while journal.len() > MAX_ENTRIES {
        let victim = journal
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| {
                let tier = match entry.state {
                    AskState::Acknowledged { .. } => 0,
                    AskState::Lost { .. } => 1,
                    AskState::Open => 2,
                };
                (tier, entry.touched_at())
            })
            .map(|(index, _)| index);
        match victim {
            Some(index) => drop(journal.remove(index)),
            None => break,
        }
    }
}

/// The losses to narrate in one session's transcript, oldest first, as
/// `(tool call id, sentence)`.
///
/// Owned pairs rather than borrows on purpose: the caller reads this out of a
/// signal, and a view that held that read guard while rendering a button
/// whose handler writes the same signal is a re-entrant borrow panic.
pub(crate) fn losses_in(journal: &[AskRecord], session_id: &str) -> Vec<(String, String)> {
    journal
        .iter()
        .filter(|entry| entry.session_id == session_id && entry.is_unreported_loss())
        .map(|entry| (entry.tool_call_id.clone(), sentence(entry)))
        .collect()
}

/// How many unreported losses a session has, for the Chats list.
pub(crate) fn loss_count(journal: &[AskRecord], session_id: &str) -> usize {
    journal
        .iter()
        .filter(|entry| entry.session_id == session_id && entry.is_unreported_loss())
        .count()
}

/// The sentence a lost ask gets, in the open chat's transcript.
///
/// Written against the measurement rather than against the account it
/// falsified. There is no declined tool and no "the user declined it" note in
/// the transcript — there is nothing at all — so the copy says the round was
/// discarded and points at the prompt, which is the part that survived and
/// the part the reader can act on.
fn sentence(record: &AskRecord) -> String {
    let when = match record.state {
        AskState::Lost {
            cause: LostCause::AppEnded,
            ..
        } => "while the app was closed",
        _ => "when the connection dropped",
    };
    format!(
        "{} was waiting on your answer {when}. goose discarded the reply it \
         was working on — your message is still above. Ask again to retry.",
        record.title
    )
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::{
        acknowledge, forget_open, lose_open, loss_count, losses_in, note, prune,
        reconcile_at_startup, resolve, sentence, AskRecord, AskState, LostCause, MAX_ENTRIES,
    };

    const NOW: i64 = 1_800_000_000;

    fn ask(tool_call_id: &str) -> AskRecord {
        AskRecord::open(
            "20260827_3".to_owned(),
            "Run uname command".to_owned(),
            tool_call_id.to_owned(),
            "shell · uname -a".to_owned(),
            NOW,
        )
    }

    /// The measured case, end to end, and the one no drain-time hook can
    /// reach: the ask arrives, the process dies without running another line,
    /// and the next launch has to be able to say so.
    ///
    /// The kill is modelled by a serialize/deserialize round trip rather than
    /// by calling a "we are shutting down" function, because there is no such
    /// function — that is the point.
    #[test]
    fn an_ask_the_app_was_killed_on_is_a_loss_at_the_next_launch() {
        let mut journal = Vec::new();
        note(&mut journal, ask("call_01a0"), NOW);

        // Everything after this line is a different process.
        let carried = serde_json::to_vec(&journal).unwrap();
        let mut journal: Vec<AskRecord> = serde_json::from_slice(&carried).unwrap();

        assert!(reconcile_at_startup(&mut journal, NOW + 90));
        assert_eq!(
            journal[0].state,
            AskState::Lost {
                at: NOW + 90,
                cause: LostCause::AppEnded
            }
        );
        assert_eq!(losses_in(&journal, "20260827_3").len(), 1);
        assert!(sentence(&journal[0]).contains("shell · uname -a"));
        assert!(
            sentence(&journal[0]).contains("while the app was closed"),
            "the sentence has to say which of the two happened"
        );
    }

    /// A launch that finds nothing to fix must not write, or every cold start
    /// rewrites the file for no reason.
    #[test]
    fn a_clean_journal_costs_no_write_at_startup() {
        let mut journal = Vec::new();
        assert!(!reconcile_at_startup(&mut journal, NOW));

        let mut journal = vec![ask("call_01a0")];
        lose_open(&mut journal, LostCause::Connection, NOW);
        assert!(!reconcile_at_startup(&mut journal, NOW + 5));
        assert_eq!(
            journal[0].state,
            AskState::Lost {
                at: NOW,
                cause: LostCause::Connection
            },
            "a loss already dated must not be re-dated by a restart"
        );
    }

    /// Answering is the ordinary path and it leaves nothing behind. So does
    /// the agent withdrawing its own question.
    #[test]
    fn an_answered_or_withdrawn_ask_leaves_no_trace() {
        let mut journal = Vec::new();
        note(&mut journal, ask("call_01a0"), NOW);
        resolve(&mut journal, "call_01a0");
        assert!(journal.is_empty());

        note(&mut journal, ask("call_01a0"), NOW);
        lose_open(&mut journal, LostCause::Connection, NOW);
        // Even a loss can be resolved: goose re-raises its outstanding asks
        // on `session/load`, and an answer to the re-raised one settles it.
        resolve(&mut journal, "call_01a0");
        assert!(journal.is_empty());
    }

    /// The user pressing Disconnect is not a failure, and the app must not
    /// report it as one. This is the same code path as a dropped tailnet
    /// everywhere except in the cause the transport reports, which is why
    /// `DisconnectCause` had to be carried on the event.
    #[test]
    fn a_deliberate_disconnect_narrates_nothing() {
        let mut journal = Vec::new();
        note(&mut journal, ask("call_01a0"), NOW);
        forget_open(&mut journal);
        assert!(journal.is_empty());
    }

    /// goose re-raises the asks it still holds when a session is loaded. An
    /// entry this journal already gave up for lost has to become open again,
    /// or the reader is told the round was lost while it is in front of them.
    #[test]
    fn a_re_raised_ask_takes_its_own_record_back() {
        let mut journal = Vec::new();
        note(&mut journal, ask("call_01a0"), NOW);
        lose_open(&mut journal, LostCause::Connection, NOW + 1);
        assert_eq!(loss_count(&journal, "20260827_3"), 1);

        note(&mut journal, ask("call_01a0"), NOW + 30);
        assert_eq!(journal.len(), 1, "two records for one ask");
        assert!(journal[0].is_open());
        assert_eq!(loss_count(&journal, "20260827_3"), 0);
    }

    /// A dismissal has to survive the next sighting of the journal, and it
    /// has to stop the card coming back.
    #[test]
    fn a_dismissed_loss_stops_being_reported() {
        let mut journal = Vec::new();
        note(&mut journal, ask("call_01a0"), NOW);
        lose_open(&mut journal, LostCause::Connection, NOW);
        acknowledge(&mut journal, "call_01a0", NOW + 5);
        assert_eq!(loss_count(&journal, "20260827_3"), 0);
        assert!(losses_in(&journal, "20260827_3").is_empty());

        // And a restart must not resurrect it.
        assert!(!reconcile_at_startup(&mut journal, NOW + 10));
        assert_eq!(loss_count(&journal, "20260827_3"), 0);
    }

    /// One session's losses are not another's — the modal already names the
    /// session an ask came from, and the transcript card is per-chat.
    #[test]
    fn losses_are_reported_against_the_session_that_lost_them() {
        let mut journal = Vec::new();
        note(&mut journal, ask("call_a"), NOW);
        let mut other = ask("call_b");
        other.session_id = "20260827_4".to_owned();
        note(&mut journal, other, NOW);
        lose_open(&mut journal, LostCause::Connection, NOW);

        assert_eq!(loss_count(&journal, "20260827_3"), 1);
        assert_eq!(loss_count(&journal, "20260827_4"), 1);
        assert_eq!(loss_count(&journal, "20260827_9"), 0);
        assert_eq!(losses_in(&journal, "20260827_3")[0].0, "call_a");
    }

    /// The cap is on the write path of every permission ask, so it has to
    /// hold, and it has to give up the least useful entry: one already shown
    /// to the reader before one that never was.
    #[test]
    fn the_cap_sheds_what_has_already_been_read() {
        let mut journal = Vec::new();
        for n in 0..MAX_ENTRIES {
            note(&mut journal, ask(&format!("call_{n}")), NOW);
            lose_open(
                &mut journal,
                LostCause::Connection,
                NOW + n.try_into().unwrap_or(0),
            );
        }
        acknowledge(&mut journal, "call_5", NOW);
        note(&mut journal, ask("call_new"), NOW + 100);

        assert_eq!(journal.len(), MAX_ENTRIES);
        assert!(
            !journal.iter().any(|e| e.tool_call_id == "call_5"),
            "the dismissed one should have gone first"
        );
        assert!(journal.iter().any(|e| e.tool_call_id == "call_0"));
        assert!(journal.iter().any(|e| e.tool_call_id == "call_new"));
    }

    /// A notification queue, not an archive: a loss nobody came back for
    /// inside a week is not news any more.
    #[test]
    fn a_stale_entry_ages_out() {
        let mut journal = vec![ask("call_old")];
        lose_open(&mut journal, LostCause::Connection, NOW);
        prune(&mut journal, NOW + 8 * 86_400);
        assert!(journal.is_empty());
    }

    /// The claim the whole design rests on, run rather than read: [`Backing`]
    /// actually writes a file, and a value put through it comes back after
    /// the process that wrote it is gone.
    ///
    /// This is a gate and not a demonstration. The obvious backing —
    /// `use_persistent`, which `settings` and `code_cache` use — resolves to
    /// an in-memory `HashMap` on every target this app builds for. Everything
    /// else in this file would still typecheck and still pass with it, and
    /// the journal would evaporate on exactly the event it exists to survive.
    /// So the alias is what `crate::state` is required to name, and this is
    /// what the alias is required to be.
    ///
    /// `set_directory` writes a process-wide `OnceLock` and `.unwrap()`s the
    /// result, so exactly ONE caller in a test binary may set it. This test
    /// used to be that caller and claimed the binary for itself; it is not any
    /// more, because `crate::testkit` has to reach the same storage to mount a
    /// view and the second caller panics. `testkit::storage_dir` is the single
    /// owner and hands back the path — still a temp path, so `cargo test`
    /// writes nothing anyone would keep.
    #[test]
    fn the_journals_storage_backing_really_reaches_the_disk() {
        use super::Backing;
        use dioxus_sdk_storage::StorageBacking;

        let dir = crate::testkit::storage_dir();

        let mut journal = Vec::new();
        note(&mut journal, ask("call_01a0"), NOW);
        Backing::set("lost_asks_test".to_owned(), &journal);

        assert!(
            dir.join("lost_asks_test").is_file(),
            "the journal's backing wrote no file, so nothing here survives a \
             restart — which is the one thing it is for"
        );
        let read: Option<Vec<AskRecord>> = Backing::get(&"lost_asks_test".to_owned());
        assert_eq!(read.as_ref(), Some(&journal));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
