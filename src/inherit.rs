//! What the desktop INHERITS, and what nobody decided.
//!
//! `assets/shared.css` is the design system and `src/app.rs` emits it into both
//! shells, so every rule in it lands in a 1440pt window as surely as on a 402pt
//! one. `assets/desktop/` is where a rule is meant to land differently. Between
//! those two facts sits the failure this module is the instrument for: a class
//! the desktop renders, styled by the shared sheet, that no desktop rule has
//! ever said anything about. It is not styled wrongly. It is styled by a
//! decision taken for a phone, and nobody has looked.
//!
//! MEASURED AT f386ebb, which is why this exists rather than a note in a
//! design doc: the shared sheet styles 178 leaf classes, the desktop can render
//! 163 of them, and `assets/desktop/` decides 56. **107 have no desktop
//! decision at all.** Eight of the defects found in one week of real-server
//! testing live in that set — `.action-row` (#201), `.scroll-bottom` (#209),
//! `.tool-icon` (#211), `.session-tile` (#63) are four of them — and every one
//! was found by a human hand-injecting markup into a captured state, because
//! nothing in the repo could ask the question.
//!
//! Nothing could ask it because nothing looked in the right place.
//! `every_class_the_desktop_shell_renders_is_in_the_captured_store`
//! (`src/shell/desktop/mod.rs`) scans four files — the shell's own — and
//! `src/views/` is not among them. 102 of the 107 are rendered from
//! `src/views/`. That is the whole of why `.action-row` was invisible, and it
//! is why the file list below is checked against the directory rather than
//! written once and trusted.
//!
//! # The three sets
//!
//! **styled** — the leaf class of every selector in `assets/shared.css`.
//!
//! **reachable** — every class name written as a `class:` string literal in any
//! `src/**/*.rs` that writes one, minus `src/shell/mobile.rs` (the phone shell,
//! which no desktop binary compiles) and minus every item annotated
//! `#[cfg(any(target_os = "ios", target_os = "android"))]` or `#[cfg(test)]`.
//! The `cfg` arm is not decoration: without it `.drawer-group` reads as desktop
//! surface, and `src/shell/mod.rs`'s `render_group` — the only thing that
//! renders it — is compiled on phones alone.
//!
//! **decided** — the leaf class of every selector in `assets/desktop/`, read
//! through [`crate::css::SHELL_PARTS`], which two other tests already hold
//! equal to the string the binary embeds and to the directory on disk.
//!
//! The gap is `(styled ∩ reachable) − decided`.
//!
//! # What counts as a desktop decision
//!
//! **A rule whose SUBJECT carries the class** — the last compound of the
//! selector, the element the declarations actually land on. `.pane .topbar`
//! decides `.topbar` and says nothing about `.pane`; `.btn.primary` decides
//! both, because both are on the subject.
//!
//! The cheaper definition — "the class appears anywhere in a desktop selector"
//! — was tried and is WRONG, and the four names it differs on are the proof.
//! `80-measure.css` has `.pane-main .scroll:not(.diff)` and `95-transcript.css`
//! has `.pane-main .composer .send:not(.stop)`, whose own comment explains that
//! the `:not` is load-bearing precisely so the rule does NOT reach `.stop`. A
//! definition under which a rule that excludes a class counts as a decision
//! about it is a definition that goes green over the thing it was built to
//! find. The other two are `.insp-chip.off > i` and
//! `.nav-library.open > .icon:first-child`, where the subject is the child and
//! the class is context on its parent — and `.dot.off`, the offline connection
//! badge that #213 is about, would have read as decided on the strength of it.
//!
//! So context does not count, `:not(…)` and `:has(…)` do not count (a guard is
//! not a subject), and `:is(…)`/`:where(…)` DO, because those are the subject
//! written as an alternation. A rule that only sets a custom property on an
//! ancestor — `.pane { --measure: … }` — is a decision about `.pane` and about
//! nothing inside it, which is the same answer both ways round.
//!
//! # What this cannot see, stated rather than papered over
//!
//! **Granularity is the class, not the pair.** `.btn.primary` IS decided, by
//! `40-home-chat.css`'s `.home-compose-row .btn.primary` — and #120 is about
//! the same class in the permission modal, where no desktop rule reaches it.
//! One rule anywhere makes a class decided everywhere. Narrowing that would
//! mean modelling the ancestor chain, which is a cascade resolver and not a
//! test.
//!
//! **A class built at run time is invisible.** Nine `class:` values in the
//! scanned files are a call rather than a literal — `Mark::class`,
//! `TreeState::class`, `RecentState::class` (`src/shell/desktop/`),
//! `tag_class`, `value_class` (`inspector.rs`), `RowAction::class`
//! (`views/chrome.rs`), `ExtState::dot` (`views/extensions.rs`) and
//! `permission_button_class` (`views/chat.rs`) — and the scan reads none of
//! them. Measured, that hides nothing today: every name those helpers return
//! is either styled only by `assets/desktop/` and so outside this question
//! entirely (`mark`, `tree`, `recent`, `insp-tag`, `row-action`, `ok`, `bad`,
//! `accent`, `waiting`, `running`, `idle`, `awake`), or already reachable from
//! a literal somewhere else (`off`, `busy`, `err` from `class: "dot off"` in
//! `inspector.rs`; `btn`, `primary`, `secondary`, `danger-outline` from
//! `src/scheduler.rs`). Resolving them is not free and would be worse: the
//! Mobile arm of `RowAction::class` returns `"swipe-action"`, which is the
//! phone's alone, so a scan that read helper bodies would have to read
//! `Shell::Mobile` too or start reporting phone surface as desktop surface.
//!
//! **Element selectors are not classes.** `.md pre` is a shared rule the
//! desktop inherits and this module has no name to hang it on.
//!
//! **`assets/features/` is out of scope**, deliberately and not by oversight:
//! those five sheets reach the desktop the same way and have the same gap (12
//! classes, measured). They are a second wave, not a second definition — the
//! allowlist below is what Phase 2 triages, and it is long enough.

