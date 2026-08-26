//! Every method this crate sends, checked against goose's own declaration of
//! it — without a socket, and in milliseconds.
//!
//! The bug this catches is the one nothing else can: a method string or a
//! request key that both sides' unit tests are happy with because both sides'
//! unit tests only ever talk to themselves. `scheduleId` spelled
//! `schedule_id` is a `-32602` on a real server and a green suite here, and
//! goose sets `deny_unknown_fields` on nothing, so several of those spellings
//! are not even an error — they are a call that succeeds and does nothing.
//!
//! # Where the declaration comes from
//!
//! `tests/fixtures/acp-meta.json` is goose's own file, verbatim, and
//! `tests/fixtures/acp-request-keys.json` is the request half of its
//! `acp-schema.json` resolved down to `{method: {keys, required}}`. Both are
//! written by `scripts/vendor-acp-contract.py` and carry the goose commit
//! they came from. Vendored rather than read from a sibling checkout because
//! this has to run on a machine that has never cloned goose; derived rather
//! than copied because the schema is 246 KB and the question asked of it is
//! two lists per method.
//!
//! # Why the params are literals
//!
//! An integration test links the non-test build of the crate and cannot see a
//! private `fn id_params`. So each feature module in `src/goose/` carries a
//! unit test — `requests_use_goose_casing` — pinning its builders to the same
//! literals that appear here. The chain is: builder == literal (unit test),
//! literal ⊆ goose's keys (here).

// Test code: a failing unwrap, or a panic on a malformed fixture, IS the
// failing check. `expect` rather than `allow`: if a use goes away, so should
// its exception.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test harness: an unwrap or a panic is the assertion"
)]

// `#[path]` because this file is a test target's crate root, so a bare
// `mod scheduler;` would look for `tests/scheduler.rs` — which cargo would
// then also compile as a test binary of its own. The feature modules live in
// `tests/contract/`, which cargo does not auto-discover.
#[path = "contract/scheduler.rs"]
mod scheduler;

use std::collections::BTreeSet;

use serde_json::Value;

/// One call this app makes: the method string, and a request body in the
/// shape the crate builds one.
#[derive(Debug)]
pub(crate) struct Sample {
    pub method: &'static str,
    pub params: fn() -> Value,
}

/// Every feature's samples, alphabetical, one line each.
///
/// A slice of slices rather than one flat list because `const` slices cannot
/// be concatenated — and because it makes adding a feature exactly one line,
/// which is the arrangement `nav.rs`'s destination table and the mock's
/// `HANDLERS` array already use for the same merge reason.
const GROUPS: &[&[Sample]] = &[
    // extensions — PR 2 replaces this line
    //
    // recipes — PR 3 replaces this line
    //
    scheduler::SAMPLES,
    //
    // skills — PR 4 replaces this line
    //
    // session history — PR 7 replaces this line
];

const META: &str = include_str!("fixtures/acp-meta.json");
const REQUEST_KEYS: &str = include_str!("fixtures/acp-request-keys.json");

fn meta() -> Value {
    serde_json::from_str(META).unwrap()
}

fn request_keys() -> Value {
    serde_json::from_str(REQUEST_KEYS).unwrap()
}

fn samples() -> Vec<&'static Sample> {
    GROUPS.iter().copied().flatten().collect()
}

/// The method strings goose declares, from the file goose generates.
fn declared_methods() -> BTreeSet<String> {
    meta()["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["method"].as_str().unwrap().to_string())
        .collect()
}

/// The strongest typo check in the stack: a method goose has never heard of.
///
/// On a real server this is a `-32601`, which this crate deliberately reads as
/// "the feature is switched off" — so a mis-typed method does not surface as
/// an error at all. It draws the screen that says the server does not have
/// the feature, over a server that does.
#[test]
fn every_method_this_app_sends_is_one_goose_declares() {
    let declared = declared_methods();
    for sample in samples() {
        assert!(
            declared.contains(sample.method),
            "goose declares no method `{}` — a -32601, which this crate reads \
             as a feature being switched off rather than as a typo",
            sample.method,
        );
    }
}

/// Every key sent is one goose declares, and every key goose requires is sent.
///
/// Both directions matter and they fail differently. An undeclared key is
/// dropped in silence — serde on the server ignores it — so the call succeeds
/// having done nothing. A missing required key is a `-32602`, which is loud
/// but only ever at runtime, against a real server, on a phone.
#[test]
fn every_request_key_is_one_goose_declares() {
    let index = request_keys();
    for sample in samples() {
        let declared = &index["methods"][sample.method];
        assert!(
            !declared.is_null(),
            "no request-key entry for `{}` — re-run scripts/vendor-acp-contract.py",
            sample.method,
        );
        let keys: BTreeSet<&str> = declared["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();

        let params = (sample.params)();
        let sent = params.as_object().unwrap_or_else(|| {
            panic!(
                "{} builds params that are not an object: {params}",
                sample.method
            )
        });
        for key in sent.keys() {
            assert!(
                keys.contains(key.as_str()),
                "{} sends `{key}`, which goose does not declare — serde drops \
                 it and answers success. goose declares: {keys:?}",
                sample.method,
            );
        }
        for required in declared["required"].as_array().unwrap() {
            let required = required.as_str().unwrap();
            assert!(
                sent.contains_key(required),
                "{} omits `{required}`, which goose marks required (-32602)",
                sample.method,
            );
        }
    }
}

/// Half a regeneration is worse than none: the method list and the key index
/// disagreeing would let a method pass one check against one release of goose
/// and the other against a different one.
#[test]
fn the_two_vendored_files_came_from_one_goose() {
    let (meta, keys) = (meta(), request_keys());
    let (a, b) = (&meta["_source"], &keys["_source"]);
    assert!(a.is_string() && a == b, "vendored from {a} and {b}");
}

/// A scan that silently matched nothing would pass forever, and this file's
/// whole job is to be the thing that notices.
#[test]
fn the_aggregator_is_not_empty() {
    assert!(
        !samples().is_empty(),
        "no feature registered a group in GROUPS"
    );
    let mut methods: Vec<&str> = samples().iter().map(|s| s.method).collect();
    let count = methods.len();
    methods.sort_unstable();
    methods.dedup();
    assert_eq!(methods.len(), count, "a method is sampled twice");
}
