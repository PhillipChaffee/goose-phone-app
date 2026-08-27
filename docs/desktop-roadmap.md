# Desktop roadmap

## What this is, and what it is not

This is an inventory of what the desktop shell could do and does not, written
against the pinned dependency graph rather than against an impression of what
Electron provides. It exists because an earlier survey concluded that eleven
capabilities in goose's own Electron desktop app were "Electron-only", and that
conclusion was reached by reasoning from Electron's API surface instead of from
this repository's `Cargo.lock` — so it was wrong about most of them, in a
direction worth being precise about: several of the machinery it called
unavailable is already constructed and running inside the binary that ships
today. It is **not** a plan, not a commitment, and not an estimate of calendar
time. Every cost claim below says what it is based on, and where a claim could
not be checked against vendored source it says so in the text rather than being
left out. Versions are read from
`.claude/worktrees/testing/Cargo.lock`; API claims are read from the vendored
crate under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`; protocol
claims are read from `crates/goose-acp-client/tests/fixtures/`, which is pinned
to goose commit `3810898a`.

A later pass **executed** the claims at the end of this document that a
compiler, a linker or a running process could settle, and three of them came
back wrong: `dx`'s platform autodetection reads `[features]` even though its
build does not, the obvious `notify-rust` dependency line is a silent no-op on
macOS 26, and goose has since added the second agent→client notification whose
arrival this document named as its own falsifier. Those corrections are folded
in below where they belong, and the last section records what was run, what came
out, and — separately and without softening — what is still unrun.

## The impossible list

The earlier survey named eleven capabilities as not-applicable because Electron
provides them and we do not have Electron: window vibrancy, open a session in a
new window, the `.goosehints` editor, the spellcheck toggle, a quick-launcher
overlay with a global hotkey, multi-window / always-on-top / mouse back button,
tray or menu-bar icon / dock icon / dock menu / Prevent Sleep, auto-update,
local `goose serve` lifecycle, telemetry, and the announcement modal.

**Eight of the eleven are reachable and four of those are already paid for.**
`dioxus-desktop-0.7.10/src/lib.rs:43-49` re-exports `muda` and declares
`pub mod trayicon` under gates this repository already writes by hand
(`src/shell/mod.rs:28-35`), `hooks.rs` exposes `use_tray_menu_event_handler`
(:60), `use_tray_icon_event_handler` (:78), `use_muda_event_handler` (:44) and
`use_global_shortcut` (:114), and the shipped binary carries a
`GlobalHotKeyManager` construction on the un-`cfg`'d path through `App::new`
whether or not a hotkey is ever registered — measured in the linked image
(`otool -L` lists `Carbon.framework`, `nm -u` lists `_InstallEventHandler`, and
`ShortcutRegistry::new` disassembles to `bl … GlobalHotKeyManager3new`), though
the binary was never run, so "on every launch" is read from
`dioxus-desktop-0.7.10/src/app.rs:80` rather than watched. Spellcheck is an HTML
attribute. Telemetry is an HTTP POST with a
`reqwest` already in the lockfile. The announcement modal is a fetch and a div.
The `.goosehints` editor is not a filesystem affordance at all for a remote
client — the file lives on the server and `_goose/unstable/tools/call` reaches
it, because `on_call_tool` (`/Users/phillipchaffee/git/goose/crates/goose/src/acp/server/tools.rs:60-90`)
dispatches straight into `extension_manager` with **no permission gate**, its
only filter being `is_tool_visible_to_app`, which returns `true` for any tool
that declares no `_meta.ui.visibility` (`reply_parts.rs:787-801`).

Three things are genuinely impossible, and none of the three is impossible
because of Electron. Two of those three have since been struck as NON-GOALS
rather than gaps — see the decision recorded under each — which leaves exactly
ONE capability that is both wanted and out of reach. It has a design document of
its own: `docs/push-notifications.md`.

**1. A notification on iOS that an agent finished while the app is
backgrounded.** The event that would fire it arrives on a WebSocket, and iOS
suspends the socket when the app leaves the foreground, so there is nothing
running to raise a local notification. The route that exists for this on iOS is
a remote push, and a remote push needs something on the server to send it —
goose has no push endpoint. At the pinned commit the protocol has exactly one
agent→client notification (`_goose/unstable/session/update`) and exactly one
agent→client request (`_goose/unstable/session/recipe/request-params`), both
verified as the only entries in the `notifications` and `agentRequests` arrays
of `crates/goose-acp-client/tests/fixtures/acp-meta.json`. Re-vendoring against
goose `origin/main` as of 2026-08-21 adds a second notification
(`_goose/unstable/providers/authentication/device-code`) and no push endpoint,
so the conclusion is unchanged and the count is not. This is not a client gap.
It is a missing capability on both sides at once, and neither side is ours alone
to add. The macOS half of the same capability is buildable — a foreground
process can raise a notification when a turn ends, and that was compiled and
run — with one packaging caveat recorded in the notifications row.

**2. Local `goose serve` lifecycle on iOS and Android.** NOT A GAP — a NON-GOAL,
decided by the product owner on 2026-08-26: "I actually don't want either the
desktop client or the phone app to be able to work on their own. They should
both assume that there's a server running somewhere else."

The mechanism was never exotic — goose's own Electron version is
`child_process.spawn` plus a readiness poll, and `std::process::Command` is the
same thing with no new dependency. iOS forbids executing a binary that is not
the app, so it is impossible on two of the four targets; but the reason it is
not being built on the other two is the decision above, not the platform. Every
part of this app already says so out loud: `scripts/check-server.sh` opens with
"Run this ON the VPS that runs goose", and `docs/iphone-setup.md` exists because
goose is somewhere else. Building a local backend would be building a different
product. This row should not appear on a future gap list.

**3. Auto-update on iOS and Android.** NOT A GAP — a NON-GOAL, from the same
decision. Store distribution owns the update channel on the two mobile targets;
on macOS a notarized `.app` that rewrites its own bundle invalidates its own
signature unless something like Sparkle drives the replacement.

The code was never the cost — `self_update` 0.42.0 would integrate in an
afternoon, with `reqwest`, `hyper`, `flate2` and `semver` already in the
lockfile. The cost is notarization round-trips and a distribution channel that
does not exist, against an app installed on one person's devices where
TestFlight and a local build already do this. Filing it as "large" read as "lots
of code", which was the opposite of true; filing it at all was the real error.

A fourth item deserves naming even though it is not on the eleven. **The native
file picker cannot browse the server.** `rfd`'s `pick_folder` picks a folder on
the machine running the client, and the working directory the agent uses is on
the machine running goose. That is a physical fact about a remote client and
there is no `fs/list`, `fs/read`, `fs/browse` or `fs/stat` anywhere in the
114-method surface — the only path-shaped method in the whole contract is
`_goose/unstable/session/working-dir/update`. The *capability* survives by a
different route (the `developer` extension's `tree` tool through
`tools/call`), but the obvious mechanism is genuinely unavailable, and anyone
reaching for `rfd` here will reach for the wrong thing.

### A fifth category, which is ours and is not impossible

Four capabilities are blocked by a decision this repository made rather than by
the world: native right-click context menus, the dock menu, replacing the dock
icon image, and the clean begin/end form of Prevent Sleep. All four need an
`unsafe` block written in *this* crate, and `Cargo.toml:26` sets
`unsafe_code = "forbid"` at the workspace level, which `#[allow]` and
`#[expect]` cannot lift inside a package that opts in — and this package opts in
at `Cargo.toml:82-83`. Verified as `unsafe fn` in the pinned sources:
`muda-0.17.2/src/lib.rs:445` (`unsafe fn show_context_menu_for_nsview`, whose
only safe sibling `show_context_menu_for_gtk_window` at :414 is Linux-only),
`objc2-app-kit-0.3.2/src/generated/NSApplication.rs:737`
(`pub unsafe fn setApplicationIconImage`), and
`objc2-foundation-0.3.2/src/generated/NSProcessInfo.rs:267`
(`pub unsafe fn endActivity`). The dock menu additionally needs
`objc2::define_class!` or `ClassBuilder::add_method`, both of which expand to
`unsafe` in the caller's crate.

**A prior claim that these could be reached with `#[expect(...)]` per block is
wrong**, and it is wrong on the exact distinction between `deny` and `forbid` —
compiled, not argued: `#[expect(unsafe_code, reason = "...")]` over an `unsafe`
block in this crate gives `error[E0453]: expect(unsafe_code) incompatible with
previous forbid`, with `note: forbid lint level was set on command line (-F
unsafe_code)`. A prior claim in the other direction — that *every* AppKit route
needs unsafe — is also wrong, and that is the more consequential error, because
it is what made vibrancy look expensive. See the next section.

### What the "AppKit needs unsafe" claim got wrong

`objc2-app-kit` 0.3.2's `extern_methods!` blocks generate **safe** `pub fn`
wherever main-thread-ness is proven by a `MainThreadMarker` and the argument
types are already checked. Verified in the vendored source, all declared
`pub fn` with no `unsafe` qualifier:

