<!-- Produced by a research workflow: four agents surveyed OpenCode's own
surface, our gateway's route table, goose's four library features and the two
reference desktop apps; two more then checked their claims by driving the REAL
production container image rather than the mock. 13 of 36 claims came back
wrong or oversimplified — including "the standard harness can settle this",
which it cannot, because the mock is protocol-faithful and not
behaviour-faithful. Claims that are code reads rather than measurements are
marked unverified in place. -->

> [!IMPORTANT]
> **Superseded in part by [`corrected-findings.md`](corrected-findings.md).**
> This document's *negative* claims were produced by searching for fields rather
> than for capabilities. A later adversarial pass attacked all sixteen of them
> and **refuted thirteen**. The headline errors are fixed inline below; the full
> list, the corrected capability-first picture, and the method post-mortem are in
> that file. Read it first.

# The Code half's library: what OpenCode actually offers

*A decision document. Everything in it was read on disk or measured against a running server; where a claim is a code read rather than a measurement it is marked **unverified**.*

**Sources.** OpenCode at `/Users/phillipchaffee/git/opencode` (package version 1.18.5, commit 5e2a6257b2). Our gateway at `/Users/phillipchaffee/git/personal-ai-setup/scripts/vps/code-agent-manager.py`. Our client at `/Users/phillipchaffee/git/goose-phone-app/crates/opencode-client/src/lib.rs`. goose at `/Users/phillipchaffee/git/goose`. Citations below are repo-relative against those four roots.

**Version drift, which matters for every citation.** Three versions are in play: the source tree everyone cites is **1.18.5** (`packages/opencode/package.json:3`), the local binary is **1.18.20**, and production runs `ARG OPENCODE_IMAGE=ghcr.io/anomalyco/opencode:latest` (`config/code-agents/Containerfile:15`) which reported **1.18.25**. Measurements below were taken on 1.18.20 and 1.18.25 and agreed with the 1.18.5 loader in every respect checked — but the tag floats, and the Containerfile's own comment (`:12-14`) already flags pinning as a TODO.

---

## 1. The answer in a paragraph

**The premise is wrong on two of four, and the honest answer is "one library entry, not four."** OpenCode has Skills — literally the same `SKILL.md` format, and it deliberately reads `.claude/skills/` too (`packages/opencode/src/skill/index.ts:37-42`, `:195-215`). It has the Recipes analogue as **Commands**, and the Extensions analogue as **MCP servers**. It has **no scheduler of its own** — but it is a first-class cron *target* via GitHub Actions' `schedule` event, which has a dedicated code path in the shipped binary. goose's scheduler has a destination: an inbound trigger, not a peer. But the structural fact that decides the design is not the count: it is that OpenCode does not keep four namespaces, it keeps **one**. `GET /command` returns config commands, MCP prompts and every skill folded into a single list, each tagged `source: "command" | "mcp" | "skill"`, each carrying `hints` (the `$1`/`$ARGUMENTS` placeholders) ready to drive argument prompting (`packages/opencode/src/command/index.ts:27`, `:73-152`). Measured live in the production image, one GET returned 16 entries spanning a phone-authored command, a repo-committed command, a built-in skill, a gateway-seeded skill and a registry-installed skill. So the Code half should get **one list with a source badge** — "what this repo's agent can do" — plus Agents (which our client already fetches) as its "how it runs" facet, and that list belongs **inside a code chat**, next to Diff and Pulls, not as a peer of Recipes in the drawer. The gateway is not the bottleneck: `ROUTE_CHAT` (`code-agent-manager.py:1263`) is matched first in `handle_any` and `proxy()` (`:1466-1510`) forwards method, path, query and body verbatim, so all ~110 OpenCode routes are already reachable; our client touches 11 of them. The entire cost of the recommended scope is **one Rust method per list, copied from `agents()`** (`crates/opencode-client/src/lib.rs:1407`).

---

## 2. The inventory

Three columns come apart deliberately: **(a)** OpenCode supports it, **(b)** our gateway exposes it, **(c)** our client speaks it. Sorted by value for effort, best first.

