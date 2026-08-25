pub(crate) mod chat;
pub(crate) mod code;
pub(crate) mod session_settings;
pub(crate) mod sessions;
pub(crate) mod settings;

use dioxus::prelude::*;

use crate::icons::Icon;
use crate::state::{AppCtx, ConnState};

/// Small colored dot + label reflecting the connection state.
#[component]
pub fn ConnBadge() -> Element {
    let ctx: AppCtx = crate::state::use_app_ctx();
    let (class, label) = match (ctx.conn)() {
        ConnState::Disconnected => ("dot off", "offline".to_string()),
        ConnState::Connecting => ("dot busy", "connecting…".to_string()),
        ConnState::Connected { agent } => ("dot on", agent),
        ConnState::Failed(_) => ("dot err", "error".to_string()),
    };
    rsx! {
        span { class: "conn-badge",
            span { class: "{class}" }
            span { class: "conn-label", "{label}" }
        }
    }
}

/// The tray behind a session row, revealed by dragging the row to the left.
///
/// It is the last item in the row's horizontal scroller, so at rest it sits
/// past the card's right edge and is clipped away entirely — there is no
/// always-visible destructive control on a list you scroll with your thumb.
/// The tap stops propagating because the row itself is the tap target
/// (design rule 9), and "Delete" is not "open".
#[component]
pub fn SwipeDelete(on_delete: EventHandler<()>) -> Element {
    rsx! {
        div { class: "session-actions",
            button {
                class: "swipe-action danger",
                onclick: move |e: Event<MouseData>| {
                    e.stop_propagation();
                    on_delete.call(());
                },
                Icon { name: "trash" }
                "Delete"
            }
        }
    }
}

/// The confirmation a swipe earns, as a sheet rather than a row in the card.
///
/// Both planes delete for good: goose's `session/delete` is not a soft
/// delete, and the code plane purges the container and the workspace with the
/// branch still in it. With no undo to offer, a drag on its own is not
/// consent. That is Messages' arrangement rather than Mail's — Mail can
/// afford a one-tap swipe because it has a Trash to fish things back out of.
#[component]
pub fn ConfirmDelete(
    title: String,
    body: String,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal",
                h2 { "{title}" }
                p { class: "modal-body", "{body}" }
                div { class: "modal-actions",
                    button {
                        class: "btn secondary",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "btn danger",
                        onclick: move |_| on_confirm.call(()),
                        "Delete"
                    }
                }
            }
        }
    }
}

/// One entry in an overflow menu.
#[derive(Clone, PartialEq)]
pub(crate) struct MenuItem {
    pub icon: &'static str,
    pub label: &'static str,
    /// Destructive, and coloured as such.
    pub danger: bool,
}

/// The `⋯` in the top bar.
///
/// Only the button: the sheet it opens is [`OverflowSheet`], rendered at the
/// view's root beside the other overlays. They cannot be one component,
/// because the bar's controls carry `backdrop-filter` — and a filtered element
/// becomes the containing block for every `position: fixed` descendant, so a
/// sheet rendered in here is trapped inside a 94px pill in the corner instead
/// of covering the screen. The same property is why `.app` deliberately avoids
/// a transform.
#[component]
pub fn OverflowButton(onopen: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "icon-btn",
            title: "More",
            onclick: move |_| onopen.call(()),
            Icon { name: "more" }
        }
    }
}

/// What the `⋯` opens. Render this at the root of a view, not inside the bar.
#[component]
pub fn OverflowSheet(
    items: Vec<MenuItem>,
    onpick: EventHandler<usize>,
    onclose: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| onclose.call(()),
            div {
                // `menu` as well as `sheet`: it is the same pane, but the
                // capture harness has to be able to tell an overflow menu from
                // the settings sheet, or they file under one gallery state and
                // whichever was seen last wins.
                class: "modal sheet menu",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                div { class: "setting-list",
                    for (i, item) in items.iter().enumerate() {
                        button {
                            key: "{item.label}",
                            class: if item.danger { "setting-row danger" } else { "setting-row" },
                            onclick: move |_| onpick.call(i),
                            Icon { name: "{item.icon}" }
                            span { class: "setting-main",
                                span { class: "setting-name", "{item.label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
