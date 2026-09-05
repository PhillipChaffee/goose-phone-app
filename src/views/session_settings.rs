//! The composer's session-settings sheet — one chip, one sheet, one row
//! grammar, shared by both backends.
//!
//! The grammar is the whole design. A row is either
//!
//!   - a **control**: name, current value, chevron; tapping it pushes the
//!     list of values, or
//!   - a **fact**: name, value, and the reason it is a fact, with no chevron
//!     and no press state.
//!
//! There is a third shape, and it is deliberately rare: a row that opens
//! something of its own rather than a list of values ([`SettingRow::action`],
//! today the chat's title). It wears the control's clothes because it is one
//! — name, value, chevron, pressable — and the sheet does not know what it
//! opens, only that the caller asked to be told.
//!
//! Which one a setting gets is decided by whether choosing would change
//! anything — [`SettingRow::select`] downgrades itself to a fact when there
//! is one value or none. That is design rule 11 made mechanical: nothing
//! that cannot change ever renders as pressable, and nothing real ever
//! vanishes. It also covers the identical edge case both backends have —
//! goose ships `thinking_effort` as a lone `off` on a non-reasoning model,
//! and `OpenCode` returns no variants at all for the minimax/qwen/glm/kimi
//! families — so instead of the setting disappearing, the user is told why
//! it is not adjustable here.
//!
//! The two tabs share the grammar, not the list. Each backend contributes
//! exactly what it can actually do, so a shorter list reads as "this backend
//! offers less", never as "the app forgot something". What they must not
//! differ in is presentation: both end up **Provider / Model / Thinking
//! effort / Context length**, in that order, with the notes written in one
//! voice — the two sheets used to be built by unrelated code and read like
//! two products.
//!
//! Mode is the exception, on both. It left the sheet for a chip of its own in
//! the composer row and [`ChoicePickerSheet`], because it is the setting you
//! change mid-conversation rather than the one you set and forget — and every
//! reference app puts it exactly there.

use dioxus::prelude::*;

use crate::icons::Icon;

/// One selectable value of a setting.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SettingChoice {
    pub value: String,
    pub label: String,
    /// One line under the label saying what picking this does. Both backends
    /// send one for their modes and for nothing else, which is why it is an
    /// option on the choice rather than a second kind of list.
    pub note: Option<String>,
    /// A leading icon from [`crate::icons`]. A mode is a way of working, and
    /// a column of glyphs is what lets you find the one you want without
    /// reading four descriptions.
    pub icon: Option<String>,
}

impl SettingChoice {
    pub(crate) fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            note: None,
            icon: None,
        }
    }

    /// What this value does, in the backend's own words where it sends any.
    pub(crate) fn with_note(mut self, note: Option<String>) -> Self {
        self.note = note.filter(|n| !n.trim().is_empty());
        self
    }

    pub(crate) fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Which icon a mode wears, from its id or its name.
///
/// Both backends name their modes themselves and neither sends an icon, so
/// this reads the name — goose ships `auto` / `approve` / `chat`, `OpenCode`
/// ships `build` / `plan` and whatever the repository adds. Matched on
/// substrings rather than on the exact word so a `plan-only` or a
/// `smart_approve` still lands somewhere sensible, and anything unrecognised
/// gets the bolt, which is the generic "a mode" mark rather than a claim
/// about what it does.
pub(crate) fn mode_icon(id: &str) -> &'static str {
    let id = id.to_ascii_lowercase();
    if id.contains("plan") {
        "list"
    } else if id.contains("chat") {
        "message"
    } else if id.contains("approve") || id.contains("ask") {
        "shield-check"
    } else if id.contains("build") || id.contains("edit") || id.contains("write") {
        "wrench"
    } else if id.contains("explore") || id.contains("search") || id.contains("research") {
        "search"
    } else {
        "bolt"
    }
}

