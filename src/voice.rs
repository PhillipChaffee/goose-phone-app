//! WHO THE APP IS TALKING TO, and the one thing about it no compiler checks.
//!
//! `src/views/` is worn by both shells. It carries zero `cfg(target_os)` and
//! that is deliberate — `src/shell/mod.rs` gives the reason, and it is a good
//! one: the presentation rules that hang off the platform are total functions
//! of [`crate::shell::Shell`], so `cargo test` runs BOTH arms in one process,
//! where a `cfg` at each leaf would leave the mobile arm verified by nothing.
//! Shared views can therefore share STRUCTURE freely.
//!
//! They cannot share COPY. Every sentence in `src/views/` is read in a
//! 1440x860 macOS window as surely as on a 402pt phone, and a sentence that
//! names the reader's hardware, or the gesture they make at it, is wrong in one
//! of the two places. #145 fixed one such string. #161 was filed as "the tail",
//! swept `src/views/` and listed seven. The re-sweep found TEN, in six files —
//! and two of them are `src/recipes.rs` and `src/scheduler.rs`, outside
//! `src/views/` entirely, because a facts card composes its sentence one layer
//! below the view that renders it. That is why the earlier directory-scoped
//! sweep could go green with live sites still shipping, and it is why this is a
//! gate and not a fourth sweep.
//!
//! # The mechanism was not missing; the rule was
//!
//! [`crate::shell::Shell::CURRENT`] is a compile-time `const`, so a view picks
//! wording per shell with no `cfg`, no runtime branch and nothing extra in the
//! binary. The shape the repo settled on is a function that TAKES a `Shell` —
//! [`crate::shell::this_device`] is the general one — with the call site
//! passing `CURRENT`. Taking it as a parameter is load-bearing: a host
//! `cargo test` is always `Shell::Desktop`, so a function that read `CURRENT`
//! itself would make the phone arm unassertable.
//!
//! Nothing was missing from that toolbox. What was missing is anything that
//! notices when nobody reaches for it.
//!
//! # What this reads, and what it refuses to read
//!
//! **Every `.rs` file under `src/`**, minus [`NOT_SCANNED`] — and the list of
//! files is the DIRECTORY rather than a table written by hand. The sketch in
//! #196 proposed a hand-written list; `src/inherit.rs` is the repo's evidence
//! against one, because the only other source scan in this tree names four
//! files and `.action-row` was invisible to it for the life of the shell. A
//! written list has exactly the failure mode this gate exists to close.
//!
//! **String literals only**, and only ones that read as a sentence. A class
//! attribute's value is a token list and so is a set of state modifiers, which
//! is what keeps `"swipe-action danger"` — the phone's row-action class in
//! `src/views/chrome.rs` — from tripping a gate about prose. A hyphen is a word
//! boundary, so nothing weaker than [`is_prose`] holds that line.
//!
//! **Not test code**, through [`crate::selfscan::without_items_under`], and not
//! comments: an assertion may quote the sentence it is asserting about, and a
//! doc comment above a string will eventually quote the string.
//!
//! **Not a lint justification.** `#[expect(…, reason = "the phone drawer's
//! alone")]` in `src/nav.rs` and `src/state.rs` is prose about the code,
//! addressed to whoever hits the lint. Nine of them say "phone" and none of
//! them is copy.
//!
//! # What counts as having decided
//!
//! A device word is fine INSIDE a shell branch — the literal's own line names a
//! `Shell::` variant, or the function it sits in takes a `Shell`. The gate does
//! not ban the word; it bans the word where one shell's answer is being given
//! to both. `views::settings`'s Tailscale hint is the reference case and passes
//! for that reason.
//!
//! # Stated rather than papered over
//!
//! **One direction.** The words below are the phone's. The desktop's own —
//! "click", "window", "hover" — are not listed, because every JavaScript blob
//! in `src/viewport.rs`, `src/attach.rs`, `src/domdump.rs` and
//! `src/shell/desktop/mod.rs` is a string literal full of `window.` and
//! `addEventListener('click')`, and telling those from copy costs more than the
//! risk is worth. The phone is the product and shared copy is written
//! phone-first; this catches the direction the mistakes have actually come in.
//!
//! **Granularity is the literal.** A sentence assembled at run time out of
//! pieces that each say nothing is invisible here — which is what the fixes do,
//! and correctly: `format!("… not on {}.", this_device(CURRENT))` has no device
//! word in it because the device word has become a shell's answer.
//!
//! `#[cfg(test)]` at the declaration in `src/main.rs`, following `selfscan` and
//! `inherit`, so none of this is in any binary.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::selfscan::{without_items_under, TEST_ONLY};