| # | Candidate | The job it does | OpenCode? | Gateway? | Client? | Cost |
|---|---|---|---|---|---|---|
| 1 | **Commands** (`GET /command`) | The whole Code-side library in one call — saved prompts, MCP prompts and every skill, tagged by `source`, with `hints` for argument prompting | **Yes** — `packages/opencode/src/command/index.ts:73-152`; route constant `command: "/command"` at `.../httpapi/groups/instance.ts:51` | **Yes**, wildcard proxy | No | **One client method.** Running one is `POST /session/:id/command` (`groups/session.ts:97,343`) — fire-and-forget it |
| 2 | **Skills** (`GET /skill`) | Name, description, location, **full `SKILL.md` content** | **Yes**, same format, reads `.claude/skills/` too (`skill/index.ts:37-42`) | **Yes** | No | **One client method.** `src/skills.rs:1-25`'s payload rules transfer verbatim — `customize-opencode` alone is 16,060 bytes |
| 3 | **Skill registry** (`skills.urls`) | Install a whole skill library into a chat from an `index.json` | **Yes** — `packages/core/src/v1/config/skills.ts:9-11`; fetcher at `packages/opencode/src/skill/discovery.ts:49-131` | **Yes** | No | **One config line.** Measured: all 12 real skills in `personal-ai-setup/config/skills/` installed into a live chat, multi-file, hot, surviving stop/start |
| 4 | **Agents / subagents** (`GET /agent`) | Per-repo personas with their own tools, model and permissions | **Yes** (`groups/instance.ts:52`) | **Yes** | **Yes, already** (`lib.rs:1407`, feeding the mode picker at `src/code.rs:634-735`) | **Presentation only.** Promoting the picker to a library detail is not plumbing |
| 5 | **Durable authoring** (`PATCH /global/config`) | Write a command, agent, MCP server, skill path or permission rule from the phone, live, no restart | **Yes** — `groups/global.ts:68,103`; `Config.updateGlobal` at `config/config.ts:637-660` | **Yes** (`do_PATCH` → `handle_any`, `code-agent-manager.py:1552`) | No | **One client method.** `CodeClient::req` is already generic over the verb (`lib.rs:998-1002`) — `Method::PATCH` needs no new code path |
| 6 | **MCP servers** (`GET /mcp`) | The Extensions analogue: what the agent is plugged into, with a real 5-way status | **Yes** — `groups/mcp.ts:32-39,45,55` | **Yes** | No | One method to list. **Remote-only** for us (see §5), and durable adds must go through #5, not `POST /mcp` |
| 7 | **Saved permissions** (`/api/permission/saved`) | The accumulated "always allow" list — inspect, audit, prune | **Yes** — `packages/protocol/src/groups/permission.ts:37-58` | **Yes** (measured 200 `{"data":[]}`, DELETE 204) | No | Two methods. **The only genuine DELETE verb in the whole surface** |
| 8 | **Permission policy** | The per-repo sandbox as an editable, visible thing | **Yes** (config `permission`, incl. `permission.skill` globs) | **Yes** | Partially — we write one rule per chat at create time (`code-agent-manager.py:889-903`) | Read is free with #5; write is a design decision, not plumbing |
| 9 | **Rules** (`AGENTS.md`, `instructions[]`) | What the repo tells its agent | **Yes**; `POST /session/:id/init` writes `AGENTS.md` | **Yes** | No | Low value as a *library* — it is one file, better read in the repo |
| 10 | **References** | Aliases to other repos/dirs | Yes | Yes | No | **Blocked by our own template** — `external_directory: {"*": "deny"}` (`config/code-agents/opencode.json`) is exactly the boundary references punch through |
| 11 | **Formatters / LSP** | `GET /formatter`, `GET /lsp` (`groups/instance.ts:54-55`) | Yes | Yes | No | Diagnostics, not a library. Belongs in a status line if anywhere |
| 12 | **Plugins** | Arbitrary JS/TS in the container | Yes | n/a — **no HTTP route at all** | No | See §5 |
| 13 | **Themes / keybinds** | TUI appearance | Yes, TUI-only | Routes proxy, and lie | No | See §5 |
| 14 | **PTY / terminal** | A shell in the container | Yes | **No — structural** | No | See §5 |
| 15 | **Scheduler** | Unattended recurrence | **No. Nothing.** | n/a | n/a | Ours to build, in the gateway |