/// One row of the sheet.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SettingRow {
    /// Identifies the row to the caller's change handler; for goose this is
    /// the ACP `configId`.
    pub id: String,
    pub name: String,
    /// The current value, already in UI copy.
    pub value: String,
    /// What the setting means, or why it cannot change. Shown under a fact.
    pub note: Option<String>,
    /// Empty means this row is a fact, not a control.
    pub choices: Vec<SettingChoice>,
    pub current: Option<String>,
    /// This row hands itself back to the caller instead of drilling into a
    /// value list. Kept separate from `choices` so that drilling in stays
    /// impossible for it: a sheet that pushed an empty choice list would be a
    /// dead end with a back button.
    pub action: bool,
}

impl SettingRow {
    /// A setting the user chooses between — unless there is nothing to
    /// choose, in which case it is a fact stating where it is stuck.
    pub(crate) fn select(
        id: impl Into<String>,
        name: impl Into<String>,
        current: Option<&str>,
        choices: Vec<SettingChoice>,
        note: Option<String>,
    ) -> Self {
        let value = current
            .and_then(|c| choices.iter().find(|ch| ch.value == c))
            .map_or_else(
                || {
                    let raw = current.unwrap_or_default();
                    if raw.is_empty() {
                        "—".to_owned()
                    } else {
                        humanize(raw)
                    }
                },
                |ch| ch.label.clone(),
            );
        Self {
            id: id.into(),
            name: name.into(),
            value,
            note,
            choices: if choices.len() > 1 {
                choices
            } else {
                Vec::new()
            },
            current: current.map(str::to_owned),
            action: false,
        }
    }

    /// A row that opens something of the caller's rather than a value list.
    ///
    /// It is here for the one thing about a session that is typed rather than
    /// chosen: its title. That belongs in this sheet and not only in the list,
    /// because the moment you notice goose named the chat wrong is while you
    /// are reading the chat.
    pub(crate) fn action(
        id: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            value: value.into(),
            note: None,
            choices: Vec::new(),
            current: None,
            action: true,
        }
    }

    /// Something true about the session that no call can change.
    pub(crate) fn fact(
        id: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            value: value.into(),
            note: Some(note.into()),
            choices: Vec::new(),
            current: None,
            action: false,
        }
    }

    /// Pressable, and something happens. Both shapes that are: the one that
    /// pushes a value list, and the one that calls the caller back.
    const fn is_control(&self) -> bool {
        !self.choices.is_empty() || self.action
    }

    /// Pressable *and* it has values of its own to push.
    const fn drills(&self) -> bool {
        !self.choices.is_empty()
    }
}

/// UI copy for a choice whose backend gave it no label of its own.
///
/// goose sends `thinking_effort`'s values as their own names — `off`, `low`,
/// `xhigh` — and a backend enum set in a menu reads as debug output (design
/// rule 8). Only a name that is *identical* to its id is rewritten, so a
/// real label like `Claude Opus 5` is never mangled.
pub(crate) fn choice_label(name: &str, value: &str) -> String {
    if name.is_empty() {
        return humanize(value);
    }
    if name == value {
        return humanize(name);
    }
    name.to_owned()
}

/// What the composer chip says about thinking effort, after the model name —
/// or `None` when there is nothing worth saying.
///
/// A default is not worth saying. The chip is for what is true and notable
/// about the next message, and every session has a default; spelling one out
/// as "Default" turns the one place effort is visible at a glance into a place
/// it is usually noise. Both backends have their own way of writing it down
/// and both are filtered here rather than at each call site: `OpenCode`
/// records the literal string `default` on a session whose turn asked for no
/// variant (see `SessionModel::effort`), while the app carries `None` for the
/// same state and goose sends no value at all until one is set.
///
/// The two long tiers are shortened, because the chip has nowhere to put
/// them. On the goose composer at 360pt the label is 120px and "Claude Sonnet
/// 5" wants 94 of it, so a six-letter tier standing beside it took the name
/// down to "Claude…" — the chip stopped answering the one question it exists
/// to answer, in order to answer the second one. `medium`, which goose
/// serves, and `minimal`, which `OpenCode` does, are the only tiers either
/// backend has that are long enough to do that, and neither short form is
/// ambiguous next to the `Max` already on the chip. The sheet the chip opens
/// spells every tier out in full, and `.chip-effort` caps anything a backend
/// sends that is not on either ladder.
pub(crate) fn chip_effort(current: Option<&str>) -> Option<String> {
    let raw = current
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "default")?;
    Some(match raw.to_ascii_lowercase().as_str() {
        "minimal" => "Min".to_owned(),
        "medium" => "Med".to_owned(),
        _ => choice_label(raw, raw),
    })
}