use std::collections::{BTreeMap, BTreeSet};

/// Every file under `src/` that writes a `class:` attribute, except the one
/// named in [`NOT_SCANNED`].
///
/// `include_str!` rather than a path built out of `CARGO_MANIFEST_DIR`, for the
/// reason [`crate::selfscan::code_of`] gives: `include_str!` resolves at
/// compile time, so a file renamed out from under this list fails to BUILD,
/// while a path composed at run time fails as "cannot read", one assertion
/// deep, on a machine where the rest of the suite has already gone green.
///
/// The list is written down AND checked against the directory
/// (`the_scan_reads_every_file_that_writes_a_class`), because a written list
/// is exactly how the previous scan came to miss `src/views/` entirely.
const SCANNED: &[(&str, &str)] = &[
    ("src/app.rs", include_str!("app.rs")),
    ("src/icons.rs", include_str!("icons.rs")),
    ("src/scheduler.rs", include_str!("scheduler.rs")),
    (
        "src/shell/desktop/home.rs",
        include_str!("shell/desktop/home.rs"),
    ),
    (
        "src/shell/desktop/inspector.rs",
        include_str!("shell/desktop/inspector.rs"),
    ),
    (
        "src/shell/desktop/mod.rs",
        include_str!("shell/desktop/mod.rs"),
    ),
    (
        "src/shell/desktop/sidebar.rs",
        include_str!("shell/desktop/sidebar.rs"),
    ),
    ("src/shell/mod.rs", include_str!("shell/mod.rs")),
    ("src/views/attach.rs", include_str!("views/attach.rs")),
    ("src/views/chat.rs", include_str!("views/chat.rs")),
    ("src/views/chrome.rs", include_str!("views/chrome.rs")),
    ("src/views/code.rs", include_str!("views/code.rs")),
    (
        "src/views/extensions.rs",
        include_str!("views/extensions.rs"),
    ),
    ("src/views/mod.rs", include_str!("views/mod.rs")),
    ("src/views/recipes.rs", include_str!("views/recipes.rs")),
    ("src/views/scheduler.rs", include_str!("views/scheduler.rs")),
    (
        "src/views/session_settings.rs",
        include_str!("views/session_settings.rs"),
    ),
    ("src/views/sessions.rs", include_str!("views/sessions.rs")),
    ("src/views/settings.rs", include_str!("views/settings.rs")),
    ("src/views/skills.rs", include_str!("views/skills.rs")),
];

/// The one file that writes classes and is not desktop surface.
///
/// `src/shell/mobile.rs` is declared `#[cfg(any(target_os = "ios", target_os =
/// "android"))] mod mobile;` in `src/shell/mod.rs`, so no desktop binary
/// contains a byte of it. Excluded by NAME rather than by the `cfg` scan below
/// because the `cfg` is on the declaration in another file, and a scan that
/// read the file itself would find nothing to exclude on.
/// `the_scan_reads_every_file_that_writes_a_class` re-reads that declaration,
/// so this exemption cannot outlive the `cfg` that justifies it.
const NOT_SCANNED: &[&str] = &["src/shell/mobile.rs"];