---

## 3. Free today

Reachable through the manager as it stands, with **zero new gateway code**, proven by driving the real production image (`ghcr.io/anomalyco/opencode:latest`, 1.18.25) started exactly as `run_container` starts it (`code-agent-manager.py:797-828`), with a chat volume rendered exactly as `render_chat_config` renders it (`:889-903`). Draw these into the mockup.

**`GET /chat/<id>/command` — the unified library.** One call returned 16 entries: built-in `init` and `review` (hints `['$ARGUMENTS']`), a phone-authored command (hints `['$1','$2']`, `agent=plan`), commands from the repo clone's `.opencode/commands/`, and 13 skills tagged `source="skill"`. This single list *is* Recipes + Skills + MCP-prompts with a badge. The MCP-prompt third (`command/index.ts:105-131`) is **unverified** — no reachable MCP server was available to exercise it.

**`GET /chat/<id>/skill`.** Discovery proven from four sources: built-in, the repo clone's `.opencode/skills/`, `$HOME/.config/opencode/skills/`, and `skills.paths` dirs. Note the reload asymmetry, which nobody would guess: **global-HOME skills rescan live per request**, but project-scoped skills, markdown commands and markdown agents are cached and need an instance disposal. `POST /global/dispose` is sufficient for those; a *changing* `PATCH /global/config` also works. A no-change PATCH does nothing (measured — `updateGlobal` only invalidates `if (changed)`, `config/config.ts:658`).

**`GET /chat/<id>/agent` — already spoken.** `lib.rs:1407`. A phone-authored subagent appeared in it immediately after a PATCH; `agent.<name>.disable: true` removed one from the list entirely.

**`PATCH /chat/<id>/global/config` — the whole authoring story in one call.** Measured: one body carrying an agent, a command, two MCP servers and `skills.urls` returned 200, landed in `<chat>/home/.config/opencode/opencode.json` preserving every pre-existing key, took effect with **no restart**, and survived `docker stop` + `docker start`. That path *is* `Global.Path.config` (`packages/core/src/global.ts:3,13` + `HOME=/chat/home` at `Containerfile:27`), which is precisely the file `render_chat_config` writes.

Three sharp edges on it, all measured:
- **Objects cannot be deleted.** `{"command":{"x":null}}` → 400 `Expected object, got null`. Rejected by the schema before `mergeDeep` is reached, so no silent corruption either.
- **Arrays are replaced wholesale**, not concatenated — so `instructions`, `skills.paths`, `skills.urls` and `plugin` *are* fully editable, including emptying. (`config.ts:646` uses plain `mergeDeep`; the dedupe-concat at `:42` applies at load, not at update.)
- **`disable: true` / `enabled: false` are the real delete verbs** for agents and MCP servers.

**Skill registry — the highest value-per-line finding in this document.** One line, `{"skills":{"urls":["http://..."]}}`, installed all twelve of `personal-ai-setup/config/skills/` (`ci-lint-test`, `clean-plan`, `code-review`, `connect-service`, `deep-research`, `looping-code-review`, `looping-plan-review`, `mr-review`, `plan-review`, `pre-mr-checklist`, `refactor-planner`, `ship`) into a live code chat — multi-file (`code-review` brought `SKILL.md` + `checklists.md` + `examples.md`), hot, cached on the chat volume under `~/.cache/opencode/skills`, surviving spin-down. Today those skills are deployed only to the VPS host's `~/.agents/skills/` and never reach a code chat. No gateway code, no client code — a config line and an `index.json`.

**`GET /api/permission/saved` / `DELETE /api/permission/saved/:id`** — 200 and 204 through the manager.

