//! EVERY PATH A COMMENT NAMES STILL RESOLVES.
//!
//! This tree argues for itself in prose, and the prose cites: 871 repo-relative
//! paths in the comments of 68 files, at the commit this landed. A citation is
//! only worth writing because the reader can follow it, and a reader who
//! follows two dead ones stops following any of them — so a path that has
//! stopped resolving is not a typo, it is the whole mechanism failing quietly.
//!
//! MEASURED, WHICH IS WHY THIS EXISTS RATHER THAN A HABIT. The desktop-v1
//! campaign was bitten by a stale citation four separate times in one week: a
//! brief that named a file two commits after it was renamed, an issue that
//! cited `60-sidebar-extra.css:44` a day after #154 deleted it, a comment block
//! that quoted the layout its own commit had just replaced, and a ledger note
//! that overstated its own measurement and cost a lane a wrong assumption. The
//! sweep that cleaned that up (#143) found **twelve** live citations of one
//! path, `src/shell/desktop.rs`, which #40 had turned into a directory: three
//! region files, seven places in `docs/audit.js`, and two in `docs/design.md`
//! that this gate does not read. Ten of the twelve are inside this scope, and
//! all twelve survived a green `cargo test --workspace`, a Clean
//! `node docs/audit.js both` and every review in between, because nothing in
//! the repo could ask.
//!
//! # What it reads
//!
//! [`SCANNED`] — `src/**/*.rs`, `assets/**/*.css` and `docs/*.js`. That is the
//! code, the sheets and the audit: the three places this campaign churns, and
//! the three where a citation is read by someone who cannot see the file tree
//! beside it. The DIRECTORY rather than a written list, for `src/inherit.rs`'s
//! reason — the only source scan in this tree that names its files by hand had
//! `.action-row` invisible to it for the life of the desktop shell.
//!
//! **Comments only.** `src/views/chat.rs` and `src/views/code.rs` are full of
//! `"src/new.rs"`, `"src/a.rs"`, `"assets/logo.png"` — diff fixtures, which are
//! DATA that happens to look like a path, and a gate that read them would
//! demand the repo contain a file a test invented. Rust `//` and `/* */`, CSS
//! `/* */`, JavaScript both.
//!
//! **A citation that wrapped is one citation.** A comment line ending in `-`
//! is joined to the next one before the scan, because
//! `docs/permission-` + `durability.md` and `assets/desktop/40-home-` +
//! `chat.css` are both in this tree and neither is two paths. The line reported
//! is the first of the pair, which is the one to open.
//!
//! # What it does NOT read, and why the scope is not wider
//!
//! **`.md` is out.** Not because docs matter less — the sweep fixed two dead
//! paths in `docs/design.md` — but because this repo's Markdown legitimately
//! cites three other trees. `docs/permission-durability.md` walks goose's own
//! source (`crates/goose/src/agents/agent.rs`), `docs/push-notifications.md`
//! walks dioxus-cli's and objc2's (`src/build/apple.rs`,
//! `src/generated/UNUserNotificationCenter.rs`), `docs/shared-artifacts.md` is
//! *about* personal-ai-setup, and `docs/desktop-roadmap.md` contains the
//! sentence "neither `src/platform/` nor `src/action.rs` exists (`ls src/`)",
//! which is a citation whose whole point is that it does not resolve. Gating
//! those would be an allowlist longer than the finding.
//!
//! **Bare file names are out.** `agent.rs`, `SKILL.md`, `opencode.json`,
//! `NSWindow.rs` — hundreds of them, nearly all in other trees, and a basename
//! match would be a weaker question anyway.
//!
//! So this is the NARROWEST version that still fails on a deleted path, and it
//! is stated that way on purpose: the wider gate was measured (557 unresolved
//! lines across the whole repo, the overwhelming majority of them correct) and
//! rejected on its own numbers.
//!
//! # The two ledgers
//!
//! [`ANOTHER_REPO`] and [`DELIBERATELY_GONE`] are the seven citations in scope
//! that are right to be unresolvable, and each **fails in both directions**:
//! an entry the scan no longer finds is as loud as a path nobody listed. They
//! are not shrink-only — a historical note is allowed to accumulate — but a
//! new line in either is a claim about intent, and the reviewer's question is
//! whether the comment says which tree it means or that the name is a past one.
//!
//! `#[cfg(test)]` at the declaration in `src/main.rs`, following `selfscan`,
//! `inherit` and `voice`, so none of this is in any binary.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The directories walked, and the extension read in each.
///
/// `docs` is `js` alone: `docs/audit.js` and the three `measure-*.js` are the
/// only files there that are code, and the module comment gives the argument
/// against the `.md`.
const SCANNED: &[(&str, &str)] = &[("src", "rs"), ("assets", "css"), ("docs", "js")];

