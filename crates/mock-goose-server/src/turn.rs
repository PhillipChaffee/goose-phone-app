//! The scripted agent turn: what `session/prompt` streams back.
//!
//! Every step is cancellable, because the Stop button is the thing this
//! script exists to exercise. Prompt keywords: "slow" = a long stream (time
//! to hit Stop), "notool" = skip the tool call and its permission prompt,
//! "diff" = add a file edit that carries both halves of its diff, "nokind" =
//! add a tool call with no `kind` field on it at all.
//!
//! A prompt is a `ContentBlock` array, not a string: the text is the first
//! *text* block, and anything else in the array is an attachment, which the
//! answer says back and the replay record keeps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{oneshot, Notify};
use tokio_tungstenite::tungstenite::Message;

use crate::rpc::{error_frame, notify, session_update, Out};
use crate::state::{now_epoch, stamp, Shared};

static SERVER_REQ_ID: AtomicU64 = AtomicU64::new(1);

/// Server->client requests awaiting an answer (permission prompts), keyed by
/// the JSON-encoded request id the client echoes back.
pub(crate) type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Rounds running on this connection right now, keyed by the `session/prompt`
/// id that started them. A turn removes its own entry when it finishes; what
/// is left when the socket dies is what [`abandon`] is handed.
pub(crate) type InFlight = Arc<Mutex<HashMap<String, Abandoned>>>;

/// What a round leaves behind when the client goes away in the middle of it.
///
/// The prompt and the title, and nothing else. That is not a simplification:
/// it is what goose 1.46.0 was measured doing (`docs/permission-durability.md`
/// section 0), and the session coming back named after work with no trace of
/// having happened is the whole failure.
pub(crate) struct Abandoned {
    sid: String,
    user_text: String,
    /// The `user_message_chunk` records for the prompt, in the order the
    /// blocks arrived — an attachment is one of these too.
    prompt: Vec<Value>,
}

/// Write down the only part of an abandoned round that survives.
///
/// Deliberately NOT symmetric with [`finish`]: no round updates, no snippet
/// worth reading, `message_count` up by the one message that exists. A mock
/// that also wrote a `failed` tool call, or a "declined" note, would be
/// reproducing the account section 0 falsified rather than what it measured.
pub(crate) fn abandon(state: &Shared, round: Abandoned) {
    let now = stamp(now_epoch()).rfc3339;
    let mut s = state.lock().unwrap();
    if let Some(data) = s.sessions.get_mut(&round.sid) {
        data.conversation.extend(round.prompt);
        data.message_count += 1;
        data.updated_at = now.clone();
        data.sort_at = now;
        if !data.user_set_name && data.title.is_empty() {
            data.title = auto_title(&round.user_text);
        }
    }
}

/// One scripted turn in flight: where its updates go, how it is cancelled,
/// and the transcript accumulated for replay on `session/load`.
struct Turn {
    out: Out,
    sid: String,
    cancel: Arc<Notify>,
    delay: Duration,
    record: Vec<Value>,
    /// How many blocks of the prompt were not text. Said back in the answer:
    /// against a mock, "did the attachment actually reach the server" is
    /// otherwise a question only a packet capture can settle.
    attachments: usize,
}

/// The client sent `session/cancel` while the turn was streaming.
struct Cancelled;

impl Turn {
    /// Send a `session/update` and keep it for replay on `session/load`.
    fn emit(&mut self, update: Value) {
        session_update(&self.out, &self.sid, &update);
        self.record.push(update);
    }

    /// Pause between updates, ending the turn early if a cancel lands first.
    async fn step(&self) -> Result<(), Cancelled> {
        tokio::select! {
            () = tokio::time::sleep(self.delay) => Ok(()),
            () = self.cancel.notified() => Err(Cancelled),
        }
    }

