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

// ---- one directory, nine hundred mounts, one namespace each ------------

thread_local! {
    /// The namespace a mount on THIS thread puts in front of every persisted
    /// key, or `None` when nobody has asked for one.
    ///
    /// Thread-local rather than global because mounts run concurrently:
    /// `cargo test` gives each test its own thread by default, and a global
    /// would make two mounts fight over one slot. It is claimed per mount
    /// rather than per thread for the case that has no threads at all —
    /// `--test-threads=1` runs every test on the main one, and a namespace
    /// keyed on the thread would put the whole suite back in one file the
    /// moment anyone reached for that flag to debug an ordering failure.
    static SCOPE: std::cell::RefCell<Option<String>> = const {
        std::cell::RefCell::new(None)
    };
}

/// A NAMESPACE FOR ONE MOUNT'S PERSISTED KEYS.
///
/// #220's second half is the reason this exists. `settings` could not reach the
/// disk while every mounted test shared one file: one test's saved working
/// directory became the next test's starting state, four tests went red, and
/// which four depended on the order the suite ran in. That is the worst
/// property a merge gate can have, and it is a harness problem rather than a
/// product one.
///
/// WHERE THE NAMESPACE IS APPLIED IS THE WHOLE DESIGN.
/// `crate::state::use_app_ctx_provider` reads it ONCE, in a `use_hook`, and
/// builds its four storage keys out of it — so the namespace is baked into the
/// key the moment the mount exists and travels with it for ever. The obvious
/// alternative, a wrapper backing that prefixed every key it was handed, reads
/// the namespace at ACCESS time instead, and `LocalStorage::set` is called from
/// a task Dioxus polls long after the render that spawned it. That version
/// needs the guard held for the whole life of the dom, which means every
/// bespoke test harness in this repository has to know about it — and there are
/// ten of them, in eight files. Measured on the way through: with the guard on
/// the two harnesses in this file only, `views::code::pressing` still leaked,
/// because `a_saved_server_is_dialled_the_first_time_the_tab_is_opened` saves a
/// `code_server_url` and `src/views/code.rs` mounts through a harness of its
/// own.
///
/// WHAT THIS IS NOT: a way to stop testing persistence. The namespace is a
/// key-level choice inside a directory that is ALREADY the harness's —
/// [`storage_dir`] is a temp path where the app uses
/// `~/Library/Application Support/goose-mobile` — and the code under test is
/// the same line either way. A test that wants to prove a value SURVIVES pins a
/// namespace with [`StorageScope::pinned`] and mounts twice inside it, which is
/// a stronger check than the shared file could ever carry: two mounts on a name
/// nothing else in the binary writes.
///
/// HOW MUCH IT IS HOLDING UP, measured on this tree: force the prefix to `""`,
/// so every mount shares one set of files the way they all did before, and
/// **31 of 955** tests in this binary fail. The four the issue named are in
/// there and so are twenty-seven more; the shared file was never only about
/// `settings`.
pub(crate) struct StorageScope(Option<String>);

impl StorageScope {
    /// Take a namespace by name, so two mounts can deliberately share one.
    ///
    /// `""` is a legal name and means the bare key — the namespace a shipping
    /// build is permanently in.
    /// `a_write_after_the_last_mount_is_dropped_does_not_panic` pins it,
    /// because that test is about a subscription a mount created and a write
    /// made after the mount is gone, and those two have to be talking about the
    /// same key for the check to be a check.
    pub(crate) fn pinned(prefix: &str) -> Self {
        // Before the swap, not after: `storage_dir` anchors a subscription on
        // its first call, and an anchor taken inside a namespace would hold a
        // key nothing outside that namespace ever writes.
        let _ = storage_dir();
        Self(SCOPE.with(|slot| slot.replace(Some(prefix.to_owned()))))
    }

    /// A namespace nothing else in the binary uses, pinned.
    ///
    /// For a harness that has to write a key BEFORE the mount reads it —
    /// `crate::state`'s journal harness seeds `lost_asks` on disk and then
    /// launches over it, and the two have to name the same file. It holds the
    /// guard and asks [`storage_key`] for the name.
    pub(crate) fn fresh() -> Self {
        Self::pinned(&next_prefix())
    }
}

