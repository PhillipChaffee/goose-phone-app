//! The stylesheet, assembled at compile time.
//!
//! Embedded rather than served, so styling works identically under
//! `cargo run`, `dx serve` and a mobile bundle — there is no file next to the
//! binary on a phone.
//!
//! It is a `concat!` rather than one file because `assets/main.css` is one
//! file five branches would all be appending to at once. A feature that needs
//! rules of its own brings `assets/features/<feature>.css` and replaces its
//! own placeholder line below; nothing else in this list moves. The directory
//! is part of the contract: `docs/audit.js` links `main.css` plus everything
//! in `assets/features/`, so a stylesheet parked anywhere else audits as
//! markup with no rules against it. `assets/platform/` is the one exception,
//! and it is one on purpose — see [`PLATFORM`].

/// Every stylesheet in the app, in cascade order. `main.css` is the design
/// system — tokens, chrome, the shared components — so it comes first and
/// everything after it is a feature's own additions.
pub(crate) const STYLES: &str = concat!(
    include_str!("../assets/main.css"),
    include_str!("../assets/features/recipes.css"),
    include_str!("../assets/features/skills.css"),
    include_str!("../assets/features/scheduler.css"),
    include_str!("../assets/features/extensions.css"),
    // session history (PR 7): the search box above the chats list, and the
    // rename sheet's field
    include_str!("../assets/features/session-history.css"),
);

/// The one platform-conditional sheet, emitted after [`STYLES`].
///
/// iOS only, because it opts the web view into Dynamic Type through
/// `font: -apple-system-body` — a keyword macOS's WKWebView also parses, and
/// resolves to a flat 13px, which would shrink `dx serve --desktop` by about
/// 19%. Android and the desktop build get an empty string and therefore the
/// browser's own 16px root, which is exactly what `assets/main.css` was
/// authored against. The `cargo check --target aarch64-apple-ios` gate is what
/// keeps the iOS arm compiling.
///
/// A second const rather than another line in the `concat!` above because
/// `concat!` takes literals only, so a `cfg` cannot go inside it.
///
/// Deliberately NOT in `assets/features/`: `docs/audit.js` links that whole
/// directory, and this sheet must not reach it. The audit runs in Chromium,
/// which cannot parse the keyword at all — it simulates Dynamic Type by
/// setting the root font-size in px, at four sizes, which is the same claim in
/// the only form that browser can hear.
#[cfg(target_os = "ios")]
pub(crate) const PLATFORM: &str = include_str!("../assets/platform/ios.css");

#[cfg(not(target_os = "ios"))]
pub(crate) const PLATFORM: &str = "";

/// The desktop shell's sheet, emitted after [`PLATFORM`].
///
/// Everything the pinned nav, the panes and the pointer tier need — hover,
/// focus rings, inline row actions — and, in two `@media (min-width: …)`
/// rules, the whole of the width story: three columns become two and then
/// one as the window narrows.
///
/// That those rules live HERE is the reason no Rust in this app observes a
/// window resize. This string is only concatenated into a desktop binary, so
/// a width breakpoint inside it is physically unreachable from a phone — which
/// means pane count needs no bridge, no `document::eval` and no extension to
/// `src/viewport.rs`, and the rule that every listened-to event costs a
/// synchronous XHR is honoured by having nothing to listen to.
///
/// A separate const rather than another line in the `concat!` above for
/// [`PLATFORM`]'s reason: `concat!` takes literals only, so a `cfg` cannot go
/// inside it.
///
/// Deliberately NOT in `assets/features/`, also for [`PLATFORM`]'s reason:
/// `docs/audit.js` links that whole directory into 402x874 PHONE frames. A
/// desktop sheet there would restyle every audited phone state into a layout
/// no phone binary can produce, and the audit would then report on it.
///
/// `target_os` and not `feature`, following [`PLATFORM`] again: the
/// `cargo check --target aarch64-apple-ios` gate runs with default features,
/// which is `desktop`.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) const SHELL: &str = "";

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) const SHELL: &str = include_str!("../assets/desktop.css");

// The phone's stylesheet is exactly what it was, and this is the whole of that
// claim in one line: `STYLES` and `PLATFORM` are untouched by the desktop
// branch, so if `SHELL` is empty on a phone then the string the web view is
// handed is byte-for-byte the one it was handed before. Checked at COMPILE
// time on the targets it is about, which is the only place it can be checked —
// `cargo test` runs the desktop arm.
#[cfg(any(target_os = "ios", target_os = "android"))]
const _: () = assert!(SHELL.is_empty());