/// THE GAP, NAMED: every class `assets/shared.css` styles, the desktop
/// renders, and `assets/desktop/` says nothing about.
///
/// THIS LIST MAY ONLY SHRINK. It is not a set of exemptions — it is the
/// unreviewed inherited surface of this shell, written down at the size it was
/// on the day the gate landed, and the test below fails if an entry stops being
/// needed as loudly as it fails when a new one appears. Adding to it is how the
/// gate gets given away one name at a time; the answer to a new name is a rule
/// in `assets/desktop/` that decides how the class should look in a 1440pt
/// window, or a decision — recorded — that the phone's value is already right
/// there, which is a rule too (`70-overrides.css` is full of them).
///
/// Grouped by the file that renders each one, because that is the unit of
/// triage: 102 of the 107 come out of `src/views/`, the shared views the two
/// shells wear differently, and the four biggest groups are the code plane's
/// diff viewer, the chat composer's chips and modals, the attachment tray and
/// the extensions/settings rows. None of those four has ever been reviewed at
/// desktop width.
const UNDECIDED: &[&str] = &[
    // src/views/code.rs — the diff viewer, the review actions, the code
    // plane's own lists. 44, the largest single block.
    "action-chip",
    "action-row",
    "banner",
    "bare",
    "btn-row",
    "busy",
    "cbox",
    "chip",
    "chip-name",
    "compose-field",
    "count",
    "diff",
    "diff-badge",
    "diff-body",
    "diff-code",
    "diff-dir",
    "diff-file",
    "diff-file-head",
    "diff-name",
    "diff-note",
    "diff-path",
    "diff-progress",
    "diff-progress-fill",
    "diff-progress-label",
    "diff-progress-track",
    "diff-seen",
    "diff-sign",
    "diff-skip",
    "diff-skip-at",
    "diff-skip-label",
    "diff-stat",
    "err",
    "grow",
    "has-fab",
    "needed",
    "nowrap",
    "pull-actions",
    "session-ask",
    "session-ask-actions",
    "session-ask-more",
    "session-ask-title",
    "session-list",
    "session-meta",
    "stat",
    // src/views/chat.rs — the composer's model and effort chips and the
    // permission modal.
    "chip-effort",
    "chip-label",
    "chip-model",
    "chip-row",
    "dot-anim",
    "ellipsis",
    "error-box",
    "lost-ask",
    "modal-actions",
    "modal-pending",
    "modal-session",
    "modal-tool",
    "mode",
    "small",
    "stop",
    // src/views/attach.rs — the attachment tray above the composer.
    "attach",
    "attach-chip",
    "attach-icon",
    "attach-image",
    "attach-list",
    "attach-meta",
    "attach-name",
    "attach-remove",
    "attach-thumb",
    "attach-tray",
    "composer-chip",
    "reading",
    // src/views/extensions.rs — the extension cards and the setting rows the
    // settings screens share.
    "card",
    "fact",
    "field-label",
    "hint",
    "setting-list",
    "setting-main",
    "setting-name",
    "setting-note",
    "setting-row",
    "setting-value",
    "sheet-head",
    // src/views/chrome.rs — the session tile (#63) and its parts.
    "attention",
    "session-age",
    "session-main",
    "session-tile",
    "session-title",
    "subtitle",
    // src/views/recipes.rs — the recipe chooser.
    "choice",
    "choice-list",
    "choice-name",
    "selected",
    // src/views/session_settings.rs — the per-session sheet.
    "choice-lead",
    "choice-main",
    "choice-note",
    "sheet-search",
    // src/views/mod.rs — the scroll-to-bottom affordance's slot (#209 decided
    // the button itself) and the modal body every sheet is built from.
    "modal-body",
    "scroll-bottom-slot",
    // src/scheduler.rs — two of the three button faces.
    "danger-outline",
    "secondary",
    // src/views/settings.rs — the About block.
    "about",
    "about-conn",
    // src/app.rs — the toast, which floats over both shells at the phone's
    // width and position.
    "toast",
    // src/shell/desktop/inspector.rs — `.dot.off`, the offline badge #213 is
    // about, and the one name on this list the desktop shell renders itself.
    "off",
    // src/shell/desktop/mod.rs — the disclosure state on the sidebar's
    // Library group.
    "open",
];

/// The attribute that marks an item as the phone's alone.
const PHONE_ONLY: &str = "#[cfg(any(target_os = \"ios\", target_os = \"android\"))]";

/// The attribute that marks an item as test code.
const TEST_ONLY: &str = "#[cfg(test)]";

// ---- what the desktop can render ---------------------------------------

/// Every class name the desktop can put in the DOM, mapped to the file that
/// writes it.
fn rendered_classes() -> BTreeMap<String, &'static str> {
    let mut out = BTreeMap::new();
    for &(name, source) in SCANNED {
        let code = desktop_code(name, source);
        for start in code.match_indices("class:").map(|(at, m)| at + m.len()) {
            let value = attribute_value(code.get(start..).unwrap_or_default());
            // Odd `split('"')` fields are the string literals — so both arms of
            // `class: if on { "nav-row on" } else { "nav-row" }` are read, and
            // the state modifiers, which are exactly the names least likely to
            // have been considered, are not lost to the first branch.
            for literal in value.split('"').skip(1).step_by(2) {
                for token in literal.split_whitespace() {
                    // `class: "insp-step-dot {step.dot}"` interpolates: the
                    // name is decided at run time and the literal `{step.dot}`
                    // is a string no browser ever sees.
                    if token.contains('{') || token.contains('}') {
                        continue;
                    }
                    out.entry(token.to_owned()).or_insert(name);
                }
            }
        }
    }
    out
}