/// The words that name a phone, or a way of touching one.
///
/// Inflections are spelled out because the match is on WHOLE WORDS: "tap" does
/// not find "taps", since `s` is alphanumeric and so not a boundary. A hyphen
/// IS a boundary, which is the whole reason [`is_prose`] has to exist.
const DEVICE_WORDS: &[&str] = &[
    "handset", "phone", "phones", "pinch", "swipe", "swipes", "swiping", "tap", "tapped",
    "tapping", "taps",
];

/// Phrases that name the wrong thing rather than the wrong device.
///
/// "your desktop" is the goose Desktop APPLICATION in every place this app
/// writes it, and it reads as the reader's own machine — ambiguous on a phone
/// and actively misleading in a macOS window, where the reader IS at a desktop
/// and the sentence is telling them to go somewhere else. The fix is to name
/// the product, which is right on both shells and needs no branch, so this
/// belongs in a list of its own rather than among the words above.
const AMBIGUOUS_PHRASES: &[&str] = &["your desktop"];

/// The files under `src/` this scan does not read, and why.
///
/// Two reasons, and both are re-derived from the declaration that justifies
/// them by [`the_scan_reads_every_file_that_ships_a_string`] rather than
/// trusted: a module compiled only into a phone binary may say "phone", and a
/// module compiled only into a test binary is not copy at all — its strings are
/// fixtures and failure messages, several of which quote the very sentences
/// this gate is about.
const NOT_SCANNED: &[&str] = &[
    "src/inherit.rs",
    "src/selfscan.rs",
    "src/serverkit.rs",
    "src/shell/mobile.rs",
    "src/testkit.rs",
    "src/views/press.rs",
    "src/voice.rs",
];

/// THE SITES, NAMED: shipping sentences that tell a desktop reader they are
/// holding a phone.
///
/// THIS LIST MAY ONLY SHRINK, for [`crate::inherit`]'s `UNDECIDED` reason and
/// checked the same way — an entry that has stopped matching anything fails as
/// loudly as a new sentence does. It is not a set of exemptions. It landed at
/// ten, which is what #161 found, so that the eleventh would be a red build
/// rather than a fourth sweep.
///
/// Each entry is a file and a FRAGMENT of the sentence, matched against the
/// literal as the compiler builds it — so a line continuation in the source
/// does not have to be reproduced here, and rewording around the fragment still
/// fails.
///
/// NINE OF THE TEN WENT IN #161. This is the tenth, and it is still here for
/// ownership rather than for difficulty: `src/views/code.rs` belongs to another
/// lane of this campaign, whose own fix is in `CodeNewView`, and an edit here
/// would conflict with it.
///
/// It is the worst of the ten. A `title` attribute is a HOVER tooltip, so this
/// sentence is ONLY ever read on the desktop — it exists to be read with a
/// pointer and it says tap. The wording it wants is
/// `views::chrome::row_action_words`'s: a `Shell` branch with "tap" on one side
/// and "click" on the other.
const ADDRESSING_A_PHONE: &[(&str, &str)] = &[("src/views/code.rs", "tap to unmark")];

/// One shipping sentence that names a device.
#[derive(Debug)]
struct Sighting {
    /// Repo-relative, with `/` separators on every host.
    file: String,
    /// In the file AS WRITTEN, so the message is something to open.
    line: usize,
    /// The word that was found, so the message says what is wrong with it.
    word: &'static str,
    /// The literal as the compiler builds it.
    text: String,
}

// ---- reading the tree ---------------------------------------------------