fn humanize(raw: &str) -> String {
    let mut words = raw.replace('_', " ");
    if let Some(first) = words.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    words
}

/// The sheet itself: the row list, and the value list a control pushes.
#[component]
pub(crate) fn SessionSettingsSheet(
    /// Named in the subtitle, so a list that is shorter on one tab reads as
    /// that backend offering less rather than the app losing something.
    backend: String,
    rows: Vec<SettingRow>,
    onchoose: EventHandler<(String, String)>,
    /// Told which [`SettingRow::action`] row was pressed. Optional because a
    /// sheet made only of settings has nothing to hand back, and the tab that
    /// has none should not have to pass an empty handler.
    #[props(default)]
    onaction: Option<EventHandler<String>>,
    onclose: EventHandler<()>,
) -> Element {
    let mut open_row = use_signal(|| None::<String>);
    // ONE QUERY FOR THE SHEET, NOT ONE PER ROW, and it is declared out here
    // because hooks are positional: the drill-down is a match arm, and a
    // `use_signal` inside it would run on some renders and not others.
    //
    // Cleared on the way OUT rather than on the way in, because the way out
    // is where it can be enforced: there are exactly two exits and both are
    // in this component, while the way in is `render_row`'s press handler,
    // which knows nothing about a query. The signal starts empty, so a row
    // opened after another row's search is opened against nothing.
    let mut query = use_signal(String::new);
    let drilled = open_row()
        .and_then(|id| rows.iter().find(|r| r.id == id).cloned())
        .filter(SettingRow::drills);

    let body = match drilled {
        Some(row) => {
            let id = row.id.clone();
            // `None`, because this emptiness cannot be reached from here:
            // `drilled` is filtered by `SettingRow::drills`, which is
            // `!choices.is_empty()`. Writing a sentence for it would be copy
            // no reader can ever see.
            let list = searchable_choices(
                &row.choices,
                row.current.as_deref(),
                query,
                None,
                move |value| {
                    query.set(String::new());
                    onchoose.call((id.clone(), value));
                    open_row.set(None);
                },
            );
            rsx! {
                div { class: "sheet-head",
                    button {
                        class: "icon-btn back",
                        title: "Back",
                        onclick: move |_| {
                            query.set(String::new());
                            open_row.set(None);
                        },
                        Icon { name: "chevron-left" }
                    }
                    h2 { "{row.name}" }
                }
                {list}
            }
        }
        None => rsx! {
            h2 { "Session settings" }
            p { class: "modal-session", "{backend} · applies from your next message" }
            if rows.is_empty() {
                p { class: "empty", "This session has no settings to show yet." }
            }
            div { class: "setting-list",
                for row in rows.iter() {
                    {render_row(row, open_row, onaction)}
                }
            }
        },
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| onclose.call(()),
            div {
                class: "modal sheet",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                {body}
            }
        }
    }
}

/// A list long enough that finding is faster than scrolling gets a filter.
/// Below this the field would be a control that does nothing (rule 11) and a
/// row of chrome above a five-item list.
const SEARCH_AT: usize = 8;

/// The choices whose label, value or note contains `needle`, case-folded.
///
/// A free fn so it can be tested without a renderer. The note is searched as
/// well as the name, which is what makes a repo findable by its owner.
fn matching(choices: &[SettingChoice], needle: &str) -> Vec<SettingChoice> {
    if needle.is_empty() {
        return choices.to_vec();
    }
    let needle = needle.to_ascii_lowercase();
    let hit = |s: &str| s.to_ascii_lowercase().contains(&needle);
    choices
        .iter()
        .filter(|c| hit(&c.label) || hit(&c.value) || c.note.as_deref().is_some_and(hit))
        .cloned()
        .collect()
}

