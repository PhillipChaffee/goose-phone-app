//! Extensions: the services the agent is plugged into, and the one screen
//! that can turn one off when it misbehaves.
//!
//! An "extension" is goose's word for an MCP server it has been told about.
//! This destination lists the ones configured on the server, lets you switch
//! one on or off, and adds a new one from a small built-in catalogue.
//!
//! It is called Extensions and not "Connect" because Connect is not one of
//! goose's own names for this, and it collides with this app's connection
//! vocabulary: the drawer would read "Connect" directly above a `ConnBadge`
//! reading "connecting…" and a screen reading "Not connected — reconnect in
//! Settings".
//!
//! Two things shape every decision in here, and both of them are about
//! failing closed.
//!
//! **The tool allowlist is one word away from being a no-op.** `available_tools`
//! is `snake_case` on the ACP wire while its neighbours are camelCase; goose sets
//! no `deny_unknown_fields`, so a camelCase spelling is dropped in silence, and
//! a dropped allowlist means *every* tool the MCP server publishes is callable.
//! So nothing here trusts an `add` that returned OK: `add_extension_verified`
//! adds every extension switched OFF, re-lists and compares, and only switches
//! it on once the allowlist has come back intact. An unrestricted extension is
//! therefore never live, not even for the round trip it would take to notice
//! and undo.
//!
//! **A credential goes one way only.** Secrets are written with
//! `config/upsert` + `isSecret`, land in the server's `secrets.yaml`, and are
//! never read back — `config/read` on a secret returns a clear prefix, so
//! "check what we stored" is a leak. Verification is a handshake instead:
//! bring the extension up in a session and let goose fail if the credential is
//! missing. That handshake always runs — with no chat open, a throwaway
//! session is created for it and deleted again — because "connected" has to
//! mean the service answered, not that a token was filed away. Nothing typed
//! into a credential field is persisted on the phone, and there is
//! deliberately no reveal control.
//!
//! OAuth-based services are absent from the catalogue on purpose. goose binds
//! the redirect URI on the agent host, never puts the authorization URL in an
//! ACP message, and refuses URL-mode elicitation at the ACP bridge — so an
//! OAuth connector cannot be finished from a phone at all. What works from a
//! phone is a bearer token or an app password.

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{
    AcpError, GooseExtension, GooseExtensionEntry, HttpHeader, HttpMcpServer, McpServer,
    StdioMcpServer,
};

use crate::state::{load_remote, show_toast, AppCtx, Remote};

/// Extensions' own back stack, so the drawer can leave and come back to it.
///
/// Two screens, not three: adding is an overlay over the list rather than a
/// screen of its own, because it is a decision you finish or abandon in one
/// sitting and never something to navigate *back* to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    List,
    Detail,
}

/// The dump key for each screen.
///
/// A free function over the plain enum rather than a closure over signals, so
/// the resolution can be tested without a Dioxus runtime — the same reason
/// `nav::chats_key` and `SettingRow::select` are free functions.
pub(crate) const fn dump_key(screen: Screen) -> &'static str {
    match screen {
        Screen::List => "extensions",
        Screen::Detail => "extensions-detail",
    }
}

/// The add flow, which is a sheet over the list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sheet {
    Closed,
    /// The catalogue, with nothing chosen yet.
    Picking,
    /// One catalogue entry and its credential fields.
    Picked(usize),
}

/// The toggle the user last asked for: still in flight, or the one that
/// failed.
///
/// One slot rather than one flag per row, because there is only ever one —
/// the tray closes when the row is tapped, and a second toggle replaces the
/// first as the thing the screen is reporting on.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Toggle {
    pub key: String,
    pub failed: bool,
}

/// Everything the Extensions screens keep.
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub screen: Signal<Screen>,
    pub list: Signal<Remote<GooseExtensionEntry>>,
    /// Config-file problems goose reported while loading. Worth a banner: an
    /// extension it could not parse is simply *missing* from the list, so
    /// this is the only place a phone user could ever learn about it.
    pub warnings: Signal<Vec<String>>,
    /// Which extension Detail is showing, by `GooseExtension::name` — the one
    /// identifier every variant has (a builtin may have no `config_key`).
    pub open: Signal<Option<String>>,
    pub toggle: Signal<Option<Toggle>>,
    pub sheet: Signal<Sheet>,
    /// An add is in flight: credentials are being stored, the extension
    /// verified, the handshake run.
    pub busy: Signal<bool>,
}