impl Drop for StorageScope {
    fn drop(&mut self) {
        SCOPE.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// Run `f` with every mount inside it under a namespace of your choosing.
///
/// The half of [`StorageScope`] that makes persistence testable rather than
/// only isolatable: two mounts inside one call share a namespace, so the second
/// reads what the first saved.
pub(crate) fn in_storage_scope<T>(prefix: &str, f: impl FnOnce() -> T) -> T {
    let _scope = StorageScope::pinned(prefix);
    f()
}

/// The key a write OUTSIDE a mount should use, under whatever namespace is
/// pinned right now.
///
/// Nothing pinned yields the bare key, which is what the two tests that write
/// through the backing directly want, and what [`anchor_subscriptions`] holds.
pub(crate) fn storage_key(key: &str) -> String {
    SCOPE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map_or_else(|| key.to_owned(), |prefix| format!("{prefix}{key}"))
    })
}

/// THE NAMESPACE A MOUNT IS BORN INTO: the pinned one if there is one, and a
/// fresh one nothing else uses if there is not.
///
/// Called once per mount, from `use_app_ctx_provider`'s `use_hook`, which is
/// the single place in the app that every harness goes through. The
/// pinned-wins rule is what lets [`in_storage_scope`] wrap a mount rather than
/// be overridden by it.
pub(crate) fn mount_prefix() -> String {
    let _ = storage_dir();
    SCOPE
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(next_prefix)
}

