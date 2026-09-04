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

/// THE FIVE-BLOCK PROPORTION BAR the mockups draw beside a file's counts, as
/// the class each of the five takes: `add`, `del` or nothing.
///
/// A PROPORTION, NOT A MAGNITUDE, and that is the one decision in it. The
/// mockups' own row draws three green, one red and ONE EMPTY for `+12 −3`,
/// which is a size scale: four of five blocks lit because fifteen lines is
/// "medium". This shell has no such scale and will not invent one — "medium"
/// is a threshold in lines that neither wire states, and a bar drawn against
/// an invented scale is the same fault as a figure nobody sent, one step
/// harder to catch because it prints no digits. So all five blocks are spent
/// on the split itself and the empty rung keeps one real meaning: a file with
/// no line changes at all, which is what a rename or a mode change is.
///
/// Rounded to the nearest block, then pinned at both ends: a file with one
/// deletion among two hundred additions keeps one red block rather than
/// rounding away the only thing that made it interesting.
fn diff_bars(added: u32, removed: u32) -> [&'static str; 5] {
    let total = u64::from(added) + u64::from(removed);
    if total == 0 {
        return [""; 5];
    }
    let scaled = (u64::from(added) * 5 + total / 2) / total;
    let mut green = usize::try_from(scaled).unwrap_or(5).min(5);
    if added > 0 && green == 0 {
        green = 1;
    }
    if removed > 0 && green == 5 {
        green = 4;
    }
    let mut out = ["del"; 5];
    for block in out.iter_mut().take(green) {
        *block = "add";
    }
    out
}

/// THE MOCKUP'S `Files touched · 2 of 5 viewed`, and both halves are real.
///
/// `FileDiff`'s `file`/`additions`/`deletions` are the rows; "viewed" is
/// `DiffState.view[path].seen == DiffFile.fingerprint`, which is the same
/// comparison the review screen itself uses to decide whether a file is still
/// marked read after the agent changed it again.
///
/// WHAT IS NOT ON A ROW: the mockups' `.file.cur` row fill, which marks the
/// file the reader is on. Nothing records that. `DiffState` (src/code.rs) has
/// `seen`, `open`, `expanded` and `show_removed` per path and no cursor, and
/// these rows are `div`s precisely because nothing here opens one — a fill
/// naming a "current" file would be a state this app does not have.
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
                    Facts { title: "goose server", facts }
                }
                if !tree.is_empty() {
                    Facts { title: "Working tree", facts: tree }
                }
                if !session.is_empty() {
                    Facts { title: "This conversation", facts: session }
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
                                // THE GLYPH THE SHEET WAS WRITTEN FOR. This
                                // span had no text child, so `.insp-file-tick`'s
                                // `color`, `font-size` and flex centring styled
                                // nothing and a read file was a filled square.
                                // Drawn in both states: unread, the box's own
                                // `color: transparent` hides it, which is what
                                // keeps one box rather than two shapes.
                                span {
                                    class: if file.seen { "insp-file-tick seen" } else { "insp-file-tick" },
                                    "aria-label": if file.seen { "viewed" } else { "not viewed" },
                                    "\u{2713}"
                                }
                                span { class: "insp-file-path", "{file.path}" }
                                span { class: "insp-file-count",
                                    span { class: "add", "+{file.added}" }
                                    span { class: "del", "\u{2212}{file.removed}" }
                                    // CLASSLESS, like `.insp-chip > i` and
                                    // `.insp-track > i` above it: a decorative
                                    // shape with no name of its own. The two
                                    // that carry an ink reuse `add`/`del`,
                                    // which are the names this cell already
                                    // spends those two inks under.
                                    i {
                                        for (n, block) in diff_bars(file.added, file.removed).into_iter().enumerate() {
                                            i { key: "{n}", class: "{block}" }
                                        }
                                    }
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
                                // THE STATUS DOT, and the chip's asymmetric
                                // 7px/9px padding was built to hold it: with
                                // no dot every label sat 2px right of centre
                                // and on/off was carried by text colour alone.
                                // `.insp-chip > i` and `.insp-chip.off > i`
                                // have styled this since the sheet was written.
                                i {}
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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "test scaffolding: a fixture this file wrote that will not parse \
              is a broken test rather than a runtime condition"
)]
mod tests {
    use super::{
        chat_facts, code_facts, context_meter, diff_bars, extension_chips, session_facts,
        tool_timeline, touched_files, Chord, Inspector, CHORDS,
    };
    use crate::nav::Plane;
    use crate::state::{ChatItem, ConnState};
    use dioxus::prelude::*;

