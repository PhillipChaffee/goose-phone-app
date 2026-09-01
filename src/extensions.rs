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
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test scaffolding: a fixture that will not serialize, or a call \
              the assertion is about that was never made, is the failing check"
)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    use crate::serverkit::{ok, rpc_error, short, Harness, Reply};

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

    // ------------------------------------------------------------ the server
    //
    // Everything past the catalogue needs a client. `set_enabled` and
    // `add_from_catalog` are one `let Some(client) = ... else` away from
    // returning, and — more to the point — the three rules this module exists
    // to hold are properties of WHAT WENT OUT, not of any signal left behind:
    // the credential is stored before the extension is added, the extension is
    // added switched OFF, and it is only switched on after the allowlist has
    // come back intact. None of those is visible in `Ctx` afterwards. So these
    // run against `crate::serverkit`'s loopback JSON-RPC server and assert on
    // its request log.

    /// Todoist is the entry every add test below uses. Named as well as
    /// indexed, because a reordering of `CATALOG` would otherwise move these
    /// tests silently onto a different service with different secrets.
    const TODOIST: usize = 0;

    /// The token an add test types in. Never a real-looking one, and checked
    /// for by string: it must reach exactly one frame.
    const TOKEN: &str = "tok-secret-42";

    /// The catalogue entry at `index` listed back exactly as it was sent —
    /// what a goose that stored it correctly answers with.
    fn faithful_entry(index: usize) -> Value {
        json!({
            "extension": serde_json::to_value((CATALOG[index].build)()).unwrap(),
            "enabled": false,
            "configKey": CATALOG[index].id,
        })
    }

    /// The same entry with its allowlist tampered with. `Some(list)` rewrites
    /// it; `None` removes the key entirely, which is what goose sends when the
    /// stored allowlist is empty — the silent camelCase failure this whole
    /// module is built around.
    fn mangled_entry(index: usize, tools: Option<&[&str]>) -> Value {
        let mut entry = faithful_entry(index);
        let extension = entry["extension"]
            .as_object_mut()
            .expect("an extension serializes as an object");
        match tools {
            Some(tools) => {
                extension.insert("available_tools".to_owned(), json!(tools));
            }
            None => {
                extension.remove("available_tools");
            }
        }
        entry
    }

    /// A goose that stores the secret, accepts the add, lists it back intact
    /// and starts the MCP server.
    fn happy(method: &str, _params: &Value) -> Reply {
        match short(method) {
            "config/extensions/list" => ok(json!({
                "extensions": [faithful_entry(TODOIST)],
                "warnings": [],
            })),
            "session/new" => ok(json!({ "sessionId": "throwaway-1", "configOptions": [] })),
            _ => ok(json!({})),
        }
    }

    /// A goose whose secret store will not take a write. It refuses
    /// everything, which is the point: the assertion below is that nothing
    /// after the `config/upsert` was even attempted.
    fn refuses_every_write(_method: &str, _params: &Value) -> Reply {
        rpc_error(-32603, "secrets.yaml is not writable")
    }

    /// A goose that stores the credential and then rejects the extension
    /// itself — an `sse` server, a field combination it will not take.
    fn refuses_the_extension(method: &str, params: &Value) -> Reply {
        if short(method) == "config/extensions/add" {
            return rpc_error(-32602, "unsupported extension shape");
        }
        happy(method, params)
    }

    fn drops_a_tool(method: &str, params: &Value) -> Reply {
        if short(method) == "config/extensions/list" {
            return ok(json!({
                "extensions": [mangled_entry(TODOIST, Some(&["find-tasks"]))],
                "warnings": [],
            }));
        }
        happy(method, params)
    }

    fn drops_the_allowlist(method: &str, params: &Value) -> Reply {
        if short(method) == "config/extensions/list" {
            return ok(json!({
                "extensions": [mangled_entry(TODOIST, None)],
                "warnings": [],
            }));
        }
        happy(method, params)
    }

    fn will_not_start(method: &str, params: &Value) -> Reply {
        if short(method) == "session/extensions/add" {
            return rpc_error(-32603, "401 Unauthorized from ai.todoist.net");
        }
        happy(method, params)
    }

    fn set_enabled_fails(method: &str, params: &Value) -> Reply {
        if short(method) == "config/extensions/set-enabled" {
            return rpc_error(-32602, "no extension has that config key");
        }
        happy(method, params)
    }

    /// A goose whose config plane is broken.
    fn list_breaks(_method: &str, _params: &Value) -> Reply {
        rpc_error(-32603, "config.yaml is unreadable")
    }

    /// A goose with no extensions plane at all. `-32601` is its own signal for
    /// "this feature is absent", not merely "this server is old".
    fn no_extensions_at_all(_method: &str, _params: &Value) -> Reply {
        rpc_error(-32601, "Method not found")
    }

    /// The screen's whole state after an add, in one readable tuple: the
    /// sticky failure, whether the add is still in flight, and whether the
    /// sheet closed.
    fn outcome(h: &Harness) -> (Option<String>, bool, Sheet) {
        h.with(|ctx| {
            (
                ctx.extensions.list.peek().sticky.clone(),
                *ctx.extensions.busy.peek(),
                *ctx.extensions.sheet.peek(),
            )
        })
    }

    /// Open the catalogue on Todoist, the way tapping it does, so a test can
    /// say whether a failure left the sheet where the user could retry.
    fn open_the_sheet(h: &Harness) {
        h.with(|ctx| ctx.extensions.sheet.clone().set(Sheet::Picked(TODOIST)));
    }

    #[test]
    fn the_add_tests_are_pointed_at_todoist() {
        assert_eq!(
            CATALOG[TODOIST].id, "todoist",
            "CATALOG was reordered, so every add test below is now typing a \
             Todoist token into a different service"
        );
    }

    // ---- the list ----

    /// The warnings banner is the only place a phone user can ever learn that
    /// an extension exists but failed to parse: goose simply leaves it out of
    /// `extensions`, so without this the screen would show a shorter list and
    /// say nothing at all.
    #[test]
    fn a_config_file_problem_reaches_the_screen_beside_the_list_it_shortened() {
        fn warns(_method: &str, _params: &Value) -> Reply {
            ok(json!({
                "extensions": [faithful_entry(TODOIST)],
                "warnings": ["mail-imap: unknown field `timeout_ms`"],
            }))
        }

        let (mut h, _server) = Harness::connected(warns);
        h.drive(|ctx| async move { refresh(&ctx).await });

        h.with(|ctx| {
            let list = ctx.extensions.list.peek();
            assert_eq!(
                list.items
                    .iter()
                    .map(|e| e.extension.name())
                    .collect::<Vec<_>>(),
                ["todoist"],
                "the listed extension never reached the screen"
            );
            assert!(!list.loading, "the list is still spinning after it landed");
            assert_eq!(
                *ctx.extensions.warnings.peek(),
                ["mail-imap: unknown field `timeout_ms`"],
                "goose reported a config file it could not parse and the screen \
                 dropped it, so an extension is missing with nothing to say why"
            );
        });
    }

    /// A refresh with no client must leave the list exactly as it was — and
    /// above all must not start it loading, because nothing would ever stop
    /// the spinner.
    #[test]
    fn a_refresh_without_a_connection_leaves_the_list_alone() {
        let mut h = Harness::offline();
        h.drive(|ctx| async move { refresh(&ctx).await });
        h.with(|ctx| {
            let list = ctx.extensions.list.peek();
            assert!(!list.loading, "a disconnected refresh armed a spinner");
            assert!(list.items.is_empty() && list.sticky.is_none());
        });
        assert_eq!(h.toast(), None, "being offline is not news worth a toast");
    }

    /// A server that cannot read its config is a failure to keep ON SCREEN:
    /// there is nothing behind it to look at, so a toast would take the only
    /// explanation away with it after four seconds.
    #[test]
    fn a_list_that_fails_with_nothing_behind_it_stays_on_screen() {
        let (mut h, _server) = Harness::connected(list_breaks);
        h.drive(|ctx| async move { refresh(&ctx).await });
        h.with(|ctx| {
            let list = ctx.extensions.list.peek();
            let sticky = list.sticky.clone().expect("the failure was thrown away");
            assert!(
                sticky.contains("config.yaml is unreadable"),
                "the sticky failure does not say what went wrong: {sticky}"
            );
            assert!(
                !list.unsupported,
                "a broken config was reported as a server without the feature, \
                 which hides the Retry"
            );
        });
    }

    /// `-32601` is goose's own "this feature is absent", and it must read as
    /// unsupported rather than as an error with a Retry that cannot work.
    #[test]
    fn a_server_without_the_extensions_plane_says_so_rather_than_failing() {
        let (mut h, _server) = Harness::connected(no_extensions_at_all);
        h.drive(|ctx| async move { refresh(&ctx).await });
        h.with(|ctx| {
            let list = ctx.extensions.list.peek();
            assert!(
                list.unsupported,
                "the screen offers a Retry that cannot work"
            );
            assert_eq!(
                list.sticky, None,
                "an absent feature is not an error to keep on screen"
            );
        });
    }

    // ---- the toggle ----

    /// Not an optimistic flip. Switching an extension on spawns a subprocess
    /// or dials a remote MCP server and can fail after the call returns, so
    /// the server is asked again — and the re-list must come AFTER the toggle,
    /// or the screen settles on the state it had before the change.
    #[test]
    fn a_toggle_is_followed_by_a_re_list_rather_than_believed() {
        let (mut h, server) = Harness::connected(happy);
        h.act(|ctx| set_enabled(ctx, "todoist", false));

        assert_eq!(
            server.methods(),
            ["config/extensions/set-enabled", "config/extensions/list"],
            "the screen either skipped the re-list or ran it before the change"
        );
        assert_eq!(
            server.params("config/extensions/set-enabled", 0),
            json!({ "configKey": "todoist", "enabled": false })
        );
        assert!(
            h.with(|ctx| ctx.extensions.toggle.peek().is_none()),
            "the row is still showing \"Switching…\" after the toggle landed"
        );
    }

    /// A failed toggle keeps a failed dot on the row as well as toasting,
    /// because the toast fades and the news must not fade with it.
    #[test]
    fn a_failed_toggle_marks_the_row_and_not_only_the_toast() {
        let (mut h, server) = Harness::connected(set_enabled_fails);
        h.act(|ctx| set_enabled(ctx, "todoist", true));

        let toggle = h
            .with(|ctx| ctx.extensions.toggle.peek().clone())
            .expect("the failed toggle was forgotten, so the row shows nothing");
        assert_eq!(toggle.key, "todoist");
        assert!(
            toggle.failed,
            "the row went back to a plain dot, so only the toast said anything"
        );
        let toast = h.toast().expect("a failed toggle said nothing at all");
        assert!(
            toast.contains("Could not change todoist"),
            "the toast does not name the extension: {toast}"
        );
        assert_eq!(
            server.count("config/extensions/list"),
            1,
            "a failed toggle still has to re-list: what the row shows now is \
             whatever the server still holds"
        );
    }

    // ---- adding from the catalogue ----

    /// The whole add, in the order the rules require: the credential is stored
    /// first, the extension is added SWITCHED OFF, the allowlist is read back,
    /// only then is it switched on, and only then is it brought up in a
    /// session to prove the credential works.
    ///
    /// Reordering any two of those is a real security regression and none of
    /// them is visible in `Ctx` afterwards, which is why this asserts on the
    /// request log rather than on the screen.
    #[test]
    fn an_add_stores_the_secret_first_and_never_goes_live_unverified() {
        let (mut h, server) = Harness::connected(happy);
        h.set_working_dir("/srv/goose");
        open_the_sheet(&h);
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        assert_eq!(
            server.methods(),
            [
                "config/upsert",
                "config/extensions/add",
                "config/extensions/list",
                "config/extensions/set-enabled",
                "session/new",
                "session/extensions/add",
                "session/delete",
                "config/extensions/list",
            ],
            "the add did not happen in the order the failing-closed rules require"
        );
        assert_eq!(
            server.params("config/extensions/add", 0)["enabled"],
            json!(false),
            "the extension was added ENABLED, so it was live and unrestricted \
             for the length of the read-back"
        );
        assert_eq!(
            server.params("config/extensions/set-enabled", 0),
            json!({ "configKey": "todoist", "enabled": true })
        );
        assert_eq!(
            server.params("session/new", 0)["cwd"],
            json!("/srv/goose"),
            "the throwaway handshake session was rooted somewhere else"
        );

        let (sticky, busy, sheet) = outcome(&h);
        assert_eq!(sticky, None, "a clean add still reported a failure");
        assert!(
            !busy,
            "the sheet is still showing a spinner after a clean add"
        );
        assert!(
            matches!(sheet, Sheet::Closed),
            "the add finished and the catalogue sheet stayed over the list"
        );
        let toast = h.toast().expect("a finished add said nothing");
        assert!(
            toast.contains("Todoist connected"),
            "the toast does not name the service: {toast}"
        );
    }

    /// A credential goes one way only. The value the user typed rides in
    /// exactly one frame — `config/upsert` with `isSecret` — and is never
    /// echoed back, never sent again, and never written into `Settings`, which
    /// is persisted to disk.
    #[test]
    fn the_typed_credential_reaches_one_frame_and_no_persisted_state() {
        let (mut h, server) = Harness::connected(happy);
        h.set_working_dir("/srv/goose");
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        assert_eq!(
            server.params("config/upsert", 0),
            json!({ "key": "TODOIST_API_KEY", "value": TOKEN, "isSecret": true }),
            "a secret written without `isSecret` lands in plaintext config.yaml"
        );
        let frames = server.frames().to_string();
        assert_eq!(
            frames.matches(TOKEN).count(),
            1,
            "the token crossed the wire more than once — everything after the \
             upsert must travel as the NAME of a stored secret: {frames}"
        );
        assert!(
            !frames.contains("Bearer tok-secret-42"),
            "the token was substituted into the header instead of being left as \
             ${{TODOIST_API_KEY}} for goose to expand: {frames}"
        );

        let settings = h.with(|ctx| serde_json::to_string(&*ctx.settings.peek()).unwrap());
        assert!(
            !settings.contains(TOKEN),
            "the credential was written into persisted settings: {settings}"
        );
    }

    /// The failure this module is built around: goose accepts a camelCase
    /// `availableTools`, drops it in silence, and reads the absent allowlist as
    /// "every tool this server publishes". It must never go live, and the
    /// screen has to say so where the user is looking rather than in a toast
    /// they are about to miss.
    #[test]
    fn an_allowlist_that_did_not_stick_never_switches_the_extension_on() {
        let cases: [(crate::serverkit::Script, &str); 2] = [
            (drops_the_allowlist, "NO tool allowlist"),
            (drops_a_tool, "different tool allowlist"),
        ];
        for (script, expected) in cases {
            let (mut h, server) = Harness::connected(script);
            h.set_working_dir("/srv/goose");
            open_the_sheet(&h);
            h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

            assert_eq!(
                server.count("config/extensions/set-enabled"),
                0,
                "an extension whose allowlist did not survive was switched ON"
            );
            assert_eq!(
                server.count("session/extensions/add"),
                0,
                "an unverified extension was started in a session anyway"
            );

            let (sticky, busy, sheet) = outcome(&h);
            let sticky = sticky.expect("the allowlist failure was never reported");
            assert!(
                sticky.contains(expected),
                "the report does not say what came back: {sticky}"
            );
            assert!(
                sticky.contains("switched off on the server and was never started"),
                "the report leaves the user thinking the extension may be live: \
                 {sticky}"
            );
            assert!(
                !busy,
                "the sheet is stuck showing a spinner after a failure"
            );
            assert!(
                matches!(sheet, Sheet::Picked(TODOIST)),
                "the sheet closed over a failure, so the report is on a screen \
                 the user was just navigated away from"
            );
        }
    }

    /// A `Verification` failure survives the re-list that follows it. `Remote`
    /// clears the sticky failure when a fetch BEGINS, so reporting before the
    /// refresh would wipe the report — which is the ordering bug this asserts
    /// against, and the reason `report` is called last.
    #[test]
    fn the_report_outlives_the_refresh_that_follows_it() {
        let (mut h, server) = Harness::connected(drops_a_tool);
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));
        assert!(
            server.count("config/extensions/list") >= 2,
            "the failure path skipped the re-list, so the screen still shows \
             the list from before the add"
        );
        assert!(
            outcome(&h).0.is_some(),
            "the re-list cleared the failure it was supposed to follow"
        );
    }

    /// An add goose rejects outright is not a verification failure, and must
    /// not borrow that sentence: nothing is on the server, so "it is switched
    /// off there" would be a claim about something that does not exist.
    #[test]
    fn an_extension_goose_refuses_is_reported_as_itself() {
        let (mut h, server) = Harness::connected(refuses_the_extension);
        open_the_sheet(&h);
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        assert_eq!(
            server.methods(),
            ["config/upsert", "config/extensions/add"],
            "the add was rejected and the screen carried on anyway"
        );
        let (sticky, busy, sheet) = outcome(&h);
        let sticky = sticky.expect("goose refused the extension and nothing said so");
        assert!(
            sticky.contains("Could not add Todoist"),
            "the report does not name the service in the words the catalogue              uses: {sticky}"
        );
        assert!(
            !sticky.contains("never started"),
            "a rejected add borrowed the verification failure's sentence, which              claims something is on the server: {sticky}"
        );
        assert!(!busy, "the sheet is stuck showing a spinner");
        assert!(
            matches!(sheet, Sheet::Picked(TODOIST)),
            "the sheet closed over the failure it was supposed to show"
        );
    }

    /// A credential that could not be stored means the extension must not be
    /// added at all: a stdio server whose `envKeys` name a missing secret dies
    /// at startup, and a header one just 401s later.
    #[test]
    fn a_credential_that_would_not_store_stops_the_add_before_it_starts() {
        let (mut h, server) = Harness::connected(refuses_every_write);
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        assert_eq!(
            server.methods(),
            ["config/upsert"],
            "the extension was configured anyway, with no credential behind it"
        );
        let (sticky, busy, _) = outcome(&h);
        let sticky = sticky.expect("the store failure was never reported");
        assert!(
            sticky.contains("Could not store TODOIST_API_KEY"),
            "the report does not name the credential that failed: {sticky}"
        );
        assert!(!busy, "the sheet is stuck showing a spinner");
    }

    /// The handshake is the only honest credential check there is, and its
    /// failure has to read as "configured but not working" plus what to do
    /// about it — not as a bare RPC error, and not as success.
    #[test]
    fn an_extension_that_will_not_start_is_reported_as_a_credential_problem() {
        let (mut h, server) = Harness::connected(will_not_start);
        h.set_working_dir("/srv/goose");
        open_the_sheet(&h);
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        let (sticky, busy, sheet) = outcome(&h);
        let sticky = sticky.expect("a mistyped credential reported nothing");
        assert!(
            sticky.contains("would not start"),
            "the report does not say the handshake failed: {sticky}"
        );
        assert!(
            sticky.contains("Re-enter it above"),
            "the report says what broke but not what to do: {sticky}"
        );
        assert!(!busy);
        assert!(
            matches!(sheet, Sheet::Picked(TODOIST)),
            "the sheet closed, so there is no field left to re-enter the \
             credential into"
        );
        assert_eq!(
            server.count("session/delete"),
            1,
            "the throwaway handshake session was left behind on the server"
        );
        assert_eq!(h.toast(), None, "a failed add still toasted \"connected\"");
    }

    /// With a chat already open the handshake borrows it. Creating a second
    /// session would be a session the user did not ask for, and deleting it
    /// afterwards is a delete this test proves is never aimed at theirs.
    #[test]
    fn an_open_chat_is_borrowed_for_the_handshake_rather_than_a_new_session() {
        let (mut h, server) = Harness::connected(happy);
        h.set_working_dir("/srv/goose");
        h.with(|ctx| {
            ctx.chat.clone().set(crate::state::ChatState {
                session_id: Some("s-live".to_owned()),
                ..crate::state::ChatState::default()
            });
        });
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        assert_eq!(
            server.count("session/new"),
            0,
            "a throwaway session was created while a chat was already open"
        );
        assert_eq!(
            server.count("session/delete"),
            0,
            "the handshake deleted the user's own open session"
        );
        assert_eq!(
            server.params("session/extensions/add", 0)["sessionId"],
            json!("s-live")
        );
    }

    /// A half-filled-in Settings must not stop the credential being checked:
    /// the throwaway session exists for as long as it takes one MCP server to
    /// start, and nothing runs in it.
    #[test]
    fn a_working_directory_that_is_not_a_path_still_gets_a_handshake() {
        let (mut h, server) = Harness::connected(happy);
        h.set_working_dir("work/pilot");
        h.act(|ctx| add_from_catalog(ctx, TODOIST, vec![TOKEN.to_owned()]));

        assert_eq!(
            server.params("session/new", 0)["cwd"],
            json!("/"),
            "a relative working directory was sent as the session's cwd, which \
             goose refuses — so the credential would never be checked"
        );
        assert_eq!(
            server.count("session/extensions/add"),
            1,
            "the handshake was skipped because Settings was half filled in"
        );
    }

    /// The fallback rule on its own, including the trim: a directory with
    /// spaces around it is still the directory it names.
    #[test]
    fn the_handshake_root_is_the_configured_directory_or_slash() {
        let h = Harness::offline();
        for (configured, expected) in [
            ("/srv/goose", "/srv/goose"),
            ("  /srv/goose  ", "/srv/goose"),
            ("", "/"),
            ("   ", "/"),
            ("work/pilot", "/"),
            ("~/work", "/"),
        ] {
            h.set_working_dir(configured);
            assert_eq!(
                h.with(handshake_cwd),
                expected,
                "a working directory of {configured:?} was rooted wrongly"
            );
        }
    }

    /// An index the catalogue does not have does nothing at all — no half-add,
    /// no spinner, no request.
    #[test]
    fn an_index_off_the_end_of_the_catalogue_sends_nothing() {
        let (mut h, server) = Harness::connected(happy);
        h.act(|ctx| add_from_catalog(ctx, CATALOG.len(), vec![TOKEN.to_owned()]));
        let attempted = server.methods();
        assert!(
            attempted.is_empty(),
            "an out-of-range catalogue index still talked to the server:              {attempted:?}"
        );
        assert!(
            !outcome(&h).1,
            "an add that never started left the sheet showing a spinner"
        );
    }
}