/// The part of `source` a desktop binary compiles and no test wrote.
///
/// Comment lines go first, so that prose quoting an attribute cannot answer a
/// question about code — and so that the two `cfg` cuts below cannot be
/// triggered by a doc comment naming the attribute they look for.
/// `src/shell/mod.rs:139` is exactly that: a doc comment containing
/// `#[cfg(test)]`, 156 lines above the test module, which truncated an earlier
/// draft of this scan at a third of the file.
///
/// Through [`crate::selfscan::code_of`] at the end for its post-condition,
/// which is the guarantee the two cuts are only heuristics for: whatever
/// arrangement of test modules a file grows, source that still holds a
/// `#[test]` fails there, loudly, instead of quietly supplying a scan with
/// class names no browser ever sees.
fn desktop_code(name: &str, source: &str) -> String {
    let prose_free: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let no_tests = without_items_under(&prose_free, TEST_ONLY);
    let desktop = without_items_under(&no_tests, PHONE_ONLY);
    // THE CUT'S OWN ASSUMPTION, CHECKED. `without_items_under` matches a whole
    // LINE, so it only ever removes an item at column 0 — and an
    // `#[cfg(test)]` or a phone `cfg` on a method inside an `impl` block is
    // indented, would be left behind, and would put its class literals into a
    // scan of what a desktop binary renders. None of the scanned files has one
    // today; this is what says so when one arrives, rather than the gate
    // quietly widening.
    for marker in [TEST_ONLY, PHONE_ONLY] {
        assert!(
            !desktop.contains(marker),
            "{name} still carries `{marker}` after the cut, which means it is \
             INDENTED — on a method in an `impl`, or inside another item. The \
             line-based cut cannot see that, so the code under it is about to \
             be read as desktop surface."
        );
    }
    crate::selfscan::code_of(name, &desktop)
}

