# Is Smart approve real? — how an approval mode reaches goose

Yes. This document is the trace, written down because it existed nowhere in the
repo: the owner asked *"How does Smart Approve work? I'm worried that it doesn't
actually work"* while running the app against a real goose server, and the only
way to answer was to re-derive it from six files. A user cannot read a stack
trace to reassure themselves that a safety control is wired, and this is a
question that will be asked again.

It is the companion to [`permission-durability.md`](permission-durability.md),
which answers the adjacent question — what happens to an ask that is already in
flight when the transport dies. This one is about the setting that decides
whether there is an ask at all.

**Every claim below was re-checked by running the tests named**, not by
reading. Line numbers are this commit's; the SYMBOL names are the durable half
of each citation, and `src/citations.rs` is the gate that holds the paths.

---

## The short answer

1. The modes are **goose's**, not this app's. Their names, their descriptions
   and the set of them all arrive over the wire.
2. Picking one sends a real RPC — `session/set_config_option` — and nothing
   about the chip is optimistic.
3. The chip is redrawn **only from the option set goose answers with**. A
   refusal toasts and leaves the old value showing.
4. The desktop gets all of it unchanged: one `ChatView`, one permission modal,
   both shells.

## The trace

### 1. The chip is goose's words

`src/views/chat.rs:205-227` renders the mode chip in the composer.
`is_mode_chip` (`chat.rs:369-372`) picks the option out of goose's
`configOptions` by either the ACP `mode` **category** or the `mode` **id** —
either will do, so an agent that sends only one of them is still understood —
and only when the option is *adjustable*, which
`ConfigOption::is_adjustable` (`crates/goose-acp-client/src/types/config.rs:67`)
defines as having more than one value. `mode_choices` (`chat.rs:383-391`) builds
the picker's rows out of `option_choices`
(`src/views/session_settings.rs:243-252`), which reads each choice's `name` and
its `description` straight off the wire.

So **"Smart approve" and "Ask only for sensitive tool calls" are goose's
strings**, and that is checkable from the other side: neither phrase is in any
source file here. `git grep -i "smart approve" -- src crates assets scripts`
matches nothing, and this document is the one place in the tree the words are
written down at all. The fake in
`crates/mock-goose-server/src/features/core.rs:222-234` ships a different set
again — `auto` / `approve` / `chat`, "Run tools without asking." / "Ask before
every tool call." / "No tools at all." — because the fixtures are one server's
answer and not a list this app knows.

The only thing the app adds is an icon, because neither ACP nor goose has a
field for one: `mode_icon` (`src/views/session_settings.rs:90-105`) matches on
substrings of the id, so an id this app has never seen — `smart_approve` among
them — still lands on the shield rather than on the generic bolt. Anything
unrecognised gets the bolt, which is a mark meaning "a mode" rather than a claim
about what it does.

### 2. Picking one sends a real RPC

The picker's `onchoose` (`chat.rs:300-306`) calls
`crate::state::set_config_option`, which is `src/state.rs:2086-2104`, which
calls `SessionClient::set_config_option`
(`crates/goose-acp-client/src/client/session.rs:209-229`). That emits:

```json
{
  "sessionId": "...",
  "configId": "mode",
  "type":     "id",
  "value":    "smart_approve"
}
```

`type: "id"` is the discriminator every id-based option kind uses and `value` is
flattened beside it, not nested under it. The exact params are asserted:

```
cargo test -p goose-acp-client set_config_option_sends_the_id_discriminator_and_returns_the_new_set
cargo test -p goose-mobile  switching_a_session_option_takes_the_answer_the_agent_gives_back
```

The second (`src/state.rs:5940`) asserts the wire params verbatim.

`set_config_option` is deliberately id-agnostic: goose routes exactly four ids —
`provider`, `mode`, `model`, `thinking_effort` — and rejects anything else, so
what is settable is the agent's to state and this app's to relay.

### 3. Confirmation is server-first, which is the safe direction

`src/state.rs:2099-2101` is the whole of it:

```rust
Ok(opts) if !opts.is_empty() => ctx.config_options.clone().set(opts),
Ok(_) => {}
Err(e) => show_toast(&ctx, format!("Could not switch: {e}")),
```

The chip is **never** redrawn from what was tapped. It is redrawn from the
option set goose sends back, which is also pushed as a `config_option_update`
notification, so a second client watching the same session stays in step and
whichever arrives first wins.

A refusal is visible and does not lie: the same test drives an `rpc_error` and
asserts both the toast (`"Could not switch: no such mode"`) and that the chip
still reads the value it had before.

### 4. It reaches the desktop unchanged

One `ChatView` serves both shells (`src/nav.rs:501`) and the permission modal is
rendered at the app root for both (`src/app.rs:79-83`), so a mode that produces
an ask produces one in a 1440×860 macOS window exactly as on a phone.

### 5. Measured against a real server, not inferred

[`permission-durability.md`](permission-durability.md) §0 records a run against
**goose 1.46.0** over a Tailscale tailnet in which
`crates/goose-acp-client/examples/perm_loss.rs:196-202` set the mode through
this same call and the ask arrived:

```
ASK RECEIVED: Some("shell · uname -a")
```

That harness sets `approve` rather than a smart mode, deliberately — the
server's default is `auto`, which approves everything, so there would be no ask
to measure. It is the same RPC on the same route; what it establishes is that
the mode this app sets is the mode the server enforces.

### 6. "Applies from your next message" is accurate

The sheet's subtitle (`src/views/session_settings.rs:361`) says
`{backend} · applies from your next message`.
`session/set_config_option`'s own contract
(`crates/goose-acp-client/src/client/session.rs:200-202`) is that the change
lands on the session immediately and the next `session/prompt` uses it. Nothing
in flight is re-decided, which is the honest reading of that sentence and the
one a user acts on.

## One narrow caveat, deliberate

If goose ever answered with **no parseable `configOptions`** — `Ok(_) => {}`
above — the stale mode stays on the chip with no toast. That is a tested
decision and not an oversight: the same test drives an `ok(json!({}))` answer
and asserts the picker is left alone rather than emptied. It is unreachable
against a server that answers normally, because the answer to a successful
`set_config_option` is the full option set.

## What would make this false

Written down so the next reader can check rather than re-derive:

- **The chip starts drawing the tapped value.** Any `set()` on
  `ctx.config_options` before the `await` in `src/state.rs:2086` turns this from
  server-first into optimistic, and a refused switch would then paint a safety
  setting the server is not using.
- **The mode is filtered out of the settings sheet on purpose**
  (`goose_setting_rows`, `chat.rs:448`), because the chip is where it lives.
  Hiding the chip therefore hides the control entirely rather than moving it.
- **`is_mode_chip` requires more than one value.** An agent that offers exactly
  one mode gets a fact in the sheet and no chip — a chip opening a one-row
  picker is a control that does nothing (design rule 11).