    /// THE LEGEND MAY NOT PROMISE A CHORD NOTHING BINDS.
    ///
    /// This is the panel's honesty check and the one a reader catches you out
    /// on fastest, because they press it. The mockups' legends list ten
    /// shortcuts — ⌘N, ⌘⇧K, ⌘1/⌘2, ⌘K, ⌘⇧N, ⌘D, ⌘⇧P, J/K, V, ⌘↵ — and this
    /// shell listens for four.
    ///
    /// Held BOTH ways, which is what makes it more than a spelling test: every
    /// row of the legend must name a chord some script matches, and the number
    /// of keydown listeners in the shell must equal the number of rows. Add a
    /// row without a listener and the first half fails; bind a chord without
    /// listing it and the second does.
    ///
    /// REPRODUCED: delete `INSP_KEY`'s listener and the count drops to 3 while
    /// `CHORDS` still has 4 — fails. Add a fifth `Chord` — fails. Put both
    /// back and it is green.
    #[test]
    fn the_legend_promises_only_chords_the_shell_binds() {
        let shell = crate::selfscan::code_of("src/shell/desktop/mod.rs", include_str!("mod.rs"));
        let listeners = shell.matches("addEventListener('keydown'").count();
        assert_eq!(
            listeners,
            CHORDS.len(),
            "the shell binds {listeners} keydown listeners and the inspector's \
             legend lists {} chords. A legend row with nothing behind it is \
             worse than a wrong number, because the reader presses it",
            CHORDS.len()
        );

        // And each row names a key some script actually tests for. The scripts
        // match on the CHARACTER, so the last character of the chord is the
        // one to look for — 'esc' is the exception and matches by name.
        for Chord { keys, what } in CHORDS {
            // The scripts match on the CHARACTER, so the last character of
            // the chord is the one to look for. Lowercased, because
            // `REFRESH_KEY` compares `e.key.toLowerCase()` — a chord's glyph
            // is upper case and the key it is is not. Escape is the one that
            // matches by name rather than by character.
            let needle = match keys {
                "esc" => "Escape".to_owned(),
                other => format!(
                    "'{}'",
                    other
                        .chars()
                        .next_back()
                        .unwrap_or('?')
                        .to_lowercase()
                        .collect::<String>()
                ),
            };
            assert!(
                shell.contains(&needle),
                "the legend offers `{keys}` for \"{what}\" and no script in the \
                 shell tests for {needle}"
            );
        }
    }