/// `code` with every top-level item annotated `marker` removed.
///
/// LINE-BASED, and `cargo fmt --all -- --check` is what makes that sound: a
/// top-level item starts at column 0 and ends either at a line ending in `;`
/// or at a `}` alone in column 0, and everything in between is indented. So
/// the cut needs no brace matching — which is the version that was tried and
/// is not safe, because `src/shell/desktop/mod.rs`'s own test module contains
/// `'{' => depth += 1` and `'"' => quoted = true`, and a scanner that counts
/// braces and quotes without lexing Rust reads both wrong.
///
/// Ending EARLY leaves test code behind and [`crate::selfscan::code_of`]'s
/// post-condition fires. Ending LATE would silently delete real code, which is
/// why `the_scan_agrees_with_the_sheet_about_what_is_the_phones_alone` pins
/// both sides of the phone cut by name.
fn without_items_under(code: &str, marker: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in code.lines() {
        if skipping {
            let ends_item = line.trim_end().ends_with(';') || line == "}";
            let top_level = !line.starts_with(char::is_whitespace);
            skipping = !(top_level && ends_item);
            continue;
        }
        if line == marker {
            skipping = true;
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

/// The source of one `rsx!` attribute value: from `rest` to the comma that
/// ends it.
///
/// Not `split(',')`: a dozen class attributes in this tree are
/// `class: if row.selected { "nav-row on" } else { "nav-row" },` and a naive
/// split takes the first branch and loses the second. Depth counts braces and
/// parens together so `class: value_class(fact.mono, fact.accent),` is one
/// value and not two.
///
/// A near-copy of the private helper of the same name in
/// `src/shell/desktop/mod.rs`'s test module, and deliberately not shared with
/// it: that module is `cfg`-gated to non-phone targets, this one is not, and
/// hoisting a helper out of the file every other lane of this campaign is
/// editing buys twenty lines at the price of a conflict in all of them.
fn attribute_value(rest: &str) -> &str {
    let mut depth = 0_i32;
    let mut quoted = false;
    for (at, c) in rest.char_indices() {
        if quoted {
            quoted = c != '"';
            continue;
        }
        match c {
            '"' => quoted = true,
            '{' | '(' => depth += 1,
            '}' | ')' => {
                depth -= 1;
                // Out of the element this attribute is on: the value was the
                // last one and had no trailing comma.
                if depth < 0 {
                    return rest.get(..at).unwrap_or_default();
                }
            }
            ',' if depth == 0 => return rest.get(..at).unwrap_or_default(),
            _ => {}
        }
    }
    rest
}

// ---- what a stylesheet decides -----------------------------------------

/// The classes `assets/shared.css` styles.
fn styled_classes() -> BTreeSet<String> {
    subjects_of(crate::css::SHARED)
}

/// The classes `assets/desktop/` decides.
fn decided_classes() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for &(_, body) in crate::css::SHELL_PARTS {
        out.extend(subjects_of(body));
    }
    out
}

/// Every class on the SUBJECT of every rule in one sheet.
fn subjects_of(css: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for selector in selectors(css) {
        for one in split_top(&selector, ',') {
            compound_classes(&subject_of(&one), &mut out);
        }
    }
    out
}

/// Every selector list in `css` that introduces a declaration block.
///
/// At-rules are skipped by their `@` and walked INTO, so a rule inside
/// `@media (min-width: 1200px)` is read like any other — a responsive
/// override is still somebody having decided.
fn selectors(css: &str) -> Vec<String> {
    let stripped = crate::css::without_comments(css);
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    for c in stripped.chars() {
        if let Some(q) = quote {
            buf.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                buf.push(c);
            }
            '{' => {
                let selector = std::mem::take(&mut buf).trim().to_owned();
                if !selector.is_empty() && !selector.starts_with('@') {
                    out.push(selector);
                }
            }
            '}' => buf.clear(),
            _ => buf.push(c),
        }
    }
    out
}

/// `s` split on every `sep` that sits outside parens, brackets and quotes.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut buf = String::new();
    let mut depth = 0_i32;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        if let Some(q) = quote {
            buf.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == sep && depth == 0 {
            parts.push(std::mem::take(&mut buf));
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        buf.push(c);
    }
    parts.push(buf);
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect()
}

/// The last compound of one comma-free selector — the element its declarations
/// land on.
///
/// Splits on descendant, `>`, `+` and `~` alike: all four say "the thing on the
/// right is the subject and the thing on the left is where it has to be".
fn subject_of(selector: &str) -> String {
    let mut last = String::new();
    let mut buf = String::new();
    let mut depth = 0_i32;
    let mut quote: Option<char> = None;
    for c in selector.chars() {
        if let Some(q) = quote {
            buf.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        if depth == 0 && (c.is_whitespace() || c == '>' || c == '+' || c == '~') {
            if !buf.is_empty() {
                last = std::mem::take(&mut buf);
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        buf.push(c);
    }
    if buf.is_empty() {
        last
    } else {
        buf
    }
}

/// The classes an element matching `compound` has to carry.
///
/// `:is(…)` and `:where(…)` expand, because those ARE the subject, written as
/// an alternation. `:not(…)` and `:has(…)` do not, because a guard is not a
/// subject — see the module comment for the four rules in `assets/desktop/`
/// where that distinction is the whole answer. Attribute selectors are skipped
/// whole so a `[data-x=".y"]` cannot be read as a class.
fn compound_classes(compound: &str, out: &mut BTreeSet<String>) {
    let chars: Vec<char> = compound.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        match chars.get(at) {
            Some('.') => {
                let start = at + 1;
                let mut end = start;
                while chars
                    .get(end)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                {
                    end += 1;
                }
                if end > start {
                    out.insert(chars.get(start..end).unwrap_or_default().iter().collect());
                }
                at = end.max(start);
            }
            Some('[') => at = past_group(&chars, at, '[', ']'),
            Some(':') => at = past_pseudo(&chars, at, out),
            _ => at += 1,
        }
    }
}

/// Past the pseudo-class or pseudo-element beginning at `at`, having expanded
/// it if it is a subject alternation.
fn past_pseudo(chars: &[char], at: usize, out: &mut BTreeSet<String>) -> usize {
    let mut end = at + 1;
    if chars.get(end) == Some(&':') {
        end += 1;
    }
    let start = end;
    while chars
        .get(end)
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '-')
    {
        end += 1;
    }
    if chars.get(end) != Some(&'(') {
        return end.max(at + 1);
    }
    let name: String = chars
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    let close = past_group(chars, end, '(', ')');
    if matches!(name.as_str(), "is" | "where") {
        let inner: String = chars
            .get(end + 1..close.saturating_sub(1))
            .unwrap_or_default()
            .iter()
            .collect();
        for alternative in split_top(&inner, ',') {
            compound_classes(&subject_of(&alternative), out);
        }
    }
    close
}

/// The index just past the `close` that matches the `open` at `at`.
fn past_group(chars: &[char], at: usize, open: char, close: char) -> usize {
    let mut depth = 0_i32;
    let mut end = at;
    while let Some(&c) = chars.get(end) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return end + 1;
            }
        }
        end += 1;
    }
    chars.len()
}

// ---- the gates ---------------------------------------------------------

