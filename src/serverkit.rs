//! The other half of [`crate::testkit`]: a real [`AcpClient`], talking to a
//! real socket, under a real [`AppCtx`].
//!
//! `testkit` mounts a *view* and hands back markup, which is the right shape
//! for `src/views/` and the wrong one for the state modules beside them. A
//! feature module like `src/extensions.rs` or `src/skills.rs` is almost
//! entirely code that runs AFTER a `let Some(client) = ... else { return }` —
//! store this credential, add that extension, re-list and compare — and
//! [`AcpClient`] has no constructor that is not `connect`. So without a server
//! every one of those functions stops at its first line and the branch that
//! matters is unreachable.
//!
//! `src/scheduler.rs`'s test module solved that first: stand up a plain-`ws://`
//! JSON-RPC listener on a loopback port and connect a real client to it.
//! `ws_url` only reaches for TLS on an `https://` base, so `http://127.0.0.1`
//! means no certificate, no fingerprint and no mock of the client itself. This
//! is that harness lifted out of one feature's tests so a second feature does
//! not need a second copy — a copy being the thing that goes quietly out of
//! step with the [`AppCtx`] the app actually builds.
//!
//! **Why the request log is the point.** A server that answers is only half of
//! it. The other half is [`Server::methods`] and [`Server::params`]: what went
//! out, in what order, and with what in it. That is how a test can say "the
//! credential was stored before the extension was added", or "no `set-enabled`
//! was ever sent", which are statements about the security properties these
//! modules exist to hold — and neither of them is visible in a signal after
//! the fact.
//!
//! **Why the context is built field by field** rather than by calling
//! `state::use_app_ctx_provider`. Two of that provider's fields are persistent
//! and reach `dioxus-sdk-storage`, which keeps ONE process-global sender per
//! key and `.unwrap()`s the send while every receiver belongs to a live
//! `VirtualDom`. A harness that holds a dom open across socket work would then
//! be racing every other mounted dom in the binary, and Dioxus swallows a panic
//! thrown during render — so the symptom is not a stack trace but a test that
//! sometimes sees an empty signal. This one touches no disk and subscribes to
//! nothing, so it cannot take part in that race. The cost is that a new field
//! on [`AppCtx`] fails to compile here, which is the intended trade: the
//! alternative is a harness quietly handing a feature a context the app never
//! builds.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use futures_util::{SinkExt as _, StreamExt as _};
use goose_acp_client::{AcpClient, ConnectConfig};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::state::{AppCtx, Tab};

thread_local! {
    /// Where [`Probe`] leaves the context it built, for [`Harness::offline`]
    /// to collect. A thread-local rather than a static: `cargo test` runs
    /// these in parallel and two mounts on two threads must not see each
    /// other's.
    static PUBLISHED: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
}

/// One component holding one [`AppCtx`], built field by field. See the module
/// doc for why it is not `state::use_app_ctx_provider`.
#[expect(
    non_snake_case,
    reason = "a Dioxus component is named like a component, not like a fn"
)]
fn Probe() -> Element {
    let ctx = AppCtx {
        screen: use_signal(|| crate::state::Screen::Settings),
        settings: use_signal(crate::state::Settings::default),
        conn: use_signal(|| crate::state::ConnState::Disconnected),
        client: use_signal(|| None),
        want_connected: use_signal(|| false),
        sessions: use_signal(Vec::new),
        sessions_next: use_signal(|| None),
        sessions_loading: use_signal(|| false),
        sessions_query: use_signal(String::new),
        sessions_epoch: use_signal(|| 0),
        chat: use_signal(crate::state::ChatState::default),
        running_sessions: use_signal(HashSet::new),
        permission: use_signal(Vec::new),
        lost_asks: use_signal(Vec::new),
        usage: use_signal(|| None),
        config_options: use_signal(Vec::new),
        chat_draft: use_signal(String::new),
        toast: use_signal(|| None),
        attachments: use_signal(Vec::new),
        attach_reading: use_signal(Vec::new),
        tab: use_signal(|| Tab::Home),
        drawer_open: use_signal(|| false),
        inspector_open: use_signal(|| true),
        code_screen: use_signal(|| crate::code::CodeScreen::List),
        code_client: use_signal(|| None),
        code_conn: use_signal(|| crate::state::ConnState::Disconnected),
        code_chats: use_signal(Vec::new),
        code_chats_loading: use_signal(|| false),
        code_repos: use_signal(Vec::new),
        code_models: use_signal(Vec::new),
        code_models_loading: use_signal(|| false),
        code_agents: use_signal(Vec::new),
        code_agents_from: use_signal(String::new),
        code_agents_loading: use_signal(|| false),
        code_branches: use_signal(crate::code::BranchList::default),
        code_chat: use_signal(crate::code::CodeChatState::default),
        code_permissions: use_signal(Vec::new),
        code_answered: use_signal(HashSet::new),
        code_cache: use_signal(crate::code::CodeCache::default),
        code_epoch: use_signal(|| 0),
        code_poll: use_signal(|| 0),
        code_stream: use_signal(|| None),
        code_diff: use_signal(crate::code::DiffState::default),
        code_pulls: use_signal(crate::code::PullsState::default),
        code_diff_wrap: use_signal(|| true),
        code_draft: use_signal(String::new),
        code_attachments: use_signal(Vec::new),
        new_attachments: use_signal(Vec::new),
        extensions: crate::extensions::use_ctx(),
        skills: crate::skills::use_ctx(),
        recipes: crate::recipes::use_recipes(),
        scheduler: crate::scheduler::use_ctx(),
    };
    use_context_provider(|| ctx);
    PUBLISHED.with(|slot| *slot.borrow_mut() = Some(ctx));
    rsx! { div {} }
}