/// What a citation starts with. Anything else in a comment is prose.
const ROOTS: &[&str] = &["src/", "assets/", "docs/", "scripts/", "crates/"];

/// THIS MODULE, AND NOTHING ELSE.
///
/// It has to exempt itself, for `src/voice.rs`'s reason one level over: the
/// prose above and the ledgers below QUOTE dead paths — `src/shell/desktop.rs`,
/// `assets/desktop.css`, `src/platform/`, the diff fixtures `src/a.rs` and
/// `assets/logo.png` — because naming them is how the argument is made. A scan
/// that read its own source would report seventeen findings, every one of them
/// a sentence explaining why the finding matters. Measured: that is exactly
/// what the first run of this gate did.
///
/// Through `file!()` rather than a literal, following `src/inherit.rs` — it
/// keeps naming this file when the module is renamed, where a literal would
/// quietly stop matching and the exemption would silently widen to nothing.
/// A one-name list, and [`the_scan_reads_everything_but_its_own_ledger`] is
/// what keeps it one.
const NOT_SCANNED: &[&str] = &["src/citations.rs"];

/// Citations that resolve in a DIFFERENT repository, as (file, path).
///
/// Every one of these names the other tree in its own sentence — "personal-ai
/// -setup", "the brain repo's", or a `crates/goose/` prefix no crate in this
/// workspace has — which is what makes them citations rather than mistakes.
const ANOTHER_REPO: &[(&str, &str)] = &[
    ("src/code.rs", "docs/code-agents.md"),
    ("src/code.rs", "docs/privacy.md"),
    ("src/code.rs", "scripts/vps/code-agent-manager.py"),
    ("src/extensions.rs", "docs/privacy.md"),
    ("src/skills.rs", "crates/goose/src/skills/mod.rs"),
];

/// Citations that name a path this tree used to have, on purpose.
///
/// Both are the same sentence in two region files, and both are load-bearing
/// history rather than a stale pointer: `assets/desktop.css` is the single
/// sheet #157 cut into `assets/desktop/`, and the reason a rule sits where it
/// does is that the cut ran in filename order. Correcting the name to the
/// directory would make the sentence false, which is the distinction this
/// ledger exists to record.
const DELIBERATELY_GONE: &[(&str, &str)] = &[
    ("assets/desktop/30-sidebar-list.css", "assets/desktop.css"),
    ("assets/desktop/40-home-chat.css", "assets/desktop.css"),
];

/// One path named in one comment.
#[derive(Debug, PartialEq, Eq)]
struct Citation {
    /// Repo-relative, with `/` separators on every host.
    file: String,
    /// In the file AS WRITTEN — the FIRST line when the citation wrapped, so
    /// the message names the line to open.
    line: usize,
    /// The cited path, repo-relative.
    path: String,
}

// ---- reading the tree ---------------------------------------------------

/// Every file in [`SCANNED`], as (repo-relative path, source).
fn scanned_files() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for &(dir, ext) in SCANNED {
        let mut stack = vec![root.join(dir)];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // `docs/js` is one level by construction; the others nest.
                    if dir != "docs" {
                        stack.push(path);
                    }
                    continue;
                }
                if path.extension().is_none_or(|it| it != ext) {
                    continue;
                }
                let name = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if NOT_SCANNED.contains(&name.as_str()) {
                    continue;
                }
                out.push((name, std::fs::read_to_string(&path).unwrap_or_default()));
            }
        }
    }
    out.sort();
    out
}

