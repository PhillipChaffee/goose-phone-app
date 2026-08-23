use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{probe, ProbeOutcome};

use crate::icons::Icon;
use crate::state::{
    disconnect, establish, refresh_sessions, show_toast, use_app_ctx, ConnState, Screen, Settings,
};

#[component]
pub fn SettingsView() -> Element {
    let ctx = use_app_ctx();
    let mut settings = ctx.settings;
    let saved = (ctx.settings)();

    // Local drafts so typing doesn't hammer persistent storage.
    let mut server_url = use_signal(|| saved.server_url.clone());
    let mut secret_key = use_signal(|| saved.secret_key.clone());
    let mut fingerprint = use_signal(|| saved.fingerprint.clone());
    let mut working_dir = use_signal(|| saved.working_dir.clone());
    let mut code_server_url = use_signal(|| saved.code_server_url.clone());
    let mut code_password = use_signal(|| saved.code_password.clone());
    let testing = use_signal(|| false);

    let conn = (ctx.conn)();
    let connected = conn.is_connected();
    let connecting = matches!(conn, ConnState::Connecting);

    let mut save = move || {
        settings.set(Settings {
            server_url: server_url.peek().trim().to_string(),
            secret_key: secret_key.peek().trim().to_string(),
            fingerprint: fingerprint.peek().trim().to_string(),
            working_dir: working_dir.peek().trim().to_string(),
            code_server_url: code_server_url.peek().trim().to_string(),
            code_password: code_password.peek().trim().to_string(),
        });
    };

    let on_connect = move |_| {
        save();
        // Root-scope task: it navigates away from this screen and must
        // survive the unmount.
        spawn_forever(async move {
            if establish(&ctx).await {
                let mut screen = ctx.screen;
                screen.set(Screen::Sessions);
                refresh_sessions(&ctx, false).await;
            }
        });
    };

    let on_test = move |_| {
        save();
        let s = ctx.settings.peek().clone();
        spawn_forever(async move {
            let mut testing = testing;
            testing.set(true);
            let pinned = goose_acp_client::parse_fingerprint(&s.fingerprint)
                .ok()
                .flatten()
                .is_some();
            let outcome = probe(&s.server_url, &s.secret_key, pinned).await;
            testing.set(false);
            match outcome {
                ProbeOutcome::Ok => show_toast(&ctx, "Server reachable, secret accepted ✓"),
                ProbeOutcome::AuthFailed => {
                    show_toast(&ctx, "Server reachable, but the secret key was rejected");
                }
                ProbeOutcome::Unreachable(e) => show_toast(&ctx, format!("Unreachable: {e}")),
            }
        });
    };

    rsx! {
        header { class: "topbar",
            // Always present. Settings is a drawer destination, not a pushed
            // screen, and the back chevron it replaces only rendered when
            // connected — which left a disconnected user with no way off this
            // screen once the tab bar went away.
            button {
                class: "icon-btn menu",
                onclick: move |_| {
                    let mut open = ctx.drawer_open;
                    open.set(true);
                },
                Icon { name: "menu" }
            }
            h1 { class: "title", "Settings" }

        }
        main { class: "scroll settings",
            section { class: "card",
                h2 { "Server" }
                label { class: "field-label", "Server URL" }
                input {
                    class: "field",
                    r#type: "url",
                    placeholder: "https://goose-box.tailnet-name.ts.net",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{server_url}",
                    oninput: move |e| server_url.set(e.value()),
                }
                p { class: "hint",
                    "Over Tailscale: use your server's MagicDNS name. With "
                    code { "tailscale serve" }
                    " fronting goose this is an https:// URL; a plain "
                    code { "goose serve" }
                    " on the tailnet is http://host:3284."
                }

                label { class: "field-label", "Secret key" }
                input {
                    class: "field",
                    r#type: "password",
                    placeholder: "GOOSE_SERVER__SECRET_KEY value",
                    autocapitalize: "off",
                    autocomplete: "off",
                    value: "{secret_key}",
                    oninput: move |e| secret_key.set(e.value()),
                }

                label { class: "field-label", "TLS certificate fingerprint (optional)" }
                input {
                    class: "field",
                    r#type: "text",
                    placeholder: "AA:BB:CC:…",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{fingerprint}",
                    oninput: move |e| fingerprint.set(e.value()),
                }
                p { class: "hint",
                    "Paste the GOOSED_CERT_FINGERPRINT line goose prints at startup to pin a "
                    "self-signed certificate. Leave empty for real certificates (tailscale serve)."
                }
            }

            section { class: "card",
                h2 { "Agent" }
                label { class: "field-label", "Working directory on the server" }
                input {
                    class: "field",
                    r#type: "text",
                    placeholder: "/home/you/projects",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{working_dir}",
                    oninput: move |e| working_dir.set(e.value()),
                }
                p { class: "hint",
                    "Absolute path on the goose server; new chats start here."
                }
            }

            section { class: "card",
                h2 { "Code agents" }
                label { class: "field-label", "Code server URL" }
                input {
                    class: "field",
                    r#type: "url",
                    placeholder: "https://brain.tailnet-name.ts.net:4300",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{code_server_url}",
                    oninput: move |e| code_server_url.set(e.value()),
                }
                label { class: "field-label", "Code server password" }
                input {
                    class: "field",
                    r#type: "password",
                    placeholder: "OPENCODE_SERVER_PASSWORD value",
                    autocapitalize: "off",
                    autocomplete: "off",
                    value: "{code_password}",
                    oninput: move |e| code_password.set(e.value()),
                }
                p { class: "hint",
                    "The code-agent gateway on the brain (docs/setup/70-code-agents.md). "
                    "Save, then open the Code tab — it connects with these."
                }
            }

            if let ConnState::Failed(error) = &conn {
                p { class: "error-box", "{error}" }
            }

            div { class: "btn-row",
                button {
                    class: "btn secondary",
                    disabled: testing() || connecting,
                    onclick: on_test,
                    if testing() { "Testing…" } else { "Test connection" }
                }
                button {
                    class: "btn primary",
                    disabled: connecting,
                    onclick: on_connect,
                    if connecting { "Connecting…" }
                    else if connected { "Reconnect" }
                    else { "Save & Connect" }
                }
            }
            if connected {
                div { class: "btn-row",
                    button {
                        class: "btn danger-outline",
                        onclick: move |_| {
                            disconnect(&ctx);
                        },
                        "Disconnect"
                    }
                }
            }

            section { class: "about",
                // The bar shows connection state as a bare dot, so the thing
                // it is connected to gets named here instead.
                if let ConnState::Connected { agent } = &conn {
                    p { class: "about-conn",
                        span { class: "dot on" }
                        " Connected to {agent}"
                    }
                }
                p { "Connects to a remote goose AI agent over its ACP WebSocket API." }
                p { "Reach a private server from anywhere with the Tailscale app enabled on this phone." }
            }
        }
    }
}