| call | file:line |
|---|---|
| `MainThreadMarker::new() -> Option<Self>` | `objc2-0.6.4/src/main_thread_marker.rs:230` |
| `NSApplication::sharedApplication(mtm)` | `NSApplication.rs:488` |
| `NSApplication::keyWindow` / `mainWindow` / `windows` | `NSApplication.rs:526` / `:521` / `:690` |
| `NSWindow::contentView` | `NSWindow.rs:761` |
| `NSWindow::setOpaque` | `NSWindow.rs:1390` |
| `NSWindow::setLevel` | `NSWindow.rs:1327` |
| `NSWindow::setCollectionBehavior` | `NSWindow.rs:1446` |
| `NSView::addSubview` / `addSubview_positioned_relativeTo` | `NSView.rs:293` / `:298` |
| `NSVisualEffectView::new(mtm)` | `NSVisualEffectView.rs:305` |
| `NSVisualEffectView::setMaterial` / `setBlendingMode` / `setState` | `:208` / `:224` / `:234` |
| `NSProcessInfo::beginActivityWithOptions_reason` | `NSProcessInfo.rs:256` |
| `NSProcessInfo::performActivityWithOptions_reason_usingBlock` | `NSProcessInfo.rs:272` |

`unsafe_code = "forbid"` forbids *writing* `unsafe` in this crate. It says
nothing about *calling* a safe `fn` whose body was compiled in a dependency.
**This was compiled, not argued**: the whole sequence above lands in
`goose-mobile` with zero `unsafe` tokens and passes `cargo clippy --workspace
--all-targets -- -D warnings`, and a control `unsafe` block in the same file is
rejected with `note: requested on the command line with -F unsafe-code`.

The consequence is a rule, not an aside, because it is the difference between
buildable and unbuildable: **the `MainThreadMarker` → `sharedApplication` →
`keyWindow` route is the only one that compiles here, and no `#[expect]`
reopens the other.** The route through `tao`'s `WindowExtMacOS::ns_window() ->
*mut c_void` (`tao-0.34.8/src/platform/macos.rs:24`) — where the earlier
analysis assumed the pointer cast had to live — type-checks fine and then dies
on that same `-F unsafe-code` error, because `Retained::retain` on a raw pointer
is `unsafe` by construction; annotating it `#[expect(unsafe_code, reason =
"...")]` gives `error[E0453]: expect(unsafe_code) incompatible with previous
forbid`. The app crate contains zero `unsafe` today and still contained zero
after vibrancy, window level and collection behaviour were added and linked.

Vibrancy in particular is therefore cheaper than every prior estimate.
`objc2-app-kit` is already in `Cargo.lock:3041` and already compiled for the
macOS target. `cargo tree --target aarch64-apple-darwin -e features -i
objc2-app-kit` reports **44 enabled features** including `NSApplication`,
`NSWindow`, `NSView`, `NSPanel`, `NSScreen`, `NSStatusItem` — and
`NSVisualEffectView` is not among them. The feature declares four sub-features
of a crate that is already compiled (`objc2-app-kit-0.3.2/Cargo.toml:1934-1939`),
and the measured delta of turning it on is smaller still, because all four are
already enabled: **44 → 45 features on `objc2-app-kit`, 51 → 51 on
`objc2-foundation`, 599 → 599 packages**, and a two-line `Cargo.lock` diff. Zero
new crates, zero network fetch, zero unsafe. The one cost that is real and that
earlier passes did not name: the feature union changes `objc2-app-kit`'s
fingerprint, so enabling it recompiles `objc2-web-kit`, `muda`, `tao`, `wry`,
`rfd`, `global-hotkey`, `tray-icon`, `webbrowser`, `dioxus-desktop` and
`dioxus`. That is a scheduling fact, not a code cost. `cocoa` 0.26.1 —
recommended by an earlier pass as the cheapest route because it is already in
`Cargo.lock:388` — is the *worst* route, because every call in it is a raw
`msg_send!`. Being in the lockfile is not being callable.

Prevent Sleep gets the same correction one step further, **on source only —
unlike vibrancy, nothing here was compiled.** `begin` is safe and `endActivity`
is not (`NSProcessInfo.rs:256` vs `:267`), so the obvious begin/end pair is
unwritable here — but `performActivityWithOptions_reason_usingBlock`
(`NSProcessInfo.rs:272`) is declared `pub fn`. It holds the assertion for the
duration of a synchronously-executed block, so the shape is a parked thread on a
channel rather than a `use_effect`. Three things separate it from the vibrancy
result and none was checked: it lives in `objc2-foundation`, which is not a
direct dependency of `goose-mobile` and would have to become one with `block2`
and `NSString` named explicitly rather than inherited transitively; its argument
is a `&block2::DynBlock<dyn Fn()>` built by `RcBlock::new`; and no lockfile or
feature delta was measured for it the way one was for `objc2-app-kit`. Treat it
as likely-safe and unpriced until one `cargo check` says otherwise.

**Honest count of impossible things: three, plus one mechanism that is
unavailable while its capability is not.** None of them is impossible because
of Electron. Two are distribution policy on platforms we chose, one is a missing
capability on the server, and the fourth is arithmetic about which machine a
filesystem is on.

## Code organisation — the decision, the tree, and the first commit

The decision is: **keep one tree and deepen it, add a fifth layer for
platform capabilities, and split a package only where the lint policy needs a
seam.** A separate `goose-ui` / `goose-phone` / `goose-desk` split was
considered and rejected. The reasoning, and the fact that settles it:

### The `feature = "desktop"` half of every double gate buys nothing

`dioxus-0.7.10/src/lib.rs:107-113` shows `desktop` and `mobile` are the *same*
re-export of the *same* crate:

```rust
#[cfg(feature = "desktop")]
pub use dioxus_desktop as desktop;

#[cfg(feature = "mobile")]
pub use dioxus_desktop as mobile;
```

And `dioxus-desktop-0.7.10/src/lib.rs:43-49` already gates the platform-only
surface on `target_os`, using this repository's exact predicate:

```rust
// Reexport muda only if we are on desktop platforms that support menus
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub use muda;

// Tray icon
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub mod trayicon;
```

Upstream made platform the target, not the feature. So the eight
`#[cfg(all(feature = "desktop", not(any(target_os = "ios", target_os = "android"))))]`
sites in this repo (`src/main.rs:74`, `:127`, `:144`, `:155`;
`src/shell/mod.rs:41`; `src/shell/desktop.rs:51`, `:517`, `:573`) are carrying a
predicate that is implied by the other half. Replacing the two feature bodies
with two target-conditional `[dependencies]` tables collapses all eight to a
single `not(any(ios, android))` and makes the guarantee *stronger*, because it
stops depending on remembering `--no-default-features --features mobile` at
`.github/workflows/ci.yml:127-128`. That is the first commit: a manifest edit,
eight `cfg`s halved, and roughly 25 lines of now-false prose deleted
(`src/main.rs:63-72`, `src/shell/desktop.rs:45-51`). No files move.

**This was built, and it corrected the plan.** All four gates pass on the edited
manifest — `cargo check`, `cargo test --workspace`, `cargo clippy --workspace
--all-targets -- -D warnings`, and `cargo check --target aarch64-apple-ios` —
and `dx build --ios` and `dx build --desktop` both produce a bundle with **no**
`--features` flag, so `dx` does read target-conditional tables. What it does not
tolerate is `[features]` being *deleted*: `dx check` then fails with
`ERROR dx check: Could not automatically detect target triple`, because
`dioxus-cli-0.7.10/src/build/renderer.rs:7-30` picks the renderer by taking the
first `dioxus` dependency entry and otherwise reading `[features]` keys. So the
`[features]` table shrinks to a marker rather than disappearing:

```toml
# Read only by dx's platform autodetection (dioxus-cli renderer.rs:7-30).
# Enables nothing; no cfg in this crate reads it. The real selection is the
# two target-conditional [dependencies] tables below.
[features]
default = ["desktop"]
desktop = []
```

With that line present, `dx check`, bare `dx build`, `dx build --desktop` and
`dx build --ios` all succeed and the eight `cfg`s stay collapsed. The earlier
fallback recorded here — enable both features unconditionally — is not needed
and is strictly worse, since it would leave `dioxus::desktop` nameable on iOS.

**LANDED.** Everything above this paragraph is the plan as it was written; read
it in the past tense. The file:line citations are pre-change and now stale —
`ci.yml:127-128` is one line, `ci.yml:131`, and carries no flags. Two things
the plan did not anticipate, both measured on the shipped manifest:

- **The marker feature is enabled on phones too**, so `feature = "desktop"` is
  TRUE in an iOS build and the name says the opposite of what it means. Before
  the change the iOS job passed `--no-default-features`, so it was false there;
  now nothing turns it off, and `dx` re-adds it as well
  (`dx build --platform ios --verbose` → `features: ["desktop",
  "dioxus/mobile"]`). Renaming out of the trap is not available —
  `dioxus-cli-0.7.10/src/platform.rs:188` matches only
  web/desktop/mobile/native/liveview/server — so the ceiling is a test:
  `shell::tests::no_cfg_reads_the_dx_marker_feature` scans `src/` and fails the
  build if any `cfg` reads it.
