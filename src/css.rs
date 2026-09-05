//! The stylesheet, assembled at compile time.
//!
//! Embedded rather than served, so styling works identically under
//! `cargo run`, `dx serve` and a mobile bundle — there is no file next to the
//! binary on a phone.
//!
//! It is a `concat!` rather than one file because `assets/shared.css` is one
//! file five branches would all be appending to at once. A feature that needs
//! rules of its own brings `assets/features/<feature>.css` and replaces its
//! own placeholder line below; nothing else in this list moves. The directory
//! is part of the contract: `docs/audit.js` links `shared.css` plus everything
//! in `assets/features/`, so a stylesheet parked anywhere else audits as
//! markup with no rules against it. `assets/platform/` is the one exception,
//! and it is one on purpose — see [`PLATFORM`].

/// Every stylesheet in the app, in cascade order. `shared.css` is the design
/// system — tokens, chrome, the shared components — so it comes first and
/// everything after it is a feature's own additions.
///
/// It is named for its reach, not for a shell: this const is emitted by
/// `src/app.rs` on the phone AND on the desktop, so a rule in `shared.css` is
/// a rule in both. Its own header names the three phone-only exceptions and
/// the rule for changing anything else — an override in `assets/desktop/`,
/// not an edit there.
pub(crate) const STYLES: &str = concat!(
    include_str!("../assets/shared.css"),
    include_str!("../assets/features/recipes.css"),
    include_str!("../assets/features/skills.css"),
    include_str!("../assets/features/scheduler.css"),
    include_str!("../assets/features/extensions.css"),
    // session history (PR 7): the search box above the chats list, and the
    // rename sheet's field
    include_str!("../assets/features/session-history.css"),
);

/// `assets/shared.css` ALONE, which [`STYLES`] is not — that is this sheet
/// plus the five in `assets/features/`.
///
/// `src/inherit.rs` needs the shared design system by itself, because the
/// question it asks is about the sheet BOTH shells link and not about a
/// feature's own additions. Named here rather than `include_str!`'d there so
/// this module stays the one place a stylesheet path is written down, and held
/// to the front of `STYLES` by `the_shared_sheet_is_the_front_of_the_sheet_that_ships`
/// so it cannot become a second copy that drifts.
///
/// `cfg(test)` because only that gate reads it, and a non-test const nothing
/// calls is `dead_code` — which CI sets `-D warnings` against.
#[cfg(test)]
pub(crate) const SHARED: &str = include_str!("../assets/shared.css");

/// `css` with every `/* ... */` taken out.
///
/// The block `the_two_dark_bodies_are_one_palette` reads has a `}` INSIDE a
/// comment — it quotes the mockups' own rule,
/// `mask-image:linear-gradient(...)}` — so a brace scan that counts it ends the
/// block one declaration in and finds neither number. Stripping first is what
/// makes the scan honest, and `src/inherit.rs`'s selector walk needs it for the
/// same reason at larger scale: `assets/shared.css` carries 30 banner comments
/// naming the classes below them, and a walk that read those would report every
/// name in the prose as a rule.
///
/// Module level rather than inside `mod tests` so both readers share one
/// answer; `cfg(test)` for [`SHARED`]'s reason.
#[cfg(test)]
pub(crate) fn without_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(open) = rest.find("/*") {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open + 2..).unwrap_or_default();
        rest = after
            .find("*/")
            .and_then(|close| after.get(close + 2..))
            .unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The platform-conditional sheet, emitted after [`STYLES`]. One per
/// platform that has one, and never more than one in a binary.
///
/// iOS opts the web view into Dynamic Type through
/// `font: -apple-system-body` — a keyword macOS's WKWebView also parses, and
/// resolves to a flat 13px, which would shrink `dx serve --desktop` by about
/// 19%. Android and the desktop build get an empty string and therefore the
/// browser's own 16px root, which is exactly what `assets/shared.css` was
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
/// list moves. It was fifteen files and is thirteen: `60-sidebar-extra.css`
/// and `98-home-sched.css` were named for where their rules SAT rather than
/// for a region — the split cut the old sheet in filename order, so rules
/// appended late to it landed late — and #154 and #155 put them back beside
/// what they style, in `30-sidebar-list.css` and `40-home-chat.css`. Moving a
/// rule between these files moves it in the cascade, which is why that was two
/// commits after the split and not part of it, and why each carries the
/// computed-style diff that proved nothing moved.
/// **The filename prefixes ARE the cascade order**: they are
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
    include_str!("../assets/desktop/65-responsive.css"),
    include_str!("../assets/desktop/70-overrides.css"),
    include_str!("../assets/desktop/80-measure.css"),
    include_str!("../assets/desktop/90-inspector.css"),
    include_str!("../assets/desktop/95-transcript.css"),
    include_str!("../assets/desktop/97-home-code.css"),
);

