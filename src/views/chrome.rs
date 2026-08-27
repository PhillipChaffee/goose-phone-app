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
use crate::shell::Shell;
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
    ///
    /// Two shells, two classes, and deliberately not one class with a desktop
    /// override: `.swipe-action` is a full-height 84px slab with the card's
    /// curve on its trailing edge, which is the right shape for the last item
    /// of a snap scroller and the wrong shape for anything else. A row that
    /// does not swipe should not be wearing the swipe's clothes with three
    /// declarations taken back off.
    pub(crate) const fn class(self, shell: Shell) -> &'static str {
        match (shell, self.danger) {
            (Shell::Mobile, true) => "swipe-action danger",
            (Shell::Mobile, false) => "swipe-action",
            (Shell::Desktop, true) => "row-action danger",
            (Shell::Desktop, false) => "row-action",
        }
    }
}

/// The word beside a row action's icon, and the button's `title`. One or the
/// other, never both, and which one is a question about the shell.
///
/// On a phone the tray is a labelled column: the row has been dragged aside to
/// reveal it, the buttons are 84px wide, and the word is the whole reason the
/// drag was worth making. A `title` there would be an attribute nothing can
/// read — there is no pointer to hover.
///
/// On the desktop the same action is a 32px icon sitting on every row all the
/// time, and a word beside it on every row would be a second list running down
/// the right-hand side. So the label becomes the accessible name instead,
/// which is the arrangement goose's own desktop app uses for the identical
/// control (`ui/desktop/src/components/sessions/SessionListView.tsx` puts
/// `title` on every icon-only row button; `ExtensionItem.tsx` uses
/// `aria-label`). The icon is always painted — `opacity-0
/// group-hover:opacity-100` exists in that codebase, but on secondary chrome,
/// and a delete you cannot see until you find it is not an affordance.
///
/// Returned as data rather than as two `rsx!` arms so the rule can be tested:
/// the whole promise of this branch is that the mobile arm is unchanged, and
/// an `Element` is not something a test can hold.
///
/// The word is `""` on the desktop rather than an `if` around the text node,
/// and that is not a shortcut — it is what keeps the phone's markup provably
/// identical. `"{word}"` with `word` bound to the label is the same rsx
/// construct emitting the same text node as the bare `"{face.label}"` it
/// replaces, whereas a conditional makes the node a branch: Dioxus writes an
/// HTML comment where a false branch would have been, and the phone's DOM
/// would then differ from the one every captured gallery state holds. An empty
/// text node generates no flex item, so on the desktop it contributes nothing
/// at all.
pub(crate) const fn row_action_words(
    shell: Shell,
    face: RowFace,
) -> (&'static str, Option<&'static str>) {
    match shell {
        Shell::Mobile => (face.label, None),
        Shell::Desktop => ("", Some(face.label)),
    }
}

/// Whether a row paints itself as the one the detail column is showing.
///
/// `false` on the phone whatever the caller passed, and that is the whole
/// reason this is a function rather than the prop used directly. One screen is
/// on the phone at a time, so a list is never beside what it opened and there
/// is nothing to mark — but the state the lists key off (`ctx.chat.session_id`,
/// `ctx.scheduler.open`, …) outlives the screen that set it, so a row could
/// come back from a chat wearing a highlight the phone has never had. Deciding
/// it here means every list gets that answer from one place, and the phone's
/// class string stays the literal it was captured with.
pub(crate) const fn row_is_marked(shell: Shell, selected: bool) -> bool {
    match shell {
        Shell::Mobile => false,
        Shell::Desktop => selected,
    }
}

