//! Connect: what the agent can reach, and how to give it one more thing.
//!
//! Two screens sharing the Connect back stack. The list is the honest picture
//! of the server's config — including the extensions that predate this app and
//! have no tool allowlist at all, which are shown as what they are rather than
//! quietly rendered as "3 tools". The add screen is a catalogue plus a
//! credential field, and the credential field is write-only: no reveal
//! control, nothing persisted on the phone, and no read-back (goose returns a
//! clear prefix for a secret, so reading one back to "confirm" it would leak
//! it).

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

use crate::connect::{
    add_from_catalog, refresh_extensions, set_enabled, tool_summary, ConnectScreen, CATALOG,
};
use crate::icons::Icon;
use crate::state::use_app_ctx;
use crate::views::ConnBadge;

#[component]
pub fn ConnectView() -> Element {
    let ctx = use_app_ctx();
    let entries = (ctx.extensions)();
    let warnings = (ctx.extension_warnings)();
    let loading = (ctx.extensions_loading)();
    let connected = (ctx.conn)().is_connected();
    let error = (ctx.connect_error)();

    // First look at this screen after connecting fills it in. `use_effect`
    // reruns when `connected` changes, so reconnecting refreshes too.
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            spawn_forever(async move { refresh_extensions(&ctx).await });
        }
    });

    rsx! {
        header { class: "topbar",
            button {
                class: "icon-btn menu",
                onclick: move |_| {
                    let mut open = ctx.drawer_open;
                    open.set(true);
                },
                Icon { name: "menu" }
            }
            h1 { class: "title", "Connect" }
            ConnBadge {}
            div { class: "topbar-actions",
                button {
                    class: "icon-btn",
                    disabled: loading || !connected,
                    onclick: move |_| {
                        spawn_forever(async move { refresh_extensions(&ctx).await });
                    },
                    if loading { "…" } else { Icon { name: "refresh" } }
                }
            }
        }

        main { class: "scroll has-fab",
            if !connected {
                p { class: "empty",
                    "Not connected. Services live on the goose server, so connect in Settings first."
                }
            }

            if let Some(error) = error {
                p { class: "error-box", "{error}" }
            }

            for warning in warnings {
                p { class: "error-box", "{warning}" }
            }

            if connected && entries.is_empty() && !loading {
                p { class: "empty", "No services configured yet." }
            }

            ul { class: "ext-list",
                for entry in entries {
                    ExtensionRow { entry }
                }
            }
        }

        button {
            class: "fab",
            disabled: !connected,
            onclick: move |_| {
                let mut screen = ctx.connect_screen;
                let mut error = ctx.connect_error;
                error.set(None);
                screen.set(ConnectScreen::Add);
            },
            Icon { name: "plus" }
            "Add service"
        }
    }
}

/// One configured extension.
///
/// The allowlist line is the point of the row. When it is empty the row says
/// so in danger colours rather than leaving a reassuring blank: an extension
/// with no allowlist can call every tool its MCP server publishes, which is
/// the failure this whole surface is built to make visible.
#[component]
fn ExtensionRow(entry: goose_acp_client::GooseExtensionEntry) -> Element {
    let ctx = use_app_ctx();
    let unrestricted = entry.extension.available_tools().is_empty();
    let enabled = entry.enabled;
    let config_key = entry.config_key.clone();
    let name = entry.extension.name().to_string();

    rsx! {
        li { class: "ext-item",
            div { class: "session-tile", Icon { name: "package" } }
            div { class: "session-main",
                div { class: "session-head",
                    div { class: "session-title", "{name}" }
                    span {
                        class: if enabled { "ext-state on" } else { "ext-state" },
                        if enabled { "on" } else { "off" }
                    }
                }
                if let Some(description) = entry.extension.description() {
                    div { class: "session-snippet", "{description}" }
                }
                div { class: "session-meta",
                    span { "{entry.extension.transport()}" }
                    if !entry.extension.env_keys().is_empty() {
                        // Names only. The values are on the server, and there
                        // is no way to ask for them from here.
                        span { "{entry.extension.env_keys().join(\", \")}" }
                    }
                }
                div {
                    class: if unrestricted { "ext-tools unrestricted" } else { "ext-tools" },
                    if unrestricted {
                        Icon { name: "wrench" }
                        "No tool allowlist — every tool this server publishes is allowed"
                    } else {
                        Icon { name: "wrench" }
                        "{tool_summary(&entry)}"
                    }
                }
            }
            if let Some(config_key) = config_key {
                button {
                    class: if enabled { "btn secondary small" } else { "btn primary small" },
                    onclick: move |_| set_enabled(&ctx, &config_key, !enabled),
                    if enabled { "Disable" } else { "Enable" }
                }
            }
        }
    }
}

