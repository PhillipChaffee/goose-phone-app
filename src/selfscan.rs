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

/// The attribute that marks an item as test code.
pub(crate) const TEST_ONLY: &str = "#[cfg(test)]";

/// The code in `source` that no test in `source` wrote: everything outside the
/// `#[cfg(test)]` items, minus the comment lines.
///
/// `file` names the file in the failure messages and is not read. `source` is
/// meant to arrive from an `include_str!` at the CALL SITE rather than from a
/// path composed out of `CARGO_MANIFEST_DIR`, which is the other way this repo
/// reads files in tests (`src/shell/mod.rs`, `src/views/chrome.rs`).
/// `include_str!` resolves at compile time, so a file that is renamed or moved
/// out from under one of these scans fails to BUILD; a path composed at run
/// time fails as "cannot read", one assertion deep, on a machine where the
/// rest of the suite has already gone green.
///
/// NOT A TRUNCATION, and that is #225. This used to cut the file at its FIRST
/// `#[cfg(test)]` and keep everything above, on the reading that the first one
/// is where the test module starts. It is, in 31 of the 36 files under `src/`
/// that have one — and in the other five the first `#[cfg(test)]` is a
/// test-only item near the top. `src/views/mod.rs:8` is
/// `#[cfg(test)] pub(crate) mod press;`, so the truncation kept **7 of 552
/// lines** and threw away `ScrollToBottom`, `Confirm` and the shared row
/// chrome; the `fn ` floor below then fired, and the whole file was unscannable
/// by anything built on this helper. Measured on the pass that produced #222:
/// deriving the desktop's inherited surface through the truncation lost
/// `.scroll-bottom` (which is #209), `.scroll-bottom-slot` and `.modal-body`.
///
/// So the cut REMOVES the test items and keeps the code between and after
/// them, through [`without_items_under`], which is the same line-based walk
/// `src/inherit.rs` already had to write for itself to get past this.
pub(crate) fn code_of(file: &str, source: &str) -> String {
    // Comments go first, and that is not tidiness. This file's callers assert
    // that the shell still renders `"data-fullscreen": if fullscreen()`; the
    // doc comment three lines above that attribute is about the attribute, and
    // one day someone writing prose will quote it. Dropping comment lines is
    // what keeps a scan of the code a scan of the code, and it is the habit
    // `src/shell/mod.rs`'s `no_cfg_reads_the_dx_marker_feature` already has,
    // for the same reason stated there: "comment lines are dropped below so
    // the prose above is free to say it plainly".
    //
    // BEFORE the cut rather than after it, because the cut is line-based: a
    // comment line inside a test module that happens to end in `;` would
    // otherwise be read as the end of the item and let the rest of the module
    // back in.
    let prose_free: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let code = without_items_under(&prose_free, TEST_ONLY);

    // THE CUT'S OWN ASSUMPTION, CHECKED. `without_items_under` matches a whole
    // LINE, so it only ever removes an item written at column 0 — and a
    // `#[cfg(test)]` on a method inside an `impl` block is indented, would be
    // left behind, and would hand this scan test source to answer with. No file
    // under `src/` has one today; this is what says so when one arrives.
    assert!(
        !code.contains(TEST_ONLY),
        "{file}: still carries `{TEST_ONLY}` after the cut, which means it is \
         INDENTED — on a method in an `impl`, or inside another item. The \
         line-based cut cannot see that, so the test code under it is about to \
         be read as shipping code."
    );

    // THE POST-CONDITION, and the whole reason to call this rather than
    // `include_str!` directly.
    //
    // The cut above is one heuristic about where a test item starts and ends.
    // This is the guarantee, and it holds however the file is arranged: a
    // `#[test]` fn written above the code, a second test module, a test module
    // reached by some other attribute, an item whose end the line walk missed.
    // Any of them leaves test source in the region and fails HERE, loudly,
    // instead of quietly going back to a suite that cannot fail.
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
        "{file}: the code outside the test items holds no function at all — \
         every claim made about it below would be a claim about an empty string"
    );
    code
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
/// Stacked attributes come with the item: `src/views/code.rs:5341` is
/// `#[cfg(test)]` followed by a four-line `#[expect(…)]` and then
/// `mod pressing {`, and none of those lines ends an item, so the walk runs on
/// to the module's closing brace.
///
/// Ending EARLY leaves test code behind and [`code_of`]'s post-condition
/// fires. Ending LATE would silently delete real code, which is why
/// `src/inherit.rs`'s `the_scan_agrees_with_the_sheet_about_what_is_the_phones_alone`
/// pins both sides of the phone cut by name.
///
/// Lives here rather than in `src/inherit.rs`, where it was written, because
/// [`code_of`] is the repo's declared way to read a source file for what no
/// compiler can check and this is the cut that makes it work. Two copies of
/// one heuristic is the failure this whole module is about, one level up.
pub(crate) fn without_items_under(code: &str, marker: &str) -> String {
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
    #[should_panic(expected = "holds no function")]
    fn a_region_with_no_code_in_it_is_not_a_clean_scan() {
        let _ = code_of("fixture.rs", "// all prose\n#[cfg(test)]\nmod tests {}\n");
    }

    /// #225, on a fixture in the shape of the file it was found on.
    ///
    /// `src/views/mod.rs` declares `#[cfg(test)] pub(crate) mod press;` at
    /// line 8 and its real test module at line 338, and the truncation this
    /// replaces kept the seven lines above the first one — no `fn `, so the
    /// floor fired and the file could not be scanned at all. What has to
    /// survive is the code BETWEEN and AFTER the test items, which is where
    /// `ScrollToBottom` lives, and with it `.scroll-bottom`: #209.
    #[test]
    fn a_test_only_item_above_the_code_does_not_take_the_code_with_it() {
        let source = concat!(
            "mod attach;\n",
            "\n",
            "#[cfg(test)]\n",
            "pub(crate) mod press;\n",
            "\n",
            "fn scroll_to_bottom() -> &'static str { \"scroll-bottom\" }\n",
            "\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    #[test]\n",
            "    fn t() { assert!(x.contains(\"never-shipped\")); }\n",
            "}\n",
        );
        let code = code_of("fixture.rs", source);
        assert!(
            code.contains("scroll-bottom"),
            "the code below a test-only `mod` declaration went with it, which \
             is #225: {code}"
        );
        assert!(
            !code.contains("press"),
            "the test-only module declaration itself survived the cut: {code}"
        );
        assert!(
            !code.contains("never-shipped"),
            "the second test module came back, so an assertion in it could \
             still answer a question about the shell: {code}"
        );
    }

    /// A `#[cfg(test)]` an `impl` block puts on one method is indented, so the
    /// line walk cannot see where it ends — and everything under it would be
    /// read as shipping code. Nothing in `src/` is written that way today;
    /// this is what says so the day one is.
    #[test]
    #[should_panic(expected = "INDENTED")]
    fn an_indented_test_attribute_is_a_cut_this_walk_cannot_make() {
        let source = concat!(
            "impl Shell {\n",
            "    #[cfg(test)]\n",
            "    fn only_for_tests() {}\n",
            "}\n",
        );
        let _ = code_of("fixture.rs", source);
    }

    /// The cut takes an item's stacked attributes with it. `src/views/code.rs`
    /// writes `#[cfg(test)]` and then a four-line `#[expect(…)]` above
    /// `mod pressing`, and an attribute line ends no item — so the walk has to
    /// run past all of them to the module's own closing brace.
    #[test]
    fn stacked_attributes_belong_to_the_item_under_them() {
        let source = concat!(
            "fn shipped() {}\n",
            "#[cfg(test)]\n",
            "#[expect(\n",
            "    clippy::panic,\n",
            "    reason = \"test scaffolding\"\n",
            ")]\n",
            "mod pressing {\n",
            "    fn helper() {}\n",
            "}\n",
            "fn also_shipped() {}\n",
        );
        let code = code_of("fixture.rs", source);
        assert!(
            !code.contains("pressing") && !code.contains("scaffolding"),
            "the walk stopped at one of the attribute lines and let the test \
             module back in: {code}"
        );
        assert!(
            code.contains("shipped") && code.contains("also_shipped"),
            "the walk ran past the module's closing brace and ate the code \
             after it: {code}"
        );
    }

    /// A file with no test module at all is not an error: the whole of it is
    /// code, and the post-conditions are what decide whether that was true.
    #[test]
    fn a_file_with_no_tests_in_it_is_all_code() {
        let source = "fn shell() {}\nfn other() {}\n";
        assert_eq!(code_of("fixture.rs", source), source.trim_end());
    }
}
