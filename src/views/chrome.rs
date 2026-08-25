//! The chrome every screen is made of: the bar at the top, the rows in the
//! middle, and the field that searches them.
//!
//! Seven hand-written copies of `header.topbar` exist in this app, and the
//! five features arriving after this one would have made twelve. Copies drift
//! — one of them shipped a back chevron that rendered only `if connected`,
//! which sealed a disconnected user into the one screen they were on
//! *because* they were not connected — so the bar is a component with the
//! rules built in rather than a shape to be re-typed.

use std::time::Duration;

use dioxus::prelude::*;

use crate::icons::Icon;
use crate::views::ConnBadge;

/// The floating top bar: a leading control, the title, and whatever the
/// screen puts in the trailing group.
///
/// `on_back: None` means this is a root screen, and the leading control is
/// the drawer's hamburger. Every root screen carries it (design rule 2) —
/// that is not a choice a caller gets to make, because the screen that once
/// made it wrong was unreachable afterwards.
///
/// `children` may hold **controls, never overlays.** The bar's controls carry
/// `backdrop-filter`, and a filtered element becomes the containing block for
/// every `position: fixed` descendant — so a sheet rendered inside the bar is
/// trapped inside a ~94px pill in the corner instead of covering the screen.
/// It is the same property that makes `.app` deliberately avoid a
/// `transform` (see `src/viewport.rs`). Anything a bar button *opens* renders
/// at the view's root, with the button only setting the signal.
#[component]
pub(crate) fn TopBar(
    title: String,
    /// A second line under the title — where the thing lives, what it is part
    /// of. Rendered inside `.titlegroup`, which takes the centre cell exactly
    /// as a lone title does.
    #[props(default)]
    subtitle: Option<String>,
    /// `None` is a root screen: the leading control becomes the hamburger.
    #[props(default)]
    on_back: Option<EventHandler<()>>,
    /// Show the goose connection badge. A prop and not a child because it
    /// sits *between* the title and the trailing group, which is a position
    /// no child list can express.
    #[props(default)]
    conn: bool,
    children: Element,
) -> Element {
    let ctx = crate::state::use_app_ctx();

    rsx! {
        header { class: "topbar",
            if let Some(on_back) = on_back {
                button {
                    class: "icon-btn back",
                    onclick: move |_| on_back.call(()),
                    Icon { name: "chevron-left" }
                }
            } else {
                button {
                    class: "icon-btn menu",
                    onclick: move |_| {
                        let mut open = ctx.drawer_open;
                        open.set(true);
                    },
                    Icon { name: "menu" }
                }
            }
            if let Some(subtitle) = subtitle {
                div { class: "titlegroup",
                    h1 { class: "title ellipsis", "{title}" }
                    span { class: "subtitle ellipsis", "{subtitle}" }
                }
            } else {
                h1 { class: "title ellipsis", "{title}" }
            }
            if conn {
                ConnBadge {}
            }
            div { class: "topbar-actions", {children} }
        }
    }
}

/// What a [`RowAction`] looks like, apart from what it does.
///
/// Split from the handler because an `EventHandler` can only be built inside
/// a live Dioxus runtime, and the house rule is that a decision is tested as
/// plain data rather than by mounting the thing that renders it (see
/// `SettingRow::select`). This is the half a test can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowFace {
    pub label: &'static str,
    pub icon: &'static str,
    pub danger: bool,
}

// `cfg_attr(not(test))` because the tests below do use these: an expectation
// that holds in one cfg and not the other is an error in whichever cfg it
// does not hold in.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no list in the shell has a tray yet; the first one arrives \
                  in PR 3, and this expectation fails then"
    )
)]
impl RowFace {
    /// The one action every list in this app has. Spelled once so four
    /// future lists cannot each invent their own word for it.
    pub(crate) const DELETE: Self = Self {
        label: "Delete",
        icon: "trash",
        danger: true,
    };

    /// A tray button that does not destroy anything.
    pub(crate) const fn plain(label: &'static str, icon: &'static str) -> Self {
        Self {
            label,
            icon,
            danger: false,
        }
    }

    /// Design rule 7 exempts the destructive control that is already being
    /// pressed: it takes the saturated fill, where everything else tints.
    pub(crate) const fn class(self) -> &'static str {
        if self.danger {
            "swipe-action danger"
        } else {
            "swipe-action"
        }
    }
}

/// One button in a row's swipe tray.
#[derive(Clone, PartialEq)]
pub(crate) struct RowAction {
    pub face: RowFace,
    pub on_pick: EventHandler<()>,
}

#[expect(dead_code, reason = "as `RowFace` above")]
impl RowAction {
    pub(crate) const fn new(face: RowFace, on_pick: EventHandler<()>) -> Self {
        Self { face, on_pick }
    }

