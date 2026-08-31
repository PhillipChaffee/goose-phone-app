//! Mounting a real view, with a real [`AppCtx`] under it, and reading the
//! markup back.
//!
//! Every view in `src/views/` is a Dioxus component, and until this existed
//! nothing in the suite could run one: `cargo test` builds for the host, the
//! components take their state from a context that only `src/app.rs` provided,
//! and there was no way to see what came out. So the whole of `src/views/` was
//! measured at or near zero — `views/settings.rs`, `views/recipes.rs`,
//! `views/extensions.rs`, `views/mod.rs` and `views/attach.rs` were all
//! **0.00%** — and the parts of `src/state.rs` that only a rendered component
//! reaches were at 20.85%.
//!
//! THE POINT IS THE ASSERTION, NOT THE EXECUTION. A `VirtualDom` runs a
//! component's body on `rebuild_in_place`, and that alone moves the coverage
//! number — measured, on one bare render of `SettingsView`: `views/settings.rs`
//! went 0.00% → 13.70%. A suite built on that and nothing else would be tests
//! that say "it did not panic", which is exactly the shape of check this
//! repository has already been burned by (see `src/selfscan.rs` for the last
//! one, where 231 assertions passed over a deleted feature). `dioxus-ssr` is a
//! dev-dependency so a view test can say what the view PUT ON SCREEN, and the
//! 95% bar means something.
//!
//! What this cannot mount, and why it is not a gap in the harness:
//! `src/shell/desktop/`'s `AppShell` calls `dioxus::desktop::window()`, which
//! is `consume_context()` (`dioxus-desktop-0.7.10/src/desktop_context.rs:34`)
//! and panics without a real event loop. That is the one thing
//! `src/selfscan.rs` says to solve by scanning source rather than by faking a
//! context, and it still is. Views are different: they read `AppCtx` and
//! nothing else, which is precisely what this provides.

use dioxus::prelude::*;

use crate::state::AppCtx;

/// What a harness renders: the state to put under the view, and the view.
///
/// Both are plain `fn` pointers rather than closures, so the struct is `Copy`
/// and needs no boxing to cross into a component's props.
#[derive(Clone, Copy)]
pub(crate) struct Mount {
    seed: fn(&AppCtx),
    view: fn() -> Element,
}

/// Dioxus requires props to be `PartialEq` so it can skip re-rendering a child
/// whose props did not change. This answers `false` — always re-render — and
/// that is the correct answer rather than a shortcut.
///
/// Deriving it would compare the two `fn` pointers, which clippy rejects and
/// is right to: "function pointer comparisons do not produce meaningful
/// results since their addresses are not guaranteed to be unique". The same
/// function can hold different addresses in different codegen units, and two
/// different functions can be merged to one address — so a derived `eq` could
/// answer either way about the same pair.
///
/// Nothing is lost. A harness mounts once per call, renders once, and is
/// dropped; there is no second render for memoisation to skip.
impl PartialEq for Mount {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

/// Provide the context, seed it, then render the view under it.
///
/// The seed runs in a `use_hook`, so it happens once, BEFORE the view's body
/// is called in the same render pass — which is what makes
/// [`render_seeded`] able to put a view into a state the app would need a
/// server round trip to reach.
#[expect(
    non_snake_case,
    reason = "a Dioxus component is named like a component, not like a fn"
)]
fn Harness(props: Mount) -> Element {
    let ctx = crate::state::use_app_ctx_provider();
    use_hook(|| (props.seed)(&ctx));
    (props.view)()
}

/// Render a view with an untouched [`AppCtx`] and hand back its markup.
///
/// The default context is the one the app launches with, which
/// `use_app_ctx_provider` documents: disconnected, on Settings, every list
/// empty. That is a real state — it is what the window shows before anyone
/// types a URL — so it is worth asserting on rather than only worth passing
/// through.
pub(crate) fn render(view: fn() -> Element) -> String {
    render_seeded(|_| {}, view)
}