**Two things the client must never do.**
1. **Never call `PATCH /chat/<id>/config`.** Measured: 200, with a cheerful echo of the body — and it writes `<workspace>/config.json`, a file the project loader never reads (`config/config.ts:624-631` writes it; `config/paths.ts:17` only targets `opencode.json`/`opencode.jsonc`), leaving `?? config.json` in the user's clone. A lie plus an untracked file.
2. **Never treat 404 as "route absent."** Unknown paths hit the catch-all UI route and return **200 `text/html`**. Measured: `/schedule`, `/cron`, `/recipe` and `/definitely/not/a/route` all answered 200. This makes the doc comment on `CodeClient::models` (`crates/opencode-client/src/lib.rs:1363`) wrong about its own mechanism — the method survives only because it branches on decode success, not status, and on the shipping build both `/config/providers` and `/provider` answer 200 JSON anyway, so the fallback never fires. **Worth a one-line comment fix.**

---

## 4. Cheap, and worth it

### 4a. A shared or per-repo config overlay (the only new gateway route the whole library needs)

`render_chat_config` is defined at `code-agent-manager.py:889` and called from **exactly one place** — line 992, inside `create_chat`. There is no other write path into `<chat>/home/.config/opencode/opencode.json` and no overlay. So a command or MCP server the phone authors in chat A **does not exist in chat B on the same repo**, and dies when chat A is purged.

The work: a shared (and/or per-repo) JSON fragment on the manager's disk, merged into `render_chat_config`, plus a route to edit it. That is the difference between "a per-chat scratchpad" and "a library."

Two seeding paths already work and need **no new route at all**, and may be enough:
- Point `skills.urls` at an `index.json` the manager serves out of `personal-ai-setup/config/skills/` (measured, §3).
- Write files into a **stopped** chat's volume before wake — measured: `docker stop`, drop `<chat>/home/.config/opencode/command/gateway-seeded.md`, add an `agent` key to the volume's `opencode.json`, `docker start` — and both appeared. Against a *running* chat the seeder must nudge (`POST /global/dispose` for `.md` files; a changing `PATCH /global/config` for the JSON).

### 4b. Close the repo-override hole before any Code-side library ships

**This is a security finding, not a design note.** Merge order in `config/config.ts` is global (`:398-399`), then project files (`:406-410`), then `.opencode` dirs (`:424-433`), all via `mergeDeep` — **later wins**. Measured: with our rendered template on the volume saying `git push*: ask`, `external_directory: deny`, `share: disabled`, a `.opencode/opencode.json` **committed to the repo clone** flipped all three to `allow`, `allow` and `auto`. That is the phone's push-approval prompt silenced, the external-directory boundary opened, and transcripts re-enabled for sst's public share backend — by a file in the repo. The same file also contributed agents, commands and MCP servers.

Two mitigations, both measured working, both one line in code we already own:
- **Managed config dir** — merges *after* project config (`config/config.ts:516-522`); on Linux `/etc/opencode` (`packages/opencode/src/config/managed.ts:20-31`). One `COPY` line in the Containerfile. With only that dir populated, the effective config snapped back to our template's values despite the repo file still saying otherwise.
- **`OPENCODE_PERMISSION` env var** — merged last for `permission` (`config/config.ts:545-551`), and `run_container` already sets env (`code-agent-manager.py:813-819`).

This matters *now* because "the library lives in the repo" and "the repo can edit the sandbox" are the same mechanism. A Code-side library page that proudly shows `.opencode/` contents is also the page that should show what that directory did to the permission rules.

### 4c. A busy guard, if anything writes config

**Nothing anywhere has one.** On our side, `chat_busy` (`code-agent-manager.py:1120`) is referenced exactly once, from `reaper_loop` (`:1157`); `route_wake_or_stop` (`:1386`) calls `engine("stop", ...)` with no state check, so an explicit stop kills a running turn. On OpenCode's side, `disposeAll` checks nothing about running turns and `PATCH /global/config` forks disposal unconditionally when changed (`.../httpapi/handlers/global.ts:86-90`). Note that session-level `shell` has `SessionError.mapBusy` while `command` does not (`handlers/session.ts:331-346`) — so OpenCode knows what busy means, it just does not consult it on dispose. **Unverified:** whether a dispose actually aborts an in-flight turn; what is proven is only that no code path checks.

### 4d. A scheduler, if one is wanted — and it is ours