/// A sheet that is one picker and nothing else — what the composer's mode
/// chip opens, and what each of the new-session screen's four pills opens.
///
/// Mode is the one setting with a chip of its own, because it is the one you
/// change mid-conversation: the rest of the sheet is what the session runs
/// on, and this is how it behaves while it runs. It is still the same list of
/// choices a settings row drills into — literally so, through
/// [`choice_list`] — so learning one teaches the other.
#[component]
pub(crate) fn ChoicePickerSheet(
    title: String,
    /// Named in the subtitle, exactly as the settings sheet names it.
    backend: String,
    /// Replaces the "{backend} · applies from your next message" line where
    /// that copy is not true. The new-session screen has no next message; it
    /// has a first one, and saying otherwise about a session that does not
    /// exist yet is the kind of borrowed sentence rule 8 is about.
    subtitle: Option<String>,
    /// One line about the LIST rather than about a choice in it — today, the
    /// free models a private repo is not being offered. Same sentence the
    /// settings sheet's Model row carries, from the same helper.
    note: Option<String>,
    choices: Vec<SettingChoice>,
    current: Option<String>,
    /// Stands in for the list when there is nothing in it yet.
    empty: String,
    onchoose: EventHandler<String>,
    onclose: EventHandler<()>,
) -> Element {
    let query = use_signal(String::new);
    let subtitle =
        subtitle.unwrap_or_else(|| format!("{backend} · applies from your next message"));
    let list = searchable_choices(
        &choices,
        current.as_deref(),
        query,
        Some(empty),
        move |value| {
            onchoose.call(value);
        },
    );
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| onclose.call(()),
            div {
                // `picker` as well as `sheet`, for the same reason the
                // overflow menu carries `menu`: the capture harness keys a
                // state off the DOM, and a choice list inside a sheet is
                // exactly what the settings sheet looks like once you have
                // drilled into a row. Without this they file as one state and
                // whichever was captured last wins.
                class: "modal sheet picker",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                h2 { "{title}" }
                p { class: "modal-session", "{subtitle}" }
                if let Some(note) = note {
                    p { class: "hint", "{note}" }
                }
                {list}
            }
        }
    }
}

/// The filtered list, whichever emptiness it landed in, and the field that
/// produced it.
///
/// SHARED BY BOTH SHEETS, AND THAT IS THE FIX RATHER THAN A TIDY-UP. The same
/// values are reachable two ways — the composer's Model chip opens
/// [`ChoicePickerSheet`], and Session settings → Model drills into a row —
/// and until #195 only the first of the two could be searched. Against a
/// provider with a real catalogue that is not a missing convenience: the
/// owner reported that the model could not be changed at all, because the
/// list ran past the bottom of an 1150pt window and there was no field to
/// type into. One helper is what stops the two answering "is this list long
/// enough to need finding" differently again.
///
/// `empty` is what to say when the SERVER offered nothing, and `None` means
/// this call site cannot reach that state — the settings sheet only ever
/// drills into a row `SettingRow::drills` accepted, which is exactly a row
/// with choices. Copy for a screen no reader can open is copy nobody can
/// check.
fn searchable_choices<F: FnMut(String) + Clone + 'static>(
    choices: &[SettingChoice],
    current: Option<&str>,
    mut query: Signal<String>,
    empty: Option<String>,
    onpick: F,
) -> Element {
    let needle = query();
    let shown = matching(choices, &needle);
    // The field is decided by the WHOLE list, not by what survived the
    // filter: a search that narrows forty models to one must not then take
    // away the field that narrowed them.
    let searchable = choices.len() > SEARCH_AT;
    let list = choice_list(&shown, current, onpick);
    rsx! {
        if shown.is_empty() {
            // Two different emptinesses, and conflating them is how a search
            // that matched nothing came out reading like a server that
            // offered nothing.
            p { class: "empty",
                if let Some(empty) = empty.filter(|_| choices.is_empty()) {
                    "{empty}"
                } else {
                    "Nothing matches “{needle}”."
                }
            }
        }
        {list}
        // At the bottom, where a thumb is and where it stays put as the list
        // moves under it — which is where the reference puts it, and what
        // costs the sheet its single scrollbox (see
        // .modal.sheet:has(.sheet-search) in shared.css).
        if searchable {
            div { class: "sheet-search",
                Icon { name: "search" }
                input {
                    class: "field",
                    r#type: "text",
                    placeholder: "Search",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{needle}",
                    oninput: move |e| query.set(e.value()),
                }
            }
        }
    }
}