/// Pick a service, type its credential, connect it.
#[component]
pub fn ConnectAddView() -> Element {
    let ctx = use_app_ctx();
    let mut picked = use_signal(|| None::<usize>);
    // One draft per credential field of the picked entry. Local to this
    // component, so it dies with the screen — credentials are never written to
    // the persisted `Settings`.
    let mut drafts = use_signal(Vec::<String>::new);
    let busy = (ctx.connect_busy)();
    let error = (ctx.connect_error)();

    let back = move |_| {
        let mut screen = ctx.connect_screen;
        screen.set(ConnectScreen::List);
    };

    rsx! {
        header { class: "topbar",
            button { class: "icon-btn back", onclick: back, Icon { name: "chevron-left" } }
            h1 { class: "title", "Add service" }
            div { class: "topbar-actions", ConnBadge {} }
        }

        main { class: "scroll settings",
            if let Some(error) = error {
                p { class: "error-box", "{error}" }
            }

            if picked().is_none() {
                for (index , entry) in CATALOG.iter().enumerate() {
                    button {
                        key: "{entry.id}",
                        class: "catalog-item",
                        onclick: move |_| {
                            drafts.set(vec![String::new(); CATALOG[index].secrets.len()]);
                            picked.set(Some(index));
                        },
                        div { class: "session-tile", Icon { name: "cloud" } }
                        div { class: "session-main",
                            div { class: "session-title", "{entry.display_name}" }
                            div { class: "session-snippet", "{entry.summary}" }
                            div { class: "session-meta", span { "privacy tier {entry.tier}" } }
                        }
                    }
                }
                p { class: "hint",
                    "Only services that can be finished from a phone are listed. A service "
                    "that needs an OAuth consent screen cannot be: goose opens the browser "
                    "on the server, and the authorization URL never reaches this app."
                }
            } else if let Some(index) = picked() {
                CredentialForm { index, drafts, busy }
            }

            if picked().is_some() && !busy {
                div { class: "btn-row",
                    button {
                        class: "btn secondary grow",
                        onclick: move |_| picked.set(None),
                        "Pick a different service"
                    }
                }
            }
        }
    }
}

/// The credential fields for one catalogue entry, plus what connecting will
/// permit.
#[component]
fn CredentialForm(index: usize, drafts: Signal<Vec<String>>, busy: bool) -> Element {
    let ctx = use_app_ctx();
    let Some(entry) = CATALOG.get(index) else {
        return rsx! {};
    };
    let extension = (entry.build)();
    let tools = extension.available_tools().to_vec();
    // Every field must be filled: a blank one would store an empty secret and
    // the extension would fail to start with a confusing error later.
    let ready = drafts.read().iter().all(|v| !v.trim().is_empty());

    rsx! {
        section { class: "card",
            h2 { "{entry.display_name}" }
            p { class: "hint", "{entry.summary}" }

            for (position , secret) in entry.secrets.iter().enumerate() {
                label { class: "field-label", "{secret.key}" }
                input {
                    class: "field",
                    // Always a password field, and there is deliberately no
                    // control to reveal it. Even the non-secret-looking ones
                    // (a CalDAV URL, a username) are stored as secrets, so
                    // treating them alike here keeps the story simple.
                    r#type: "password",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{drafts.read()[position]}",
                    oninput: move |e| {
                        if let Some(slot) = drafts.write().get_mut(position) {
                            *slot = e.value();
                        }
                    },
                }
                p { class: "hint", "{secret.prompt}" }
            }

            p { class: "hint",
                "Stored on the goose server, in its secrets file. It is never sent back to "
                "this phone and never saved here — if you need it again, get it from the "
                "service that issued it."
            }
        }

        section { class: "card",
            h2 { "What it will be allowed to do" }
            p { class: "hint", "{entry.scope}" }
            ul { class: "tool-allowlist",
                for tool in tools {
                    li { key: "{tool}", "{tool}" }
                }
            }
            p { class: "hint",
                "These are the only tools the agent can call on this service. The list is "
                "checked against the server after it is saved — if it did not stick, the "
                "service is switched back off and you get told."
            }
        }

        div { class: "btn-row",
            button {
                class: "btn primary grow",
                disabled: busy || !ready,
                onclick: move |_| {
                    let values = drafts.read().clone();
                    add_from_catalog(&ctx, index, values);
                },
                if busy { "Connecting…" } else { "Connect" }
            }
        }
        if !ready && !busy {
            p { class: "hint", "Fill in every field to continue." }
        }
    }
}