OpenCode has none. Verified three ways: source (`grep -riE '\bcron\b|scheduler'` over `packages/opencode/src` and `packages/core/src` returns only `scheduleRows`, a TUI row-render debouncer in `cli/cmd/run/footer.prompt.tsx`); docs (only `packages/web/src/content/docs/github.mdx:126`, GitHub Actions cron, and `ecosystem.mdx:45`, a third-party launchd/systemd plugin); and live (all four probe paths returned the 200-HTML catch-all).

If recurring code work is wanted, the honest designs, in order of how little new trust they need:
1. **GitHub Actions cron** — upstream's own answer. Runs in CI under the repo's credentials, and produces a branch/PR that `GET /api/chats/{id}/pulls` already shows.
2. **A gateway-side timer** — `POST /api/chats` (fresh container, fresh clone, allowlist enforced) then `POST /chat/<id>/session/<sid>/prompt_async`. The manager already has the daemon (`reaper_loop`), the index and wake. Note it fights `MAX_ACTIVE` and `IDLE_SECONDS`.
3. **Never: teaching goose's scheduler to `cd` into a repo.** `ScheduledJob` has no working-directory field (`goose/crates/goose/src/scheduler.rs:216-236`); `execute_job` uses `std::env::current_dir()` (`:1044`), pinned by `WorkingDirectory=/home/agent` in `scripts/vps/systemd/goose-serve.service:30`; and it runs with `GooseMode::Auto` — tool approval off (`:1026-1068`). A scheduled recipe doing code work would run unapproved, uncontained code with the brain's full secret store and every personal-data connector attached, on a timer, with no notification. It bypasses the allowlist, the container, `external_directory: deny` and the push prompt by construction. It looks like a shortcut and it is a hole.

---

## 5. Not worth it, or impossible

Each of these exists in OpenCode and does not survive our architecture. This section is here so the question is not re-asked in a month.

**PTY / terminal — the one true structural block.** Measured on both sides: `POST /chat/<id>/pty` → 200 with a real `/bin/zsh` pty object, but `GET /chat/<id>/pty/<id>/connect` with upgrade headers → **400 through the manager, 101 Switching Protocols sent directly to the container**. The route works; the gateway is the wall. `proxy()` strips `upgrade`/`connection` as hop headers and forwards via `http.client` (`code-agent-manager.py:1466-1510`), which cannot carry a 101. A terminal you can create and never attach to. Fixing it means a genuinely WebSocket-capable proxy.

**Local (stdio) MCP servers.** Measured against the real image: `node`, `npx`, `bun`, `python3`, `python`, `uv`, `uvx`, `deno` and `pip` are all **absent** — the image is Alpine plus the static binary plus `git openssh-client github-cli` (`Containerfile:18`). Every canonical `["npx","-y","@modelcontextprotocol/server-*"]` fails, and the failure surfaces exactly as an Extensions screen would render it: `{"local-probe":{"status":"failed","error":"Executable not found in $PATH: \"npx\""}}`. It is policy rather than physics — `Containerfile:30-33` documents the extension, and a repo can install toolchains via its allowlist `setup` field — but **JS/TS stdio MCP servers are viable today** — `opencode` is a Bun executable and injects `BUN_BE_BUN=1`, so `opencode x -y @modelcontextprotocol/server-…` reaches `connected` in the real image, with the install cache landing in the persisted volume. CPython servers (`uvx …`) still need a runtime added, and that is fine, since remote is the shape a phone wants anyway.

**`POST /mcp` as a durable add.** It is a **try-it button only**. Measured twice: the server appeared in `GET /mcp` and wrote **zero bytes** to the config file (`MCP.add` mutates `InstanceState` only, `packages/opencode/src/mcp/index.ts:641-659`), then vanished at the next reload — a lifetime shorter than one idle spin-down. Any UI that offers "add an extension" must write through `PATCH /global/config`.

**MCP OAuth.** The routes exist (`groups/mcp.ts:34-36`) but the registered redirect is a localhost HTTP server *inside the container*, which the phone's browser cannot reach. Same dead end `src/extensions.rs:38-42` already documents for goose. A bearer token works; OAuth does not. **Unverified** — no reachable MCP server was available, so the connect and OAuth legs were never exercised; only the `disabled` and `failed` statuses were observed.