- **The featureless `dioxus` entry in `[dependencies]` is load-bearing too**,
  which nothing here predicted. It is compile-redundant, since the two target
  tables cover every triple, so it reads as leftover — but `renderer.rs:17`
  finds the *first* `dioxus` entry with no target filter, and deleting the line
  makes that the `mobile` table: `dx check` then fails with "Could not
  autodetect mobile platform. Use --ios or --android instead." Every cargo gate
  still passes in that state, so `the_manifest_lines_that_only_dx_reads_are_
  still_there` holds both lines instead.

### Why not three crates

The separate-tree proposal's two headline benefits — killing the feature pair
and getting a scope where `unsafe_code` can differ — are both collectable
without moving a file. Against that, there is a concrete breakage.
`src/shell/desktop.rs:724` is:

```rust
let dispatch = include_str!("../viewport.rs");
```

That is a cross-layer source scan: the desktop shell's test reads
`viewport.rs`'s **source** to prove every `DESTINATIONS` id has a
`refresh_named` arm. Its twin at `src/viewport.rs:564` walks
`CARGO_MANIFEST_DIR/src/views` on disk and greps for
`include_str!("viewport.rs")` at `:571`. Split the tree and the first becomes
`include_str!("../../goose-ui/src/viewport.rs")` — a cross-crate source include
cargo does not track for rebuilds, which breaks under vendoring, and which the
sibling crate is free to invalidate. Both guard a `match` whose fallthrough is
`_ => {}`, and the codebase's own note at `viewport.rs:452-458` says what an
unclaimed name is: "a pull that spins and fetches nothing". These are exactly
the class of check a long capability list needs more of, and a crate boundary is
where they stop working. `src/css.rs:20-27` and `:59-97` would take the same
damage — seven `include_str!`s reaching up out of a shared crate into
`assets/`. Add a `pub(crate)`→`pub` sweep across ~19,800 lines, on a repo whose
`src/main.rs:9-11` is explicitly laid out to keep concurrent branches out of
each other's hunks, and it is three commits and a wrecked `git blame` in
exchange for something one manifest already provides.

### The fifth layer

`src/views/` is **7,722 lines across 13 modules with zero `cfg(target_os)` and
zero `cfg(feature)`** — verified by grep; the only `cfg` in any of them is
`cfg(test)`. That invariant is the asset the whole arrangement rests on, and
sixty capabilities is exactly the pressure that would break it. The answer is a
layer they can land in that is not `views/` and not `shell/`:

```
src/
  main.rs        launch only. integrate_titlebar moves out.
  action.rs      NEW. enum Action { Go, Refresh, ToggleNav, NewChat, ... }
                 A menu item, a tray item and a hotkey are routes four, five
                 and six into viewport::refresh_named (viewport.rs:466), which
                 already exists because three routes converge on one match.
  platform/      NEW — the fifth layer, gated by Surface, not Shell.
    mod.rs         enum Surface { MacOs, Windows, Linux, Ios, Android } with
                   Surface::CURRENT as a cfg!-derived VALUE and the per-target
                   `const _: () = assert!` guards from shell/mod.rs:80-83, for
                   the reason stated there: the list is a list, so the failure
                   mode is a target quietly missing from it.
    menu.rs        muda via Config::with_menu; Go section generated from
                   nav::DESTINATIONS, as shell/mobile.rs already does for the
                   drawer.
    hotkey.rs      use_global_shortcut (hooks.rs:114).
    tray.rs        dioxus::desktop::trayicon (trayicon.rs:28/50/64).
    windowing.rs   new_window (desktop_context.rs:141), toggle_maximized (:169).
    launcher.rs    Frameless always-on-top overlay. Gated on the AppCtx
                   question below.
    macos.rs       integrate_titlebar (moved from main.rs:131), dock control,
                   vibrancy. All safe fns; no unsafe.
    ios.rs         Share sheet, haptics. The mobile arms of the same table —
    android.rs     which is why this is `platform/` and not `desktop/`.
  shell/         UNCHANGED in shape. Still exactly two arms, forever.
  views/         UNCHANGED. The zero-cfg invariant gains a test.
```

Each module in `platform/` carries **one** `#[cfg(not(any(target_os = "ios",
target_os = "android")))]` — the same shape `shell/mod.rs:28-35` already uses.
Sixty capabilities is sixty single gates, linear, not combinatorial. And the
gate is backstopped by the compiler: naming `dioxus::desktop::muda` on iOS is a
hard error from `dioxus-desktop/src/lib.rs:44` even if the module gate is
forgotten.

### The one package split, and its ceiling

`crates/goose-native/`, a leaf of a few hundred lines that omits
`[lints] workspace = true` and restates `unsafe_code = "deny"` locally, holding
only the four things above that have no safe route: context menus, the dock
menu, the dock icon image, and (if the block form turns out not to fit) the
Prevent Sleep teardown. It is created **when the first capability that genuinely
needs it arrives**, not before.

This crate has one failure mode and it is worth naming in advance: metastasis.
If it starts absorbing logic rather than FFI, or if a second capability finds it
easier to add a function there than to find a safe route, the seam drifts from
"the FFI boundary" to "the desktop boundary" one commit at a time and the crate
split nobody chose happens anyway. The ceiling is: no dependency on
`goose-mobile`, no Dioxus, no `AppCtx`, and every `pub fn` a thin safe wrapper
over one OS call. That ceiling needs a test, not a convention — the first three
are checkable from `Cargo.toml` in a build script or a CI grep.

## The capability inventory

Cost tiers are relative effort on this repository's measured baseline (below),
not calendar time. "Already paid" means the machinery is constructed in the
shipping binary and only the call site is missing.

### Window and OS integration (macOS-first; Windows/Linux where noted)

| Capability | Status | What it needs | Cost |
|---|---|---|---|
| Tray / menu-bar icon with menu and click handling | reachable, already paid | `dioxus::desktop::trayicon::{init_tray_icon, default_tray_icon}` (`trayicon.rs:28`, `:50`), events via `use_tray_menu_event_handler` (`hooks.rs:60`). `App::new` already calls `set_tray_icon_receiver()`. Safe throughout; `TrayIcon::ns_status_item()` (`tray-icon-0.21.3/src/lib.rs:476`) is a safe `pub fn` if needed | small |
| Global hotkey (quick-launcher chord) | reachable, already paid | `use_global_shortcut` (`hooks.rs:114`); a tray menu plus one `use_global_shortcut` call compiled clean on the first try, and the only two diagnostics in the whole attempt were `redundant clone` and `redundant closure`. The `GlobalHotKeyManager` construction is on the un-`cfg`'d path in `App::new` and its Carbon imports are in the linked binary — not observed at runtime. macOS uses Carbon `RegisterEventHotKey`, so no TCC prompt | trivial code |
| Menu bar (replace, not adopt) | already shipping | `src/main.rs:86-88` never calls `with_menu`, so `MenuBuilderState::Unset` resolves to `default_menu_bar()` (`config.rs:45-52`) and Window+Edit menus with ⌘C/⌘V/⌘W/⌘Q are on screen now. Replacing it is `Config::with_menu` | small |
| Frameless, always-on-top overlay window | reachable | `tao` safe builders: `with_decorations(false)` (`window.rs:493`), `with_always_on_top` (:516), `with_transparent` (:482), `with_visible_on_all_workspaces` (:589). The repo already uses this API at `src/main.rs:81-87` | small |
| Window vibrancy | reachable, no unsafe, **compiled** | One line enabling `objc2-app-kit`'s `NSVisualEffectView` feature (measured: 44 → 45 features, 599 → 599 packages, two lines of `Cargo.lock`), then `NSVisualEffectView::new`/`setMaterial`/`setBlendingMode`/`setState` + `addSubview`, all of which pass `-D warnings` with `unsafe_code = "forbid"` and link. Side cost: the feature union recompiles `tao`, `wry`, `muda`, `rfd`, `tray-icon` and `dioxus-desktop`. **The real cost is still CSS**: `assets/main.css` surfaces are opaque tokens, so this means desktop-only tokens and a deliberate gallery re-capture | medium (CSS, not objc) |
| Window level / collection behaviour (true floating panel) | reachable, no unsafe, **compiled** | `NSWindow::setLevel` (:1327), `setCollectionBehavior` (:1446) via `sharedApplication(mtm).keyWindow()`. Both compiled and linked under `-F unsafe_code`; the `ns_window()` pointer route does not compile and cannot be excepted | trivial |
| Dock: activation policy, visibility, badge, hide/show | reachable | All safe `fn` on tao's `EventLoopWindowTargetExtMacOS` (`platform/macos.rs:397`, `:403`, `:406`, `:387`, `:389`). A tao-event path, so no synchronous-XHR cost | trivial |
| Dock: replace the icon image | **blocked by our lint policy** | `setApplicationIconImage` is `pub unsafe fn` (`NSApplication.rs:737`). Needs `crates/goose-native/` | small, after the seam |
| Dock menu (right-click the dock icon) | **blocked by our lint policy** | `applicationDockMenu:` on a delegate subclass; `define_class!` expands to `unsafe impl` in the caller's crate | medium, after the seam |
| Native context menus (right-click) | **blocked by our lint policy** | `muda`'s `unsafe fn show_context_menu_for_nsview` (`lib.rs:445`); no safe macOS sibling. Also needs `with_disable_context_menu(false)` — the release build currently suppresses WebKit's own (`config.rs:117`) | small, after the seam |
| Prevent Sleep | reachable on paper, **not compiled** | `performActivityWithOptions_reason_usingBlock` (`NSProcessInfo.rs:272`) is `pub fn`; `endActivity` (:267) is `pub unsafe fn`, so only the block form is available. Holds for a block's duration, so: a parked thread on a channel, not a `use_effect`. Unlike vibrancy this was never built — it needs `objc2-foundation` as a *direct* dependency with `block2`+`NSString` named, and an `RcBlock::new` argument, neither of which was priced | small, unverified |
| Multiple windows / session in a new window | mechanism reachable, **gated** | `DesktopService::new_window` (`desktop_context.rs:141`) is routed end-to-end already. The deliverable is gated on the shared-state question below. See Sequencing | trivial + a large prerequisite |
| Double-click titlebar to zoom | reachable | `toggle_maximized()` (`desktop_context.rs:169`) as `ondoubleclick` on the drag strip that already carries `onmousedown -> drag_window()` at `src/shell/desktop.rs:425-432`. One DOM event, inside the rule | trivial |
| Mouse back button | reachable, **still unverified end to end** | Not via tao — `platform_impl/macos/view.rs:962-970` collapses every non-left/right button to `MouseButton::Middle`, and wry's WKWebView covers tao's content view anyway (the finding already recorded at `src/main.rs:129-137`). Via the DOM: `MouseButton::Fourth` from `button: 3` (`dioxus-html/src/input_data.rs:22`, `:42`). WebKit *trunk* maps `buttonNumber` 3 to `MouseButton::Back` and forwards it unfiltered, but trunk is not the WKWebView macOS 26 loads and nothing was run: no click, no synthetic event, no logger. Settle it with a `CGEventCreateMouseEvent(…, kCGEventOtherMouseDown, …, 3)` posted at the running app (costs one Accessibility grant), or with a mouse that reports `buttonNumber` 3 | trivial code, gated on one measurement |
| Native notifications | needs a new dep, macOS only, **and a signing step** | `notify-rust` 4.18.0, now vendored and run. **Not** `notify-rust = "4.18"` — that resolves to the `NSUserNotificationCenter` backend, which `usernoted` denies as a legacy client on macOS 26 while `show()` still returns `Ok`. The line is `notify-rust = { version = "4.18", default-features = false, features = ["preview-macos-un"] }` under `[target.'cfg(target_os = "macos")'.dependencies]` — 12 lockfile entries instead of 38, of which two compile here. Wired into the `spawn_forever` tail of `send_prompt` in `src/state.rs` (the arm that reads `"end_turn" \| "cancelled" => {}`), gated on `Window::is_focused()` (`tao-0.34.8/src/window.rs:877`, safe), plus a one-time **async** `request_auth()` at mount; workspace clippy and the iOS check both pass. The plist work below is already done for this one key (`CFBundleIdentifier = com.goosemobile.app`); what is unsettled is signing — `dx`'s bundle is linker-signed with `Info.plist=not bound`, and whether that bundle can raise a notification was never tested. iOS half is impossible (see The impossible list) | medium |
| Auto-update | non-goal | Struck 2026-08-26: the client assumes a server elsewhere and is installed by hand | — |