/// The reply the mock server sends for one request: how long it sits on it,
/// then a JSON-RPC `result` or `error`.
pub(crate) type Reply = (Duration, Result<Value, Value>);

/// What a mock server answers, per method. A plain `fn` and never a closure,
/// so the whole script of a test is one readable `match`.
pub(crate) type Script = fn(&str, &Value) -> Reply;

/// A successful result, answered at once.
pub(crate) fn ok(result: Value) -> Reply {
    (Duration::ZERO, Ok(result))
}

/// A JSON-RPC error. `-32601` is goose's own "this feature is absent", which
/// the client turns into `AcpError::Unsupported`; anything else is a plain
/// failure.
pub(crate) fn rpc_error(code: i64, message: &str) -> Reply {
    (
        Duration::ZERO,
        Err(json!({ "code": code, "message": message })),
    )
}

/// The method name with goose's unstable namespace taken off, so a script and
/// its assertions read as `config/extensions/list` rather than as a URL.
pub(crate) fn short(method: &str) -> &str {
    method.trim_start_matches("_goose/unstable/")
}

/// A handle on the mock server: its address, and everything it was asked.
pub(crate) struct Server {
    base_url: String,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

#[expect(
    clippy::expect_used,
    reason = "test scaffolding: a missing call is the failing check, and the \
              message names it"
)]
impl Server {
    /// Every request this server was sent, in order, by short name. The
    /// handshake is left out: it is the harness's, not the feature's.
    pub(crate) fn methods(&self) -> Vec<String> {
        self.log()
            .iter()
            .map(|(method, _)| short(method).to_owned())
            .filter(|method| method != "initialize")
            .collect()
    }

    pub(crate) fn count(&self, method: &str) -> usize {
        self.methods().iter().filter(|m| *m == method).count()
    }

    /// The params of the `n`th call to `method`.
    pub(crate) fn params(&self, method: &str, n: usize) -> Value {
        self.log()
            .iter()
            .filter(|(m, _)| short(m) == method)
            .nth(n)
            .map(|(_, params)| params.clone())
            .expect("the call the assertion is about was never made")
    }

    /// Every request body this server received, as one JSON array.
    ///
    /// For the assertion a per-call getter cannot make: that a secret the user
    /// typed appears in exactly one frame and nowhere else.
    pub(crate) fn frames(&self) -> Value {
        Value::Array(
            self.log()
                .iter()
                .map(|(method, params)| json!({ "method": method, "params": params }))
                .collect(),
        )
    }

