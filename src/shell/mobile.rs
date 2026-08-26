//! The phone shell: one screen at a time, with the nav as a panel over it.
//!
//! Everything in this file is `src/app.rs` as it stood before the desktop
//! shell existed, moved rather than rewritten. That is deliberate and it is
//! the point: the promise this branch makes is that mobile rendering does not
//! change, and a move is a promise a reviewer can check by reading the diff.
//! Dioxus components emit no element of their own, so `.app`'s children are
//! the same three nodes in the same order they have always been — the screen,
//! the scrim, the panel.
//!
//! The two gesture hooks live here rather than in `app.rs` so that "no swipe
//! tray and no pull-to-refresh on the desktop" is structural instead of
//! accidental. `PULL_TO_REFRESH` appends a spinner to `document.body` and
//! installs four touch listeners; none of that belongs in a Mac window, and
//! the way to be sure it is absent is for the code that installs it to not be
//! compiled.

use dioxus::prelude::*;

use crate::nav::{self, Group};
use crate::shell::render_group;

/// The phone shell: the destination on top of the stack, and the drawer.
#[component]
pub(crate) fn AppShell() -> Element {
    let ctx = crate::state::use_app_ctx();

    crate::viewport::use_close_open_row();
    crate::viewport::use_pull_to_refresh();

    let body = (nav::current(&ctx).view)(&ctx);

    rsx! {
        {body}
        Drawer {}
    }
}

/// The navigation drawer.
///
/// It replaced a bottom tab bar, which spent 100px of every screen on two
/// destinations and left no room for a third. The destinations themselves
/// live in `nav::DESTINATIONS`; this walks them in group order, so a feature
/// appears in the drawer by adding a row to that table and touching nothing
/// here.
#[component]
fn Drawer() -> Element {
    let ctx = crate::state::use_app_ctx();
    let mut open = ctx.drawer_open;

    rsx! {
        div {
            class: if open() { "drawer-scrim open" } else { "drawer-scrim" },
            onclick: move |_| open.set(false),
        }
        aside { class: if open() { "drawer open" } else { "drawer" },
            h2 { class: "drawer-brand", "goose" }
            nav { class: "drawer-nav",
                for group in Group::ALL {
                    {render_group(&ctx, group)}
                }
            }
        }
    }
}