**Themes and keybinds.** TUI-only (`tui.json`, `OPENCODE_TUI_CONFIG`), and we run `opencode serve` headless. Worse than inert: measured, `POST /chat/<id>/tui/open-themes` returns **200 `true`** with no TUI attached. Any UI built on a `/tui/*` call shows a green tick for something that never happened.

**Plugins.** `.opencode/plugins/*.{js,ts}` or npm packages named in config, loaded at startup. There is **no plugin list/install/uninstall HTTP route anywhere** in the httpapi groups. They execute arbitrary JS in the container, and egress is already an accepted-risk exfil path (`personal-ai-setup/docs/code-agents.md`). Exclude.

**Share.** Deliberately off, and it takes effect: `POST /chat/<id>/session/<sid>/share` → 500, container log `error="Error: Sharing is disabled in configuration"`. A privacy decision (`config/code-agents/opencode.json`), not a gap.

**References.** Blocked by our own `external_directory: {"*": "deny"}`, which is exactly the boundary references are designed to punch through — and a chat container sees only its own volume, so there is nothing outside `/chat/workspace` to reference except a git URL.

**Scheduler.** Nothing to reach. See §4d.

**Structured output / `response.json_schema`, `retry`, `activities`.** goose recipes have them (`goose/crates/goose/src/recipe/mod.rs:41-84`); OpenCode's `ConfigCommandV1.Info` is only `{template, description, agent, model, variant, subtask}` (`packages/core/src/v1/config/command.ts:5-12`). Recipes-only, permanently.

---

## 6. The symmetry question

**Deliberately differ, and by a lot.** The two halves should not mirror each other, for three independent reasons.

**Reason one: OpenCode keeps one namespace where goose keeps four.** goose needs four screens because it has four protocols — `skills_list` (`crates/goose-acp-client/src/goose/skills.rs:245`), `recipes_list` (`recipes.rs:388`), `schedules_list` (`scheduler.rs:294`), `config_extensions_list` (`extensions.rs:496`) — and those four are not even independent: `recipes_schedule` (`recipes.rs:448`) means Recipes and Scheduler are one loop, which is why `src/scheduler.rs:9-12` points its empty state at Recipes. OpenCode's `GET /command` is already the union of all the things goose splits (`command/index.ts:73-152`). Building four Code screens would be inventing a taxonomy the server does not have.

**Reason two: the Code library is per-repo and per-chat, so it is a property of a chat, not a sibling of Recipes.** goose's library is one global brain-wide thing. Ours is `.opencode/skills/`, `.opencode/commands/`, `.opencode/agents/` and `AGENTS.md` in the clone at `/chat/workspace`, plus whatever the chat's own `$HOME` config carries. Those are already live in every code chat today — version-controlled, per-repo, and invisible from the phone. The natural framing is **"what this repo's agent can do"**, which is a question you ask *from inside a chat*, not from a top-level drawer item.

**Reason three: both shipped products the design is drawing from answer it the same way, and neither ships a Code-side library page.** Claude desktop has the exact split being designed — a Home/Code segmented control — and puts Skills, Connectors, Plugins and Memory in **one global settings modal shared by both halves**; the segmented control changes what work you are looking at, not which library you are configuring. OpenCode's own desktop app has no library UI at all: `packages/app/src/components/settings-v2/` contains only `general | models | providers | servers`, and skills surface inline in the composer as `/`-command entries with a badge. goose desktop is the only one with a library sidebar, and goose desktop has **no code plane** — grepping every `.tsx` under `ui/desktop/src/components/` for pull request, diff or branch returns one hit, a git-branch *icon*. Its sidebar is evidence about what a Chat-only app looks like, not about what a Code half should contain.

