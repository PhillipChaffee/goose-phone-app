//! Turning `OpenCode`'s per-file patch into something reviewable on a phone.
//!
//! `GET /session/:id/diff` does not hand back hunks. `Snapshot.diffFull`
//! builds each patch with `jsdiff`'s `structuredPatch(..., { context:
//! Number.MAX_SAFE_INTEGER })`, which means **one hunk per file spanning the
//! whole file, with every unchanged line present as a context line**. A
//! three-line edit to a 1200-line file arrives as a 1204-line patch. That is
//! the reason the old `.diff-panel` was unreadable: it was not a raw blob by
//! accident, the server hands you the entire repo.
//!
//! So the work here is client-side re-hunking: parse the patch into numbered
//! lines, then decide which of them to show. Everything in this module is
//! pure — no Dioxus, no I/O — so it is unit-testable under
//! `cargo test --workspace`, which is where the parser's edge cases live.

use std::collections::HashMap;

/// Unchanged lines kept either side of a changed run.
pub(crate) const CONTEXT: usize = 3;

/// Lines a tap on a collapsed band reveals, split between its two ends.
///
/// Split rather than revealed from the top because a band sits *between* two
/// changes: you tap it to see the end of the one above or the start of the
/// one below, and one control that grows in both directions serves both
/// without adding a second target to a 402px row.
pub(crate) const EXPAND_STEP: usize = 20;

/// Ceiling on rows rendered for one file.
///
/// Re-hunking puts a typical file at 20–60 rows, so this only bites on a huge
/// *added* file — all `+`, nothing to collapse — which is exactly the case
/// that would otherwise stall the `WebView` for seconds.
pub(crate) const RENDER_CAP: usize = 800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    Context,
    Add,
    Del,
}

/// One row of a patch, with the line numbers the collapsed bands need even
/// though no row displays one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffLine {
    pub kind: LineKind,
    /// Number on the old side; 0 for an added line.
    pub old_no: u32,
    /// Number on the new side; 0 for a removed line.
    pub new_no: u32,
    pub text: String,
    /// The patch carried `\ No newline at end of file` after this line.
    pub no_newline: bool,
}

impl DiffLine {
    pub(crate) const fn changed(&self) -> bool {
        !matches!(self.kind, LineKind::Context)
    }

    /// The glyph in the sign column. This is the primary way a changed line
    /// is told apart: it survives greyscale, every form of colour blindness,
    /// and a screenshot pasted into a chat.
    pub(crate) const fn sign(&self) -> &'static str {
        match self.kind {
            LineKind::Context => " ",
            LineKind::Add => "+",
            LineKind::Del => "-",
        }
    }

    pub(crate) const fn row_class(&self) -> &'static str {
        match self.kind {
            LineKind::Context => "diff-line ctx",
            LineKind::Add => "diff-line add",
            LineKind::Del => "diff-line del",
        }
    }
}

/// A run of unchanged lines far enough from any change to be worth hiding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Gap {
    /// Index of the run's first line. Also its identity in the expand map:
    /// it is derived from the parse, so revealing a band from its ends never
    /// moves the key out from under the state that tracks it.
    pub start: usize,
    pub len: usize,
}

/// What to render, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    /// Lines `start..end` shown as rows.
    Rows { start: usize, end: usize },
    /// A collapsed band standing in for `hidden` lines, the first of which is
    /// line `at` on the new side.
    Gap { key: usize, hidden: usize, at: u32 },
}

/// The blocks for one file, plus how many trailing lines the render cap ate.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Rendered {
    pub blocks: Vec<Block>,
    pub dropped: usize,
    /// How many of the dropped rows were actual changes rather than context.
    ///
    /// The cap fills in document order, so a file whose changes are scattered
    /// through it can exhaust the budget early and lose changes at the end.
    /// Saying "too long to render" would describe that as mere overflow; a
    /// reader needs to know the screen is hiding edits, not just lines.
    pub dropped_changes: usize,
}