/// Build the signals. Hooks inside, so it is called exactly once, from
/// `use_app_ctx_provider`.
pub(crate) fn use_ctx() -> Ctx {
    Ctx {
        screen: use_signal(|| Screen::List),
        list: use_signal(Remote::new),
        warnings: use_signal(Vec::new),
        open: use_signal(|| None),
        toggle: use_signal(|| None),
        sheet: use_signal(|| Sheet::Closed),
        busy: use_signal(|| false),
    }
}

// ---------------------------------------------------------------------------
// What a row says about itself.

/// The four things a row can be, as a dot and a word.
///
/// Never the raw enum and never a switch: a switch carries no text and no
/// icon, so `docs/audit.js`'s contrast walk skips it entirely and its
/// legibility becomes unverifiable — the wrong trade in a repo whose whole
/// quality gate is that audit. Words and a dot instead (design rule 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowState {
    On,
    Off,
    /// A toggle is in flight. Deliberately not an optimistic flip: switching
    /// an extension on spawns a subprocess or dials a remote MCP server and
    /// can fail *after* the call returns success, so the dot goes busy and
    /// the server is asked again rather than guessed at.
    Busy,
    /// The last toggle for this row came back an error, and the row is
    /// showing whatever the server says it still is.
    Failed,
}

impl RowState {
    /// What the row is, given the server's answer and what we last asked for.
    pub(crate) const fn of(enabled: bool, busy: bool, failed: bool) -> Self {
        if busy {
            Self::Busy
        } else if failed {
            Self::Failed
        } else if enabled {
            Self::On
        } else {
            Self::Off
        }
    }

    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::On => "On",
            Self::Off => "Off",
            Self::Busy => "Switching…",
            Self::Failed => "Failed",
        }
    }

    /// The dot's class, from the four `main.css` already defines.
    pub(crate) const fn dot(self) -> &'static str {
        match self {
            Self::On => "dot on",
            Self::Off => "dot off",
            Self::Busy => "dot busy",
            Self::Failed => "dot err",
        }
    }
}

/// What kind of thing an extension is, in words rather than in goose's enum.
///
/// `streamable_http` is goose's own spelling in its config file while the ACP
/// discriminator is plain `http`; both mean a server somewhere else, so both
/// land on the same sentence. The fallback exists because `transport()`
/// returns a `&'static str` and a newer goose may add a fifth kind — showing
/// "Extension" beats showing a word from a schema (design rule 8).
pub(crate) fn kind_copy(transport: &str) -> &'static str {
    match transport {
        "stdio" => "Local command",
        "http" | "streamable_http" => "Remote",
        "builtin" | "platform" => "Built in",
        _ => "Extension",
    }
}

/// The row's second line: what state it is in, then what kind of thing it is.
pub(crate) fn meta_line(state: RowState, transport: &str) -> String {
    format!("{} · {}", state.word(), kind_copy(transport))
}

/// A one-line description of what an extension may call, for the list row.
///
/// An empty allowlist is not "nothing" — it is *everything*, and it says so.
pub(crate) fn tool_summary(entry: &GooseExtensionEntry) -> String {
    let tools = entry.extension.available_tools();
    match tools.len() {
        0 => "every tool this server publishes".to_string(),
        1 => tools[0].clone(),
        n if n <= 3 => tools.join(", "),
        n => format!("{}, and {} more", tools[..2].join(", "), n - 2),
    }
}

/// The name to put on the row: goose's `display_name` where it has one, and
/// otherwise its config name made readable.
///
/// A config name is a key — `mail-imap`, `computer_controller` — and a list of
/// keys reads as a config file rather than as a list of services.
pub(crate) fn title_for(extension: &GooseExtension) -> String {
    display_name(extension).map_or_else(|| humanize(extension.name()), ToString::to_string)
}

/// `display_name` is not on the `GooseExtension` accessors, because only two
/// of the three variants carry one — `mcp` has none, and its name is the MCP
/// server's own.
fn display_name(extension: &GooseExtension) -> Option<&str> {
    match extension {
        GooseExtension::Builtin { display_name, .. }
        | GooseExtension::Platform { display_name, .. } => display_name.as_deref(),
        GooseExtension::Mcp { .. } => None,
    }
}

