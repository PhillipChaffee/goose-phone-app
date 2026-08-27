# A permission ask does not survive the phone

What happens when the transport dies while goose is waiting on a permission,
why the answer is not the one this repo has been writing down, and what can
actually be built.

Everything below is cited. Where a claim rests on reading rather than running,
it says so — section 7 is the list of experiments that would settle each one,
and two of them would change the fix.

## 1. What happens, from the user's side

You send a prompt, the agent starts a turn, and one of its tool calls needs
your approval. The modal comes up asking whether `developer__shell` may run
`rm -rf build/`. Before you answer, the phone locks — or you take a call, or
the tailnet roams, or you switch apps for long enough. The socket dies with the
process. When you come back, the app reconnects on its own after a few seconds,
the transcript reloads, and the modal is gone. Nothing says it was ever there.
What you find in its place is either a tool card marked **Failed** whose
collapsed output reads "The user has declined to run this tool", or — more
likely, see section 3 — a transcript that simply stops one round early, with no
assistant message, no tool call and no explanation, as though the agent never
replied. Either way a decision you were being asked to make was made without
you, and the app's only trace of it is a red dot on the connection badge
(`src/views/mod.rs:37`) and a four-second toast reading `Prompt failed:
connection closed` (`src/state.rs:1364`) that has almost certainly expired
before you look at the screen.

## 2. The mechanism, on both sides of the wire

### 2.1 What this repo currently believes

Two places in the tree state the model this document is replacing.

`src/state.rs:671` says:

> Transport is gone; the server resolves its own pending permission requests
> via the transport-error path.

and `docs/design.md:658` says:

> drop the connection and the server resolves it as a transport error and the
> turn unwinds with it, which is why the app clears that queue on disconnect.

The first is the belief this design refutes. The second happens to describe the
likely outcome correctly while naming the wrong cause, which is worse than being
wrong, because it reads as corroboration.

### 2.2 The code the belief points at is real

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

So *if* that arm runs, the ask is answered as a user denial, persisted, and the
model is instructed not to retry.

### 2.3 On the WebSocket this app uses, that arm almost certainly never runs

This is the part that changes the fix.

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
destroys the conversation that contained your ask"**. The `Err` arm at
`server.rs:1313` is a real bug on the *stdio* transport, where EOF genuinely
does fail pending replies; this app inherited the fear of it without inheriting
the behaviour.

I have not run this. It is inference from the pinned source, and section 7.1 is
the experiment that settles it.

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

**Worst realistic case.** You approve a plan; the model opens a round with
several tool calls; the auto-approved ones start running and touch the disk; one
hits an approval gate; you lock the phone. Under section 2.3 you lose the whole
provider round — the assistant message, every tool request in it, and every tool
response in it — because none of it is persisted until `agent.rs:3339`. Earlier
rounds and your prompt survive. What does **not** get rolled back are the side
effects the already-dispatched tools produced on the server's disk. So the
session file ends at the previous round while the working tree has moved, and
your next prompt is sent against a history that does not know those files were
written. That is the real severity: not a lost message, a lost *record* of work
that happened.

Under section 2.2's world instead — if the abort does not win — you lose the tool
call to a decline you never made, attributed to you in the transcript, with the
model instructed not to retry.

There is a third case, and on a phone it may be the most common: the network
simply vanishes with no FIN (tailnet roam, cell handoff). Then the server
observes nothing. The ws loop is not reading anything, there is no server-side
keepalive at all — `websocket_server.rs:131` answers inbound pings and never
sends one, and goose adds none in `acp/transport/mod.rs` — and
`confirmation_rx.await` has no timeout. The turn hangs indefinitely inside an
unreachable connection, holding a live `Agent`, while the app reconnects onto a
brand-new `GooseAgentConnection` with an agent of its own
(`crates/goose/src/acp/server.rs:2332-2345`) writing to the same session file.
One leaked agent and one leaked turn per lock-and-reconnect cycle.

**How often.** Every time the phone is locked or backgrounded for more than a
few seconds while an ask is on screen — which is the situation the whole app is
for, since the reason to run goose from a phone is to be away from the desk. It
is not an edge case; it is the primary interaction pattern colliding with the
platform.

