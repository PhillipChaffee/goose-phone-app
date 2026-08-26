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
`use_global_shortcut` (:114), and the shipped binary constructs a
`GlobalHotKeyManager` on every launch whether or not a hotkey is ever
registered. Spellcheck is an HTML attribute. Telemetry is an HTTP POST with a
`reqwest` already in the lockfile. The announcement modal is a fetch and a div.
The `.goosehints` editor is not a filesystem affordance at all for a remote
client — the file lives on the server and `_goose/unstable/tools/call` reaches
it, because `on_call_tool` (`/Users/phillipchaffee/git/goose/crates/goose/src/acp/server/tools.rs:60-90`)
dispatches straight into `extension_manager` with **no permission gate**, its
only filter being `is_tool_visible_to_app`, which returns `true` for any tool
that declares no `_meta.ui.visibility` (`reply_parts.rs:787-801`).

Three things are genuinely impossible, and none of the three is impossible
because of Electron.

**1. A notification on iOS that an agent finished while the app is
backgrounded.** The event that would fire it arrives on a WebSocket, and iOS
suspends the socket when the app leaves the foreground, so there is nothing
running to raise a local notification. The route that exists for this on iOS is
a remote push, and a remote push needs something on the server to send it —
goose has no push endpoint. The protocol has exactly one agent→client
notification (`_goose/unstable/session/update`) and exactly one agent→client
request (`_goose/unstable/session/recipe/request-params`), both verified as the
only entries in the `notifications` and `agentRequests` arrays of
`crates/goose-acp-client/tests/fixtures/acp-meta.json`. This is not a client
gap. It is a missing capability on both sides at once, and neither side is ours
alone to add. The macOS half of the same capability is fine: a foreground
process can raise a notification when a turn ends.

**2. Local `goose serve` lifecycle on iOS and Android.** The mechanism is not
exotic — goose's own Electron version is `child_process.spawn` plus a readiness
poll, and `std::process::Command` is the same thing with no new dependency. But
iOS forbids executing a binary that is not the app, so the capability does not
exist on the platform. It is buildable on the desktop shell in an afternoon and
is separately out of scope there, for a reason that is about this app's premise
rather than its capability: the entire networking layer, `scripts/check-server.sh`
("Run this ON the VPS that runs goose") and `docs/iphone-setup.md` exist because
goose is somewhere else. Recording it as "trivial to build, dead on two of four
targets, and deliberately declined on the other two" is more useful than a cost
tier.

**3. Auto-update on iOS and Android.** Store distribution. A signed, notarized
macOS `.app` that rewrites its own bundle also invalidates its own signature
unless something like Sparkle drives the replacement, so the honest shape is:
an updater would be a macOS-only path duplicating what the two stores already
do for the other two targets. `self_update` 0.42.0 would integrate in an
afternoon — `reqwest`, `hyper`, `flate2` and `semver` are already in the
lockfile — and the cost is entirely notarization round-trips and a distribution
channel that does not exist. Filing this as "large" reads as "lots of code",
which is the opposite of true.

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
wrong**, and it is wrong on the exact distinction between `deny` and `forbid`.
A prior claim in the other direction — that *every* AppKit route needs unsafe —
is also wrong, and that is the more consequential error, because it is what
made vibrancy look expensive. See the next section.

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
| `NSWindow::contentView` | `NSWindow.rs:760` |
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
The crucial consequence is that you never need `tao`'s
`WindowExtMacOS::ns_window() -> *mut c_void` (`tao-0.34.8/src/platform/macos.rs:24`),
which is where the earlier analysis assumed the pointer cast had to live: the
`NSWindow` is reachable entirely through `sharedApplication(mtm).keyWindow()`,
so no pointer is ever cast here. The app crate contains zero `unsafe` today and
would still contain zero after adding vibrancy, window level and collection
behaviour.

