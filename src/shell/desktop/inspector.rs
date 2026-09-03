//! The window's third column: what is true about the thing that is open.
//!
//! Its own file rather than more of `mod.rs`, for `sidebar.rs`'s reason —
//! `AppShell` calls `dioxus::desktop::window()` and cannot be mounted in a
//! test, and everything here reads `AppCtx` and nothing else, so it can be.
//!
//! WHAT IS NOT HERE, and this is the sharpest case of that rule in the whole
//! design. The mockups' inspector is where nearly every invented number lives:
//! eight meters, of which seven are money, server memory, container capacity
//! or weekly token aggregates; a server event log this client has no
//! subscription for; round-trip percentiles nothing records; a Checks section
//! naming individual CI jobs where the wire carries one enum; a Commits
//! section where the wire carries none; container slots; a fallback model
//! goose does not send. Of eleven sections, five survive with a real source.
//!
//! The legend at the foot is the same rule applied to keyboard shortcuts, and
//! it is the one a reader can catch you out on fastest: the mockups promise
//! ten chords and this shell binds four. A legend that lists a chord nothing
//! listens for is worse than a wrong number, because the reader presses it.

use dioxus::prelude::*;

use crate::nav::Plane;
use crate::state::{AppCtx, ChatItem, ConnState};

/// One key/value line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fact {
    pub key: String,
    pub value: String,
    /// A value the reader compares or copies — a host, a branch, a model
    /// reference. This sheet's rule, stated on `.nav-row-age`.
    pub mono: bool,
    /// The one value in a block that is its subject.
    pub accent: bool,
}

/// WHERE THE CHAT HALF IS TALKING TO.
///
/// Three rows. The mockup's other two — `round trip p50 12 ms · p99 31 ms` and
/// `tailnet tag:phone → tag:server` — have no source: `AcpClient` records no
/// per-request timing, and nothing in this app reads Tailscale ACL tags. It
/// knows a URL and a secret.
pub(crate) fn chat_facts(ctx: &AppCtx) -> Vec<Fact> {
    let mut out = Vec::new();
    if let Some(host) = super::home::host_of(&ctx.settings.peek().server_url) {
        out.push(Fact {
            key: "host".to_owned(),
            value: host,
            mono: true,
            accent: true,
        });
    }
    if let ConnState::Connected { agent } = (ctx.conn)() {
        out.push(Fact {
            key: "agent".to_owned(),
            value: agent,
            mono: true,
            accent: false,
        });
    }
    // A CONSTANT, and legitimately one: it is a fact about this client rather
    // than a figure from a server. `goose-acp-client` is ACP over a tungstenite
    // WebSocket and cannot be anything else. It stays because it is the row
    // that tells the reader what the host row above it means.
    out.push(Fact {
        key: "transport".to_owned(),
        value: "ACP over WebSocket".to_owned(),
        mono: false,
        accent: false,
    });
    out
}

/// WHAT THE OPEN CONVERSATION IS RUNNING ON — the server's own config options,
/// all of them, rather than the mockup's hardcoded primary/fallback pair.
///
/// goose sends `model`, `mode` and `thinking_effort`; a server that grows a
/// fourth gets a row without this function changing. `fallback` is not one of
/// them and no goose sends it, which is why the mockup's second row is gone.
pub(crate) fn session_facts(ctx: &AppCtx) -> Vec<Fact> {
    (ctx.config_options)()
        .iter()
        .filter_map(|opt| {
            let value = opt.current_label()?.to_owned();
            let key = if opt.name.trim().is_empty() {
                opt.config_id.clone()
            } else {
                opt.name.clone()
            };
            Some(Fact {
                key,
                value,
                mono: false,
                accent: opt.config_id == "model",
            })
        })
        .collect()
}