**What is not affected.** Nothing on the Code tab: OpenCode exposes a
pending-permissions endpoint and the app polls it (`src/code.rs:389-413`), so an
ask there genuinely does survive the app being away. That asymmetry is already
written up as design rule 13, and the half of rule 13 that explains why the
goose plane needs nothing is the half that is wrong.

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
names the tool and the session and says what is not known — "`shell: rm -rf
build/` was waiting on you when the connection dropped. goose may have cancelled
it." Where an ask belonged to a session that is not on screen, the chat row in
the Chats list is the place, in the register rule 8 gives it.

Note what this collides with: rule 13 currently says the Chats list must show
nothing. That rule's justification is section 2.2's model, and it has to be
rewritten in the same change.

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
file. That is section 3's third case, deliberately induced.

### 4.4 Correct the two places that record the wrong model

`src/state.rs:671-672` and `docs/design.md:658`. Both currently assert the
transport-error path as fact. A comment that encodes a false mechanism is how
this understanding propagated in the first place.

### 4.5 Teach the mock the failure, before writing any test

`crates/mock-goose-server/`. Today the mock **parks the ask forever** on client
disconnect: `Turn::ask_permission` waits on a oneshot whose `Sender` is in the
`Pending` map (`turn.rs:73-79`), and that map is cloned into a detached turn task
(`main.rs:272-284`), so a dead socket leaves the sender alive and the turn simply
waits. It behaves the way we wish goose behaved. A regression test written
against it today passes on a server that never had the bug, which is worse than
no test.

It needs two opt-in modes, because we do not yet know which world we are in:
`MOCK_DIE_ON_CLOSE=cancel` (self-answer `cancelled` when the socket dies with a
server-initiated request outstanding — section 2.2's world) and
`MOCK_DIE_ON_CLOSE=abort` (drop the turn task outright, emitting nothing —
section 2.3's world). The client fix must render both correctly, and it can,
because it reports from its own snapshot rather than from anything the server
says afterwards.

### 4.6 What none of this fixes

The ask is still decided or destroyed. The work in the aborted round is still
lost, and its side effects still have no transcript record. The user still
cannot answer a question they were asked. This is a change from a silent wrong
outcome to a stated one — which is worth shipping, and is not a fix.

Two more things explicitly not worth building, with the reasons:

- **Answering deliberately on backgrounding.** tao maps only
  `applicationWillResignActive:` to `Event::Suspended`
  (`tao-0.34.8/src/platform_impl/ios/view.rs:618-620`); `did_enter_background`
  is registered with a literally empty body (`:623`). So the signal also fires
  for a Control Center pull and a notification banner, and denying on it would
  destroy turns the user never left. And the payoff at the end of that work is a
  transcript byte-identical to today's, because `reject_once` and the synthesized
  `Cancel` both write `DECLINED_RESPONSE`. Worse: an explicitly answered ask can
  never benefit from the upstream fix in section 5, because
  `resend_pending_tool_permissions` skips anything with a recorded response.
  (For the record, `unsafe_code = "forbid"` is *not* what blocks this —
  `beginBackgroundTaskWithExpirationHandler`, `endBackgroundTask`,
  `backgroundTimeRemaining` and `sharedApplication` are all safe `pub fn` in
  objc2-ui-kit 0.3.2. The window is reachable; there is nothing worth sending in
  it.)
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

**Minimal change.** On ws loop exit, close the agent's *inbound* path first —
drop or close `inbound_tx` so the agent's incoming stream reaches EOF — and give
the connection future a bounded grace period to unwind before aborting. That is
what turns section 2.3's world into section 2.2's: pending replies fail with
`incoming_transport_closed`, close callbacks run, and any state the handler was
holding gets its chance to flush.

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
sends one, and goose adds nothing in `acp/transport/mod.rs`, so a network that
vanishes without a FIN leaks a turn and an agent indefinitely (section 3's third
case). That leak exists independently of anything to do with permissions.

## 6. Staging

The ordering is chosen so that nothing depends on a question that has not been
answered yet.

**Stage 0 — settle which world we are in.** Section 7.1. One afternoon against a
real goose over ws. Everything after this is cheaper once it is known, and
sections 4.5 and 5.1 are shaped differently depending on the answer.

**Stage 1 — the mock learns to fail (4.5).** Both modes. Nothing can be tested
before this and the CLAUDE.md lockstep rule requires it anyway.

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

## 7. What would falsify this, and how to check

**7.1 — Does the abort win, or does the `Err` arm run?** This is the load-bearing
uncertainty and it changes stages 1, 4 and 5.

Run a real goose over ws with the pinned SDK. Start a turn that hits an approval
gate. From a scratch client, half-close the socket (or `SIGSTOP` the client, or
send a Close frame) while the permission is outstanding. Then read the session
file. Three possible outcomes, three different worlds:

- a `DECLINED_RESPONSE` tool response present → section 2.2's world; the original
  report is right and 5.1 is unnecessary.
- the round absent entirely — no assistant message, no tool request → section
  2.3's world; this document is right.
- a tool request present with no response → a fourth world, in which
  `fix_conversation` will silently delete the orphan on the next prompt
  ("Removed orphaned tool request",
  `crates/goose-provider-types/src/conversation.rs:500-513`, applied at
  `agent.rs:801`). Also bad, also different.

Watching for the `error!("permission request failed")` line at
`acp/server.rs:1313` in the server log settles it directly: present means the arm
ran, absent means it was aborted.

**7.2 — Is the deployed goose the pinned rev?** All of section 2.3 assumes the
server the phone talks to is built from `c97a5203` and serves ws. A binary
predating it, or one reached over stdio behind a bridge, may be in section 2.2's
world regardless. Check the deployed build before writing PR 5.1.

**7.3 — Which client task wins the drain race?** Section 2.5 claims the
`send_prompt` sweep usually empties the queue before the pump's `Disconnected`
arm. Add a temporary log line at `src/state.rs:673` and `:1339` and kill the mock
mid-permission a dozen times. If `:673` ever sees a non-empty queue, the race is
real and both sites need treating regardless of which usually wins — which is
what 4.1 already does, so this experiment can only strengthen the design, never
change it.

**7.4 — Does a no-FIN disappearance really leak an agent?** Section 3's third
case. Block the tailnet route rather than closing the socket, wait, reconnect,
and count `GooseAcpAgent` instances or watch for two writers on one session file.
If it does not leak, the keepalive ask in 5.2 can be dropped.

**7.5 — Does WKWebView under dioxus-mobile fire `visibilitychange` on
backgrounding?** Unverified, and nothing in this design depends on it — it is
listed because two rejected mitigations did, and anyone revisiting them needs to
answer it first. The bridge pattern to test with is the one
`use_pull_to_refresh` already uses (`src/viewport.rs:437-450`): a JS listener
registered once, one message per transition, so it costs no per-frame
synchronous XHR.