/// Parse a unified patch into numbered lines.
///
/// Everything before the first `@@` is skipped rather than pattern-matched,
/// which is what lets one parser take both preambles in play: upstream's
/// `jsdiff` header (`Index:`, a rule of `=`, `--- <file>\t`, `+++ <file>\t`)
/// and the git-shaped `--- a/… +++ b/…` the personal-ai-setup mock writes.
pub(crate) fn parse(patch: &str) -> Vec<DiffLine> {
    let mut out: Vec<DiffLine> = Vec::new();
    let (mut old_no, mut new_no) = (0_u32, 0_u32);
    let mut in_hunk = false;

    for raw in patch.lines() {
        if let Some(header) = raw.strip_prefix("@@") {
            if let Some((old, new)) = hunk_start(header) {
                old_no = old;
                new_no = new;
            }
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        // "\ No newline at end of file" is a note about the line above, not a
        // line of its own — and the patch emits it after *both* halves of a
        // changed last line, so it can land on either.
        if raw.starts_with('\\') {
            if let Some(last) = out.last_mut() {
                last.no_newline = true;
            }
            continue;
        }
        // Indexing at 1 is safe: every arm below matched a one-byte ASCII
        // prefix. An unrecognised prefix is treated as context and kept
        // whole, so a payload change degrades visibly instead of silently
        // dropping lines out of a review.
        let (kind, text) = match raw.as_bytes().first() {
            Some(b'+') => (LineKind::Add, &raw[1..]),
            Some(b'-') => (LineKind::Del, &raw[1..]),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            _ => (LineKind::Context, raw),
        };
        let (old_at, new_at) = match kind {
            LineKind::Add => {
                new_no += 1;
                (0, new_no)
            }
            LineKind::Del => {
                old_no += 1;
                (old_no, 0)
            }
            LineKind::Context => {
                old_no += 1;
                new_no += 1;
                (old_no, new_no)
            }
        };
        out.push(DiffLine {
            kind,
            old_no: old_at,
            new_no: new_at,
            text: text.to_owned(),
            no_newline: false,
        });
    }
    out
}

/// The line numbers a `@@ -a,b +c,d @@` header starts at, as the number
/// *before* the first line of the hunk (so the walk above can pre-increment).
fn hunk_start(header: &str) -> Option<(u32, u32)> {
    let (mut old, mut new) = (None, None);
    for token in header.split_whitespace() {
        if let Some(rest) = token.strip_prefix('-') {
            if old.is_none() {
                old = start_of(rest);
            }
        } else if let Some(rest) = token.strip_prefix('+') {
            if new.is_none() {
                new = start_of(rest);
            }
        }
    }
    // An added file's header is `@@ -0,0 +1,N @@`: saturating, not
    // `- 1`, so the zero side stays at zero.
    Some((old?.saturating_sub(1), new?.saturating_sub(1)))
}

fn start_of(range: &str) -> Option<u32> {
    range.split(',').next()?.parse().ok()
}

/// The runs of unchanged lines worth collapsing.
///
/// A run of `2 * CONTEXT` or fewer is left alone: the band that would hide it
/// is itself a row, so hiding four lines behind it saves three and costs the
/// reader a tap.
pub(crate) fn gaps(lines: &[DiffLine]) -> Vec<Gap> {
    let mut keep = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if line.changed() {
            let lo = i.saturating_sub(CONTEXT);
            let hi = (i + CONTEXT + 1).min(lines.len());
            for slot in keep.iter_mut().take(hi).skip(lo) {
                *slot = true;
            }
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if keep[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && !keep[i] {
            i += 1;
        }
        if i - start > 2 * CONTEXT {
            out.push(Gap {
                start,
                len: i - start,
            });
        }
    }
    out
}

/// Lay a file out: rows, collapsed bands, and the render cap.
///
/// `expanded` maps a gap's `start` to how many of its lines the reader has
/// asked for; those are given back from the band's two ends, so the context
/// grows towards the changes either side of it.
pub(crate) fn blocks(
    lines: &[DiffLine],
    gaps: &[Gap],
    expanded: &HashMap<usize, usize>,
) -> Rendered {
    let mut blocks = Vec::new();
    let mut budget = RENDER_CAP;
    let mut cursor = 0_usize;

    for gap in gaps {
        let revealed = expanded.get(&gap.start).copied().unwrap_or(0).min(gap.len);
        let top = revealed.div_ceil(2);
        let hidden_start = gap.start + top;
        let hidden_end = gap.start + gap.len - (revealed - top);
        if hidden_end <= hidden_start {
            continue; // fully revealed: its lines fall into the run around it
        }
        take_rows(&mut blocks, &mut cursor, hidden_start, &mut budget);
        if budget == 0 {
            break;
        }
        blocks.push(Block::Gap {
            key: gap.start,
            hidden: hidden_end - hidden_start,
            at: lines[hidden_start].new_no,
        });
        cursor = hidden_end;
    }
    take_rows(&mut blocks, &mut cursor, lines.len(), &mut budget);

    Rendered {
        blocks,
        dropped: lines.len() - cursor,
        dropped_changes: lines[cursor..]
            .iter()
            .filter(|l| l.kind != LineKind::Context)
            .count(),
    }
}

fn take_rows(out: &mut Vec<Block>, cursor: &mut usize, end: usize, budget: &mut usize) {
    if end <= *cursor {
        return;
    }
    let take = (end - *cursor).min(*budget);
    if take > 0 {
        out.push(Block::Rows {
            start: *cursor,
            end: *cursor + take,
        });
        *cursor += take;
        *budget -= take;
    }
}

/// How much of a band one tap gives back. A band worth two steps or less
/// opens in one, rather than making the reader tap twice for 30 lines.
pub(crate) const fn expand_to(revealed: usize, hidden: usize) -> usize {
    if hidden <= 2 * EXPAND_STEP {
        revealed + hidden
    } else {
        revealed + EXPAND_STEP
    }
}

/// FNV-1a over a patch, so "reviewed" can be pinned to the exact bytes that
/// were reviewed: the agent touching the file again changes the patch and
/// the mark clears itself, which is the rule `GitHub`'s "Viewed" checkbox
/// uses.
///
/// Hand-rolled rather than `DefaultHasher` because this is persisted in
/// `CodeCache` and has to survive an app restart — `DefaultHasher`'s output
/// is explicitly not stable across releases.
pub(crate) fn fingerprint(patch: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in patch.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Split a path into the part that may be truncated and the part that may
/// not — the filename is what a reader scans for, the directory is context.
pub(crate) fn split_path(path: &str) -> (&str, &str) {
    path.rfind('/')
        .map_or(("", path), |i| (&path[..=i], &path[i + 1..]))
}

#[cfg(test)]
mod tests {
    use super::{
        blocks, fingerprint, gaps, parse, split_path, Block, DiffLine, Gap, LineKind, RENDER_CAP,
    };
    use std::collections::HashMap;

    /// Byte-exact `jsdiff@8.0.2` `formatPatch` output, which is what
    /// `Snapshot.diffFull` produces: four header lines, then one hunk.
    const JSDIFF: &str = "Index: src/app.ts\n\
        ===================================================================\n\
        --- src/app.ts\t\n\
        +++ src/app.ts\t\n\
        @@ -1,5 +1,6 @@\n\
        \x20line one\n\
        -line two\n\
        +CHANGED\n\
        \x20line three\n\
        \x20line four\n\
        \x20line five\n\
        +line six\n";

    /// What the personal-ai-setup mock server writes: a git-shaped preamble
    /// and a short hunk. Different header, same parser.
    const GIT_SHAPED: &str = "--- a/README.md\n\
        +++ b/README.md\n\
        @@ -1 +1,3 @@\n\
        -old\n\
        +new line one\n\
        +new line two\n\
        +new line three\n";

    fn kinds(lines: &[DiffLine]) -> Vec<LineKind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn skips_the_jsdiff_preamble_and_numbers_both_sides() {
        let lines = parse(JSDIFF);
        assert_eq!(
            kinds(&lines),
            vec![
                LineKind::Context,
                LineKind::Del,
                LineKind::Add,
                LineKind::Context,
                LineKind::Context,
                LineKind::Context,
                LineKind::Add,
            ]
        );
        assert_eq!(lines[0].text, "line one");
        // The removed line keeps its old number and has no new one; the
        // added line the other way round.
        assert_eq!((lines[1].old_no, lines[1].new_no), (2, 0));
        assert_eq!((lines[2].old_no, lines[2].new_no), (0, 2));
        // Context after the change is numbered differently on each side.
        assert_eq!((lines[3].old_no, lines[3].new_no), (3, 3));
        assert_eq!((lines[6].old_no, lines[6].new_no), (0, 6));
    }

    #[test]
    fn takes_a_git_shaped_preamble_too() {
        let lines = parse(GIT_SHAPED);
        assert_eq!(
            kinds(&lines),
            vec![LineKind::Del, LineKind::Add, LineKind::Add, LineKind::Add]
        );
        assert_eq!(lines[0].text, "old");
        assert_eq!(lines[3].new_no, 3);
    }

    #[test]
    fn an_added_file_starts_the_new_side_at_one() {
        let lines = parse("@@ -0,0 +1,2 @@\n+first\n+second\n");
        assert_eq!(lines[0].new_no, 1);
        assert_eq!(lines[1].new_no, 2);
        assert_eq!(lines[0].old_no, 0);
    }

    #[test]
    fn a_deleted_file_starts_the_old_side_at_one() {
        let lines = parse("@@ -1,2 +0,0 @@\n-first\n-second\n");
        assert_eq!((lines[0].old_no, lines[0].new_no), (1, 0));
        assert_eq!((lines[1].old_no, lines[1].new_no), (2, 0));
    }

    #[test]
    fn no_newline_marker_annotates_rather_than_becoming_a_row() {
        let lines = parse(
            "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[0].no_newline);
        assert!(lines[1].no_newline);
    }

    #[test]
    fn an_empty_context_line_is_a_line() {
        // jsdiff writes a bare " " for an unchanged empty line; some tools
        // strip the trailing space and write nothing at all.
        let lines = parse("@@ -1,3 +1,3 @@\n a\n\n-b\n+c\n");
        assert_eq!(kinds(&lines)[..2], [LineKind::Context, LineKind::Context]);
        assert_eq!(lines[1].text, "");
    }

    #[test]
    fn a_patch_with_no_hunk_yields_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("Index: assets/icon.png\n").is_empty());
    }

    fn synth(changed_at: &[usize], total: usize) -> Vec<DiffLine> {
        (0..total)
            .map(|i| DiffLine {
                kind: if changed_at.contains(&i) {
                    LineKind::Add
                } else {
                    LineKind::Context
                },
                old_no: u32::try_from(i).unwrap_or(0) + 1,
                new_no: u32::try_from(i).unwrap_or(0) + 1,
                text: format!("line {i}"),
                no_newline: false,
            })
            .collect()
    }

    #[test]
    fn a_short_run_of_context_is_not_worth_hiding() {
        // Changes at 0 and 7: everything between them is within CONTEXT of
        // one or the other, so the only gap is the tail.
        assert_eq!(gaps(&synth(&[0, 7], 20)), vec![Gap { start: 11, len: 9 }]);
        // Changes at 0 and 13 leave a run of exactly 2 * CONTEXT between
        // them. Hiding six lines behind a band that is itself a row saves
        // five and costs a tap, so it stays.
        assert_eq!(gaps(&synth(&[0, 13], 40)), vec![Gap { start: 17, len: 23 }]);
    }

    #[test]
    fn a_long_run_of_context_collapses_with_context_either_side() {
        let lines = synth(&[10, 200], 300);
        let gaps = gaps(&lines);
        assert_eq!(
            gaps,
            vec![
                Gap { start: 0, len: 7 },
                Gap {
                    start: 14,
                    len: 183
                },
                Gap {
                    start: 204,
                    len: 96
                },
            ]
        );
        let rendered = blocks(&lines, &gaps, &HashMap::new());
        assert_eq!(
            rendered.blocks,
            vec![
                Block::Gap {
                    key: 0,
                    hidden: 7,
                    at: 1
                },
                Block::Rows { start: 7, end: 14 },
                Block::Gap {
                    key: 14,
                    hidden: 183,
                    at: 15
                },
                Block::Rows {
                    start: 197,
                    end: 204
                },
                Block::Gap {
                    key: 204,
                    hidden: 96,
                    at: 205
                },
            ]
        );
        assert_eq!(rendered.dropped, 0);
    }

    #[test]
    fn expanding_a_band_gives_lines_back_from_both_ends() {
        let lines = synth(&[10, 200], 300);
        let gaps = gaps(&lines);
        let expanded = HashMap::from([(14_usize, 20_usize)]);
        let rendered = blocks(&lines, &gaps, &expanded);
        // 10 back at the top, 10 at the bottom; the key does not move.
        assert!(rendered.blocks.contains(&Block::Rows { start: 7, end: 24 }));
        assert!(rendered.blocks.contains(&Block::Gap {
            key: 14,
            hidden: 163,
            at: 25
        }));
        assert!(rendered.blocks.contains(&Block::Rows {
            start: 187,
            end: 204
        }));
    }

    #[test]
    fn a_fully_expanded_band_disappears() {
        let lines = synth(&[10, 200], 300);
        let gaps = gaps(&lines);
        let expanded = HashMap::from([(14_usize, 1000_usize)]);
        let rendered = blocks(&lines, &gaps, &expanded);
        assert!(!rendered
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Gap { key: 14, .. })));
        assert!(rendered
            .blocks
            .contains(&Block::Rows { start: 7, end: 204 }));
    }

    #[test]
    fn the_render_cap_truncates_and_says_how_much_it_took() {
        // An added file: every line is a change, so nothing collapses.
        let total = RENDER_CAP + 250;
        let lines = synth(&(0..total).collect::<Vec<_>>(), total);
        let rendered = blocks(&lines, &gaps(&lines), &HashMap::new());
        assert_eq!(
            rendered.blocks,
            vec![Block::Rows {
                start: 0,
                end: RENDER_CAP
            }]
        );
        assert_eq!(rendered.dropped, 250);
    }

    #[test]
    fn a_patch_with_no_changes_at_all_collapses_whole() {
        let lines = synth(&[], 50);
        let rendered = blocks(&lines, &gaps(&lines), &HashMap::new());
        assert_eq!(
            rendered.blocks,
            vec![Block::Gap {
                key: 0,
                hidden: 50,
                at: 1
            }]
        );
    }

    #[test]
    fn the_mark_is_pinned_to_the_bytes_that_were_reviewed() {
        assert_eq!(fingerprint(JSDIFF), fingerprint(JSDIFF));
        assert_ne!(fingerprint(JSDIFF), fingerprint(GIT_SHAPED));
        assert_ne!(fingerprint(""), fingerprint(" "));
    }

    #[test]
    fn the_filename_is_split_off_the_directory() {
        assert_eq!(split_path("src/views/code.rs"), ("src/views/", "code.rs"));
        assert_eq!(split_path("README.md"), ("", "README.md"));
    }
}
