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

/// The platform-conditional sheet, emitted after [`STYLES`]. One per
/// platform that has one, and never more than one in a binary.
///
/// iOS opts the web view into Dynamic Type through
/// `font: -apple-system-body` — a keyword macOS's WKWebView also parses, and
/// resolves to a flat 13px, which would shrink `dx serve --desktop` by about
/// 19%. Android and the desktop build get an empty string and therefore the
/// browser's own 16px root, which is exactly what `assets/main.css` was
/// authored against. The `cargo check --target aarch64-apple-ios` gate is what
/// keeps the iOS arm compiling.
///
/// macOS carries the window-chrome reservation instead, because `src/main.rs`
/// hides the titlebar on macOS and nowhere else. [`SHELL`] defaults
/// `--chrome-h` and `--traffic-w` to zero and this is the only thing that
/// raises them — so a Windows or Linux desktop build, which keeps its native
/// frame, does not also hold 52 points empty for traffic lights it does not
/// have. The two sheets are mutually exclusive by construction: nothing is
/// both iOS and macOS.
///
/// A second const rather than another line in the `concat!` above because
/// `concat!` takes literals only, so a `cfg` cannot go inside it.
///
/// Deliberately NOT in `assets/features/`: `docs/audit.js` links that whole
/// directory, and neither sheet must reach it. The audit runs in Chromium,
/// which cannot parse `-apple-system-body` at all — it simulates Dynamic Type
/// by setting the root font-size in px, at four sizes, which is the same claim
/// in the only form that browser can hear; and the macOS sheet describes a
/// window a phone frame does not have.
#[cfg(target_os = "ios")]
pub(crate) const PLATFORM: &str = include_str!("../assets/platform/ios.css");

#[cfg(target_os = "macos")]
pub(crate) const PLATFORM: &str = include_str!("../assets/platform/macos.css");

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
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
/// Measured rather than feared: copying the desktop sheets into
/// `assets/features/` makes a Clean tree audit at 716 findings.
///
/// `target_os` and not `feature`, following [`PLATFORM`] again: the
/// `cargo check --target aarch64-apple-ios` gate runs with default features,
/// which is `desktop`.
///
/// A `concat!` of `assets/desktop/` rather than one file, for [`STYLES`]'s
/// reason at a larger scale: one 4278-line sheet is one file every branch of a
/// wide redesign appends to at once, and parallel appends have already left it
/// carrying the same declaration twice. A region that needs rules of its own
/// brings its own `assets/desktop/<nn>-<region>.css` and nothing else in this
/// list moves. **The filename prefixes ARE the cascade order**: they are
/// zero-padded so that Rust's `sort`, JS's `Array.prototype.sort` and Python's
/// `sorted` all produce this same sequence, because the two other places that
/// link these sheets — `docs/audit.js` and `scripts/capture-gallery.py` — read
/// the directory rather than restating the list, and a sheet that sorts to a
/// different slot there is a different cascade from the one that ships.
/// Renumber nothing without renumbering everything after it.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) const SHELL: &str = "";

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) const SHELL: &str = concat!(
    include_str!("../assets/desktop/00-tokens.css"),
    include_str!("../assets/desktop/10-sidebar-frame.css"),
    include_str!("../assets/desktop/20-plane-switch.css"),
    include_str!("../assets/desktop/30-sidebar-list.css"),
    include_str!("../assets/desktop/40-home-chat.css"),
    include_str!("../assets/desktop/50-band.css"),
    include_str!("../assets/desktop/55-panes.css"),
    include_str!("../assets/desktop/60-sidebar-extra.css"),
    include_str!("../assets/desktop/65-responsive.css"),
    include_str!("../assets/desktop/70-overrides.css"),
    include_str!("../assets/desktop/80-measure.css"),
    include_str!("../assets/desktop/90-inspector.css"),
    include_str!("../assets/desktop/95-transcript.css"),
    include_str!("../assets/desktop/97-home-code.css"),
    include_str!("../assets/desktop/98-home-sched.css"),
);