/// Every `.rs` file under `src/` that ships in a binary, as (path, source).
///
/// From disk rather than from `include_str!`, which is the other way this repo
/// reads source in tests. The trade is deliberate and it is the one
/// `src/inherit.rs`'s directory check makes as well: `include_str!` resolves at
/// compile time, so a renamed file fails to BUILD — but it can only do that for
/// files somebody remembered to list, and the failure this gate exists for is a
/// file nobody listed. The floor in
/// [`no_shared_string_tells_a_desktop_reader_they_are_holding_a_phone`] stands
/// in for the compiler here.
fn shared_sources() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let skipped: BTreeSet<&str> = NOT_SCANNED.iter().copied().collect();
    let mut out = Vec::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let name = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if skipped.contains(name.as_str()) {
                continue;
            }
            out.push((name, std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
    out.sort();
    out
}

/// The sentences in one file that name a device outside a shell branch.
fn phone_voice(file: &str, source: &str) -> Vec<Sighting> {
    let prose_free: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let code = without_items_under(&prose_free, TEST_ONLY);
    // The post-condition `crate::selfscan::code_of` carries, for its reason: a
    // test module left in the region supplies its own needles, and the tests
    // in this tree quote the very sentences this gate is about.
    assert!(
        !code.contains("#[test]"),
        "{file}: the region this scan reads still contains test code, so an \
         assertion quoting a sentence would be read as the sentence"
    );

    let lines: Vec<&str> = code.lines().collect();
    let mut out = Vec::new();
    for (at, raw) in literals(&code) {
        let before = code.get(..at).unwrap_or_default();
        // `#[expect(…, reason = "…")]` is prose about the code, addressed to
        // whoever hits the lint. Nine of them in this tree say "phone".
        if before.trim_end().ends_with("reason =") {
            continue;
        }
        let text = text_of(&raw);
        if !is_prose(&text) {
            continue;
        }
        let Some(word) = device_words(&text).first().copied() else {
            continue;
        };
        if shell_selected(&lines, before.matches('\n').count()) {
            continue;
        }
        out.push(Sighting {
            file: file.to_owned(),
            line: line_in(source, &raw),
            word,
            text,
        });
    }
    out
}

/// Where `raw` starts in the file as written.
///
/// The scan runs over a copy with the comments and the test items taken out, so
/// its own line numbers are not the file's. The literal itself is untouched by
/// both cuts, so finding it again in the original is what puts a number in the
/// message that an editor understands. First occurrence when a file writes one
/// sentence twice — which #161 found two files doing, and the fix for both was
/// to stop doing it.
fn line_in(source: &str, raw: &str) -> usize {
    source.find(raw).map_or(0, |at| {
        source.get(..at).unwrap_or_default().matches('\n').count() + 1
    })
}

// ---- reading Rust -------------------------------------------------------

/// Every string literal in `code`, as (byte offset of its opening delimiter,
/// the RAW text between the delimiters).
///
/// Raw rather than unescaped, because both halves are used to find the literal
/// again — the offset in `code` for the `reason =` question and the line, the
/// text in the original source for the line number.
///
/// A hand-rolled walk over four things Rust says with quotes in them, each of
/// which desynchronises a naive scan for the rest of the file:
///
/// - `"…"` with `\"` inside (`src/icons.rs` writes SVG path data that way);
/// - `r"…"` and `r#"…"#`, which have no escapes at all — `src/domdump.rs`'s
///   `DUMP_JS` is a multi-line `r"` holding JavaScript;
/// - `'"'`, a char literal whose content is a quote, which `src/inherit.rs`'s
///   selector walk and `src/css.rs`'s comment stripper both contain, and which
///   a scan that skipped every `'` would read as the start of a lifetime;
/// - a trailing `// …` comment, which the line filter above does NOT remove
///   because the line starts with code.
fn literals(code: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = code.chars().collect();
    // Byte offsets, kept alongside the char walk so the result can index back
    // into `code` without a second pass over it.
    let mut offsets: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut byte = 0;
    for c in &chars {
        offsets.push(byte);
        byte += c.len_utf8();
    }
    offsets.push(byte);
    let offset_at = |i: usize| offsets.get(i).copied().unwrap_or(byte);

    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        // A comment that starts after code on the same line.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while chars.get(i).is_some_and(|c| *c != '\n') {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while chars.get(i).is_some()
                && !(chars.get(i) == Some(&'*') && chars.get(i + 1) == Some(&'/'))
            {
                i += 1;
            }
            i += 2;
            continue;
        }
        // A raw string, and however many hashes it chose.
        if c == 'r' && !preceded_by_word_char(&chars, i) {
            let mut hashes = 0;
            while chars.get(i + 1 + hashes) == Some(&'#') {
                hashes += 1;
            }
            if chars.get(i + 1 + hashes) == Some(&'"') {
                let (text, end) = raw_string(&chars, i + 2 + hashes, hashes);
                out.push((offset_at(i), text));
                i = end;
                continue;
            }
        }
        if c == '\'' {
            i = past_char_or_lifetime(&chars, i);
            continue;
        }
        if c == '"' {
            let (text, end) = quoted_string(&chars, i + 1);
            out.push((offset_at(i), text));
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// Whether the character before `at` can be part of an identifier, which is how
/// the `r` of `mirror` is told from the `r` of `r"…"`.
fn preceded_by_word_char(chars: &[char], at: usize) -> bool {
    at > 0
        && chars
            .get(at - 1)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
}

/// From `open` to the closing quote: the text, and the index just past it.
fn quoted_string(chars: &[char], open: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = open;
    while let Some(&c) = chars.get(i) {
        if c == '\\' {
            text.push(c);
            if let Some(&escaped) = chars.get(i + 1) {
                text.push(escaped);
            }
            i += 2;
            continue;
        }
        if c == '"' {
            return (text, i + 1);
        }
        text.push(c);
        i += 1;
    }
    (text, chars.len())
}

/// The same for `r##"…"##`, which has no escapes at all: the close is a quote
/// followed by exactly `hashes` hashes.
fn raw_string(chars: &[char], open: usize, hashes: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = open;
    while let Some(&c) = chars.get(i) {
        if c == '"' && (0..hashes).all(|n| chars.get(i + 1 + n) == Some(&'#')) {
            return (text, i + 1 + hashes);
        }
        text.push(c);
        i += 1;
    }
    (text, chars.len())
}

/// Past the `'` at `at`, whichever of the two things it starts.
///
/// `'a` is a lifetime and one character long; `'x'`, `'\n'` and `'\''` are char
/// literals and have to be stepped over whole, because the quote inside `'"'`
/// would otherwise open a string that runs to the next one.
fn past_char_or_lifetime(chars: &[char], at: usize) -> usize {
    if chars.get(at + 1) == Some(&'\\') {
        let mut i = at + 2;
        while chars.get(i).is_some_and(|c| *c != '\'') {
            i += 1;
        }
        return i + 1;
    }
    if chars.get(at + 2) == Some(&'\'') {
        return at + 3;
    }
    at + 1
}

/// A literal's text as the compiler builds it.
///
/// The escape that changes what a sentence SAYS is `\` at end of line, which
/// takes the newline and the next line's indentation with it: without that,
/// `"on the phone yet, so this recipe \` … `has to be started"` holds a word
/// this gate is looking for with thirteen spaces glued to it.
fn text_of(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c != '\\' {
            out.push(c);
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('\n') => {
                i += 2;
                while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                    i += 1;
                }
            }
            Some('n') => {
                out.push('\n');
                i += 2;
            }
            Some('t') => {
                out.push(' ');
                i += 2;
            }
            Some(&other) => {
                out.push(other);
                i += 2;
            }
            None => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

// ---- deciding ------------------------------------------------------------

/// Whether `text` reads as a sentence rather than as a list of tokens.
///
/// The distinction the whole gate rests on. A class attribute's value is
/// lowercase, hyphens and spaces and nothing else — `"swipe-action danger"`,
/// `"modal sheet rename"`, `"dot off"` — and a hyphen is a word boundary, so
/// without this the phone's own row-action class in `src/views/chrome.rs` is a
/// failure. Anything a person wrote to be read has a capital, a comma, a full
/// stop or a digit in it somewhere; anything a stylesheet reads does not.
fn is_prose(text: &str) -> bool {
    text.contains(' ')
        && text
            .chars()
            .any(|c| !(c.is_ascii_lowercase() || c == '-' || c == ' '))
}

/// The device words `text` says, in the order they are listed.
fn device_words(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    DEVICE_WORDS
        .iter()
        .chain(AMBIGUOUS_PHRASES)
        .copied()
        .filter(|needle| says(&lower, needle))
        .collect()
}

/// Whether `lower` contains `needle` as a whole word.
///
/// Boundaries are non-alphanumeric, so a hyphen is one — `"swipe-action"` says
/// "swipe" — and `s` is not, so "tap" does not find "taps". That is why
/// [`DEVICE_WORDS`] spells the inflections out.
fn says(lower: &str, needle: &str) -> bool {
    lower.match_indices(needle).any(|(at, _)| {
        let before = lower.get(..at).unwrap_or_default().chars().next_back();
        let after = lower
            .get(at + needle.len()..)
            .unwrap_or_default()
            .chars()
            .next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Whether the literal starting on line `at` is inside a shell branch.
///
/// Two shapes, because the repo writes both. The literal's own line naming a
/// `Shell::` variant is the match arm — `Shell::Mobile => "this phone"` — and
/// the enclosing function taking a `Shell` is the rest of it, which is the
/// shape [`crate::shell::this_device`] and `views::chrome::row_action_words`
/// have and the shape #145 established.
///
/// A literal at column 0 is a module-level constant with no enclosing function
/// at all, so the nearest `fn` above it is a preceding sibling and decides
/// nothing about it. Without that guard one `const` written under a
/// shell-taking function would be exempt by accident.
fn shell_selected(lines: &[&str], at: usize) -> bool {
    let Some(line) = lines.get(at) else {
        return false;
    };
    if line.contains("Shell::") {
        return true;
    }
    if !line.starts_with(char::is_whitespace) {
        return false;
    }
    signature_above(lines, at).is_some_and(|signature| signature.contains("Shell"))
}

/// The signature of the nearest function declared above `at`.
///
/// Joined forward to the line that opens the body, because a signature in this
/// app is regularly four lines long and `shell: Shell` is not always on the
/// first of them. Capped, so a `fn` with no body cannot make this read to the
/// end of the file.
fn signature_above(lines: &[&str], at: usize) -> Option<String> {
    let start = (0..at)
        .rev()
        .find(|&i| lines.get(i).copied().is_some_and(declares_a_function))?;
    let mut signature = String::new();
    for line in lines.get(start..=at)?.iter().take(12) {
        signature.push_str(line);
        signature.push(' ');
        if line.contains('{') || line.trim_end().ends_with(';') {
            break;
        }
    }
    Some(signature)
}

/// Whether `line` declares a function.
///
/// Token-wise rather than by substring: `where F: Fn(&str)`, a call to
/// `fn_name(…)` and a trailing `// fn` comment all contain the letters and none
/// of them opens a signature. What may stand before the `fn` is a visibility, a
/// `const`, an `async`, an `unsafe` or an `extern`, and nothing else.
fn declares_a_function(line: &str) -> bool {
    let mut before: Vec<&str> = Vec::new();
    for token in line.split_whitespace() {
        if token == "fn" {
            return before.iter().all(|word| {
                word.starts_with("pub")
                    || matches!(*word, "const" | "async" | "unsafe" | "extern" | "default")
            });
        }
        before.push(token);
    }
    false
}

/// Whether one ledger entry is about one sighting.
fn covers(found: &Sighting, file: &str, fragment: &str) -> bool {
    found.file == file && found.text.contains(fragment)
}

// ---- the gates -----------------------------------------------------------

/// THE GATE. A sentence the desktop reads that tells the reader they are
/// holding a phone is one shell's answer given to both.
///
/// Three assertions, and the second and third are what make
/// [`ADDRESSING_A_PHONE`] a ledger rather than a suppression: it fails as
/// loudly when an entry stops being true as when a new sentence appears.
///
/// REPRODUCED, both directions, on this tree: delete any line from the ledger
/// and this names the sentence back, with its file and the line it is on;
/// reword any of those sentences past its fragment without deleting the line
/// and the third assertion says the entry has stopped matching.
#[test]
fn no_shared_string_tells_a_desktop_reader_they_are_holding_a_phone() {
    let sources = shared_sources();
    let mut sightings = Vec::new();
    let mut prose = 0_usize;
    for (file, source) in &sources {
        sightings.extend(phone_voice(file, source));
        prose += literals(source)
            .into_iter()
            .filter(|(_, raw)| is_prose(&text_of(raw)))
            .count();
    }

    // The floors, `src/shell/mod.rs`'s habit: say out loud that the scan found
    // something. A walk that matched nothing would pass forever, and this one
    // reads a source format a rustfmt release could move under it.
    assert!(
        sources.len() > 25,
        "only {} files under src/ were read, which is fewer than this app has \
         screens — the walk found nothing to scan",
        sources.len()
    );
    assert!(
        prose > 400,
        "only {prose} literals in those files read as prose — either the tree \
         has lost its copy or `is_prose` has stopped recognising a sentence"
    );

    let listed: BTreeSet<(&str, &str)> = ADDRESSING_A_PHONE.iter().copied().collect();
    assert_eq!(
        listed.len(),
        ADDRESSING_A_PHONE.len(),
        "ADDRESSING_A_PHONE names {} sentences and {} of them are distinct. A \
         duplicate is a line nobody can delete, because deleting either one \
         leaves the list still naming it and the gate still green.",
        ADDRESSING_A_PHONE.len(),
        listed.len()
    );

    let unlisted: Vec<String> = sightings
        .iter()
        .filter(|found| {
            !listed
                .iter()
                .any(|&(file, fragment)| covers(found, file, fragment))
        })
        .map(|found| {
            format!(
                "{}:{} says {:?} in {:?}",
                found.file, found.line, found.word, found.text
            )
        })
        .collect();
    assert!(
        unlisted.is_empty(),
        "{} shipping sentence(s) name the reader's device outside a shell \
         branch: {}. src/views/ is worn by BOTH shells, so this is read in a \
         1440x860 macOS window as well as on a phone. Pick the wording per \
         shell — `crate::shell::this_device(Shell::CURRENT)` is the general \
         one and `views::settings`'s Tailscale hint is the worked example — or \
         say the thing that is true on both, which for a claim about this \
         CLIENT rather than about hardware is usually the shorter sentence. \
         Adding a line to ADDRESSING_A_PHONE is not the answer: that list may \
         only shrink.",
        unlisted.len(),
        unlisted.join("; ")
    );

    let gone: Vec<&(&str, &str)> = ADDRESSING_A_PHONE
        .iter()
        .filter(|&&(file, fragment)| !sightings.iter().any(|found| covers(found, file, fragment)))
        .collect();
    assert!(
        gone.is_empty(),
        "ADDRESSING_A_PHONE names {gone:?}, which the scan no longer finds — \
         the sentence has been fixed, moved, or reworded past its fragment. \
         The list may only shrink: delete those lines, so that the next \
         sentence to address a phone is a failure and not an entry in a list \
         that has stopped being true."
    );
}

/// [`NOT_SCANNED`] IS AN EXEMPTION LIST, and this is what stops it becoming a
/// place to put a file.
///
/// Every name on it is justified by a declaration in another file — either
/// `#[cfg(test)]`, which means the module is in no shipping binary, or the
/// phone `cfg`, which means it is in the one binary where "phone" is the right
/// word. Both are re-read here rather than trusted, so un-gating a module takes
/// its exemption with it: `src/shell/mobile.rs` compiled into a desktop binary
/// is a file full of phone copy this gate has stopped reading.
///
/// This module exempts ITSELF, and has to: [`ADDRESSING_A_PHONE`] quotes ten
/// sentences about phones, so a scan that read its own source would report the
/// scanner. Through `file!()` rather than a literal, following
/// `src/inherit.rs` — it keeps saying which file that is when the module is
/// renamed, where a literal would quietly stop matching.
#[test]
fn the_scan_reads_every_file_that_ships_a_string() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let itself = file!().replace('\\', "/");
    assert!(
        NOT_SCANNED.contains(&itself.as_str()),
        "{itself} is not in NOT_SCANNED, so this module is about to scan its \
         own ledger and report every sentence in it"
    );

    let phone_only = "#[cfg(any(target_os = \"ios\", target_os = \"android\"))]";
    for name in NOT_SCANNED {
        let path = root.join(name);
        assert!(
            path.exists(),
            "{name} is exempt from a scan of a tree that no longer holds it"
        );
        // Where a module is declared: the `mod.rs` beside it, or `main.rs` for
        // a file directly under `src/`.
        let stem = name
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .trim_end_matches(".rs");
        let parent = path
            .parent()
            .map(|dir| dir.join("mod.rs"))
            .filter(|beside| beside.exists())
            .unwrap_or_else(|| root.join("src/main.rs"));
        let declaration = std::fs::read_to_string(&parent).unwrap_or_default();
        let lines: Vec<&str> = declaration.lines().collect();
        let wanted = format!("mod {stem};");
        let gated = lines.iter().enumerate().any(|(at, line)| {
            line.trim_end().ends_with(&wanted)
                && at > 0
                && lines
                    .get(at - 1)
                    .is_some_and(|above| *above == TEST_ONLY || *above == phone_only)
        });
        assert!(
            gated,
            "{name} is exempt from this scan because {} declares it under \
             `{TEST_ONLY}` or the phone `cfg`, and that declaration is no \
             longer there. Either the module ships in a desktop binary now — in \
             which case its copy is read in a 1440pt window and belongs in the \
             scan — or it moved, and this exemption is naming nothing.",
            parent.display()
        );
    }
}

// ---- the scanner's own machinery, on fixtures ----------------------------

/// THE MECHANISM WORKS, on a fixture rather than on a real file — `selfscan`'s
/// discipline and its reason: the tree is supposed to be clean, so a test that
/// read one of its files could only ever say "still fine". This one says "the
/// gate fires", which is a different claim and the one that has to survive.
///
/// Four sentences and only the first is a failure: the second is the wording
/// chosen per shell in the arm itself, the third is the same choice made by a
/// function that takes a `Shell`, and the fourth is a class attribute's value,
/// which says "swipe" with a hyphen after it.
#[test]
fn the_gate_fires_on_a_sentence_and_not_on_a_shell_branch() {
    let source = concat!(
        "fn shipped() -> &'static str {\n",
        "    \"Set on another client, in a form this phone cannot edit.\"\n",
        "}\n",
        "fn chosen() -> &'static str {\n",
        "    match Shell::CURRENT {\n",
        "        Shell::Mobile => \"Tap to unmark. Really.\",\n",
        "        Shell::Desktop => \"Click to unmark. Really.\",\n",
        "    }\n",
        "}\n",
        "const fn taken(shell: Shell) -> &'static str {\n",
        "    match shell {\n",
        "        Mobile => \"Swipe the row. It deletes it.\",\n",
        "        Desktop => \"Use the row's own button.\",\n",
        "    }\n",
        "}\n",
        "fn styled() -> &'static str {\n",
        "    \"swipe-action danger\"\n",
        "}\n",
    );
    let found = phone_voice("fixture.rs", source);
    assert_eq!(
        found.len(),
        1,
        "the gate should see exactly the one sentence written for both shells \
         at once: {found:?}"
    );
    assert_eq!(
        found.first().map(|one| (one.word, one.line)),
        Some(("phone", 2)),
        "the gate found something other than the sentence on line 2: {found:?}"
    );
}

/// Comments and test code are not copy, and a lint's justification is not copy
/// either — `src/nav.rs` and `src/state.rs` write nine `reason =` strings that
/// say "phone" about the code rather than to the reader.
#[test]
fn prose_about_the_code_is_not_prose_the_reader_sees() {
    let source = concat!(
        "/// A tap on this phone opens it.\n",
        "#[expect(\n",
        "    dead_code,\n",
        "    reason = \"the phone drawer's alone; see `Group`\"\n",
        ")]\n",
        "fn shipped() {}\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    #[test]\n",
        "    fn t() { assert!(html.contains(\"Tap it, on this phone.\")); }\n",
        "}\n",
    );
    let found = phone_voice("fixture.rs", source);
    assert!(
        found.is_empty(),
        "a doc comment, a lint reason or a test assertion was read as shipping \
         copy: {found:?}"
    );
}

/// A word is a word. `-` is a boundary and `s` is not, which is why the
/// inflections are spelled out and why [`is_prose`] has to carry the class
/// attributes on its own.
#[test]
fn a_device_word_is_a_word_and_not_a_substring() {
    assert_eq!(device_words("Tap to unmark."), ["tap"]);
    assert_eq!(device_words("Two taps, no more."), ["taps"]);
    assert_eq!(device_words("swipe-action danger"), ["swipe"]);
    assert_eq!(device_words("Add one from your desktop."), ["your desktop"]);
    assert_eq!(
        device_words("Started from goose Desktop."),
        Vec::<&str>::new(),
        "naming the product is the fix for the ambiguous phrase, so the fix \
         must not still read as one"
    );
    assert_eq!(
        device_words("Tapered, telephony, untapped."),
        Vec::<&str>::new(),
        "a device word inside a longer word is not a device word"
    );
}

/// The line the whole gate rests on: a token list is not a sentence.
#[test]
fn a_token_list_is_not_prose() {
    assert!(!is_prose("swipe-action danger"));
    assert!(!is_prose("modal sheet rename"));
    assert!(!is_prose("session-swipe"));
    assert!(is_prose("Run on the goose server, not on this phone."));
    assert!(is_prose("this phone and never saved here — get it again"));
    assert!(
        is_prose("2 taps and it is gone"),
        "a digit is something a person wrote, so a sentence may be all \
         lowercase otherwise"
    );
}

/// The four things Rust says with quotes in them, each of which desynchronises
/// a naive walk for the rest of the file — plus the two ways a literal can run
/// off the end of one.
#[test]
fn the_walk_reads_every_shape_a_rust_string_comes_in() {
    let code = concat!(
        "let a = \"escaped \\\" quote\";\n",
        "let b = r#\"raw \"quoted\" text\"#;\n",
        "let c = '\"';\n",
        "let d = \"after the char literal\";\n",
        "let e = 'a';\n",
        "let n = '\\n';\n",
        "let mirror = 1 /* a \"block\" comment */ + 2;\n",
        "let f = \"after the lifetime\"; // \"a trailing comment\"\n",
        "let g = \"last\";\n",
    );
    let found: Vec<String> = literals(code).into_iter().map(|(_, raw)| raw).collect();
    assert_eq!(
        found,
        [
            "escaped \\\" quote",
            "raw \"quoted\" text",
            "after the char literal",
            "after the lifetime",
            "last",
        ],
        "a quote inside a char literal, a raw string, a block comment or a \
         trailing comment took the rest of the file with it"
    );
    assert_eq!(
        literals("let a = \"unterminated"),
        [(8, "unterminated".to_owned())],
        "a literal with no closing quote must stop at the end of the input"
    );
    assert_eq!(
        literals("let a = r#\"unterminated"),
        [(8, "unterminated".to_owned())],
        "a raw literal with no closing delimiter must stop there too"
    );
}

/// What the compiler builds, not what the source shows. The escape that matters
/// is `\` at end of line, which eats the newline AND the next line's
/// indentation — without it a device word arrives glued to thirteen spaces and
/// the word match misses it.
#[test]
fn a_continued_line_closes_up_the_way_the_compiler_closes_it() {
    assert_eq!(
        text_of("on the phone yet, so this recipe \\\n             has to be started"),
        "on the phone yet, so this recipe has to be started"
    );
    assert_eq!(text_of("a \\\"quoted\\\" word"), "a \"quoted\" word");
    assert_eq!(text_of("one\\ntwo"), "one\ntwo");
    assert_eq!(text_of("a tab\\there"), "a tab here");
    assert_eq!(
        text_of("a trailing backslash \\"),
        "a trailing backslash \\",
        "a literal that ends mid-escape must not run off the end"
    );
}

/// A `fn` in a body is not a signature. Three of the shapes below appear in
/// this tree and each of them contains the letters.
#[test]
fn only_a_signature_declares_a_function() {
    assert!(declares_a_function(
        "pub(crate) const fn this_device(shell: Shell)"
    ));
    assert!(declares_a_function("    async fn go() {"));
    assert!(declares_a_function("fn main() {"));
    assert!(!declares_a_function("    let x = fn_name(y);"));
    assert!(!declares_a_function("where F: Fn(&str) -> bool,"));
    assert!(!declares_a_function("    let x = 1; // fn"));
}

/// A module-level constant has no enclosing function, so the nearest `fn` above
/// it is a preceding sibling that decided nothing about it — and a signature
/// spread over four lines is still one signature.
#[test]
fn a_constant_beside_a_shell_function_is_not_inside_it() {
    let lines = [
        "pub(crate) const fn taken(",
        "    shell: Shell,",
        ") -> &'static str {",
        "    match shell {",
        "        Shell::Mobile => \"this phone\",",
        "        Shell::Desktop => \"this computer\",",
        "    }",
        "}",
        "const LOOSE: &str = \"Tap it here.\";",
    ];
    assert!(
        shell_selected(&lines, 4),
        "the arm names a Shell variant on its own line, which is the branch"
    );
    assert!(
        shell_selected(&lines, 6),
        "a line inside a function whose signature — four lines of it — takes a \
         Shell is inside the branch"
    );
    assert!(
        !shell_selected(&lines, 8),
        "a module-level const was exempted by a function it is merely written \
         under"
    );
    assert!(
        !shell_selected(&lines, 99),
        "a line past the end of the file decides nothing"
    );
    assert!(
        !shell_selected(&["    \"Tap it here.\""], 0),
        "an indented literal with no function above it at all has no signature \
         to be exempted by"
    );
}
