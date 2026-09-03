use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{probe, ProbeOutcome};

use crate::nav::Crumb;
use crate::shell::Shell;
use crate::state::{
    disconnect, establish, refresh_sessions, show_toast, use_app_ctx, ConnState, Screen, Settings,
};
use crate::views::chrome::TopBar;
use crate::views::Confirm;

/// What this screen is called, once.
///
/// Read by two things that are never on screen together: this view's own
/// header, and — on the desktop — the window's bar, which takes the heading
/// out of the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop/mod.rs`). One expression, so the window and the pane cannot
/// end up calling the same screen two different things.
pub(crate) fn crumb() -> Crumb {
    Crumb::plain("Settings")
}

/// The machine the Tailscale client has to be running on, named as the reader
/// would name it.
///
/// This screen is shared by both shells and the sentence it sits in is about
/// hardware, so a single spelling is wrong on one of them: the desktop opens a
/// 1440x860 window (`src/main.rs`) and told the reader it was a phone. Not a
/// cosmetic point — this is the one screen a reader arrives at *because* the
/// connection is not working, and being told to check an app on a device they
/// are not holding is advice that cannot be followed.
///
/// TAKES THE SHELL, and does not read `Shell::CURRENT` itself, which is the
/// rule `views::chrome`'s own test module states and the reason for it:
/// `cargo test` runs on a host, where `CURRENT` is always `Shell::Desktop`, so
/// a phone assertion against an ambient read would be an assertion about the
/// desktop arm passing under a phone's name. `views::chat::attributed` picks
/// the desktop *structure* the other way and can only be checked against the
/// captured markup; a string has somewhere better to be checked.
///
/// The call site passes the `const`, so this is still selected at compile time
/// with no `cfg` and no branch in the binary, and `src/views/` keeps its zero
/// `cfg(target_os)`.
///
/// "computer" and not "Mac": `Shell::Desktop` is every target that is not iOS
/// or Android (`src/shell/mod.rs`), so naming the hardware would just be a
/// different wrong answer on Linux and Windows.
const fn tailscale_host(shell: Shell) -> &'static str {
    match shell {
        Shell::Mobile => "this phone",
        Shell::Desktop => "this computer",
    }
}

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

    let mut connect_now = move || {
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

    // Reconnecting closes the live client first (`state::establish`), which
    // sends a real Close frame — and a Close frame under a running turn
    // destroys that turn's round on the server, prompt and title surviving
    // and nothing else (docs/permission-durability.md section 0). The app
    // cannot undo that afterwards, so it asks first. Only when there is
    // something to lose: with nothing running this button is unchanged.
    let mut confirm_reconnect = use_signal(|| false);
    let running = !ctx.running_sessions.read().is_empty();
    let on_connect = move |_| {
        if running {
            confirm_reconnect.set(true);
        } else {
            connect_now();
        }
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
        // No `on_back`: Settings is a drawer destination, not a pushed
        // screen, so TopBar gives it the hamburger. The back chevron it
        // replaces rendered only when connected, which left a disconnected
        // user with no way off this screen once the tab bar went away — the
        // component is where that stops being a per-screen decision.
        TopBar { title: crumb().title }
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
                p { "Reach a private server from anywhere with the Tailscale app enabled on {tailscale_host(Shell::CURRENT)}." }
            }
        }

        if confirm_reconnect() {
            Confirm {
                title: "A turn is still running",
                body: "Reconnecting closes this connection, and goose throws away \
                       whatever the agent was working on. Your message stays in the \
                       chat; the reply does not.",
                confirm_label: "Reconnect anyway",
                danger: true,
                on_cancel: move |()| confirm_reconnect.set(false),
                on_confirm: move |()| {
                    confirm_reconnect.set(false);
                    connect_now();
                },
            }
        }
    }
}

