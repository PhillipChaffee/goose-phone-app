# A permission ask does not survive the phone

What happens when the transport dies while goose is waiting on a permission,
why the answer is not the one this repo has been writing down, and what can
actually be built.

**This document has been wrong twice, and both times it was written down as
fact.** Section 0 is the measurement that settled it; read that before
anything else. Everything after it is cited, and where a claim still rests on
reading rather than running it says so — section 7 is the list of what is still
open.

## 0. What was measured, on a real server

Not a reading. A run. Where this section and any other document in the
repository disagree, this section wins.

**Setup.** goose 1.46.0 over a Tailscale tailnet. A fresh session with
`mode = approve`, prompted: "Run the shell command `uname -a` using your
developer tool." The harness is
`crates/goose-acp-client/examples/perm_loss.rs`, driven by
`scripts/verify/permission-loss-experiment.sh`. The client received the
permission ask and then **never answered**. The process was then killed, two
ways, 75 seconds after the ask. A fresh client reconnected and called
`session/load`.

**During the turn**, from the saved log:

```
update: SessionInfoUpdate(... meta: {"goose": {"activeRunId": "run_882cc68c-..."}})
update: SessionInfoUpdate(title: Some("Run uname command"), updated_at: ..., meta: {"messageCount": ...})
update: ToolCall(ToolCallUpdate { tool_call_id: "call_01a0422902557371bf35096b",
                                  title: Some("shell · uname -a"), status: None, ... })
ASK RECEIVED: Some("shell · uname -a")
PARKED pid=9963 session=20260827_3
```

**After the reconnect**, `session/load` replayed exactly four things:

```
REPLAY UserMessageChunk(... "Run the shell command `uname -a` ...")
REPLAY GooseUpdate usage_update
REPLAY UsageUpdate
REPLAY AvailableCommandsUpdate
```

No `ToolCall`. No assistant message. No "declined". No tool response of any
kind.

**Identical for both deaths.** `kill -STOP` — the process frozen with its
socket still open, which is what iOS does to a backgrounded app — and `kill -9`
— the fd closed and the peer sees a FIN. Two sessions, **`20260827_3`** and
**`20260827_4`**, same result.

### 0.1 The three accounts, and how each one fared

Three competing accounts of this failure had been written into this repository,
all of them from reading source. Two of them were stated as fact.

1. **"goose answers with `Permission::Cancel`, the tool is DECLINED, and the
   transcript says the user declined it."** — **FALSIFIED.** This was stated as
   fact in two places: the `AcpEvent::Disconnected` arm of `pump` in
   `src/state.rs` (now `:671-689`, corrected in place) and §2 of
   `docs/push-notifications.md` (now `:90-113`, corrected in place). Both keep
   the old wording quoted so the mistake stays legible. There is no declined
   tool in the replay. No `Failed` status, no `DECLINED_RESPONSE` text, no tool
   response at all. Nothing.
2. **"The whole ROUND is discarded."** — **CONFIRMED.** This was section 2.3 of
   this document, hedged as inference, and it is what happened.
3. **"Nothing happens; the server never notices, so a quiet client looks like a
   thinking one."** — **FALSIFIED.** Something happened. The round is gone
   within 75 seconds, and the tool call is not sitting there pending.

The second entry is the only one that survives, and it survived because it was
the only one that was *not* asserted as fact.

### 0.2 The cruel part

What survives is the user's prompt **and the generated title**. The session is
called **"Run uname command"** and contains only the request. A user coming
back finds a session named after work that has no trace of having happened.

### 0.3 What this run did NOT establish

Read these as carefully as the result. Getting one thing right does not license
the next confident source read.

- **Which mechanism destroyed the round.** The outcome is measured; the cause
  is not. Section 2.3's abort predicts exactly this replay — but so does "the
  `Err` arm ran and answered `Permission::Cancel`, and the round was never
  persisted anyway, so the answer left no trace." The two are indistinguishable
  from the client. The discriminator is the server's own log line
  `error!("permission request failed")` at
  `/Users/phillipchaffee/git/goose/crates/goose/src/acp/server.rs:1313`, which
  was not captured on this run. See 7.1.
- **Whether the round dies while the socket is still open.** The `STOP` case
  does not isolate this: `scripts/verify/permission-loss-experiment.sh:107`
  sends `kill -CONT` and then `kill -9` after the 75-second wait and before the
  inspect, so the fd is closed either way before `session/load` runs. What the
  `STOP` case proves is that the *outcome* is identical whether or not the peer
  ever saw a FIN — which is enough to falsify account 3, because under account 3
  the tool call would still be in the replay, pending. It is not enough to date
  the death. See 7.4.
- **That side effects of already-run tools remain on disk with no transcript
  record.** The tool never executed here; it was blocked on the permission. The
  claim in section 3 that a round's already-dispatched tools leave their marks
  behind with nothing in the session file to say so is **unverified**, and this
  run does not support it. See 7.6 for the experiment that would.

## 1. What happens, from the user's side

You send a prompt, the agent starts a turn, and one of its tool calls needs
your approval. The modal comes up asking whether `developer__shell` may run
`rm -rf build/`. Before you answer, the phone locks — or you take a call, or
the tailnet roams, or you switch apps for long enough. When you come back, the
app reconnects on its own after a few seconds, the transcript reloads, and the
modal is gone.

What you find is measured, not guessed (section 0): a transcript that **stops
one round early**. No assistant message, no tool card, no explanation, as though
the agent never replied — and a session title describing the work anyway. Your
prompt is there. The answer to it is not, and neither is any record that it was
ever attempted.