**What this means for the sidebar, concretely.** Today's drawer is Chats, Code, Recipes, Skills, Scheduler, Extensions, Settings (`src/nav.rs:219,254,292,324,353,386,418`). Do **not** add a Code-side peer to Recipes. Add one destination *inside* Code, exactly the way `CodeScreen::Diff` and `CodeScreen::Pulls` already sit as details under it (`src/nav.rs:270-286`): a per-chat **Library** inspector showing the unified `GET /command` list with `source` badges, the skills, the agents, MCP status, and — per §4b — what the repo's own `.opencode/` did to the permission rules. Read-first. Authoring a command is a coding task the agent already does better than a phone form, and an OpenCode command is a file in the repo, so "create one" is `/init`-shaped work, not UI work.

One asymmetry is worth stating out loud in the design: **the Chat half's Skills page is read-only in goose too** — `ui/desktop/src/components/skills/SkillsView.tsx:210-219` renders "Add Skill" with `hidden` and `title="Coming soon"`. The largest library page in the app being copied is a GET and a search box. That is the right ceiling for the Code side as well.

---

## 7. What would change this answer

**It rests on these assumptions. Each is falsifiable, and here is what would falsify it.**

1. **The gateway stays a transparent wildcard proxy.** Everything "free today" depends on `ROUTE_CHAT` (`code-agent-manager.py:1263`) being matched first with no path or method allowlist. If a future hardening pass adds an allowlist — which would be a defensible thing to do — every item in §3 becomes gateway work. **If that pass happens, it should allowlist `/command`, `/skill`, `/agent`, `/mcp`, `/global/config` and `/api/permission/saved` explicitly, and it should blacklist `/config` (§3) and `/tui/*` (§5).**

2. **The production image tag floats.** `latest` is 20 patch releases ahead of the source tree everyone cites. A future release could move `/command`'s shape, split the `source` discriminator, or add a scheduler. Pinning the tag (`Containerfile:12-15` already flags it) converts this from a standing risk to a deliberate upgrade.

3. **`PATCH /global/config` remains additive-with-no-delete.** If upstream adds a real delete verb, the "disable: true is the delete" workaround in §3 becomes unnecessary. If upstream instead makes it strict-replace rather than merge, a naive client PATCH would **destroy** the rendered security template. Anything writing that route should send the full object it intends, not a fragment, if that ever changes.

4. **`GET /command`'s MCP-prompt branch is unverified.** `command/index.ts:105-131` was read, not run — no reachable MCP server existed during measurement. If those entries turn out to have a different shape or to be lazy/slow (the template is a promise, `:109`), the "one list, three sources" story needs a loading state for the MCP third.

5. **`POST /session/:id/command` is synchronous.** Read-verified (`handlers/session.ts:331-339` awaits the turn, unlike `promptAsync` at `:325-329` which forks and returns 204) and measured indirectly via the sibling `shell` route (`sleep 8` blocked for 8.076s). A real command turn was **never run** — no provider credential was available. The ceilings are real regardless: client timeout 150s (`crates/opencode-client/src/lib.rs:989`), gateway response budget 600s, connect 20s (`code-agent-manager.py:1488-1494`). **Any Run button must fire-and-forget and let the event stream be authoritative, exactly as `src/scheduler.rs:26-30` already does for `run-now`.** Also note `POST /session/:id/shell` requires a mandatory `agent` key — `{"command": "..."}` alone returns 400 `Missing key at ["agent"]`.

6. **Nobody wants a Code-side scheduler badly enough to build one in the gateway.** If that changes, §4d is the design, and the goose scheduler is not it.

7. **The `/api/integration/*` and `/api/credential/*` v2 surface was never exercised.** `packages/protocol/src/api.ts` mounts a discoverable third-party integration catalogue with key and OAuth connect flows. It is **entirely unverified** — no probe touched it — and it is the one plausible candidate that could displace MCP as the Extensions analogue if it turns out to be real and headless-friendly. Worth one measurement before the design is frozen.

8. **The scope inversion holds.** `HOME=/chat/home` (`Containerfile:27`) means OpenCode's "global" is our "per-chat." Every doc page that says a feature is global-only is, for us, describing something per-chat — which is the granularity a Code library wants. If the container layout ever changes to share a `$HOME` across chats, the multi-tenancy analysis in this document has to be redone from the start.