/// The form, filled in and pressed.
///
/// This screen is six fields and four buttons, and until this existed it was
/// measured at 13.70%: `crate::testkit` renders it, and a rendered form is a
/// form nobody has typed into. Everything below the markup — what a keystroke
/// saves, what Save & Connect connects to, what Test connection reports, and
/// the question a reconnect asks while a turn is running — is a closure, and
/// they were 63 of this file's 73 lines.
///
/// It presses on `views::sessions::pressing`, which is the same harness the
/// chats list uses, for a reason that is about this screen in particular: what
/// Save & Connect *does* is leave for the chats list with a live connection
/// behind it, so the two screens share one story and should share the socket
/// that tells it.
///
/// The one thing that is this screen's own is the second server. `probe` is
/// plain HTTP and not the WebSocket — `GET /status` proves the server is up,
/// `GET /acp` returning 406 proves the secret was accepted — so Test
/// connection needs an HTTP listener, and it is the only way to reach the
/// three sentences that button can say.
#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test scaffolding: a socket that will not open has nothing left \
              to assert, so failing loudly there IS the check"
)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "a mounted screen holds the harness's one-at-a-time guard, and it \
              has to hold it until the dom is gone — dropping it at the last \
              assertion is exactly what would let the next test's dom start \
              rendering into the same process-wide storage subscription"
)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use dioxus::prelude::*;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::{crumb, Shell};
    use crate::state::{AppCtx, ConnState, Screen, Settings};
    use crate::testkit::render_seeded;
    use crate::views::sessions::pressing::{every_element, taps_that, Mounted};

    // ---------------------------------------------------------------- seeds

    /// Six values that cannot be mistaken for one another, so a field bound to
    /// the wrong signal shows up as a value in the wrong place rather than as
    /// nothing at all.
    ///
    /// The fingerprint is a REAL one — 32 hex bytes — and that is load-bearing
    /// twice over. `connect_config` refuses to build at all on a fingerprint it
    /// cannot parse, so a decorative one would make every "the connection
    /// failed" assertion below pass without a socket ever being opened; and
    /// `on_test` reads the same value to decide whether the probe should skip
    /// certificate validation, which is the arm a blank one never reaches.
    fn saved() -> Settings {
        Settings {
            server_url: "https://goose-box.tailnet.ts.net".to_owned(),
            secret_key: "seeded-secret".to_owned(),
            fingerprint: "9F:2C:41:AB:6D:3E:05:17:9F:2C:41:AB:6D:3E:05:17:\
                          9F:2C:41:AB:6D:3E:05:17:9F:2C:41:AB:6D:3E:05:17"
                .to_owned(),
            working_dir: "/home/demo/projects".to_owned(),
            code_server_url: "https://brain.tailnet.ts.net:4300".to_owned(),
            code_password: "seeded-code-password".to_owned(),
        }
    }

    fn a_saved_server(ctx: &AppCtx) {
        let mut settings = ctx.settings;
        settings.set(saved());
    }

    fn connected(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connected {
            agent: "goose 1.10".to_owned(),
        });
    }

    /// Connected, wanting to be, with a turn running somewhere.
    fn connected_with_a_turn_running(ctx: &AppCtx) {
        connected(ctx);
        let (mut want, mut running) = (ctx.want_connected, ctx.running_sessions);
        want.set(true);
        running.write().insert("s-1".to_owned());
    }

    fn connected_and_wanted(ctx: &AppCtx) {
        connected(ctx);
        let mut want = ctx.want_connected;
        want.set(true);
    }

    /// Wanting a connection and not having one: what the app is between a
    /// failed attempt and the next. There is nothing to disconnect from here,
    /// and the screen must not offer to.
    fn wanted_but_not_connected(ctx: &AppCtx) {
        let mut want = ctx.want_connected;
        want.set(true);
    }

    fn connecting(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connecting);
    }

    fn refused(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Failed(
            "handshake failed: HTTP 401 Unauthorized".to_owned(),
        ));
    }

    fn settings_view() -> Element {
        rsx! { super::SettingsView {} }
    }

    /// A goose that answers the one call this screen makes after connecting.
    ///
    /// The `Result` belongs to the harness's `Script` signature rather than to
    /// this goose, which has nothing to refuse.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the signature belongs to the harness's `Script`, not to this \
                  function"
    )]
    fn a_goose_with_one_chat(method: &str, _params: &Value) -> Result<Value, Value> {
        match method {
            "session/list" => Ok(json!({
                "sessions": [{ "sessionId": "s-1", "cwd": "/home/demo", "title": "Standup" }]
            })),
            _ => Ok(json!({})),
        }
    }

    /// A port nothing is listening on, so a connection attempt is refused
    /// rather than left hanging on a name that has to be resolved first.
    const NOWHERE: &str = "http://127.0.0.1:1";

    fn head(html: &str) -> &str {
        &html[..html.len().min(600)]
    }

    // --------------------------------------------------------- the buttons

    // Neither of the two buttons at the foot of the form can be pressed by its
    // word. Both words are chosen by an `if` inside the button, and a literal
    // in either arm of an `if` compiles to a template of its own — so "Test
    // connection" and "Save & Connect" never appear in the mutation stream at
    // all, and neither does any class or `title` that would tell the two apart.
    //
    // What does tell them apart is the one attribute each of them binds:
    // `disabled`, first on the secondary and then on the primary, in the order
    // the form declares them. That is document order and nothing else — so
    // every assertion below is about what the press DID (a probe request, a
    // handshake, a question) rather than about which button was found, and a
    // press that landed on the other one fails rather than passing quietly.

    fn press_test(screen: &mut Mounted) {
        screen.press_with_attribute("disabled", 0);
    }

    fn press_connect(screen: &mut Mounted) {
        screen.press_with_attribute("disabled", 1);
    }

    // ------------------------------------------------------- what it shows

    /// Everything saved is in the field it was saved from.
    ///
    /// Six drafts are initialised from one struct in six lines that differ by a
    /// word, and two of them crossed would put the secret in the fingerprint
    /// box — where it is not a password field, and where saving would send it
    /// to `parse_fingerprint` instead of to the server. So this asserts on the
    /// ORDER as well: the values have to appear down the page in the order the
    /// form asks for them.
    #[test]
    fn every_saved_setting_comes_back_into_its_own_field() {
        let html = render_seeded(a_saved_server, settings_view);
        let saved = saved();
        let fields = [
            saved.server_url,
            saved.secret_key,
            saved.fingerprint,
            saved.working_dir,
            saved.code_server_url,
            saved.code_password,
        ];
        let mut at = 0;
        for value in fields {
            let found = html.find(&format!("value=\"{value}\"")).unwrap_or_else(|| {
                panic!("{value:?} is saved and no field on the screen shows it: {html}")
            });
            assert!(
                found > at,
                "{value:?} is on the screen out of order, so two fields are \
                 crossed and what you typed into one is saved as the other"
            );
            at = found;
        }
    }

    /// A disconnected screen offers to connect and claims nothing else. The
    /// Disconnect button and the "Connected to" line are both answers about a
    /// connection that does not exist.
    #[test]
    fn a_disconnected_screen_offers_to_connect_and_claims_nothing() {
        let html = render_seeded(a_saved_server, settings_view);
        assert!(
            html.contains(">Save &#38; Connect<"),
            "the primary button does not offer to connect: {}",
            head(&html)
        );
        assert!(
            !html.contains("Disconnect"),
            "a disconnected screen offers to disconnect: {html}"
        );
        assert!(
            !html.contains("Connected to"),
            "a disconnected screen names an agent it is not talking to: {html}"
        );
        assert!(
            !html.contains("error-box"),
            "nothing has failed yet and the screen is showing an error"
        );
    }

    /// The bar shows the connection as a bare dot, so this screen is the one
    /// place the thing on the other end is named — and the way off it lives
    /// here too.
    #[test]
    fn a_connected_screen_names_the_agent_and_offers_the_way_off() {
        let html = render_seeded(connected, settings_view);
        assert!(
            html.contains("Connected to goose 1.10"),
            "the screen does not say what it is connected to: {html}"
        );
        assert!(
            html.contains("Disconnect"),
            "a connected screen offers no way to disconnect: {html}"
        );
        assert!(
            html.contains(">Reconnect<"),
            "the primary button still offers to connect something already \
             connected: {html}"
        );
    }

    /// A connection in flight has to say so on the button and refuse a second
    /// press. Both buttons: a Test connection fired mid-handshake would put a
    /// second socket on the same server and report about the wrong one.
    #[test]
    fn a_connection_in_flight_disables_both_buttons() {
        let html = render_seeded(connecting, settings_view);
        assert!(
            html.contains("Connecting…"),
            "the button says nothing while the handshake is out: {html}"
        );
        assert_eq!(
            html.matches("disabled").count(),
            2,
            "both buttons should be disabled while a connection is in flight, \
             or a second press starts a second handshake: {html}"
        );
    }

    /// A failed connection prints what the server or the transport actually
    /// said. The alternative — a generic "could not connect" — is the one
    /// screen in the app that can tell you the secret was rejected rather than
    /// that the machine is asleep.
    #[test]
    fn a_failed_connection_puts_the_servers_own_words_on_screen() {
        let html = render_seeded(refused, settings_view);
        assert!(
            html.contains("handshake failed: HTTP 401 Unauthorized"),
            "the reason the connection failed never reached the reader: {html}"
        );
        assert!(
            html.contains("error-box"),
            "the failure is on screen as ordinary prose: {html}"
        );
    }

    /// The screen is named once, and the header is what shows the name. On the
    /// desktop the window's bar reads the same expression, so a header that
    /// went its own way would put two different names on one screen.
    /// The Tailscale hint names a device, and this screen is shared, so it has
    /// to name the device the reader is actually holding.
    ///
    /// Both arms asserted here rather than through the render, because only one
    /// of them can ever be rendered in a `cargo test`: the host build is
    /// `Shell::Desktop`, so a render-only check would leave the phone's
    /// wording — the one that shipped, and the one `assets/main.css` states
    /// nothing about — verified by nothing at all. The render half below is
    /// then the other question: that the screen consumes the function rather
    /// than carrying a second, frozen copy of the sentence.
    #[test]
    fn the_tailscale_hint_names_the_machine_the_reader_is_holding() {
        assert_eq!(super::tailscale_host(Shell::Mobile), "this phone");
        assert_eq!(super::tailscale_host(Shell::Desktop), "this computer");

        let html = render_seeded(a_saved_server, settings_view);
        assert!(
            html.contains(&format!(
                "Tailscale app enabled on {}.",
                super::tailscale_host(Shell::CURRENT)
            )),
            "the hint is not the one this shell selects, so the sentence has \
             been frozen somewhere the shell cannot reach it: {html}"
        );
        assert!(
            !html.contains("this phone"),
            "a 1440x860 window is telling the reader they are holding a \
             phone: {html}"
        );
    }

    #[test]
    fn the_screen_is_named_once() {
        assert_eq!(crumb().title, "Settings");
        let html = render_seeded(a_saved_server, settings_view);
        assert!(
            html.contains(&format!(
                "<h1 class=\"title ellipsis\">{}</h1>",
                crumb().title
            )),
            "the header is not showing the one name this screen has: {}",
            head(&html)
        );
    }

    // ------------------------------------------------------------- typing

    /// What is typed is what is saved, trimmed, into the field it was typed
    /// into.
    ///
    /// Six `oninput` closures and one `save` that reads six drafts: a pair
    /// crossed anywhere along that path silently sends the code-server password
    /// as the goose secret. The trimming is not cosmetic either — a URL or a
    /// secret with a pasted trailing newline fails the handshake with an error
    /// about the value, not about the space.
    #[test]
    fn what_is_typed_is_what_is_saved_and_it_is_saved_trimmed() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        for (nth, typed) in [
            "  http://127.0.0.1:1  ",
            "  typed-secret\n",
            " AA:BB:CC:DD ",
            "  /srv/work  ",
            "  https://brain:4300  ",
            " typed-code-password ",
        ]
        .into_iter()
        .enumerate()
        {
            screen.type_into_nth(nth, typed);
        }

        press_connect(&mut screen);
        screen.settle();

        // Field by field rather than one comparison of the whole struct:
        // `Settings` holds two secrets and deliberately does not derive
        // `Debug`, so there is nothing to print a mismatch with — and naming
        // the field is what makes a crossed pair say which pair.
        let stored = screen.with(|ctx| ctx.settings.peek().clone());
        for (field, got, want) in [
            ("the server URL", stored.server_url, "http://127.0.0.1:1"),
            ("the secret key", stored.secret_key, "typed-secret"),
            ("the fingerprint", stored.fingerprint, "AA:BB:CC:DD"),
            ("the working directory", stored.working_dir, "/srv/work"),
            (
                "the code server URL",
                stored.code_server_url,
                "https://brain:4300",
            ),
            (
                "the code password",
                stored.code_password,
                "typed-code-password",
            ),
        ] {
            assert_eq!(
                got, want,
                "{field} was saved as something else, so a field is bound to \
                 the wrong signal or the value was stored untrimmed"
            );
        }
    }

    /// A connection that cannot be made leaves the reader on the screen that
    /// can fix it, with the reason on it. Moving to the chats list on a failure
    /// would show an empty list and no way to tell why.
    #[test]
    fn a_connection_that_fails_stays_on_the_screen_that_can_fix_it() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        screen.type_into_nth(0, NOWHERE);
        press_connect(&mut screen);
        screen.settle();

        screen.with(|ctx| {
            let ConnState::Failed(reason) = &*ctx.conn.peek() else {
                panic!("a refused connection did not end as a failure");
            };
            // Named, because a fixture with an unparseable fingerprint in it
            // would fail here BEFORE a socket was opened — and this whole test
            // would pass having proved nothing about a connection at all.
            assert!(
                !reason.contains("fingerprint"),
                "the connection was refused by the app's own validation rather \
                 than by the machine at the other end: {reason}"
            );
            assert!(
                matches!(*ctx.screen.peek(), Screen::Settings),
                "the app left for the chats list on a connection that never \
                 happened"
            );
        });
        assert!(
            screen.html().contains("error-box"),
            "the failure is not on the screen: {}",
            head(&screen.html())
        );
    }

    /// Save & Connect connects to what was JUST TYPED, not to what was saved
    /// before it — the drafts are local signals and `save` runs first for
    /// exactly this reason — and it ends on the chats list with the list
    /// fetched.
    ///
    /// The last part is the one a reader would notice: without the
    /// `refresh_sessions` after the screen change, connecting lands on an empty
    /// Chats screen that only a pull-to-refresh fills in.
    #[test]
    fn save_and_connect_uses_what_was_typed_and_ends_on_the_chats_list() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        let server = screen.serve(a_goose_with_one_chat);
        screen.type_into_nth(0, &server.base_url);

        press_connect(&mut screen);
        screen.settle();

        screen.with(|ctx| {
            assert!(
                matches!(*ctx.conn.peek(), ConnState::Connected { .. }),
                "the typed server was never reached, so the field the button \
                 reads is not the field that was typed into"
            );
            assert!(
                matches!(*ctx.screen.peek(), Screen::Sessions),
                "the connection was made and the reader was left on Settings"
            );
        });
        assert_eq!(
            server.count("session/list"),
            1,
            "connecting landed on a Chats screen nothing had fetched: {:?}",
            server.methods()
        );
    }

    // ------------------------------------------------------ testing the URL

    /// Test connection is the button that separates "the box is asleep" from
    /// "the secret is wrong", and it can only do that by asking. 406 on
    /// `GET /acp` is goose's documented auth-success signal, and the secret in
    /// the field has to be the one that goes with the request — a test that
    /// passed without it would report a server as reachable using nobody's
    /// credentials.
    #[test]
    fn a_reachable_server_that_takes_the_secret_is_reported_as_both() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        let probe = serve_probe(&screen, 406, Duration::ZERO);
        screen.type_into_nth(0, &probe.base_url);
        screen.type_into_nth(1, "typed-secret");

        press_test(&mut screen);
        screen.settle();

        assert_eq!(
            screen.with(|ctx| ctx.toast.peek().clone()),
            Some("Server reachable, secret accepted ✓".to_owned()),
            "a server that accepted the secret was not reported as accepting it"
        );
        assert_eq!(
            probe.secret_for("/acp"),
            Some("typed-secret".to_owned()),
            "the secret in the field is not the secret the probe sent, so a \
             pass here says nothing about the key that will be used"
        );
    }

    /// The failure this button exists to tell apart from the other one. A
    /// rejected secret is a server that is up, and saying "unreachable" about
    /// it sends the reader to check the tailnet instead of the key.
    #[test]
    fn a_rejected_secret_is_not_reported_as_an_unreachable_server() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        let probe = serve_probe(&screen, 401, Duration::ZERO);
        screen.type_into_nth(0, &probe.base_url);

        press_test(&mut screen);
        screen.settle();

        let toast = screen
            .with(|ctx| ctx.toast.peek().clone())
            .unwrap_or_default();
        assert!(
            toast.contains("the secret key was rejected"),
            "a 401 was not reported as a rejected key: {toast}"
        );
        assert!(
            !toast.contains("Unreachable"),
            "a server that answered was reported as unreachable: {toast}"
        );
    }

    /// And the third answer: nothing there at all. It carries the transport's
    /// own words, because "unreachable" on its own does not distinguish a
    /// misspelled host from a machine that is asleep.
    #[test]
    fn a_server_that_is_not_there_is_reported_with_what_went_wrong() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        screen.type_into_nth(0, NOWHERE);

        press_test(&mut screen);
        screen.settle();

        let toast = screen
            .with(|ctx| ctx.toast.peek().clone())
            .unwrap_or_default();
        assert!(
            toast.starts_with("Unreachable:"),
            "a refused connection was not reported as unreachable: {toast}"
        );
        assert!(
            toast.len() > "Unreachable:".len() + 1,
            "the reason was dropped, so the reader is told it failed and not \
             what failed: {toast}"
        );
    }

    /// While the probe is out the button says so and refuses a second press.
    /// Both matter on a phone: the round trip is seconds long over a tailnet,
    /// and a button that looks idle gets pressed again — which is a second
    /// probe whose answer arrives after the first and overwrites it.
    #[test]
    fn the_test_button_says_it_is_working_and_will_not_be_pressed_twice() {
        let mut screen = Mounted::mount(a_saved_server, settings_view);
        let probe = serve_probe(&screen, 406, Duration::from_millis(400));
        screen.type_into_nth(0, &probe.base_url);

        press_test(&mut screen);
        screen.settle_for(5);
        let mid = screen.html();
        assert!(
            mid.contains("Testing…"),
            "the button is idle while a probe is out: {}",
            head(&mid)
        );
        assert!(
            mid.contains("disabled"),
            "the button can be pressed again while its own probe is still \
             out: {}",
            head(&mid)
        );

        screen.settle_for(120);
        let after = screen.html();
        assert!(
            after.contains("Test connection"),
            "the button never came back, so one slow probe disables it for \
             good: {}",
            head(&after)
        );
    }

    // -------------------------------------------------- reconnect, and off

    /// Reconnecting closes the live client, and a Close frame under a running
    /// turn destroys that turn's round on the server — prompt and title
    /// survive, the answer does not, and nothing the app does afterwards can
    /// get it back (`docs/permission-durability.md` section 0). So it asks
    /// first, and asking means NOT connecting yet.
    #[test]
    fn a_running_turn_is_asked_about_before_the_reconnect_throws_it_away() {
        let mut screen = Mounted::mount(connected_with_a_turn_running, settings_view);
        screen.type_into_nth(0, NOWHERE);
        press_connect(&mut screen);

        let html = screen.html();
        assert!(
            html.contains("A turn is still running"),
            "a reconnect went through under a running turn with no question \
             in between: {}",
            head(&html)
        );
        assert!(
            html.contains("Your message stays in the chat; the reply does not."),
            "the question does not say what is lost: {html}"
        );
        screen.settle();
        screen.with(|ctx| {
            assert!(
                matches!(*ctx.conn.peek(), ConnState::Connected { .. }),
                "the connection was torn down by the question about tearing \
                 it down"
            );
        });

        screen.press("Reconnect anyway");
        screen.settle();
        assert!(
            !screen.html().contains("A turn is still running"),
            "the question stayed up after it was answered"
        );
        screen.with(|ctx| {
            assert!(
                matches!(*ctx.conn.peek(), ConnState::Failed(_)),
                "answering the question did not reconnect, so the only way \
                 past it is to lose the turn for nothing"
            );
        });
    }

    /// The other half: the question can be declined, and declining changes
    /// nothing at all.
    #[test]
    fn declining_the_reconnect_leaves_the_connection_alone() {
        let mut screen = Mounted::mount(connected_with_a_turn_running, settings_view);
        press_connect(&mut screen);
        assert!(
            screen.html().contains("A turn is still running"),
            "the question never opened, so there is nothing to decline"
        );

        screen.press_first();
        screen.settle();

        assert!(
            !screen.html().contains("A turn is still running"),
            "Cancel left the question on screen"
        );
        screen.with(|ctx| {
            assert!(
                matches!(*ctx.conn.peek(), ConnState::Connected { .. }),
                "declining the reconnect reconnected anyway"
            );
            assert!(
                *ctx.want_connected.peek(),
                "declining the reconnect gave up on the connection"
            );
        });
    }

    /// With nothing running there is nothing to lose, so the same button goes
    /// straight through. The confirm is for the case that has a cost; a
    /// confirm on every reconnect is a confirm nobody reads.
    #[test]
    fn a_reconnect_with_nothing_running_asks_nothing() {
        let mut screen = Mounted::mount(connected_and_wanted, settings_view);
        screen.type_into_nth(0, NOWHERE);
        press_connect(&mut screen);

        assert!(
            !screen.html().contains("A turn is still running"),
            "a reconnect with no turn running asked about a turn"
        );
        screen.settle();
        screen.with(|ctx| {
            assert!(
                matches!(*ctx.conn.peek(), ConnState::Failed(_)),
                "the reconnect never left"
            );
        });
    }

    /// Exactly one control gives up the connection, and it is not one of the
    /// two that make one.
    ///
    /// `want_connected` is the flag the app's own reconnect logic reads, so a
    /// Disconnect that closed the socket without clearing it would be undone
    /// by the next reconnect attempt — the button would look like it had done
    /// nothing. Counted rather than named because every word and class on it is
    /// a literal.
    #[test]
    fn exactly_one_control_gives_up_the_connection() {
        fn gave_up(ctx: &AppCtx) -> bool {
            !*ctx.want_connected.peek()
        }
        assert_eq!(
            taps_that(connected_and_wanted, settings_view, gave_up),
            1,
            "either nothing on the settings screen disconnects, or something \
             else does it by accident"
        );
        assert_eq!(
            every_element(wanted_but_not_connected, settings_view, gave_up).count(),
            0,
            "a screen with no connection to give up still has a control that \
             gives one up"
        );
    }

    // ------------------------------------------- the HTTP server `probe` uses

    /// What each request asked for, and what it carried as its secret.
    type Requests = Arc<Mutex<Vec<(String, Option<String>)>>>;

    struct HttpProbe {
        base_url: String,
        requests: Requests,
    }

    impl HttpProbe {
        /// The `X-Secret-Key` the probe sent with its request for `path`.
        fn secret_for(&self, path: &str) -> Option<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .find(|(target, _)| target == path)
                .and_then(|(_, secret)| secret.clone())
        }
    }

    /// A server that answers `GET /status` with 200 and `GET /acp` with
    /// `acp_status`, after `delay`.
    ///
    /// On the harness's own runtime, which is the same current-thread runtime
    /// the probe it is answering will run on. That works because the wait is a
    /// `tokio::time::sleep` and not a sleeping thread: the harness drives that
    /// runtime in 10 ms slices between renders, and a blocking sleep in here
    /// would stop the client's half from being polled at all — a deadlock
    /// dressed up as a timeout.
    fn serve_probe(screen: &Mounted, acp_status: u16, delay: Duration) -> HttpProbe {
        let rt = screen.runtime();
        let listener = rt.block_on(async { TcpListener::bind("127.0.0.1:0").await.unwrap() });
        let port = listener.local_addr().unwrap().port();
        let requests: Requests = Arc::default();
        let log = Arc::clone(&requests);
        rt.spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut buf = [0_u8; 2048];
                    let Ok(read) = socket.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..read]).to_string();
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    let secret = request.lines().find_map(|line| {
                        line.strip_prefix("x-secret-key: ")
                            .or_else(|| line.strip_prefix("X-Secret-Key: "))
                            .map(str::to_owned)
                    });
                    let status = if path == "/status" { 200 } else { acp_status };
                    log.lock().unwrap().push((path, secret));
                    tokio::time::sleep(delay).await;
                    // The reason phrase is not read by anything here, and
                    // `connection: close` is: `probe` makes two requests, and
                    // a kept-alive socket would have the second one waiting on
                    // a task that has already answered and gone.
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        HttpProbe {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
        }
    }
}