### Files and pickers

| Capability | Status | What it needs | Cost |
|---|---|---|---|
| Native file / directory picker (local machine) | reachable | `rfd` 0.17.2 is already compiled in — it is what backs `<input type=file>` (`dioxus-desktop-0.7.10/Cargo.toml:244`). **But not as one line.** rfd has no iOS backend (`backend.rs:31` gates `mod macos` on `target_os = "macos"`; no `ios` module), so a plain `[dependencies]` entry breaks `cargo check --target aarch64-apple-ios`. And rfd's `default` is `["xdg-portal", "wayland"]` while only `xdg-portal` and `pollster` are enabled today (verified by `cargo tree -e features -i rfd`), so a bare version line unions `wayland` in and adds three crates to the lockfile. Correct form: `rfd = { version = "0.17", default-features = false, features = ["xdg-portal"] }` under `[target.'cfg(not(any(target_os = "ios", target_os = "android")))'.dependencies]` | small |
| Working-directory picker for a path on the **server** | reachable, server-dependent | `tools/call` → the `developer` extension's `tree` tool, then `session/working-dir/update`. Today the working dir is a raw text field validated only by `starts_with('/')` (`src/state.rs:1238`). **Risk worth pricing:** this depends on `developer` being enabled with `unprefixed_tools` on a machine the client does not control, and when it is not, the feature degrades silently rather than failing loudly. `tools/list` is the honest runtime probe | small, with a caveat |
| Drag-and-drop a local file onto the transcript | reachable, macOS only | dioxus already installs wry's drag-drop handler and replaces the DOM event's file list with real `PathBuf`s (`webview.rs:152-186`). **Architectural catch:** HTML only fires `drop` if `dragover` is `preventDefault`ed, and `dragover` is a ~60Hz stream — so it must be a JS-owned listener (`document::eval`, same pattern as `src/attach.rs:219`); only the single `drop` may be Rust | small |
| `.goosehints` editor | reachable | `tools/call` with `shell` to read and `write` to write. **This is a one-way security decision, not a small feature**: a UI driving an unpermissioned `shell` tool is arbitrary remote command execution reachable from a phone. Decide it deliberately | trivial code, one-way decision |
| Session export to a file | reachable | `session/export` already returns a markdown string and we already classify the method (`error.rs:188`); the file half is `rfd`'s `save_file`, target-gated as above. On iOS the correct affordance is a share sheet, and the cheap route there is a JS-owned Blob + `<a download>` rather than `UIActivityViewController` | small |

### Protocol surface

The contract is 114 client→agent methods plus one notification and one agent
request, all pinned in `tests/fixtures/acp-request-keys.json` and
`acp-meta.json` (verified: exactly 114 entries in each, `_source` =
`3810898a`, and the fixture is byte-identical to goose's own file at that
commit modulo the added `_source` key). We speak 21, and 114 − 22 method
strings in the tree = the 92 below. **Nothing in the missing 92 is streaming
*at the pinned commit*** — every one is plain request/response, including the
two that look like streams (`local-inference/models/download/progress` and the
dictation equivalent are polled snapshots). Note that
`crates/goose-acp-client/src/error.rs:167-194` lists more method strings than we
send; that is the `Feature::of_method` classification table, and a naive grep of
the tree overstates coverage by six.

**That sentence has an expiry date and it has passed.** Re-vendoring against
goose `origin/main` (`8d844eecb`, 2026-08-21, 123 commits past the pin) gives
113 methods and **two** notifications: `providers/config/authenticate` now
pushes `_goose/unstable/providers/authentication/device-code` out of band, and
it is one of the missing 92, so it is exactly the counter-example this document
predicted would arrive. The other diff hunk is `session/delete` leaving
`acp-meta.json` — it was *not* removed from goose, it migrated to the standard
ACP SDK, and `crates/goose-acp-client/src/client/session.rs:240` still sends it
successfully. That matters for the contract test rather than for the wire: with
a refreshed fixture it reports `goose declares no method 'session/delete' — a
-32601`, which is false, because `acp-meta.json` only covers goose-*custom*
methods. Whoever re-vendors needs either an allowlist for base-ACP methods or a
different failure message. Nothing in this repository notices contract drift on
its own; a periodic `scripts/vendor-acp-contract.py ~/git/goose && git diff` is
a standing chore.