/// A namespace no other mount in this process has had.
fn next_prefix() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "mount{}-",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
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
/// `lost_asks` is the whole list, and the rule that decides it is
/// `use_synced_storage` — the only API that subscribes — rather than which
/// store a key is on. `inspector_open` goes through it too and would belong
/// here if anything wrote it after its dom was dropped. `code_cache` and
/// `settings` are fs-backed since #220 but reach the store through plain
/// `use_storage`, so `LocalStorage::set` finds no subscription for either key
/// and sends on no channel (`client_storage/fs.rs:64-73`).
///
/// IT ANCHORS THE BARE KEY, and [`StorageScope`] is why that is still the right
/// one. This runs inside `storage_dir`'s `OnceLock`, before any namespace is
/// pinned, so the name it holds is `lost_asks` and not a mount's namespaced
/// copy. That is the key the two tests which write through the backing DIRECTLY
/// use — the case this exists for — while a mount's own writes and its own
/// subscription now share a namespace nothing outside that mount touches, so
/// the hazard cannot arise there at all.
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
    use super::{render, render_seeded, render_settled, with_ctx};

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
    /// PINNED TO THE BARE NAMESPACE, and that is what keeps it a check.
    /// [`mount_prefix`] hands every mount a namespace of its own, so a mount's
    /// subscription and a later bare `Backing::set` would otherwise be talking
    /// about two different keys and this would pass with the anchor deleted.
    /// `""` is the namespace a shipping build is permanently in, so pinning it
    /// puts the mount and the write back on one key — which is the shape the CI
    /// failure had.
    ///
    /// Shown to fail: comment out the `anchor_subscriptions()` call in
    /// `storage_dir` and this panics at `fs.rs:71`. Shown to STOP failing —
    /// which is why the pin is here — by taking `in_storage_scope("")` away as
    /// well: the mount then subscribes `mountN-lost_asks`, the write below
    /// names `lost_asks`, and there is no sender for it to fail on.
    #[test]
    fn a_write_after_the_last_mount_is_dropped_does_not_panic() {
        use dioxus_sdk_storage::StorageBacking;

        super::in_storage_scope("", || {
            // Mount and drop, so nothing this test holds is keeping a receiver
            // alive — which is the state every later test runs in.
            drop(render(|| rsx! { crate::views::settings::SettingsView {} }));

            // The write the app makes whenever a permission ask is journalled.
            <crate::ask_journal::Backing as StorageBacking>::set(
                "lost_asks".to_owned(),
                &Vec::<crate::ask_journal::AskRecord>::new(),
            );
        });
    }

    /// EACH MOUNT GETS ITS OWN NAMESPACE, AND A PINNED ONE IS SHARED — the two
    /// halves of [`StorageScope`], and the reason `settings` could reach the
    /// disk at all (#220).
    ///
    /// The first half is isolation: without it an fs-backed `settings` makes
    /// one test's saved working directory the next test's starting state, and
    /// the four tests that went red the first time this was tried are every one
    /// of them about an EMPTY working directory. The second half is the answer
    /// to the obvious objection — that isolating every mount would leave
    /// persistence exercised by nothing. Two mounts inside one pinned namespace
    /// are a stronger check than the shared file ever was, because the name is
    /// one nothing else in the binary writes.
    ///
    /// Asserted through the app's own signal rather than by reading the file,
    /// so what is checked is that a LAUNCH sees the save: `settings` is read
    /// straight into `AppCtx` by `use_app_ctx_provider`, and that is the only
    /// path that matters.
    ///
    /// The saving half is [`render_settled`] and not [`with_ctx`], and that is
    /// not a preference. `save_to_storage_on_change` is a SPAWNED task, so a
    /// harness that renders once and reads the value straight back has set a
    /// signal nothing has written yet; the settle loop is what polls it.
    /// Measured — with `with_ctx` on both halves this reports `""`, which reads
    /// exactly like the bug it is testing for.
    ///
    /// Shown to fail: put `settings` back on `use_persistent` and the second
    /// assertion reports `""` against the saved path.
    #[test]
    fn a_saved_setting_survives_the_mount_that_saved_it() {
        use crate::state::Settings;

        const SAVED: &str = "/srv/kept-across-a-launch";

        fn nothing() -> Element {
            rsx! {}
        }

        // Isolated by default: a fresh mount is on the launch state whatever
        // any other test in the binary has saved.
        assert_eq!(
            with_ctx(|_| {}, |ctx| ctx.settings.peek().working_dir.clone()),
            String::new(),
            "an unpinned mount inherited a working directory from somewhere, so \
             mounts are not isolated and every test's result depends on the \
             order the suite happened to run in"
        );

        super::in_storage_scope("survives-", || {
            let _ = render_settled(
                |ctx| {
                    let mut settings = ctx.settings;
                    settings.set(Settings {
                        working_dir: SAVED.to_owned(),
                        ..Settings::default()
                    });
                },
                nothing,
            );
            assert_eq!(
                with_ctx(|_| {}, |ctx| ctx.settings.peek().working_dir.clone()),
                SAVED,
                "a second mount in the same namespace did not see what the first \
                 one saved, so `settings` is not reaching the disk — which is \
                 the whole of #220"
            );
        });
    }

    /// THE SAVED FILE CANNOT HOLD A CREDENTIAL, whatever is typed into the
    /// form.
    ///
    /// This is the condition #220's second half had to meet before `settings`
    /// was allowed near the disk at all. `secret_key` and `code_password` are
    /// `#[serde(skip_serializing)]`, and the check is a full round trip through
    /// the store rather than a reading of the attribute: what the file holds is
    /// exactly what the serializer emitted, so a field the reader cannot get
    /// back is a field the writer never wrote.
    ///
    /// NOT A BYTE SCAN OF THE FILE, deliberately, and this is the trap worth
    /// naming. `dioxus-sdk-storage` compresses with zlib and hex-encodes
    /// (`serde_to_string`), so a search for `"s3cr3t-goose"` in those bytes
    /// passes whether or not the field is in there — a check that cannot fail,
    /// which is the shape `crate::selfscan` exists because of. The decode is the
    /// only reading that can tell the two apart.
    ///
    /// Under a probe key of its own, and no mount: the question is about the
    /// TYPE's serialization, and a mount would drag in a namespace and four
    /// other keys that have nothing to do with it.
    #[test]
    fn the_saved_settings_file_holds_no_credential() {
        use crate::state::Settings;
        use dioxus_sdk_storage::StorageBacking as _;

        let dir = super::storage_dir();
        crate::ask_journal::Backing::set(
            "settings_credential_probe".to_owned(),
            &Settings {
                server_url: "http://brain:3285".to_owned(),
                secret_key: "s3cr3t-goose".to_owned(),
                fingerprint: "ab:cd".to_owned(),
                working_dir: "/srv/work".to_owned(),
                code_server_url: "http://brain:4399".to_owned(),
                code_password: "s3cr3t-code".to_owned(),
            },
        );

        let path = dir.join("settings_credential_probe");
        assert!(
            path.is_file(),
            "no settings file was written at all, so this check is about \
             nothing: {}",
            path.display()
        );

        let read: Settings =
            crate::ask_journal::Backing::get(&"settings_credential_probe".to_owned())
                .unwrap_or_else(Settings::default);
        assert_eq!(
            (read.server_url.as_str(), read.working_dir.as_str()),
            ("http://brain:3285", "/srv/work"),
            "the four non-secret fields did not survive the round trip, so \
             skipping the other two cost the ones that were meant to persist"
        );
        // The default and not `""`: `serde(default)` fills a missing field from
        // `Settings::default()`, which is where `dev_seed!` lives — so a
        // development build gets its two seeds back on every load and does not
        // pay for this with a retype per launch. In a release build every seed
        // is the empty string and the two readings coincide.
        assert_eq!(
            (read.secret_key, read.code_password),
            (
                Settings::default().secret_key,
                Settings::default().code_password
            ),
            "a secret came back off the disk, so it went to the disk"
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
