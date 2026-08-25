//! Recipes, driven end to end: the real binary, a real socket, the real
//! client.
//!
//! Neither side's unit tests can see the bug this file exists for. The client
//! pins `_goose/unstable/recipes/list` against a `json!` literal; the mock
//! answers a method its own tests hand it by name; a `recipe` vs `recipes` in
//! either constant passes both and shows an empty screen against a server
//! holding three recipes. Only the string travelling between two
//! implementations catches that, so the flow below sends every method this
//! feature added.

// Test code: a failing unwrap IS the failing check. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test harness: an unwrap or a panic is the assertion"
)]

mod common;

use goose_acp_client::{AcpError, RecipeListEntry};

fn find<'a>(entries: &'a [RecipeListEntry], id: &str) -> Option<&'a RecipeListEntry> {
    entries.iter().find(|entry| entry.id == id)
}

/// list -> scan -> schedule -> list again -> delete, with the assertions that
/// only mean something once the bytes have travelled: that the mock's ordering
/// survives, that fields the client does not model come back in `extra`, and
/// that a mutation is visible to the next read.
#[tokio::test]
async fn recipes_round_trip_over_a_real_socket() {
    let (mut server, client) = common::spawn_mock().await;

    let recipes = client.recipes_list().await.unwrap();
    assert_eq!(recipes.len(), 3);

    // Newest file first, which is the order goose sorts its manifests in and
    // the order the screen renders without sorting again.
    let research = &recipes[0];
    assert!(
        research.recipe.title.chars().count() > 60,
        "the stress fixture lost its long title: {}",
        research.recipe.title
    );
    assert!(research.recipe.description.chars().count() > 200);

    // The round-trip guarantee, over the wire rather than over a fixture: the
    // client models neither of these and still has to hand them back.
    assert!(
        research.recipe.extra.contains_key("sub_recipes"),
        "sub_recipes was dropped: {:?}",
        research.recipe.extra.keys().collect::<Vec<_>>()
    );
    assert_eq!(research.recipe.extra["retry"]["max_retries"], 2);
    assert!(research.needs_input());

    let standup = find(&recipes, "1c9d4f2a6b083e57").unwrap();
    assert!(standup.is_scheduled());
    assert_eq!(standup.slash_command.as_deref(), Some("standup"));
    assert_eq!(standup.input_count(), 0);

    let review = find(&recipes, "8e35b0a7d914c26f").unwrap();
    assert!(!review.is_scheduled());
    assert_eq!(review.input_count(), 2);
    assert!(review.needs_input());

    // scan: the flagged recipe and an unflagged one, so a mock that answered
    // a constant would fail one of them.
    assert!(client.recipes_scan(&research.recipe).await.unwrap());
    assert!(!client.recipes_scan(&standup.recipe).await.unwrap());

    // The bare URL-safe base64 `recipe_deeplink::encode` returns — no
    // `goose://` wrapper, and no padding.
    let deeplink = client.recipes_encode(&standup.recipe).await.unwrap();
    assert!(
        !deeplink.contains('=') && !deeplink.contains("://"),
        "{deeplink}"
    );
    assert!(
        deeplink.starts_with("eyJ"),
        "not base64 of a JSON object: {deeplink}"
    );

    // schedule one, unschedule the other, and read both back.
    client
        .recipes_schedule(&review.id, Some("0 7 * * 1-5"))
        .await
        .unwrap();
    client.recipes_schedule(&standup.id, None).await.unwrap();

    let recipes = client.recipes_list().await.unwrap();
    assert_eq!(
        find(&recipes, &review.id).unwrap().schedule_cron.as_deref(),
        Some("0 7 * * 1-5")
    );
    assert!(!find(&recipes, &standup.id).unwrap().is_scheduled());

    client.recipes_delete(&review.id).await.unwrap();
    let recipes = client.recipes_list().await.unwrap();
    assert_eq!(recipes.len(), 2);
    assert!(find(&recipes, &review.id).is_none());

    // An id nothing on disk answers to. `-32602` with a reason, reaching the
    // client as an `Rpc` error rather than an `Unsupported` one: the feature
    // is present, this call was wrong.
    let error = client.recipes_delete(&review.id).await.unwrap_err();
    match error {
        AcpError::Rpc { code, data, .. } => {
            assert_eq!(code, -32602);
            let reason = data.unwrap();
            assert!(
                reason.as_str().unwrap().contains(&review.id),
                "the reason does not name the id: {reason}"
            );
        }
        other => panic!("expected an RPC error, got {other:?}"),
    }

    // Recipes are request/response only. Anything pushed at the client during
    // all of that would be a notification the app has no handler for.
    assert!(
        server.events.try_recv().is_err(),
        "a recipe call pushed a notification"
    );
}

/// A goose started without `--enable-scheduler`, end to end: the `-32601` and
/// the sentence in `data` have to survive the socket and arrive as
/// [`AcpError::Unsupported`] with the reason intact, because that pair is what
/// the detail screen reads — `is_unsupported` retires the Schedule control and
/// the reason is the copy under it.
#[tokio::test]
async fn a_scheduler_less_server_refuses_to_schedule() {
    let (_server, client) = common::spawn_mock_with(&[("MOCK_NO_SCHEDULER", "1")]).await;

    let recipes = client.recipes_list().await.unwrap();
    let review = find(&recipes, "8e35b0a7d914c26f").unwrap();

    let error = client
        .recipes_schedule(&review.id, Some("0 7 * * 1-5"))
        .await
        .unwrap_err();
    match error {
        AcpError::Unsupported { reason, .. } => assert_eq!(
            reason.as_deref(),
            Some("Scheduled recipe execution is not enabled")
        ),
        other => panic!("expected an unsupported error, got {other:?}"),
    }

    // Nothing was scheduled, and the rest of the screen still works: the flag
    // switches off the scheduler, not recipes.
    let recipes = client.recipes_list().await.unwrap();
    assert!(!find(&recipes, &review.id).unwrap().is_scheduled());
    assert!(!client.recipes_scan(&review.recipe).await.unwrap());
}