    fn log(&self) -> std::sync::MutexGuard<'_, Vec<(String, Value)>> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A mounted context, a tokio runtime, and optionally a live client.
pub(crate) struct Harness {
    dom: VirtualDom,
    rt: tokio::runtime::Runtime,
    ctx: AppCtx,
    /// The connection's event stream, parked here for its lifetime. The
    /// client's actor gives up the socket when this end goes away, so a
    /// harness that dropped it would have a connection that died between the
    /// handshake and the first request.
    events: Option<tokio::sync::mpsc::Receiver<goose_acp_client::AcpEvent>>,
}

#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test scaffolding: a harness that cannot start is the failing check"
)]
impl Harness {
    /// A mounted app context with no connection: the offline half of every
    /// action.
    pub(crate) fn offline() -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut dom = VirtualDom::new(Probe);
        dom.rebuild_in_place();
        let ctx = PUBLISHED
            .with(|slot| *slot.borrow())
            .expect("the probe rendered, so it published its context");
        Self {
            dom,
            rt,
            ctx,
            events: None,
        }
    }

    /// The same, plus a live client talking to a server running `script`.
    pub(crate) fn connected(script: Script) -> (Self, Server) {
        let mut harness = Self::offline();
        let server = harness.serve(script);
        let cfg = ConnectConfig {
            base_url: server.base_url.clone(),
            secret: String::new(),
            fingerprint: None,
        };
        let (client, events, _info) = harness
            .rt
            .block_on(AcpClient::connect(&cfg))
            .expect("the mock server accepted the handshake");
        harness.events = Some(events);
        harness.with(|ctx| ctx.client.clone().set(Some(client)));
        (harness, server)
    }

    fn serve(&self, script: Script) -> Server {
        let listener = self
            .rt
            .block_on(async { TcpListener::bind("127.0.0.1:0").await.unwrap() });
        let port = listener.local_addr().unwrap().port();
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let log = Arc::clone(&calls);
        self.rt.spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                tokio::spawn(async move { session_loop(socket, log, script).await });
            }
        });
        Server {
            base_url: format!("http://127.0.0.1:{port}"),
            calls,
        }
    }

    /// Read or write the context. Signals belong to the virtual DOM's runtime
    /// and panic outside it, so every touch goes through here.
    pub(crate) fn with<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
        let ctx = self.ctx;
        self.dom.in_runtime(|| f(&ctx))
    }

    /// Call a synchronous entry point the way a tap would, then let whatever
    /// it spawned run to completion.
    pub(crate) fn act(&mut self, f: impl FnOnce(&AppCtx)) {
        self.with(f);
        self.settle();
    }

    /// Drive an `async fn` that takes the context — the shape a `use_future`
    /// on a screen would put on the executor — and let it finish.
    pub(crate) fn drive<Fut>(&mut self, f: impl FnOnce(AppCtx) -> Fut)
    where
        Fut: std::future::Future<Output = ()> + 'static,
    {
        let task = f(self.ctx);
        self.dom.in_runtime(|| spawn_forever(task));
        self.settle();
    }

    /// Let the queued Dioxus tasks — and the socket under them — run.
    ///
    /// Dioxus polls a spawned task from its own executor, so nothing an action
    /// started happens without this; the timeout is what lets the tokio
    /// runtime carrying the WebSocket actor make progress while the virtual
    /// DOM has nothing to do. The budget is 400 ms of *idle* — an iteration
    /// with work in it returns at once — so a loaded machine has room before
    /// this becomes a flake.
    pub(crate) fn settle(&mut self) {
        let dom = &mut self.dom;
        self.rt.block_on(async {
            for _ in 0..40 {
                let _ = tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
                dom.render_immediate_to_vec();
            }
        });
    }

    pub(crate) fn toast(&self) -> Option<String> {
        self.with(|ctx| ctx.toast.peek().clone())
    }

    /// Put an absolute (or deliberately not-absolute) working directory into
    /// Settings, which is where every `cwd` and `projectDir` in these features
    /// comes from.
    pub(crate) fn set_working_dir(&self, dir: &str) {
        self.with(|ctx| {
            let mut settings = ctx.settings;
            settings.write().working_dir = dir.to_owned();
        });
    }
}

async fn session_loop(
    socket: tokio::net::TcpStream,
    log: Arc<Mutex<Vec<(String, Value)>>>,
    script: Script,
) {
    let Ok(ws) = tokio_tungstenite::accept_async(socket).await else {
        return;
    };
    let (mut sink, mut stream) = ws.split();
    let (out, mut outbox) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(text) = outbox.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
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
        log.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((method.clone(), params.clone()));
        let out = out.clone();
        // Answered on a task of its own, so a scripted delay holds up one
        // reply rather than the whole socket.
        tokio::spawn(async move {
            let (delay, body) = if method == "initialize" {
                ok(json!({
                    "protocolVersion": 1,
                    "agentInfo": { "name": "mock", "version": "0" },
                }))
            } else {
                script(&method, &params)
            };
            tokio::time::sleep(delay).await;
            let frame = match body {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            };
            let _ = out.send(frame.to_string());
        });
    }
}
