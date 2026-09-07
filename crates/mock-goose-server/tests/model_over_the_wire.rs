//! The two methods a client asks BEFORE it has a session:
//! `_goose/unstable/config/read` and `_goose/unstable/providers/list`.
//!
//! They exist for one screen. The chat home composer has to be able to say
//! what a conversation started right now would run on, and every other route
//! to that answer in this protocol needs a session that does not exist yet —
//! which is why against a real server the home had no model chip at all
//! (#280). A method string that the two crates spell differently would put it
//! straight back, silently, and neither crate's own tests could see it: that
//! is what `common::spawn_mock` is for.

// Test code: a failing unwrap IS the failing check. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test harness: an unwrap or an expect is the assertion, and the \
              expect's message names what was missing"
)]

mod common;

use goose_acp_client::GlobalKey;

/// The invariant the whole reconstruction rests on, asserted against the fake
/// the way it was measured against `goose 1.46.0`: the `model` option built
/// with NO session is the `model` option a session carries.
///
/// On the real server that was 168 choices in the same order with the same
/// names; here it is four. What matters is that the fake cannot drift — if
/// `providers/list` and `session/new` ever answer different catalogues, the
/// home composer starts offering models the session will refuse, and this is
/// the only test in either crate that would notice.
#[tokio::test]
async fn the_model_option_is_the_same_with_and_without_a_session() {
    let (mut server, client) = common::spawn_mock().await;

    let without = client
        .default_model_option()
        .await
        .unwrap()
        .expect("the fake has a model configured");

    // NOTHING ARRIVED ON THE NOTIFICATION STREAM, which is the other half of
    // "before a session": these three calls name no session id, so they can
    // neither replay a transcript into an open conversation nor push a
    // `config_option_update` at one. A reader on the home screen has an open
    // chat behind them more often than not.
    assert!(
        server.events.try_recv().is_err(),
        "asking what the next chat runs on must not touch any session"
    );

    let session = client.session_new("/home/demo").await.unwrap();
    let with = session
        .config_options
        .into_iter()
        .find(|option| option.config_id == "model")
        .expect("a new session carries a model option");

    assert_eq!(without, with);
    assert!(without.is_adjustable(), "four models is a control");
    assert_eq!(without.current_label(), Some("Claude Sonnet 5"));
}

/// Both global keys travel, and a key the server has no value for comes back
/// as an absence rather than as an error — which is how a goose that has
/// never been configured is told apart from one that is unreachable.
#[tokio::test]
async fn the_two_global_keys_come_back_over_the_wire() {
    let (_server, client) = common::spawn_mock().await;

    assert_eq!(
        client
            .read_global(GlobalKey::Model)
            .await
            .unwrap()
            .as_deref(),
        Some("claude-sonnet-5")
    );
    assert_eq!(
        client
            .read_global(GlobalKey::Provider)
            .await
            .unwrap()
            .as_deref(),
        Some("anthropic")
    );
}

/// The catalogue itself: every provider, configured or not, and the models on
/// the one the config names.
#[tokio::test]
async fn the_provider_list_travels_with_its_catalogue() {
    let (_server, client) = common::spawn_mock().await;

    let providers = client.providers().await.unwrap();
    assert_eq!(providers.len(), 3);

    let anthropic = providers
        .iter()
        .find(|entry| entry.provider_id == "anthropic")
        .expect("the configured provider");
    assert!(anthropic.configured);
    assert_eq!(anthropic.provider_name, "Anthropic");
    assert_eq!(anthropic.models.len(), 4);
    assert_eq!(anthropic.models[0].id, "claude-opus-5");
    assert_eq!(anthropic.models[0].name.as_deref(), Some("Claude Opus 5"));

    // An unconfigured provider is in the list too — the reply says what the
    // agent KNOWS about, not what it can use, and telling the two apart is
    // the client's job.
    assert!(providers.iter().any(|entry| !entry.configured));

    // The fields this crate does not model are still on the wire, where the
    // round-trip check can see them. `configKeys` is the one worth naming: it
    // carries the NAMES of a provider's settings and a `secret` flag beside
    // each, and never a value.
    assert!(anthropic.extra.contains_key("configKeys"));
}
