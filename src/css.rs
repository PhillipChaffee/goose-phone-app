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
    // scheduler — PR 5 replaces this line
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