/// `mail-imap` → `Mail imap`. Separators become spaces and the first letter
/// is capitalised; nothing else is guessed at, because a wrong expansion of
/// someone's extension name is worse than a plain one.
///
/// `pub(crate)` because a scheduled job's id is the other thing that reaches
/// a screen as a file stem — `nightly-dependency-audit` — and a second copy
/// of a string-shaping rule is a second thing to keep in step. The copy in
/// `views/session_settings.rs` splits only on `_`, so it is not the one to
/// share.
pub(crate) fn humanize(raw: &str) -> String {
    let mut words = raw.replace(['_', '-'], " ");
    if let Some(first) = words.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    words
}

// ---------------------------------------------------------------------------
// The catalogue.

/// One credential a catalogue entry needs, named but never valued.
pub(crate) struct CatalogSecret {
    /// The environment variable the MCP server itself reads. It is the
    /// server's own name, not ours: `${VAR}` substitution applies only to a
    /// remote extension's url/headers/socket, never to stdio env, and `cwd` is
    /// hardcoded `None` over ACP — so a wrapper script cannot rename it either.
    pub key: &'static str,
    pub prompt: &'static str,
}

/// A service the phone can finish connecting on its own.
///
/// Mirrors one `acp_extension:` block from a connector manifest
/// (`config/connectors/*.yaml` in the brain repo), which is where these tool
/// lists come from and where the vetting that justifies them is written down.
pub(crate) struct CatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub summary: &'static str,
    pub secrets: &'static [CatalogSecret],
    /// Privacy tier per the brain repo's `docs/privacy.md`. Shown because
    /// "this connector reads your mail" is a decision, not a detail.
    pub tier: u8,
    /// What the allowlist below does and does not permit, in a sentence.
    pub scope: &'static str,
    /// Builds the ACP payload. A function rather than a value because
    /// [`GooseExtension`] owns its strings and this table is `static`.
    pub build: fn() -> GooseExtension,
}

/// The catalogue.
///
/// Every entry is `first_run_auth: phone_secret` — one credential you can type
/// on a phone, no browser step. Every allowlist is read-biased and explicit;
/// none of them is empty, because an empty allowlist means "everything".
///
/// There is deliberately no free-form "add a custom extension" form beside it.
/// Typing an MCP `command`, an `args` list and a set of `env_keys` on a phone
/// keyboard is the worst form in this program, and every character of it has
/// to be right or the extension fails to start with an error from a
/// subprocess.
pub(crate) const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "todoist",
        display_name: "Todoist",
        summary: "Tasks and projects through Doist's own remote MCP server.",
        secrets: &[CatalogSecret {
            key: "TODOIST_API_KEY",
            prompt: "API token from Todoist → Settings → Integrations → Developer",
        }],
        tier: 2,
        scope: "Reads tasks and projects, and can add, update, complete and \
                reschedule them. There is no delete tool to withhold.",
        build: todoist_extension,
    },
    CatalogEntry {
        id: "mail-imap",
        display_name: "Mail (IMAP)",
        summary: "Any IMAP mailbox, using a provider app password.",
        secrets: &[CatalogSecret {
            key: "MCP_EMAIL_SERVER_PASSWORD",
            prompt: "App password from your mail provider (not your login password)",
        }],
        tier: 2,
        scope: "Lists and reads mail, marks messages read and saves drafts. \
                It cannot send and it cannot delete: no such tool is allowed.",
        build: mail_extension,
    },
    CatalogEntry {
        id: "cal-caldav",
        display_name: "Calendar (CalDAV)",
        summary: "Read-only calendar and tasks over CalDAV.",
        secrets: &[
            CatalogSecret {
                key: "CALDAV_BASE_URL",
                prompt: "CalDAV URL, e.g. https://caldav.example.com/dav/",
            },
            CatalogSecret {
                key: "CALDAV_USERNAME",
                prompt: "CalDAV username",
            },
            CatalogSecret {
                key: "CALDAV_PASSWORD",
                prompt: "App password for CalDAV",
            },
        ],
        tier: 2,
        scope: "Read-only: lists calendars, events and todos. No create or \
                update tool is allowed.",
        build: caldav_extension,
    },
];