Vibrancy in particular is therefore cheaper than every prior estimate.
`objc2-app-kit` is already in `Cargo.lock:3041` and already compiled for the
macOS target. `cargo tree --target aarch64-apple-darwin -e features -i
objc2-app-kit` reports **44 enabled features** including `NSApplication`,
`NSWindow`, `NSView`, `NSPanel`, `NSScreen`, `NSStatusItem` — and
`NSVisualEffectView` is not among them. Enabling it pulls in exactly four
things, all of them sub-features of a crate that is already compiled:
`objc2-foundation/{NSCoder, NSGeometry, NSObject, objc2-core-foundation}`
(`objc2-app-kit-0.3.2/Cargo.toml:1934-1939`). Zero new crates, zero network
fetch, zero unsafe. `cocoa` 0.26.1 — recommended by an earlier pass as the
cheapest route because it is already in `Cargo.lock:388` — is the *worst* route,
because every call in it is a raw `msg_send!`. Being in the lockfile is not
being callable.

Prevent Sleep gets the same correction one step further. `begin` is safe and
`endActivity` is not, so the obvious begin/end pair is unwritable here — but
`performActivityWithOptions_reason_usingBlock` (`NSProcessInfo.rs:272`) is safe
and is gated on `NSString` + `block2`, **both of which are already enabled**
(verified: `cargo tree --target aarch64-apple-darwin -e features -i
objc2-foundation` lists `NSProcessInfo`, `NSString` and `block2`). It holds the
assertion for the duration of a synchronously-executed block, so the shape is a
parked thread on a channel rather than a `use_effect` — a different design at a
similar cost, and it needs no unsafe and no boundary crate.

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
predicate that is implied by the other half. Replacing `[features]` with two
target-conditional `[dependencies]` tables collapses all eight to a single
`not(any(ios, android))` and makes the guarantee *stronger*, because it stops
depending on remembering `--no-default-features --features mobile` at
`.github/workflows/ci.yml:127-128`. That is the first commit: a manifest edit,
eight `cfg`s halved, and roughly 25 lines of now-false prose deleted
(`src/main.rs:63-72`, `src/shell/desktop.rs:45-51`). No files move.