| Capability | Status | What it needs | Cost |
|---|---|---|---|
| Elicitation (answering a mid-turn form) | **reachable, and today's behaviour is wrong** | Not a missing feature — a declined capability. `client.rs:181-185` sends no `elicitation` flag and no `recipeParameterRequests`, and `answer_agent_request` replies `-32601 "method not supported by this client"` to everything but `session/request_permission` (`client.rs:539-547`). A recipe with parameters fails against us today. The plumbing exists: `AcpEvent::Permission` already carries a `request_id` and `respond_permission` already replies over `Cmd::Respond` | small code, one-way |
| `session/steer` (type a follow-up mid-turn) | reachable | One method. `expectedRunId` arrives as `_meta.goose.activeRunId` on the `session/update` notification we already receive — `src/state.rs:646-657` already destructures it and only reads `usage_update`. **`activeRunId` appears nowhere in this repo today** (verified by grep across `src/` and `crates/`), so the mock's `turn.rs` must start stamping it | small |
| Slash-command + @-mention autocomplete | reachable | Two list reads, no capability negotiation. Cheapest real feature in the missing 92 | trivial |
| `diagnostics/get` | reachable | One method, response is deliberately opaque on the wire, so it renders as preformatted text — zero DTO modelling | trivial |
| Session extensions list/remove | reachable | Closes a one-sided pair: we can `add` but a user cannot see or detach what they attached | trivial |
| Session archive / metadata (6 methods) | reachable | Six ~15-line wrappers; `session/rename` already establishes the pattern and the timeout constant | trivial |
| Schedule create + running-job inspect | reachable | Two methods, **both already implemented in the mock** — zero test-side work. Today a schedule can only be born by scheduling a recipe | trivial |
| Recipe authoring + deep-link decode (5 methods) | reachable | The mock already answers `recipes/{decode,parse,save,slash-command}`, and the full `Recipe` DTO with its casing rules exists. Same serializer in the other direction | trivial |
| Per-tool permission policy (4 methods) | reachable | `tools/permissions/set` is the natural sibling of the modal `AcpEvent::Permission` already drives | trivial |
| Preferences + defaults (6 methods) | reachable | `PreferenceKey` is a closed five-variant enum, so the DTO is fully specifiable and round-trip-testable | trivial |
| Config store + prompt templates (8 methods) | reachable | `config/upsert` is already sent, so the write half exists; `config/read` is its inverse. **The prompt-template editor is the server-side answer to what the earlier survey filed as ".goosehints: Electron-only"** — four calls over a `{name, defaultContent, userContent, isCustomized}` record and a textarea | small + one screen |
| Edit-and-resend (fork / truncate) | reachable | Three methods across two namespaces; `session/fork` is base-ACP, not goose-namespace, and the mock has no fork | medium |
| Server-side dictation | reachable | Ship 2 of 10 (`transcribe`, `config`); the model-management 8 live on the server. Capture is JS-owned MediaRecorder. **The cost is not the methods** — it is `NSMicrophoneUsageDescription` in the plist and mic-permission behaviour under WKWebView on a physical iPhone | medium, device-gated |
| Local inference management (11 methods) | reachable, declined on scope, **and gated at compile time on the server** | The download runs entirely inside goose (`custom_dispatch.rs:863-867`); the phone only starts a job and polls a snapshot, and `reconnect_loop` (`src/state.rs:666-681`) already absorbs the drop. So the earlier "the phone can't hold a multi-GB download" objection is a category error. New constraint, read from goose's source and not yet observed on a wire: `local-inference` is a **Cargo feature** of the `goose` crate — in `goose-cli`'s defaults, absent from the `portable-default` musl release build — and the not-enabled arm returns `-32602`, which `goose/mod.rs:142` does *not* convert to `AcpError::Unsupported`. This row would need its own capability probe rather than riding the soft-degrade. Declined because the control belongs where the disk is, not because it is hard | small effort |
| Provider administration (19 methods) | declined on scope | Eighteen independent wrappers with no ordering constraint — the most parallelisable item in the whole set, and rating it "large" misfiles a product decision as a cost. The nineteenth is not a wrapper: on goose main, `providers/config/authenticate` runs an interactive OAuth device flow, pushes a notification `forward_notification` currently drops at its `_ => {}` (`client.rs:580`), outlives the 30s `MUTATE_TIMEOUT` (`goose/mod.rs:100,104`), and opens a browser and writes the clipboard **on the server host** — which for a Tailscale client is not the reader's machine. Price it separately. Declined because a remote client talks to a server that is already configured, and because `providers/config/save` means typing an API key into a phone. The read-only 3-method slice ("why is this model unavailable") is trivial and defensible alone | medium effort, declined |
| Skills authoring (5 methods) | declined | Already a recorded decision (`crates/goose-acp-client/src/goose/skills.rs:1-20`), and the reason still holds: goose Desktop's own Add Skill button ships `hidden` with `title="Coming soon"` | — |
| MCP-UI "apps" | declined on design | Declaring `mcpHostCapabilities` is a promise to render arbitrary server-supplied HTML. **The security half of the old reasoning does not survive**: the app already injects server-derived HTML via `dangerous_inner_html` at seven sites, so a separate-scheme opaque-origin iframe would be *more* isolated than what ships. The reason that survives is the design system — extension markup does not obey our tokens, our tiered rounding or Dynamic Type, and it cannot be gallery-captured | very large |

