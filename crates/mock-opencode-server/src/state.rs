//! What the mock knows, and the fixtures it starts with.
//!
//! One store for both tiers, because the real system's two tiers are one
//! machine: the manager knows a chat exists and the `OpenCode` server behind it
//! knows what was said in it, and a mock that split them would have to invent a
//! way for them to agree.
//!
//! Fixtures are dated RELATIVE TO PROCESS START, which is `mock-goose-server`'s
//! rule and its reason: a literal timestamp ages out, and within a week every
//! row in the app reads "3d" and the age badges stop being exercised at all.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the epoch, as the manager reports them.
pub(crate) fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// Milliseconds since the epoch — `OpenCode`'s own unit, and NOT the manager's.
///
/// The two tiers genuinely disagree: `ChatMeta.created`/`last_active` are
/// float SECONDS and `SessionMeta.time.created`/`updated` are integer
/// MILLISECONDS. Getting this backwards puts a session's timestamp in 1970 or
/// in the year 57000, and nothing in the client complains.
pub(crate) fn now_ms() -> i64 {
    ms(now())
}

/// Seconds to `OpenCode`'s milliseconds, and the one place the cast happens.
#[expect(
    clippy::cast_possible_truncation,
    reason = "an epoch millisecond is ~2^41, well inside i64, and a fixture \
              that lands a millisecond either side of where it meant to is a \
              fixture that still works"
)]
pub(crate) fn ms(secs: f64) -> i64 {
    (secs * 1000.0) as i64
}

/// How much the store starts with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fixtures {
    /// A fleet worth looking at: several repos, a tree in each state, an ask
    /// parked in one, a pull request, a diff.
    Full,
    /// Connected and empty — the state a new gateway is in, and the one every
    /// empty-list message in the app is written for.
    Empty,
}

/// One working tree, as the manager reports it.
#[derive(Clone)]
pub(crate) struct Chat {
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) title: String,
    pub(crate) branch: String,
    pub(crate) base: String,
    pub(crate) model: Option<String>,
    /// `running` | `stopped` | `absent` — the CONTAINER's lifecycle, not a
    /// turn's. The app's own `code::status_label` calls a running container
    /// with no turn in flight "idle".
    pub(crate) status: String,
    pub(crate) created: f64,
    pub(crate) last_active: f64,
    /// The session inside this chat's `OpenCode` server.
    pub(crate) session: Session,
    /// Asks parked in it, front of the queue first.
    pub(crate) asks: Vec<Ask>,
    /// What `session/:id/diff` answers.
    pub(crate) diff: Vec<FileDiff>,
    /// What `/api/chats/:id/pulls` answers.
    pub(crate) pulls: Vec<Pull>,
}

#[derive(Clone)]
pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) created_ms: i64,
    pub(crate) updated_ms: i64,
    /// Chronological. Each is one message and its parts.
    pub(crate) messages: Vec<Message>,
}

#[derive(Clone)]
pub(crate) struct Message {
    pub(crate) id: String,
    /// `user` | `assistant`.
    pub(crate) role: String,
    pub(crate) created_ms: i64,
    pub(crate) parts: Vec<Part>,
}

/// A part of a message. `text`, `reasoning` and `tool` are the three the app
/// folds into a transcript; anything else it ignores, so the mock sends none.
#[derive(Clone)]
pub(crate) struct Part {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) text: String,
    /// For a `tool` part: the tool's name and its state.
    pub(crate) tool: Option<Tool>,
}

#[derive(Clone)]
pub(crate) struct Tool {
    pub(crate) name: String,
    /// `pending` | `running` | `completed` | `error`.
    pub(crate) status: String,
    pub(crate) title: String,
    pub(crate) output: String,
}

#[derive(Clone)]
pub(crate) struct Ask {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) command: String,
}

#[derive(Clone)]
pub(crate) struct FileDiff {
    pub(crate) file: String,
    pub(crate) patch: String,
    pub(crate) additions: u32,
    pub(crate) deletions: u32,
    pub(crate) status: String,
}

#[derive(Clone)]
pub(crate) struct Pull {
    pub(crate) number: u64,
    pub(crate) title: String,
    /// `open` | `merged` | `closed`.
    pub(crate) state: String,
    pub(crate) draft: bool,
    pub(crate) mergeable: Option<bool>,
    /// `passing` | `failing` | `pending` | `none`.
    pub(crate) checks: String,
    pub(crate) head: String,
    pub(crate) base: String,
}

