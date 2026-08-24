//! Extensions: what the agent is plugged into, one extension in full, and the
//! sheet that adds one more.
//!
//! The list is the honest picture of the server's config — including the
//! extensions that predate this app and have no tool allowlist at all, which
//! are shown as what they are rather than quietly rendered as "3 tools". The
//! add sheet is a catalogue plus a credential field, and the credential field
//! is write-only: no reveal control, nothing persisted on the phone, and no
//! read-back (goose returns a clear prefix for a secret, so reading one back
//! to "confirm" it would leak it).

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{GooseExtension, GooseExtensionEntry, McpServer};

use crate::extensions::{
    add_from_catalog, kind_copy, meta_line, refresh, set_enabled, title_for, tool_summary,
    CatalogEntry, RowState, Screen, Sheet, CATALOG,
};
use crate::icons::Icon;
use crate::state::use_app_ctx;
use crate::views::chrome::{ListRow, RowAction, RowFace, TopBar};

#[component]
pub(crate) fn ExtensionsView() -> Element {
    let ctx = use_app_ctx();
    let remote = (ctx.extensions.list)();
    let warnings = (ctx.extensions.warnings)();
    let toggle = (ctx.extensions.toggle)();
    let sheet = (ctx.extensions.sheet)();
    let connected = (ctx.conn)().is_connected();
    let loading = remote.loading;

    // First look at this screen after connecting fills it in. `use_effect`
    // reruns when `connected` changes, so reconnecting refreshes too.
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            spawn_forever(async move { refresh(&ctx).await });
        }
    });

    rsx! {
        TopBar { title: "Extensions", conn: true }

        main {
            class: "scroll has-fab",
            // Pull, not a button. A refresh control in the bar is a control
            // that is wrong most of the time it is on screen.
            "data-refresh": "extensions",
            "data-refreshing": "{loading}",

            if remote.unsupported {
                // Not a failure and so not a retry: this goose does not have
                // the extensions plane at all, and asking again cannot change
                // that.
                p { class: "empty",
                    "This goose server does not offer extensions."
                }
            } else {
                if !connected {
                    p { class: "empty",
                        "Not connected. Extensions live on the goose server, so connect in Settings first."
                    }
                }

                if let Some(message) = remote.sticky.clone() {
                    p { class: "error-box", "{message}" }
                }

                // goose reports a config it could not load and then carries
                // on without it, so the extension is simply absent from the
                // list. This banner is the only way a phone user finds out.
                for warning in warnings {
                    div { key: "{warning}", class: "banner", "{warning}" }
                }

                if connected && remote.items.is_empty() && !loading {
                    p { class: "empty", "No extensions configured yet." }
                }

                ul { class: "session-list",
                    for entry in remote.items {
                        ExtensionRow {
                            key: "{entry.extension.name()}",
                            entry,
                            toggle: toggle.clone(),
                        }
                    }
                }
            }
        }

        if !remote.unsupported {
            button {
                class: "fab",
                disabled: !connected,
                onclick: move |_| {
                    let mut sheet = ctx.extensions.sheet;
                    let mut list = ctx.extensions.list;
                    list.write().sticky = None;
                    sheet.set(Sheet::Picking);
                },
                Icon { name: "plus" }
                "Add extension"
            }
        }

        // At the view's root, never inside `TopBar`: the bar carries
        // `backdrop-filter`, which makes it the containing block for every
        // `position: fixed` descendant, and a sheet opened inside it is
        // trapped in a 94px pill.
        if sheet != Sheet::Closed {
            AddSheet { sheet }
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
fn ExtensionRow(entry: GooseExtensionEntry, toggle: Option<crate::extensions::Toggle>) -> Element {
    let ctx = use_app_ctx();
    let open = entry.extension.name().to_string();
    let config_key = entry.config_key.clone();
    let enabled = entry.enabled;
    let unrestricted = entry.extension.available_tools().is_empty();

    let mine = toggle.filter(|t| Some(t.key.as_str()) == config_key.as_deref());
    let state = RowState::of(
        enabled,
        mine.as_ref().is_some_and(|t| !t.failed),
        mine.as_ref().is_some_and(|t| t.failed),
    );

    // Only offer controls that do something (design rule 11): an extension the
    // server stored without a config key has nothing `set-enabled` can address,
    // so it gets no tray rather than a button that always fails.
    let actions = config_key.map_or_else(Vec::new, |key| {
        let face = if enabled {
            RowFace::plain("Disable", "pause")
        } else {
            RowFace::plain("Enable", "play")
        };
        vec![RowAction::new(
            face,
            EventHandler::new(move |()| set_enabled(&ctx, &key, !enabled)),
        )]
    });

    rsx! {
        ListRow {
            icon: "package",
            title: title_for(&entry.extension),
            actions,
            on_open: move |()| {
                let (mut screen, mut showing) = (ctx.extensions.screen, ctx.extensions.open);
                showing.set(Some(open.clone()));
                screen.set(Screen::Detail);
            },
            div { class: "ext-meta",
                span { class: state.dot() }
                span { "{meta_line(state, entry.extension.transport())}" }
                span {
                    class: if unrestricted { "ext-tools unrestricted" } else { "ext-tools" },
                    Icon { name: "wrench" }
                    "{tool_summary(&entry)}"
                }
            }
            if let Some(description) = entry.extension.description() {
                div { class: "session-quote", "{description}" }
            }
        }
    }
}

/// One extension in full: what it is, what it runs, what it may call.
///
/// Read-only. Enabling and disabling live behind the row's swipe on the list,
/// which is where a state-changing control belongs (design rule 9), and goose
/// offers nothing else here that a phone should be editing.
#[component]
pub(crate) fn ExtensionDetailView() -> Element {
    let ctx = use_app_ctx();
    let showing = (ctx.extensions.open)();
    let entry = ctx
        .extensions
        .list
        .read()
        .items
        .iter()
        .find(|e| Some(e.extension.name()) == showing.as_deref())
        .cloned();

    let back = move |()| {
        let mut screen = ctx.extensions.screen;
        screen.set(Screen::List);
    };

    let Some(entry) = entry else {
        // The list was refreshed and this one is gone — removed on the server,
        // or the config was reloaded. Say so rather than showing a blank page.
        return rsx! {
            TopBar { title: "Extension", on_back: back, conn: true }
            main { class: "scroll",
                p { class: "empty", "This extension is no longer configured." }
            }
        };
    };

    let title = title_for(&entry.extension);
    let name = entry.extension.name().to_string();
    let state = RowState::of(entry.enabled, false, false);

    rsx! {
        TopBar {
            title: title.clone(),
            // The config name, when it is not already the title: it is what
            // you would grep the server's config.yaml for.
            subtitle: (name != title).then(|| name.clone()),
            on_back: back,
            conn: true,
        }
        main { class: "scroll settings",
            section { class: "card",
                div { class: "setting-list",
                    Fact {
                        name: "Status",
                        value: state.word(),
                        note: if entry.enabled {
                            "Used by new chats."
                        } else {
                            "Configured, but not started for new chats."
                        },
                    }
                    Fact {
                        name: "Kind",
                        value: kind_copy(entry.extension.transport()),
                        note: "How goose reaches it.",
                    }
                    {render_transport(&entry.extension)}
                    if let Some(timeout) = timeout_of(&entry.extension) {
                        Fact {
                            name: "Timeout",
                            value: "{timeout}s",
                            note: "How long goose waits for it to start.",
                        }
                    }
                }
            }

            section { class: "card",
                h2 { "Credentials" }
                if entry.extension.env_keys().is_empty() {
                    p { class: "hint", "This extension needs no credential." }
                } else {
                    // Names only. The values are in the server's secret store,
                    // and there is deliberately no way to ask for them from
                    // here — `config/read` on a secret returns a clear prefix,
                    // so reading one back to display it would leak it.
                    for key in entry.extension.env_keys() {
                        p { key: "{key}", class: "hint",
                            "Reads {key} from the server's keyring."
                        }
                    }
                }
            }

            section { class: "card",
                h2 { "Tools it may call" }
                if entry.extension.available_tools().is_empty() {
                    p { class: "error-box",
                        "All of them. This extension has no tool allowlist, so every tool its "
                        "MCP server publishes is callable."
                    }
                } else {
                    p { class: "modal-tool",
                        "{entry.extension.available_tools().join(\", \")}"
                    }
                }
            }

            p { class: "hint",
                "Changes here apply to new chats. A chat already open keeps the extensions "
                "it started with."
            }
        }
    }
}

/// The transport's own facts: a command line for a local server, a URL for a
/// remote one, and nothing at all for a built-in — which runs inside goose and
/// has neither.
fn render_transport(extension: &GooseExtension) -> Element {
    match extension {
        GooseExtension::Mcp { server, .. } => match server.as_ref() {
            McpServer::Stdio(stdio) => {
                let command = if stdio.args.is_empty() {
                    stdio.command.clone()
                } else {
                    format!("{} {}", stdio.command, stdio.args.join(" "))
                };
                rsx! {
                    Fact {
                        name: "Command",
                        value: command,
                        note: "Run on the goose server, not on this phone.",
                        mono: true,
                    }
                }
            }
            McpServer::Http(http) => rsx! {
                Fact {
                    name: "URL",
                    value: http.url.clone(),
                    note: "goose connects to this over streamable HTTP.",
                    mono: true,
                }
            },
        },
        GooseExtension::Builtin { .. } | GooseExtension::Platform { .. } => rsx! {},
    }
}

const fn timeout_of(extension: &GooseExtension) -> Option<u64> {
    match extension {
        GooseExtension::Builtin { timeout, .. } | GooseExtension::Mcp { timeout, .. } => *timeout,
        GooseExtension::Platform { .. } => None,
    }
}

/// Something true about an extension that no control here can change — the
/// fact half of design rule 11's two row shapes, borrowed from the settings
/// sheet so the two lists read as one grammar.
#[component]
fn Fact(
    name: String,
    value: String,
    note: String,
    /// A command line or a URL: server text, and the one thing on this screen
    /// that has to be read character by character.
    #[props(default)]
    mono: bool,
) -> Element {
    rsx! {
        div { class: "setting-row fact",
            span { class: "setting-main",
                span { class: "setting-name", "{name}" }
                span {
                    class: if mono { "setting-value ext-mono" } else { "setting-value" },
                    "{value}"
                }
                span { class: "setting-note", "{note}" }
            }
        }
    }
}

/// Pick a service, type its credential, connect it.
#[component]
fn AddSheet(sheet: Sheet) -> Element {
    let ctx = use_app_ctx();
    // One draft per credential field of the picked entry. Local to this
    // component, so it dies with the sheet — credentials are never written to
    // the persisted `Settings`, and never to `AppCtx` either.
    let mut drafts = use_signal(Vec::<String>::new);
    let busy = (ctx.extensions.busy)();
    let sticky = ctx.extensions.list.read().sticky.clone();

    // A tap outside closes — unless an add is in flight. Dismissing mid-add
    // would take the credential form with it while the calls kept running,
    // and the one thing you would want next is the field you just typed into.
    let close = move |_| {
        if !busy {
            let mut sheet = ctx.extensions.sheet;
            sheet.set(Sheet::Closed);
        }
    };

    let body = match sheet {
        Sheet::Closed => rsx! {},
        Sheet::Picking => rsx! {
            h2 { "Add an extension" }
            div { class: "setting-list",
                for (index , entry) in CATALOG.iter().enumerate() {
                    button {
                        key: "{entry.id}",
                        class: "setting-row",
                        onclick: move |_| {
                            drafts.set(vec![String::new(); CATALOG[index].secrets.len()]);
                            let mut sheet = ctx.extensions.sheet;
                            sheet.set(Sheet::Picked(index));
                        },
                        span { class: "setting-main",
                            span { class: "setting-name", "{entry.display_name}" }
                            span { class: "setting-value", "privacy tier {entry.tier}" }
                            span { class: "setting-note", "{entry.summary}" }
                        }
                        Icon { name: "chevron-right" }
                    }
                }
            }
            p { class: "hint",
                "Only services that can be finished from a phone are listed. A service "
                "that needs an OAuth consent screen cannot be: goose opens the browser "
                "on the server, and the authorization URL never reaches this app."
            }
        },
        Sheet::Picked(index) => rsx! {
            CredentialForm { index, drafts, busy }
        },
    };

    rsx! {
        div { class: "modal-backdrop", onclick: close,
            div {
                class: "modal sheet",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                if let Some(message) = sticky {
                    p { class: "error-box", "{message}" }
                }
                {body}
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
    let tools = extension.available_tools().join(", ");
    // Every field must be filled: a blank one would store an empty secret and
    // the extension would fail to start with a confusing error later.
    let ready = drafts.read().iter().all(|v| !v.trim().is_empty());

    rsx! {
        div { class: "sheet-head",
            button {
                class: "icon-btn back",
                onclick: move |_| {
                    let mut sheet = ctx.extensions.sheet;
                    sheet.set(Sheet::Picking);
                },
                Icon { name: "chevron-left" }
            }
            h2 { "{entry.display_name}" }
        }
        p { class: "hint", "{entry.summary}" }

        {credential_fields(entry, drafts)}

        p { class: "hint",
            "Stored on the goose server, in its secrets file. It is never sent back to "
            "this phone and never saved here — if you need it again, get it from the "
            "service that issued it."
        }

        p { class: "hint", "{entry.scope}" }
        p { class: "modal-tool", "{tools}" }
        p { class: "hint",
            "These are the only tools the agent can call on this service. The list is "
            "checked against the server after it is saved — if it did not stick, the "
            "service is switched back off and you get told."
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

/// One password field per secret the entry names.
fn credential_fields(entry: &'static CatalogEntry, mut drafts: Signal<Vec<String>>) -> Element {
    rsx! {
        for (position , secret) in entry.secrets.iter().enumerate() {
            label { key: "{secret.key}", class: "field-label", "{secret.key}" }
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
    }
}