/// WHAT THE CODE HALF'S WORKING TREE IS. `ChatMeta`, field by field — the
/// section that replaces the mockup's invented `image rust 1.89 · node 22` and
/// `runners busy 2 of 4`.
pub(crate) fn code_facts(ctx: &AppCtx) -> Vec<Fact> {
    let open = (ctx.code_chat)();
    let Some(id) = open.chat_id.clone() else {
        return Vec::new();
    };
    let chats = (ctx.code_chats)();
    let Some(meta) = chats.iter().find(|c| c.id == id) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut push = |key: &str, value: String, mono: bool, accent: bool| {
        if !value.trim().is_empty() {
            out.push(Fact {
                key: key.to_owned(),
                value,
                mono,
                accent,
            });
        }
    };
    push("repo", meta.repo.clone(), true, true);
    push("branch", meta.branch.clone(), true, false);
    push("base", meta.base.clone(), true, false);
    push("container", meta.status.clone(), false, false);

    if let Some(model) = meta.model.clone().filter(|m| !m.trim().is_empty()) {
        // The catalogue is the only place a window size is stated on this
        // plane, and only the WINDOW — nothing here counts tokens used, which
        // is why the code half gets a fact and not the meter below.
        let window = (ctx.code_models)()
            .iter()
            .find(|m| m.reference() == model)
            .and_then(|m| m.limit.context_tokens())
            .map(crate::views::chat::format_tokens);
        out.push(Fact {
            key: "model".to_owned(),
            value: model,
            mono: true,
            accent: true,
        });
        if let Some(window) = window {
            out.push(Fact {
                key: "context window".to_owned(),
                value: window,
                mono: true,
                accent: false,
            });
        }
    }
    if let Some(agent) = opencode_client::resolve_agent(open.agent.as_deref(), &(ctx.code_agents)())
    {
        out.push(Fact {
            key: "agent".to_owned(),
            value: agent.to_owned(),
            mono: false,
            accent: false,
        });
    }
    if meta.last_active > 0.0 {
        out.push(Fact {
            key: "last active".to_owned(),
            value: crate::state::relative_time_secs(meta.last_active),
            mono: true,
            accent: false,
        });
    }
    out
}

/// One extension chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ext {
    pub name: String,
    pub on: bool,
}

pub(crate) fn extension_chips(ctx: &AppCtx) -> Vec<Ext> {
    (ctx.extensions.list)()
        .items
        .iter()
        .map(|entry| Ext {
            name: entry.extension.name().to_owned(),
            on: entry.enabled,
        })
        .collect()
}

/// One meter. `pct` is already clamped to `0..=100`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Meter {
    pub label: &'static str,
    pub value: String,
    pub pct: u128,
    pub warn: bool,
}

/// THE ONE METER THAT HAS A SOURCE.
///
/// The mockups draw eight across their two home screens and their session
/// screen. Seven are money, server memory, container capacity or weekly token
/// aggregates, none of which any server here sends. This is the eighth —
/// `Context 83k / 1M · 8%` — and it is exactly `ctx.usage`, which is
/// `(tokens used, context limit)` and not, as this shell's own comments said
/// twice before they were corrected, tokens in and out.
///
/// A zero limit is "no answer" rather than "no room", which is `crowding`'s
/// own guard and the behaviour `home.rs` already holds a test on.
pub(crate) fn context_meter(ctx: &AppCtx) -> Option<Meter> {
    let usage = (ctx.usage)();
    let (used, limit) = usage?;
    if limit == 0 {
        return None;
    }
    let pct = (u128::from(used) * 100 / u128::from(limit)).min(100);
    Some(Meter {
        label: "Context",
        value: format!(
            "{} / {} \u{b7} {pct}%",
            crate::views::chat::format_tokens(used),
            crate::views::chat::format_tokens(limit)
        ),
        pct,
        warn: crate::views::chat::crowding(usage).is_some(),
    })
}

/// One row of the tool timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    pub name: String,
    pub arg: Option<String>,
    pub state: String,
    pub dot: &'static str,
}

/// WHAT THE AGENT HAS BEEN DOING, out of the transcript this window already
/// holds.
///
/// The mockup's `.tl` CSS is written for exactly this; its content — a server
/// event log — is not, because nothing in this client subscribes to one. So
/// that section is gone and this takes its place.
///
/// Newest last, capped, because the block is pinned above two others in a
/// 344px column and a fifty-tool turn would push the meter off the screen.
pub(crate) fn tool_timeline(ctx: &AppCtx, limit: usize) -> Vec<Step> {
    let chat = (ctx.chat)();
    let mut steps: Vec<Step> = chat
        .items
        .iter()
        .rev()
        .filter_map(|item| match item {
            ChatItem::Tool {
                title,
                kind,
                status,
                ..
            } => Some(Step {
                name: title.clone(),
                arg: (!kind.trim().is_empty()).then(|| kind.clone()),
                state: crate::views::chat::tool_status_label(status),
                dot: match status.as_str() {
                    "pending" | "in_progress" | "running" => "live",
                    "failed" | "error" => "bad",
                    _ => "",
                },
            }),
            _ => None,
        })
        .take(limit)
        .collect();
    steps.reverse();
    steps
}

/// One touched file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Touched {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub seen: bool,
}

/// THE MOCKUP'S `Files touched · 2 of 5 viewed`, and both halves are real.
///
/// `FileDiff`'s `file`/`additions`/`deletions` are the rows; "viewed" is
/// `DiffState.view[path].seen == DiffFile.fingerprint`, which is the same
/// comparison the review screen itself uses to decide whether a file is still
/// marked read after the agent changed it again.
pub(crate) fn touched_files(ctx: &AppCtx) -> (Vec<Touched>, usize) {
    let diff = (ctx.code_diff)();
    let files: Vec<Touched> = diff
        .files
        .iter()
        .map(|file| Touched {
            path: file.info.file.clone(),
            added: file.info.additions,
            removed: file.info.deletions,
            seen: diff.view.get(&file.info.file).and_then(|view| view.seen)
                == Some(file.fingerprint),
        })
        .collect();
    let seen = files.iter().filter(|f| f.seen).count();
    (files, seen)
}