/// Doist's first-party remote MCP. The token rides in an `Authorization`
/// header as `${TODOIST_API_KEY}`, which goose expands from its secret store
/// when the extension starts — so the value never crosses the ACP frame.
///
/// Note the failure mode this shares with every header credential: an unknown
/// `${VAR}` is left LITERAL rather than erroring, so a missing secret shows up
/// as a 401 from Todoist rather than a startup failure. The handshake after
/// adding is what catches it.
fn todoist_extension() -> GooseExtension {
    GooseExtension::mcp(
        McpServer::Http(HttpMcpServer::new(
            "todoist",
            "https://ai.todoist.net/mcp",
            vec![HttpHeader::new(
                "Authorization",
                "Bearer ${TODOIST_API_KEY}",
            )],
        )),
        vec!["TODOIST_API_KEY".to_string()],
        "Todoist tasks via Doist's first-party remote MCP",
        [
            "get-overview",
            "find-tasks",
            "find-tasks-by-date",
            "find-projects",
            "add-tasks",
            "update-tasks",
            "complete-tasks",
            "reschedule-tasks",
        ]
        .map(str::to_string)
        .to_vec(),
    )
}

/// Read-biased, not read-only: `mark_emails_as_read` and `save_to_mailbox`
/// change flags and drafts. Neither sends nor deletes, and no send or delete
/// tool appears below — the allowlist IS the enforcement, so it matches that
/// sentence exactly.
fn mail_extension() -> GooseExtension {
    GooseExtension::mcp(
        McpServer::Stdio(StdioMcpServer::new(
            "mail-imap",
            "uvx",
            vec!["mcp-email-server@1.4.2".to_string(), "stdio".to_string()],
        )),
        vec!["MCP_EMAIL_SERVER_PASSWORD".to_string()],
        "IMAP/SMTP mail via a provider app password",
        [
            "list_available_accounts",
            "list_mailboxes",
            "list_emails_metadata",
            "get_emails_content",
            "mark_emails_as_read",
            "save_to_mailbox",
        ]
        .map(str::to_string)
        .to_vec(),
    )
}

/// Read-only: no `create-event` or `update-event` here.
fn caldav_extension() -> GooseExtension {
    GooseExtension::mcp(
        McpServer::Stdio(StdioMcpServer::new(
            "cal-caldav",
            "npx",
            vec!["--yes".to_string(), "caldav-mcp@0.10.0".to_string()],
        )),
        vec![
            "CALDAV_BASE_URL".to_string(),
            "CALDAV_USERNAME".to_string(),
            "CALDAV_PASSWORD".to_string(),
        ],
        "CalDAV calendar and tasks via a provider app password",
        ["list-calendars", "list-events", "list-todos"]
            .map(str::to_string)
            .to_vec(),
    )
}

// ---------------------------------------------------------------------------
// What the screens do.

/// Fetch the configured extensions. Safe to call when disconnected: it just
/// leaves the list alone.
pub(crate) async fn refresh(ctx: &AppCtx) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let mut warnings = ctx.extensions.warnings;
    // `load_remote` keeps loading, unsupported and the sticky failure in step
    // with each other; the warnings are set from inside the same fetch rather
    // than by a second pass that could end up disagreeing with it.
    load_remote(ctx, ctx.extensions.list, async move {
        let listed = client.config_extensions_list().await?;
        warnings.set(listed.warnings);
        Ok(listed.extensions)
    })
    .await;
}

/// Keep a failure on screen instead of toasting it away.
///
/// This is why the scaffolding generalised `Remote::sticky`: an allowlist that
/// did not stick is not a message you want to miss because you were looking at
/// the keyboard. It is written *after* the re-list, because `Remote::begin`
/// clears the last failure when a fetch starts — reporting first and then
/// refreshing would wipe the report.
fn report(ctx: &AppCtx, message: String) {
    let mut list = ctx.extensions.list;
    list.write().sticky = Some(message);
}

/// Switch a configured extension on or off, then re-list so what is shown is
/// what the server holds rather than what we hoped it would hold.
///
/// Not an optimistic flip. Enabling spawns a subprocess or dials a remote MCP
/// server, and either can fail after the call has already returned success —
/// so the dot goes busy, the call is awaited, and the server is asked again.
/// It is the discipline `set_config_option` already uses.
pub(crate) fn set_enabled(ctx: &AppCtx, config_key: &str, enabled: bool) {
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(ctx, "Not connected — reconnect in Settings");
        return;
    };
    let (ctx, config_key) = (*ctx, config_key.to_owned());
    let mut toggle = ctx.extensions.toggle;
    toggle.set(Some(Toggle {
        key: config_key.clone(),
        failed: false,
    }));
    spawn_forever(async move {
        let outcome = client
            .config_extension_set_enabled(&config_key, enabled)
            .await;
        refresh(&ctx).await;
        match outcome {
            Ok(()) => toggle.set(None),
            Err(e) => {
                // The list is still readable behind it, so this is a toast —
                // and the row keeps a failed dot, so the toast fading does not
                // take the news with it.
                toggle.set(Some(Toggle {
                    key: config_key.clone(),
                    failed: true,
                }));
                show_toast(&ctx, format!("Could not change {config_key}: {e}"));
            }
        }
    });
}