/// [`SHELL`]'s parts, named, in the same order — so a test that finds a fault
/// in the concatenated string can say WHICH file carries it. A whole-sheet
/// assertion that reports "unbalanced braces" against 165k characters names
/// nothing; the same assertion over this array names one 90-line file.
///
/// `cfg(test)` because only tests read it, and a non-test const nothing calls
/// is `dead_code` — which CI sets `-D warnings` against.
#[cfg(test)]
pub(crate) const SHELL_PARTS: &[(&str, &str)] = &[
    (
        "00-tokens.css",
        include_str!("../assets/desktop/00-tokens.css"),
    ),
    (
        "10-sidebar-frame.css",
        include_str!("../assets/desktop/10-sidebar-frame.css"),
    ),
    (
        "20-plane-switch.css",
        include_str!("../assets/desktop/20-plane-switch.css"),
    ),
    (
        "30-sidebar-list.css",
        include_str!("../assets/desktop/30-sidebar-list.css"),
    ),
    (
        "40-home-chat.css",
        include_str!("../assets/desktop/40-home-chat.css"),
    ),
    ("50-band.css", include_str!("../assets/desktop/50-band.css")),
    (
        "55-panes.css",
        include_str!("../assets/desktop/55-panes.css"),
    ),
    (
        "60-sidebar-extra.css",
        include_str!("../assets/desktop/60-sidebar-extra.css"),
    ),
    (
        "65-responsive.css",
        include_str!("../assets/desktop/65-responsive.css"),
    ),
    (
        "70-overrides.css",
        include_str!("../assets/desktop/70-overrides.css"),
    ),
    (
        "80-measure.css",
        include_str!("../assets/desktop/80-measure.css"),
    ),
    (
        "90-inspector.css",
        include_str!("../assets/desktop/90-inspector.css"),
    ),
    (
        "95-transcript.css",
        include_str!("../assets/desktop/95-transcript.css"),
    ),
    (
        "97-home-code.css",
        include_str!("../assets/desktop/97-home-code.css"),
    ),
    (
        "98-home-sched.css",
        include_str!("../assets/desktop/98-home-sched.css"),
    ),
];

// The phone's stylesheet is exactly what it was, and this is the whole of that
// claim in one line: `STYLES` and `PLATFORM` are untouched by the desktop
// branch, so if `SHELL` is empty on a phone then the string the web view is
// handed is byte-for-byte the one it was handed before. Checked at COMPILE
// time on the targets it is about, which is the only place it can be checked —
// `cargo test` runs the desktop arm.
#[cfg(any(target_os = "ios", target_os = "android"))]
const _: () = assert!(SHELL.is_empty());

#[cfg(test)]
mod tests {
    /// THE THREE LISTS ARE ONE LIST, and this is the only thing that says so.
    ///
    /// `SHELL` names its fifteen region files by hand, because `concat!` and
    /// `include_str!` cannot walk a directory. `docs/audit.js` and
    /// `scripts/capture-gallery.py` both `readdir` the same directory and sort
    /// it. So the app's cascade is written down and the two review tools'
    /// cascades are computed, and nothing in either language can notice when
    /// they stop being the same one.
    ///
    /// The failure that makes this worth a test is not a typo. It is the next
    /// region file: add `45-something.css` to `assets/desktop/` and the audit
    /// links it, the gallery links it, both render a window that looks right —
    /// and the shipped binary never sees a byte of it, because nobody added the
    /// `include_str!`. A Clean audit over rules the app does not emit is the
    /// exact failure mode `docs/design.md` records the gallery already having
    /// had once.
    ///
    /// Both directions, which is why it is an equality and not a `contains`:
    /// a file on disk missing from `SHELL` is the case above, and a name in
    /// `SHELL` with no file behind it cannot compile at all — but a name in
    /// `SHELL` in the wrong ORDER compiles, ships a different cascade from the
    /// one the audit measures, and is caught here by the sort.
    #[test]
    fn the_shell_concatenates_every_region_file_in_sorted_order() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/desktop");
        let read = std::fs::read_dir(&dir);
        assert!(
            read.is_ok(),
            "cannot read {} — it is the desktop shell's whole stylesheet",
            dir.display()
        );
        // `extension()` rather than `ends_with(".css")`: the two review tools
        // match the suffix exactly, and a case-insensitive comparison here
        // would accept a name neither of them links.
        let mut on_disk: Vec<String> = read
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "css"))
            .filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
            .collect();
        on_disk.sort();

        let listed: Vec<&str> = super::SHELL_PARTS.iter().map(|&(name, _)| name).collect();
        assert_eq!(
            listed, on_disk,
            "`SHELL` and `assets/desktop/` disagree. The filename sort IS the \
             cascade order — `docs/audit.js` and `scripts/capture-gallery.py` \
             both build their link lists by sorting this directory — so a file \
             the `concat!` skips is styled in every review surface and in no \
             shipped binary, and a file out of order ships a cascade the audit \
             never measures."
        );
    }

    /// [`super::SHELL_PARTS`] is [`super::SHELL`], not a second copy of it that
    /// drifts. Everything the tests assert about the parts is only a claim
    /// about the app because of this line.
    #[test]
    fn the_parts_reassemble_the_sheet_the_binary_embeds() {
        let joined: String = super::SHELL_PARTS.iter().map(|&(_, body)| body).collect();
        assert_eq!(
            joined,
            super::SHELL,
            "the named parts do not concatenate to `SHELL`, so a test that \
             walks them is measuring a sheet the app does not embed"
        );
    }
}