/// One keyboard chord, and it is bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chord {
    pub keys: &'static str,
    pub what: &'static str,
}

/// EVERY CHORD THIS SHELL ACTUALLY BINDS, AND NOTHING ELSE.
///
/// The mockups' legends promise ten — ⌘N, ⌘⇧K, ⌘1/⌘2, ⌘K, ⌘⇧N, ⌘D, ⌘⇧P, J/K,
/// V, ⌘↵ — and this shell listens for four. A legend that lists a chord
/// nothing binds is worse than an invented number, because the reader presses
/// it and nothing happens.
///
/// Not per plane: all four are the SHELL's, and a legend that changed when you
/// crossed the plane switch would suggest the chords did.
///
/// `the_legend_promises_only_chords_the_shell_binds` in `mod.rs` holds this
/// array against the listeners themselves.
pub(crate) const CHORDS: [Chord; 4] = [
    Chord {
        keys: "\u{2318}/",
        what: "Show or hide the sidebar",
    },
    Chord {
        keys: "\u{2318}\u{2325}/",
        what: "Show or hide the inspector",
    },
    Chord {
        keys: "\u{2318}R",
        what: "Refresh what is on screen",
    },
    Chord {
        keys: "esc",
        what: "Close what is open over the page",
    },
];

/// The three tag colours, chosen off the dot class the two existing label
/// functions already return — so the inspector cannot disagree with the list
/// rows about what a passing check looks like.
const fn tag_class(dot: &str) -> &'static str {
    match dot.as_bytes() {
        b"dot on" => "insp-tag ok",
        b"dot busy" => "insp-tag warn",
        b"dot err" => "insp-tag bad",
        _ => "insp-tag",
    }
}

/// The class a fact's value takes, from its two flags.
const fn value_class(mono: bool, accent: bool) -> &'static str {
    match (mono, accent) {
        (true, true) => "insp-kv-value mono accent",
        (true, false) => "insp-kv-value mono",
        (false, true) => "insp-kv-value accent",
        (false, false) => "insp-kv-value",
    }
}

/// A block of key/value rows under a heading.
#[component]
fn Facts(title: String, facts: Vec<Fact>) -> Element {
    rsx! {
        div { class: "insp-sec",
            span { class: "insp-sec-title", "{title}" }
        }
        div { class: "insp-kv",
            for fact in facts.iter() {
                div { key: "{fact.key}", class: "insp-kv-row",
                    span { class: "insp-kv-key", "{fact.key}" }
                    span { class: value_class(fact.mono, fact.accent), "{fact.value}" }
                }
            }
        }
    }
}