/// THE GATE. A class the desktop renders, the shared sheet styles, and
/// `assets/desktop/` has never mentioned, is a phone's decision shipping in a
/// 1440pt window with nobody's name on it.
///
/// Three assertions, and the second and third are what make [`UNDECIDED`] a
/// ledger rather than a suppression — it fails as loudly when an entry stops
/// being true as when a new name appears.
///
/// REPRODUCED, both directions, on this tree:
///
/// - Delete `80-measure.css`'s `.pane-main .fab` rule — the ONLY desktop
///   decision about `.fab`, and the one that moves the button off the pane's
///   edge onto the measure's — and this fails with `.fab (src/views/code.rs)`.
///   Nothing else in the repo does: `node docs/audit.js both` on that same
///   tree reports Clean over 98 phone states and 26 desktop states, because
///   the audit measures the captured store and no captured desktop state has
///   a `.fab` in it.
/// - Add `"topbar"` to [`UNDECIDED`] and the second assertion names it back:
///   `70-overrides.css` decides `.topbar`, so the line is a claim that has
///   stopped being true. Add a name nothing renders and the third does.
#[test]
fn every_shared_class_the_desktop_renders_has_a_desktop_decision() {
    let styled = styled_classes();
    let decided = decided_classes();
    let rendered = rendered_classes();

    // The floors, `src/shell/mod.rs`'s habit: say out loud that each scan found
    // something. A scan that matched nothing would pass forever, and all three
    // of these read a format — a selector, a `class:` attribute — that a tool
    // or a framework could change under them.
    assert!(
        styled.len() > 150,
        "assets/shared.css styles only {} leaf classes, which is far fewer than \
         it has — has the selector scan stopped recognising rules?",
        styled.len()
    );
    assert!(
        decided.len() > 150,
        "assets/desktop/ decides only {} leaf classes, which is far fewer than \
         it has",
        decided.len()
    );
    assert!(
        rendered.len() > 250,
        "the class scan found only {} names across the {} files of desktop \
         surface — has `class:` stopped being how an attribute is written?",
        rendered.len(),
        SCANNED.len()
    );

    let listed: BTreeSet<&str> = UNDECIDED.iter().copied().collect();
    assert_eq!(
        listed.len(),
        UNDECIDED.len(),
        "UNDECIDED names {} classes and {} of them are distinct. A duplicate is \
         a line nobody can delete, because deleting either one leaves the list \
         still naming it and the gate still green.",
        UNDECIDED.len(),
        listed.len()
    );

    let gap: BTreeMap<&str, &str> = rendered
        .iter()
        .filter(|(class, _)| styled.contains(class.as_str()))
        .filter(|(class, _)| !decided.contains(class.as_str()))
        .map(|(class, file)| (class.as_str(), *file))
        .collect();

    let unlisted: Vec<String> = gap
        .iter()
        .filter(|(class, _)| !listed.contains(*class))
        .map(|(class, file)| format!(".{class} ({file})"))
        .collect();
    assert!(
        unlisted.is_empty(),
        "{} class(es) the desktop renders are styled by assets/shared.css and \
         decided by nothing in assets/desktop/: {}. That is the phone's answer \
         shipping in a 1440pt window with nobody's name on it — the sheet is \
         emitted into BOTH shells, so a value chosen against a 402pt frame is \
         in force here until a desktop rule says otherwise. Write the rule, or \
         write the rule that says the phone's value is right (70-overrides.css \
         is full of those). Adding a line to UNDECIDED is not the answer: that \
         list may only shrink.",
        unlisted.len(),
        unlisted.join(", ")
    );

    let landed: Vec<&&str> = UNDECIDED
        .iter()
        .filter(|class| decided.contains(**class))
        .collect();
    assert!(
        landed.is_empty(),
        "UNDECIDED names {landed:?}, which assets/desktop/ now decides. The \
         list may only shrink: delete them, so that the next class to go \
         undecided is a failure and not a line in a list that has stopped \
         being true."
    );

    let gone: Vec<&&str> = UNDECIDED
        .iter()
        .filter(|class| !decided.contains(**class))
        .filter(|class| !gap.contains_key(**class))
        .collect();
    assert!(
        gone.is_empty(),
        "UNDECIDED names {gone:?}, which is not in the gap at all — either \
         assets/shared.css no longer styles it, or nothing the desktop \
         compiles renders it. Same rule: delete them. An entry that names \
         nothing is an entry nobody can check."
    );
}