    /// Thinking stream.
    async fn think(&mut self) -> Result<(), Cancelled> {
        for part in ["Let me think about ", "what you're asking…"] {
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"agent_thought_chunk","messageId":"th_1",
                       "content":{"type":"text","text":part}}),
            );
        }
        Ok(())
    }

    /// Ask the client to approve the scripted tool call and wait for its
    /// answer; `true` once one of the `allow*` options comes back.
    async fn ask_permission(&self, pending: &Pending) -> Result<bool, Cancelled> {
        let (tx, rx) = oneshot::channel();
        let req_id = format!("srv-{}", SERVER_REQ_ID.fetch_add(1, Ordering::Relaxed));
        pending.lock().unwrap().insert(format!("\"{req_id}\""), tx);
        let sid = &self.sid;
        let frame = json!({"jsonrpc":"2.0","id":req_id,"method":"session/request_permission","params":{
        "sessionId": sid,
        "toolCall": {"toolCallId":"tc_1","title":"shell: uname -a","kind":"execute","rawInput":{"command":"uname -a"}},
        "options": [
            {"optionId":"allow_always","name":"allow_always","kind":"allow_always"},
            {"optionId":"allow_once","name":"allow_once","kind":"allow_once"},
            {"optionId":"reject_once","name":"reject_once","kind":"reject_once"},
            {"optionId":"reject_always","name":"reject_always","kind":"reject_always"}
        ]}});
        let _ = self.out.send(Message::Text(frame.to_string().into()));

        let outcome = tokio::select! {
            r = rx => r.unwrap_or(Value::Null),
            () = self.cancel.notified() => return Err(Cancelled),
        };
        Ok(outcome
            .pointer("/outcome/optionId")
            .and_then(Value::as_str)
            .is_some_and(|o| o.starts_with("allow")))
    }

    /// Tool call with a permission round-trip.
    async fn tool_call(&mut self, pending: &Pending) -> Result<(), Cancelled> {
        self.step().await?;
        self.emit(
            json!({"sessionUpdate":"tool_call","toolCallId":"tc_1","title":"shell: uname -a",
                   "kind":"execute","status":"pending","rawInput":{"command":"uname -a"},
                   "_meta":{"goose":{"toolCall":{"toolName":"developer__shell","extensionName":"developer"}}}}),
        );

        if self.ask_permission(pending).await? {
            self.emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"in_progress"}),
            );
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"completed",
                       "content":[{"type":"content","content":{"type":"text","text":"Linux goose-box 6.8.0 x86_64 GNU/Linux"}}]}),
            );
        } else {
            self.emit(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_1","status":"failed",
                       "content":[{"type":"content","content":{"type":"text","text":"Tool call rejected by the user"}}]}),
            );
        }
        Ok(())
    }

    /// A file edit, carrying BOTH halves of its diff.
    ///
    /// The one shape of `ToolCallContent` no fixture in this repo could
    /// produce. The mock's other tool call answers with `{"type":"content"}`,
    /// and `crates/mock-opencode-server` has no diff either, so
    /// `goose-acp-client`'s `diff` arm was decoded only by unit tests over
    /// hand-written JSON — which is how the client came to throw `oldText`
    /// away for the life of the crate without anything noticing (#191). A
    /// client type nothing on the wire can reach is a client type nobody is
    /// testing.
    ///
    /// BOTH HALVES AND THEY DIFFER, which is the whole point: `oldText` alone
    /// proves the field is carried, and a renderer that has to compute
    /// additions against deletions needs two texts that are actually
    /// different. Three lines each, with one line changed and one added, so a
    /// diff card built on this has a `+`, a `−` and a context line to show.
    ///
    /// No permission round-trip. The shell call above already exercises that
    /// path, and this one is here to put a shape on the wire, not to script a
    /// second ask.
    async fn edit_call(&mut self) -> Result<(), Cancelled> {
        self.step().await?;
        self.emit(
            json!({"sessionUpdate":"tool_call","toolCallId":"tc_2",
                   "title":"edit: src/scheduler.rs","kind":"edit","status":"in_progress",
                   "_meta":{"goose":{"toolCall":{"toolName":"developer__text_editor","extensionName":"developer"}}}}),
        );
        self.step().await?;
        self.emit(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_2","status":"completed",
                   "content":[{"type":"diff","path":"src/scheduler.rs",
                               "oldText":"fn tick() {\n    sleep(1);\n}\n",
                               "newText":"fn tick() {\n    sleep(2);\n    log(\"tick\");\n}\n"}]}),
        );
        Ok(())
    }

    /// A tool call with NO `kind` on it, which is the one shape neither fake
    /// could produce.
    ///
    /// `src/views/chat.rs` gives a tool card one leading mark and picks which
    /// by asking whether the kind has a word: the eight it knows get
    /// `.tool-kind`, and everything else — `other`, an extension's own verb,
    /// or the field simply not being there — gets `.tool-icon` with a glyph.
    /// Every tool `mock-goose-server` and `mock-opencode-server` put on the
    /// wire carries `execute`, `edit` or `read`, so the icon arm was
    /// unreachable by driving: `.tool-icon` appears in two captured states and
    /// both are phone keys, while all twenty `desktop-` states render
    /// `.tool-kind`. `assets/desktop/95-transcript.css`'s
    /// `.pane-main .scroll.chat .tool-icon` — the 38px column #211 put the two
    /// marks on — was measured by nothing on the half it was written for
    /// (#248).
    ///
    /// THE FIELD ABSENT AND NOT `"kind":"other"`, which are the same card and
    /// different wires. `ToolCall.kind` is `Option<String>` in
    /// `goose-acp-client` and `src/state.rs` resolves `None` to `"other"`, so
    /// the app cannot tell them apart — but nothing on either wire has ever
    /// made that `None`, so the `unwrap_or_else` was decoded only by unit
    /// tests over hand-written JSON. Same reasoning as the diff above: a
    /// client type nothing on the wire can reach is a client type nobody is
    /// testing.
    ///
    /// A THIRD CALL RATHER THAN THE SECOND ONE'S KIND DROPPED. #248 offers
    /// both; this is the one that leaves a transcript holding BOTH card
    /// shapes, which is the comparison #211 was arguing about and the reason
    /// the capture wants this keyword rather than a quieter edit.
    ///
    /// No permission round-trip, for `edit_call`'s reason: the shell call
    /// scripts that path already and this is here to put a shape on the wire.
    async fn plain_call(&mut self) -> Result<(), Cancelled> {
        self.step().await?;
        self.emit(
            json!({"sessionUpdate":"tool_call","toolCallId":"tc_3",
                   "title":"memory: remember_memory","status":"in_progress",
                   "_meta":{"goose":{"toolCall":{"toolName":"memory__remember_memory","extensionName":"memory"}}}}),
        );
        self.step().await?;
        self.emit(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"tc_3","status":"completed",
                   "content":[{"type":"content","content":{"type":"text",
                               "text":"Remembered: the box is Linux x86_64."}}]}),
        );
        Ok(())
    }

    /// Assistant message stream (markdown showcase).
    async fn answer(&mut self, slow: bool) -> Result<(), Cancelled> {
        if self.attachments > 0 {
            let n = self.attachments;
            let plural = if n == 1 { "" } else { "s" };
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"agent_message_chunk","messageId":"m_1",
                   "content":{"type":"text","text":format!("Got {n} attachment{plural}.\n\n")}}),
            );
        }
        let chunks: Vec<String> = if slow {
            (1..=40)
                .map(|i| format!("chunk {i} of a very long streaming answer… "))
                .collect()
        } else {
            vec![
                "Here's what I found:\n\n".into(),
                "1. Your server is a **Linux x86_64** box\n".into(),
                "2. Everything looks healthy\n\n".into(),
                "```bash\nuname -a  # the command I ran\n```\n\n".into(),
                "| Check | Result |\n|---|---|\n| Kernel | 6.8.0 |\n| Arch | x86_64 |\n\n".into(),
                "Anything ~~broken~~ else you'd like me to look at?".into(),
            ]
        };
        for chunk in chunks {
            self.step().await?;
            self.emit(
                json!({"sessionUpdate":"agent_message_chunk","messageId":"m_1",
                       "content":{"type":"text","text":chunk}}),
            );
        }
        Ok(())
    }

    /// The scripted turn in order: thinking, the optional tool calls, the
    /// answer. Stops at the first step the client cancelled.
    async fn script(&mut self, pending: &Pending, script: &Script) -> Result<(), Cancelled> {
        self.think().await?;
        for step in &script.steps {
            match *step {
                Step::Shell => self.tool_call(pending).await?,
                Step::Edit => self.edit_call().await?,
                Step::Plain => self.plain_call().await?,
            }
        }
        self.answer(script.slow).await
    }
}

