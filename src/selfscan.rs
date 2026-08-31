//! Reading a source file for something no compiler can check — without the
//! read answering itself.
//!
//! Several rules in this app are one decision held in two languages: Rust
//! writes an attribute or a class name and a stylesheet is the only thing that
//! reads it, so a rename on either side leaves a control that visibly does
//! nothing and nothing in the compiler says a word. The tests that hold those
//! rules together have to read source for the half the compiler cannot see,
//! and the obvious way to do that is `include_str!` of the file the test is
//! written in.
//!
//! IT DOES NOT WORK, AND IT FAILS OPEN. `include_str!("desktop.rs")` from
//! inside `desktop.rs` includes that file's own `mod tests`, so the needle is
//! supplied by the assertion's own source line and the check is green whether
//! or not the feature it names still exists. Measured on this tree at
//! d716047: with `use_fullscreen`, its call site and the `data-fullscreen`
//! attribute all deleted — the whole feature, nothing left of it but an
//! orphaned doc comment — `cargo test --package goose-mobile` reported
//! `231 passed; 0 failed`, and the two assertions written to catch exactly
//! that deletion were two of the ones that passed.
//!
//! `src/views/code.rs` had the same hole from the other end. Its check counts
//! how many times a screen's name appears in the file and wants two — one for
//! the window's bar, one for the pane's own heading — and the test's own
//! `"Review".to_owned()` was one of the two, so deleting the heading left it
//! green with a count of two.
//!
//! So a scan of a file's own source goes through [`code_of`], which returns
//! the part of that file the tests did not write and asserts that what it
//! returns holds no test code at all. The post-condition is the mechanism: it
//! is what makes the answer come from the shell rather than from the question,
//! and it goes on holding when someone moves a test module, adds a second one,
//! or writes a doc comment that quotes the attribute it is documenting.
//!
//! REJECTED, and worth writing down because it is the stronger check on paper:
//! asserting against RENDERED markup instead of source. There are two ways to
//! get some in this tree and neither answers this question.
//! `docs/gallery-states.json` is real markup dumped out of the running app and
//! `src/views/chrome.rs` reads it — but it is a CAPTURE, and "the shell has
//! stopped rendering this" is precisely the change a capture cannot show: the
//! markup goes on saying what the app said on the day it was driven. The store
//! in the tree is proof of that on its own. Every desktop state in it carries
//! `data-fullscreen` on `.app`, where the JS heuristic used to write it; the
//! render has been writing it on `.shell` since ece4857, one commit after the
//! capture, and nothing has gone red about it because a stale answer is still
//! an answer.
//! Rendering in-process is the other, and it needs a `DesktopContext` —
//! `dioxus::desktop::window()` is `consume_context()`
//! (`dioxus-desktop-0.7.10/src/desktop_context.rs:34`), so `use_fullscreen`
//! panics without a real event loop — and an `AppCtx` with a live storage
//! provider under it. Faking both renders a component that is not the one that
//! ships, which is this same bug one level up.
//!
//! `#[cfg(test)]` at the declaration in `src/main.rs`, so nothing of this is
//! in any binary.