    /// The row's delete, worded and iconed like every other one.
    pub(crate) const fn delete(on_pick: EventHandler<()>) -> Self {
        Self::new(RowFace::DELETE, on_pick)
    }
}

/// A row in a list: leading tile, title, an optional trailing word, whatever
/// the caller puts under it, and a swipe tray behind it.
///
/// The class names are the `.session-*` set on purpose, and the purpose is
/// not brevity. Three things key off them and would otherwise have to be
/// found and changed together: `src/viewport.rs`'s close-the-open-row
/// handler, `src/domdump.rs`'s `swiped` suffix detector, and
/// `docs/audit.js`'s longest-text stress map. Reusing the class is how four
/// future lists get all three for free. Read `.session-*` as "list row".
///
/// An empty `actions` renders no tray at all, so the row does not move: a row
/// that swipes open onto nothing is worse than one that does not swipe, and
/// two of the five features arriving after this one have nothing destructive
/// to put behind a row.
#[component]
pub(crate) fn ListRow(
    /// Name of the icon in the leading tile.
    icon: String,
    title: String,
    /// The right end of the title line — an age, a count, a status word. It
    /// never wraps and never shrinks.
    #[props(default)]
    trailing: Option<String>,
    #[props(default)] actions: Vec<RowAction>,
    on_open: EventHandler<()>,
    children: Element,
) -> Element {
    rsx! {
        li {
            class: "session-item",
            // The whole row is the tap target (design rule 9); the tray's own
            // buttons stop propagation below.
            onclick: move |_| on_open.call(()),
            div { class: "session-swipe",
                div { class: "session-tile", Icon { name: icon } }
                div { class: "session-main",
                    div { class: "session-head",
                        div { class: "session-title", "{title}" }
                        if let Some(trailing) = trailing {
                            span { class: "session-age", "{trailing}" }
                        }
                    }
                    {children}
                }
            }
            if !actions.is_empty() {
                div { class: "session-actions",
                    for action in actions {
                        button {
                            key: "{action.face.label}",
                            class: action.face.class(),
                            onclick: move |e: Event<MouseData>| {
                                e.stop_propagation();
                                action.on_pick.call(());
                            },
                            Icon { name: action.face.icon }
                            "{action.face.label}"
                        }
                    }
                }
            }
        }
    }
}

/// How long a keystroke waits before it becomes a request.
///
/// Long enough that typing a word is one call rather than four, short enough
/// that the list feels like it is following you.
#[expect(
    dead_code,
    reason = "nothing in the shell is searchable; the first list long enough \
              to need it arrives in PR 3, and this expectation fails then"
)]
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);

/// A text field whose value becomes a network call on a timer.
///
/// It is the only input in the app that works this way. Everything else is a
/// draft signal saved on an explicit action, by policy: a keystroke that
/// writes to the server is a keystroke that can fail halfway through a word.
/// Search is exempt because it writes nothing — the worst a stale one costs
/// is a list you were about to replace anyway.
///
/// The debounce timer is a plain `spawn`, deliberately not the `spawn_forever`
/// the rest of the app uses: a search whose screen has gone should go with it.
#[component]
pub(crate) fn SearchField(
    #[props(default = "Search".to_owned())] placeholder: String,
    /// What the field starts with.
    ///
    /// The filter it drives lives above this component, and this component is
    /// behind a screen `match`, so it unmounts whenever you open something and
    /// remounts empty when you come back — while the list it filtered stays
    /// filtered. Without this the recovery from "no results" is to guess that
    /// you must type a character into an already-blank box and delete it.
    #[props(default)]
    value: String,
    on_search: EventHandler<String>,
) -> Element {
    let mut text = use_signal(|| value);
    // Every keystroke starts a timer and invalidates the ones before it; the
    // last one standing is the only one that fires.
    let mut latest = use_signal(|| 0u64);

    rsx! {
        input {
            class: "field",
            r#type: "search",
            placeholder: "{placeholder}",
            autocapitalize: "off",
            autocomplete: "off",
            spellcheck: "false",
            value: "{text}",
            oninput: move |e| {
                let value = e.value();
                text.set(value.clone());
                let id = latest() + 1;
                latest.set(id);
                spawn(async move {
                    tokio::time::sleep(SEARCH_DEBOUNCE).await;
                    if *latest.peek() == id {
                        on_search.call(value);
                    }
                });
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_is_destructive_and_says_so() {
        let face = RowFace::DELETE;
        assert_eq!(face.label, "Delete");
        assert_eq!(face.icon, "trash");
        assert!(face.danger);
        assert_eq!(face.class(), "swipe-action danger");
    }

    /// The saturated fill is the destructive exemption, not the default: a
    /// tray button that only archives must not wear it.
    #[test]
    fn an_ordinary_tray_button_is_not_painted_as_a_danger() {
        let face = RowFace::plain("Archive", "archive");
        assert!(!face.danger);
        assert_eq!(face.class(), "swipe-action");
    }
}
