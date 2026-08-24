//! Connect tab: state + logic for the services the agent can reach.
//!
//! An "extension" is goose's word for an MCP server it has been told about.
//! This tab lists the ones configured on the server, lets you switch one on or
//! off, and adds a new one from a small built-in catalogue.
//!
//! Two things shape every decision in here, and both of them are about
//! failing closed.
//!
//! **The tool allowlist is one word away from being a no-op.** `available_tools`
//! is `snake_case` on the ACP wire while its neighbours are camelCase; goose sets
//! no `deny_unknown_fields`, so a camelCase spelling is dropped in silence, and
//! a dropped allowlist means *every* tool the MCP server publishes is callable.
//! So nothing here trusts an `add` that returned OK: `add_extension_verified`
//! re-lists and compares, and when the comparison fails the extension is
//! switched off on the server before the error is shown.
//!
//! **A credential goes one way only.** Secrets are written with
//! `config/upsert` + `isSecret`, land in the server's `secrets.yaml`, and are
//! never read back — `config/read` on a secret returns a clear prefix, so
//! "check what we stored" is a leak. Verification is a handshake instead:
//! bring the extension up in the open session and let goose fail if the
//! credential is missing. Nothing typed into a credential field is persisted
//! on the phone, and there is deliberately no reveal control.
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

use crate::state::{show_toast, AppCtx};

/// Connect's own back stack, so the drawer can leave and come back to it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectScreen {
    List,
    Add,
}

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

/// Fetch the configured extensions. Safe to call when disconnected: it just
/// leaves the list alone.
pub(crate) async fn refresh_extensions(ctx: &AppCtx) {
    let Some(client) = ctx.client.peek().clone() else {
        return;
    };
    let mut loading = ctx.extensions_loading;
    loading.set(true);
    match client.config_extensions_list().await {
        Ok(listed) => {
            ctx.extensions.clone().set(listed.extensions);
            ctx.extension_warnings.clone().set(listed.warnings);
        }
        Err(e) => show_toast(ctx, format!("Could not list services: {e}")),
    }
    loading.set(false);
}

/// Switch a configured extension on or off, then re-list so what is shown is
/// what the server holds rather than what we hoped it would hold.
pub(crate) fn set_enabled(ctx: &AppCtx, config_key: &str, enabled: bool) {
    let Some(client) = ctx.client.peek().clone() else {
        show_toast(ctx, "Not connected — reconnect in Settings");
        return;
    };
    let (ctx, config_key) = (*ctx, config_key.to_owned());
    spawn_forever(async move {
        if let Err(e) = client
            .config_extension_set_enabled(&config_key, enabled)
            .await
        {
            show_toast(&ctx, format!("Could not change {config_key}: {e}"));
        }
        refresh_extensions(&ctx).await;
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

    let ctx = *ctx;
    let mut busy = ctx.connect_busy;
    let mut error = ctx.connect_error;
    busy.set(true);
    error.set(None);

    spawn_forever(async move {
        // 1. Credentials first: an extension that starts without them is a
        //    hard failure for stdio and a silent 401 for a header, and
        //    neither is a useful thing to debug from a phone.
        for (key, value) in keys.iter().zip(values) {
            if let Err(e) = client.store_secret(key, &value).await {
                error.set(Some(format!("Could not store {key}: {e}")));
                busy.set(false);
                return;
            }
        }

        // 2. Add, and refuse to believe it worked until the allowlist comes
        //    back intact.
        match client.add_extension_verified(&extension, true).await {
            Ok(_) => {}
            Err(AcpError::Allowlist(message)) => {
                // The extension is on the server and unrestricted. Turn it off
                // before saying anything, so the window where it is live is as
                // short as the round trip.
                let quarantined = quarantine(&client, extension.name()).await;
                let tail = if quarantined {
                    "It has been switched off on the server."
                } else {
                    "It could NOT be switched off — disable it from the list below."
                };
                error.set(Some(format!("{message}\n\n{tail}")));
                busy.set(false);
                refresh_extensions(&ctx).await;
                return;
            }
            Err(e) => {
                error.set(Some(format!("Could not add {display_name}: {e}")));
                busy.set(false);
                return;
            }
        }

        // 3. The handshake. Adding it to the open session is what actually
        //    launches the MCP server, so this is where a wrong or missing
        //    credential surfaces — and it does so without reading anything
        //    back. With no session open there is nothing to hand shake with;
        //    the credential is still stored, and the next chat will use it.
        if let Some(session_id) = session_id {
            if let Err(e) = client.session_extension_add(&session_id, &extension).await {
                error.set(Some(format!(
                    "{display_name} is configured, but would not start: {e}\n\n\
                     The usual cause is a credential that was mistyped or has \
                     expired. Re-enter it above to overwrite it."
                )));
                busy.set(false);
                refresh_extensions(&ctx).await;
                return;
            }
        }

        busy.set(false);
        refresh_extensions(&ctx).await;
        ctx.connect_screen.clone().set(ConnectScreen::List);
        show_toast(&ctx, format!("{display_name} connected"));
    });
}

/// Switch off an extension we have just added and do not trust. Returns
/// whether it is definitely off.
async fn quarantine(client: &goose_acp_client::AcpClient, name: &str) -> bool {
    let Ok(listed) = client.config_extensions_list().await else {
        return false;
    };
    let Some(key) = listed
        .extensions
        .iter()
        .find(|e| e.extension.name() == name)
        .and_then(|e| e.config_key.clone())
    else {
        return false;
    };
    client
        .config_extension_set_enabled(&key, false)
        .await
        .is_ok()
}