/// One allowed repo.
#[derive(Clone)]
pub(crate) struct Repo {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) default_branch: String,
    pub(crate) branches: Vec<String>,
    /// A repo the manager will let a FREE model touch. `code::is_free_model`
    /// is gated on it, so without one in the fixtures that whole rule — and
    /// the model picker's behaviour on the repos it applies to — is unreachable.
    pub(crate) public_throwaway: bool,
}

pub(crate) struct State {
    pub(crate) password: String,
    pub(crate) chats: Vec<Chat>,
    pub(crate) repos: Vec<Repo>,
    /// Chats the manager could not reach on its last sweep, reported by
    /// `/api/permissions` so the app can say a container is unreachable rather
    /// than that it has nothing to ask.
    pub(crate) unreachable: Vec<String>,
    /// Turns the mock has been asked to run and has not finished, keyed by
    /// chat id. The SSE stream drains these.
    pub(crate) pending: HashMap<String, Vec<Step>>,
}

/// One thing the mock will do to a chat, a beat at a time, when a prompt
/// arrives. This is what makes the fake feel alive rather than static: the app
/// renders a transcript that grows, a tool that runs and finishes, and a
/// permission ask that blocks until it is answered.
#[derive(Clone)]
pub(crate) enum Step {
    /// Announce a message so the transcript knows whose it is. The app's role
    /// map is fed ONLY by `message.updated`, so a part whose message was never
    /// announced renders as an assistant bubble whatever it really was.
    Message { id: String, role: String },
    /// Append or replace a part.
    Part { message: String, part: Part },
    /// Park an ask. The stream stops until it is answered.
    Ask(Ask),
    /// The turn is over.
    Idle,
    /// Wait, so a reader can watch it happen.
    Beat(u64),
}

impl State {
    pub(crate) fn from_env() -> Self {
        let password =
            std::env::var("MOCK_CODE_PASSWORD").unwrap_or_else(|_| "mock-code-secret".to_owned());
        let fixtures = match std::env::var("MOCK_FIXTURES").as_deref() {
            Ok("empty") => Fixtures::Empty,
            _ => Fixtures::Full,
        };
        let mut state = Self {
            password,
            chats: Vec::new(),
            repos: Vec::new(),
            unreachable: Vec::new(),
            pending: HashMap::new(),
        };
        if fixtures == Fixtures::Full {
            state.seed();
        } else {
            // Connected and empty is a STATE, not an absence: the repo list
            // still answers, because a gateway with no chats still knows which
            // repos it is allowed to touch, and the new-session screen needs
            // them to be pickable.
            state.repos = repos();
        }
        state
    }

    /// A fleet worth looking at: three repos, five trees, one of each state,
    /// an ask parked in one, a diff to review, and **four different builds**
    /// across four pull requests.
    ///
    /// The builds are the fixture issue #84 is about. Before them exactly one
    /// tree carried a pull request and its checks were `passing`, so a red
    /// build — the whole reason the list draws a build at all — could not be
    /// seen anywhere in the development loop, and neither could a running one
    /// or a branch that had already landed. `Checks::None` is deliberately
    /// still unrepresented: three of the fixtures carry no pull request, and
    /// "the repo runs no checks" is what the list is asked to say nothing
    /// about (`code::row_checks_label`).
    fn seed(&mut self) {
        let t = now();
        self.repos = repos();
        self.chats = vec![
            waiting_chat(t),
            awake_chat(t),
            reviewed_chat(t),
            asleep_chat(
                t,
                "goose-phone-app",
                "Add a --no-color flag",
                4.0 * 3600.0,
                // The red build. Open, mergeable as far as GitHub is
                // concerned, and the only thing wrong with it is the thing
                // the row now says.
                vec![pull(121, "open", "failing", Some(true))],
            ),
            asleep_chat(
                t,
                "notes-public",
                "Fix the broken anchor links",
                26.0 * 3600.0,
                // Landed. `mergeable` is null on a merged pull request —
                // GitHub stops computing it — which is also the shape that
                // proves the client is not coercing null to false.
                vec![pull(109, "merged", "passing", None)],
            ),
        ];
        // One container the manager swept and could not reach. The app has a
        // word for this and nothing else exercises it.
        self.unreachable = vec!["notes-public-4b71de".to_owned()];
    }