/// [`SCANNED`] IS THE DIRECTORY, and this is the only thing that says so.
///
/// The gate above is exactly as wide as this list, and the list is written by
/// hand. That is not a hypothetical failure:
/// `every_class_the_desktop_shell_renders_is_in_the_captured_store` names four
/// files and `src/views/` is not among them, which is why `.action-row` (#201)
/// was invisible to it and to everything else in the repo. A new view file, or
/// an old one that gains its first `class:`, must not be able to arrive
/// unscanned.
///
/// The exemption is re-derived rather than trusted: `src/shell/mobile.rs` is
/// excluded because `src/shell/mod.rs` declares it under the phone `cfg`, and
/// this reads that declaration back. Un-gate the module and the exemption
/// fails with it.
///
/// This file excludes ITSELF, through `file!()` rather than by name, and that
/// is `src/selfscan.rs`'s lesson in its smallest form: the needle this walk
/// looks for is `class:`, which is written sixteen times in the module you are
/// reading. A scan that counted its own source would report the scanner as
/// desktop surface — and `file!()` keeps saying which file that is when the
/// module is renamed, where a literal would quietly stop matching and put the
/// old name back in the diff.
#[test]
fn the_scan_reads_every_file_that_writes_a_class() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let itself = file!().replace('\\', "/");
    let mut on_disk: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_rust = path.extension().is_some_and(|ext| ext == "rs");
            if is_rust
                && std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("class:")
            {
                if let Ok(relative) = path.strip_prefix(root) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    if relative != itself {
                        on_disk.insert(relative);
                    }
                }
            }
        }
    }
    assert!(
        on_disk.len() > 15,
        "only {} files under src/ write a `class:` attribute, which is fewer \
         than this app has screens — the walk found nothing to read",
        on_disk.len()
    );

    let mut listed: BTreeSet<String> = SCANNED.iter().map(|&(name, _)| name.to_owned()).collect();
    listed.extend(NOT_SCANNED.iter().map(|&name| name.to_owned()));
    assert_eq!(
        listed, on_disk,
        "SCANNED plus NOT_SCANNED is not the set of files under src/ that write \
         a class. A file missing from both is desktop surface the inheritance \
         gate cannot see, which is the exact shape of #201: `.action-row` lives \
         in src/views/, the only other class scan in this repo reads four files \
         in src/shell/desktop/, and nothing noticed for the life of the shell."
    );

    let shell = SCANNED
        .iter()
        .find(|&&(name, _)| name == "src/shell/mod.rs")
        .map_or("", |&(_, source)| source);
    for name in NOT_SCANNED {
        let file = name
            .trim_start_matches("src/shell/")
            .trim_end_matches(".rs");
        assert!(
            shell.contains(&format!("{PHONE_ONLY}\nmod {file};")),
            "{name} is exempt from the scan because src/shell/mod.rs declares \
             it under the phone `cfg`, and that declaration is no longer there. \
             Either the module is compiled into desktop binaries now — in which \
             case it is desktop surface and belongs in SCANNED — or it moved, \
             and this exemption is naming nothing."
        );
    }
}

/// THE PHONE CUT DOES SOMETHING, AND NOT TOO MUCH.
///
/// `assets/shared.css`'s own header names the phone-only classes: `.ptr`, the
/// drawer panel head (`.drawer`, `.drawer-scrim`, `.drawer-brand`) and
/// `.drawer-group`. Three different mechanisms keep them out of the reachable
/// set — `.ptr` is written by `src/viewport.rs` in JavaScript and never as a
/// `class:` attribute at all, the drawer head comes from the un-compiled
/// `src/shell/mobile.rs`, and `.drawer-group` comes from `render_group` in
/// `src/shell/mod.rs`, which is `cfg`'d to phones inside a file the scan does
/// read. Only the third exercises [`without_items_under`], and it is the one
/// that would fail silently: a cut that did nothing would put five classes the
/// desktop cannot render into the gap, and 107 unreviewed names is not a number
/// anyone would notice five wrong ones in.
///
/// The other half is the danger the line-based cut actually has — ending LATE
/// and eating real code. `render_destination` sits directly below
/// `render_group` in the same file and is NOT `cfg`'d, because both shells
/// render a nav row; `.drawer-item` is what it writes, and if the cut ran on
/// past the closing brace it would go missing.
#[test]
fn the_scan_agrees_with_the_sheet_about_what_is_the_phones_alone() {
    let styled = styled_classes();
    let rendered = rendered_classes();
    for class in [
        "ptr",
        "drawer",
        "drawer-scrim",
        "drawer-brand",
        "drawer-group",
    ] {
        assert!(
            styled.contains(class),
            "assets/shared.css no longer styles .{class}, which its own header \
             lists as one of the phone's own — the header and the sheet have \
             come apart"
        );
        assert!(
            !rendered.contains_key(class),
            "the scan reads .{class} as desktop surface. assets/shared.css's \
             header says it is the phone's alone, so either the header is now \
             wrong or the cut that is supposed to drop it — the phone `cfg`, \
             src/shell/mobile.rs, or JavaScript — has stopped dropping it. \
             Every rule in shared.css about it would then be counted as \
             inherited surface nobody decided."
        );
    }
    assert_eq!(
        rendered.get("drawer-item").copied(),
        Some("src/shell/mod.rs"),
        "src/shell/mod.rs no longer offers .drawer-item to the desktop. \
         `render_destination` writes it and is deliberately NOT `cfg`'d — the \
         two shells share the row — so this going missing most likely means the \
         phone cut above it ran past `render_group`'s closing brace and ate the \
         rest of the file."
    );
}

