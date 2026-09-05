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
use crate::nav::Crumb;
use crate::shell::{this_device, Shell};
use crate::state::{use_app_ctx, AppCtx};
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
            // On the phone: pull, not a button. A refresh control in the bar
            // is a control that is wrong most of the time it is on screen. The
            // desktop has no pull and no button either — it re-fetches on
            // arrival, and ⌘R sends this same name
            // (`src/shell/desktop/mod.rs`).
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

    // Which row the desktop's detail column came from. Ignored on the phone,
    // where the list is not on screen beside it (`views::chrome::row_is_marked`).
    let selected = ctx.extensions.open.read().as_deref() == Some(open.as_str());

    rsx! {
        ListRow {
            icon: "package",
            title: title_for(&entry.extension),
            actions,
            selected,
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

/// What the open extension is called, once.
///
/// Read by two things that are never on screen together: the header below,
/// and — on the desktop — the window's own bar, which takes the heading out of
/// the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop/mod.rs`, `assets/desktop/`). The `None` arm is the
/// view's own fallback, and it is reachable: the list is refreshed under this
/// screen, so an extension removed on the server disappears while its detail
/// is up.
pub(crate) fn crumb(ctx: &AppCtx) -> Crumb {
    let Some(entry) = ctx
        .extensions
        .list
        .read()
        .items
        .iter()
        .find(|e| Some(e.extension.name()) == (ctx.extensions.open)().as_deref())
        .cloned()
    else {
        return Crumb::plain("Extension");
    };
    let title = title_for(&entry.extension);
    let name = entry.extension.name().to_owned();
    // The config name, when it is not already the title: it is what you would
    // grep the server's config.yaml for.
    let subtitle = (name != title).then_some(name);
    Crumb::detailed(title, subtitle)
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

    // The one expression the window's bar also reads, so an extension cannot
    // be called one thing in the pane and another in the chrome. Both arms
    // hand `TopBar` a `String` equal to the one the expression they replace
    // produced, into the same prop of the same component — so the phone's
    // captured markup does not move.
    let bar = crumb(&ctx);

    let Some(entry) = entry else {
        // The list was refreshed and this one is gone — removed on the server,
        // or the config was reloaded. Say so rather than showing a blank page.
        return rsx! {
            TopBar { title: bar.title, on_back: back, conn: true }
            main { class: "scroll",
                p { class: "empty", "This extension is no longer configured." }
            }
        };
    };

    let state = RowState::of(entry.enabled, false, false);

    rsx! {
        TopBar {
            title: bar.title,
            subtitle: bar.subtitle,
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
                        // A claim about hardware, so it names the reader's own
                        // machine rather than dropping the noun (#161): the
                        // point of the sentence is that this `npx …` is not
                        // about to be executed where you are sitting.
                        note: format!(
                            "Run on the goose server, not on {}.",
                            this_device(Shell::CURRENT),
                        ),
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
    let drafts = use_signal(Vec::<String>::new);
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
                // "this app" and not a shell branch (#161). The sentence
                // explains itself two clauses later: what cannot finish an
                // OAuth flow is this CLIENT, because goose opens the browser
                // on the server and the authorization URL never arrives here.
                // That is as true in a 1440pt window as on a phone, so
                // renaming the device would have made the desktop's copy true
                // by accident while still naming the wrong obstacle.
                "Only services that can be finished from this app are listed. A service "
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

    // ONE DRAFT PER SECRET, SIZED FROM THE ENTRY RATHER THAN FROM THE TAP THAT
    // CHOSE IT. `drafts` belongs to `AddSheet` and dies with it, while
    // `Sheet::Picked` is in `AppCtx` and does not — so leaving Extensions for
    // another destination with a service picked and coming back remounts the
    // sheet with no drafts at all, and the fields below would index past the
    // end of an empty vector. Dioxus catches a panic thrown in render and
    // draws nothing, so the symptom was not a crash: it was an empty grey
    // sheet with no fields, no title and no back button.
    //
    // Compared before it is written, which is what makes a set during render
    // safe (the same shape as `use_arrival` in `src/shell/desktop/mod.rs`):
    // the second pass finds the lengths equal and stops.
    let mut drafts = drafts;
    if drafts.peek().len() != entry.secrets.len() {
        drafts.set(vec![String::new(); entry.secrets.len()]);
    }

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
            // A claim about hardware and the strongest one on the screen —
            // "nothing you just typed lands on the machine in front of you" —
            // so the noun is the reader's own machine, chosen per shell (#161).
            "Stored on the goose server, in its secrets file. It is never sent back to "
            "{this_device(Shell::CURRENT)} and never saved here — if you need it again, "
            "get it from the service that issued it."
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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "a test that cannot fail on a bad value is worse than one that panics on it"
)]
mod tests {
    use super::{
        crumb, this_device, timeout_of, AppCtx, GooseExtension, GooseExtensionEntry, McpServer,
        Screen, Sheet, Shell,
    };
    use crate::extensions::Toggle;
    use crate::state::{ConnState, Tab};
    use crate::testkit::{render, render_seeded};

    use dioxus::html::{
        set_event_converter, PlatformEventData, SerializedFormData, SerializedHtmlEventConverter,
        SerializedMouseData,
    };
    use dioxus::prelude::*;
    use futures_util::{SinkExt as _, StreamExt as _};
    use goose_acp_client::{
        AcpClient, AcpEvent, ConnectConfig, HttpHeader, HttpMcpServer, StdioMcpServer,
    };
    use serde_json::{json, Map, Value};
    use std::any::Any;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    // -----------------------------------------------------------------------
    // Fixtures. Seeds are `fn` pointers and cannot capture, so everything a
    // seed needs is a free function it can call.

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// A local MCP server, the shape `mail-imap` has on a real server.
    fn stdio(
        name: &str,
        command: &str,
        args: &[&str],
        env: &[&str],
        tools: &[&str],
    ) -> GooseExtension {
        GooseExtension::mcp(
            McpServer::Stdio(StdioMcpServer::new(name, command, owned(args))),
            owned(env),
            "Mail the agent can read",
            owned(tools),
        )
    }

    fn mail() -> GooseExtension {
        stdio(
            "mail-imap",
            "uvx",
            &["mcp-email-server@1.4.2", "stdio"],
            &["MCP_EMAIL_SERVER_PASSWORD"],
            &["list_mailboxes", "get_emails_content"],
        )
    }

    fn remote(name: &str, url: &str, tools: &[&str]) -> GooseExtension {
        GooseExtension::mcp(
            McpServer::Http(HttpMcpServer::new(
                name,
                url,
                vec![HttpHeader::new(
                    "Authorization",
                    "Bearer ${TODOIST_API_KEY}",
                )],
            )),
            owned(&["TODOIST_API_KEY"]),
            "Tasks the agent can read",
            owned(tools),
        )
    }

    fn builtin(name: &str, display_name: Option<&str>, timeout: Option<u64>) -> GooseExtension {
        GooseExtension::Builtin {
            name: name.to_owned(),
            description: None,
            display_name: display_name.map(ToOwned::to_owned),
            timeout,
            bundled: None,
            available_tools: Some(owned(&["shell"])),
            extra: Map::new(),
        }
    }

    /// Enabled, with the config key goose derives from the name.
    fn entry(extension: GooseExtension) -> GooseExtensionEntry {
        let config_key = Some(extension.name().to_owned());
        GooseExtensionEntry {
            extension,
            enabled: true,
            config_key,
            extra: Map::new(),
        }
    }

    fn connect(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connected {
            agent: "goose".to_owned(),
        });
    }

    /// Connected, with a list the server has already answered.
    fn show(ctx: &AppCtx, items: Vec<GooseExtensionEntry>) {
        connect(ctx);
        let mut list = ctx.extensions.list;
        list.write().items = items;
    }

    fn list_view() -> Element {
        rsx! { super::ExtensionsView {} }
    }

    fn detail_view() -> Element {
        rsx! { super::ExtensionDetailView {} }
    }

    /// The first 600 characters, for a message that has to say what DID render.
    fn head(html: &str) -> &str {
        &html[..html.len().min(600)]
    }

    // -----------------------------------------------------------------------
    // Tapping.
    //
    // `src/testkit.rs` renders; it does not tap, and every control on this
    // screen is an `onclick` that `dioxus_ssr` will never fire. So the whole
    // of what the FAB, the rows, the catalogue and the sheet's own buttons DO
    // sat outside the suite: the markup could be perfect while the FAB set the
    // wrong signal.
    //
    // A tap needs the `VirtualDom` itself, which `render_seeded` does not hand
    // back, so these mount their own — six lines, and each scenario is a plain
    // `fn() -> Element` that seeds and then renders, so no harness type has to
    // be rebuilt to carry them.
    //
    // WHICH ELEMENT IS TAPPED IS NOT GUESSED AT. Dioxus addresses an element
    // by an `ElementId` assigned in creation order, and nothing in the markup
    // maps back to one — picking "the fourth listener" would be a test that
    // silently moves to another control the moment a button is added above it.
    // So every element on the screen is tapped, each in its own fresh mount,
    // and the assertion is on HOW MANY of them did the thing. "Exactly one
    // control opens the catalogue" is a stronger claim than "this one does",
    // and it cannot rot into pointing at the wrong element.

    /// An `ElementId` past the end is ignored rather than fatal
    /// (`Runtime::handle_event` does a `get`), so this only has to be larger
    /// than any screen here.
    const EVERY_ELEMENT: u32 = 120;

    /// Mount, deliver one event to one element, and hand back the markup that
    /// produced.
    ///
    /// The listener an `onclick` installs takes a `PlatformEventData`, not a
    /// `MouseData`: the shell that owns the window is what turns one into the
    /// other, through a converter registered process-wide. There is no shell
    /// here, so this registers the serialized one dioxus-html ships for
    /// exactly this — a write of a global, but an idempotent one, and nothing
    /// else in the suite delivers an event for it to affect.
    fn deliver(
        app: fn() -> Element,
        name: &str,
        data: Box<dyn Any>,
        target: u32,
        bubbles: bool,
    ) -> String {
        let _ = crate::testkit::storage_dir();
        set_event_converter(Box::new(SerializedHtmlEventConverter));
        // A handler that reaches `show_toast` spawns a `tokio::time::sleep`,
        // and Dioxus polls it during the re-render below.
        let reactor = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a current-thread runtime for the toast timer to register with");
        let _guard = reactor.enter();
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let event: Rc<dyn Any> = Rc::new(PlatformEventData::new(data));
        dom.runtime().handle_event(
            name,
            dioxus::dioxus_core::Event::new(event, bubbles),
            dioxus::dioxus_core::ElementId(target as usize),
        );
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    /// Tap one element. Without bubbling unless the question is about
    /// bubbling: a tap that walked up the tree would fire the same handler
    /// once per element inside the control, and "exactly one control does this"
    /// is the whole shape of the assertions below.
    fn tap(app: fn() -> Element, target: u32, bubbles: bool) -> String {
        deliver(
            app,
            "click",
            Box::new(SerializedMouseData::default()),
            target,
            bubbles,
        )
    }

    /// How many of the screen's elements, tapped one at a time, leave it in a
    /// state `outcome` recognises.
    fn taps_that(app: fn() -> Element, bubbles: bool, outcome: fn(&str) -> bool) -> usize {
        (1..=EVERY_ELEMENT)
            .filter(|target| outcome(&tap(app, *target, bubbles)))
            .count()
    }

    /// The same, for a keystroke into one element.
    fn type_into(app: fn() -> Element, target: u32, text: &str) -> String {
        deliver(
            app,
            "input",
            Box::new(SerializedFormData::new(text.to_owned(), Vec::new())),
            target,
            false,
        )
    }

    // -----------------------------------------------------------------------
    // The list.

    /// A goose without the extensions plane must not be offered a way to add
    /// one. The empty state, the FAB and the retry-shaped copy all belong to a
    /// server that has this feature; showing them on one that does not is an
    /// invitation to press a button whose call the server has never heard of.
    #[test]
    fn a_server_without_the_feature_is_told_apart_from_one_with_nothing_in_it() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut list = ctx.extensions.list;
            list.write().unsupported = true;
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("This goose server does not offer extensions."),
            "an unsupported server said nothing about why the screen is blank: {}",
            head(&html)
        );
        assert!(
            !html.contains("Add extension"),
            "the add button is on screen for a server with no extensions plane, \
             so pressing it opens a sheet whose Connect can only ever fail"
        );
        assert!(
            !html.contains("No extensions configured yet."),
            "an unsupported server is being described as an empty one, which \
             reads as 'add your first' rather than 'this server cannot'"
        );
    }

    /// Disconnected, the list is empty for a reason that has nothing to do with
    /// the server's config — so it says where to fix it, and the add button is
    /// dead rather than merely disappointing.
    #[test]
    fn disconnected_says_where_to_connect_and_cannot_open_the_add_sheet() {
        let html = render(list_view);
        assert!(
            html.contains("Not connected. Extensions live on the goose server"),
            "a disconnected Extensions screen is blank with no explanation: {}",
            head(&html)
        );
        assert!(
            html.contains("class=\"fab\" disabled=true"),
            "the add button is live while disconnected, so the credential form \
             opens onto a Connect that cannot reach anything: {}",
            head(&html)
        );
        assert!(
            !html.contains("No extensions configured yet."),
            "an offline phone is claiming the server has no extensions \
             configured, which it has no way of knowing"
        );
    }

    /// The connected-and-genuinely-empty arm, which is the only one entitled to
    /// that sentence.
    #[test]
    fn a_connected_server_with_nothing_configured_says_so() {
        let html = render_seeded(connect, list_view);
        assert!(
            html.contains("No extensions configured yet."),
            "a connected server with an empty list left the screen blank: {}",
            head(&html)
        );
        assert!(
            !html.contains("Not connected."),
            "the offline copy is on screen while connected"
        );
        assert!(
            !html.contains("class=\"fab\" disabled=true"),
            "the add button is disabled while connected, so an extension can \
             never be added at all"
        );
    }

    /// A fetch in flight must not be reported as an answer. "No extensions
    /// configured yet" during the first load is a lie the user acts on — it is
    /// the moment they would tap Add and create a duplicate.
    #[test]
    fn a_list_still_loading_does_not_claim_the_server_is_empty() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut list = ctx.extensions.list;
            list.write().loading = true;
        }
        let html = render_seeded(seed, list_view);
        assert!(
            !html.contains("No extensions configured yet."),
            "the empty state is shown while the first fetch is still running: {}",
            head(&html)
        );
        assert!(
            html.contains("data-refreshing=\"true\""),
            "the pull-to-refresh control has no way to know a fetch is in \
             flight, so it will never show the spinner: {}",
            head(&html)
        );
    }

    /// goose drops an extension it could not parse and carries on, so the
    /// extension is simply absent from the list. The banner is the only place a
    /// phone user could ever learn that something is missing.
    #[test]
    fn a_config_goose_could_not_load_is_named_on_screen() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut warnings = ctx.extensions.warnings;
            warnings.set(vec![
                "failed to load extension 'jira': invalid url".to_owned()
            ]);
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("class=\"banner\">failed to load extension &#39;jira&#39;"),
            "goose's own complaint about a config it could not load never \
             reached the screen, so the missing extension is invisible: {}",
            head(&html)
        );
    }

    /// A failure over a list you can still read is normally a toast — but an
    /// allowlist that did not stick is not news to miss because you were
    /// looking at the keyboard, so it stays on screen.
    #[test]
    fn a_failure_worth_keeping_stays_on_the_list() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut list = ctx.extensions.list;
            list.write().sticky = Some("Todoist came back without its allowlist.".to_owned());
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("<p class=\"error-box\">Todoist came back without its allowlist.</p>"),
            "the sticky failure is not on the list, so the one message that \
             must survive a glance away has been dropped: {}",
            head(&html)
        );
    }

    /// The row is the screen. It carries the state as a dot AND a word, the
    /// kind in words rather than in goose's enum, the allowlist, and goose's
    /// own description underneath.
    #[test]
    fn a_row_carries_the_state_the_kind_and_what_it_may_call() {
        fn seed(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("<div class=\"session-title\">Mail imap</div>"),
            "the row is titled with a raw config key rather than a readable \
             name: {}",
            head(&html)
        );
        assert!(
            html.contains("<span class=\"dot on\"></span>"),
            "an enabled extension has no on dot, so a scroll down the list \
             cannot answer which ones are live: {}",
            head(&html)
        );
        assert!(
            html.contains("<span>On · Local command</span>"),
            "the row's meta line is missing its state or is leaking goose's \
             transport enum instead of words: {}",
            head(&html)
        );
        assert!(
            html.contains("list_mailboxes, get_emails_content</span>"),
            "the allowlist — the point of the row — is not on it: {}",
            head(&html)
        );
        assert!(
            html.contains("<div class=\"session-quote\">Mail the agent can read</div>"),
            "goose's own description of the extension never reached the row"
        );
    }

    /// An empty allowlist means EVERY tool the MCP server publishes is
    /// callable. It gets the danger class and it says so in words — a blank
    /// where the tools go would read as "none", which is the opposite.
    #[test]
    fn an_extension_with_no_allowlist_is_flagged_rather_than_left_blank() {
        fn seed(ctx: &AppCtx) {
            show(ctx, vec![entry(stdio("wide-open", "uvx", &[], &[], &[]))]);
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("class=\"ext-tools unrestricted\""),
            "an extension that can call every tool its server publishes is \
             painted like a restricted one: {}",
            head(&html)
        );
        assert!(
            html.contains("every tool this server publishes"),
            "the row leaves the tools line blank for the one case where blank \
             means everything: {}",
            head(&html)
        );
    }

    /// A toggle in flight is the most recent true thing about the row, and it
    /// belongs to ONE row: the tray was opened on a single extension, and
    /// colouring the rest busy would report a change nobody asked for.
    #[test]
    fn only_the_row_whose_toggle_is_in_flight_goes_busy() {
        fn seed(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut toggle = ctx.extensions.toggle;
            toggle.set(Some(Toggle {
                key: "mail-imap".to_owned(),
                failed: false,
            }));
        }
        fn other_row(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut toggle = ctx.extensions.toggle;
            toggle.set(Some(Toggle {
                key: "todoist".to_owned(),
                failed: false,
            }));
        }
        let mine = render_seeded(seed, list_view);
        assert!(
            mine.contains("<span class=\"dot busy\"></span>")
                && mine.contains("<span>Switching… · Local command</span>"),
            "the row whose toggle is in flight is still showing the server's \
             last answer, so a tap on Disable looks like it did nothing: {}",
            head(&mine)
        );
        let theirs = render_seeded(other_row, list_view);
        assert!(
            theirs.contains("<span>On · Local command</span>"),
            "a toggle on a DIFFERENT extension turned this row busy, so one \
             tap reports a change on every row in the list: {}",
            head(&theirs)
        );
    }

    /// The toast that reported the failure fades. The dot does not — otherwise
    /// a failed enable ends with a row that looks exactly like one nobody ever
    /// touched.
    #[test]
    fn a_failed_toggle_leaves_the_row_saying_so_after_the_toast_is_gone() {
        fn seed(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut toggle = ctx.extensions.toggle;
            toggle.set(Some(Toggle {
                key: "mail-imap".to_owned(),
                failed: true,
            }));
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("<span class=\"dot err\"></span>"),
            "a failed toggle left no mark on the row: {}",
            head(&html)
        );
        assert!(
            html.contains("<span>Failed · Local command</span>"),
            "the failure is a colour and nothing else, which design rule 8 \
             exists to stop: {}",
            head(&html)
        );
    }

    /// `set-enabled` addresses an extension by its config key. An extension the
    /// server stored without one has nothing the call could name, so it gets no
    /// control at all rather than a button that always fails.
    #[test]
    fn an_extension_with_no_config_key_is_offered_no_control() {
        fn with_key(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
        }
        fn without_key(ctx: &AppCtx) {
            let mut row = entry(mail());
            row.config_key = None;
            show(ctx, vec![row]);
        }
        let addressable = render_seeded(with_key, list_view);
        assert!(
            addressable.contains("title=\"Disable\""),
            "an enabled extension the server can address offers no way to \
             switch it off: {}",
            head(&addressable)
        );
        let orphan = render_seeded(without_key, list_view);
        assert!(
            !orphan.contains("session-actions"),
            "a row goose gave no config key still shows a toggle, and pressing \
             it can only ever produce an error: {}",
            head(&orphan)
        );
    }

    /// Disabled reads Enable, not Disable. One inverted boolean here offers the
    /// user the action they have already taken.
    #[test]
    fn a_disabled_extension_offers_the_opposite_word() {
        fn seed(ctx: &AppCtx) {
            let mut row = entry(mail());
            row.enabled = false;
            show(ctx, vec![row]);
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("title=\"Enable\"") && !html.contains("title=\"Disable\""),
            "a switched-off extension is offering Disable: {}",
            head(&html)
        );
        assert!(
            html.contains("<span>Off · Local command</span>"),
            "a switched-off extension is described as On: {}",
            head(&html)
        );
    }

    /// On the desktop the list stays beside the pane, so the row it opened
    /// wears the highlight — and only while the pane is actually showing
    /// something. A row that kept the mark after the back chevron made the two
    /// columns say opposite things at once.
    #[test]
    fn the_row_is_marked_only_while_the_pane_beside_it_is_open() {
        fn open(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut tab = ctx.tab;
            tab.set(Tab::Extensions);
            let mut showing = ctx.extensions.open;
            showing.set(Some("mail-imap".to_owned()));
            let mut screen = ctx.extensions.screen;
            screen.set(Screen::Detail);
        }
        fn back_at_the_list(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut tab = ctx.tab;
            tab.set(Tab::Extensions);
            // The name outlives the screen that set it, which is the whole
            // reason the mark cannot be read off it alone.
            let mut showing = ctx.extensions.open;
            showing.set(Some("mail-imap".to_owned()));
        }
        let showing = render_seeded(open, list_view);
        assert!(
            showing.contains("class=\"session-item on\""),
            "the row the detail pane is showing is not marked, so the desktop's \
             two columns give no clue which one they are about: {}",
            head(&showing)
        );
        let closed = render_seeded(back_at_the_list, list_view);
        assert!(
            !closed.contains("session-item on"),
            "a row kept the highlight after the pane closed, so the list claims \
             to be showing something the pane says is not open: {}",
            head(&closed)
        );
    }

    // -----------------------------------------------------------------------
    // The detail.

    /// The list is refreshed under this screen, so an extension removed on the
    /// server disappears while its detail is up. Saying so beats a blank page,
    /// and the bar still needs a word in it.
    #[test]
    fn a_detail_whose_extension_vanished_says_so_rather_than_blanking() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut showing = ctx.extensions.open;
            showing.set(Some("mail-imap".to_owned()));
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            html.contains("This extension is no longer configured."),
            "an extension removed on the server leaves a blank detail page: {}",
            head(&html)
        );
        assert!(
            html.contains("<h1 class=\"title ellipsis\">Extension</h1>"),
            "the bar over a vanished extension has no title at all: {}",
            head(&html)
        );
    }

    /// The detail's whole job: what it is, what it runs, what it may call. The
    /// command line is server text and takes the mono treatment, and the
    /// credential is named and never valued.
    #[test]
    fn a_local_extension_shows_the_command_line_it_runs_on_the_server() {
        fn seed(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut showing = ctx.extensions.open;
            showing.set(Some("mail-imap".to_owned()));
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            html.contains(
                "<span class=\"setting-value ext-mono\">uvx mcp-email-server@1.4.2 stdio</span>"
            ),
            "the command goose runs is missing, incomplete or not in the mono \
             face the one read-character-by-character value needs: {}",
            head(&html)
        );
        assert!(
            html.contains("<span class=\"setting-name\">Timeout</span>")
                && html.contains("<span class=\"setting-value\">300s</span>"),
            "the start timeout is not shown, so a slow first launch looks like \
             a hang with no budget attached to it: {}",
            head(&html)
        );
        assert!(
            html.contains("Reads MCP_EMAIL_SERVER_PASSWORD from the server&#39;s keyring."),
            "the credential this extension needs is not named: {}",
            head(&html)
        );
        assert!(
            html.contains("<p class=\"modal-tool\">list_mailboxes, get_emails_content</p>"),
            "the allowlist is not spelled out in full on the one screen that \
             has room for it: {}",
            head(&html)
        );
        assert!(
            html.contains("<span class=\"subtitle ellipsis\">mail-imap</span>"),
            "the config name is missing from the bar, so there is nothing on \
             screen to grep the server's config.yaml for: {}",
            head(&html)
        );
    }

    /// A command with no arguments is the command, with no trailing space and
    /// no empty join artefact.
    #[test]
    fn a_command_with_no_arguments_is_shown_alone() {
        fn seed(ctx: &AppCtx) {
            show(
                ctx,
                vec![entry(stdio("bare", "goosed", &[], &[], &["ping"]))],
            );
            let mut showing = ctx.extensions.open;
            showing.set(Some("bare".to_owned()));
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            html.contains("<span class=\"setting-value ext-mono\">goosed</span>"),
            "a command with no arguments picked up a stray separator: {}",
            head(&html)
        );
        assert!(
            html.contains("This extension needs no credential."),
            "an extension that reads no secret says nothing about credentials, \
             leaving an empty card: {}",
            head(&html)
        );
    }

    /// A remote extension has a URL and no command. Showing the wrong one — or
    /// both — would describe a process that does not exist.
    #[test]
    fn a_remote_extension_shows_its_url_instead_of_a_command() {
        fn seed(ctx: &AppCtx) {
            show(
                ctx,
                vec![entry(remote(
                    "todoist",
                    "https://ai.todoist.net/mcp",
                    &["find-tasks"],
                ))],
            );
            let mut showing = ctx.extensions.open;
            showing.set(Some("todoist".to_owned()));
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            html.contains("<span class=\"setting-name\">URL</span>")
                && html.contains(
                    "<span class=\"setting-value ext-mono\">https://ai.todoist.net/mcp</span>"
                ),
            "the address goose dials is not on the screen that exists to say \
             what an extension is: {}",
            head(&html)
        );
        assert!(
            !html.contains("<span class=\"setting-name\">Command</span>"),
            "a remote extension is being described as a local command line"
        );
        assert!(
            html.contains("<span class=\"setting-value\">Remote</span>"),
            "an http extension is not described as remote: {}",
            head(&html)
        );
    }

    /// A builtin runs inside goose: no command, no URL, and nothing to read
    /// from the keyring. It also carries goose's own display name, which is the
    /// one that belongs on screen.
    #[test]
    fn a_builtin_has_no_transport_facts_of_its_own() {
        fn seed(ctx: &AppCtx) {
            show(
                ctx,
                vec![entry(builtin("developer", Some("Developer"), None))],
            );
            let mut showing = ctx.extensions.open;
            showing.set(Some("developer".to_owned()));
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            !html.contains("<span class=\"setting-name\">Command</span>")
                && !html.contains("<span class=\"setting-name\">URL</span>"),
            "a builtin is being shown a command line or a URL it does not \
             have: {}",
            head(&html)
        );
        assert!(
            !html.contains("<span class=\"setting-name\">Timeout</span>"),
            "a builtin with no configured timeout is showing one anyway: {}",
            head(&html)
        );
        assert!(
            html.contains("<h1 class=\"title ellipsis\">Developer</h1>"),
            "goose's own display name lost to the config key: {}",
            head(&html)
        );
        assert!(
            html.contains("<span class=\"subtitle ellipsis\">developer</span>"),
            "goose's display name replaced the config key instead of sitting \
             above it, so there is nothing on screen to grep config.yaml for: {}",
            head(&html)
        );
        assert!(
            html.contains("This extension needs no credential."),
            "a builtin claims to read a secret from the keyring: {}",
            head(&html)
        );
    }

    /// The detail is the last place to notice an extension that can call
    /// everything, and it is the place with room to say what that means.
    #[test]
    fn a_detail_with_no_allowlist_says_every_tool_is_callable() {
        fn seed(ctx: &AppCtx) {
            let mut row = entry(stdio("wide-open", "uvx", &[], &[], &[]));
            row.enabled = false;
            show(ctx, vec![row]);
            let mut showing = ctx.extensions.open;
            showing.set(Some("wide-open".to_owned()));
        }
        let html = render_seeded(seed, detail_view);
        assert!(
            html.contains("<p class=\"error-box\">All of them."),
            "an extension with no tool allowlist is described in the same calm \
             face as a restricted one: {}",
            head(&html)
        );
        assert!(
            html.contains("every tool its MCP server publishes is callable."),
            "the consequence of an empty allowlist is not spelled out: {}",
            head(&html)
        );
        assert!(
            html.contains("<span class=\"setting-value\">Off</span>")
                && html.contains("Configured, but not started for new chats."),
            "a switched-off extension is described as one new chats will use: {}",
            head(&html)
        );
    }

    /// `crumb` is read by two things that are never on screen together — this
    /// pane's own header and, on the desktop, the window's bar — so an
    /// extension cannot be called one thing in the pane and another in the
    /// chrome. The `None` arm is reachable: the list refreshes under the open
    /// detail.
    #[test]
    fn the_name_the_window_bar_reads_is_the_name_the_pane_reads() {
        fn probe() -> Element {
            let ctx = crate::state::use_app_ctx();
            let crumb = crumb(&ctx);
            let subtitle = crumb.subtitle.unwrap_or_default();
            rsx! { p { "[{crumb.title}][{subtitle}]" } }
        }
        fn open_mail(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
            let mut showing = ctx.extensions.open;
            showing.set(Some("mail-imap".to_owned()));
        }
        // A config name that already reads as a name: there is nothing for a
        // subtitle to add, so there must not be one.
        fn open_self_titled(ctx: &AppCtx) {
            show(
                ctx,
                vec![entry(remote(
                    "Todoist",
                    "https://ai.todoist.net/mcp",
                    &["find-tasks"],
                ))],
            );
            let mut showing = ctx.extensions.open;
            showing.set(Some("Todoist".to_owned()));
        }
        assert!(
            render_seeded(open_mail, probe).contains("<p>[Mail imap][mail-imap]</p>"),
            "an mcp extension's crumb lost either its readable title or the \
             config name under it"
        );
        assert!(
            render_seeded(open_self_titled, probe).contains("<p>[Todoist][]</p>"),
            "a config name that is already the title is being repeated \
             underneath itself"
        );
        assert!(
            render(probe).contains("<p>[Extension][]</p>"),
            "an extension that is gone from the list leaves the window's bar \
             with no word in it at all"
        );
    }

    /// A `platform` extension carries no timeout field at all, so the row must
    /// be absent rather than showing a guessed number. Direct, because the
    /// difference between `None` and `Some(0)` is invisible in markup.
    #[test]
    fn only_the_variants_that_carry_a_timeout_report_one() {
        assert_eq!(timeout_of(&mail()), Some(300));
        assert_eq!(timeout_of(&builtin("developer", None, Some(60))), Some(60));
        assert_eq!(timeout_of(&builtin("developer", None, None)), None);
        assert_eq!(
            timeout_of(&GooseExtension::Platform {
                name: "router".to_owned(),
                description: None,
                display_name: None,
                bundled: None,
                available_tools: None,
                extra: Map::new(),
            }),
            None,
            "a platform extension has no timeout on the wire, so reporting one \
             would put a number on screen that came from nowhere"
        );
    }

    // -----------------------------------------------------------------------
    // The add sheet.

    /// The catalogue is a decision, not a menu: each entry shows the privacy
    /// tier and what it is for, and the screen says out loud why an OAuth
    /// service will never appear on it.
    #[test]
    fn the_catalogue_names_every_service_with_the_tier_it_costs() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut sheet = ctx.extensions.sheet;
            sheet.set(Sheet::Picking);
        }
        let html = render_seeded(seed, list_view);
        for name in ["Todoist", "Mail (IMAP)", "Calendar (CalDAV)"] {
            assert!(
                html.contains(name),
                "{name} is missing from the catalogue sheet, so it cannot be \
                 added from the phone at all: {}",
                head(&html)
            );
        }
        assert!(
            html.matches("privacy tier").count() == 3,
            "a catalogue entry is on screen without the tier it costs, which \
             is the fact that makes it a decision: {}",
            head(&html)
        );
        assert!(
            html.contains("A service that needs an OAuth consent screen cannot be"),
            "the catalogue is silent about why it is so short, so its absences \
             read as an oversight: {}",
            head(&html)
        );
    }

    /// Picking a service shows the credential fields it needs, what connecting
    /// will permit, and nothing that could read a secret back. The fields are
    /// password fields with autocomplete off — every one of them, including the
    /// URL and the username, because they are all stored as secrets.
    #[test]
    fn a_picked_service_asks_for_its_credentials_and_shows_what_it_will_permit() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut sheet = ctx.extensions.sheet;
            // CalDAV: the three-field entry, so a form that only ever renders
            // its first secret is caught.
            sheet.set(Sheet::Picked(2));
        }
        let html = render_seeded(seed, list_view);
        for key in ["CALDAV_BASE_URL", "CALDAV_USERNAME", "CALDAV_PASSWORD"] {
            assert!(
                html.contains(&format!("<label class=\"field-label\">{key}</label>")),
                "{key} has no field, so the extension would be added without \
                 the credential it needs: {}",
                head(&html)
            );
        }
        assert_eq!(
            html.matches("type=\"password\"").count(),
            3,
            "a credential field is not a password field — a secret is on screen \
             in the clear: {}",
            head(&html)
        );
        assert_eq!(
            html.matches("autocomplete=\"off\"").count(),
            3,
            "a credential field is offering the keyboard's autofill, which is \
             how the wrong service's token gets stored: {}",
            head(&html)
        );
        assert!(
            html.contains("<p class=\"modal-tool\">list-calendars, list-events, list-todos</p>"),
            "the sheet does not say which tools connecting will permit: {}",
            head(&html)
        );
        assert!(
            html.contains("Read-only: lists calendars, events and todos."),
            "the scope sentence that justifies the allowlist is missing: {}",
            head(&html)
        );
        assert!(
            html.contains(&format!(
                "It is never sent back to {} and never saved here",
                this_device(Shell::CURRENT)
            )),
            "the one-way promise about the credential is not made where the \
             credential is typed: {}",
            head(&html)
        );
        assert!(
            !html.contains("this phone"),
            "a 1440x860 window is being promised that a secret will not be \
             sent to a phone it is not on: {}",
            head(&html)
        );
    }

    /// THE SHEET SURVIVES ITS OWN VIEW. `Sheet::Picked` lives in `AppCtx` while
    /// the drafts live in the sheet's component, so leaving Extensions with a
    /// service picked and coming back mounts the form with no drafts at all.
    /// Before the fields were sized from the entry, that indexed past the end
    /// of an empty vector; Dioxus catches a panic in render and draws nothing,
    /// so the symptom was an empty grey sheet with no fields, no heading and no
    /// way back — not a crash anyone could report.
    #[test]
    fn a_sheet_remounted_on_a_picked_service_still_has_its_fields() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut sheet = ctx.extensions.sheet;
            sheet.set(Sheet::Picked(0));
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("<h2>Todoist</h2>"),
            "a sheet mounted straight onto a picked service rendered empty — \
             the form panicked during render and Dioxus swallowed it: {}",
            head(&html)
        );
        assert!(
            html.contains("<label class=\"field-label\">TODOIST_API_KEY</label>"),
            "the remounted form has no credential field: {}",
            head(&html)
        );
        assert!(
            html.contains("<p class=\"hint\">Fill in every field to continue.</p>"),
            "the remounted form counts its zero drafts as filled in, so Connect \
             is live and would store an empty secret: {}",
            head(&html)
        );
        assert!(
            html.contains("class=\"btn primary grow\" disabled=true"),
            "Connect is pressable with nothing typed, which stores an empty \
             secret and fails later with an error about the wrong thing: {}",
            head(&html)
        );
    }

    /// An add in flight says so on the button and cannot be started twice —
    /// storing the same secret twice is harmless, but a second `add` while the
    /// first is verifying is how the allowlist check races itself.
    #[test]
    fn an_add_in_flight_says_so_and_cannot_be_pressed_again() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut sheet = ctx.extensions.sheet;
            sheet.set(Sheet::Picked(0));
            let mut busy = ctx.extensions.busy;
            busy.set(true);
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("Connecting…"),
            "an add in flight leaves the button reading Connect, so it looks \
             like the tap was missed: {}",
            head(&html)
        );
        assert!(
            html.contains("class=\"btn primary grow\" disabled=true"),
            "Connect is still pressable while an add is running: {}",
            head(&html)
        );
        assert!(
            !html.contains("Fill in every field to continue."),
            "the sheet is nagging about empty fields while it is busy using \
             them: {}",
            head(&html)
        );
    }

    /// The sheet reports its own failures. The list is behind a backdrop while
    /// the sheet is up, so a message left on the list would be invisible at the
    /// exact moment it matters.
    #[test]
    fn the_sheets_own_failure_is_shown_inside_the_sheet() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut sheet = ctx.extensions.sheet;
            sheet.set(Sheet::Picked(0));
            let mut list = ctx.extensions.list;
            list.write().sticky = Some("Could not store TODOIST_API_KEY: timed out".to_owned());
        }
        let html = render_seeded(seed, list_view);
        let sheet_start = html
            .find("class=\"modal sheet\"")
            .expect("the sheet is not on screen at all");
        assert!(
            html[sheet_start..].contains("Could not store TODOIST_API_KEY: timed out"),
            "the failure is outside the sheet, behind the backdrop that covers \
             the list: {}",
            head(&html[sheet_start..])
        );
    }

    /// A catalogue index the table no longer has renders nothing rather than
    /// panicking — the sheet is the one piece of state that outlives its view,
    /// so a shrunk `CATALOG` reaches this with a stale index.
    #[test]
    fn a_catalogue_index_that_no_longer_exists_renders_no_form() {
        fn seed(ctx: &AppCtx) {
            connect(ctx);
            let mut sheet = ctx.extensions.sheet;
            sheet.set(Sheet::Picked(99));
        }
        let html = render_seeded(seed, list_view);
        assert!(
            html.contains("class=\"modal sheet\""),
            "the sheet vanished entirely rather than rendering empty, which is \
             what a panic swallowed during render looks like: {}",
            head(&html)
        );
        assert!(
            !html.contains("field-label"),
            "a credential field rendered for a catalogue entry that does not \
             exist: {}",
            head(&html)
        );
    }

    /// A closed sheet is not rendered at all. It is at the view's root — never
    /// inside `TopBar`, whose `backdrop-filter` would trap a fixed descendant
    /// in a 94px pill — and an always-rendered backdrop would swallow every tap
    /// on the list behind it.
    #[test]
    fn a_closed_sheet_puts_no_backdrop_over_the_list() {
        fn seed(ctx: &AppCtx) {
            show(ctx, vec![entry(mail())]);
        }
        let html = render_seeded(seed, list_view);
        assert!(
            !html.contains("modal-backdrop"),
            "the sheet's backdrop is over the list while the sheet is closed, \
             so nothing on the list can be tapped: {}",
            head(&html)
        );
    }

    // -----------------------------------------------------------------------
    // What the controls do when they are tapped.

    /// The FAB is the only way into the add flow, and it clears the last
    /// failure on the way in — a sticky error about the previous attempt would
    /// otherwise greet the next one from the top of a fresh sheet.
    #[test]
    fn the_add_button_opens_the_catalogue_and_clears_the_last_failure() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                let mut list = ctx.extensions.list;
                list.write().sticky = Some("Could not add Todoist: timed out".to_owned());
            });
            rsx! { super::ExtensionsView {} }
        }
        fn opened(html: &str) -> bool {
            html.contains("<h2>Add an extension</h2>")
        }
        assert_eq!(
            taps_that(screen, false, opened),
            1,
            "the Extensions screen does not have exactly one control that \
             opens the catalogue — either the add button no longer opens it, \
             or something else on the screen does too"
        );
        let after = (1..=EVERY_ELEMENT)
            .map(|target| tap(screen, target, false))
            .find(|html| opened(html))
            .expect("no tap opened the catalogue");
        assert!(
            !after.contains("Could not add Todoist"),
            "the previous attempt's failure is still on screen behind the new \
             sheet: {}",
            head(&after)
        );
    }

    /// Tapping a row opens that extension. The whole row is the target (design
    /// rule 9), so the count is the row plus nothing else — and the detail it
    /// opens must be the row that was tapped, not whichever one the signal
    /// happened to hold.
    #[test]
    fn tapping_a_row_opens_that_extension_and_not_another() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                show(
                    &ctx,
                    vec![
                        entry(mail()),
                        entry(remote(
                            "todoist",
                            "https://ai.todoist.net/mcp",
                            &["find-tasks"],
                        )),
                    ],
                );
                let mut tab = ctx.tab;
                tab.set(Tab::Extensions);
            });
            // The list and whatever it opened, so one tap can be read end to
            // end: the row sets the signals, the pane reads them.
            rsx! {
                super::ExtensionsView {}
                if (ctx.extensions.screen)() == Screen::Detail {
                    super::ExtensionDetailView {}
                }
            }
        }
        let opened: Vec<String> = (1..=EVERY_ELEMENT)
            .map(|target| tap(screen, target, false))
            .filter(|html| html.contains("Changes here apply to new chats."))
            .collect();
        assert_eq!(
            opened.len(),
            2,
            "a two-row list does not have exactly two controls that open a \
             detail — a row has stopped opening, or something that is not a \
             row opens one"
        );
        assert!(
            opened.iter().any(|html| html.contains(
                "<span class=\"setting-value ext-mono\">uvx mcp-email-server@1.4.2 stdio</span>"
            )),
            "no row opens the mail extension's own detail"
        );
        assert!(
            opened.iter().any(|html| html.contains(
                "<span class=\"setting-value ext-mono\">https://ai.todoist.net/mcp</span>"
            )),
            "both rows open the same extension, so which one you tapped makes \
             no difference to what you get"
        );
    }

    /// The detail's back chevron returns to the list. Without it the only way
    /// off the screen is the drawer, which on the phone means the detail is a
    /// dead end.
    #[test]
    fn the_detail_goes_back_to_the_list() {
        fn screen() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                show(&ctx, vec![entry(mail())]);
                let mut showing = ctx.extensions.open;
                showing.set(Some("mail-imap".to_owned()));
                let mut sc = ctx.extensions.screen;
                sc.set(Screen::Detail);
            });
            rsx! {
                super::ExtensionDetailView {}
                if (ctx.extensions.screen)() == Screen::List {
                    p { "back at the list" }
                }
            }
        }
        assert_eq!(
            taps_that(screen, false, |html| html.contains("back at the list")),
            1,
            "the open extension has no single control that returns to the \
             list, so the back chevron is either gone or no longer the only \
             thing that goes back"
        );
    }

    /// Picking a service from the catalogue moves the sheet on to its
    /// credential form, and the back chevron there returns to the catalogue —
    /// picking the wrong service must not mean abandoning the whole add.
    #[test]
    fn the_catalogue_walks_forward_to_a_service_and_back_again() {
        fn catalogue() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                let mut sheet = ctx.extensions.sheet;
                sheet.set(Sheet::Picking);
            });
            rsx! { super::ExtensionsView {} }
        }

        // The sheet on its own, wired to the signal exactly as `ExtensionsView`
        // wires it. Not the whole screen, because the FAB is still in the tree
        // behind the backdrop and it sets `Picking` too — a count over the
        // whole screen would be counting a control no tap can reach.
        fn form() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                let mut sheet = ctx.extensions.sheet;
                sheet.set(Sheet::Picked(1));
            });
            rsx! { super::AddSheet { sheet: (ctx.extensions.sheet)() } }
        }

        let picked: Vec<String> = (1..=EVERY_ELEMENT)
            .map(|target| tap(catalogue, target, false))
            .filter(|html| html.contains("class=\"field-label\""))
            .collect();
        assert_eq!(
            picked.len(),
            crate::extensions::CATALOG.len(),
            "the catalogue does not have exactly one control per service that \
             reaches its credential form"
        );
        assert!(
            picked.iter().any(|html| html.contains("TODOIST_API_KEY"))
                && picked
                    .iter()
                    .any(|html| html.contains("MCP_EMAIL_SERVER_PASSWORD"))
                && picked.iter().any(|html| html.contains("CALDAV_PASSWORD")),
            "two catalogue rows lead to the same form, so picking a service \
             does not decide which credential you are typing"
        );

        assert_eq!(
            taps_that(form, false, |html| html
                .contains("<h2>Add an extension</h2>")),
            1,
            "the credential form has no single way back to the catalogue, so \
             picking the wrong service costs you the whole sheet"
        );
    }

    /// A tap outside the sheet dismisses it; a tap ON the sheet must not. The
    /// backdrop covers the list, so without the modal's own
    /// `stop_propagation` every tap into the credential field would close the
    /// form under the keyboard.
    #[test]
    fn only_a_tap_outside_the_sheet_dismisses_it() {
        fn sheet() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                let mut sheet = ctx.extensions.sheet;
                sheet.set(Sheet::Picked(0));
            });
            rsx! { super::ExtensionsView {} }
        }
        // Bubbling, because that is the question: a tap lands on a field or a
        // label inside the sheet and walks up towards the backdrop.
        assert_eq!(
            taps_that(sheet, true, |html| !html.contains("modal-backdrop")),
            1,
            "either nothing dismisses the sheet, or a tap inside it reaches \
             the backdrop and takes the half-typed credential with it"
        );
    }

    /// An add in flight cannot be dismissed. The calls keep running whatever
    /// the sheet does, and the one thing you would want back is the field you
    /// just typed into.
    #[test]
    fn a_sheet_with_an_add_in_flight_cannot_be_tapped_away() {
        fn busy_sheet() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                let mut sheet = ctx.extensions.sheet;
                sheet.set(Sheet::Picked(0));
                let mut busy = ctx.extensions.busy;
                busy.set(true);
            });
            rsx! { super::ExtensionsView {} }
        }
        assert_eq!(
            taps_that(busy_sheet, true, |html| !html.contains("modal-backdrop")),
            0,
            "a tap dismissed the sheet while an add was in flight, so the \
             credential form is gone while the calls that need it keep running"
        );
    }

    /// Typing fills the draft for the field that was typed into — and only
    /// that one. Three `CalDAV` fields writing to one slot is a form that looks
    /// filled in and stores the same value three times.
    #[test]
    fn a_keystroke_reaches_the_field_it_was_typed_into() {
        fn form() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                connect(&ctx);
                let mut sheet = ctx.extensions.sheet;
                sheet.set(Sheet::Picked(2));
            });
            rsx! { super::ExtensionsView {} }
        }
        // Not a CalDAV-shaped string: the entry's own prompt already carries
        // an example URL, and a filter that matched it would count every
        // element on the screen as a field.
        const TYPED: &str = "hunter2-typed-here";
        let filled: Vec<String> = (1..=EVERY_ELEMENT)
            .map(|target| type_into(form, target, TYPED))
            .filter(|html| html.contains(&format!("value=\"{TYPED}\"")))
            .collect();
        assert_eq!(
            filled.len(),
            3,
            "CalDAV asks for three secrets and the form does not have three \
             fields that take a keystroke"
        );
        for html in &filled {
            assert_eq!(
                html.matches(&format!("value=\"{TYPED}\"")).count(),
                1,
                "one keystroke landed in more than one field, so the three \
                 CalDAV secrets would be stored as copies of whichever was \
                 typed last: {}",
                head(html)
            );
            assert!(
                html.contains("Fill in every field to continue."),
                "one field of three is enough to enable Connect, so the \
                 extension is added with two empty secrets: {}",
                head(html)
            );
        }
        assert!(
            filled[0] != filled[1] && filled[1] != filled[2] && filled[0] != filled[2],
            "two of the three fields produced identical markup, so they are \
             writing to the same draft and the URL, the username and the \
             password are one value"
        );
    }

    /// Connect reaches `add_from_catalog`, and with no client behind the
    /// screen that says so rather than reporting a service connected. A
    /// "Todoist connected" toast over a server that was never asked is the
    /// worst outcome this whole surface has: the user stops checking.
    #[test]
    fn connect_reports_the_missing_connection_rather_than_a_success() {
        fn form() -> Element {
            let ctx = crate::state::use_app_ctx_provider();
            use_hook(|| {
                // Connected as far as the UI knows — the badge is on — but no
                // client behind it, which is exactly the window between a URL
                // being accepted and the socket being up.
                connect(&ctx);
                let mut sheet = ctx.extensions.sheet;
                sheet.set(Sheet::Picked(0));
            });
            rsx! {
                super::ExtensionsView {}
                if let Some(toast) = (ctx.toast)() {
                    p { class: "probe-toast", "{toast}" }
                }
            }
        }
        assert_eq!(
            taps_that(form, false, |html| html.contains("probe-toast")),
            1,
            "the credential form does not have exactly one control that tries \
             to connect the service"
        );
        let reported = (1..=EVERY_ELEMENT)
            .map(|target| tap(form, target, false))
            .find(|html| html.contains("probe-toast"))
            .expect("nothing on the form tried to connect");
        assert!(
            reported.contains("Not connected — reconnect in Settings"),
            "Connect reported something other than the missing connection, so \
             a service can read as added when nothing was ever stored: {}",
            head(&reported)
        );
        assert!(
            reported.contains("class=\"modal sheet\""),
            "the sheet closed on a Connect that never reached the server, so \
             the credential that was typed is gone: {}",
            head(&reported)
        );
    }

    // -----------------------------------------------------------------------
    // Against a real server.
    //
    // Everything above seeds the list by hand, which leaves the one thing this
    // screen does on arrival — fetch — outside the suite entirely. `AcpClient`
    // has no constructor but `connect`, so the only way to drive it is to put a
    // server in front of it: a plain-`ws://` JSON-RPC listener on a loopback
    // port. `ws_url` only reaches for TLS on an `https://` base, so `http://`
    // here means no certificate and no fingerprint.

    thread_local! {
        /// The context [`Live`]'s mount built, so a test can reach it.
        static PUBLISHED: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
        /// The client [`Live`] connected, for [`Live::connect`] to hand over.
        static CLIENT: RefCell<Option<AcpClient>> = const { RefCell::new(None) };
    }

    /// The Extensions list over a real context, published so a test can
    /// connect it after the fact — which is the case the effect exists for.
    fn published_list() -> Element {
        let ctx = crate::state::use_app_ctx_provider();
        use_hook(move || PUBLISHED.with(|slot| *slot.borrow_mut() = Some(ctx)));
        rsx! { super::ExtensionsView {} }
    }

    /// One configured extension and one config goose could not load, in the
    /// shape `config/extensions/list` answers with.
    fn wire(method: &str, _params: &Value) -> Value {
        assert_eq!(method, "_goose/unstable/config/extensions/list");
        json!({
            "extensions": [entry(mail())],
            "warnings": ["failed to load extension 'jira': invalid url"],
        })
    }

    struct Live {
        dom: VirtualDom,
        rt: tokio::runtime::Runtime,
        ctx: AppCtx,
        /// The connection's event stream, parked here for its lifetime: the
        /// client's actor gives up the socket when this end goes away.
        _events: tokio::sync::mpsc::Receiver<AcpEvent>,
    }

    impl Live {
        /// A mounted Extensions screen, disconnected, with a live client
        /// waiting in [`CLIENT`] for [`Self::connect`] to hand it over.
        fn new(script: fn(&str, &Value) -> Value) -> Self {
            let _ = crate::testkit::storage_dir();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("a tokio runtime for the socket to live on");
            let base_url = serve(&rt, script);
            let cfg = ConnectConfig {
                base_url,
                secret: String::new(),
                fingerprint: None,
            };
            let (client, events, _info) = rt
                .block_on(AcpClient::connect(&cfg))
                .expect("the mock server accepted the handshake");
            CLIENT.with(|slot| *slot.borrow_mut() = Some(client));
            let mut dom = VirtualDom::new(published_list);
            rt.block_on(async { dom.rebuild_in_place() });
            let ctx = PUBLISHED
                .with(|slot| *slot.borrow())
                .expect("the mount rendered, so it published its context");
            Self {
                dom,
                rt,
                ctx,
                _events: events,
            }
        }

        /// Read or write the context. Signals belong to the virtual DOM's
        /// runtime and panic outside it, so every touch goes through here.
        fn with<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
            let ctx = self.ctx;
            self.dom.in_runtime(|| f(&ctx))
        }

        /// The connection coming up under a screen that is already on.
        fn connect(&mut self) {
            self.with(|ctx| {
                ctx.client
                    .clone()
                    .set(CLIENT.with(|slot| slot.borrow().clone()));
                connect(ctx);
            });
            self.settle();
        }

        /// Let the queued Dioxus tasks — and the socket under them — run.
        ///
        /// Bounded on purpose: a screen whose tasks never settle must not hang
        /// the suite, and 400ms of idle against a loopback round trip is room
        /// to spare rather than a wall-clock wait.
        fn settle(&mut self) {
            let dom = &mut self.dom;
            self.rt.block_on(async {
                for _ in 0..40 {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
                    dom.render_immediate_to_vec();
                }
            });
        }

        fn markup(&self) -> String {
            dioxus_ssr::render(&self.dom)
        }
    }

    /// A JSON-RPC server on a loopback port, answering `script`.
    fn serve(rt: &tokio::runtime::Runtime, script: fn(&str, &Value) -> Value) -> String {
        let listener = rt.block_on(async {
            TcpListener::bind("127.0.0.1:0")
                .await
                .expect("a loopback port")
        });
        let port = listener
            .local_addr()
            .expect("the listener's own address")
            .port();
        rt.spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move { session_loop(socket, script).await });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    async fn session_loop(socket: tokio::net::TcpStream, script: fn(&str, &Value) -> Value) {
        let Ok(ws) = tokio_tungstenite::accept_async(socket).await else {
            return;
        };
        let (mut sink, mut stream) = ws.split();
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            let Some(id) = frame.get("id").cloned() else {
                continue;
            };
            let method = frame
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            let result = if method == "initialize" {
                json!({
                    "protocolVersion": 1,
                    "agentInfo": { "name": "mock", "version": "0" },
                })
            } else {
                script(&method, &params)
            };
            let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
            if sink
                .send(Message::Text(reply.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    }

    /// THE EFFECT IS THE ONLY FETCH THIS SCREEN HAS. There is no refresh
    /// button and no timer — the phone pulls, the desktop re-fetches on
    /// arrival — so a screen that is already up when the connection comes back
    /// has to notice by itself. Break the effect and Extensions stays
    /// permanently empty for anyone who opened it before typing a server URL,
    /// and the FAB above it stays dead with no way to wake it.
    #[test]
    fn a_screen_opened_before_connecting_fills_itself_in_when_the_socket_comes_up() {
        let mut live = Live::new(wire);
        live.settle();
        let before = live.markup();
        assert!(
            before.contains("Not connected. Extensions live on the goose server"),
            "the screen did not start disconnected, so what follows proves \
             nothing about the connection arriving"
        );
        assert!(
            !before.contains("Mail imap"),
            "the list was already full before anything was connected"
        );

        live.connect();
        let after = live.markup();
        assert!(
            after.contains("<div class=\"session-title\">Mail imap</div>"),
            "the connection came up under an open Extensions screen and \
             nothing fetched, so the list stays empty and the add button above \
             it is the only live control on a screen that knows nothing"
        );
        assert!(
            after.contains("failed to load extension &#39;jira&#39;"),
            "the warnings goose reported alongside the list never reached the \
             screen, so an extension it could not parse is invisible — and \
             they are set from inside the same fetch, so this is the fetch \
             half-landing rather than the banner being wrong"
        );
        assert!(
            !after.contains("No extensions configured yet."),
            "the screen is claiming the server has nothing configured while \
             showing a row it just fetched"
        );
    }
}