    pub(crate) fn chat(&self, id: &str) -> Option<&Chat> {
        self.chats.iter().find(|c| c.id == id)
    }

    pub(crate) fn chat_mut(&mut self, id: &str) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|c| c.id == id)
    }
}

fn repos() -> Vec<Repo> {
    vec![
        Repo {
            name: "goose-phone-app".to_owned(),
            url: "https://github.com/PhillipChaffee/goose-phone-app.git".to_owned(),
            default_branch: "main".to_owned(),
            branches: vec!["main".to_owned(), "desktop-match-the-designs".to_owned()],
            public_throwaway: false,
        },
        Repo {
            name: "personal-ai-setup".to_owned(),
            url: "https://github.com/PhillipChaffee/personal-ai-setup.git".to_owned(),
            default_branch: "main".to_owned(),
            branches: vec!["main".to_owned()],
            public_throwaway: false,
        },
        Repo {
            name: "notes-public".to_owned(),
            url: "https://github.com/PhillipChaffee/notes-public.git".to_owned(),
            default_branch: "main".to_owned(),
            branches: vec!["main".to_owned(), "drafts".to_owned()],
            public_throwaway: true,
        },
    ]
}

/// A tree blocked on the reader. The one state the whole app is arranged to
/// surface — an amber mark in the sidebar, an amber tile on the board, a count
/// in the window's band — so the fixture set is not worth much without one.
fn waiting_chat(t: f64) -> Chat {
    let id = "goose-phone-app-9f2c1a";
    Chat {
        id: id.to_owned(),
        repo: "goose-phone-app".to_owned(),
        title: "Tighten the composer's chip row".to_owned(),
        branch: format!("agent/{id}"),
        base: "main".to_owned(),
        model: Some("opencode/claude-sonnet-4-5".to_owned()),
        status: "running".to_owned(),
        created: t - 3.0 * 3600.0,
        last_active: t - 240.0,
        session: session(
            "ses_9f2c1a",
            "Tighten the composer's chip row",
            t,
            vec![
                user_message(
                    "msg_1",
                    t - 3.0 * 3600.0,
                    "Tighten the composer's chip row so the model name stops being cut at 360pt.",
                ),
                Message {
                    id: "msg_2".to_owned(),
                    role: "assistant".to_owned(),
                    created_ms: ms(t - 3.0 * 3600.0 + 6.0),
                    parts: vec![
                        text_part("prt_2a", "Reading the composer's stylesheet first."),
                        Part {
                            id: "prt_2b".to_owned(),
                            kind: "tool".to_owned(),
                            text: String::new(),
                            tool: Some(Tool {
                                name: "read".to_owned(),
                                status: "completed".to_owned(),
                                title: "assets/shared.css".to_owned(),
                                output: "3140 lines".to_owned(),
                            }),
                        },
                        text_part(
                            "prt_2c",
                            "The chip row is a flex line with no `min-width: 0`, so the model \
                             name cannot shrink below its content and the whole row overflows \
                             instead. I want to push the branch and let CI measure it.",
                        ),
                    ],
                },
            ],
        ),
        asks: vec![Ask {
            id: "per_1f4a".to_owned(),
            kind: "bash".to_owned(),
            title: "Run git push".to_owned(),
            command: format!("git push -u origin agent/{id}"),
        }],
        diff: vec![],
        pulls: vec![],
    }
}