/// The comment text in `source`, as (1-based line, text), one entry per line
/// that carries any.
///
/// `slashes` is whether `//` starts a comment, which is true of Rust and
/// JavaScript and false of CSS. Block comments are tracked across lines, so a
/// `/* … */` spanning forty lines yields forty entries and each keeps its own
/// number.
///
/// A string literal containing `/*` would open a comment that is not there.
/// That is a real hole and it is left open deliberately: closing it needs the
/// four-way string walk `src/voice.rs::literals` carries, and the failure it
/// would cause here is a scan that reads MORE text than it should — which can
/// only produce a path that has to resolve, never hide one that does not.
fn comment_lines(source: &str, slashes: bool) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (at, line) in source.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut text = String::new();
        let mut i = 0;
        while i < chars.len() {
            if in_block {
                if chars.get(i) == Some(&'*') && chars.get(i + 1) == Some(&'/') {
                    in_block = false;
                    i += 2;
                } else {
                    text.extend(chars.get(i));
                    i += 1;
                }
                continue;
            }
            if chars.get(i) == Some(&'/') && chars.get(i + 1) == Some(&'*') {
                in_block = true;
                i += 2;
                continue;
            }
            if slashes && chars.get(i) == Some(&'/') && chars.get(i + 1) == Some(&'/') {
                text.extend(chars.get(i + 2..).unwrap_or_default().iter());
                break;
            }
            i += 1;
        }
        if !text.is_empty() {
            out.push((at + 1, text));
        }
    }
    out
}

/// Join every comment line that ends in `-` to the one after it.
///
/// The leading `/`, `!` and `*` of the continuation are dropped first — a
/// `///` line arrives here as `/ durability.md`, and gluing that on would
/// produce `docs/permission-/ durability.md`, which is the false positive this
/// exists to prevent rather than a theoretical one.
fn close_up_wraps(lines: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((at, first)) = lines.get(i) else {
            break;
        };
        let mut text = first.clone();
        while text.trim_end().ends_with('-') {
            let Some((_, next)) = lines.get(i + 1) else {
                break;
            };
            i += 1;
            text = text.trim_end().to_owned();
            text.push_str(next.trim().trim_start_matches(['/', '!', '*']).trim());
        }
        out.push((*at, text));
        i += 1;
    }
    out
}

/// Every repo-relative path named in one line of comment text.
///
/// A citation runs from one of [`ROOTS`] through the path characters after it,
/// then loses whatever trailing punctuation the sentence added — a `.` ending
/// the sentence, the `:` before a line number, the `/` of a directory written
/// with one. `src/views/*.rs` yields `src/views`, and a directory resolving is
/// the right answer for it.
fn cited_paths(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let tail: String = chars.get(i..).unwrap_or_default().iter().collect();
        let Some(root) = ROOTS.iter().find(|it| tail.starts_with(**it)) else {
            i += 1;
            continue;
        };
        // A root that is the tail of a longer word or path is not a citation:
        // `src/` inside `crates/goose/src/` is already covered by the whole,
        // and `-src/` is a word.
        if chars
            .get(i.wrapping_sub(1))
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
            && i > 0
        {
            i += 1;
            continue;
        }
        let mut end = i;
        while chars
            .get(end)
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
        {
            end += 1;
        }
        let raw: String = chars.get(i..end).unwrap_or_default().iter().collect();
        let path = raw.trim_end_matches(['.', '/', '-']);
        if path.len() > root.len() - 1 {
            out.push(path.to_owned());
        }
        i = end.max(i + 1);
    }
    out
}

