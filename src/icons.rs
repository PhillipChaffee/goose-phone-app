//! Monochrome line icons.
//!
//! These used to be Unicode glyphs — `⚙`, `🗑`, `📄` and friends. On Linux
//! they resolved to whatever mono glyph the fallback font had and looked
//! plausible; on iOS every one of them has emoji presentation, so the real
//! device rendered a shiny skeuomorphic gear and a 3D wastebasket in the
//! middle of a flat, monochrome UI. Variation selector 15 does not rescue
//! them either: most of these codepoints have no text presentation at all.
//!
//! Drawn instead, at a 24-unit grid with a 2-unit stroke, inheriting
//! `currentColor` so every existing colour rule keeps working.

use dioxus::prelude::*;

/// Stroke path data for `name`, or `None` if there is no such icon.
pub(crate) fn path_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "gear" => "M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z\
                   M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
        "trash" => "M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m3 0v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V6M10 11v6M14 11v6",
        "refresh" => "M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6",
        "home" => "M3 10.5 12 3l9 7.5M5.5 9.5V20a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1V9.5",
        "code" => "m8 6-6 6 6 6M16 6l6 6-6 6",
        "chevron-left" => "m15 5-7 7 7 7",
        "chevron-right" => "m9 5 7 7-7 7",
        "plus" => "M12 5v14M5 12h14",
        "chevron-down" => "m6 9 6 6 6-6",
        "check" => "m5 13 4 4 10-10",
        "close" => "M6 6l12 12M18 6 6 18",
        "terminal" => "m4 7 5 5-5 5M13 17h7",
        "file" => "M14 3H7a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V7zM14 3v4h4M9 13h6M9 17h4",
        "pencil" => "M4 20h4L19.5 8.5a2.12 2.12 0 0 0-3-3L5 17zM14.5 6.5l3 3",
        "package" => "m12 3 8 4.5v9L12 21l-8-4.5v-9zM4 7.5l8 4.5 8-4.5M12 12v9",
        "search" => "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
        "globe" => "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM3 12h18M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18z",
        "think" => "M9 18h6M10 21h4M12 3a6 6 0 0 0-3.6 10.8c.5.4.8.9.9 1.2h5.4c.1-.3.4-.8.9-1.2A6 6 0 0 0 12 3z",
        "menu" => "M4 7h16M4 12h11M4 17h16",
        "message" => "M21 11.5a8.4 8.4 0 0 1-9 8.4 9 9 0 0 1-3.9-.9L3 21l1.9-5.1A8.4 8.4 0 0 1 4 11.5 8.4 8.4 0 0 1 12.5 3 8.4 8.4 0 0 1 21 11.5z",
        "cloud" => "M18 17a4 4 0 0 0 0-8 6 6 0 0 0-11.7 1.6A3.7 3.7 0 0 0 7 17z",
        "stop" => "M7 7h10v10H7z",
        "arrow-up" => "M12 20V4M5 11l7-7 7 7",
        "wrench" => "M14.7 6.3a4 4 0 0 0 5 5l-9.4 9.4a2.1 2.1 0 0 1-3-3z",
        "diff" => "M12 5v10M7 10h10M7 19h10",
        // Three dots need three subpaths of zero length; a round cap turns
        // each into a disc, so this stays one stroked path like the rest.
        "more" => "M6 12h.01M12 12h.01M18 12h.01",
        "arrow-down" => "M12 4v16M5 13l7 7 7-7",
        // A line that runs on, turns back and points home: the soft-wrap
        // toggle on the review screen.
        "wrap-text" => "M4 6h16M4 12h13a3 3 0 1 1 0 6h-5M14 16l-2 2 2 2M4 18h7",
        "pull-request" => "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 9v12M18 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 15V9a3 3 0 0 0-3-3h-4m0 0 3-3m-3 3 3 3",
        // The mode chip's own family (views/session_settings.rs `mode_icon`).
        // A bolt is the generic one — it is what the reference app puts on
        // "Auto" — and the rest say what that mode does instead of it.
        "bolt" => "M13 3 4 14h7l-1 7 9-11h-7z",
        "shield-check" => "M12 21c4.7-2.3 7-5.6 7-9.6V5.6L12 3 5 5.6V11.4c0 4 2.3 7.3 7 9.6zM9.2 11.6l2 2 3.6-3.6",
        "list" => "M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01",
        // The repository pill on the new-session screen. GitHub's own mark is
        // a logo and this app draws none; a repo is a book everywhere else in
        // the tooling, and the shape survives at 12px.
        "repo" => "M6 3h11a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H6a2 2 0 0 1 0-4h12",
        // Two commits and the cut between them — the base-branch pill. Same
        // circle idiom as "pull-request" above, so the two read as a family.
        "git-branch" => "M6 3v12M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 9a9 9 0 0 1-9 9",
        // Appended for the recipes/skills/scheduler/extensions screens, in
        // the order they were needed rather than sorted into the set above —
        // a re-sort is a diff over every line for the sake of alphabetical
        // order nobody reads.
        "book" => "M4 4.5A1.5 1.5 0 0 1 5.5 3H19v14H5.5A1.5 1.5 0 0 0 4 18.5zM4 18.5A1.5 1.5 0 0 0 5.5 20H19v-3",
        "clock" => "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3.5 2",
        "play" => "M8 5.5v13l11-6.5z",
        "pause" => "M9 5v14M15 5v14",
        // Four strokes crossing at their midpoints: the "this was generated"
        // mark, at a size where a literal star would be a blob.
        // The mockups' 2x2 disclosure glyph, rescaled from their 16-unit grid
        // to this file's 24-unit one. Nothing here was close: `archive` is a
        // box with a lid, `package` a cube, `list` three rules.
        "grid" => "M3.5 3.5h6.5v6.5H3.5zM14 3.5h6.5v6.5H14zM3.5 14h6.5v6.5H3.5zM14 14h6.5v6.5H14z",
        "sparkle" => "M12 3v6M12 15v6M3 12h6M15 12h6M6.5 6.5 9 9M15 15l2.5 2.5M17.5 6.5 15 9M9 15l-2.5 2.5",
        "archive" => "M3 5h18v4H3zM5 9v10a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V9M9.5 13h5",
        // The desktop nav's collapse control: a pane with its leading column
        // ruled off. One glyph for both directions, because the button is a
        // toggle and not two buttons — which side is "the panel" never
        // changes, and an icon that flips is an icon you have to read. It is
        // the same mark goose's own desktop app uses for this exact control
        // (`PanelLeft`, ui/desktop/src/components/Layout/AppLayout.tsx).
        "sidebar" => "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM9.5 5v14",
        _ => return None,
    })
}

/// A 1em-square line icon that takes its colour from the surrounding text.
#[component]
pub(crate) fn Icon(name: String) -> Element {
    let Some(d) = path_for(&name) else {
        return rsx! {};
    };
    rsx! {
        svg {
            class: "icon",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: "{d}" }
        }
    }
}
