pub(crate) mod chat;
pub(crate) mod code;
pub(crate) mod connect;
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