/// The list of values a setting can take: name, what it does, and a check on
/// the one in force. Shared by the settings sheet's drill-down and the mode
/// picker so the two cannot drift apart.
fn choice_list<F: FnMut(String) + Clone + 'static>(
    choices: &[SettingChoice],
    current: Option<&str>,
    onpick: F,
) -> Element {
    let rows = choices.iter().map(|choice| {
        let selected = current == Some(choice.value.as_str());
        let (label, note, icon) = (
            choice.label.clone(),
            choice.note.clone(),
            choice.icon.clone(),
        );
        let value = choice.value.clone();
        let mut onpick = onpick.clone();
        rsx! {
            button {
                key: "{value}",
                class: if selected { "choice selected" } else { "choice" },
                onclick: move |_| onpick(value.clone()),
                if let Some(icon) = icon {
                    // Wrapped rather than a bare Icon: the check beside it is
                    // painted with the success colour by a direct-child rule,
                    // and a leading mark is not a statement about state.
                    span { class: "choice-lead", Icon { name: "{icon}" } }
                }
                span { class: "choice-main",
                    span { class: "choice-name", "{label}" }
                    if let Some(note) = note {
                        span { class: "choice-note", "{note}" }
                    }
                }
                if selected {
                    Icon { name: "check" }
                }
            }
        }
    });
    rsx! {
        div { class: "choice-list", {rows} }
    }
}

