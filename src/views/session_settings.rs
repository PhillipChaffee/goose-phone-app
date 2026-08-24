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
//! offers less", never as "the app forgot something".

use dioxus::prelude::*;

use crate::icons::Icon;

/// One selectable value of a setting.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SettingChoice {
    pub value: String,
    pub label: String,
}

impl SettingChoice {
    pub(crate) fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
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
    let drilled = open_row()
        .and_then(|id| rows.iter().find(|r| r.id == id).cloned())
        .filter(SettingRow::drills);

    let body = match drilled {
        Some(row) => {
            let id = row.id.clone();
            rsx! {
                div { class: "sheet-head",
                    button {
                        class: "icon-btn back",
                        title: "Back",
                        onclick: move |_| open_row.set(None),
                        Icon { name: "chevron-left" }
                    }
                    h2 { "{row.name}" }
                }
                div { class: "choice-list",
                    for choice in row.choices.iter() {
                        button {
                            key: "{choice.value}",
                            class: if row.current.as_deref() == Some(choice.value.as_str()) {
                                "choice selected"
                            } else {
                                "choice"
                            },
                            onclick: {
                                let (id, value) = (id.clone(), choice.value.clone());
                                move |_| {
                                    onchoose.call((id.clone(), value.clone()));
                                    open_row.set(None);
                                }
                            },
                            span { class: "choice-name", "{choice.label}" }
                            if row.current.as_deref() == Some(choice.value.as_str()) {
                                Icon { name: "check" }
                            }
                        }
                    }
                }
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

    #[test]
    fn only_a_label_that_is_its_own_id_gets_rewritten() {
        assert_eq!(choice_label("low", "low"), "Low");
        assert_eq!(
            choice_label("Claude Opus 5", "claude-opus-5"),
            "Claude Opus 5"
        );
        assert_eq!(choice_label("", "thinking_hard"), "Thinking hard");
    }
}