/// The comment text of one file, whole citations closed up, keyed on the
/// syntax its extension implies.
///
/// CSS has one comment form and Rust and JavaScript have two, so the only
/// question the caller has is whether `//` opens one. Asked through
/// [`std::path::Path`] rather than `ends_with(".css")`, which clippy's
/// `case_sensitive_file_extension_comparisons` is right about here for a
/// reason that is not hypothetical: this walk builds its names off a
/// case-preserving `read_dir`, and a `.CSS` read as Rust would take every
/// `//` in a URL for a comment.
fn comments_of(file: &str, source: &str) -> Vec<(usize, String)> {
    let css = std::path::Path::new(file)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("css"));
    close_up_wraps(&comment_lines(source, !css))
}

/// Every citation in the scanned tree that does not resolve.
fn broken_citations(files: &[(String, String)]) -> Vec<Citation> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for (file, source) in files {
        for (at, text) in comments_of(file, source) {
            for path in cited_paths(&text) {
                if root.join(&path).exists() {
                    continue;
                }
                out.push(Citation {
                    file: file.clone(),
                    line: at,
                    path,
                });
            }
        }
    }
    out
}

/// How many citations the scan found in total, resolving or not.
fn citation_count(files: &[(String, String)]) -> usize {
    files
        .iter()
        .map(|(file, source)| {
            comments_of(file, source)
                .into_iter()
                .map(|(_, text)| cited_paths(&text).len())
                .sum::<usize>()
        })
        .sum()
}

/// Whether one ledger entry is about one broken citation.
fn covers(found: &Citation, file: &str, path: &str) -> bool {
    found.file == file && found.path == path
}

// ---- the gate -----------------------------------------------------------

/// THE GATE. A path a comment names is a path the reader can open.
///
/// Three assertions, and the second and third are what make the ledgers
/// ledgers rather than suppressions: an entry that has stopped matching fails
/// as loudly as a path nobody listed.
///
/// REPRODUCED, both directions, on the tree this landed on: rename any file in
/// `assets/desktop/` without fixing its citations and this names each of them
/// with the file and line to open; delete either `assets/desktop.css` line from
/// [`DELIBERATELY_GONE`] and it names it straight back.
#[test]
fn every_path_a_comment_names_still_resolves() {
    let files = scanned_files();
    let found = broken_citations(&files);
    let citations = citation_count(&files);

    // The floors, `src/voice.rs`'s habit: say out loud that the scan found
    // something. A walk that matched nothing would pass forever, and this one
    // reads three comment syntaxes that a formatter release could move under
    // it.
    assert!(
        files.len() > 50,
        "only {} files were read, which is fewer than this app has screens — \
         the walk found nothing to scan",
        files.len()
    );
    assert!(
        citations > 600,
        "only {citations} paths were cited in those files' comments — either \
         the tree has stopped arguing for itself or `cited_paths` has stopped \
         recognising a citation"
    );

    let listed: BTreeSet<(&str, &str)> = ANOTHER_REPO
        .iter()
        .chain(DELIBERATELY_GONE)
        .copied()
        .collect();
    assert_eq!(
        listed.len(),
        ANOTHER_REPO.len() + DELIBERATELY_GONE.len(),
        "the two ledgers name {} citations and {} of them are distinct. A \
         duplicate is a line nobody can delete, because deleting either one \
         leaves the list still naming it and the gate still green.",
        ANOTHER_REPO.len() + DELIBERATELY_GONE.len(),
        listed.len()
    );

    let unlisted: Vec<String> = found
        .iter()
        .filter(|it| !listed.iter().any(|&(file, path)| covers(it, file, path)))
        .map(|it| format!("{}:{} cites {}", it.file, it.line, it.path))
        .collect();
    assert!(
        unlisted.is_empty(),
        "{} comment(s) name a path that is not in this tree: {}. Fix the \
         citation — a reader who follows two dead ones stops following any of \
         them, which is how `src/shell/desktop.rs` stayed in twelve comments \
         for five months after #40 turned it into a directory. If the path is \
         in ANOTHER repository, say which in the same sentence and add it to \
         ANOTHER_REPO; if it names something this tree deliberately no longer \
         has, add it to DELIBERATELY_GONE. Both are read by a reviewer, and \
         neither is the cheap answer to a red build.",
        unlisted.len(),
        unlisted.join("; ")
    );

    let gone: Vec<&(&str, &str)> = listed
        .iter()
        .filter(|&&(file, path)| !found.iter().any(|it| covers(it, file, path)))
        .collect();
    assert!(
        gone.is_empty(),
        "the ledgers name {gone:?}, which the scan no longer finds — the \
         comment has been reworded, moved, or the path now resolves. Delete \
         those lines, so the list goes on being a list of what is really there."
    );
}