/// A card the scripted turn puts on the wire, and the keyword that asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The shell call and its permission round-trip. IN by default and taken
    /// out by "notool", because it is the path most of this fake exists for.
    Shell,
    /// "diff": a file edit carrying both halves of its diff (#191).
    Edit,
    /// "nokind": a tool call with no `kind` field at all (#248).
    Plain,
}

/// What this prompt's keywords asked the turn to do.
///
/// A LIST AND NOT A ROW OF FLAGS, which is clippy's own advice at
/// `struct_excessive_bools` and is the better model anyway: the steps have an
/// ORDER, that order is what a transcript ends up looking like, and four
/// parallel `if`s over four bools said nothing about it. It also puts every
/// keyword in one place, which is a list the module comment, `main.rs`,
/// `README.md` and `CLAUDE.md` all repeat.
struct Script {
    /// The cards, in the order the turn plays them.
    steps: Vec<Step>,
    /// "slow": forty chunks at 400ms, which is time to press Stop.
    slow: bool,
}

impl Script {
    /// OPT-IN FOR EVERYTHING BUT THE SHELL CALL, and that asymmetry is the
    /// point rather than an accident: the scripted turn is what every
    /// app-level test and every captured gallery state is built out of, so a
    /// second card appearing in it unasked would change all of them to make
    /// one wire shape reachable.
    fn from_prompt(user_text: &str) -> Self {
        let text = user_text.to_lowercase();
        let mut steps = Vec::new();
        if !text.contains("notool") {
            steps.push(Step::Shell);
        }
        if text.contains("diff") {
            steps.push(Step::Edit);
        }
        if text.contains("nokind") {
            steps.push(Step::Plain);
        }
        Self {
            steps,
            slow: text.contains("slow"),
        }
    }
}