The app's only trace of it is a red dot on the connection badge
(`src/views/mod.rs:37`) and a four-second toast reading `Prompt failed:
connection closed` (`src/state.rs:1364`) that has almost certainly expired
before you look at the screen.

Note what you do *not* find, because this repository told you twice that you
would: a tool card marked **Failed** whose collapsed output reads "The user has
declined to run this tool." That card does not exist. See 0.1.

## 2. The mechanism, on both sides of the wire

### 2.1 What this repo believed, and how it was wrong

Two places in the tree stated the model section 0 falsified, both as fact.

The `AcpEvent::Disconnected` arm of `pump` in `src/state.rs` said:

> Transport is gone; the server resolves its own pending permission requests
> via the transport-error path.

and §2 of `docs/push-notifications.md` said it at length, with the source quoted
and an arrow drawn at the line:

> When the phone's transport dies, that request fails, and goose answers its
> own question with `Permission::Cancel`. […] **It silently denies the tool and
> kills the run.**

Both are wrong, and both have been corrected in place, with a note saying so
rather than a silent edit.

Design rule 13 (`docs/design.md`, "A list only reports what its backend can be
asked") said a third thing:

> drop the connection and the server resolves it as a transport error and the
> turn unwinds with it, which is why the app clears that queue on disconnect.

That one reaches the right *conclusion* — a goose session cannot be found
sitting blocked while the app is away, which section 0 confirms — by naming a
cause that was never observed. That is worse than being wrong, because it reads
as corroboration of the sentence above it.

The rest of section 2 is the source reading that *predicted* the measured
result. It is kept because it is still the best account of the mechanism, and
it is still marked as reading, because 0.3 says the mechanism was not measured.

### 2.2 The code the falsified belief points at is real

`/Users/phillipchaffee/git/goose/crates/goose/src/acp/server.rs:1299-1325`
sends the ask and registers a callback whose `Err` arm answers on the client's
behalf:

```rust
cx.send_request(permission_request)
    .on_receiving_result(move |result| async move {
        match result {
            Ok(response) => { /* … */ }
            Err(e) => {
                error!(error = ?e, "permission request failed");
                agent.handle_confirmation(
                    request_id,
                    PermissionConfirmation {
                        principal_type: PrincipalType::Tool,
                        permission: Permission::Cancel,
                    },
                ).await;
                Ok(())
            }
        }
    })?;
```

`Permission::Cancel` is a full denial and not a turn-unwind. The waiter is
`confirmation_rx.await` at
`crates/goose/src/agents/tool_execution.rs:183`; the allow test at `:202` is
`AllowOnce || AlwaysAllow`, so `Cancel` falls to the `else` at `:223` and writes
`CallToolResult::error(DECLINED_RESPONSE)` into `request_to_response_map`
(`:224-229`). `DECLINED_RESPONSE` is defined at
`crates/goose/src/agents/tool_execution.rs:135-137`:

> "The user has declined to run this tool. DO NOT attempt to call this tool
> again. If there are no alternative methods to proceed, clearly explain the
> situation and STOP."

That is the text the model is told you said. `Cancel` and `DenyOnce` are
indistinguishable there — only `AlwaysDeny` differs, by additionally writing a
`NeverAllow` rule at `:232`. And a client that *deliberately* answers
`RequestPermissionOutcome::Cancelled` maps to the same `Permission::Cancel`
(`crates/goose/src/acp/common.rs:42-60`), so the wire cannot tell "the user
pressed cancel" from "the socket died" either.

So *if* that arm runs, the ask is answered as a user denial — and the reading
above concluded, wrongly, that the denial would therefore be **persisted** and
visible. Section 0 measured the transcript and there is no denial in it. Either
the arm did not run (2.3) or it ran and its answer went down with the
un-persisted round (2.4). What the arm cannot do is put a declined tool in front
of the user, because the run looked and there was none.

### 2.3 On the WebSocket this app uses, that arm probably never runs

This is the part that predicted the measured result. It is still a source
reading — 0.3 is explicit that the mechanism was not measured — but it is the
only one of the three accounts that came out of the run intact.

The app connects over a WebSocket
(`crates/goose-acp-client/src/client.rs:273`, tungstenite). goose serves `/acp`
through `AcpHttpServer` from the pinned `agent-client-protocol-http`
(`/Users/phillipchaffee/git/goose/crates/goose/src/acp/transport/mod.rs:7,189`;
git rev `c97a5203`, `Cargo.toml:114-115`). That crate's source is at
`/tmp/acp-rev/rust-sdk-c97a5203d3392f7f231514d84eea014f9f43e6fb/`, and its
teardown is abortive by design and says so in a comment:

`src/agent-client-protocol-http/src/connection.rs:190-200`

```rust
pub(crate) async fn shutdown(&self) {
    // Explicit peer teardown is abortive. Natural agent completion instead
    // awaits the router in `close_connection_task` before closing streams.
    self.close_streams();
    if let Some(h) = self.agent_handle.lock().await.take() { h.abort(); }
    if let Some(h) = self.router_handle.lock().await.take() { h.abort(); }
}
```

The ws message loop breaks on a Close frame, a stream error or EOF
(`websocket_server.rs:127-139`), and `run_ws` then does exactly
`registry.remove(&connection_id)` followed by `conn.shutdown()`
(`websocket_server.rs:73-77`).

`agent_handle` is the single tokio task holding goose's entire ACP connection
future plus the inbound/outbound pump (`connection.rs:513-515, 534-553`). And
that connection future is where *everything* lives: the incoming actor that
dispatches `session/prompt` into goose's `on_prompt`, the outgoing actor, and
`task_actor` — which is what polls the `on_receiving_result` consumer that the
permission callback was handed to (`agent-client-protocol/src/jsonrpc.rs:
1752-1786`; `ConnectionTo::spawn` at `:3173-3180` sends the task to that same
actor rather than to the tokio runtime). Aborting the handle drops all of it
unpolled.

Two consequences, and neither is a race:

- **The agent never sees EOF.** The SDK's documented "fail every pending reply
  with `incoming_transport_closed`" behaviour is triggered by the agent's
  *incoming* stream ending. That stream is fed from `inbound_tx`, which lives in
  the `Connection` struct (`connection.rs:107, 486-500`) that `run_ws` still
  holds an `Arc` to when it calls `shutdown()`. `close_streams()` only flips a
  watch channel that the ws loop itself reads (`connection.rs:202-204`). So
  there is no window in which the pending permission request is failed before
  the abort — the abort is the only thing that happens.
- **The turn dies with it.** goose's `on_prompt` is an ordinary handler future
  (`crates/goose/src/acp/server.rs:1788`), and the agent loop it drains is an
  `async_stream::try_stream!` generator (`crates/goose/src/agents/agent.rs:2286`)
  pulled by `stream.next()` in that handler. Nothing is spawned onto the runtime
  to outlive it. Drop the handler and the generator is dropped mid-`await`.

So on this transport the bug is not "goose answers your ask". It is **"goose
destroys the conversation that contained your ask"** — and *that* is what the
run in section 0 saw: prompt in, title in, round gone. The `Err` arm at
`server.rs:1313` is a real bug on the *stdio* transport, where EOF genuinely
does fail pending replies; this app inherited the fear of it without inheriting
the behaviour.

The **outcome** described here is measured (section 0). The **mechanism** is
still inference from the pinned source: the run cannot separate "aborted before
the arm ran" from "the arm ran and the round was discarded anyway", because both
produce a replay with no tool call in it. Section 7.1 is the one extra
observation — the server's log — that separates them, and it is now a small
follow-up rather than the load-bearing unknown it used to be.

### 2.4 What is lost when the generator is dropped

The user's prompt is safe: it is persisted before the loop starts
(`crates/goose/src/agents/agent.rs:1732-1735`). Everything the *current provider
round* produced is not. `messages_to_add` is declared inside the round loop
(`agent.rs:2454`) and flushed only at the end of the iteration
(`agent.rs:3339-3341`):

```rust
for msg in &messages_to_add {
    session_manager.add_message(&session_config.id, msg).await?;
}
```

The assistant message and its tool requests are in that same batch. Abort
mid-approval and the round is never written at all.

The ask itself was never durable in the first place. In the default loop the
`ActionRequired` / `ToolConfirmation` message is only `yield`ed
(`tool_execution.rs:173-181`, consumed at `agent.rs:2671-2673`) — note that the
*tool-stream* `ActionRequired` twenty lines later **is** persisted
(`agent.rs:2700-2702`), so the omission is specific to approvals. And the only
durability the ask has in memory is one `oneshot::Sender` in a
`Mutex<HashMap<String, _>>` on a per-connection `Agent`
(`crates/goose/src/agents/tool_confirmation_router.rs:8-25`), since
`GooseAgentConnection::connect_to` builds a fresh agent per WebSocket
(`crates/goose/src/acp/server.rs:2332-2345`). There is no store, no queue, and
no timeout: `grep -ri 'permission.*timeout'` over the goose tree finds nothing,
and `confirmation_rx.await` at `tool_execution.rs:183` is not inside a `select!`
on the cancel token (the token is passed only to `dispatch_tool_call` at `:203`).

goose does already have the *second half* of a fix. `resend_pending_tool_permissions`
(`crates/goose/src/acp/server/load_session.rs:206-268`, called unconditionally
from `handle_load_session` at `:302`) walks the active turn for persisted
`ToolConfirmation` entries with no recorded response and re-issues
`session/request_permission` for each. It is a no-op here for both possible
reasons at once: nothing persisted the ask, and — in the world where the `Err`
arm does run — the `Cancel` already produced a tool response, so the id counts
as answered.

### 2.5 The client's half

The socket actor fails every pending request **before** it announces the
disconnect (`crates/goose-acp-client/src/client.rs:330-333`):

```rust
for (_, reply) in pending.drain() {
    let _ = reply.send(Err(AcpError::Closed));
}
let _ = event_tx.send(AcpEvent::Disconnected { reason }).await;
```

That ordering matters more than it looks. The `send_prompt` task is waiting on
one of those replies (`src/state.rs:1329`), and the first thing it does when it
wakes is call `answer_pending_permissions(&ctx, &client, &session_id)`
(`:1339`), which drains every queued ask for that session
(`:1387-1397`) — unconditionally, whether the turn ended cleanly or the socket
died. The pump's `Disconnected` arm then clears whatever is left
(`src/state.rs:673`).

So **the queue is usually already empty by the time the pump reaches line 673**,
and which of the two tasks wins is a scheduler detail, not a guarantee. Any fix
that snapshots the queue at `:673` alone would be a no-op in exactly the
single-session, turn-in-flight case that is the whole bug. Both drain sites have
to be treated, and the one at `:1339` needs a discriminator, because it is also
the ordinary end-of-turn sweep and `stop_turn` calls the same function
(`:1381`) for a cancel the user pressed themselves.

Three other client facts worth having on the table:

- `reconnect_loop` reloads the open chat **only** when the current screen is
  `Screen::Chat` (`src/state.rs:707-711`). Locked on the Chats list, no
  `session/load` is issued at all — which is the only channel through which
  goose could ever re-ask.
- `establish()` closes the previous client first (`src/state.rs:597-600`). On
  the disconnect path that line is unreachable, because the pump already set the
  slot to `None` at `:668`. It **is** reachable from Settings' Connect button
  (`src/views/settings.rs:45`) with a live connection, and it sends a real WS
  Close frame (`client.rs:364-366`) — which, under 2.3, aborts whatever turn is
  running on the server.
- The keepalive cannot be the thing that kills a suspended phone's connection:
  the interval uses `MissedTickBehavior::Delay` and is reset at startup
  (`client.rs:296-298`), so after a suspension it fires once on resume and sets
  `unanswered_pings` to 1, never reaching `MAX_MISSED_PONGS = 2` (`client.rs:280`)
  from suspension alone.

### 2.6 Why the phone is where this bites

iOS gives an app about five seconds after `applicationDidEnterBackground:` and
then suspends it; while suspended the kernel may reclaim the socket's resources
at any time, with no documented bound (Apple TN2277, "Networking and
Multitasking"). No client code runs at that moment — the tokio timers are frozen
— so the *server* is always the party that observes the dead transport. Android
is no longer the safe platform either: the cached apps freezer, default since
Android 14, freezes an app's threads ten seconds after it is cached and then
"terminates any active TCP sockets maintained by the app"
([source.android.com](https://source.android.com/docs/core/perf/cached-apps-freezer)).
macOS is safe against window hiding — tao emits no `Suspended` on that platform
and the socket actor is a tokio task, not the UI thread — but not against system
sleep.

## 3. How bad it really is

**The measured case.** Section 0, minimally: one prompt, one tool call, one ask,
no answer. The round is gone. The prompt and the title remain. That is the floor,
and it is already bad — a session called "Run uname command" that contains only
the request to run it.

**Worst case, and it is a projection, not a measurement.** You approve a plan;
the model opens a round with several tool calls; the auto-approved ones start
running and touch the disk; one hits an approval gate; you lock the phone. The
round is lost the same way — the assistant message, every tool request in it,
and every tool response in it — because none of it is persisted until
`agent.rs:3339`. Earlier rounds and your prompt survive.

What is **not established** is the next sentence this document used to assert:
that the side effects the already-dispatched tools produced on the server's disk
survive with no transcript record of them. It follows from the persistence
reading, and it is the reason to care, but the run in section 0 never executed a
tool — it was blocked on the permission the whole time. Treat it as a
well-supported prediction and nothing more until 7.6 is run. If it holds, the
real severity is not a lost message but a lost *record* of work that happened.

Two other cases this document used to weigh equally, now settled:

- **"You lose the tool call to a decline you never made."** Section 2.2's world.
  **Falsified** (0.1). There is no decline in the transcript to be attributed to
  you, because there is no tool call in the transcript at all.
- **"The network vanishes with no FIN, so the server observes nothing and the
  turn hangs forever."** **Falsified as the user-visible outcome** (0.1,
  account 3). The `STOP` run is exactly this shape — a frozen process holding an
  open socket, which is what a suspended phone is — and 75 seconds later the
  round was gone, not parked. Under this account the replay would have shown the
  tool call still pending; it showed no tool call.

  What survives of it is the *resource* concern, which the replay cannot speak
  to. There is still no server-side keepalive — `websocket_server.rs:131`
  answers inbound pings and never sends one, and goose adds none in
  `acp/transport/mod.rs` — and `confirmation_rx.await` still has no timeout. So
  a turn may still be holding a live `Agent` inside an unreachable connection
  while the app reconnects onto a brand-new `GooseAgentConnection` with an agent
  of its own (`crates/goose/src/acp/server.rs:2332-2345`) writing to the same
  session file. Whether that leak is real is 7.4, and it is now a
  count-the-agents question rather than a what-does-the-user-see one.

**How often.** Every time the phone is locked or backgrounded for more than a
few seconds while an ask is on screen — which is the situation the whole app is
for, since the reason to run goose from a phone is to be away from the desk. It
is not an edge case; it is the primary interaction pattern colliding with the
platform.

**What is not affected.** Nothing on the Code tab: OpenCode exposes a
pending-permissions endpoint and the app polls it (`src/code.rs:389-413`), so an
ask there genuinely does survive the app being away. That asymmetry is already
written up as design rule 13, whose *conclusion* — the Chats list gets nothing —
survives section 0 intact, because a goose ask really cannot be found parked
while the app is away. Its stated *reason* did not survive, and
design rule 13 (`docs/design.md:661-678`) has been corrected to say what was
measured instead of the transport-error story it used to tell.

## 4. What can be fixed client-side today

Nothing here makes the ask survive. Everything here is narration and damage
limitation, and the section says so twice on purpose.

### 4.1 Record the loss at both drain sites, with a discriminator

`src/state.rs`. Add to `AppCtx` (near `permission`, `:321-324`):

```rust
/// Asks that left the queue because the connection died rather than
/// because anyone answered them. Survives `reload_chat`, which clears
/// `chat.items` (:726), so this cannot live in the transcript.
pub lost_permissions: Signal<Vec<LostAsk>>,
```

with `struct LostAsk { session_id: String, tool_call_id: String, title: String,
when: u64 }` — every field is already on `PermissionRequest`
(`crates/goose-acp-client/src/types/mod.rs:313-322`; `tool_call` is a full
`ToolCallUpdate` with `tool_call_id` and `title`, `:167-180`).

Two edits, not one:

- `src/state.rs:673` — `permission.write().clear()` becomes a drain into
  `lost_permissions`.
- `src/state.rs:1339` — `answer_pending_permissions` splits in two. The
  deliberate one keeps today's behaviour and stays on the `stop_turn` path
  (`:1381`) and the clean `Ok("end_turn")` path. A new `abandon_pending_permissions`
  records instead of answering, and is what runs when the prompt future resolved
  `Err` or the client handle is already gone.

The split is the whole point of this item. Snapshotting only at `:673` reports
nothing in the common case (section 2.5); snapshotting inside the existing
shared function reports "denied when the connection dropped" for turns that
ended cleanly and for cancels the user pressed.

**Rejected: deriving the loss from the reloaded transcript.** It cannot work,
for two independent reasons. A real rejection and a transport death produce the
same bytes on replay — both are `Permission::Cancel`
(`crates/goose/src/acp/common.rs:42-60`), both write `DECLINED_RESPONSE`, and
both come back as `ToolCallStatus::Failed`
(`crates/goose/src/acp/server/tool_calls/conversion.rs:284-297`). And
`apply_update` gates tool updates on the session being the one on screen
(`src/state.rs:744-782`), while `reconnect_loop` only loads at all on
`Screen::Chat`. Report from the local snapshot alone, unconditionally.

**Rejected: matching on the declined message text.** goose's string
(`tool_execution.rs:135`) and the mock's ("Tool call rejected by the user",
`crates/mock-goose-server/src/turn.rs:120`) already differ. Correlate on
`tool_call_id` if anything is ever correlated at all — goose builds the
permission request with `ToolCallId::new(request_id)` and replays the response
under `ToolCallId::new(tool_response.id)`, so the ids agree by construction.

### 4.2 Surface it durably, not as a toast

`src/views/chat.rs` and `assets/main.css`. `show_toast` is one slot with
newest-wins semantics and a four-second life (`src/state.rs:559-571`), and the
same code path already raises `Prompt failed: connection closed` at `:1364` and
can raise `Failed to reload session` at `:739`. A report delivered there is a
report that loses a race to itself, and on iOS its timer runs while the app is
suspended.

Shape, per the design guide: a tint and a hairline with a dot, not a slab (rule
7), sitting above the composer on the chat screen and dismissible; the wording
names the tool and the session. **The wording changes now that section 0 is
measured.** "goose may have cancelled it" was hedging against account 1, and
account 1 is dead: nothing was cancelled, the round was thrown away. Say that —
"`shell: rm -rf build/` was waiting on you when the connection dropped. goose
discarded the reply it was working on; your prompt is still there." Where an ask
belonged to a session that is not on screen, the chat row in the Chats list is
the place, in the register rule 8 gives it.

Note what this collides with: rule 13 says the Chats list must show nothing, and
section 0 says its *conclusion* is right. This surface does not contradict it —
rule 13 is about reporting an ask that is still live and answerable, and there
is no such thing on the goose plane. This reports an ask that is **already
lost**, which is a different claim and a different register.

### 4.3 Stop destroying live turns on a deliberate reconnect

`src/state.rs:597-600` and `src/views/settings.rs:45`. Under section 2.3 the
Close frame that `establish()` sends is an abort request for whatever turn is
running. The honest fix is to refuse or confirm rather than to reconnect
silently: if `running_sessions` is non-empty, say so before tearing the socket
down.

**Rejected: dropping the handle without a Close, to "keep the ask alive".** It
was on the table and it does not survive contact. Suppressing the Close leaves
the old socket open server-side until TCP notices, which puts two connections —
and therefore two `Agent`s, per `server_factory.rs:63-108` — on the same session
file. Section 0's `STOP` case is the closest thing to a measurement of that
shape, and it lost the round anyway, so this buys nothing even before the
double-writer problem.

### 4.4 Correct every place that records the wrong model — DONE

Four places asserted the transport-error path, two of them as fact, and all four
have been corrected against section 0 rather than quietly edited: the
`AcpEvent::Disconnected` arm in `src/state.rs:671-689` (**comment only — no
behaviour changed in that commit**), design rule 13 in
`docs/design.md:661-678`, §2 of `docs/push-notifications.md`, and this document.
Each keeps the wrong sentence quoted next to the correction. Two more files
posed the three accounts as an open question and now record the answer:
`crates/goose-acp-client/examples/perm_loss.rs` and
`scripts/verify/permission-loss-experiment.sh`. A fifth,
`docs/desktop-roadmap.md`, did not state the mechanism but did rest on "the
client can go away and the server carries on", and has been given the measured
counter-example.

A comment that encodes a false mechanism is how this understanding propagated in
the first place, and it propagated to four files before anyone ran the thing.

### 4.5 Teach the mock the failure, before writing any test

`crates/mock-goose-server/`. Today the mock **parks the ask forever** on client
disconnect: `Turn::ask_permission` waits on a oneshot whose `Sender` is in the
`Pending` map (`turn.rs:73-79`), and that map is cloned into a detached turn task
(`main.rs:272-284`), so a dead socket leaves the sender alive and the turn simply
waits. It behaves the way we wish goose behaved. A regression test written
against it today passes on a server that never had the bug, which is worse than
no test.

**Section 0 changes what to build here.** The plan used to be two opt-in modes,
`MOCK_DIE_ON_CLOSE=cancel` and `MOCK_DIE_ON_CLOSE=abort`, because we did not
know which world we were in. We know: the `abort` mode is the one that
reproduces the measured server. Build that one, and make it faithful to the
replay in section 0 rather than to the source reading — on client disconnect,
drop the turn task, persist nothing from the round, **and keep the user message
and the session title**, because those are what actually survived.

`cancel` mode is no longer needed to hedge a live uncertainty. Keep it only if
it is wanted as a regression guard for the stdio transport, where the `Err` arm
genuinely does run (2.3), and label it as such — not as "the other possible
world".

The client fix must render the `abort` case correctly, and it can, because it
reports from its own snapshot rather than from anything the server says
afterwards. That property is worth more now than it was: section 0 proves the
server says *nothing* afterwards.

### 4.6 What none of this fixes

The round is still destroyed. The user still cannot answer a question they were
asked. Whether the destroyed round's side effects are left on disk unrecorded is
still unverified (0.3, 7.6). This is a change from a silent wrong outcome to a
stated one — which is worth shipping, and is not a fix.

Two more things explicitly not worth building, with the reasons:

- **Answering deliberately on backgrounding.** tao maps only
  `applicationWillResignActive:` to `Event::Suspended`
  (`tao-0.34.8/src/platform_impl/ios/view.rs:618-620`); `did_enter_background`
  is registered with a literally empty body (`:623`). So the signal also fires
  for a Control Center pull and a notification banner, and denying on it would
  destroy turns the user never left. That objection stands and is on its own
  sufficient.

  **One of the old objections here is falsified and has to be withdrawn.** This
  section used to argue that the payoff is "a transcript byte-identical to
  today's, because `reject_once` and the synthesized `Cancel` both write
  `DECLINED_RESPONSE`". Section 0 measured today's transcript and there is no
  `DECLINED_RESPONSE` in it — there is no tool call in it. So a deliberate
  answer sent before the socket dies would let the round *finish and persist*, a
  strictly better outcome than the measured one, not an identical one. The
  argument against it is now only the false-positive `Suspended` and the fact
  that an explicitly answered ask can never benefit from the upstream fix in
  section 5 (`resend_pending_tool_permissions` skips anything with a recorded
  response). Weigh it on those, and note that the second reason is contingent on
  an upstream change nobody has made.

  (For the record, `unsafe_code = "forbid"` is *not* what blocks this —
  `beginBackgroundTaskWithExpirationHandler`, `endBackgroundTask`,
  `backgroundTimeRemaining` and `sharedApplication` are all safe `pub fn` in
  objc2-ui-kit 0.3.2. The window is reachable.)
- **A second always-connected "notifier" process.** Every connection gets its own
  `GooseAcpAgent` with its own `sessions` map and its own `ToolConfirmationRouter`
  (`crates/goose/src/acp/transport/mod.rs:188-190`,
  `crates/goose/src/acp/server.rs:210-215`, `server_factory.rs:63-108`). A second
  connection cannot see, hold or answer the phone's ask. It only works if the
  notifier owns the prompt, which is a new service plus a second client
  transport — `docs/push-notifications.md:127-145` already rejects it on exactly
  these grounds.

## 5. What needs an upstream change

There are two, in two repositories, and they are not interchangeable. Written as
the pull requests would be.

Both were sized when it was still unknown which world we were in. Section 0
settles the ordering: 5.1 is the one that addresses the measured failure, and
5.2 addresses a conflation that — on this transport — currently has no observable
victim, because there is no persisted denial to be wrongly attributed. 5.2 stops
being theoretical the moment 5.1 lands, which is the argument for landing them in
that order and not the other.

### 5.1 `agent-client-protocol-http`: peer teardown must unwind, not abort

**Problem.** When a WebSocket peer goes away, `run_ws` removes the connection and
calls `Connection::shutdown()`, which aborts the task holding the agent's entire
connection future (`websocket_server.rs:73-77`, `connection.rs:190-200, 534-553`).
Any request the agent has outstanding is dropped unpolled, so the SDK's own
`incoming_transport_closed` machinery never runs; any handler future in flight —
for goose, a whole `session/prompt` turn — is dropped mid-`await`, taking its
un-persisted state with it. The comment at `connection.rs:191` says this is
deliberate, and for a peer that has finished it is fine. For a peer that
vanished mid-request it is data loss.

**Evidence it is the right target.** Section 0: the round the peer's departure
interrupted is not in the session file afterwards, on either kind of departure.
Quote the replay list in the PR; it is four lines and it is the whole argument.

**Minimal change.** On ws loop exit, close the agent's *inbound* path first —
drop or close `inbound_tx` so the agent's incoming stream reaches EOF — and give
the connection future a bounded grace period to unwind before aborting: pending
replies fail with `incoming_transport_closed`, close callbacks run, and any state
the handler was holding gets its chance to flush.

**Must not break.** The grace must be bounded, or a wedged handler holds the
task forever. `close_connection_task` (`connection.rs:574-585`) already has the
shape for the natural-completion case and is the place to converge on. The
existing tests `websocket_drains_final_agent_frame_before_closing` and
`inbound_after_agent_exit_drains_queued_final_frame`
(`websocket_server.rs:474, 553`) both assert the drain-on-exit behaviour and must
stay green.

Note what this PR alone buys: a transcript that is at least consistent, and a
denial that is at least recorded. It does not fix the conflation. That is 5.2.

### 5.2 goose: do not conflate a dead transport with a user saying no

**Problem.** `crates/goose/src/acp/server.rs:1312-1321` answers its own pending
permission request with `Permission::Cancel` on *any* error, and `Cancel` is
indistinguishable from `DenyOnce` downstream (`tool_execution.rs:202, 223-230`):
both write `DECLINED_RESPONSE`, which tells the model the user declined and must
not be retried. A client whose socket died is recorded as a user who said no.
The same conflation appears in the gateway path
(`crates/goose/src/gateway/handler.rs:77-90, 656-674`).

**Minimal change.** The SDK already exposes the discriminator:
`agent_client_protocol::is_incoming_transport_closed(&e)` exists and is called
nowhere in the goose tree. Gate the auto-cancel on it. When the error *is* a
transport close, do not answer — instead persist the approval's
`ActionRequired` / `ToolConfirmation` message into the session, which the default
loop today conspicuously does not do (`tool_execution.rs:173-181` →
`agent.rs:2671-2673`, against the tool-stream case at `agent.rs:2700-2702` which
does). `resend_pending_tool_permissions`
(`crates/goose/src/acp/server/load_session.rs:206-268`) then re-issues the ask on
the next `session/load`, and it is already called on every load (`:302`).

**Must not break — and this is where the "four line patch" framing fails.**

1. **The turn must not hang.** `confirmation_rx.await` (`tool_execution.rs:183`)
   has no timeout and is not in a `select!` on the cancel token, and
   `session/cancel` cannot reach it either — `on_cancel` only cancels the token
   (`acp/server.rs:2039-2058`) and the prompt loop checks the token only between
   stream events (`:1854-1860`). Parking therefore needs a companion: either the
   token joins the await, or the await grows a timeout in the shape
   `ActionRequiredManager::request_and_wait` already uses for elicitations
   (`crates/goose/src/action_required_manager.rs:62-181`). Without one, "leave it
   outstanding" trades a wrong answer for a wedged turn.
2. **A re-asked permission must be answerable.** A reconnect builds a fresh
   `Agent` (`acp/server.rs:2332-2345`) with a fresh `ToolConfirmationRouter`, so
   answering a re-issued ask lands in `handle_confirmation`'s failure path —
   `error!("Failed to deliver confirmation")` (`agent.rs:1583-1588`), via
   `ToolConfirmationRouter::deliver` returning false because nothing is
   registered (`tool_confirmation_router.rs:27-45`). The ask would render, the
   user would tap Allow, and nothing would happen. Making the re-ask real means
   the reload has to resume the tool loop, not just re-send the question. This
   is the largest piece of work in the whole document and it should be sized
   honestly.
3. **A real client Cancel must still deny.** `RequestPermissionOutcome::Cancelled`
   from a live client maps to the same `Permission::Cancel`
   (`acp/common.rs:42-60`) and must keep denying. Only the transport-close branch
   changes. The existing assertions at `acp/server.rs:2619-2625` cover both
   mappings and must stay.
4. **The `AlwaysDeny` path must stay untouched.** A transport death must never
   write a persistent `NeverAllow` rule (`tool_execution.rs:232-236`). It does not
   today, and that is worth keeping.

**Also worth doing in the same PR, and much cheaper:** a server-side keepalive on
the ACP WebSocket. `websocket_server.rs:131` answers inbound pings and never
sends one, and goose adds nothing in `acp/transport/mod.rs`. Whether that leaks a
turn and an agent is now the only live part of section 3's third case, and it is
7.4. Note that it does not leak *visibly*: section 0's `STOP` run had the socket
open and unread for 75 seconds and the round was destroyed regardless, so any
leak is a resource leak on the server, not a parked turn a user could return to.
That leak, if it exists, exists independently of anything to do with permissions.

## 6. Staging

The ordering is chosen so that nothing depends on a question that has not been
answered yet.

**Stage 0 — settle which world we are in. DONE.** Section 0. Run against goose
1.46.0 over the tailnet, sessions `20260827_3` and `20260827_4`; harness kept at
`crates/goose-acp-client/examples/perm_loss.rs` and
`scripts/verify/permission-loss-experiment.sh` so it can be re-run against a new
server build. Result: the round is discarded, and the two accounts that had been
written down as fact are both wrong. What is left open is in 7.1 (the mechanism)
and 7.6 (the side effects), and neither blocks a stage below.

**Stage 1 — the mock learns to fail (4.5).** The `abort` mode, matched to
section 0's replay: user message and title kept, everything else from the round
dropped. Nothing can be tested before this and the CLAUDE.md lockstep rule
requires it anyway.

**Stage 2 — record and report (4.1, 4.2, 4.4).** One commit or two: the signal
and the two drain sites, then the surface. No new dependency, no protocol change,
no upstream anything. This is the whole of what ships this week, and it converts
a silent wrong decision into a stated one.

**Stage 3 — stop making it worse (4.3).** Small, localised, independent of
everything else.

**Stage 4 — upstream PR 5.1.** Contained, mechanical, and it improves matters for
every ACP client on that transport, not just this one. It is also the PR most
likely to be accepted quickly, because the abortive comment reads like a
decision made for the finished-peer case that was never revisited for the
vanished-peer case.

**Stage 5 — upstream PR 5.2, in two parts.** The discriminator and the
persistence first (small, and safe once 5.1 landed). The resumable re-ask second
(large; see must-not-break 2). Only after that does dropping the `Screen::Chat`
guard at `src/state.rs:708` buy anything — today it is a real gap with nothing
on the other side of it.

## 7. What is still open, and how to check

The headline question — which of the three accounts is true — is **closed**, by
section 0, and closing it falsified two of them. What follows is what that run
did not reach. Read 0.3 first; it says in one place what each of these is
missing.

**7.1 — Does the abort win, or does the `Err` arm run and leave no trace?**
*Narrowed, not answered.* The outcome is settled: the round is discarded either
way. The mechanism is not, because the replay is identical under both. This no
longer changes the fix — 5.1 is the right PR under either — but it changes what
the PR says about cause, so capture it before writing one.

One observation settles it, and the session ids and harness from section 0 make
it cheap: run the same experiment with the server's log captured, and look for
`error!("permission request failed")` at
`/Users/phillipchaffee/git/goose/crates/goose/src/acp/server.rs:1313`. Present
means the arm ran and its `Permission::Cancel` was simply thrown away with the
round; absent means the task was aborted before it could. Nothing else in the
transcript distinguishes them.

Still worth watching for while there: a tool request persisted with no response.
That was listed as a possible fourth world and section 0 rules it out for the
blocked-on-permission case, but it may still arise for a round where some tool
calls completed (7.6). If it does, `fix_conversation` silently deletes the orphan
on the next prompt ("Removed orphaned tool request",
`crates/goose-provider-types/src/conversation.rs:500-513`, applied at
`agent.rs:801`).

**7.2 — Does this generalise off the measured build?** *Partly answered.*
Section 0 measured goose **1.46.0** over ws on the tailnet, which is the
deployment that matters. The source reading in 2.3 is against the pinned SDK rev
`c97a5203`; whether 1.46.0 is built from exactly that rev was not checked, and a
server reached over stdio behind a bridge is a different transport with a
different EOF story (2.3). Re-run the harness after a server upgrade rather than
assuming the result travels — that is what it is checked in for.

**7.3 — Which client task wins the drain race?** Section 2.5 claims the
`send_prompt` sweep usually empties the queue before the pump's `Disconnected`
arm. Add a temporary log line at `src/state.rs:673` and `:1339` and kill the mock
mid-permission a dozen times. If `:673` ever sees a non-empty queue, the race is
real and both sites need treating regardless of which usually wins — which is
what 4.1 already does, so this experiment can only strengthen the design, never
change it.

**7.4 — Does a no-FIN disappearance leak an agent, and when does the round
actually die?** Two questions that share one experiment, and section 0 sharpened
both.

The user-visible half is answered: a frozen client with an open socket loses the
round anyway. The `STOP` case does not date the death, because
`scripts/verify/permission-loss-experiment.sh:107` closes the fd with `kill -9`
after the 75-second wait and before the inspect (0.3). To date it, `session/load`
from a *third* client while the frozen one is still frozen and its socket still
open — the harness's `inspect` mode already does exactly this and takes a session
id, so it is a second invocation, not new code. Round already gone at that point
→ the server acts on something other than the FIN. Round still there → it dies at
the close, and the STOP and KILL cases only looked identical because of the
cleanup kill.

The resource half is unchanged: block the tailnet route rather than closing the
socket, wait, reconnect, and count `GooseAcpAgent` instances or watch for two
writers on one session file. If it does not leak, the keepalive ask in 5.2 can be
dropped.

**7.5 — Does WKWebView under dioxus-mobile fire `visibilitychange` on
backgrounding?** Unverified, and nothing in this design depends on it — it is
listed because two rejected mitigations did, and anyone revisiting them needs to
answer it first. The bridge pattern to test with is the one
`use_pull_to_refresh` already uses (`src/viewport.rs:437-450`): a JS listener
registered once, one message per transition, so it costs no per-frame
synchronous XHR.

**7.6 — Do the side effects of tools that already ran survive with no transcript
record?** *Unverified, and it is the claim that sets the severity.* Section 0
cannot speak to it: the tool never executed, because it was blocked on the
permission for the whole run. Section 3 states it as a prediction and labels it
as one. Nobody should build a mitigation whose justification is this sentence
until this experiment has been run.

The design, as a variant of the existing harness rather than a new one:

1. Same setup as section 0 — fresh session, `mode = approve`, real goose over ws.
2. Prompt for a round with **two** tool calls where the first is auto-approved
   and lands a durable, checkable mark on the server's disk, and the second
   needs approval. Something in the shape of "write the current date to
   `/tmp/perm-loss-probe-<nonce>`, then run `uname -a`". The nonce matters: it
   ties the file to this run and nothing else.
3. `perm_loss ask` parks on the second ask exactly as it does today; the log
   already prints the first tool call's `ToolCall` update, so it is visible that
   the write was dispatched.
4. Kill the client both ways, wait, reconnect, `perm_loss inspect`.
5. **The two observations, and they must both be made or the result means
   nothing.** Read the replayed transcript: is the *first* tool call and its
   result in it? Then stat `/tmp/perm-loss-probe-<nonce>` on the server: does the
   file exist, and does its content match what the model was told to write?

The four outcomes are all informative. File present and transcript empty is the
predicted one and confirms section 3's severity. File present and transcript
complete means the round is flushed incrementally after all and only the
in-flight part is lost, which would soften section 3 considerably. File absent
means the first tool never really ran — check step 2's phrasing before concluding
anything, because a model that batched both calls into one request and blocked
before dispatching either is a setup failure, not a result. File absent with the
transcript showing it as completed would be a fifth world and worth its own
write-up.

The obvious trap: the check in step 5 must read the file **on the server's
filesystem**, not through the agent, because asking the agent to look means
starting a new turn on a session whose history is the thing under test.