/// [`NOT_SCANNED`] IS AN EXEMPTION LIST, and this is what stops it becoming a
/// place to put a file.
///
/// It may hold exactly one name and that name must be this module's, read from
/// `file!()` rather than written down. Anything else — a region file whose
/// citations are inconvenient, a view that quotes a path in a fixture — is a
/// hole in the gate that nothing else in the repo could see.
#[test]
fn the_scan_reads_everything_but_its_own_ledger() {
    let itself = file!().replace('\\', "/");
    assert_eq!(
        NOT_SCANNED,
        [itself.as_str()],
        "NOT_SCANNED is meant to hold this module and nothing else — it quotes \
         dead paths on purpose, and every other file's citations are the \
         question"
    );

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(
        root.join(&itself).exists(),
        "{itself} is exempt from a scan of a tree that no longer holds it"
    );
    assert!(
        !scanned_files().iter().any(|(file, _)| *file == itself),
        "{itself} is on NOT_SCANNED and the walk read it anyway"
    );
}

// ---- the parts, so the gate's own reading is checked --------------------

#[test]
fn a_block_comment_is_read_across_every_line_it_spans() {
    let css = "/* one\n * assets/desktop/50-band.css\n */\n.x { color: red }\n";
    let lines = comment_lines(css, false);
    assert_eq!(
        lines.len(),
        3,
        "a three-line block is three lines of comment"
    );
    assert_eq!(
        lines.get(1).map(|(at, _)| *at),
        Some(2),
        "the number is the file's, not the comment's"
    );
    assert!(
        comment_lines(".x { color: red }\n", false).is_empty(),
        "a declaration is not a comment"
    );
}

#[test]
fn a_slash_comment_is_rust_and_javascript_and_not_css() {
    let source = "let x = 1; // docs/audit.js\n";
    assert_eq!(
        comment_lines(source, true)
            .first()
            .map(|(_, text)| text.trim().to_owned()),
        Some("docs/audit.js".to_owned()),
        "a trailing comment is comment text"
    );
    assert!(
        comment_lines(source, false).is_empty(),
        "`//` is not a comment in CSS, and reading it as one would scan a URL"
    );
}

#[test]
fn a_citation_that_wrapped_is_one_citation() {
    let wrapped = vec![
        (10, "/ the case (docs/permission-".to_owned()),
        (11, "/ durability.md section 0)".to_owned()),
    ];
    let joined = close_up_wraps(&wrapped);
    assert_eq!(
        joined.first().map(|(at, _)| *at),
        Some(10),
        "the line reported is the first of the pair"
    );
    assert_eq!(
        joined
            .first()
            .map(|(_, text)| cited_paths(text))
            .unwrap_or_default(),
        vec!["docs/permission-durability.md".to_owned()],
        "the halves close up the way the reader's eye closes them"
    );
}

#[test]
fn a_path_is_taken_without_the_punctuation_around_it() {
    assert_eq!(
        cited_paths("see `src/state.rs:557` and src/nav.rs."),
        vec!["src/state.rs".to_owned(), "src/nav.rs".to_owned()],
        "a line number and a full stop are the sentence's, not the path's"
    );
    assert_eq!(
        cited_paths("every src/views/*.rs is shared"),
        vec!["src/views".to_owned()],
        "a glob resolves to the directory it globs, which is the real claim"
    );
    assert_eq!(
        cited_paths("crates/goose-acp-client/src/lib.rs"),
        vec!["crates/goose-acp-client/src/lib.rs".to_owned()],
        "the inner `src/` is part of the whole and not a second citation"
    );
    assert!(
        cited_paths("no paths here, just prose about assets and docs").is_empty(),
        "a root word without a slash is prose"
    );
}