/// A tree whose container is up and mid-turn.
fn awake_chat(t: f64) -> Chat {
    let id = "personal-ai-setup-3c7e01";
    Chat {
        id: id.to_owned(),
        repo: "personal-ai-setup".to_owned(),
        title: "Document the code-agent ports".to_owned(),
        branch: format!("agent/{id}"),
        base: "main".to_owned(),
        model: Some("opencode/claude-sonnet-4-5".to_owned()),
        status: "running".to_owned(),
        created: t - 40.0 * 60.0,
        last_active: t - 35.0,
        session: session(
            "ses_3c7e01",
            "Document the code-agent ports",
            t,
            vec![user_message(
                "msg_1",
                t - 40.0 * 60.0,
                "Document which ports the code-agent manager and its chats listen on.",
            )],
        ),
        asks: vec![],
        diff: vec![],
        // The build that has not answered yet, on the tree that is mid-turn:
        // a draft pushed while the agent keeps working, which is the shape
        // `Checks::Pending` and `PullState::Open { draft }` both exist for and
        // which nothing in the fixtures reached before.
        pulls: vec![Pull {
            draft: true,
            // GitHub answers null while it works mergeability out, and a
            // branch this young is exactly when it does.
            mergeable: None,
            head: format!("agent/{id}"),
            title: "Document the code-agent ports".to_owned(),
            ..pull(126, "open", "pending", None)
        }],
    }
}

/// A tree with something to review and a pull request open on it — the fixture
/// the review screen, the inspector's file list and the pull card all need.
fn reviewed_chat(t: f64) -> Chat {
    let id = "goose-phone-app-7b13de";
    Chat {
        id: id.to_owned(),
        repo: "goose-phone-app".to_owned(),
        title: "Give the sidebar a search box".to_owned(),
        branch: format!("agent/{id}"),
        base: "main".to_owned(),
        model: Some("opencode/claude-sonnet-4-5".to_owned()),
        status: "stopped".to_owned(),
        created: t - 30.0 * 3600.0,
        last_active: t - 2.0 * 3600.0,
        session: session(
            "ses_7b13de",
            "Give the sidebar a search box",
            t,
            vec![
                user_message("msg_1", t - 30.0 * 3600.0, "Give the sidebar a search box."),
                Message {
                    id: "msg_2".to_owned(),
                    role: "assistant".to_owned(),
                    created_ms: ms(t - 30.0 * 3600.0 + 30.0),
                    parts: vec![text_part(
                        "prt_2a",
                        "Done — the box filters the plane's own list and asks the server for a \
                         message search when the text is longer than two characters.",
                    )],
                },
            ],
        ),
        asks: vec![],
        diff: vec![
            FileDiff {
                file: "src/shell/desktop/sidebar.rs".to_owned(),
                patch: PATCH_SIDEBAR.to_owned(),
                additions: 14,
                deletions: 2,
                status: "modified".to_owned(),
            },
            FileDiff {
                file: "assets/desktop/30-sidebar-list.css".to_owned(),
                patch: PATCH_CSS.to_owned(),
                additions: 9,
                deletions: 0,
                status: "modified".to_owned(),
            },
        ],
        pulls: vec![Pull {
            number: 118,
            title: "Give the sidebar a search box".to_owned(),
            state: "open".to_owned(),
            draft: false,
            mergeable: Some(true),
            checks: "passing".to_owned(),
            head: format!("agent/{id}"),
            base: "main".to_owned(),
        }],
    }
}

/// One pull request off a fixture's branch. `head` is filled in by the caller
/// that knows the branch, because the manager only ever answers with pull
/// requests whose head IS this chat's branch and a fixture that disagreed
/// would teach the reader the wrong contract.
fn pull(number: u64, state: &str, checks: &str, mergeable: Option<bool>) -> Pull {
    Pull {
        number,
        title: String::new(),
        state: state.to_owned(),
        draft: false,
        mergeable,
        checks: checks.to_owned(),
        head: String::new(),
        base: "main".to_owned(),
    }
}

fn asleep_chat(t: f64, repo: &str, title: &str, ago: f64, mut pulls: Vec<Pull>) -> Chat {
    let id = format!(
        "{repo}-{:x}",
        (ago as u64).wrapping_mul(2_654_435_761) & 0xff_ffff
    );
    for p in &mut pulls {
        p.head = format!("agent/{id}");
        p.title = title.to_owned();
    }
    Chat {
        id: id.clone(),
        repo: repo.to_owned(),
        title: title.to_owned(),
        branch: format!("agent/{id}"),
        base: "main".to_owned(),
        model: Some("opencode/claude-sonnet-4-5".to_owned()),
        status: "stopped".to_owned(),
        created: t - ago - 600.0,
        last_active: t - ago,
        session: session(
            &format!("ses_{id}"),
            title,
            t,
            vec![user_message("msg_1", t - ago - 600.0, title)],
        ),
        asks: vec![],
        diff: vec![],
        pulls,
    }
}