    /// THE ONE METER WITH A SOURCE, and a zero limit is not one.
    ///
    /// `Usage` is `(tokens used, context limit)` — the fact this shell's own
    /// comments got wrong twice — so the mockups' `Context 83k / 1M · 8%` is
    /// honest. A `contextLimit` of 0 is a server that did not answer, not a
    /// window with no room in it, and `views::chat::crowding` guards the same
    /// way for the same reason.
    ///
    /// REPRODUCED: drop the `limit == 0` return and the second half fails.
    #[test]
    fn the_context_meter_reads_the_window_and_refuses_a_zero() {
        let meter = crate::testkit::with_ctx(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((83_000, 1_000_000)));
            },
            context_meter,
        );
        assert!(
            meter.is_some(),
            "a usage update arrived, so there is a meter"
        );
        let meter = meter.unwrap_or_else(|| super::Meter {
            label: "Context",
            value: String::new(),
            pct: 0,
            warn: false,
        });
        assert_eq!(meter.pct, 8, "83k of 1M is 8%");
        assert!(meter.value.contains("83k"), "{}", meter.value);
        assert!(meter.value.contains("1.0M"), "{}", meter.value);
        assert!(!meter.warn, "8% is not crowding");

        let none = crate::testkit::with_ctx(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((0, 0)));
            },
            context_meter,
        );
        assert!(
            none.is_none(),
            "a zero limit was rendered as a window with no room in it"
        );
    }

    /// Past the point `crowding` speaks up, the meter says so.
    #[test]
    fn a_crowded_window_is_marked_as_one() {
        let meter = crate::testkit::with_ctx(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((900_000, 1_000_000)));
            },
            context_meter,
        );
        let meter = meter.expect("a usage update arrived");
        assert_eq!(meter.pct, 90);
        assert!(
            meter.warn,
            "90% of the window is past the point `views::chat::crowding` \
             already treats as worth telling the reader about, and the two \
             must not disagree"
        );
    }

    /// The server block says where it is talking to, and nothing it cannot
    /// know. The mockup's other two rows — a round-trip percentile and a
    /// tailnet ACL pair — have no source at all.
    #[test]
    fn the_server_block_says_only_what_this_client_knows() {
        let facts = crate::testkit::with_ctx(
            |ctx| {
                let mut settings = ctx.settings;
                settings.write().server_url = "https://tail-mini.ts.net:3285/acp".to_owned();
                let mut conn = ctx.conn;
                conn.set(ConnState::Connected {
                    agent: "goose 1.47.0".to_owned(),
                });
            },
            chat_facts,
        );
        let by = |k: &str| {
            facts
                .iter()
                .find(|f| f.key == k)
                .map(|f| f.value.clone())
                .unwrap_or_default()
        };
        assert_eq!(by("host"), "tail-mini.ts.net:3285", "scheme and path kept");
        assert_eq!(by("agent"), "goose 1.47.0");
        assert!(!by("transport").is_empty());
        for fact in &facts {
            assert!(
                !fact.value.contains("ms") && !fact.value.contains('$'),
                "a row is quoting a latency or a price, neither of which any \
                 server here reports: {} = {}",
                fact.key,
                fact.value
            );
        }
    }

    /// A disconnected client names no agent — it has not been told one.
    #[test]
    fn an_unopened_socket_names_no_agent() {
        let facts = crate::testkit::with_ctx(|_| {}, chat_facts);
        assert!(
            facts.iter().all(|f| f.key != "agent"),
            "an agent was named over a socket nobody has opened"
        );
    }

    /// The session block is the server's OWN option list, all of it — not the
    /// mockup's hardcoded primary/fallback pair. goose sends no `fallback` and
    /// a server that grows a fourth option gets a row for free.
    #[test]
    fn the_session_block_is_whatever_the_server_offers() {
        let facts = crate::testkit::with_ctx(
            |ctx| {
                let mut opts = ctx.config_options;
                opts.set(vec![
                    option("model", "Model", "opus", "Claude Opus 5"),
                    option("mode", "Mode", "auto", "Auto"),
                ]);
            },
            session_facts,
        );
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].key, "Model");
        assert_eq!(facts[0].value, "Claude Opus 5");
        assert!(facts[0].accent, "the model is the block's subject");
        assert!(!facts[1].accent, "and it is the only one");
    }

    /// An option the server sent no current value for is not a row: a key with
    /// nothing after it is a slot pretending to be a fact.
    #[test]
    fn an_option_with_no_answer_is_not_a_row() {
        let facts = crate::testkit::with_ctx(
            |ctx| {
                let mut opts = ctx.config_options;
                opts.set(vec![serde_json::from_value(serde_json::json!({
                    "id": "model", "name": "Model", "options": [],
                }))
                .expect("a config option this test wrote")]);
            },
            session_facts,
        );
        assert!(facts.is_empty(), "{facts:?}");
    }

    /// With no code chat open the working-tree block is absent rather than a
    /// heading over nothing.
    #[test]
    fn no_open_tree_means_no_tree_block() {
        let facts = crate::testkit::with_ctx(|_| {}, code_facts);
        assert!(facts.is_empty());
    }

    /// "2 of 5 viewed" is real on both halves: the count is the files, and
    /// "viewed" is the fingerprint comparison the review screen itself uses to
    /// decide whether a file is still read after the agent changed it again.
    ///
    /// REPRODUCED: compare `seen` against anything but `fingerprint` — against
    /// `Some(_)`, say — and the stale row counts as viewed and this fails.
    #[test]
    fn a_file_is_viewed_only_while_its_fingerprint_still_matches() {
        let (files, seen) = crate::testkit::with_ctx(
            |ctx| {
                let mut diff = ctx.code_diff;
                // a.rs was read at the fingerprint it still has; b.rs was read
                // and then changed underneath the reader.
                let view = [("src/a.rs", 7_u64), ("src/b.rs", 4)]
                    .into_iter()
                    .map(|(path, seen)| {
                        (
                            path.to_owned(),
                            crate::code::FileView {
                                seen: Some(seen),
                                ..crate::code::FileView::default()
                            },
                        )
                    })
                    .collect();
                diff.set(crate::code::DiffState {
                    files: vec![
                        diff_file("src/a.rs", 12, 3, 7),
                        diff_file("src/b.rs", 1, 0, 9),
                    ],
                    view,
                    ..crate::code::DiffState::default()
                });
            },
            touched_files,
        );
        assert_eq!(files.len(), 2);
        assert_eq!(
            seen, 1,
            "the file that changed after it was read still counted"
        );
        assert_eq!(files[0].added, 12);
        assert_eq!(files[0].removed, 3);
    }

    /// THE PROPORTION BAR IS A SPLIT AND NEVER A SCALE.
    ///
    /// Five blocks, all of them spent on the additions/deletions ratio, so the
    /// bar states nothing about how big the change is — "big" is a threshold in
    /// lines that neither wire sends, and this shell does not invent one. The
    /// empty rung therefore means exactly one thing, and the last case is it.
    ///
    /// REPRODUCED: make the bar a magnitude — light `min(5, total / 4)` blocks
    /// — and the first and last rows collide, because +1/−0 and +200/−0 stop
    /// being the same picture.
    #[test]
    fn the_proportion_bar_is_a_split_and_never_a_scale() {
        for (added, removed, want) in [
            (1_u32, 0_u32, ["add", "add", "add", "add", "add"]),
            (200, 0, ["add", "add", "add", "add", "add"]),
            (12, 3, ["add", "add", "add", "add", "del"]),
            (0, 7, ["del", "del", "del", "del", "del"]),
            (1, 1, ["add", "add", "add", "del", "del"]),
        ] {
            assert_eq!(
                diff_bars(added, removed),
                want,
                "+{added} \u{2212}{removed}"
            );
        }
    }

    /// Both ends are pinned, so the one thing that made a file interesting is
    /// not rounded away: a single deletion among two hundred additions keeps a
    /// red block, and a single addition among two hundred deletions a green.
    #[test]
    fn a_lone_count_is_never_rounded_off_the_bar() {
        let bars = diff_bars(200, 1);
        assert_eq!(bars[4], "del", "{bars:?}");
        assert_eq!(bars[0], "add", "{bars:?}");
        let bars = diff_bars(1, 200);
        assert_eq!(bars[0], "add", "{bars:?}");
        assert_eq!(bars[4], "del", "{bars:?}");
    }

    /// A FILE WITH NO LINE CHANGES LIGHTS NOTHING, which is the empty rung's
    /// one real meaning — a rename, or a mode change. Without this the ratio
    /// divides by zero.
    #[test]
    fn a_file_with_no_line_changes_lights_no_block() {
        assert_eq!(diff_bars(0, 0), ["", "", "", "", ""]);
    }

    /// THE THREE SHAPES THE SHEET WAS WRITTEN FOR, RENDERED.
    ///
    /// `.insp-chip > i`, `.insp-file-tick`'s glyph rules and the bar blocks
    /// were all styled before anything drew them, which no gate in this repo
    /// can see: `docs/audit.js` measures the elements it is given, and an
    /// element that is never emitted is never given to it. This asks the
    /// markup instead.
    ///
    /// REPRODUCED: drop the `i {}` from the chip, the `"\u{2713}"` from the
    /// tick or the bar loop from the count cell, and one of the three
    /// assertions names it.
    #[test]
    fn the_column_draws_the_shapes_its_own_sheet_styles() {
        let html = crate::testkit::render_settled(
            |ctx| {
                let mut list = ctx.extensions.list;
                list.write().items = vec![extension("developer", true)];
            },
            || rsx! { Inspector { plane: Plane::Chat, on_subject: true } },
        );
        assert!(
            html.contains("<span class=\"insp-chip\"><i></i>developer</span>"),
            "the extension chip has no status dot, so its 7px/9px padding \
             leaves the label off-centre and on/off is text colour alone: {html}"
        );

        let html = crate::testkit::render_settled(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(crate::code::DiffState {
                    files: vec![diff_file("src/a.rs", 12, 3, 7)],
                    ..crate::code::DiffState::default()
                });
            },
            || rsx! { Inspector { plane: Plane::Code, on_subject: true } },
        );
        assert!(
            html.contains('\u{2713}'),
            "the viewed tick is a filled box with no check in it, and the \
             `color`/`font-size`/centring rules written for the glyph do \
             nothing: {html}"
        );
        assert_eq!(
            html.matches("<i class=").count(),
            5,
            "the file row's proportion bar is not five blocks: {html}"
        );
    }

    /// The timeline is capped and reads oldest-first, so the newest call is at
    /// the bottom where the eye already is.
    #[test]
    fn the_timeline_is_capped_and_ends_with_the_newest() {
        let steps = crate::testkit::with_ctx(
            |ctx| {
                let mut chat = ctx.chat;
                chat.write().items = (0..12)
                    .map(|n| ChatItem::Tool {
                        id: format!("t{n}"),
                        title: format!("tool {n}"),
                        kind: "execute".to_owned(),
                        status: "completed".to_owned(),
                        output: String::new(),
                    })
                    .collect();
            },
            |ctx| tool_timeline(ctx, 4),
        );
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["tool 8", "tool 9", "tool 10", "tool 11"]);
    }

    /// A running call is marked live, so the block says what is happening now
    /// rather than only what happened.
    #[test]
    fn a_call_still_running_is_marked_as_one() {
        let steps = crate::testkit::with_ctx(
            |ctx| {
                let mut chat = ctx.chat;
                chat.write().items = vec![ChatItem::Tool {
                    id: "t".to_owned(),
                    title: "cargo test".to_owned(),
                    kind: "execute".to_owned(),
                    status: "in_progress".to_owned(),
                    output: String::new(),
                }];
            },
            |ctx| tool_timeline(ctx, 8),
        );
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].dot, "live");
    }

    /// Extensions are chips with a state, because "loaded" and "on" are not
    /// the same thing and the composer's own count says so too.
    #[test]
    fn the_extension_chips_carry_whether_each_is_on() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut list = ctx.extensions.list;
                list.write().items =
                    vec![extension("developer", true), extension("todoist", false)];
            },
            extension_chips,
        );
        assert_eq!(chips.len(), 2);
        assert!(chips[0].on);
        assert!(!chips[1].on);
    }

    /// THE COLUMN INVENTS NOTHING, rendered.
    ///
    /// The unit tests above hold each source; this holds the thing a reader
    /// sees. Every category on the never-fake register at once, against a
    /// context with a connection and a session — the state in which the panel
    /// has the most to say and so the most room to say something it cannot
    /// know.
    #[test]
    fn the_inspector_paints_no_figure_it_has_no_source_for() {
        let html = crate::testkit::render_settled(
            |ctx| {
                let mut conn = ctx.conn;
                conn.set(ConnState::Connected {
                    agent: "goose 1.47.0".to_owned(),
                });
                let mut usage = ctx.usage;
                usage.set(Some((83_000, 1_000_000)));
            },
            || rsx! { Inspector { plane: Plane::Chat, on_subject: true } },
        );
        assert!(html.contains("insp-scroll"), "the column did not render");
        for forbidden in ["$", " ms", "p50", "p99", "warm", "uptime", "queue"] {
            assert!(
                !html.contains(forbidden),
                "the inspector printed {forbidden:?}, which no server on either \
                 wire reports — the mockups' panel is where nearly every \
                 invented number in the design lives, and this is the check \
                 that keeps them out"
            );
        }
    }

    /// With nothing connected and nothing open the column says so, rather than
    /// rendering a stack of empty headings.
    #[test]
    fn an_empty_inspector_says_it_is_empty() {
        let html =
            crate::testkit::render(|| rsx! { Inspector { plane: Plane::Code, on_subject: false } });
        assert!(html.contains("insp-empty"), "{html}");
    }

    /// Built through serde for `home::tests::recipe`'s reason: these DTOs gain
    /// fields as the protocol does.
    fn option(id: &str, name: &str, value: &str, label: &str) -> goose_acp_client::ConfigOption {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "currentValue": value,
            "options": [{ "value": value, "name": label }],
        }))
        .expect("a config option this test wrote")
    }

    /// `home::tests::extension`'s shape, with the flag as a parameter.
    fn extension(name: &str, enabled: bool) -> goose_acp_client::GooseExtensionEntry {
        goose_acp_client::GooseExtensionEntry {
            extension: goose_acp_client::GooseExtension::mcp(
                goose_acp_client::McpServer::Stdio(goose_acp_client::StdioMcpServer::new(
                    name,
                    "uvx",
                    Vec::new(),
                )),
                Vec::new(),
                "test",
                Vec::new(),
            ),
            enabled,
            config_key: Some(name.to_owned()),
            extra: serde_json::Map::new(),
        }
    }

    fn diff_file(path: &str, added: u32, removed: u32, fingerprint: u64) -> crate::code::DiffFile {
        crate::code::DiffFile {
            info: opencode_client::FileDiff {
                file: path.to_owned(),
                additions: added,
                deletions: removed,
                ..opencode_client::FileDiff::default()
            },
            fingerprint,
            lines: Vec::new(),
            gaps: Vec::new(),
        }
    }
}