pub(crate) async fn run_turn(
    request_id: Value,
    params: Value,
    out: Out,
    state: Shared,
    pending: Pending,
    cancel: Arc<Notify>,
    in_flight: InFlight,
) {
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // The first TEXT block, not the first block: an attached image is another
    // entry in the same array, and `prompt/0/text` read as empty the moment
    // one arrived ahead of the message.
    let prompt: Vec<Value> = params
        .get("prompt")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let user_text = prompt
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let attachments = prompt
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) != Some("text"))
        .count();

    if !state.lock().unwrap().sessions.contains_key(&sid) {
        let frame = error_frame(&request_id, -32002, &format!("session not found: {sid}"));
        let _ = out.send(Message::Text(frame.to_string().into()));
        return;
    }

    let script = Script::from_prompt(&user_text);
    let mut turn = Turn {
        out,
        sid,
        cancel,
        delay: Duration::from_millis(if script.slow { 400 } else { 150 }),
        record: Vec::new(),
        attachments,
    };

    // Real goose only replays user chunks on session/load — it does NOT echo
    // them during a live turn. Record for replay without emitting. Every
    // block, not just the text one: replaying an attachment is how the
    // transcript gets it back after a reconnect, and a mock that dropped them
    // would make that path untestable without a real server.
    let prompt_record: Vec<Value> = prompt
        .into_iter()
        .map(|content| json!({"sessionUpdate": "user_message_chunk", "content": content}))
        .collect();
    turn.record.extend(prompt_record.iter().cloned());

    // Registered before the first await, so a socket that dies at any point
    // from here on finds this round rather than missing it by a scheduling
    // hair. `finish` takes it back out; an aborted turn never reaches that.
    let round_key = request_id.to_string();
    in_flight.lock().unwrap().insert(
        round_key.clone(),
        Abandoned {
            sid: turn.sid.clone(),
            user_text: user_text.clone(),
            prompt: prompt_record,
        },
    );

    notify(
        &turn.out,
        "_goose/unstable/session/update",
        &json!({"sessionId": turn.sid, "update": {"sessionUpdate":"usage_update","used":18432,"contextLimit":128_000}}),
    );

    let stop_reason = if turn.script(&pending, &script).await.is_ok() {
        // Auto-title + final usage, then resolve the prompt.
        turn.emit(
            json!({"sessionUpdate":"session_info_update","title":auto_title(&user_text),
                   "updatedAt":"2026-08-21T12:00:00Z"}),
        );
        notify(
            &turn.out,
            "_goose/unstable/session/update",
            &json!({"sessionId": turn.sid, "update": {"sessionUpdate":"usage_update","used":21580,"contextLimit":128_000}}),
        );
        "end_turn"
    } else {
        "cancelled"
    };

    // The round reached its own end, so there is nothing to abandon.
    in_flight.lock().unwrap().remove(&round_key);

    let Turn {
        out, sid, record, ..
    } = turn;
    finish(
        &out,
        &state,
        &sid,
        &request_id,
        record,
        stop_reason,
        &user_text,
    );
}

fn auto_title(user_text: &str) -> String {
    let words: Vec<&str> = user_text.split_whitespace().take(5).collect();
    if words.is_empty() {
        "New chat".to_string()
    } else {
        words.join(" ")
    }
}

fn finish(
    out: &Out,
    state: &Shared,
    sid: &str,
    request_id: &Value,
    record: Vec<Value>,
    stop_reason: &str,
    user_text: &str,
) {
    {
        let now = stamp(now_epoch()).rfc3339;
        let mut s = state.lock().unwrap();
        if let Some(data) = s.sessions.get_mut(sid) {
            data.conversation.extend(record);
            data.message_count += 2;
            data.snippet = format!("Re: {user_text}");
            // A message moves both clocks, which is what puts the session at
            // the top of the next `session/list`.
            data.updated_at = now.clone();
            data.sort_at = now;
            // goose auto-titles only where `user_set_name = FALSE`: a name
            // somebody typed survives the next thing they say.
            if !data.user_set_name && data.title.is_empty() {
                data.title = auto_title(user_text);
            }
        }
    }
    let frame = json!({"jsonrpc":"2.0","id":request_id,"result":{"stopReason":stop_reason}});
    let _ = out.send(Message::Text(frame.to_string().into()));
}