/// One row, in whichever of the three shapes it earned.
fn render_row(
    row: &SettingRow,
    mut open_row: Signal<Option<String>>,
    onaction: Option<EventHandler<String>>,
) -> Element {
    let key = row.id.clone();
    let name = row.name.clone();
    let value = row.value.clone();
    let note = row.note.clone();
    // The note renders in BOTH shapes. It used to hang off the fact branch
    // only, which silently dropped every note attached to something you can
    // actually change — including the one warning that free models are being
    // withheld from a private repo, which exists precisely for a row that is
    // still pickable.
    let handler = onaction.filter(|_| row.action);
    // A control needs somewhere for the press to go: a value list to push, or
    // a caller waiting to be told which row it was. An action row on a sheet
    // that passed no handler is a chevron that does nothing, so it renders as
    // the fact it has effectively become.
    if row.is_control() && (row.drills() || handler.is_some()) {
        let id = row.id.clone();
        return rsx! {
            button {
                key: "{key}",
                class: "setting-row",
                onclick: move |_| match handler {
                    Some(onaction) => onaction.call(id.clone()),
                    None => open_row.set(Some(id.clone())),
                },
                span { class: "setting-main",
                    span { class: "setting-name", "{name}" }
                    span { class: "setting-value", "{value}" }
                    if let Some(note) = note {
                        span { class: "setting-note", "{note}" }
                    }
                }
                Icon { name: "chevron-right" }
            }
        };
    }
    rsx! {
        div { key: "{key}", class: "setting-row fact",
            span { class: "setting-main",
                span { class: "setting-name", "{name}" }
                span { class: "setting-value", "{value}" }
                if let Some(note) = note {
                    span { class: "setting-note", "{note}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_value_is_a_fact_not_a_control() {
        let row = SettingRow::select(
            "thinking_effort",
            "Thinking effort",
            Some("off"),
            vec![SettingChoice::new("off", "Off")],
            Some("Only reasoning models take an effort.".to_owned()),
        );
        assert!(!row.is_control());
        assert_eq!(row.value, "Off");
    }

    /// The third shape is pressable and has nothing to push, so drilling into
    /// it must be impossible rather than merely unlikely — an empty choice
    /// list is a dead end with a back button on it.
    #[test]
    fn an_action_row_is_pressable_but_never_drills() {
        let row = SettingRow::action("title", "Title", "Deploy the thing");
        assert!(row.is_control());
        assert!(!row.drills());
        assert_eq!(row.value, "Deploy the thing");
    }

    #[test]
    fn two_values_is_a_control() {
        let row = SettingRow::select(
            "mode",
            "Mode",
            Some("auto"),
            vec![
                SettingChoice::new("auto", "Auto"),
                SettingChoice::new("approve", "Manual approval"),
            ],
            None,
        );
        assert!(row.is_control());
        assert_eq!(row.value, "Auto");
    }

    /// A value the backend no longer offers still has to render as itself.
    #[test]
    fn a_current_value_outside_the_choices_still_shows() {
        let row = SettingRow::select("model", "Model", Some("retired_model"), Vec::new(), None);
        assert_eq!(row.value, "Retired model");
    }

    /// Every mode this maps has to name an icon that exists, or the chip
    /// renders an empty box — `Icon` returns nothing for a name it does not
    /// know, and a typo here would be silent.
    #[test]
    fn every_mode_icon_is_a_real_icon() {
        for mode in [
            "auto",
            "approve",
            "smart_approve",
            "chat",
            "build",
            "plan",
            "general",
            "explore",
            "",
            "something-upstream-invented",
        ] {
            let name = mode_icon(mode);
            assert!(
                crate::icons::path_for(name).is_some(),
                "mode {mode} asks for a missing icon: {name}"
            );
        }
    }

    /// Every spelling of "no tier was asked for" leaves the chip saying the
    /// model name alone.
    #[test]
    fn a_default_effort_is_not_worth_a_chip() {
        assert_eq!(chip_effort(None), None);
        assert_eq!(chip_effort(Some("")), None);
        assert_eq!(chip_effort(Some("   ")), None);
        assert_eq!(chip_effort(Some("default")), None);
    }

    /// A tier the reader chose is exactly what the chip is for, and it arrives
    /// as a backend enum rather than as UI copy.
    #[test]
    fn a_chosen_effort_reaches_the_chip_as_copy() {
        assert_eq!(chip_effort(Some("max")), Some("Max".to_owned()));
        assert_eq!(chip_effort(Some("xhigh")), Some("Xhigh".to_owned()));
        assert_eq!(chip_effort(Some("off")), Some("Off".to_owned()));
    }

    /// No tier either backend serves is long enough to crowd the model name
    /// off the chip. Five characters is what `.chip-effort` gives one before
    /// it starts clipping (assets/shared.css), and the goose row at 360pt has
    /// only 120px of label to divide between the two.
    #[test]
    fn every_tier_a_backend_serves_fits_the_chip() {
        assert_eq!(chip_effort(Some("medium")), Some("Med".to_owned()));
        assert_eq!(chip_effort(Some("minimal")), Some("Min".to_owned()));
        // goose's ladder, then OpenCode's; `off` and `none` reach the chip
        // only when a reader picked them from a list that offered something
        // else, which is a choice worth stating.
        for tier in [
            "off", "low", "medium", "high", "max", "none", "minimal", "xhigh",
        ] {
            let shown = chip_effort(Some(tier)).unwrap_or_default();
            assert!(
                shown.chars().count() <= 5,
                "{tier} reaches the chip as {shown}"
            );
        }
    }

    /// A mode nobody has heard of still gets a mark, so the picker never has
    /// a row that is text where its neighbours have glyphs.
    #[test]
    fn an_unknown_mode_falls_back_to_the_bolt() {
        assert_eq!(mode_icon("auto"), "bolt");
        assert_eq!(mode_icon("wander"), "bolt");
        assert_eq!(mode_icon("Plan"), "list");
        assert_eq!(mode_icon("smart_approve"), "shield-check");
    }

    /// A backend that sends no description, or an empty one, must not leave a
    /// blank line under the name.
    #[test]
    fn an_empty_description_is_not_a_note() {
        assert_eq!(
            SettingChoice::new("auto", "Auto")
                .with_note(Some("   ".to_owned()))
                .note,
            None
        );
        assert_eq!(
            SettingChoice::new("auto", "Auto").with_note(None).note,
            None
        );
    }

    /// The filter folds case and reaches the note, which is what makes a repo
    /// findable by its owner rather than only by its bare name.
    #[test]
    fn the_filter_folds_case_and_searches_the_note() {
        let choices = vec![
            SettingChoice::new("PhillipChaffee/personal-ai-setup", "personal-ai-setup")
                .with_note(Some("PhillipChaffee".to_owned())),
            SettingChoice::new("jaegertracing/artwork", "artwork")
                .with_note(Some("jaegertracing".to_owned())),
        ];
        let by_owner = matching(&choices, "JAEGER");
        assert_eq!(by_owner.len(), 1);
        assert_eq!(by_owner[0].label, "artwork");
        assert_eq!(matching(&choices, "SETUP").len(), 1);
        assert_eq!(
            matching(&choices, "").len(),
            2,
            "an empty needle matches all"
        );
        assert!(matching(&choices, "nothing-like-this").is_empty());
    }

    #[test]
    fn only_a_label_that_is_its_own_id_gets_rewritten() {
        assert_eq!(choice_label("low", "low"), "Low");
        assert_eq!(
            choice_label("Claude Opus 5", "claude-opus-5"),
            "Claude Opus 5"
        );
        assert_eq!(choice_label("", "thinking_hard"), "Thinking hard");
    }

    // ---------------------------------------- the settings sheet's drill-down

    /// A provider's catalogue, in the shape the owner reported it in #195:
    /// forty full identifiers, long, sharing prefixes and differing only in
    /// their tails. The five `MiniMax` names are the exact ones from that
    /// report, because they are the case the filter has to answer — `m2.7`
    /// must reach one row out of six that all begin `MiniMaxAI/MiniMax-M`.
    fn catalogue() -> Vec<SettingChoice> {
        let mut out: Vec<String> = [
            "Hcompany/Holo3-35B-A3B",
            "MiniMaxAI/MiniMax-M1-40k",
            "MiniMaxAI/MiniMax-M1-80k",
            "MiniMaxAI/MiniMax-M2",
            "MiniMaxAI/MiniMax-M2.5-FP4",
            "MiniMaxAI/MiniMax-M2.7",
            "MiniMaxAI/MiniMax-M3",
            "NousResearch/Nous-Hermes-2-Mixtral-8x7B-DPO",
            "Prism-ML/Ternary-Bonsai-27B",
            "Qwen/QwQ-32B",
            "Qwen/Qwen2-1.5B-Instruct",
        ]
        .map(str::to_owned)
        .into();
        // Padding to forty, so `SEARCH_AT` is passed by a real margin rather
        // than by one row.
        for n in 0..(40 - out.len()) {
            out.push(format!("meta-llama/Llama-4-{n}B"));
        }
        out.into_iter()
            .map(|id| SettingChoice::new(id.clone(), id))
            .collect()
    }

    /// The sheet exactly as a chat mounts it, with one drilling row so that
    /// pressing `setting-row` can only mean Model.
    fn model_sheet() -> Element {
        rsx! {
            SessionSettingsSheet {
                backend: "goose",
                rows: vec![SettingRow::select(
                    "model",
                    "Model",
                    Some("Qwen/QwQ-32B"),
                    catalogue(),
                    None,
                )],
                onchoose: move |_: (String, String)| {},
                onclose: move |()| {},
            }
        }
    }

    /// Two values, which is a control and a drill-down and still nothing worth
    /// a field.
    fn mode_sheet() -> Element {
        rsx! {
            SessionSettingsSheet {
                backend: "goose",
                rows: vec![SettingRow::select(
                    "mode",
                    "Mode",
                    Some("auto"),
                    vec![
                        SettingChoice::new("auto", "Auto"),
                        SettingChoice::new("approve", "Manual approval"),
                    ],
                    None,
                )],
                onchoose: move |_: (String, String)| {},
                onclose: move |()| {},
            }
        }
    }

    /// #195, and it is the whole issue: the composer's Model chip opened a
    /// picker you could type into, and the SAME values reached through Session
    /// settings opened a list you could only scroll. Against a real provider
    /// that meant the model could not be changed at all.
    #[test]
    fn drilling_into_a_long_row_gets_the_field_the_picker_already_had() {
        let _guard = crate::views::press::alone();
        let mut screen = crate::views::press::Pressable::mount(|_| {}, model_sheet);
        assert!(
            !screen.markup().contains("sheet-search"),
            "the row list itself is short and must not carry a field: {:.400}",
            screen.markup()
        );

        screen.press("setting-row");
        let opened = screen.markup();
        assert!(
            opened.contains("sheet-search"),
            "a forty-model list drilled into from the settings sheet still \
             has no way to type: {opened:.600}"
        );
        assert!(
            opened.contains("MiniMaxAI/MiniMax-M2.7") && opened.contains("Qwen/QwQ-32B"),
            "the unfiltered list is the whole catalogue: {opened:.600}"
        );

        screen.type_into("placeholder=\"Search\"", "m2.7");
        let filtered = screen.markup();
        assert!(
            filtered.contains("MiniMaxAI/MiniMax-M2.7"),
            "the needle from the issue finds its model: {filtered:.600}"
        );
        assert!(
            !filtered.contains("Qwen/QwQ-32B"),
            "a row the needle does not match is still on screen, so the field \
             narrows nothing: {filtered:.600}"
        );
        assert!(
            filtered.contains("sheet-search"),
            "the field that narrowed the list took itself away with it: \
             {filtered:.600}"
        );
    }

    /// The other half of the same decision: `SEARCH_AT` is about the LIST, and
    /// a two-value drill-down gets no chrome it cannot use.
    #[test]
    fn a_short_drill_down_is_left_alone() {
        let _guard = crate::views::press::alone();
        let mut screen = crate::views::press::Pressable::mount(|_| {}, mode_sheet);
        screen.press("setting-row");
        let opened = screen.markup();
        assert!(
            opened.contains("Manual approval"),
            "the drill-down opened: {opened:.400}"
        );
        assert!(
            !opened.contains("sheet-search"),
            "a two-item list got a filter, which is a control that does \
             nothing: {opened:.400}"
        );
    }

    /// A needle that matches nothing has to read as a search that found
    /// nothing, not as a server that offered nothing — and the sheet must not
    /// name the state it cannot reach, which is why the drill-down passes
    /// `None` for the server's own emptiness.
    #[test]
    fn a_needle_that_matches_nothing_blames_the_needle() {
        let _guard = crate::views::press::alone();
        let mut screen = crate::views::press::Pressable::mount(|_| {}, model_sheet);
        screen.press("setting-row");
        screen.type_into("placeholder=\"Search\"", "gpt-5");
        let empty = screen.markup();
        assert!(
            empty.contains("Nothing matches"),
            "an unmatched needle says so: {empty:.400}"
        );
        assert!(
            !empty.contains("MiniMaxAI"),
            "nothing survived the needle: {empty:.400}"
        );
    }

    /// Backing out clears the query, so the next row you open is opened
    /// against the whole of its own list. The query is one signal for the
    /// whole sheet — the alternative was one per row, which hooks cannot be —
    /// so without the reset a search for a model would silently hide most of
    /// the next setting you looked at.
    #[test]
    fn backing_out_of_a_drill_down_forgets_what_was_typed() {
        let _guard = crate::views::press::alone();
        let mut screen = crate::views::press::Pressable::mount(|_| {}, model_sheet);
        screen.press("setting-row");
        screen.type_into("placeholder=\"Search\"", "m2.7");
        screen.press("title=\"Back\"");
        assert!(
            !screen.markup().contains("sheet-search"),
            "Back returned to the row list: {:.400}",
            screen.markup()
        );

        screen.press("setting-row");
        let again = screen.markup();
        assert!(
            again.contains("Qwen/QwQ-32B"),
            "the row reopened still filtered by the last row's needle: \
             {again:.600}"
        );
    }
}