**Uncertainty worth stating:** this pass ran no build, so it is unverified that
`dx serve --desktop` and `dx bundle` read target-conditional dependency tables
the way `cargo` does — `dx` interprets the manifest itself and has historically
had opinions about the `desktop` and `mobile` features by name. If it needs the
literal feature, the fallback is to enable both unconditionally, which still
collapses the double gate (because `dioxus-desktop/src/lib.rs:44` gates `muda`
on `target_os` regardless) but leaves `dioxus::desktop` nameable on iOS — a
weaker guarantee than the target table gives. Check this before writing the
commit, not after.

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
| Global hotkey (quick-launcher chord) | reachable, already paid | `use_global_shortcut` (`hooks.rs:114`). The `GlobalHotKeyManager` is constructed unconditionally on every launch today. macOS uses Carbon `RegisterEventHotKey`, so no TCC prompt | trivial code |
| Menu bar (replace, not adopt) | already shipping | `src/main.rs:86-88` never calls `with_menu`, so `MenuBuilderState::Unset` resolves to `default_menu_bar()` (`config.rs:45-52`) and Window+Edit menus with ⌘C/⌘V/⌘W/⌘Q are on screen now. Replacing it is `Config::with_menu` | small |
| Frameless, always-on-top overlay window | reachable | `tao` safe builders: `with_decorations(false)` (`window.rs:493`), `with_always_on_top` (:516), `with_transparent` (:482), `with_visible_on_all_workspaces` (:589). The repo already uses this API at `src/main.rs:81-87` | small |
| Window vibrancy | reachable, no unsafe | One line enabling `objc2-app-kit`'s `NSVisualEffectView` feature. Zero new crates. **The real cost is CSS**: `assets/main.css` surfaces are opaque tokens, so this means desktop-only tokens and a deliberate gallery re-capture | medium (CSS, not objc) |
| Window level / collection behaviour (true floating panel) | reachable, no unsafe | `NSWindow::setLevel` (:1327), `setCollectionBehavior` (:1446) via `sharedApplication(mtm).keyWindow()` | trivial |
| Dock: activation policy, visibility, badge, hide/show | reachable | All safe `fn` on tao's `EventLoopWindowTargetExtMacOS` (`platform/macos.rs:397`, `:403`, `:406`, `:387`, `:389`). A tao-event path, so no synchronous-XHR cost | trivial |
| Dock: replace the icon image | **blocked by our lint policy** | `setApplicationIconImage` is `pub unsafe fn` (`NSApplication.rs:737`). Needs `crates/goose-native/` | small, after the seam |
| Dock menu (right-click the dock icon) | **blocked by our lint policy** | `applicationDockMenu:` on a delegate subclass; `define_class!` expands to `unsafe impl` in the caller's crate | medium, after the seam |
| Native context menus (right-click) | **blocked by our lint policy** | `muda`'s `unsafe fn show_context_menu_for_nsview` (`lib.rs:445`); no safe macOS sibling. Also needs `with_disable_context_menu(false)` — the release build currently suppresses WebKit's own (`config.rs:117`) | small, after the seam |
| Prevent Sleep | reachable, no unsafe | `performActivityWithOptions_reason_usingBlock` (`NSProcessInfo.rs:272`) — safe, and `NSString`+`block2` are already enabled. Holds for a block's duration, so: a parked thread on a channel, not a `use_effect` | small |
| Multiple windows / session in a new window | mechanism reachable, **gated** | `DesktopService::new_window` (`desktop_context.rs:141`) is routed end-to-end already. The deliverable is gated on the shared-state question below. See Sequencing | trivial + a large prerequisite |
| Double-click titlebar to zoom | reachable | `toggle_maximized()` (`desktop_context.rs:169`) as `ondoubleclick` on the drag strip that already carries `onmousedown -> drag_window()` at `src/shell/desktop.rs:425-432`. One DOM event, inside the rule | trivial |
| Mouse back button | reachable, **unverified on hardware** | Not via tao — `platform_impl/macos/view.rs:962-970` collapses every non-left/right button to `MouseButton::Middle`, and wry's WKWebView covers tao's content view anyway (the finding already recorded at `src/main.rs:129-137`). Via the DOM: `MouseButton::Fourth` from `button: 3` (`dioxus-html/src/input_data.rs:22`, `:42`). Whether WebKit dispatches button 3 for a real Magic Mouse back click needs a device | trivial code, device-gated |
| Native notifications | needs a new dep, macOS only | Nothing in the lockfile does this; `notify-rust` is the candidate and **could not be verified against vendored source**, so no API is cited. Needs the bundle-ID plist work below. iOS half is impossible (see The impossible list) | medium |
| Auto-update | declined | Distribution, not code. See The impossible list | small code, large ops |

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
`3810898a`). We speak 21. **Nothing in the missing 92 is streaming** — every one
is plain request/response, including the two that look like streams
(`local-inference/models/download/progress` and the dictation equivalent are
polled snapshots). Note that `crates/goose-acp-client/src/error.rs:167-194`
lists more method strings than we send; that is the `Feature::of_method`
classification table, and a naive grep of the tree overstates coverage by six.

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
| Local inference management (11 methods) | reachable, declined on scope | The download runs entirely inside goose (`custom_dispatch.rs:863-867`); the phone only starts a job and polls a snapshot, and `reconnect_loop` (`src/state.rs:666-681`) already absorbs the drop. So the earlier "the phone can't hold a multi-GB download" objection is a category error. Declined because the control belongs where the disk is, not because it is hard | small effort |
| Provider administration (19 methods) | declined on scope | Nineteen independent wrappers with no ordering constraint — the most parallelisable item in the whole set, and rating it "large" misfiles a product decision as a cost. Declined because a remote client talks to a server that is already configured, and because `providers/config/save` means typing an API key into a phone. The read-only 3-method slice ("why is this model unavailable") is trivial and defensible alone | medium effort, declined |
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

**Stage 0 — the manifest commit.** Delete `[features]`, add two target
dependency tables, collapse eight `cfg`s, delete ~25 lines of prose that now
describes nothing. Two lines off the CI iOS job. *Compresses to one person; it
is one commit and splitting it produces only conflicts.* Verify the `dx`
question first.

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
there is no plist hook, so `CFBundleURLTypes` (deep links),
`NSMicrophoneUsageDescription` (dictation) and a notification bundle ID all need
the *same* post-build PlistBuddy-plus-re-sign step. Three prior analyses each
charged this to their own feature, which inflates all three. Paid once as a
build step it amortises across every future entitlement.