fn session(id: &str, title: &str, t: f64, messages: Vec<Message>) -> Session {
    Session {
        id: id.to_owned(),
        title: title.to_owned(),
        created_ms: ms(t - 3.0 * 3600.0),
        updated_ms: ms(t),
        messages,
    }
}

fn user_message(id: &str, at: f64, text: &str) -> Message {
    Message {
        id: id.to_owned(),
        role: "user".to_owned(),
        created_ms: ms(at),
        parts: vec![text_part(&format!("prt_{id}"), text)],
    }
}

fn text_part(id: &str, text: &str) -> Part {
    Part {
        id: id.to_owned(),
        kind: "text".to_owned(),
        text: text.to_owned(),
        tool: None,
    }
}

/// `OpenCode` answers diffs as jsdiff `formatPatch` output with the context set
/// to the whole file, so a patch is a four-line `Index:` preamble and ONE `@@`
/// hunk. `src/diff.rs` parses that; a mock that sent several small hunks would
/// be testing a shape the real server never sends.
const PATCH_SIDEBAR: &str = "Index: src/shell/desktop/sidebar.rs\n\
===================================================================\n\
--- src/shell/desktop/sidebar.rs\n\
+++ src/shell/desktop/sidebar.rs\n\
@@ -1,12 +1,24 @@\n\
 //! The sidebar's own list.\n\
 \n\
 use dioxus::prelude::*;\n\
+use crate::state::AppCtx;\n\
 \n\
 pub(crate) fn rows_for(ctx: &AppCtx, plane: Plane, now: i64) -> Vec<Row> {\n\
-    match plane {\n\
-        Plane::Chat => chat_rows(ctx, now),\n\
-        Plane::Code => code_rows(ctx, now),\n\
-    }\n\
+    let rows = match plane {\n\
+        Plane::Chat => chat_rows(ctx, now),\n\
+        Plane::Code => code_rows(ctx, now),\n\
+    };\n\
+    let needle = (ctx.search)().trim().to_lowercase();\n\
+    if needle.is_empty() {\n\
+        return rows;\n\
+    }\n\
+    rows.into_iter()\n\
+        .filter(|row| {\n\
+            row.title.to_lowercase().contains(&needle)\n\
+                || row.subtitle.as_deref().is_some_and(|s| {\n\
+                    s.to_lowercase().contains(&needle)\n\
+                })\n\
+        })\n\
+        .collect()\n\
 }\n";

const PATCH_CSS: &str = "Index: assets/desktop/30-sidebar-list.css\n\
===================================================================\n\
--- assets/desktop/30-sidebar-list.css\n\
+++ assets/desktop/30-sidebar-list.css\n\
@@ -1,6 +1,15 @@\n\
 .nav-search {\n\
   margin: 0 8px 8px;\n\
 }\n\
+\n\
+/* The box says what it filters, because a search that reaches the server and\n\
+ * one that filters the rows in front of you are different promises. */\n\
+.nav-search .field {\n\
+  width: 100%;\n\
+  min-height: 2rem;\n\
+  padding: 0 0.625rem;\n\
+  font-size: var(--text-xs);\n\
+}\n";

/// A tree the app just asked for. The manager derives the title from the task
/// and the branch from the id; the client never sends either.
pub(crate) fn new_chat(repo: &str, task: &str, base: &str, model: Option<&str>) -> Chat {
    let t = now();
    let id = format!(
        "{repo}-{:06x}",
        (t as u64).wrapping_mul(2_654_435_761) & 0xff_ffff
    );
    Chat {
        id: id.clone(),
        repo: repo.to_owned(),
        title: if task.trim().is_empty() {
            "New session".to_owned()
        } else {
            task.chars().take(60).collect()
        },
        branch: format!("agent/{id}"),
        base: base.to_owned(),
        model: model.map(ToOwned::to_owned),
        status: "running".to_owned(),
        created: t,
        last_active: t,
        session: Session {
            id: format!("ses_{id}"),
            title: task.to_owned(),
            created_ms: now_ms(),
            updated_ms: now_ms(),
            messages: Vec::new(),
        },
        asks: Vec::new(),
        diff: Vec::new(),
        pulls: Vec::new(),
    }
}