/// One button of a row's actions: the phone's swipe tray, the desktop's icons
/// on the row. [`row_action_words`] and [`RowFace::class`] are the difference.
#[derive(Clone, PartialEq)]
pub(crate) struct RowAction {
    pub face: RowFace,
    pub on_pick: EventHandler<()>,
}

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
/// the caller puts under it, and — on the phone — a swipe tray behind it. On
/// the desktop the same actions are icons on the row, and the row can say that
/// it is the one the detail column is showing.
///
/// The class names are the `.session-*` set on purpose, and the purpose is
/// not brevity. Three things key off them and would otherwise have to be
/// found and changed together: `src/viewport.rs`'s close-the-open-row
/// handler, `src/domdump.rs`'s `swiped` suffix detector, and
/// `docs/audit.js`'s longest-text stress map. Reusing the class is how four
/// future lists get all three for free. Read `.session-*` as "list row".
///
/// An empty `actions` renders no container at all. On the phone that means the
/// row does not move, because a row that swipes open onto nothing is worse than
/// one that does not swipe; on the desktop it means no icons and, through
/// `assets/desktop.css`'s `:has()` rules, no gutter reserved for them. Two of
/// the five features arriving after this one have nothing destructive to put on
/// a row.
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
    /// True when the detail column is showing THIS row.
    ///
    /// Ignored on the phone (see [`row_is_marked`]), where the list is never on
    /// screen beside what it opened. Defaulted, so a list with nothing to mark
    /// — and every existing call site — says nothing.
    #[props(default)]
    selected: bool,
    /// This row wants the reader. Rule 8's dot, on the tile, so a scroll down
    /// the list answers "which one" without reading a word — the same badge
    /// the Code list already draws for a chat blocked on a permission.
    ///
    /// Defaulted, so every list that has nothing to flag says nothing.
    #[props(default)]
    attention: bool,
    on_open: EventHandler<()>,
    children: Element,
) -> Element {
    // `selected` is a fact about the DATA — which id this destination has
    // open — and every list gets it from a signal that outlives the screen
    // that set it (`ctx.chat.session_id`, `ctx.scheduler.open`, …). So a row
    // came back from a detail still wearing the highlight, and the two columns
    // then said opposite things at the same time: the list painted a row as
    // the one being shown while the pane beside it read "Nothing open — pick
    // something from Chats to see it here". One click reproduced it (open a
    // row, press the back chevron) and a fresh launch could show it unprompted.
    //
    // The missing half is whether the detail column is showing ANYTHING, and
    // the app already knows: `at_root` is a destination's own answer to "is my
    // stack at its root", which is exactly "nothing is open". `nav::current`
    // resolves to the destination whose stack is on screen, which is the one
    // whose list this row is in.
    //
    // The `&&` short-circuits, and that is the mobile proof rather than a
    // micro-optimisation: `row_is_marked` is `false` on the phone whatever it
    // is passed, so `nav::current` is never called there and the phone's rows
    // subscribe to no signal they did not already read.
    let ctx = crate::state::use_app_ctx();
    let marked =
        row_is_marked(Shell::CURRENT, selected) && !(crate::nav::current(&ctx).at_root)(&ctx);
    rsx! {
        li {
            // A conditional class rather than a literal, and the phone's value
            // is byte-for-byte the literal it replaces. `render_group` already
            // does this to `.drawer-item` and the captured gallery shows the
            // result: `class` is still written before Dioxus's own
            // `data-dioxus-id`, so the phone's rows are unchanged.
            class: if marked { "session-item on" } else { "session-item" },
            // The whole row is the tap target (design rule 9); the tray's own
            // buttons stop propagation below.
            onclick: move |_| on_open.call(()),
            div { class: "session-swipe",
                div {
                    class: if attention { "session-tile attention" } else { "session-tile" },
                    Icon { name: icon }
                }
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
                // The container, its class and its place in the row are the
                // same on both shells; `assets/desktop.css` re-flexes
                // `.session-swipe` from `flex: 0 0 100%` to `flex: 1 1 auto`,
                // which brings this element from past the row's right edge to
                // inside it without a single change to the markup. That is why
                // every `RowAction` call site in the app is untouched.
                div { class: "session-actions",
                    for action in actions {
                        {
                            let (word, title) = row_action_words(Shell::CURRENT, action.face);
                            rsx! {
                                button {
                                    key: "{action.face.label}",
                                    class: action.face.class(Shell::CURRENT),
                                    title,
                                    onclick: move |e: Event<MouseData>| {
                                        e.stop_propagation();
                                        action.on_pick.call(());
                                    },
                                    Icon { name: action.face.icon }
                                    "{word}"
                                }
                            }
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

/// Everything here takes `Shell` as an argument and nothing reads
/// `Shell::CURRENT`. `cargo test` runs with the default features on a host, so
/// `CURRENT` is always `Desktop` there — a mobile assertion that read it would
/// be asserting about the desktop arm and passing.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_is_destructive_and_says_so() {
        let face = RowFace::DELETE;
        assert_eq!(face.label, "Delete");
        assert_eq!(face.icon, "trash");
        assert!(face.danger);
        assert_eq!(face.class(Shell::Mobile), "swipe-action danger");
    }

    /// The saturated fill is the destructive exemption, not the default: a
    /// tray button that only archives must not wear it.
    #[test]
    fn an_ordinary_tray_button_is_not_painted_as_a_danger() {
        let face = RowFace::plain("Archive", "archive");
        assert!(!face.danger);
        assert_eq!(face.class(Shell::Mobile), "swipe-action");
    }

    /// The phone's arm is FROZEN. These four strings are what
    /// `assets/main.css` styles, what `src/viewport.rs`'s tap-to-close handler
    /// and `src/domdump.rs`'s `swiped` detector look for, and what all 49
    /// captured gallery states contain. A change to any of them is a change to
    /// mobile rendering, which is the one thing the desktop shell promised not
    /// to make — so this is the assertion that turns the promise into a gate.
    #[test]
    fn the_phone_row_action_is_exactly_what_shipped() {
        let delete = RowFace::DELETE;
        let rename = RowFace::plain("Rename", "pencil");
        assert_eq!(delete.class(Shell::Mobile), "swipe-action danger");
        assert_eq!(rename.class(Shell::Mobile), "swipe-action");
        // A labelled button and no `title`, exactly as before: the tray's
        // buttons have always carried their word, and they have never carried
        // an attribute.
        assert_eq!(row_action_words(Shell::Mobile, delete), ("Delete", None));
        assert_eq!(row_action_words(Shell::Mobile, rename), ("Rename", None));
    }

    /// The desktop's arm: an icon with the label as its accessible name, and
    /// no word in the row.
    #[test]
    fn the_desktop_row_action_names_itself_without_a_word_in_the_row() {
        for face in [RowFace::DELETE, RowFace::plain("Rename", "pencil")] {
            let (word, title) = row_action_words(Shell::Desktop, face);
            assert_eq!(word, "");
            assert_eq!(
                title,
                Some(face.label),
                "an icon-only button with no accessible name is a control \
                 nobody can identify"
            );
        }
    }

    /// The literals above are pinned to this file. This pins them to the
    /// SHIPPED MARKUP: `docs/gallery-states.json` is 49 states dumped out of
    /// the running app on a device, it is what `docs/audit.js` measures, and
    /// it is never hand-edited. So the phone's tray button is reconstructed
    /// here from what the code says it emits and looked up in what the phone
    /// actually emitted — which is as close to "mobile rendering did not
    /// change" as anything can get without a device in the room.
    ///
    /// What each half actually proves, stated exactly, because a test whose
    /// comment overclaims is a test a future reader trusts for the wrong
    /// reason. `docs/gallery-states.json` was captured before this branch
    /// existed and is never hand-edited, so NOTHING the new code emits can
    /// change what is in it — the two `contains` are one-directional: they say
    /// the strings this file builds are still findable in what the phone
    /// shipped. What guards against a `title` reaching the phone is the plain
    /// `assert_eq!(title, None, …)` below, and what guards against the desktop
    /// class reaching it is the `!contains("row-action")` at the end. The
    /// opening tag pins the class and the attribute that follows it; the
    /// closing pins the word to the row, directly after the icon.
    #[test]
    fn the_phone_row_action_still_matches_the_markup_that_shipped() {
        let gallery = shipped_markup();
        for face in [RowFace::DELETE, RowFace::plain("Rename", "pencil")] {
            let (word, title) = row_action_words(Shell::Mobile, face);
            let class = face.class(Shell::Mobile);
            assert_eq!(title, None, "the tray has never carried an attribute");
            let opening = format!("<button class=\"{class}\" data-dioxus-id=\"");
            assert!(
                gallery.contains(&opening),
                "no captured state contains `{opening}` — the phone's tray \
                 button is no longer the one the gallery was captured from"
            );
            let closing = format!("</svg>{word}</button>");
            assert!(
                gallery.contains(&closing),
                "no captured state contains `{closing}` — the phone's tray \
                 button has lost its word, or gained something between the \
                 icon and it"
            );
        }
        assert!(
            !gallery.contains("row-action"),
            "a desktop class reached the phone's captured markup"
        );
    }

    /// The 49 states the PHONE was captured in, unescaped.
    ///
    /// Read from disk rather than `include_str!`ed, following
    /// `src/viewport.rs`'s own fixture test: it is 352K, and a test does not
    /// need it in the binary.
    ///
    /// The phone's half of the store, and the filter is the point rather than
    /// housekeeping. `docs/gallery-states.json` holds both shells now — a
    /// desktop dump's key carries `shell::DUMP_PREFIX` — and every assertion
    /// below is of the form "no captured state contains this desktop class",
    /// which the desktop's own states would answer for the phone and fail.
    /// Keyed rather than pattern-matched on the markup for the reason the
    /// prefix exists at all: which shell drew a state is a fact about the
    /// capture, not something to infer from what is in it.
    fn shipped_markup() -> String {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/gallery-states.json");
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!raw.is_empty(), "cannot read {}", path.display());
        let states: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).unwrap_or_default();
        let phone: Vec<String> = states
            .into_iter()
            .filter(|(key, _)| !key.starts_with(crate::shell::DUMP_PREFIX_DESKTOP))
            .map(|(_, markup)| markup)
            .collect();
        // Not `!raw.is_empty()` twice. A store that had lost its phone half
        // entirely would leave every `contains` assertion below trivially
        // satisfied and every `!contains` one vacuously true, which is a test
        // that has stopped testing rather than one that fails.
        assert!(
            !phone.is_empty(),
            "{} holds no phone state — every claim below would pass vacuously",
            path.display()
        );
        phone.join("\n")
    }

    /// The phone has no selected row, whatever a list passes.
    ///
    /// Not a tidiness point. The state the lists read — `ctx.chat.session_id`,
    /// `ctx.scheduler.open`, `ctx.skills.open` — outlives the screen that set
    /// it, so on the phone a row would come back from a chat wearing a
    /// highlight the phone has never shipped, in the one place `docs/audit.js`
    /// measures. The class string is the frozen literal, and this is the gate
    /// on it.
    #[test]
    fn the_phone_has_no_selected_row() {
        assert!(!row_is_marked(Shell::Mobile, true));
        assert!(!row_is_marked(Shell::Mobile, false));
        assert!(row_is_marked(Shell::Desktop, true));
        assert!(!row_is_marked(Shell::Desktop, false));

        assert!(
            !shipped_markup().contains("session-item on"),
            "a selected row reached the phone's captured markup"
        );
    }

    /// A desktop row must not borrow the tray's class, or the tray's 84px
    /// min-width, its full-row height and the card's curve on its trailing
    /// edge all follow it onto a row that does not swipe.
    ///
    /// Iterating the faces rather than naming two is the habit that keeps this
    /// honest: a `RowFace` added later is one line in this array away from
    /// being covered.
    #[test]
    fn no_desktop_row_class_is_a_phone_class() {
        for face in [RowFace::DELETE, RowFace::plain("Archive", "archive")] {
            assert_ne!(face.class(Shell::Desktop), face.class(Shell::Mobile));
            assert!(!face.class(Shell::Desktop).contains("swipe"));
        }
    }
}
