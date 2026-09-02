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

/// Run a closure against a live [`AppCtx`] and hand back what it returns.
///
/// The primitive for code that takes a context and produces a VALUE rather
/// than markup — the row builders in `src/shell/desktop/sidebar.rs` are the
/// first, and every feature module has some. Rendering those through
/// [`render_seeded`] and asserting on HTML would be asking the wrong question:
/// the answer is a `Vec<Row>`, and a test that could only see the markup could
/// not tell a row ordered wrongly from a row styled wrongly.
///
/// One mount, so the signals are real, `use_hook` fires, and the storage
/// backing is the one [`storage_dir`] owns. The closure runs inside the
/// runtime, which is what makes `Signal::set` and `peek` legal in it.
///
/// `seed` is a `fn` pointer for [`Mount`]'s reason; `f` is a closure because it
/// returns a value and nothing needs to compare it.
#[expect(
    clippy::expect_used,
    reason = "a probe that rendered without publishing its context is a broken \
              harness, and every test built on it would assert against nothing \
              — failing loudly here is the whole point"
)]
pub(crate) fn with_ctx<T>(seed: fn(&AppCtx), f: impl FnOnce(&AppCtx) -> T) -> T {
    let _ = storage_dir();
    let captured: std::rc::Rc<std::cell::RefCell<Option<AppCtx>>> = std::rc::Rc::default();
    let sink = std::rc::Rc::clone(&captured);
    let mut dom = VirtualDom::new_with_props(
        Probe,
        ProbeProps {
            seed,
            sink: SinkCell(sink),
        },
    );
    dom.rebuild_in_place();
    let ctx = captured
        .borrow_mut()
        .take()
        .expect("the probe rendered, so it published its context");
    dom.in_runtime(|| f(&ctx))
}

/// Somewhere for [`with_ctx`]'s probe to publish the context it built.
///
/// A newtype only so the props can be `PartialEq` without comparing an `Rc`'s
/// contents — which would need `AppCtx: PartialEq`, which it is not and should
/// not be: it is fifty signals, and equality on it would mean reading all of
/// them.
#[derive(Clone)]
struct SinkCell(std::rc::Rc<std::cell::RefCell<Option<AppCtx>>>);

impl PartialEq for SinkCell {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

/// `PartialEq` by hand for [`Mount`]'s reason: a derive would compare the `fn`
/// pointer, which clippy rejects because the same function can hold different
/// addresses in different codegen units. A probe mounts once and is dropped,
/// so there is no second render for memoisation to skip.
#[derive(Clone)]
struct ProbeProps {
    seed: fn(&AppCtx),
    sink: SinkCell,
}

impl PartialEq for ProbeProps {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

#[expect(
    non_snake_case,
    reason = "a Dioxus component is named like a component, not like a fn"
)]
fn Probe(props: ProbeProps) -> Element {
    // Destructured rather than read through `props`, so the value is genuinely
    // consumed: `ProbeProps` holds an `Rc` and so is not `Copy` like `Mount`,
    // and clippy's `needless_pass_by_value` is right that taking it by value
    // and only borrowing would be a pointless move.
    let ProbeProps { seed, sink } = props;
    let ctx = crate::state::use_app_ctx_provider();
    use_hook(move || {
        seed(&ctx);
        *sink.0.borrow_mut() = Some(ctx);
    });
    rsx! {}
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
        anchor_subscriptions();
        dir
    })
    .clone()
}

/// Hold one receiver open, for the life of the test binary, on every storage
/// key the app subscribes to.
///
/// `dioxus-sdk-storage` keeps its subscription map in a process-global
/// `static` and `LocalStorage::set` does `subscription.tx.send(...).unwrap()`
/// (`client_storage/fs.rs:64-72`). A `broadcast::Sender` with no live
/// receivers answers `Err`, so the moment the last mounted `VirtualDom` is
/// dropped, the NEXT write to that key panics — in whatever test happens to be
/// running, not in the one that dropped the dom.
///
/// That is order- and load-dependent, which is why it survived local runs and
/// surfaced on CI: `cargo test --workspace` on the Linux runner failed
/// `nav::tests::a_pushed_screen_is_not_where_the_drawer_says_you_are` with 794
/// others passing, inside `fs.rs` rather than anywhere in this repository.
///
/// Two modules already worked around it by taking a mutex around their own
/// mounts. That is a per-module fix for a process-global hazard, and it only
/// protects the modules that remember — `nav.rs` gained mounting tests later
/// and did not. Subscribing here instead makes the sender's receiver count
/// permanently non-zero, so `send` cannot fail whoever writes and whenever.
///
/// `lost_asks` is the whole list: it is the one key reached through
/// `use_synced_storage` (`src/state.rs`), which is the only API that
/// subscribes. `settings` and `code_cache` go through `use_persistent`, which
/// is an in-memory map with no channel at all.
fn anchor_subscriptions() {
    use dioxus_sdk_storage::StorageSubscriber;
    // Leaked on purpose: the receiver has to outlive every test in the binary,
    // and a `static` holding it would need a type this crate does not name.
    // One allocation, once, for the length of the process.
    let held =
        <crate::ask_journal::Backing as StorageSubscriber<crate::ask_journal::Backing>>::subscribe::<
            Vec<crate::ask_journal::AskRecord>,
        >(&"lost_asks".to_owned());
    std::mem::forget(held);
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

    /// A WRITE AFTER THE LAST MOUNT IS DROPPED MUST NOT PANIC.
    ///
    /// This is the CI failure, reduced to its mechanism.
    /// `dioxus-sdk-storage` keeps its subscription map in a process-global
    /// `static` and `LocalStorage::set` does `subscription.tx.send(..).unwrap()`
    /// (`client_storage/fs.rs:64-72`). A `broadcast::Sender` with no live
    /// receivers answers `Err`, so once every mounted `VirtualDom` has been
    /// dropped the next write to that key panics — inside the dependency, in
    /// whatever test is running, not in the one that dropped the dom.
    ///
    /// It failed exactly once on the Linux runner, in
    /// `nav::tests::a_pushed_screen_is_not_where_the_drawer_says_you_are`,
    /// with 794 other tests passing and nothing in this repository on the
    /// stack. Load- and order-dependent, so a stress run is a poor way to
    /// prove it fixed; this reproduces the shape directly.
    ///
    /// Shown to fail: comment out the `anchor_subscriptions()` call in
    /// `storage_dir` and this panics at `fs.rs:71`.
    #[test]
    fn a_write_after_the_last_mount_is_dropped_does_not_panic() {
        use dioxus_sdk_storage::StorageBacking;

        // Mount and drop, so nothing this test holds is keeping a receiver
        // alive — which is the state every later test runs in.
        drop(render(|| rsx! { crate::views::settings::SettingsView {} }));

        // The write the app makes whenever a permission ask is journalled.
        <crate::ask_journal::Backing as StorageBacking>::set(
            "lost_asks".to_owned(),
            &Vec::<crate::ask_journal::AskRecord>::new(),
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