/// The code in `source` that no test in `source` wrote: everything above the
/// test module, minus the comment lines.
///
/// `file` names the file in the failure messages and is not read. `source` is
/// meant to arrive from an `include_str!` at the CALL SITE rather than from a
/// path composed out of `CARGO_MANIFEST_DIR`, which is the other way this repo
/// reads files in tests (`src/shell/mod.rs`, `src/views/chrome.rs`).
/// `include_str!` resolves at compile time, so a file that is renamed or moved
/// out from under one of these scans fails to BUILD; a path composed at run
/// time fails as "cannot read", one assertion deep, on a machine where the
/// rest of the suite has already gone green.
pub(crate) fn code_of(file: &str, source: &str) -> String {
    // The cut. `#[cfg(test)]` is where the test module starts in every file in
    // this crate and it appears exactly once in each of them — checked, in
    // `src/views/code.rs` and `src/shell/desktop/mod.rs`, which are the two
    // callers. A file with no test module at all is not an error: the whole of
    // it is then code, which is what `map_or` hands back, and the
    // post-condition below is what decides whether that was true.
    let above = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(code, _)| code);

    // Comments go too, and that is not tidiness. This file's callers assert
    // that the shell still renders `"data-fullscreen": if fullscreen()`; the
    // doc comment three lines above that attribute is about the attribute, and
    // one day someone writing prose will quote it. Dropping comment lines is
    // what keeps a scan of the code a scan of the code, and it is the habit
    // `src/shell/mod.rs`'s `no_cfg_reads_the_dx_marker_feature` already has,
    // for the same reason stated there: "comment lines are dropped below so
    // the prose above is free to say it plainly".
    let code: String = above
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    // THE POST-CONDITION, and the whole reason to call this rather than
    // `include_str!` directly.
    //
    // The cut above is one heuristic about where a test module starts. This is
    // the guarantee, and it holds however the file is arranged: a `#[test]` fn
    // written above the code, a second test module, a test module reached by
    // some other attribute, a `#[cfg(test)]` that moved. Any of them leaves
    // test source in the region and fails HERE, loudly, instead of quietly
    // going back to a suite that cannot fail.
    assert!(
        !code.contains("#[test]"),
        "{file}: the region this scan reads still contains test code, so an \
         assertion in it can supply its own needle and pass with the feature \
         deleted. That is the bug this function exists to make impossible — \
         see the module comment for the measurement."
    );
    // And a scan that matched nothing at all would pass forever. The floor is
    // `src/shell/mod.rs`'s habit: say out loud that the fixture was found.
    assert!(
        code.contains("fn "),
        "{file}: the region above the test module holds no code — every claim \
         made about it below would be a claim about an empty string"
    );
    code
}

#[cfg(test)]
mod tests {
    use super::code_of;

    /// The bug, in miniature, on a fixture rather than on a real file.
    ///
    /// A source whose test module names a thing its code no longer does — the
    /// exact shape `src/shell/desktop/mod.rs` shipped — and the assertion is that
    /// the scan does not hand it back. Written as a fixture because the real
    /// files are supposed to be correct, so a test that read one of them could
    /// only ever say "still fine": this one can say "the mechanism works",
    /// which is a different claim and the one that has to survive.
    #[test]
    fn the_scan_cannot_be_answered_by_the_test_module() {
        let source = concat!(
            "fn shell() {}\n",
            "\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    #[test]\n",
            "    fn t() { assert!(s.contains(\"data-fullscreen\")); }\n",
            "}\n",
        );
        assert!(source.contains("data-fullscreen"), "the fixture is the bug");
        assert!(
            !code_of("fixture.rs", source).contains("data-fullscreen"),
            "the needle came back out of the test module, which is the whole \
             defect this function exists to close"
        );
    }

    /// Prose is not code. The doc comment above an attribute is about that
    /// attribute and will quote it eventually; a scan that reads comments is a
    /// scan a comment can satisfy.
    #[test]
    fn a_comment_cannot_answer_a_question_about_code() {
        let source = "/// sets \"data-nav\" on the shell\nfn shell() {}\n";
        assert!(!code_of("fixture.rs", source).contains("data-nav"));
    }

    /// The cut is a heuristic; the post-condition is the guarantee. A file
    /// that puts its tests first, or reaches them by an attribute this does
    /// not split on, has to fail loudly here rather than quietly go back to
    /// handing assertions their own source.
    ///
    /// `should_panic` on the MESSAGE and not on the panic alone, because the
    /// two asserts in `code_of` fail for opposite reasons and a test that
    /// accepted either would be satisfied by the wrong one.
    #[test]
    #[should_panic(expected = "supply its own needle")]
    fn test_code_left_in_the_region_is_a_failure_and_not_a_scan() {
        let _ = code_of("fixture.rs", "#[test]\nfn t() {}\nfn shell() {}\n");
    }

    /// The other half of that: a cut that landed on nothing says so rather
    /// than reporting a clean scan of an empty string. The floor is
    /// `src/shell/mod.rs`'s `assert!(scanned > 10, …)` habit.
    #[test]
    #[should_panic(expected = "holds no code")]
    fn a_region_with_no_code_in_it_is_not_a_clean_scan() {
        let _ = code_of("fixture.rs", "// all prose\n#[cfg(test)]\nmod tests {}\n");
    }
}