/// Render a view after putting the context into a chosen state.
///
/// This is the half that reaches the branches. A list view has an empty arm, a
/// loading arm, a not-connected arm and a rows arm, and only the first is
/// reachable without seeding — so a suite without this would report the same
/// handful of lines covered on every screen and call it coverage.
pub(crate) fn render_seeded(seed: fn(&AppCtx), view: fn() -> Element) -> String {
    let _ = storage_dir();
    let mut dom = VirtualDom::new_with_props(Harness, Mount { seed, view });
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// Render a view and then let its ASYNC work finish before reading the markup.
///
/// [`render_seeded`] renders exactly one pass, which is the right thing for a
/// view whose output is a pure function of the context. It is the wrong thing
/// for one that fetches on mount: a `use_effect` that spawns has not run when
/// the first pass ends, so the view is caught mid-flight and the test asserts
/// on a loading state it did not ask for.
///
/// The loop is bounded on purpose. An effect that has not fired after eight
/// 20ms slices is not going to, and the point of a bound is that a view whose
/// tasks never settle cannot hang the suite — which matters more than usual
/// here, because two `VirtualDom`s sharing `dioxus-sdk-storage`'s process-wide
/// subscription map can feed each other and spin (measured at 7 wedged runs in
/// 40 while that was unguarded).
///
/// The runtime is current-thread with a timer, entered only so the `tokio`
/// sleeps inside the app's own spawned tasks can be constructed. Nothing here
/// waits on wall-clock time.
pub(crate) fn render_settled(seed: fn(&AppCtx), view: fn() -> Element) -> String {
    const SETTLE_PASSES: usize = 8;
    const SETTLE_SLICE: std::time::Duration = std::time::Duration::from_millis(20);

    let _ = storage_dir();
    let mut dom = VirtualDom::new_with_props(Harness, Mount { seed, view });
    dom.rebuild_in_place();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build();
    if let Ok(runtime) = runtime {
        runtime.block_on(async {
            for _ in 0..SETTLE_PASSES {
                let _ = tokio::time::timeout(SETTLE_SLICE, dom.wait_for_work()).await;
                dom.render_immediate_to_vec();
            }
        });
    }
    dioxus_ssr::render(&dom)
}

/// WHERE THE TEST BINARY'S PERSISTENT STORAGE GOES, and the single place
/// allowed to decide it.
///
/// Two facts collide here and both were met the hard way.
///
/// The first: without a directory, mounting anything renders an empty string
/// and says nothing. `use_app_ctx_provider` reaches
/// `use_synced_storage::<LocalStorage, _>` for the ask journal, `LocalStorage`
/// is filesystem-backed (`dioxus-sdk-storage-0.7.0/src/client_storage/fs.rs`),
/// and with no directory set it panics with "Call the `set_dir` macro before
/// accessing persistant data". Dioxus catches a panic thrown during render and
/// renders nothing, so the symptom is not a failing test — it is
/// `dioxus_ssr::render` returning **0 bytes** while the component's body still
/// counts as executed. An earlier spike of this harness "passed" for exactly
/// that reason, and the coverage number moved anyway
/// (`views/settings.rs` 0.00% → 13.70%). A number going up is not a view
/// working.
///
/// The second: `set_directory` writes a process-wide `OnceLock` and
/// `.unwrap()`s the result (`fs.rs:15`), so the SECOND caller in a test binary
/// panics — and `ask_journal`'s
/// `the_journals_storage_backing_really_reaches_the_disk` was already the
/// first, its comment claiming it "owns it for the whole binary". That claim
/// was true when it was written and this file falsified it: run alone the
/// harness passed, run in the full suite all three of its tests died, one on
/// the `unwrap` and the others on the poisoned `Once` behind it. Order- and
/// parallelism-dependent failure is the worst possible property for a suite
/// about to become a merge gate.
///
/// So there is one owner, it is this function, and `ask_journal`'s test calls
/// it rather than setting a directory of its own. `OnceLock` rather than
/// `Once` so the path can be handed back — that test needs it to look for the
/// file it wrote.
///
/// A per-process temp directory, which is `ask_journal`'s original choice and
/// its reason: `cargo test` writes nothing anyone would keep, and two test
/// binaries running at once cannot land on the same path.
pub(crate) fn storage_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("goose-mobile-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        // What `set_dir!()` expands to on a non-wasm target, minus the macro,
        // which only takes a literal.
        dioxus_sdk_storage::set_directory(dir.clone());
        dir
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::{render, render_seeded};

    use dioxus::prelude::*;

    /// The harness itself, checked before anything is built on it.
    ///
    /// A harness that silently rendered nothing would make every test written
    /// against it pass while asserting on an empty string, and the coverage
    /// number would go up either way. So this asserts on real markup from a
    /// real view: `SettingsView` puts the server card's heading on screen.
    #[test]
    fn a_view_renders_with_a_real_context_under_it() {
        let html = render(|| rsx! { crate::views::settings::SettingsView {} });
        assert!(
            html.contains("Server"),
            "the settings view rendered {} bytes and none of them are its \
             server card — the harness is mounting something that is not the \
             view: {}",
            html.len(),
            &html[..html.len().min(400)]
        );
    }

    /// The seed reaches the view, which is the whole reason `render_seeded`
    /// exists — without it every test sees the launch state and the branch
    /// arms go unmeasured.
    ///
    /// Asserted on a ROW rather than on the screen's furniture, deliberately.
    /// The topbar and the FAB render whatever the state is, so a test that
    /// looked for those would pass with the seed thrown away; a session's own
    /// title can only be on screen because the seeded list reached the view.
    /// It also crosses the empty/rows branch, which is the arm an unseeded
    /// mount can never take.
    #[test]
    fn the_seed_is_visible_to_the_view_it_mounts() {
        let empty = render(|| rsx! { crate::views::sessions::SessionsView {} });
        assert!(
            empty.contains("No sessions yet"),
            "an unseeded mount should be on the empty arm: {}",
            &empty[..empty.len().min(300)]
        );

        let seeded = render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![goose_acp_client::SessionInfo {
                    session_id: "s-1".to_owned(),
                    cwd: None,
                    title: Some("Tailscale certificate rotation".to_owned()),
                    updated_at: None,
                    meta: None,
                }]);
            },
            || rsx! { crate::views::sessions::SessionsView {} },
        );
        assert!(
            seeded.contains("Tailscale certificate rotation"),
            "the seeded session's title never reached the markup, so the seed \
             is not visible to the view it mounts: {}",
            &seeded[..seeded.len().min(400)]
        );
        assert!(
            !seeded.contains("No sessions yet"),
            "the view rendered rows AND the empty state at once"
        );
    }

    /// Two mounts do not share state.
    ///
    /// `use_app_ctx_provider` builds its signals with `use_signal` inside the
    /// harness's own scope, so each `VirtualDom` owns its own — but it also
    /// reaches `dioxus_sdk_storage`, which on non-wasm targets is an in-memory
    /// map hung off a root context (`src/state.rs` records the measurement).
    /// If that map were process-global, one test's seed would leak into the
    /// next and failures would depend on test ORDER, which is the worst
    /// possible property for a suite about to become a merge gate.
    #[test]
    fn one_mount_does_not_leak_into_the_next() {
        let seeded = render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![goose_acp_client::SessionInfo {
                    session_id: "s-1".to_owned(),
                    cwd: None,
                    title: Some("Leaky session".to_owned()),
                    updated_at: None,
                    meta: None,
                }]);
            },
            || rsx! { crate::views::sessions::SessionsView {} },
        );
        assert!(
            seeded.contains("Leaky session"),
            "the seeded mount lost its own state before the comparison"
        );

        let fresh = render(|| rsx! { crate::views::sessions::SessionsView {} });
        assert!(
            !fresh.contains("Leaky session"),
            "a second mount inherited the first one's seeded session, so the \
             signals are process-global and every test's result depends on the \
             order the suite happened to run in"
        );
        assert!(
            fresh.contains("No sessions yet"),
            "a fresh mount is not on the empty arm: {}",
            &fresh[..fresh.len().min(300)]
        );
    }
}
