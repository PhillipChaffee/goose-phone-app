<!-- Produced by a research workflow: three agents mapped the goose plane, the
code plane and the client independently, then three more checked their claims by
RUNNING things — driving the mock goose server through connect/prompt/
disconnect/reconnect, standing up the code-agent stack and forcing an idle
spin-down, and running the desktop app to see what it writes to disk. Thirteen
of the mapping agents' claims came back wrong or oversimplified and were
corrected; the surprises section is where the obvious reading of the code is not
what actually happens. Anything that could not be settled without the real
server is marked as such rather than guessed.

REVISED 2026-09-08 (#314). The client half described the world before #220, and
#220 made all four storage keys reach the disk — so "the client persists exactly
one file" and "the Code transcript cache doesn't work" were both false, and every
`src/state.rs` line number in it had drifted through the desktop campaign. The
client half was re-measured and re-read for this pass. -->

# Where chats and sessions actually live

Two backends, two different answers. This is the model, with citations, and with the places where the obvious reading of the code turns out to be wrong.

Read against: this repo at `7ca4dd0`; goose at `/Users/phillipchaffee/git/goose` (commit `3810898a7`, version 1.46.0 per `crates/goose/Cargo.toml:11`); the gateway at `/Users/phillipchaffee/git/personal-ai-setup/scripts/vps/code-agent-manager.py`; OpenCode at `/Users/phillipchaffee/git/opencode` (commit `5e2a6257b2`, 1.18.5). Everything below is either read from source or measured on this machine; measured claims say so, and the last section lists what could not be checked here.

---

## The short answer

**Both planes are server-authoritative. The client persists four files, 1,702 bytes of them, and one of the four is a transcript cache.**

Measured on this machine on 2026-09-08:

```
~/Library/Application Support/goose-mobile/
-rw-r--r--  1436 Sep  7 18:29 code_cache
-rw-r--r--    18 Sep  7 18:25 inspector_open
-rw-r--r--    18 Sep  7 18:25 lost_asks
-rw-r--r--   230 Sep  7 18:25 settings
```

All four go through one `LocalStorage` backing, aliased at `src/ask_journal.rs:60` and hooked up at `src/state.rs:761-779`. `settings` is only 230 bytes because two of its six fields are `#[serde(skip_serializing)]` — `secret_key` (`src/state.rs:90-91`) and `code_password` (`:97-98`) — so the four non-secret fields persist and the two credentials cannot be written however the form is filled in. Until #220 all four keys went through `use_persistent`, which off wasm never reaches a disk at all; §5 has what that cost.

On the **goose plane**, a chat is rows in one SQLite database on whatever machine runs `goose` — one store per host, shared by the phone, the CLI and goose Desktop. The phone keeps no goose transcript at all, on purpose; every time you open a chat it asks the server to replay the whole thing. Restart the goose server and nothing is lost.

On the **code plane**, a chat is two things at two depths: a ten-field row in a single `index.json` owned by the Python gateway, and a directory on the VPS's disk holding the git clone *and* OpenCode's own SQLite database. The container in front of that directory is disposable; stopping it loses only in-flight things. The phone also keeps a transcript cache for this plane, and since #220 it works: `open_code_chat` seeds the screen's items, session id and review marks straight off the file (`src/code.rs:1029-1052`), so a chat opens instantly while its container wakes. It is a cache and not a record — LRU-capped at fifteen chats and the last three hundred items of each (`src/code.rs:451-452`), and the server stays authoritative.

The difference that matters most is not where things live, it's **what each server forgets mid-turn**. goose discards the entire round — the assistant's message and the tool call both — if a permission ask goes unanswered; only your prompt and the auto-generated title survive. OpenCode keeps the user message, the assistant message row and every *finished* tool call, but loses the streamed prose, because its durable log writes a text part twice (empty at creation, full at completion) and not once per delta.

One of the four files exists to paper over exactly the first of those, and it is the only one that exists for a reason of that kind: `src/ask_journal.rs` is not a cache of server state — after 75 seconds there is nothing on the server to reconcile against — it is the only surviving evidence anywhere that a question was ever asked.

---

## 1. The goose plane

### Where a session lives

One SQLite file per machine running `goose`:

- `SESSIONS_FOLDER = "sessions"`, `DB_NAME = "sessions.db"` — `crates/goose/src/session/session_manager.rs:28-29`
- under `Paths::data_dir()` (`crates/goose/src/config/paths.rs:52`), which honours `GOOSE_PATH_ROOT` when absolute (`paths.rs:40-46`). Note the shape: with a root override the file lands at `$ROOT/data/sessions/sessions.db`, one level deeper than you'd guess.

Verified on this machine: `~/.local/share/goose/sessions/sessions.db`, 2,506,752 bytes, `schema_version = 16` (matching `CURRENT_SCHEMA_VERSION` at `session_manager.rs:27`), 68 sessions, 786 messages, tables `schema_version, sessions, messages, usage_ledger, provider_inventory_entries, provider_inventory_models`. `pragma journal_mode` returns `wal`, matching the pool config at `session_manager.rs:897-910`.

WAL plus `BEGIN IMMEDIATE` on writes (`session_manager.rs:948`, `:1573`) is what makes concurrent writers safe. **There is one session store per host, not one per client.** A chat you start on the phone is in goose Desktop's list, and vice versa. Session ids (`20260827_3`) are minted by a `MAX(SUBSTR(id,10))+1` subquery inside the insert (`session_manager.rs:1576-1602`); observed live to increment with no gaps across separate connections and a server restart.

### What is on the row, and what isn't

The `sessions` row (DDL `session_manager.rs:993-1010`) carries `id, name, description, user_set_name, session_type, working_dir, created_at, updated_at, extension_data`, five token counters plus five accumulated ones, `accumulated_cost, schedule_id, recipe_json, user_recipe_values_json, provider_name, model_config_json, goose_mode, archived_at, project_id, parent_session_id`.

So the model, the provider, the working directory, the mode, the title, the enabled extensions, the recipe and the token/cost ledger are all **server-side session state**. Measured: setting `thinking_effort=low` through `session/set_config_option` wrote it into `model_config_json` on the row. Change the model on your phone and every later turn from any client uses it; the server pushes `config_option_update` so a second client stays in step (`src/state.rs:1251-1257`).

Two fields on the `Session` struct are **not** columns — `message_count` and `last_message_at` are computed per query (`session_manager.rs:1991-1993`). And `message_count` applies the user-visible filter: a session with two rows in `messages` reported `msgs=1`, because the second was goose's hidden turn-context message (see Surprises).

Message rows (`session_manager.rs:1016-1032`) hold `message_id, session_id, role, content_json, created_timestamp, timestamp, tokens, metadata_json`. Across all 786 rows there are exactly four content types — `text` 208, `thinking` 305, `toolRequest` 289, `toolResponse` 289 — and five metadata keys: `userVisible, agentVisible, turnContext, inference, usage`. **The model's thinking is persisted.** `actionRequired` appears in zero rows, which is the crux of §4.

### What the client does on open

The client holds no goose transcript. `ChatItem` has serde derives, and the comment says why: *"Serde derives exist for the Code tab's on-device transcript cache … goose chats are never cached"* (`src/state.rs:203-204`).

Opening a chat clears `items` and sets `loading` (`src/state.rs:1662-1672`), then calls `session/load` (`:1686`). The server reads the row with its messages, re-activates an in-memory agent, and **replays the whole conversation as live-shaped notifications** before the call returns (`crates/goose/src/acp/server/load_session.rs:269-329`), filtering to user-visible content first (`load_session.rs:25-36`) and emitting user/agent/thought chunks and tool calls (`:98-202`). The client folds those exactly as it folds live ones (`apply_update`, `src/state.rs:1212-1281`). Then a usage report built from the *stored* token total restores the context meter (`acp/server.rs:550-578`).

Measured against real goose: a seeded four-row round replayed as `UserMessageChunk, AgentThoughtChunk, AgentMessageChunk, ToolCall, ToolCallUpdate`, then `usage_update`, `UsageUpdate`, `AvailableCommandsUpdate`. Setting `total_tokens = 4242` on the row produced `UsageUpdate {"size": 1000000, "used": 4242}` on the next load.

Two fidelity losses in that projection, both measured:

- **Streaming boundaries are gone.** Two separate text blocks `PROBE-CHUNK-A` and `PROBE-CHUNK-B` came back as one chunk reading `PROBE-CHUNK-APROBE-CHUNK-B`. The merge is server-side, in `Message::user_visible_content` (`crates/goose-provider-types/src/conversation/message.rs:955-984`).
- **Times are on the wire and unread.** Every replayed chunk carried `_meta.goose.created` with the per-message unix second (`acp/server/message_meta.rs:26-32`). Nothing in `src/` reads it — the only occurrence in the whole client is a test fixture at `crates/goose-acp-client/src/types/mod.rs:550`. The comment on `mark_gap` at `src/state.rs:2083-2085` ("History arrives from the server in one burst with no per-message times") states a false premise. Replayed transcripts lose their gap marks for no reason.

### What a restart of the app looks like

The app opens on Settings, disconnected (`src/state.rs:790-796`) — but not on an empty form. Since #220 `settings` is a file, so the server URL, the certificate fingerprint, the working directory and the code-server URL are already filled in. What you retype is the two secrets, and that is now a decision with its reason written down (`src/state.rs:720-728`) rather than a consequence of the storage bug: `secret_key` and `code_password` are `#[serde(skip_serializing)]`, so the file cannot hold a credential however the form is filled in. Connect runs `refresh_sessions` (`src/views/settings.rs:70`) — one `session/list`, the only thing that repopulates the list. Tap a chat, and the whole transcript comes back off the server.

**No goose transcript is reconstructed locally. All of it is re-fetched.** What the file restores is the form, not the conversation.

---

## 2. The code plane

### Layer one: the index row

`Chat` is a ten-field dataclass — `id, repo, title, port, branch, base, model, probe, created, last_active` (`code-agent-manager.py:163-202`) — held in an `Index` that is a plain dict serialized to `/data/code-agents/index.json` (`:85-88`, `:205-231`). `save()` writes a temp file and `os.replace`s it, so a crash mid-write can't corrupt it.

Measured across a full lifecycle on a local instance: before the first create the file doesn't exist; after create it holds exactly those ten keys (`status` and `url` in the API response are computed live from `container_state`, not stored); and across create → session → prompt → spin-down → wake, exactly one field ever changed — `last_active`, bumped by the wake. `touch()` fires on every proxied request (`:1477`) and every ~30s during a stream (`:1200-1205`, `:1537`), so the file is rewritten constantly and says nothing new.

### Layer two: the volume

`/data/code-agents/chats/<id>/` with two subdirectories (`:989-991`):

- `workspace/` — a real git clone on `agent/<id>` (`:1019-1027`)
- `home/` — bind-mounted as the container's `$HOME` (`config/code-agents/Containerfile:27` sets `ENV HOME=/chat/home`; `-v {chat_dir}:/chat` at `:813`)

The conversation is all the way down inside that: `packages/core/src/global.ts:10-11` puts OpenCode's data under `$XDG_DATA_HOME` or `$HOME/.local/share`, `run_container` (`:797-826`) sets no XDG variable, so the transcript database is `/data/code-agents/chats/<id>/home/.local/share/opencode/opencode*.db` on the host. Verified empirically rather than inferred: a real `opencode` run under `HOME=/tmp/oc-home-probe` created `.local/share/opencode/opencode.db`, `.config/opencode`, `.local/state/opencode/locks` and `.cache/opencode` — all under `$HOME`, all therefore inside the volume.

The schema (`packages/core/src/session/sql.ts:22,68,82,119,140`): `project`, `session`, `message`, `part`, `session_input`, `todo`, and an `event` log with `event_sequence`. OpenCode's snapshot/checkpoint store lives in the same tree (`packages/opencode/src/snapshot/index.ts:71`) and is what backs diff and revert.

Two things seeded **only at create time**: the rendered `opencode.json` and `auth.json` (`render_chat_config` / `seed_auth`, `:889-920`, called once at `:992-993`). Redeploying with a changed config template or a rotated Zen key does not reach chats that already exist.

Because `-v {chat_dir}:/chat` is a host bind mount, nothing has to be flushed — the agent's writes were already on the host filesystem as they happened. Measured: with the container fully stopped, `git status --short` in the workspace still showed a modified `README.md` and an untracked `scratch-notes.txt`, HEAD still on `agent/<id>`, `auth.json` still `0600`, transcript state file intact.

### Idle spin-down and wake

`reaper_loop` (`:1146-1163`) runs every 60s (`:100`) and stops any running container idle longer than 900s (`:91`), after checking `chat_busy` (`:1120-1143`), which treats any failure to answer as busy. That is `podman stop`: container kept, volume kept.

**Survives:** the whole volume — clone, branch, uncommitted edits, transcript, snapshots, config, auth.

**Does not survive**, all of it in the OpenCode process's memory:

- **Parked permission asks.** `pending: Map<ID, PendingEntry>` in `InstanceState` (`packages/opencode/src/permission/index.ts:22-25`); the shutdown finalizer rejects every one (`:54-61`). There is no table for it.
- **"Always allow" grants.** Replying `always` does `approved.push(...)` into a plain in-memory array (`permission/index.ts:145-151`). The `git push` path is the V1 service — `tool/shell.ts:282-291` → `session/tools.ts:81-88` → V1 `Permission.ask`, with `tool/shell/id.ts:15-16` pinning the permission key to `"bash"`, which is exactly the key the config template gates. So "always allow this push" lasts until the container stops, then asks again.
- Session busy/retry status and the SSE subscriber set.

Measured end to end: parked ask `perm_27e1756c` visible on `/api/permissions`; `POST /api/chats/<id>/stop`; wake; `/chat/<id>/permission` → `[]`, `/api/permissions` → `{"permissions": [], "unreachable": []}`. The transcript afterwards is `user | summarise the readme` / `assistant | …` / `user | push the branch and open a pull request` — **the killed round's user message survives with no reply and no marker.**

Waking (`wake_chat`, `:1050-1108`) is `podman start` if the container is merely stopped, or a full `run_container` from the volume if it's gone (`:1101` — "volume has it all"; I confirmed this branch by deleting a chat without purge, hand-restoring its index row and waking it: container rebuilt, workspace and branch intact). Nothing is reconstructed; OpenCode reopens the SQLite file it left behind. Measured wake: 1.6-1.7s, same session id, transcript byte-identical.

### Manager restart

`ExecStopPost` is `podman stop --filter label=code-agent=1 --time 30` (`scripts/vps/systemd/code-agent-manager.service:31`) and every container carries that label (`:805-806`). A deploy SIGTERMs **every** chat container, waits 30s, then kills. The cost is the spin-down list applied to all chats at once and **without the busy guard**: every in-flight turn dies mid-stream and every parked ask is destroyed. There is no journal, no retry, and no client-side record: every call to the journal's own API — `note`, `resolve`, `forget_open`, `lose_open`, `acknowledge`, `reconcile_at_startup` — is in `src/state.rs`, on a goose-plane path (`:786`, `:1024`, `:1043`, `:1082`, `:1084`, `:1143`, `:2014`, `:2064`). `src/code.rs` names `crate::ask_journal` in exactly two places and both are `::Backing`, the storage alias every persisted key now shares (`src/code.rs:2604`, `:2610`), which is the module's other half and has nothing to do with recording an ask. A code-plane ask that dies in a deploy dies silently.

The manager itself loses nothing, because it holds nothing: every route does `Index.load()` fresh from disk (`:1314`, `:1356`, `:1410`, `:1467`). The only in-RAM state is a rate-limiter dict (`:1197`).

### The git tree is half the chat

`workspace/` is a real clone on a real branch, on the host bind mount. Uncommitted work survives spin-down, manager restart, image upgrade and reboot. It is destroyed by exactly two things: `?purge=1` on delete (`:1461-1462`, `shutil.rmtree`) and the create-failure rollback (`:1044`). This is why the design pushes toward a PR — the pushed branch is the only copy that outlives the volume (`docs/code-agents.md:50`).

---

## 3. What is lost when

| Event | goose plane | code plane | Client |
|---|---|---|---|
| **App killed / iOS jetsam** | Nothing on the server. The per-connection agent object dies (`acp/server.rs:2333-2345`), any turn in flight aborts and loses its round (§4); SQLite untouched | Nothing. The VPS doesn't notice | Everything except the four persisted keys. **Survives:** the settings form, the Code transcript cache and its review marks, the inspector's state, the journal. **Lost:** the two secrets, drafts, picked attachments, and every goose transcript (never cached by design). Open journal entries become `AppEnded` at next launch (`src/state.rs:784-789`) |
| **Backgrounded on iOS** | Nothing yet — the process freezes with the socket open. Long enough to be jetsammed and it becomes the row above | Same, plus idle spin-down eventually stops the container | Nothing at the moment of suspension; in-memory state is preserved, not lost, while suspended. There is **no shutdown hook** (see Surprises) |
| **Socket drops** | Per-connection agent destroyed, turn in flight aborts. Client clears the queue and marks open asks `Lost{Connection}` (`src/state.rs:1074-1090`), then reconnects and re-runs `session/load` (`:1160-1179`) | SSE stream ends; the container keeps running and the turn keeps going | Live UI state only |
| **Server / manager restarts** | **Nothing.** `sessions.db` is the state; measured by killing and restarting real goose against the same data root — both sessions and their message counts came back unchanged | Every chat container SIGTERMed at once: all in-flight turns killed, all parked asks destroyed, no busy guard. Volumes and `index.json` intact | — |
| **Container spins down (idle)** | n/a | Parked asks, in-memory "always" grants, busy status, SSE. Everything in the volume survives, including a partially-written turn | Nothing |
| **Chat deleted, no purge** | n/a | Index row and container gone (`podman rm -f`, `:1460`); the volume is orphaned on disk and the proxy answers 404. Recoverable only by hand-editing `index.json` — I did it, and the wake rebuilt the container from the volume | Cache entry removed (`src/code.rs:2450`), and since #220 that removal is written to the file, so it outlives the process |
| **Chat deleted with `purge=1`** (what the app always sends, `src/code.rs:2447`) | n/a | Everything, irreversibly: clone, branch, uncommitted work, transcript, snapshots. Only what was pushed to GitHub survives | — |
| **`index.json` lost** | n/a | Every chat at once, from the manager's point of view. Volumes survive; nothing maps a chat to its port or branch | — |
| **Unanswered permission ask** | The whole round: assistant message and tool call both. Prompt and generated title survive | Nothing extra at the moment — but the ask dies with the container, and it pins a concurrency slot while it lives | The journal note (the only record it happened) — goose plane only |

---

## 4. The surprises

These are the places where the code's shape suggests the wrong answer. Several were mapped wrong on the first pass and corrected by running things; where that happened, it is said plainly, because the first reading is the one your own intuition will also produce.

**1. OpenCode does not persist a streaming turn as it streams. The first pass had this wrong.** The mapping said "streaming deltas *are* durable events, so a partial assistant message is on disk as it streams." Measured against the real OpenCode database on this machine: a text part gets **exactly two** durable events — one at creation with length 0, one at completion with the full text. For the longest assistant part in that DB (`prt_03d2ed4f5001kvn3nr5MeYQLhi`, final length 5155) the event log is `459|0`, `460|5155`, and the whole turn is `step-start, reasoning(0), reasoning(5690), text(0), text(5155), step-finish`. The 842-events-to-473-parts ratio that looked like evidence *for* continuous persistence is 1.78 writes per part and refutes it. The SSE stream the phone consumes is high-frequency; the durable log is create-then-finalize. **A container killed mid-stream persists an empty text part — every character you watched arrive is gone.** The user message is durable first, and completed tool parts get three events each (pending/running/completed), so those do survive.

**2. goose's permission loss is a special case of something much more ordinary.** The mechanism (read, not run — see §6) is that in the legacy reply loop the user message is written immediately (`crates/goose/src/agents/agent.rs:1733-1735`), the title on a detached task (`:1745-1763`), and everything the assistant produced sits in a local buffer that reaches SQLite only at the bottom of the loop iteration (`:3339-3341`). The ask itself is built and yielded but never persisted (`crates/goose/src/agents/tool_execution.rs:173-183`). Measured with a deliberately invalid API key: the user message and the hidden turn-context message were persisted, and the assistant's error message ("Authentication error… 401") streamed to the client and **never reached SQLite**. So *any* round that dies before the bottom of the loop — provider outage, rate limit, network blip — leaves a session containing your prompt and nothing else. The permission ask is just the version you notice.

**3. The recovery path for lost asks already works; only the write is missing.** `resend_pending_tool_permissions` (`load_session.rs:206-269`) scans persisted `actionRequired` messages and re-raises unanswered ones, and it looked like dead code wired to a message type nothing writes (0 of 786 rows). Nobody had tested the other half. Injecting one synthetic `actionRequired` message into a throwaway database and calling `session/load` made real goose 1.46.0 **raise a live permission request** at the client. So the upstream fix is a one-line-ish "persist the ask" at `tool_execution.rs:173-183`, and the phone would recover parked rounds the day it lands with no client change. One caveat from the same run: the re-raised ask arrives with no transcript entry behind it, because the replay's content match has no arm for `ActionRequired`.

**4. `?nopurge=1` purges.** `route_delete_chat` decides with a substring test: `purge = "purge=1" in (urlparse(self.path).query or "")` — `code-agent-manager.py:1452`. Verified live: `DELETE /api/chats/<id>?nopurge=1` returned `{"volume": "purged"}` and the directory was gone. `?xpurge=1` and `?purge=1x` do it too. The app is immune only because it hardcodes purge anyway; a hand-typed curl is not. One-line fix: `parse_qs(...).get("purge") == ["1"]`.

**5. The mock goose server on :3285 is lying about exactly the failure the journal exists to survive — and it lies in the opposite direction from what was written down.** The write-up said the mock "parks a pending ask forever", i.e. too forgiving. Measured: in park mode (the default, `crates/mock-goose-server/src/state.rs:152-155`, chosen at `:163-168`) the abandoned round leaves **nothing at all** — not even your prompt — because the mock only commits a round when the turn finishes, and the session vanishes from `session/list` entirely. Real goose keeps the prompt and the title. There *is* a faithful mode (`MOCK_DIE_ON_CLOSE=abort`, `main.rs:367-368` → `discard_rounds` at `:379`), it is opt-in, and `CLAUDE.md` documents the command without it. Worse, and this one is dated rather than standing: at the time of that run the binary listening on :3285 was a stale build from a different worktree (`/…/worktrees/feat-integration/target/debug/mock-goose-server`, started Aug 25) whose source had no `DieOnClose` at all — nothing was listening on :3285 when this was revised, so the hazard is "a long-lived mock outlives the tree it was built from", not a process you can go and look at. Consider making `abort` the default. Also, the mock stores rendered notifications rather than structured messages (`state.rs:15`), so it replays back exactly the bytes it sent — every fidelity loss in §1 is invisible against it — and it restarts empty, the opposite of real goose.

**6. Debug builds put the goose secret and the OpenCode password in the binary, in plaintext.** The client-plane analysis framed "secrets never touch disk" as an accidental upside of the storage bug and a reason not to fix it. That framing is gone from the code and the conclusion survived it: #220 made `settings` a file and kept both credentials out of it deliberately, and `src/state.rs:720-728` is the argument — *"writing them to a plain file is a change to the app's threat model rather than a bug fix"*. What the binary does is untouched by any of that. `dev_seed!` (`src/state.rs:157-168`) is `option_env!`, live in any debug build, and `docs/design.md:825-832` documents seeding it; `strings target/debug/goose-mobile` after such a build prints the seeded URL and secret next to the storage key names (measured before #220, so the exact neighbours in that string differ now that there are four keys — the secret's presence does not). Since `docs/iphone-setup.md:150` makes `dx serve --ios --device` the normal loop, the credentials ride into the `.app` bundle onto the phone. (Corollary worth knowing: `option_env!` is evaluated at **compile** time, so setting `GOOSE_DEV_*` before running the binary does nothing — they must be set on the build. `src/state.rs:145-153` now says so in the code as well.)

**7. 62 of the 68 sessions on this machine can never appear in the app.** goose has seven session types (`session_manager.rs:46-55`); the client knows three — `SessionKind::ALL = [User, Scheduled, Acp]` (`crates/goose-acp-client/src/types/session.rs:96`) — and filters the list on them (`src/state.rs:1508`). The real distribution is `hidden` 52, `sub_agent` 10, `acp` 3, `user` 3. The Chats list is a filtered view of the store, not the store.

**8. Empty sessions exist forever and no list will ever show them.** `only_sessions_with_messages: true` is hardcoded (`crates/goose/src/acp/server/list_sessions.rs:117` and `:205` — not `:235`, which is the response builder) and becomes a `JOIN messages` rather than a `LEFT JOIN` (`session_manager.rs:1971`). Measured: three rows in `sessions`, `session/list` over all three kinds returned exactly one. The app's "New chat" creates the session before you type (`src/views/sessions.rs:163` → `src/state.rs:1704`), so backing out orphans a row.

**9. The working-directory rewrite is a live cross-client hazard, not a dormant fallback.** Measured: a session with `working_dir='/tmp'`, loaded with `cwd='/'`, came out of it with `working_dir='/'` — permanently (`acp/server.rs:913-917`, applied `:937-942`). The first pass concluded this was unreachable because the client's `/` fallback never fires. But `reload_chat` (`src/state.rs:1185-1209`) sends `chat.cwd` — the value cached when you opened the chat, possibly hours ago — on **every auto-reconnect**. If goose Desktop or the CLI moved that session's working directory in the meantime (they do this same write), the phone's silent reconnect moves it back. Two clients can tug a session's cwd back and forth with nothing in either UI saying so.

**10. Search matches only plain text blocks.** Measured against real goose: `PROBE-USER-MSG` → 1, `uname` → 1, but `PROBE-TOOL-OUTPUT` → 0, `PROBE-THINK` → 0, and the session's own name → 0. `session_manager.rs:362-385` does `instr(LOWER(json_extract(value,'$.text')), ?)` over user-visible text items only. `src/state.rs:1548-1553` documents the title half correctly and is silent on tool output and thinking.

**11. Attachments: text keeps its name, images lose theirs, binaries are discarded entirely.** Measured with one prompt carrying four blocks. What was persisted: the text, the text-resource wrapped as `--- Resource: file:///tmp/probe-TEXTNAME.txt ---\n…`, and `{"type":"image","data":…,"mimeType":"image/png"}` with no name. The PDF blob left **no trace, not even its URI** — `acp/server.rs:1043-1050` is an `if let … TextResourceContents` with no `else`, and `ContentBlock::Audio(..) | _` at `:1056` is a no-op. Our client sends every non-image, non-text attachment as exactly that blob (`src/attach.rs:779`), and the doc comment at `crates/goose-acp-client/src/types/mod.rs:127-129` claims *"so the agent at least receives the bytes and the name"* — **it does neither**. Attach a PDF and the model never sees it, while the UI shows a chip. That is a live bug.

**12. goose writes a hidden `<turn-context>` user message into every turn**, carrying your wall clock and working directory, marked `userVisible:false` — so it never replays and never counts toward `message_count`, but it is in the database and in the model's context. The row count per turn is always at least two.

**13. An unanswered code-plane ask pins one of two concurrency slots open indefinitely.** `chat_busy` is true while a turn is blocked, so the reaper does `touch(cid); continue` (`:1157-1159`) forever. `MAX_ACTIVE` is 2 (`:92`), and a wake beyond that is a hard 409 — the app's toast reads "Chat unreachable: wake failed: 2 chats already active — stop one or wait for idle spin-down." Combined with the ask dying silently in a deploy, the recovery is: the slot frees, and the reason it was stuck is unrecoverable.

**14. `wait_for_chat` accepts any process listening on the chat's port.** `next_port` (`:874-880`) only avoids ports claimed by other chats in `index.json` — it never checks the host — and `wait_for_chat` (`:848-863`) returns true on HTTP 200 *or* 401. Hit by accident during checking: a stale manager squatting a port made a wake report success into an unrelated service, which the proxy would then have fed that chat's prompts. Related: `podman start` reuses the port and env baked in at `podman run`, so changing a chat's port in `index.json` produces a 502 while the container is up and healthy on the old port, and a rotated `GITHUB_CODE_AGENT_PAT` or `OPENCODE_SERVER_PASSWORD` doesn't reach existing containers until `podman rm`.

**15. Smaller corrections to the first pass, so the citations can be trusted:** `index.save()` is called in **five** places, not four — the fifth is the create-failure rollback at `:1042`, the only write that removes a row without a client asking. The `ChatItem` comment is `src/state.rs:203-204`. The OpenCode database filename can be channel-suffixed (`opencode-local.db` sits beside `opencode.db` on this machine); the load-bearing part — that it's under `$HOME`, hence in the volume — is unaffected. The service file is at `scripts/vps/systemd/…`, and because `Restart=always` (`:32`), `ExecStopPost` also fires on every crash-restart; while the manager is down there is no reaper, so nothing spins down. The real OpenCode database does have a `permission` table — that's PermissionV2's, unused by the bash path, so its presence is not evidence the grant is durable.

**16. iOS has no shutdown hook, and that's survivable only by accident of design.** Source read, not run. tao raises `Event::Suspended` on `applicationWillResignActive` and `Event::Resumed` on `applicationDidBecomeActive` (`tao-0.34.8/src/platform_impl/ios/view.rs:614-620`), but dioxus-desktop's event loop drops both (`dioxus-desktop-0.7.10/src/launch.rs:27-101`, catch-all `_ => {}` at `:100`). The app freezes with no notification and nothing is flushed at suspension. That's fine only because the durable write that matters happens **eagerly, on ask arrival** (`src/state.rs:1017-1027`) rather than on the way out. It becomes a problem the moment anything wants to save *at* backgrounding.

**macOS raises neither event, so the desktop shell has no hook to lose** — also a source read (#319, re-checked here). `Event::Suspended` and `Event::Resumed` are constructed in exactly two of tao 0.34.8's backends: iOS at `view.rs:615,619` and Android at `platform_impl/android/mod.rs:130,140`. Nothing in the macOS backend emits either, and nothing in dioxus-desktop 0.7.10 names them at all. So a "flush on backgrounding" design cannot be prototyped on the desktop shell and then carried to the phone — the desktop would silently never fire it.

---

## 5. Where the client is the only holder

This is where data loss actually lives. Four things; **the first and the third are protected, and the third only since #220.**

**1. The record that an ask existed.** `src/ask_journal.rs`, backed by `use_synced_storage::<LocalStorage, _>` (`src/state.rs:776-779`, alias at `ask_journal.rs:60`). It was the only file the app wrote; it is now one of four, and the only one that exists to survive a loss rather than to save a fetch. Measured end to end: 18 bytes at rest (zlib-compressed, hex-encoded CBOR `0x80` — the empty array); 340 bytes one second after an ask arrived, while the app was still alive; still 340 bytes and still `Open` after `kill -9`; 384 bytes and `Lost{cause: "AppEnded"}` after relaunch, surfacing in the UI as an amber dot and the line "An answer never reached goose. That round was discarded."

It is worth being precise about what this is. After goose discards the round there is **nothing on the server to reconcile against** — the session keeps your prompt and the title and reads like a completed request. The journal cannot un-lose the round (`ask_journal.rs:15`); it can only stop the loss being silent. It is the only artifact anywhere that says a question was asked.

Two seams, and #220 widened both from one key to four. The write is not atomic — `create_dir_all`, then `File::create` (which truncates), then `write_all`, no temp-and-rename (`dioxus-sdk-storage-0.7.0/src/client_storage/fs.rs:37-40`) — and those three lines are three `.unwrap()`s that sit on the arrival of every permission ask. For the two *synced* keys there is a fourth, on the notify that follows the write (`fs.rs:71`), which is the panic `testkit::anchor_subscriptions` exists for. The seams now also carry `code_cache`, which is by far the largest of the four and the one written most often — see #310.

**2. What an attachment was.** goose stores image bytes and mime and nothing else, so a replayed photo comes back nameless; `sent_attachments` (`src/attach.rs:950-973`) carries the names across a reload **in memory, within one process**. Kill the app and every photo in every goose transcript becomes a grey chip called "Image", permanently. For binaries it's worse: the server discarded the block, so the chip is the only evidence — of something that did not happen.

**3. Code-plane review marks and the transcript cache — the one item on this list that got fixed.** `CachedChat::diff_seen` (`src/code.rs:443-446`) carries the comment *"Persisted because a review is a task you leave and come back to, and the app being backgrounded while a container wakes is the normal case here."* Until #220 that was false: the whole cache was on a hook that never reached a disk, so the marks and the offline transcript were rebuilt empty at every launch and the cache's stated purpose — instant open while a container wakes — was exactly the cold-start case it could not serve. Both halves work now. `open_code_chat` reads the marks and the items back out of the file (`src/code.rs:1029-1052`), and two tests hold it: `the_transcript_cache_reaches_the_disk` (`src/code.rs:2553`) asserts that mounting writes the key at all, and `the_cached_transcript_survives_the_backing_it_is_stored_through` (`:2576`) asserts a whole `CachedChat`, `diff_seen` included, round-trips through the store. There is still no server-side notion of "reviewed" to fall back on, so the file is the only holder — it is just a file that exists.

**4. Drafts and picked attachments.** `chat_draft`, `code_draft`, `attachments`, `code_attachments` are plain signals. The attachments hold the actual base64 bytes of a *downscaled* photo. Deliberately hoisted onto the context so they survive a screen change (`src/state.rs:451-466`); they do not survive a process death.

**How the first and third came to differ, and why it is history now.** `use_persistent` does not persist on this app's targets: it is hardcoded to `SessionStorage` (`dioxus-sdk-storage-0.7.0/src/persistence.rs:34`), which off-wasm is an `Rc<RefCell<HashMap>>` on the Dioxus root context (`client_storage/memory.rs:15-29`). That is still true of the hook and is no longer true of this app — nothing in `src/` calls `use_persistent`, and all four keys are `dioxus_sdk_storage::use_storage` / `use_synced_storage` over `crate::ask_journal::Backing` (`src/state.rs:761-779`). The measurement that made the case is kept because it is what #220 was written to fix, and it is dated: **before #220**, with an unseeded build, typing the URL, secret and working dir and connecting successfully left `~/Library/Application Support/goose-mobile/` holding only `lost_asks` with its mtime unchanged — no `settings`, no `code_cache` — and `kill -9`, relaunch, every field empty. The decisive detail was that the storage helper writes the default back on a cache miss at hook construction, so a filesystem-backed `settings` would exist unconditionally on first run; `lost_asks`'s mtime bumped on all three launches and `code_cache` was never created once.

That detail is now the confirmation. Measured on `7ca4dd0` (`dev` profile, the test build): mounting the app under a private storage namespace and touching nothing writes all four keys — `code_cache` 32 bytes, `inspector_open` 18, `lost_asks` 18, `settings` 112 — where before #220 it wrote one.

The app's entire deliberate on-disk footprint is those four files. `~/Library/WebKit/goose-mobile` and `~/Library/Caches/goose-mobile` are WebKit's own bookkeeping — `grep -rn 'localStorage|sessionStorage|indexedDB' src/ assets/` is empty.

---

## 6. What could not be verified here

- **The permission-loss mechanism.** §4.2 is a source read plus a corroborating measurement with a bad API key. Actually reproducing a permission ask needs a real model turn, and there is no local model on this machine. The falsifiable prediction stands: run goose with `GOOSE_STATE_MACHINE=1` (`crates/goose/src/agents/state_machine/mod.rs:63-67`, opt-in, off by default, so the live path is the legacy loop) and the round should survive, because that path persists per effect batch (`agents/state_machine/session.rs:41-44`). If it doesn't, this section is wrong.
- **That the generated title survives an aborted round.** Title generation needs a model; only reproduced against the mock in abort mode.
- **That `session/load` replays before the RPC resolves.** Not observable from a client — notifications arrive on a separate task. Believe `load_session.rs:269-329`, don't treat it as measured.
- **"75 seconds."** That is how long the harness waited before killing the client (`docs/permission-durability.md` §0), not a measured server-side timeout. What was measured is that after the client dies the round is gone, for both `kill -STOP` (socket still open) and `kill -9`. Nothing establishes what happens to a client that holds the socket open and simply never answers.
- **The Android storage path.** `directories` 4.0.1 ships `lin.rs`, `mac.rs`, `wasm.rs` and `win.rs` and no Android branch, so it routes Android to the Linux path; either `BaseDirs::new()` is `None` and the unwrap in `set_dir_name` panics at startup, or `create_dir_all` panics on the first write (`fs.rs:37`). Both ingredients confirmed; the outcome depends on `$HOME` in an Android app process. Device check, not a finding — but note #220 widened what rides on it: the first write is no longer a permission ask nobody may ever receive, it is the mount itself, because all four keys are written on the way up. iOS is fine — it resolves to `$HOME/Library/Application Support/goose-mobile`, inside the sandbox container and in the backed-up part of it.
- **That the per-repo `setup` command's installs are discarded.** Sound from source (`oneshot`, `:830-845`, runs `--rm`), but the local test harness cannot prove it and actively suggests the opposite: `stub-engine.sh:52-58` handles `--rm` by running the script on the host, so a probe marker written outside the volume survived. Needs a real podman.
- **PermissionV2 drift.** The image is `ghcr.io/anomalyco/opencode:latest` (`Containerfile:15`) and the checkout read is 1.18.5. If `:latest` ever moves the `bash` tool onto V2, asks stop appearing in the app with no other symptom, because the client only parses `permission.updated` / `permission.asked` / `permission.replied` (`crates/opencode-client/src/lib.rs:1111-1120`). A version-drift risk, not a present bug.
- **The real code plane.** Everything code-plane here was checked against a locally-run manager with a mock OpenCode behind it, plus the real OpenCode database on this machine for the event-log shape. The mock is honest about transcript persistence and in-memory pending state; do not trust it on grant persistence, which is stubbed.

---

## Two things worth fixing

1. **Binary attachments are silently dropped by the goose server** (`acp/server.rs:1043-1050` vs `src/attach.rs:779`), and our own doc comment asserts the opposite. Either stop offering binary attachment on the goose tab, send it as text, or upstream the missing `else`.
2. **`_meta.goose.created` is on the wire and unread** (`acp/server/message_meta.rs:26-32`), so replayed transcripts lose their time marks for nothing. The comment on `mark_gap` at `src/state.rs:2083-2085` states the wrong premise and should go with the fix.