/// THE SUBJECT IS THE DECISION, exercised on the four selectors that decide it.
///
/// The definition is the gate. Get it wrong in the loose direction and the gate
/// reports Clean over the classes it was built to find; the four cases here are
/// the ones `assets/desktop/` actually contains, and each was measured before
/// it was written down.
#[test]
fn only_the_subject_of_a_selector_is_a_decision_about_it() {
    let classes = |selector: &str| {
        let mut out = BTreeSet::new();
        for one in split_top(selector, ',') {
            compound_classes(&subject_of(&one), &mut out);
        }
        out.into_iter().collect::<Vec<_>>()
    };

    assert_eq!(
        classes(".btn.primary"),
        ["btn", "primary"],
        "both classes are on the subject, so a rule for the compound decides \
         both"
    );
    assert_eq!(
        classes(".pane .topbar > .conn-badge"),
        ["conn-badge"],
        "context is not a decision: this rule moves the badge and says nothing \
         about the bar it is in"
    );
    assert_eq!(
        classes(".pane-main .scroll:not(.diff)"),
        ["scroll"],
        "80-measure.css's rule EXCLUDES .diff. Counting a guard as a decision \
         is the definition under which a rule written to miss a class reports \
         it as handled."
    );
    assert_eq!(
        classes(".pane-main:has(.scroll.diff) .topbar"),
        ["topbar"],
        ":has() is a question about a descendant, not a claim about it"
    );
    assert_eq!(
        classes(".nav-row:is(.on, :hover) :is(.nav-row-sub, .nav-row-age)"),
        ["nav-row-age", "nav-row-sub"],
        ":is() in the SUBJECT is an alternation of subjects and both are \
         decided; the :is() before it is context and is not"
    );
    assert_eq!(
        classes(".insp-chip.off > i"),
        Vec::<String>::new(),
        "an element subject carries no class, so this decides nothing this gate \
         can name — and .off, which it only mentions, stays undecided"
    );
    assert_eq!(
        classes(".a::before, .b:hover, .c[data-x=\".notaclass\"]"),
        ["a", "b", "c"],
        "pseudo-elements, plain pseudo-classes and attribute values are not \
         classes, and a comma is a list of subjects"
    );

    // THE THREE FALLTHROUGHS, EXERCISED — because the sheets do not exercise
    // them and an unexercised arm is an arm nobody has read. Each is what the
    // scan does when its input runs out mid-construct, and each answer has to
    // be "stop, keeping what you had" rather than "loop" or "panic": these run
    // over 20 Rust files and 16 stylesheets that six lanes are editing at once,
    // and a scan that hangs or panics on a half-written selector is a scan
    // somebody deletes.
    assert_eq!(
        classes(".a > "),
        ["a"],
        "a selector ending in a combinator has no subject after it, so the last \
         complete compound is the subject"
    );
    assert_eq!(
        classes(".a:is(.b"),
        ["a"],
        "an unclosed :is() runs to the end of the selector and stops there \
         rather than off it: the alternation it was going to expand never \
         closed, so it contributes nothing and the compound before it still \
         does"
    );
    assert_eq!(
        attribute_value("\"nav-row\""),
        "\"nav-row\"",
        "an attribute value with no comma after it is the rest of the source, \
         which is what a `class:` as the last thing in a file would give"
    );
}

/// The selector walk reads at-rules the way the cascade does, and a `{` inside
/// a string does not end a rule.
///
/// `assets/shared.css` has no `content: "{"` today. It has 4 `@keyframes`, 5
/// `@media` and 2 `@supports` blocks, and a walk that treated an at-rule's
/// condition as a selector would put `(min-width:` into the decided set, while
/// one that skipped the block whole would lose every responsive override
/// `65-responsive.css` exists to hold.
#[test]
fn the_selector_walk_reads_nested_rules_and_ignores_strings() {
    let css = "@media (min-width: 900px) { .pane { gap: 1rem } }\n\
               .x { content: \"} .not-a-rule {\" }\n\
               @keyframes spin { from { opacity: 0 } }\n\
               .y, .z > .w { color: red }";
    assert_eq!(
        selectors(css),
        [".pane", ".x", "from", ".y, .z > .w"],
        "the at-rule conditions are skipped and their contents are not; the \
         braces inside a string are not rules"
    );
    let mut out = BTreeSet::new();
    for selector in selectors(css) {
        for one in split_top(&selector, ',') {
            compound_classes(&subject_of(&one), &mut out);
        }
    }
    assert_eq!(
        out.into_iter().collect::<Vec<_>>(),
        ["pane", "w", "x", "y"],
        "`from` contributes no class, `.z` is context, and `.not-a-rule` was \
         never a selector"
    );
}

/// The sheet this module reads is the sheet the binary embeds.
///
/// [`crate::css::SHARED`] names `assets/shared.css` alone, which
/// [`crate::css::STYLES`] does not — it is that sheet plus the five in
/// `assets/features/`. Two `include_str!` of one path cannot disagree about
/// its CONTENT, but they can disagree about whether the const still describes
/// the app: `STYLES` is what ships, and if `shared.css` ever stopped being its
/// first part, every number this module reports would be about a sheet nobody
/// links.
#[test]
fn the_shared_sheet_is_the_front_of_the_sheet_that_ships() {
    assert!(
        crate::css::STYLES.starts_with(crate::css::SHARED),
        "assets/shared.css is no longer the first part of STYLES, so the \
         inheritance gate is measuring a stylesheet the app does not emit first \
         — or does not emit at all"
    );
    assert!(
        crate::css::STYLES.len() > crate::css::SHARED.len(),
        "STYLES is no longer than SHARED, so the assets/features/ sheets have \
         gone missing from it"
    );
}
