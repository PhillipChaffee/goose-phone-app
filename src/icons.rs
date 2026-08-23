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
fn path_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "gear" => "M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z\
                   M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
        "trash" => "M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m3 0v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V6M10 11v6M14 11v6",
        "refresh" => "M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6",
        "home" => "M3 10.5 12 3l9 7.5M5.5 9.5V20a1 1 0 0 0 1 1h11a1 1 0 0 0 1-1V9.5",
        "code" => "m8 6-6 6 6 6M16 6l6 6-6 6",
        "chevron-left" => "m15 5-7 7 7 7",
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
        "pull-request" => "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 9v12M18 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 15V9a3 3 0 0 0-3-3h-4m0 0 3-3m-3 3 3 3",
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