**Stage 7 — `crates/goose-native/`, only when the first capability that
genuinely needs `unsafe` arrives.** Given the correction above, that is later
than anyone expected: vibrancy, window level and Prevent Sleep no longer need
it. What remains is context menus, the dock menu and the dock icon image.

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
(one file, one person), real-device verification (does WebKit dispatch button 3;
does the global chord collide with the user's Raycast; does mic permission work
under WKWebView on a real iPhone), the one-way decisions (declaring
`elicitation` is a promise the server acts on the next turn — you cannot ship
60% of it behind a flag the server can see; likewise exposing `shell`), Apple's
notarization latency, and the one mock change that mutates rather than appends.
Notably absent from that list: "needs an upstream change to the goose server."
All 92 unimplemented methods already exist server-side and are pinned in a
fixture in this repository. The usual excuse applies to none of this.

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
flag. That is a better guarantee than the one we have.

## Open questions

**1. Does `dx` read target-conditional dependency tables?** Stage 0 depends on
it and this pass ran no build. *Recommendation:* verify with `dx serve
--desktop` and `dx bundle` before writing the commit. If it insists on the
feature by name, fall back to enabling both features unconditionally — the
double gate still collapses, but `dioxus::desktop` stays nameable on iOS and the
`cfg` in `shell/mod.rs` goes back to carrying real weight.

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

**4. Does WebKit dispatch `button: 3` for a real mouse back button on macOS?**
Unverifiable from source. *Recommendation:* test on hardware before writing the
handler; the fallback needs the boundary crate and is therefore a different
decision.

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

## What would falsify this document

Ranked by how likely they are to be wrong and how much rests on them.

**The `dx` manifest assumption.** Stage 0 is the first commit and everything
downstream cites it. If `dx` requires `--features desktop` literally, the
cleanest version of the reorganisation does not exist and the fallback is
strictly weaker. This is the single most likely thing here to be wrong, because
it is the only load-bearing claim not read from source.

**"Calling a safe `fn` in a dependency does not violate `unsafe_code =
"forbid"`."** Everything in the vibrancy, window-level and Prevent Sleep rows
rests on it. It follows from `forbid` being a lint on `unsafe` *syntax* in the
crate that sets it, and from `extern_methods!` expanding inside
`objc2-app-kit`'s crate rather than ours — but this pass ran no build, and a
single `cargo check` with one `NSWindow::setLevel` call would settle it. Run
that before scheduling anything in those rows.

**That `objc2-app-kit`'s enabled feature set stays where it is.** The 44-feature
count is read from `cargo tree` against today's lockfile. A dependency bump that
enables `NSVisualEffectView` upstream would make one row cheaper; a bump that
changes `extern_methods!`'s safety generation would make several rows
impossible. Re-read before, not after.

**The mouse-back-button DOM claim.** Explicitly unverified against hardware, and
called out as such in the row.

**The notifications row.** The only capability in this document whose crate
could not be checked against vendored source — `notify-rust` is not under
`~/.cargo/registry/src/`. No API is cited for it and none should be trusted
until it is.

**"Nothing in the missing 92 is streaming."** Read from the generated types and
the fixture, and true of the contract at commit `3810898a`. It would stop being
true the day goose adds a second agent→client notification, and the fixture is
where that would show up.

**The server-side assumptions.** The working-directory picker and the
`.goosehints` editor both assume the `developer` extension is enabled with
`unprefixed_tools` on a machine this client does not control. `goose_request`
degrades a `-32601` into `AcpError::Unsupported` and caches it, so the failure is
soft — which is exactly the problem: the feature would degrade silently rather
than loudly. It was also not checked whether the goose build we target gates any
of the newer namespaces (`local-inference`, `apps`, `dictation`) behind a
server flag.

**The velocity baseline.** 122 commits in six days is measured, but it is
measured on a phone app built greenfield with no prior art to stay compatible
with. Capability work on an existing shell has a different shape: more of it is
merging into files that other branches are also editing, which is precisely the
serialising cost Stage 5 is about. If the sequencing above turns out optimistic,
this is where it will turn out optimistic — not in the wrapper count.

**The claim that a crate split is unnecessary.** It rests on the two
`include_str!` cross-layer tests and on `dx` cooperating. If the desktop's
information architecture genuinely diverges — multi-window, a launcher, a
menu-driven command surface with no phone analogue — then `views/` starts
carrying props no phone ever passes, and one shared layer becomes a fork wearing
the shape of a merge. Nothing here detects that state. It would have to be
noticed by someone reading `views/` and asking how many of its props
`Shell::Mobile` ever sets.