/// [`SHELL`]'s parts, named, in the same order — so a test that finds a fault
/// in the concatenated string can say WHICH file carries it. A whole-sheet
/// assertion that reports "unbalanced braces" against 165k characters names
/// nothing; the same assertion over this array names one 124-line file.
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
    /// `SHELL` names its thirteen region files by hand, because `concat!` and
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

    use super::without_comments;

    /// The body of the first `{ … }` that follows `opens`, comments already
    /// gone, as a list of `(property, value)` in source order.
    ///
    /// Innermost only, and that is what makes it safe on a sheet that nests:
    /// the needles below all name a selector whose block holds declarations and
    /// no nested rule, so stopping at the first `}` cannot truncate one.
    fn declarations(css: &str, opens: &str) -> Vec<(String, String)> {
        let after = css.split_once(opens).map(|(_, rest)| rest);
        assert!(
            after.is_some(),
            "no `{opens}` in the sheet — the block this test reads is not there \
             to be read, so it is asserting nothing"
        );
        let body = after
            .unwrap_or_default()
            .split_once('}')
            .map_or("", |(body, _)| body);
        body.split(';').filter_map(split_declaration).collect()
    }

    /// `prop: value` split in two, or `None` for anything that is not a
    /// declaration — a selector fragment, a media condition, trailing space.
    ///
    /// The property test is deliberately narrow: a CSS property is ASCII
    /// letters, digits, `-` and `_` and nothing else, so a stray `@media
    /// (max-width: 900px)` or a `url(data:…)` cannot be mistaken for one.
    fn split_declaration(decl: &str) -> Option<(String, String)> {
        let (prop, value) = decl.split_once(':')?;
        let prop = prop.trim();
        if prop.is_empty()
            || !prop
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return None;
        }
        Some((prop.to_owned(), value.split_whitespace().collect()))
    }

    /// THE PARSER'S ONE GUARD, EXERCISED — because the sheet does not exercise
    /// it, and a scan that quietly stops recognising things is a scan that
    /// quietly stops finding them.
    ///
    /// The two tests below walk thirteen files looking for a repeated property,
    /// and everything they currently meet is a real declaration, so the reject
    /// arm never runs against the shipped sheet. It exists for what those files
    /// are one rule away from holding: `@media (max-width: 900px)` has a colon
    /// and is not a declaration, and neither is the `url(data:…)` fragment a
    /// data URI leaves behind when a value carrying its own `;` is split on it.
    /// Read as a declaration, either would put a nonsense property name into
    /// the seen list and could report a duplicate that is not there — a false
    /// failure in a test whose whole job is to be believed.
    #[test]
    fn only_a_real_declaration_is_read_as_one() {
        assert_eq!(
            split_declaration("  --accent :  #13bbaf "),
            Some(("--accent".to_owned(), "#13bbaf".to_owned())),
            "a declaration with slack around it is still a declaration, and the \
             value is compared after whitespace, so `var( --x )` and `var(--x)` \
             are one value"
        );
        assert_eq!(
            split_declaration("--text-code: min(var(--text-xs), 20px)"),
            Some((
                "--text-code".to_owned(),
                "min(var(--text-xs),20px)".to_owned()
            )),
            "a value with its own spaces must survive as one string, or every \
             calc() and clamp() in the sheet compares unequal to itself"
        );
        for not_a_declaration in [
            "@media (max-width: 900px)",
            "url(data:image/svg+xml,x)",
            ": #13bbaf",
            "--accent",
            "  ",
        ] {
            assert_eq!(
                split_declaration(not_a_declaration),
                None,
                "`{not_a_declaration}` is not a declaration and must not be read \
                 as one — a scan that accepts it can report a duplicate property \
                 that does not exist"
            );
        }
    }

    /// THE TWO DARK BODIES ARE ONE PALETTE, and until this test they were not.
    ///
    /// `00-tokens.css` writes the dark ramp twice, because the two selectors
    /// mean different things: `@media (prefers-color-scheme: dark)` is "the
    /// system says dark and the app has not overridden it", and
    /// `:root[data-theme="dark"]` is "the app says dark". The comment above
    /// them claimed a test held the two identical. There was none, for the
    /// whole life of the file, and they drifted to 20 declarations against 13.
    ///
    /// Four of the seven that went missing would have painted a LIGHT value on
    /// a dark window the first time anything set `data-theme`: `--tint-warn`
    /// #f8f6f0 and `--tint-live` #f1f7f7 as near-white slabs under white text
    /// (1.08:1), and the two inspector diff inks at 2.7:1. Nothing could see
    /// it — `docs/audit.js` switches themes with `emulateMedia`, never with the
    /// attribute, so the whole attribute arm is unmeasured by every rendered
    /// check in this repo and this test is the only thing that reads it.
    ///
    /// Order and value, not just the property set: two bodies that declare the
    /// same names in a different order are still one palette, but two that give
    /// `--accent` different values are exactly the bug, and a set comparison
    /// would pass them. Whitespace inside a value is normalised away so
    /// `var(--surface-card)` and `var( --surface-card )` are one value; nothing
    /// else is.
    ///
    /// THE TWO NEEDLES ARE `.app` AND NOT `.app > .shell`, and moving them was
    /// half the cost of #254. The palette had been declared on the shell, which
    /// is a SIBLING of `.modal-backdrop` under `.app`, so no dialog in a
    /// desktop window inherited any of it. Both bodies moved up one element;
    /// the two strings below are the only place in the repo that names them, so
    /// leaving either behind would have left this test reading nothing while
    /// still passing — `declarations` asserts its needle is present for exactly
    /// that reason.
    ///
    /// REPRODUCED: delete any one line from either body, or change one hex in
    /// one of them, and this fails naming the property and both values.
    #[test]
    fn the_two_dark_bodies_are_one_palette() {
        const FILE: &str = "00-tokens.css";
        let sheet = super::SHELL_PARTS
            .iter()
            .find(|&&(name, _)| name == FILE)
            .map(|&(_, body)| without_comments(body))
            .unwrap_or_default();
        assert!(
            !sheet.is_empty(),
            "no `{FILE}` in `SHELL_PARTS` — the desktop's token block has been \
             renamed, and this test now checks nothing"
        );

        let media = declarations(&sheet, ":root:not([data-theme=\"light\"]) .app {");
        let attribute = declarations(&sheet, ":root[data-theme=\"dark\"] .app {");

        assert_eq!(
            media.len(),
            attribute.len(),
            "the dark media body declares {} properties and the \
             `data-theme=\"dark\"` body declares {}. Whichever is short falls \
             back to the LIGHT declaration on `.app`, which is how a \
             near-white slab ends up behind white text on a dark window.\n\
             \n  media only:     {:?}\n  attribute only: {:?}",
            media.len(),
            attribute.len(),
            media
                .iter()
                .filter(|d| !attribute.contains(d))
                .collect::<Vec<_>>(),
            attribute
                .iter()
                .filter(|d| !media.contains(d))
                .collect::<Vec<_>>(),
        );
        for (from_media, from_attribute) in media.iter().zip(attribute.iter()) {
            assert_eq!(
                from_media, from_attribute,
                "the two dark bodies disagree: the media query says `{}: {}` \
                 and the attribute says `{}: {}`. They are one palette said \
                 twice, and a reader who forces dark on a light system gets \
                 whichever of the two is wrong.",
                from_media.0, from_media.1, from_attribute.0, from_attribute.1,
            );
        }
    }

    /// NOTHING DECLARES THE SAME PROPERTY TWICE IN ONE BLOCK.
    ///
    /// Not a style rule — a damage detector. `00-tokens.css` shipped
    /// `--insp-add` and `--insp-del` declared twice in the same body, the
    /// second pair mis-indented, which is what two branches appending to one
    /// 4278-line sheet leaves behind: git merges both hunks cleanly, the sheet
    /// still parses, the cascade still resolves, and the only trace is that one
    /// of the two values is now unreachable. Splitting the sheet into thirteen
    /// region files narrows the target; it does not remove it, because six
    /// lanes still write six files at once and five of the issues in this
    /// campaign were found to prescribe the same declaration from two places.
    ///
    /// Whole directory, not just the tokens: the same pass found `.insp-chip`
    /// and `.insp-key` each carrying one `background` from two different
    /// issues, and those land in `90-inspector.css`.
    ///
    /// Innermost blocks only, so an `@media` wrapper is walked into rather than
    /// treated as one giant block. A repeated property with a DIFFERENT value
    /// is the bug this is about; a repeated property with the same value is
    /// caught too, and is the same mistake with the damage still latent.
    ///
    /// REPRODUCED: restore either duplicated line in `00-tokens.css`'s dark
    /// media body and this fails naming the file and the property.
    #[test]
    fn no_block_declares_the_same_property_twice() {
        for &(name, raw) in super::SHELL_PARTS {
            let sheet = without_comments(raw);
            let mut rest = sheet.as_str();
            while let Some(open) = rest.find('{') {
                let after = rest.get(open + 1..).unwrap_or_default();
                let end = after.find(['{', '}']).unwrap_or(after.len());
                // A `{` here means a nested rule, so this is not the innermost
                // block and its declarations belong to the blocks inside it.
                if after.as_bytes().get(end) == Some(&b'}') {
                    let mut seen: Vec<String> = Vec::new();
                    for (prop, value) in after
                        .get(..end)
                        .unwrap_or_default()
                        .split(';')
                        .filter_map(split_declaration)
                    {
                        assert!(
                            !seen.contains(&prop),
                            "{name} declares `{prop}` twice in one block, the \
                             second time as `{prop}: {value}`. Only one of the \
                             two is reachable; the other is a change somebody \
                             made and does not have. This is what a parallel \
                             append leaves behind, and nothing else in the repo \
                             can see it — the sheet parses either way."
                        );
                        seen.push(prop);
                    }
                }
                rest = after.get(end..).unwrap_or_default();
            }
        }
    }

    /// EVERY DESKTOP TOKEN THIS PASS DECIDED, PINNED BY NAME AND VALUE.
    ///
    /// The one defence against the failure mode this campaign is built to
    /// avoid. Six lanes write six region files; a lane that changes a colour
    /// here changes every string in the window, and git merges a one-hex edit
    /// without a marker, `cargo` does not read CSS, and `docs/audit.js` only
    /// asks whether the result still clears 4.5:1 — which a wrong value
    /// usually does. So each of these is a decision with a measurement behind
    /// it in `00-tokens.css`, and moving one has to mean editing this list.
    ///
    /// `color` is in the table and is not a token, deliberately: it is the
    /// single line that makes the ink remap arrive at all. `assets/shared.css`
    /// has `body { color: var(--text-primary) }` and that `var()` substitutes
    /// at `body`, on `:root`'s value — so without this restatement on `.app`,
    /// remapping `--text-primary` changes the colour of almost nothing and
    /// every gate in the repo passes over a no-op.
    ///
    /// REPRODUCED: change any one value in `00-tokens.css` and this fails
    /// naming the property, the value it found and the value it expected.
    #[test]
    fn the_desktop_tokens_are_the_values_that_were_measured() {
        const FILE: &str = "00-tokens.css";
        let sheet = super::SHELL_PARTS
            .iter()
            .find(|&&(name, _)| name == FILE)
            .map(|&(_, body)| without_comments(body))
            .unwrap_or_default();

        let base = declarations(&sheet, ".app {");
        let dark = declarations(&sheet, ":root[data-theme=\"dark\"] .app {");

        // The ink ladder and the line that delivers it; the status trio; the
        // faint rung, the soft hairline, the two inspector rungs and the light
        // ramp's fifth. Light first, then what dark overrides.
        for (block, block_name, pins) in [
            (
                &base,
                "the light `.app` block",
                [
                    ("color", "var(--text-primary)"),
                    ("--ink-faint", "var(--text-secondary)"),
                    ("--insp-detail", "var(--text-primary)"),
                    ("--accent-dim", "var(--accent-fill)"),
                    ("--shell-line-soft", "var(--border-primary)"),
                    ("--card-on-panel", "#ffffff"),
                    ("--text-turn", "0.84375rem"),
                    ("--icon-md", "clamp(13px,0.8125rem,16px)"),
                ]
                .as_slice(),
            ),
            (
                &dark,
                "the dark block",
                [
                    ("--text-primary", "#e9ecef"),
                    ("--text-secondary", "#98a1ac"),
                    // NOT `--text-secondary`'s value, and the difference is the
                    // whole point of the rung. Pinning both here is what would
                    // have caught the first cut, where the two were one colour
                    // and every rule spending the faint rung was a silent no-op.
                    ("--ink-faint", "#828b96"),
                    ("--text-warning", "#f0b429"),
                    ("--bg-warning", "#f0b429"),
                    ("--text-success", "#68d391"),
                    ("--bg-success", "#68d391"),
                    ("--text-danger", "#f0736f"),
                    ("--bg-danger", "#f0736f"),
                    ("--shell-line-soft", "#242830"),
                    ("--accent-dim", "#0e7a73"),
                    ("--insp-detail", "#c3cad2"),
                    ("--card-on-panel", "var(--surface-card)"),
                ]
                .as_slice(),
            ),
        ] {
            for &(prop, want) in pins {
                let found = block
                    .iter()
                    .find(|(name, _)| name == prop)
                    .map(|(_, value)| value.as_str());
                assert_eq!(
                    found,
                    Some(want),
                    "{block_name} of {FILE} gives `{prop}` as {found:?} and the \
                     measurement it was chosen by says {want}. If the value is \
                     meant to move, move it here in the same commit — this list \
                     is the only thing in the repo that can tell a considered \
                     change from a merge that silently kept one lane's answer."
                );
            }
        }

        // THE FAINT RUNG IS A RUNG, not a second name for the dim one.
        //
        // The pins above cannot catch this on their own: a change that moved
        // BOTH tokens to one value would update both pins and pass. That is
        // not hypothetical — it is what shipped first. `--text-secondary`
        // landed on #98a1ac and `--ink-faint` was declared
        // `var(--text-secondary)`, so the two were one colour, the mockups'
        // four-tier ladder rendered as two, and every rule spending the faint
        // rung was a no-op that `docs/audit.js` reported as Clean, because a
        // string in the right colour is in the right colour whichever token
        // named it.
        //
        // Only dark. Light aliases them on purpose — #828b96 measures 3.45:1
        // on white, so a light window needs a DARKER faint rung, and nothing in
        // the mockups says what it should be because they have no light mode.
        let value = |block: &[(String, String)], prop: &str| {
            block
                .iter()
                .find(|(name, _)| name == prop)
                .map(|(_, v)| v.clone())
        };
        assert_ne!(
            value(&dark, "--ink-faint"),
            value(&dark, "--text-secondary"),
            "the dark block of {FILE} gives `--ink-faint` and `--text-secondary` \
             the same value, so the third ink tier does not exist and every rule \
             that spends it paints the rung above it. Give the faint rung its \
             own value or delete it — a token whose only effect is to be \
             renamed is worse than no token, because the rules that read it \
             look considered."
        );

        // AND SOMETHING HAS TO READ IT, which is the same failure one step
        // earlier and is the state this rung shipped in.
        //
        // The assertion above catches a faint rung that resolves to the dim
        // one. It cannot catch a faint rung that resolves to a real third
        // colour and is referenced by nothing: `--ink-faint` was declared in
        // both themes, pinned by the table above, argued for in fourteen lines
        // of comment naming its eight intended consumers — and read by zero
        // rules in `assets/desktop/` for the whole life of the token. Every
        // gate in this repo passed, because a rung nothing spends renders
        // exactly like a rung that does not exist.
        //
        // Asked of the WHOLE directory rather than of a named file, because
        // #118's spend list runs across six region files owned by six lanes and
        // this check has no business saying which of them arrives first. One
        // consumer is the claim: the rung is in use.
        assert!(
            super::SHELL_PARTS
                .iter()
                .any(|&(_, body)| without_comments(body).contains("var(--ink-faint)")),
            "`--ink-faint` is declared in {FILE} and no rule in assets/desktop/ \
             reads it, so the third ink tier is a value nothing paints. Spend it \
             or delete it: a declared rung with no consumers is indistinguishable \
             from an absent one in every rendered check this repo has."
        );
    }

    /// THE FADE AND THE PADDING ARE ONE NUMBER, and for as long as there was no
    /// test the comment saying so was the only thing saying so.
    ///
    /// `.nav-sessions` dissolves its last rows into the footer with a mask, and
    /// the mockups do that with a percentage because their list does not
    /// scroll. This one scrolls, so a percentage would park the fade on top of
    /// the last row's own text at scroll-end — permanently, and invisibly to
    /// `docs/audit.js`, whose contrast walk reads `color` and `opacity` and a
    /// mask sets neither. The fix is that the gradient is a LENGTH and the
    /// scroller pads by that same length, so the ramp always lands on padding.
    ///
    /// Which makes the two numbers one number, held apart in three
    /// declarations — `padding-bottom` and the mask twice, because `WKWebView`
    /// needs the `-webkit-` copy. Change one and the layout is still legal,
    /// still renders, and quietly fades a row again. Nothing else in the
    /// repo can see that: it is one sheet's arithmetic, not a rendered fault.
    ///
    /// The comment beside the rule claimed this test existed. It did not.
    #[test]
    fn the_sidebar_fade_lands_on_the_padding_it_is_measured_from() {
        const FILE: &str = "30-sidebar-list.css";
        // The space and the brace are what keep this off `.nav-sessions-empty`,
        // which is a real selector four rules further down.
        const SELECTOR: &str = ".nav-sessions {";

        let sheet = super::SHELL_PARTS
            .iter()
            .find(|&&(name, _)| name == FILE)
            .map(|&(_, body)| without_comments(body))
            .unwrap_or_default();
        assert!(
            !sheet.is_empty(),
            "no `{FILE}` in `SHELL_PARTS` — the sidebar's region file has been \
             renamed or split, and this test now checks nothing"
        );

        let block = sheet
            .find(SELECTOR)
            .and_then(|at| sheet.get(at + SELECTOR.len()..))
            .and_then(|rest| rest.split('}').next())
            .unwrap_or_default();
        assert!(
            !block.is_empty(),
            "no `{SELECTOR}` rule in {FILE}: the scroller this is about is \
             styled somewhere else now, or under another name"
        );

        let padding = block
            .find("padding-bottom:")
            .and_then(|at| block.get(at + "padding-bottom:".len()..))
            .and_then(|rest| rest.split(';').next())
            .unwrap_or_default()
            .trim();
        let fades: Vec<&str> = block
            .match_indices("calc(100% - ")
            .filter_map(|(at, needle)| {
                block
                    .get(at + needle.len()..)
                    .and_then(|rest| rest.split(')').next())
            })
            .collect();

        assert_eq!(
            fades.len(),
            2,
            "`.nav-sessions` should carry the fade twice — `mask-image` and the \
             `-webkit-` copy WKWebView needs — and it carries {} `calc(100% - \
             ...)`. One of the two is gone, so the fade is on one engine only.",
            fades.len()
        );
        for fade in &fades {
            assert_eq!(
                *fade, padding,
                "the fade is {fade} and the scroller pads {padding}. They are \
                 one number in {FILE}: the ramp is a length precisely so it \
                 lands on the padding rather than on a row, and a mask that \
                 outruns the padding hides the last row's text at scroll-end \
                 with nothing to report it."
            );
        }
    }

    /// THE PANE HEADER DOES NOT OUTRANK THE SIDEBAR, because for a while it
    /// did and that made the app's only cross-plane control unclickable.
    ///
    /// `assets/shared.css` gives `.topbar` `position: absolute` and
    /// `z-index: var(--z-chrome)` so it can float over the phone's scroller.
    /// `70-overrides.css` puts it back in flow for the desktop — and taking
    /// the positioning away does NOT take the z-index with it. `.pane` is
    /// `display: flex`, so `.topbar` is a flex item, and CSS Flexbox §4.3
    /// keeps `z-index` applying to a flex item at `position: static`. A 20
    /// therefore lands in the root stacking context and beats the overlay
    /// sidebar's 3.
    ///
    /// Measured before the fix, `desktop-chat` at 728x860: the bar painted
    /// over all 38px of `.plane-switch` and `elementFromPoint` at the switch's
    /// centre returned `header.topbar`. `src/shell/desktop/mod.rs`'s plane
    /// switch is the only cross-plane control there is — no chord, and the
    /// badge is a span — so on the Code half there was no way back to Chat.
    ///
    /// Pinned as TEXT rather than as a rendered ratio because the failure is a
    /// missing declaration, not a wrong value: delete the line and the sheet
    /// still parses, the audit still reports Clean, and only the hit test
    /// changes. `docs/audit.js` has no occlusion check — that is the general
    /// fix and it is its own issue; this is the specific one.
    #[test]
    fn the_pane_header_claims_no_stacking_order_of_its_own() {
        const FILE: &str = "70-overrides.css";
        let sheet = super::SHELL_PARTS
            .iter()
            .find(|&&(name, _)| name == FILE)
            .map(|&(_, body)| without_comments(body))
            .unwrap_or_default();

        let topbar = declarations(&sheet, ".topbar {");
        let z = topbar
            .iter()
            .find(|(prop, _)| prop == "z-index")
            .map(|(_, value)| value.as_str());

        assert_eq!(
            z,
            Some("auto"),
            "{FILE}'s `.topbar` gives `z-index` as {z:?}. It must say `auto`: \
             the rule takes the phone's `position: absolute` away, shared.css's \
             `z-index: var(--z-chrome)` survives that on a flex item, and 20 \
             paints this bar over the overlay sidebar — including over the \
             plane switch, which is the only way to change halves."
        );
    }

    /// EVERY COMMENT CLOSES EXACTLY ONCE, because a sheet where one does not
    /// still parses and every other gate still passes.
    ///
    /// This campaign writes its measurements into CSS comments — contrast
    /// ratios, box dimensions, the argument for a value — and it writes them in
    /// markdown. Markdown emphasis is `**`, and CSS closes a comment with `*/`.
    /// So a measurement written as `**450**/#c2c8cf` — bold, then a slash —
    /// spells `*/` inside its own second asterisk pair. The comment ends 25
    /// lines early, the rest of the paragraph becomes stray CSS, and the parser
    /// discards tokens until it finds a block to throw away, which is the next
    /// rule.
    ///
    /// That happened. `cargo fmt`, `cargo clippy`, `cargo test` and
    /// `node docs/audit.js both` were all green over a rule that no longer
    /// existed: the audit renders `docs/gallery-states.json`, the store had not
    /// moved, and nothing in the repo read a selector list.
    ///
    /// COUNTING is the whole check, and it is enough because it is not
    /// symmetric. The stray `*/` is an EXTRA close — the author's own `*/` is
    /// still down the page — so the file ends with more closes than opens. A
    /// rule-count oracle is not enough and was tried: the stray selector adds a
    /// rule while eating one, so the net can be zero.
    ///
    /// Known limitation, stated rather than papered over: `/*` inside a string
    /// or a `url()` would be counted. No sheet here has one, and if one ever
    /// does, this test is where to teach the exception.
    ///
    /// REPRODUCED: `**450**/#c2c8cf` in any of the thirteen files gives that
    /// file one open and two closes, and this fails naming it.
    #[test]
    fn every_comment_in_every_region_file_closes_exactly_once() {
        for &(name, body) in super::SHELL_PARTS {
            let opens = body.matches("/*").count();
            let closes = body.matches("*/").count();
            assert_eq!(
                opens, closes,
                "{name} opens {opens} comments and closes {closes}. An extra \
                 close ends a comment where its author did not, and everything \
                 after it — up to and including the next rule — stops being \
                 CSS. Look for `**` immediately before a `/`: markdown emphasis \
                 and a slash spell a comment terminator."
            );
        }
    }
}
