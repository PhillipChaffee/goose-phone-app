//! What the mock does when a prompt arrives.
//!
//! `mock-goose-server/src/turn.rs`'s job on the other wire, and its reason: a
//! fake that only LISTS is worth much less than one that can stream a reply,
//! because half the app's behaviour — the growing transcript, the tool that
//! finishes, the permission that blocks, the Stop button — only exists while a
//! turn is in flight.
//!
//! Prompt keywords, the same idea as the goose mock's:
//!
//!   "slow"   — a long stream, so there is time to press Stop
//!   "ask"    — park a permission and wait for the answer
//!   "fail"   — a tool that ends in error
//!   "notool" — text only

use crate::state::{Ask, Part, Step};

/// One beat, in milliseconds. Slow enough to watch, fast enough that a test
/// driving this does not spend its budget waiting.
const BEAT: u64 = 260;

pub(crate) fn script(prompt: &str) -> Vec<Step> {
    let p = prompt.to_lowercase();
    let msg = "msg_turn".to_owned();
    let mut steps = vec![Step::Message {
        id: msg.clone(),
        role: "assistant".to_owned(),
    }];

    // A reasoning part first, because the transcript folds those into a
    // collapsed "Thought for …" and nothing else in the fixtures exercises it.
    steps.push(Step::Beat(BEAT));
    steps.push(Step::Part {
        message: msg.clone(),
        part: text(
            "prt_think",
            "reasoning",
            "Reading the tree before I touch anything.",
        ),
    });

    if !p.contains("notool") {
        steps.push(Step::Beat(BEAT));
        steps.push(Step::Part {
            message: msg.clone(),
            part: crate::routes::tool_part(
                "prt_tool",
                "read",
                "src/shell/desktop/home.rs",
                "running",
                "",
            ),
        });
        steps.push(Step::Beat(BEAT * 2));
        steps.push(Step::Part {
            message: msg.clone(),
            part: crate::routes::tool_part(
                "prt_tool",
                "read",
                "src/shell/desktop/home.rs",
                if p.contains("fail") {
                    "error"
                } else {
                    "completed"
                },
                if p.contains("fail") {
                    "No such file or directory"
                } else {
                    "1793 lines"
                },
            ),
        });
    }

    // The reply, a sentence at a time, so the transcript visibly grows.
    let sentences: &[&str] = if p.contains("slow") {
        &[
            "Working through it.",
            " The board groups working trees by repo, so the change belongs in `code_board`.",
            " Each group needs its own base branch only when every tree in it shares one.",
            " Two bases in one repo is a fact about the trees rather than the repo.",
            " I have made that the rule and left the heading empty otherwise.",
            " Running the suite now to see what else reads that function.",
        ]
    } else {
        &[
            "Done.",
            " The rows group by repo and the heading names a base only when every tree in the group was cut from the same one.",
        ]
    };
    let mut said = String::new();
    for s in sentences {
        said.push_str(s);
        steps.push(Step::Beat(BEAT));
        steps.push(Step::Part {
            message: msg.clone(),
            part: text("prt_say", "text", &said),
        });
    }

    if p.contains("ask") {
        steps.push(Step::Beat(BEAT));
        steps.push(Step::Ask(Ask {
            id: "per_live".to_owned(),
            kind: "bash".to_owned(),
            title: "Run the test suite".to_owned(),
            command: "cargo test --workspace".to_owned(),
        }));
        // Nothing after the ask: answering it is what pushes the rest, in
        // `routes::chat_server`. A turn parked on a question waits, which is
        // what the real one does.
        return steps;
    }

    steps.push(Step::Idle);
    steps
}

fn text(id: &str, kind: &str, body: &str) -> Part {
    Part {
        id: id.to_owned(),
        kind: kind.to_owned(),
        text: body.to_owned(),
        tool: None,
    }
}