Two mechanism notes for whoever writes this. The iframe blocker is the
navigation handler, not the protocol: `webview.rs:369-393` allows a `dioxus://`
navigation exactly once and cancels everything after, and wry does not filter
that policy to the main frame — so `<iframe src>` is dead on the default scheme,
but a second scheme registered via `Config::with_asynchronous_custom_protocol`
falls through to `navigation_handler(...).unwrap_or(true)` and is allowed with
its own origin. And `UserWindowEvent` lives in dioxus-desktop's private `mod
ipc` and is not re-exported, so a custom event handler must let inference name
the type rather than writing it.

### Shell and desktop UI

| Capability | Status | What it needs | Cost |
|---|---|---|---|
| Desktop gallery axis | **prerequisite, not a feature** | `docs/design.md:1270-1283` states plainly that the gallery covers the phone shell only and that nothing in `assets/desktop.css` (1,024 lines) is audited, and names both blockers: `scripts/capture-gallery.py` "writes the store wholesale from one log", and `window.__dumpKey` (`src/domdump.rs:68`) is the bare screen name so a desktop `chats` overwrites the phone's. The script does already have a `--merge` flag, which is half of the first blocker | small, and it must be first |
| ⌘R as a real menu item | reachable | The debt is logged at `docs/design.md:1185-1188`, deferred because "a menu accelerator *consumes* the key on macOS, so it would have to replace the JS listener rather than sit beside it". A binding table makes that one row changing `Js(REFRESH_KEY)` → `Accel("CmdOrCtrl+R")`, with a duplicate-chord test guarding the swap | trivial |
| Fix the last five ambient `Shell::CURRENT` reads | cleanup | `src/views/mod.rs:68`, `:72` (`SwipeDelete`), `src/views/chrome.rs:274`, `:308`, `:312`, `src/views/scheduler.rs:82`. These are the last places a view reads the ambient shell instead of taking it, and it is why `SwipeDelete`'s mobile arm is verified by nothing | trivial |

**On the accelerator question, which had a real answer nobody found.** Adopting
a richer menu bar does *not* force the three `document::eval` chords off.
wry's `WryWebView::performKeyEquivalent` only short-circuits `Bool::NO` when the
webview `is_child`, with a comment saying exactly why ("overriding this method
also means the cmd+key event won't be handled in webview, which means the key
cannot be listened by JavaScript" —
`wry-0.53.5/src/wkwebview/class/wry_web_view.rs:45-57`). This app builds with
`build(&window)`, not `build_as_child`, so web content sees the ⌘-chord first
and the menu item is the fallback. The thing that *would* break the chords is
adopting `Config::with_as_child_window()`.

## Sequencing

Stages, each independently shippable. What does and does not compress is
marked, because it is the only part of an estimate that is load-bearing here.

**Stage 0 — the manifest commit.** Reduce `[features]` to the `dx`-autodetect
marker (not delete it — see above), add two target dependency tables, collapse
eight `cfg`s, delete ~25 lines of prose that now describes nothing. Two lines
off the CI iOS job: `cargo check --package goose-mobile --target
aarch64-apple-ios` becomes the whole command. *Compresses to one person; it is
one commit and splitting it produces only conflicts.* All four gates were run on
this edit and pass. One unrelated thing the run turned up and that this commit
should carry: `FULLSCREEN_CLASS` (`src/viewport.rs:82`) is not `cfg`-gated while
its only caller `use_fullscreen_class` (`:105`) is, so the iOS check emits a
`dead_code` warning today. `.github/workflows/ci.yml:11` sets `RUSTFLAGS: -D
warnings` for the whole workflow, so that warning is very likely a red iOS job —
though the check was run plainly and never re-run with that variable set, which
is one command away from being a fact rather than an inference.

**Stage 1 — the five ambient `Shell::CURRENT` reads.** Do this before anything
is built on top, because every new capability that takes a `Shell` parameter
inherits the pattern. *Compresses trivially; five call sites in three files.*

**Stage 2 — the desktop gallery axis. NOT COMPRESSIBLE, and it must be first
among the UI work.** `scripts/capture-gallery.py` drives one booted simulator
through 16 keyed states, and its docstring is explicit that a run REPLACES the
gallery, because a state carried over from another branch keeps passing
`docs/audit.js` forever while the markup no longer exists. N agents each adding
a screen cannot each re-capture. This converts every screen-bearing feature from
a per-item cost into a per-batch one, and it is the single largest unpriced item
across every prior analysis. Until it exists, the desktop half of a tree whose
whole argument is "regressions are caught automatically" is caught by nothing.

**Stage 3 — the protocol backlog. Compresses almost perfectly.** A wrapper is
~15 lines (`crates/goose-acp-client/src/client/session.rs:254-262` is the
pattern). The contract fixture already pins all 114 methods, so nothing needs
re-vendoring and a contract test lands the same day as its wrapper. The mock's
dispatch is a five-element array carrying the comment "Alphabetical, so five
branches appending here merge deterministically"
(`crates/mock-goose-server/src/features/mod.rs:23-29`), and it already answers
30 goose-namespace methods — more than the client sends. **One exception that
serialises:** `crates/mock-goose-server/src/turn.rs` stamping
`_meta.goose.activeRunId` is the only mock change in this entire document that
mutates existing behaviour rather than adding a match arm, so it conflicts with
any branch touching `turn.rs`.

**Stage 4 — `src/platform/` and `Action`, with a test.** The
zero-`cfg(target_os)` invariant in `views/` is today a convention that has never
been tested. **Write the source-scanning test — in the shape of
`viewport.rs:563` — before `src/platform/` exists**, or the first capability
that is 60% native and 40% view will put its 40% behind a `cfg` in a view and
nothing will say so. Then the safe, self-contained capabilities fan out: tray,
hotkey, dock control, vibrancy, overlay window, menu bar. *Compresses well
once the layer and its test exist.*

**Stage 5 — shared state. NOT COMPRESSIBLE, and it is the flagship
serialising item.** `AppCtx` is roughly 50 `Signal` fields built by `use_signal`
inside one component scope (`src/state.rs:295-487`, constructed `:489-554`), so
the generational-box owner is window A's root scope: window B reading them
panics the moment A closes, and `WindowCloseBehaviour::WindowHides` preserves
the footgun rather than fixing it — A can then never truly close. This is one
function in the one file all seventeen live worktrees already merge into, whose
own comment at `:541-545` names it as the merge hazard, and splitting it across
agents produces nothing but conflicts.

One clarification that shrinks it. *Sharing the WebSocket does not require
sharing any Signal.* `AcpClient` is `{ tx: UnboundedSender<Cmd>, unsupported:
Arc<Mutex<HashSet<&'static str>>> }` (`client.rs:118-131`) — a plain `Clone`
value with no generational box and no runtime affinity, and `src/state.rs`
already treats it that way at eight call sites. So window B can have its own
`use_app_ctx_provider` (its own signals, its own owner, nothing that dies when A
closes) and be handed a clone of A's client instead of calling `establish()`.
The single genuinely single-consumer piece is the `mpsc::Receiver<AcpEvent>`,
and `tokio::sync::broadcast` is available today (`tokio` is a direct dependency
with the `sync` feature). **If multi-window is wanted, do the connection-sharing
version; the full re-owning refactor is only needed if windows must share
reactive state, which no capability on this list requires.**

**Stage 6 — bundling and entitlements. NOT COMPRESSIBLE (Apple's clock).**
`Dioxus.toml` carries only `[bundle] identifier` and `[bundle] publisher` —
there is no plist hook, so `CFBundleURLTypes` (deep links) and
`NSMicrophoneUsageDescription` (dictation) need the *same* post-build
PlistBuddy-plus-re-sign step. Three prior analyses each charged this to their
own feature, which inflates all three. Paid once as a build step it amortises
across every future entitlement. Two corrections from actually building a
bundle: the notification **bundle ID** is not part of this — `dx build
--platform desktop` already emits `CFBundleIdentifier = com.goosemobile.app`
from `Dioxus.toml`. What that bundle does *not* have is a signature of its own;
`codesign -dv` reports `Identifier=goose_mobile-8027ddba…`,
`adhoc,linker-signed`, `Info.plist=not bound`. A scratch probe in that state was
refused by `UNUserNotificationCenter` and accepted after
`codesign --force --sign - --identifier <bundle id>` — but that was a different
binary and the A/B had a confound, so "this stage owes notifications a
`codesign` step" is the current hypothesis and is listed as unrun at the end.

**Stage 7 — `crates/goose-native/`, only when the first capability that
genuinely needs `unsafe` arrives.** Given the correction above, that is later
than anyone expected: vibrancy and window level demonstrably do not need it —
both compiled and linked inside `goose-mobile` under `-F unsafe_code` — and
Prevent Sleep's block form probably does not, on a source read nobody has yet
put through a compiler. What remains is context menus, the dock menu and the
dock icon image, plus Prevent Sleep if that `cargo check` comes back red.

### On scale

The measured baseline: **122 commits in six days** (2026-08-21 through
2026-08-26), producing **37,808 lines of Rust and 5,611 lines of CSS**, with
seventeen named worktrees live at once under `.claude/worktrees/`. The
repository is visibly built for that — the mock's handler array and
`use_app_ctx_provider` both carry comments about how sibling branches merge into
them. Against that baseline a fifteen-line method wrapper is not a cost line
item, and calling the 92-method protocol backlog "multi-quarter" would be
nonsense: eight agents in eight worktrees land the client-plus-contract half of
all of it in a day.

The honest shape is that **the code is not the constraint and never was.** What
is left after the code compresses to nothing is six things, all in the list
above: the gallery pass (one simulator, one operator), the shared-state refactor
(one file, one person), the handful of things only a running machine can answer
(does the shipped WKWebView dispatch button 3; does a `dx`-built, linker-signed
bundle get to raise a notification; does the global chord collide with the
user's Raycast; does mic permission work under WKWebView on a real iPhone), the
one-way decisions (declaring
`elicitation` is a promise the server acts on the next turn — you cannot ship
60% of it behind a flag the server can see; likewise exposing `shell`), Apple's
notarization latency, and the one mock change that mutates rather than appends.
Notably absent from that list: "needs an upstream change to the goose server."
All 114 declared methods resolve to a `#[request(method = …)]` type, a
`#[custom_method(T)]` dispatch arm and an `on_*` handler defined under
`crates/goose/src/acp/`, with no `todo!` or `unimplemented!` anywhere in that
tree — checked exhaustively, not sampled. Two qualifications the exhaustive pass
added. Some of those handlers are compiled out rather than always present:
`local-inference` (13) and the local half of `dictation` (7) are behind a Cargo
feature of the `goose` crate, `session/share/nostr` behind another, and
`schedules/*` (12) behind the runtime `--enable-scheduler` flag; the
`portable-default` musl release build has none of the Cargo ones, and
`acp-meta.json` is itself generated with those features on, so the fixture
over-declares against such a build. And `session/fork` is base ACP, not one of
the 114, as the fork/truncate row already says. The usual excuse still applies
to none of this.

## Phone implications

This document is mostly about a desktop shell, and the phone is the product. So:

**The phone's rendering must not change.** `assets/main.css` is the shared
design system and nothing here re-renders it. Vibrancy is the one item that
touches tokens, and the correct form is desktop-only tokens plus a deliberate
gallery re-capture — which is precisely why Stage 2 comes before Stage 4 and not
after. If a capability cannot be built without changing a token the phone reads,
that is a signal to reconsider the capability, not the token.

**`src/platform/` is not a desktop layer.** It is gated by `Surface`, which has
five arms, and `ios.rs` and `android.rs` are first-class members of it. Several
capabilities on this list are *better* on a phone than on a desktop:

- **Server-side dictation** is the most phone-shaped thing in the missing 92,
  and the reference gets it from the server, not from Electron. `transcribe`
  takes base64 audio and returns text. Hold-to-talk is a better composer on a
  phone than a keyboard is.
- **Session export** hands a markdown string straight to a share sheet.
- **Recipe deep-link decode** means a shared `goose://` recipe opened from
  Messages — an affordance a desktop does not really have.
- **`session/steer`** matters more on a phone, where the composer going dead
  mid-turn is a bigger fraction of the interaction.
- **Slash-command and @-mention autocomplete** is a pure composer win on both,
  and the composer is the phone's whole surface.

**Three things get worse on the phone and should be stated rather than
discovered.** Notifications: an "agent finished" event cannot fire while the app
is backgrounded, for the reason in The impossible list, so any UI that implies
it will is a lie. Long-running server jobs: the local-inference progress
endpoint is a poll, so a phone watching a download has to stay awake — the
download itself is fine, the *watching* is not. And `rfd`: it must be
target-gated out of the iOS build entirely or the iOS check stops compiling, so
any iOS file affordance is a JS-owned Blob or a share sheet, not a picker.

**The compile-time rule protects the phone here, and Stage 0 strengthens it.**
After the manifest change, `dioxus::desktop` is unnameable on iOS because the
dependency does not exist for that target, not because someone remembered a
flag. That was measured — a probe naming `dioxus::desktop::Config` under
`cargo check --target aarch64-apple-ios` gives `error[E0433]: cannot find
'desktop' in 'dioxus'`, pointing at `dioxus-0.7.10/src/lib.rs:109`. One
qualification: the probe was run on the variant with `[features]` deleted
entirely, not on the marker version that Stage 0 actually recommends. The marker
enables nothing, so the guarantee should hold identically, and only the four
cargo gates — not the probe — were re-run on the final manifest.

## Open questions

**1. ~~Does `dx` read target-conditional dependency tables?~~ ANSWERED: yes.**
`dx build --ios` and `dx build --desktop` both build from the target tables with
no `--features` flag. The question that replaces it is narrower and is already
answered too: `dx`'s *platform autodetection* reads `[features]` keys and the
first `dioxus` dependency entry, so `[features]` must survive as a marker or
`dx check` and bare `dx build` fail with "Could not automatically detect target
triple". Recorded here because it is the kind of thing a `dx` bump could change
in either direction; re-run `dx check` after one.

**2. Multi-window: share the connection, or re-own the state?** These are
different projects. Sharing the connection is small and needs no refactor.
Re-owning ~50 signals so windows can share reactive state is the large
serialising item. *Recommendation:* share the connection. No capability in this
document requires two windows to observe the same signals, and the version that
does can be reconsidered when one does.

**3. When does `crates/goose-native/` get created, and what stops it growing?**
*Recommendation:* create it when the first capability needs it — which, after
the safe-fn correction, means context menus or the dock menu, both of which are
polish. Write its ceiling into its own `Cargo.toml` comment and add a CI check
that it depends on neither `goose-mobile` nor Dioxus.

**4. Does the WKWebView we actually load dispatch `button: 3`?** WebKit trunk
says it should, and that is still reading source rather than running anything,
against a tree that is not the one macOS 26 ships. *Recommendation:* do not
write the handler on the strength of the source read. The cheapest settling
move is not hardware — it is a synthetic `CGEventCreateMouseEvent(NULL,
kCGEventOtherMouseDown, pt, 3)` posted at the running app with a temporary
`onmouseup` logger in the shell, which exercises NSEvent → WKWebView → WebCore →
DOM → Dioxus in one go. It costs an Accessibility (TCC) grant, which is the
reader's to give. The fallback needs the boundary crate and is therefore a
different decision.

**5. Should the app declare `clientCapabilities.elicitation`?** Today we answer
`-32601` to elicitation requests, which strands any goose flow that asks a
structured question — that is a correctness gap, not a missing feature.
*Recommendation:* yes, and as a single uninterruptible piece of work. Declaring
it half-built is strictly worse than today, because a loud `-32601` at least
fails visibly.

**6. Should the `shell` tool be exposed at all?** The `.goosehints` editor and
several other affordances route through an unpermissioned `tools/call`.
*Recommendation:* decide this on its own, once, in writing. It is arbitrary
remote command execution reachable from a phone, and "it was needed for the
hints editor" is not a decision.

**7. Does vibrancy actually look right against this design system?**
`assets/main.css` surfaces are opaque by construction and its shadow ramp
assumes them. *Recommendation:* prototype it after Stage 2 exists, so the answer
is a captured state somebody can look at rather than an argument.

**8. Is the `Action` funnel's completeness checkable?** `refresh_named`'s
fallthrough is `_ => {}` and `Action::perform` will inherit that, so a menu item
can name an action nothing performs, guarded only by a third source-scanning
test. *Recommendation:* accept the third instance for now, but note that three
occurrences of the same workaround is the point at which the class is worth
solving — probably by making the destination table generate both sides.

**9. Does the bundle `dx` produces need its own signature to raise a
notification?** `dx build --platform desktop` leaves `GooseMobile.app`
linker-signed with `Info.plist=not bound`, and a scratch probe in that state was
refused by `UNUserNotificationCenter` and accepted after an ad-hoc `codesign
--force --sign - --identifier <bundle id>`. That comparison had a confound (the
refused run also lacked a permission grant for its identity), and the real app
bundle was never launched. *Recommendation:* settle it before pricing the
notifications row — launch the built `.app`, watch for the permission alert,
then re-sign and launch again. If the answer is yes, Stage 6 owns a `codesign`
step, which is a packaging change nobody reading Rust would have found.

**10. Who re-vendors the protocol fixture, and how often?** The pinned fixture
was already stale by two entries against goose `origin/main` within a week, and
nothing in CI notices. *Recommendation:* make
`scripts/vendor-acp-contract.py` plus `git diff` a periodic chore with a named
owner, and fix the contract test's failure message first — it currently asserts
that an absent `acp-meta.json` entry means `-32601`, which was false for
`session/delete` the moment that method moved to the standard SDK.

## What would falsify this document — and what happened when it was tried

This section used to be a list of things reasoned from source that nobody had
executed. A later pass executed the ones a compiler, a linker or a running
process could settle. Each entry now records what was **run**, what was
**observed**, and a verdict. The ones that still could not be run stay here,
marked as unrun, with the thing that would settle them — an unrun claim written
in the voice of a finding is the exact failure this document exists to correct,
so nothing below is upgraded on the strength of an argument. Everything was run
in scratch worktrees against this lockfile; none of it was merged.

### Settled by running something

**The `dx` manifest assumption — PARTLY FALSIFIED. Stage 0 survives in a
different shape.** Run: the manifest edit (delete `[features]`, add two
target-conditional `[dependencies]` tables), the eight `cfg`s collapsed, then
`cargo check`, `cargo test --workspace`, `cargo check --target
aarch64-apple-ios`, `cargo clippy --workspace --all-targets -- -D warnings`,
then `dx check`, `dx build --desktop` and `dx build --ios` under
`dx --version` = `dioxus 0.7.10 (57d6794)`. Observed: every cargo gate passes
(215 tests in `goose-mobile`, 0 failed), `cargo tree -e features -i dioxus`
reports `feature "mobile"` alone on `aarch64-apple-ios` and `feature "desktop"`
alone on the host, and **both `dx build --ios` and `dx build --desktop` produce
a bundle with no `--features` flag at all** — so `dx` does read
target-conditional dependency tables. But with `[features]` deleted outright,
`dx check` fails:

```
ERROR dx check: Could not automatically detect target triple
```

against `INFO No issues found.` from the same command on an unmodified copy of
the tree. The break is `dx`'s platform *autodetection*, not its build:
`dioxus-cli-0.7.10/src/build/renderer.rs:7-30` does a single
`dependencies.iter().find(|dep| dep.name == "dioxus")` and then falls back to
reading `[features]` keys, and a manifest with neither gives it nothing to
read. Verdict: the fallback this document proposed — enable both features
unconditionally — is unnecessary and strictly weaker. Keeping a marker
`[features] default = ["desktop"]` / `desktop = []` that enables nothing and
that no `cfg` in the crate reads restores `dx check` and bare `dx build` while
leaving all eight `cfg`s collapsed. Stage 0 above now says that.

**"Calling a safe `fn` in a dependency does not violate `unsafe_code =
"forbid"`" — CONFIRMED by compiler, and the escape hatch is confirmed shut.**
Run: `MainThreadMarker::new`, `sharedApplication`, `keyWindow`, `setLevel`,
`setCollectionBehavior`, `setOpaque`, `contentView`, `addSubview` and
`NSVisualEffectView::new`/`setMaterial`/`setBlendingMode`/`setState` written
into `goose-mobile` itself with zero `unsafe` tokens, then `cargo check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --
--check` and `cargo build`. Observed: all clean, and `otool -L` on the linked
binary lists `AppKit.framework` with `NSVisualEffectView` in its strings. The
gate is armed rather than absent — a control experiment adding one `unsafe`
block to the same file produces

```
error: usage of an `unsafe` block
  = note: requested on the command line with `-F unsafe-code`
```

and `#[expect(unsafe_code, reason = "...")]` over that block produces
`error[E0453]: expect(unsafe_code) incompatible with previous forbid`, which is
the `deny`/`forbid` distinction asserted above, measured. **Not covered by this
run: Prevent Sleep**, which is listed under "Still unrun" below — it is a
different crate and a different shape, and nothing resembling it was compiled.

**That `objc2-app-kit`'s enabled feature set stays where it is — CONFIRMED,
with a cost this document had omitted.** Run: `cargo tree --target
aarch64-apple-darwin -e features -i objc2-app-kit` before and after adding the
feature, `grep -c '^\[\[package\]\]' Cargo.lock` before and after, and the same
`cargo tree` against `objc2-foundation`. Observed: exactly **44** features
before with `NSVisualEffectView` absent and `NSApplication`, `NSWindow`,
`NSView`, `NSPanel`, `NSScreen`, `NSStatusItem` present; **45** after, a
one-line diff; package count `599` → `599`; `Cargo.lock` diff two lines, both
edges under `goose-mobile`. But `objc2-foundation`'s feature set is **51 before
and 51 after** — the four sub-features `NSVisualEffectView` declares
(`objc2-app-kit-0.3.2/Cargo.toml:1934-1939`) are already on, so "pulls in
exactly four things" describes what the feature *declares*, not what it costs.
What it does cost, and what this document did not mention, is a fingerprint
change on `objc2-app-kit` that recompiles `objc2-web-kit`, `muda`, `tao`, `wry`,
`rfd`, `global-hotkey`, `tray-icon`, `webbrowser`, `dioxus-desktop` and
`dioxus`. Blast radius of a bump that changed `extern_methods!`'s safety
generation, now that the safe route is compiled and known: exactly the vibrancy,
window-level and Prevent-Sleep rows. The tray row's `ns_status_item` and the
dock and overlay rows belong to `tray-icon` and `tao`, not to `objc2-app-kit`.

**"Nothing in the missing 92 is streaming" — FALSIFIED against goose as of
2026-08-21.** Run: `scripts/vendor-acp-contract.py` against
`origin/main` (`8d844eecb`, 123 commits past the pinned `3810898a`), then
`cargo test -p goose-acp-client --test contract` against both fixtures.
Observed: `methods pin 114 main 113`, `notifications pin 1 main 2`, `added
methods: []`, `removed methods: ['session/delete']`, and the new notification is
`_goose/unstable/providers/authentication/device-code`, added by goose commit
`4078158c0`. It is pushed from *inside* `providers/config/authenticate`, which
is one of the missing 92 — so the sentence is true at the pin and false on main,
and the falsifier this section predicted has already happened. Two further
findings fell out. `session/delete` was not removed from goose; it migrated to
the standard ACP SDK in `063694cf7` and left `acp-meta.json`, which covers only
goose-*custom* methods — so re-vendoring today makes the contract test assert a
lie about a method `crates/goose-acp-client/src/client/session.rs:240` sends
right now:

```
panicked at crates/goose-acp-client/tests/contract.rs:115:9:
goose declares no method `session/delete` — a -32601, which this crate reads as
a feature being switched off rather than as a typo
```

(control: the same sample against the pinned fixture is `4 passed; 0 failed`).
And nothing in this repository notices contract drift on its own, so a periodic
`scripts/vendor-acp-contract.py ~/git/goose && git diff` is a standing chore,
not a one-off. Caveat on the measurement: the goose checkout was never fetched,
only read, so `8d844eecb` is a *lower bound* on the drift.

**The notifications row — PARTLY FALSIFIED. `notify-rust` is now vendored, and
the obvious dependency line is a silent no-op.** Run: `notify-rust` 4.18.0 in a
scratch workspace member, built and executed both ways, with `/usr/bin/log show
--predicate 'process == "usernoted"'` as the ground truth for whether anything
reached the notification daemon. Observed, with the crate's **default**
features, i.e. exactly `notify-rust = "4.18"`:

```
show() -> Ok
usernoted: Legacy client com.apple.finder connecting to modern client.
           You can't mix modern clients with legacy clients.
usernoted: Denying message 3 from connection <LegacyConnection ...>
```

`Ok`, and no notification — the default backend is `NSUserNotificationCenter`
(`mac-notification-sys-0.6.15/objc/notify.m:79`, deprecated since 10.14) and it
swizzles the process's bundle id to `com.apple.Finder` when there is none
(`lib.rs:122`). A silent failure is the worst possible shape and it is what the
one-line version of this row buys. With `default-features = false, features =
["preview-macos-un"]`, from a `.app` that had been `codesign`ed ad-hoc with
`--identifier` equal to its `CFBundleIdentifier`, the same probe produced

```
usernoted: Presenting <NotificationRecord app:"com.example.notifyprobe" ...>
           as banner (["badge", "sound", "alert"])
```

— an actual banner on screen, after clicking Allow on the system permission
alert. `show()` does not request authorization for you (a bare binary returns
`Err: No bundle identifier found`, and an unauthorized bundle returns
`NotificationRejected` while `authorization_status` is `NotDetermined`), so a
one-time `notify_rust::request_auth()` is required, and it must be the **async**
one: the blocking form parked the caller for 2m12s while the alert was up.
Lockfile cost, measured by diff: `notify-rust = "4.18"` adds **38** entries
(a whole `zbus`/`async-io` stack that never compiles on macOS);
`default-features = false` plus `preview-macos-un` adds **12**, of which two
compile here. Wired into `src/state.rs` and `src/app.rs` for real, the workspace
clippy gate and the iOS check both pass. Verdict: buildable end to end, about
fifteen lines — but see "Still unrun" for the part of this that is not settled,
which is the part that matters for shipping.

### Still unrun, and what would settle each

**Whether the app bundle `dx` produces can raise a notification at all.** This
is the live risk in the row above and it was not tested. What is measured:
`dx build --platform desktop` emits `GooseMobile.app` with `CFBundleIdentifier
= com.goosemobile.app` (so the plist half of Stage 6 is already done for this
one key), and `codesign -dv` on it reports `Identifier=goose_mobile-8027ddba…`,
`flags=0x20002(adhoc,linker-signed)`, `Info.plist=not bound`. What is *not*
measured is what `UNUserNotificationCenter` does with that bundle. The scratch
probe's A/B does not settle it either: the rejected run happened seventeen
seconds *after* the accepted one on the same path, so "identifier must equal the
bundle id" and "this identity has no permission grant yet" are both consistent
with what was logged. Settles it: launch the built `GooseMobile.app` — the
`use_future`-at-mount call to `request_auth` needs no goose server and no
`end_turn` — see whether a permission alert appears; then
`codesign --force --sign - --identifier com.goosemobile.app` the bundle,
re-register it with `lsregister -f`, and run it again. Until that is done, "a
post-build `codesign` step is required" is a hypothesis, not a finding, and the
Stage 6 entry says so.

**The mouse-back-button DOM claim.** Still not executed; nothing was compiled or
run for it. What was done is *more reading*, which is what this exercise
excludes: WebKit trunk on `raw.githubusercontent.com` maps `buttonNumber` 3 to
`MouseButton::Back` in `PlatformEventFactoryMac.mm`, whose enum in
`MouseEventTypes.h` is documented as carrying the DOM's own numbering, forwarded
unfiltered by `WKWebViewMac.mm`'s `otherMouseDown:`. That is trunk, not the
WKWebView that ships in macOS 26 and that `wry` actually loads, so it is a
plausible proxy for the deployed binary and not the deployed binary. Two things
would settle it and neither needs a five-button mouse: post a synthetic
`CGEventCreateMouseEvent(NULL, kCGEventOtherMouseDown, pt, 3)` at the running
app with a temporary `onmouseup` logger in the shell — that exercises NSEvent →
WKWebView → WebCore → DOM → Dioxus end to end — or click a real one. The first
costs an Accessibility (TCC) grant, which is the reader's to give and was never
requested. Related and also unmeasured: which physical devices report
`buttonNumber` 3 at all is a property of the mouse and its driver, so the row
below asks for a mouse that reports it rather than naming a model.

**Prevent Sleep's safe route.** Never compiled. The two safety facts were
re-read and hold — `performActivityWithOptions_reason_usingBlock` is `pub fn` at
`objc2-foundation-0.3.2/src/generated/NSProcessInfo.rs:272` and `endActivity` is
`pub unsafe fn` at `:267` — but the vibrancy experiment proves nothing about it:
it is a different crate (`objc2-foundation` is not a direct dependency of
`goose-mobile` today), it needs the `block2` and `NSString` features named as a
direct dependency rather than transitively, and its argument is a
`&block2::DynBlock<dyn Fn()>` that has to be constructed with `RcBlock::new`.
Settles it: the same `cargo check` that settled vibrancy, with an
`objc2-foundation` entry added and one `performActivityWithOptions…` call.

**Whether `GlobalHotKeyManager` is constructed on every launch.** The static
half is measured and is strong: `otool -L` on the shipped debug binary lists
`Carbon.framework`, `nm -u` lists `_InstallEventHandler`,
`_GetApplicationEventTarget` and `_UnregisterEventHotKey` as undefined, and
disassembling `ShortcutRegistry::new` shows `bl … GlobalHotKeyManager3new`
followed by `bl … unwrap_failed`. That proves the call site is present and
reachable in the image. It does not prove it *executes*; that step comes from
reading the un-`cfg`'d `shortcut_manager: ShortcutRegistry::new()` field
initializer at `dioxus-desktop-0.7.10/src/app.rs:80`, and the binary was never
run. Settles it: `lldb` with a breakpoint on `InstallEventHandler`, one launch.
The row below has been reworded to say "constructed in `App::new` with no
`cfg`" rather than "on every launch".

**The server-side assumptions.** The working-directory picker and the
`.goosehints` editor both assume the `developer` extension is enabled with
`unprefixed_tools` on a machine this client does not control. Nothing was run
against a server that did not have it, so the failure shape is still predicted:
`goose_request` degrades a `-32601` into `AcpError::Unsupported` and caches it
(`goose/mod.rs:142`), so the feature would degrade silently rather than loudly.
Settles it: a `tools/list` against a goose with `developer` off.

**Server-side feature gating.** Answered from source, not from a wire, and that
distinction matters here because the claim is about what arrives on the socket.
Read: `local-inference` is a **Cargo feature** of the `goose` crate, not a server
flag — all 13 `local-inference/*` handlers are `#[cfg(feature =
"local-inference")]` with a `#[cfg(not(...))]` arm returning `-32602
invalid_params`; `apps/*` has no `cfg` at all; `dictation` is half-gated, and
`dictation/models/list` in the off case returns an empty `Ok`, not an error;
`schedules/*` is behind the runtime `--enable-scheduler` flag, which is the one
case `scripts/check-server.sh:83-87` already warns about. The sting, if that
reading is right: `crates/goose-acp-client/src/goose/mod.rs:142` converts only
`-32601`, so none of the `-32602` refusals become `AcpError::Unsupported` and
`dictation/models/list` degrades to a silent empty list. No goose binary was
built with or without those features and no method was called against a running
server, so this is a prediction about the wire, not an observation of it.
Settles it: build `goose --no-default-features --features portable-default`,
point `scripts/check-server.sh` at it, and call one `local-inference/*` method.

**The velocity baseline.** 122 commits in six days is measured, but it is
measured on a phone app built greenfield with no prior art to stay compatible
with. Capability work on an existing shell has a different shape: more of it is
merging into files that other branches are also editing, which is precisely the
serialising cost Stage 5 is about. Nothing can execute this one in advance; it
is settled by doing the work and re-measuring. If the sequencing above turns out
optimistic, this is where it will turn out optimistic — not in the wrapper count.

**The claim that a crate split is unnecessary.** It rests on the two
`include_str!` cross-layer tests and on `dx` cooperating — which it now
measurably does, given the marker feature. If the desktop's information
architecture genuinely diverges — multi-window, a launcher, a menu-driven
command surface with no phone analogue — then `views/` starts carrying props no
phone ever passes, and one shared layer becomes a fork wearing the shape of a
merge. Nothing here detects that state, and no build can. It would have to be
noticed by someone reading `views/` and asking how many of its props
`Shell::Mobile` ever sets.