/// Add a catalogue entry: store its credentials, add the extension, prove the
/// allowlist survived, then prove the credential works.
///
/// `values` is parallel to the entry's `secrets` and is moved into the task —
/// it is never written to `Settings`, which is persisted storage, and it is
/// dropped as soon as the last `upsert` returns.
pub(crate) fn add_from_catalog(ctx: &AppCtx, index: usize, values: Vec<String>) {
    let Some(entry) = CATALOG.get(index) else {
        return;
    };
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(ctx, "Not connected — reconnect in Settings");
        return;
    };

    let extension = (entry.build)();
    let keys: Vec<&'static str> = entry.secrets.iter().map(|s| s.key).collect();
    let display_name = entry.display_name;
    let session_id = ctx.chat.peek().session_id.clone();
    let cwd = handshake_cwd(ctx);

    let ctx = *ctx;
    let mut busy = ctx.extensions.busy;
    busy.set(true);
    let mut list = ctx.extensions.list;
    list.write().sticky = None;

    spawn_forever(async move {
        // 1. Credentials first: an extension that starts without them is a
        //    hard failure for stdio and a silent 401 for a header, and
        //    neither is a useful thing to debug from a phone.
        for (key, value) in keys.iter().zip(values) {
            if let Err(e) = client.store_secret(key, &value).await {
                busy.set(false);
                report(&ctx, format!("Could not store {key}: {e}"));
                return;
            }
        }

        // 2. Add, and refuse to believe it worked until the allowlist comes
        //    back intact.
        match client.add_extension_verified(&extension, true).await {
            Ok(_) => {}
            Err(AcpError::Verification(message)) => {
                // It never went live: `add_extension_verified` stores every
                // extension disabled and only switches it on once the
                // allowlist has been read back intact. So there is nothing to
                // undo here — just say what happened.
                busy.set(false);
                refresh(&ctx).await;
                report(
                    &ctx,
                    format!(
                        "{message}\n\nIt is switched off on the server and was never \
                         started."
                    ),
                );
                return;
            }
            Err(e) => {
                busy.set(false);
                report(&ctx, format!("Could not add {display_name}: {e}"));
                return;
            }
        }

        // 3. The handshake. Starting the MCP server is what proves the
        //    credential, and it needs a session — so when no chat is open one
        //    is created for this and thrown away. Skipping it on a fresh
        //    install is how a mistyped token gets a "connected" toast: a
        //    `${VAR}` that goose cannot resolve is left LITERAL in the header
        //    rather than erroring, and only turns into a 401 later.
        if let Err(e) = client
            .verify_extension_starts(session_id.as_deref(), &cwd, &extension)
            .await
        {
            busy.set(false);
            refresh(&ctx).await;
            report(
                &ctx,
                format!(
                    "{display_name} is configured, but would not start: {e}\n\n\
                     The usual cause is a credential that was mistyped or has \
                     expired. Re-enter it above to overwrite it."
                ),
            );
            return;
        }

        busy.set(false);
        refresh(&ctx).await;
        let mut sheet = ctx.extensions.sheet;
        sheet.set(Sheet::Closed);
        show_toast(&ctx, format!("{display_name} connected"));
    });
}