/// The column.
///
/// Sections render only when their source has something in it, so a home
/// screen and an open conversation differ by which sections are there rather
/// than by a branch — and a section with nothing behind it is absent instead
/// of being a heading over a blank.
#[component]
pub(crate) fn Inspector(plane: Plane, on_subject: bool) -> Element {
    let ctx = crate::state::use_app_ctx();

    let facts = match plane {
        Plane::Chat => chat_facts(&ctx),
        Plane::Code => Vec::new(),
    };
    let session = if on_subject && plane == Plane::Chat {
        session_facts(&ctx)
    } else {
        Vec::new()
    };
    let tree = if on_subject && plane == Plane::Code {
        code_facts(&ctx)
    } else {
        Vec::new()
    };
    let exts = if plane == Plane::Chat {
        extension_chips(&ctx)
    } else {
        Vec::new()
    };
    let steps = if on_subject && plane == Plane::Chat {
        tool_timeline(&ctx, 8)
    } else {
        Vec::new()
    };
    let pulls = (ctx.code_pulls)();
    let pull = (on_subject && plane == Plane::Code)
        .then(|| pulls.pulls.first().cloned())
        .flatten();
    let pull_count = pulls.pulls.len();
    let (files, seen) = if on_subject && plane == Plane::Code {
        touched_files(&ctx)
    } else {
        (Vec::new(), 0)
    };
    let meter = if plane == Plane::Chat {
        context_meter(&ctx)
    } else {
        None
    };
    let bare = facts.is_empty()
        && session.is_empty()
        && tree.is_empty()
        && exts.is_empty()
        && steps.is_empty()
        && pull.is_none()
        && files.is_empty();

    rsx! {
        aside { class: "insp", "aria-label": "Inspector",

            div { class: "insp-scroll",

                if bare {
                    p { class: "insp-empty",
                        "Nothing to inspect yet. Connect, then open something."
                    }
                }

                if !facts.is_empty() {
                    Facts { title: "goose server", facts: facts.clone() }
                }
                if !tree.is_empty() {
                    Facts { title: "Working tree", facts: tree.clone() }
                }
                if !session.is_empty() {
                    Facts { title: "This conversation", facts: session.clone() }
                }

                if let Some(pull) = pull {
                    div { class: "insp-sec",
                        span { class: "insp-sec-title", "Pull request" }
                        span { class: "insp-sec-meta",
                            {
                                if pull_count == 1 {
                                    "1 on this branch".to_owned()
                                } else {
                                    format!("{pull_count} on this branch")
                                }
                            }
                        }
                    }
                    // A BUTTON, and it holds nothing interactive — no nested
                    // control, which is the rule that cost 1600 audit findings
                    // the one time it was broken.
                    button {
                        class: "insp-card",
                        onclick: move |_| {
                            let mut screen = ctx.code_screen;
                            screen.set(crate::code::CodeScreen::Pulls);
                        },
                        div { class: "insp-card-top",
                            span { class: "insp-card-num", "#{pull.number}" }
                            {
                                let (dot, word) = crate::code::pull_state_label(&pull);
                                rsx! { span { class: tag_class(dot), "{word}" } }
                            }
                            {
                                let (dot, word) = crate::code::checks_label(pull.checks);
                                rsx! { span { class: tag_class(dot), "{word}" } }
                            }
                        }
                        div { class: "insp-card-title", "{pull.title}" }
                        div { class: "insp-card-sub", "{pull.head} \u{2192} {pull.base}" }
                    }
                }

                if !files.is_empty() {
                    div { class: "insp-sec",
                        span { class: "insp-sec-title", "Files touched" }
                        span { class: "insp-sec-meta", "{seen} of {files.len()} viewed" }
                    }
                    div { class: "insp-files",
                        for file in files.iter() {
                            // A DIV. Nothing here opens one file — the review
                            // screen is where a file is read, and a row that
                            // looked pressable and was not would be worse than
                            // one that plainly is not.
                            div { key: "{file.path}", class: "insp-file",
                                span {
                                    class: if file.seen { "insp-file-tick seen" } else { "insp-file-tick" },
                                    "aria-label": if file.seen { "viewed" } else { "not viewed" },
                                }
                                span { class: "insp-file-path", "{file.path}" }
                                span { class: "insp-file-count",
                                    span { class: "add", "+{file.added}" }
                                    span { class: "del", "\u{2212}{file.removed}" }
                                }
                            }
                        }
                    }
                }

                if !exts.is_empty() {
                    div { class: "insp-sec",
                        span { class: "insp-sec-title", "Extensions" }
                        span { class: "insp-sec-meta",
                            "{exts.iter().filter(|e| e.on).count()} on"
                        }
                    }
                    div { class: "insp-chips",
                        for ext in exts.iter() {
                            span {
                                key: "{ext.name}",
                                class: if ext.on { "insp-chip" } else { "insp-chip off" },
                                "{ext.name}"
                            }
                        }
                    }
                }

                if !steps.is_empty() {
                    div { class: "insp-sec",
                        span { class: "insp-sec-title", "Tools" }
                        span { class: "insp-sec-meta", "{steps.len()} this conversation" }
                    }
                    div { class: "insp-timeline",
                        for (n, step) in steps.iter().enumerate() {
                            div { key: "{n}-{step.name}", class: "insp-step",
                                span { class: "insp-step-dot {step.dot}" }
                                span { class: "insp-step-text",
                                    span { class: "insp-step-name", "{step.name}" }
                                    if let Some(arg) = step.arg.as_ref() {
                                        span { class: "insp-step-arg", "{arg}" }
                                    }
                                }
                                span { class: "insp-step-state {step.dot}", "{step.state}" }
                            }
                        }
                    }
                }
            }

            // Rendered only when there is one, so an empty block does not sit
            // between the scroller and the legend. With two children instead
            // of three the grid places them in rows 1 and 2, both `auto` under
            // the `1fr` — identical result, no rule needed.
            if let Some(meter) = meter {
                div { class: "insp-meters",
                    div { class: "insp-meter",
                        div { class: "insp-meter-label",
                            "{meter.label}"
                            span { class: "insp-meter-value", "{meter.value}" }
                        }
                        div { class: "insp-track",
                            i {
                                class: if meter.warn { "warn" } else { "" },
                                style: "width:{meter.pct}%",
                            }
                        }
                    }
                }
            }

            div { class: "insp-keys",
                div { class: "insp-keys-title", "Keys" }
                for chord in CHORDS {
                    div { key: "{chord.keys}", class: "insp-key-row",
                        kbd { class: "insp-key", "{chord.keys}" }
                        span { class: "insp-key-what", "{chord.what}" }
                    }
                }
            }
        }
    }
}