/// Where a throwaway handshake session should be rooted.
///
/// The configured working directory when it is usable, and `/` otherwise —
/// the session exists for as long as it takes to start one MCP server, and
/// nothing runs in it, so any path the server can open will do. Falling back
/// beats refusing to verify the credential because Settings is half filled in.
fn handshake_cwd(ctx: &AppCtx) -> String {
    let configured = ctx.settings.peek().working_dir.trim().to_string();
    if configured.starts_with('/') {
        configured
    } else {
        "/".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    fn entry(tools: Vec<&str>) -> GooseExtensionEntry {
        GooseExtensionEntry {
            extension: GooseExtension::mcp(
                McpServer::Stdio(StdioMcpServer::new("mail-imap", "uvx", Vec::new())),
                Vec::new(),
                "test",
                tools.into_iter().map(str::to_string).collect(),
            ),
            enabled: true,
            config_key: Some("mail-imap".to_string()),
            extra: Map::<String, Value>::new(),
        }
    }

    /// The row never shows goose's word for the transport. `IN_PROGRESS`-style
    /// copy is what design rule 8 exists to stop, and "stdio" is the same
    /// mistake with a different schema behind it.
    #[test]
    fn a_transport_is_shown_as_copy_never_as_the_enum() {
        assert_eq!(kind_copy("stdio"), "Local command");
        assert_eq!(kind_copy("http"), "Remote");
        // goose's config file spells the same transport this way; both are a
        // server somewhere else, so both get one sentence.
        assert_eq!(kind_copy("streamable_http"), "Remote");
        assert_eq!(kind_copy("builtin"), "Built in");
        assert_eq!(kind_copy("platform"), "Built in");
        // A kind goose has not invented yet still must not leak a schema word.
        assert_eq!(kind_copy("carrier_pigeon"), "Extension");
    }

    /// Busy outranks failed outranks the server's own answer: a toggle in
    /// flight is the most recent true thing about the row.
    #[test]
    fn what_the_user_just_asked_for_outranks_the_last_list() {
        assert_eq!(RowState::of(true, false, false), RowState::On);
        assert_eq!(RowState::of(false, false, false), RowState::Off);
        assert_eq!(RowState::of(false, false, true), RowState::Failed);
        assert_eq!(RowState::of(true, true, true), RowState::Busy);
    }

    /// Every state has a word and a dot class `main.css` actually defines —
    /// a state that fell through to no dot would be an invisible one.
    #[test]
    fn every_state_has_a_word_and_a_dot() {
        for state in [
            RowState::On,
            RowState::Off,
            RowState::Busy,
            RowState::Failed,
        ] {
            assert!(!state.word().is_empty());
            assert!(["dot on", "dot off", "dot busy", "dot err"].contains(&state.dot()));
        }
    }

    #[test]
    fn the_meta_line_is_the_state_then_the_kind() {
        assert_eq!(meta_line(RowState::On, "stdio"), "On · Local command");
        assert_eq!(meta_line(RowState::Off, "http"), "Off · Remote");
        assert_eq!(meta_line(RowState::Failed, "builtin"), "Failed · Built in");
    }

    /// An empty allowlist is the dangerous one, and the row says so in words
    /// rather than leaving a reassuring blank.
    #[test]
    fn no_allowlist_reads_as_everything() {
        assert_eq!(
            tool_summary(&entry(Vec::new())),
            "every tool this server publishes"
        );
    }

    #[test]
    fn a_long_allowlist_is_summarised_rather_than_listed() {
        assert_eq!(tool_summary(&entry(vec!["one"])), "one");
        assert_eq!(tool_summary(&entry(vec!["a", "b", "c"])), "a, b, c");
        assert_eq!(
            tool_summary(&entry(vec!["a", "b", "c", "d"])),
            "a, b, and 2 more"
        );
    }

    /// `display_name` wins where goose gives one; an `mcp` extension has none,
    /// so its config key is made readable instead of shown as a key.
    #[test]
    fn a_title_prefers_gooses_display_name_over_a_config_key() {
        let builtin = GooseExtension::Builtin {
            name: "developer".to_string(),
            description: None,
            display_name: Some("Developer".to_string()),
            timeout: None,
            bundled: None,
            available_tools: None,
            extra: Map::new(),
        };
        assert_eq!(title_for(&builtin), "Developer");
        assert_eq!(title_for(&entry(Vec::new()).extension), "Mail imap");
    }

    /// Two screens under one dump key means the second overwrites the first in
    /// the gallery, and whatever it was showing sits outside every check
    /// `docs/audit.js` performs — which has happened, to a whole branch of it.
    #[test]
    fn each_screen_dumps_under_a_key_of_its_own() {
        assert_ne!(dump_key(Screen::List), dump_key(Screen::Detail));
        // The list's key is the destination id, which is what the shell falls
        // back to when a destination names no screen.
        assert_eq!(dump_key(Screen::List), "extensions");
    }

    /// Every catalogue entry must carry a non-empty allowlist. An empty one is
    /// refused by `add_extension_verified`, so the mistake would surface as a
    /// failed add — but it is a mistake to make impossible at the source.
    #[test]
    fn no_catalogue_entry_can_call_everything() {
        for entry in CATALOG {
            let extension = (entry.build)();
            assert!(
                !extension.available_tools().is_empty(),
                "{} has no tool allowlist, which means every tool",
                entry.id
            );
            assert!(!entry.secrets.is_empty(), "{} asks for nothing", entry.id);
        }
    }
}
