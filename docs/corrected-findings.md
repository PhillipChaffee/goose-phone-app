<!-- Corrections pass. Sixteen negative claims from docs/shared-artifacts.md and
docs/code-plane-library.md were attacked capability-first; two of the sixteen
were known-false calibrators. This document is the result, and it supersedes
the field-by-field comparisons in both of those files. Every claim carries how
it was established. A dagger (†) marks the ones re-run or re-read during THIS
pass rather than inherited from the refutation pass — including one live
measurement (goose's shipped `computercontroller` tool list) that contradicts
both the checkout and one of the attackers. Static line numbers are against
OpenCode 1.18.5 (commit 5e2a6257b2) and goose 1.46.0; production OpenCode is
1.18.25 and behaviour is authoritative over the checkout. -->

# Corrected findings: goose and OpenCode, capability-first

*Replaces the field-by-field comparison sections of `docs/shared-artifacts.md` and `docs/code-plane-library.md`. §7 is the explicit change list for those two files.*

**How to read the tags.** **[measured]** — something was executed and its output read. **[read-only]** — established by reading source on disk. **[unverifiable-here]** — not settleable with what is on this machine; what would settle it is named. A **†** means it was re-run or re-read during this correction pass rather than inherited.

---

## 1. Calibration result

**The calibration passed. No attacker reported either known-false claim as surviving, so no verdict is discarded on calibration grounds.** **[measured]**

- **C01** (OpenCode has no MCP tool allowlist) — reported `refuted=true`, with the correct mechanism named: MCP tools are flattened to `<server>_<tool>` and become legal keys in the `permission` table.
- **C02** (OpenCode config can only hold literal secrets) — reported `refuted=true`, with the correct mechanism named: `{env:VAR}` / `{file:path}` substitution over the raw config text.

Four qualifications, because a passed calibration is worth less than it looks:

**(a) C01 was a weak calibrator.** The operating instruction handed to the refutation pass named the answer verbatim — `Permission.visibleTools`, `tool/registry.ts:281`, "arbitrary keys are legal (StructWithRest with a Record rest)". An attacker that returned `refuted` on C01 may only have demonstrated that it read its own brief. **C02 is the load-bearing calibrator**: its answer (`config/variable.ts:33`) was not in that preamble, and the attacker found it, proved it on source, on the shipped binary, and by running a probe config with the secret deleted from the process environment. That is a real pass. **[read-only]**, on the inference about what the attackers were shown; **[measured]** on the C02 evidence itself.

**(b) The calibration certifies the attackers who ran C01 and C02, and only those.** Thirteen of the sixteen claims got a single pass. Where the attacker for a given claim was a different agent, the calibration says nothing about it directly.

**(c) Where a second pass ran, it caught real method defects in the first — twice, without flipping a verdict.** This is the strongest available evidence that the method works. **[measured]**
- **C08**: the first pass measured with `opencode serve --pure`, which disables the plugin subsystem (`src/index.ts:62-65`, `plugin/index.ts:177`) — i.e. it measured in the one mode where one of the three durable MCP-add channels is switched off. The second pass re-ran without it, and on production 1.18.25 rather than 1.18.20. Verdict unchanged, evidence base repaired.
- **C07**: the first pass concluded "1.18.25 dropped `slash: true`" from a `curl /skill` that serializes a four-field schema with no `slash` field — a negative from an endpoint that could not have shown a positive. Verdict unchanged, one supporting fact withdrawn.

**(d) One attacker made the exact error the exercise exists to correct, inside a correct verdict.** C04's attacker asserted that goose's `computercontroller` extension ships `xlsx_tool / docx_tool / pdf_tool / computer_control` and that "the old `web_scrape` is gone", citing the checkout. **I ran the shipped binary. It is not gone.** **[measured]†**

```
$ printf '…initialize…tools/list…' | goose mcp computercontroller     # /opt/homebrew/bin/goose, 1.46.0
['automation_script', 'cache', 'computer_control', 'docx_tool',
 'pdf_tool', 'web_scrape', 'xlsx_tool']

web_scrape :: "Fetch and save content from a web page. The content can be saved as:
               - text (for HTML pages) - json (for API responses) - binary …"
automation_script :: "Create and run Shell, Ruby, or AppleScript (via osascript) scripts."
```

The checkout removed them in `0d0d130d9 refactor(computercontroller): remove automation_script, web_scrape, and cache tools (#11198)` — **after** the 1.46.0 build that is installed. **[measured]†** So a URL fetcher and a second shell exist in the deployed goose and in no line of its source tree. C04's headline verdict (refuted) is right; one of its supporting facts is the field-shaped error in miniature, one layer down.

**Net: treat all sixteen verdicts as usable. Treat the thirteen single-pass verdicts as strong-but-unreplicated, and treat any sub-claim of the form "grep says it is not in the tree" as needing an artifact check before it goes in a document.**

---

## 2. What was wrong, grouped by the kind of mistake

Nine of the sixteen claims were refuted. The pattern matters more than the list, so they are grouped by the error, not by the claim.

### A. A name is not a capability

*Searched for a field, route, config key or binary by name; found no match; reported the capability absent.* Seven of the nine refutations are this, in different disguises.

| Claim | What was searched for | Where the capability actually lives |
|---|---|---|
| **C01** | `available_tools` on `config/mcp.ts` | The `permission` table. `ConfigPermissionV1` is `Schema.StructWithRest(…, [Schema.Record(Schema.String, Rule)])` — arbitrary glob-matched keys — and MCP tools enter that keyspace as `sanitize(server)+"_"+sanitize(tool)` (`mcp/catalog.ts:119`). `Permission.disabled` removes hard-denied tools from the schema sent to the model (`permission/index.ts:204-214`). **[read-only]†** confirmed at both sites; **[measured]** on the shipped binary and via `opencode debug agent build` under `OPENCODE_PERMISSION={"*":"deny",…}` |
| **C02** | `env_keys` on the MCP schema | `ConfigVariable.substitute` rewrites `{env:VAR}` / `{file:path}` in the **raw config text before parsing** (`config/variable.ts:33-38`, called at `config/config.ts:219-227`), so every string in the document is a secret-bearing field — `command` argv, `url`, `headers`, `oauth.clientSecret`, provider `apiKey`. Plus a credential-store path (`auth.json`, 0600) whose names resolve ahead of the process env. **[read-only]†** on the substitution site; **[measured]** on the four-position probe and the env-deleted auth-store run |
| **C05** | `cron` in `packages/opencode/src` | `schedule` is one of six supported GitHub Action events with a dedicated code path — no actor, `schedule-*` branch prefix, PR-only output, `prompt` required (`cli/cmd/github.handler.ts:149, 402, 421, 528, 728`), documented with `- cron: "0 9 * * 1"` (`web/…/github.mdx:114,126`), and present in the shipped binary. Plus eight in-process recurring timers. **[measured]** |
| **C06** (response-schema limb) | `response.json_schema` on `ConfigCommandV1.Info` | `format: {type:"json_schema", schema, retryCount}` on the **session prompt**, which registers a synthetic `StructuredOutput` tool whose `inputSchema` is the caller's JSON Schema, forces `toolChoice:"required"`, and returns the validated object on `assistant.structured`. **[measured]** — a live 1.18.20 server returned `structured: {"answer": 4, "explanation": "2 + 2 = 4"}` |
| **C09** | a `/plugin` HTTP route | `GET /config` / `GET /global/config` list the `plugin[]` array; `PATCH /global/config` adds or removes, and the server npm-installs on the next instance build. **[measured]** — `PATCH` with `["is-odd@3.0.1"]` caused `~/.cache/opencode/packages/is-odd@3.0.1/node_modules/is-odd` to appear, and a plugin's tool entered `GET /experimental/tool/ids` (14→15) and left again on `{"plugin":[]}` |
| **C10** | nine runtime binaries on `PATH` | The tenth binary is a runtime. `opencode` is a Bun 1.3.14 single-file executable, and OpenCode injects `BUN_BE_BUN: "1"` for any local MCP server whose `argv[0]` is `opencode` (`mcp/index.ts:354`) — a purpose-built feature. **[measured]** — two off-the-shelf npm stdio MCP servers reached `connected` inside the real code-agent image with node/npx/bun/python3/uv/uvx/deno/pip all absent |
| **C15** (method half) | a method allowlist as a config field | It is the set of `do_*` methods on the handler class (`code-agent-manager.py:1543-1556`), enforced by `BaseHTTPRequestHandler` via `hasattr(self, 'do_'+command)`. **[measured]** — HEAD/OPTIONS/TRACE/PROPFIND/FOO all got 501 with a single `Server` header, i.e. refused at the gateway before authentication and without contacting the container |

**The generalisation:** a capability can be implemented as a *field*, a *route*, a *key in a different table*, a *text pass over the document*, a *decorator around a list*, an *executable with another name*, or *the shape of a class*. Only the first is findable by grepping the other system's vocabulary.

### B. The checkout is not the artifact

*Read source, reported behaviour.*

- **C03** — `DeveloperClient::get_tools()` really does return exactly `write, edit, shell, tree, read_image` (`developer/mod.rs:108-179`, asserted at `:273`). At runtime that extension can advertise six: `AcpTools` inserts a `read` tool at the front of the list whenever the ACP client advertises `fs.readTextFile` (`acp/fs.rs:107` `Tool::new("read", "Read a text file from disk.", …)`, injected at `acp/fs.rs:406-408`, gated at `acp/server.rs:866`, registered under the name `"developer"` at `:882`). **[read-only]†** on all four sites; **[measured]** by the attacker driving the installed binary over ACP with the boolean on and off — byte-identical tool lists differing by exactly `read`, and a forced call producing an agent→client `fs/read_text_file` and the file's text in the next provider request, with no shell.
- **C04's computercontroller sub-claim** — see §1(d). **[measured]†**
- The general form: both shipped documents carry the caveat "line numbers are 1.18.5, behaviour was executed against 1.18.25" and then, in specific paragraphs, reason from the tree anyway.

### C. The deployment is not the system

*Attributed a local configuration choice to the engine.*

- **C03** — "goose has no file-read tool" is a fact about **this client**. `crates/goose-acp-client/src/client.rs:182` declares `"fs": {"readTextFile": false, "writeTextFile": false}`. **[read-only]†** (See §6 for why that is the right setting and why "flip the boolean" — the attacker's own restatement — is wrong for this deployment.)
- **C04** — tavily is not a goose extension. `strings` on the shipped binary returns nothing for it; goose's builtin set is `autovisualiser, computercontroller, memory, tutorial` (`goose-mcp/src/lib.rs:57-64`). It is one disabled entry in `personal-ai-setup/config/goose/config.yaml:305-318`, alongside a second disabled web route (`playwright`, `:285-297`). "Ships disabled" is a fact about the owner's template. **[measured]** on the binary, **[read-only]** on the config.
- **C16** — "zero of ten are both portable and desirable" is false by two. `skills` ports at zero cost (both engines read `~/.agents/skills`; both listers enumerate the same eleven directories **[measured]**). `workspace-mcp` ports as a plain stdio MCP server: goose's exact pinned command line in an `opencode.json` reported `✓ connected`, and a raw stdio `tools/list` against that command returned exactly the ten tools goose's `available_tools` pins, character for character **[measured]**.

### D. The wrong unit of comparison

*Compared two things that are not each other's peers, then reported the mismatch as an absence.*

- **C06** — a goose recipe bundles procedure + run configuration + typed return. An OpenCode *command* is only the first two-thirds of that, and the typed return lives on the API caller. Comparing `Recipe` to `ConfigCommandV1.Info` guarantees a false negative. (Also: the live `Command.Info` is eight fields, not six — it adds `name`, `source`, `hints` — and commands are synthesised from config markdown, MCP prompts **and** skills alike, `command/index.ts:22-32, 90-152`. **[read-only]†**)
- **C08** — compared the route *named* `mcp` against goose's config-persisting extension add. goose draws the identical line in its own doc comments: `_goose/unstable/session/extensions/add` ("Add an extension to an active session") vs `_goose/unstable/config/extensions/add` ("Persist a new extension to the user's global goose config") — `goose-sdk-types/src/custom_requests.rs:30-32, 449-451`. `POST /mcp` is the peer of the *session* verb. **[read-only]**
- **C11** — compared goose's agent frontmatter to OpenCode's. goose puts subagent tool scoping on the **caller** (`delegate(extensions: […])`, empty array = no tools, `summon.rs:606-613, 1560-1580`) and in a **sibling file type** (`.agents/checks/*.md` for `goose review`, which carries `model`, `turn-limit`, `tools` and `severity-default` — `checks/mod.rs:27-37`). And `model:` on an agent file is **not** discarded: `build_recipe_from_agent` re-parses the frontmatter and turns it into `Settings { goose_model }` (`summon.rs:1504-1548`), applied with precedence delegate-arg > agent-file > `GOOSE_SUBAGENT_MODEL` > session model (`summon.rs:1626-1634`). **[read-only]†** — I read both sites. What *is* discarded is everything else: `parse_agent_content` builds the listing entry with `properties: std::collections::HashMap::new()` (`summon.rs:156`) **[read-only]†**, so a `permission:` key survives on disk and reaches nothing.

### E. Probe hygiene: the flag, the schema, the receipt

*The measurement was real and the conclusion did not follow from it.*

- **C08 first pass** — `--pure` disabled the plugin channel being searched for. **A probe's flags are part of its claim.**
- **C07 first pass** — inferred a missing frontmatter key from a route whose response schema has no such field. **A negative from an endpoint is bounded by the endpoint's schema.**
- **C12** — "the `/tui/*` routes are inert; they return success for operations that cannot have happened." They are not inert. Every fire-and-forget route publishes a real `tui.*` event onto the general instance bus (`event-v2-bridge.ts:19-44`), delivered over `GET /event` to any subscriber. **[measured]** — a plain curl SSE client on a server with no TUI received `tui.command.execute {"command":"session.list"}` and `tui.toast.show {"message":"c12 probe",…}`. Two of the thirteen routes do not return blanket success either: `/tui/select-session` validates (400 on a malformed id, 404 on an absent session) and `GET /tui/control/next` never returns headless at all (blocks on an AsyncQueue). **The reasoning error was treating "no delivery receipt" as "no delivery."**

---

## 3. What was true but misleading

Six claims hold literally and mislead as written. Each is given with its honest restatement.

**3.1 — `POST /session/:id/command` runs embedded shell and file references with no permission evaluation.** (C13) **[measured]**, twice, the second time on the production artifact: `ghcr.io/anomalyco/opencode:latest` (digest `sha256:45174ca0…`, `--version` = 1.18.25) with `permission: {"*":"deny","bash":"deny","read":"deny","skill":"deny","external_directory":"deny"}` wrote the probe file and inlined `/etc/hostname` from outside the worktree, with `grep -ciE "asking|evaluated"` over the container log returning zero real hits.

> **Restate as:** *Permissions gate the model, not the API caller.* The command route spawns the shell directly (`session/prompt.ts:1397-1408` — `Process.text([cmd], {shell, nothrow:true})`, no `ctx.ask`, not even the `shell.env` plugin hook) **[read-only]†** and performs `@file` reads with `ask: () => Effect.void` and `bypassCwdCheck: true` (`prompt.ts:822-826`). But this is not a hole in one route: `POST /session/:id/shell` is equally unpermissioned, and the neutered read sits in `createUserMessage`, which every user message with a file part goes through. The consequence to state is the general one — **anything that can authenticate to the HTTP API has unconditional shell and unconditional read outside the worktree**, which is precisely the boundary `config/code-agents/opencode.json:25-27` (`external_directory: {"*":"deny"}`) is relying on. The vector is "a repo-provided command *or* skill *or* MCP prompt template is a shell trigger"; `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS` removes only the `.claude/skills` half **[measured]**, and the identical attack shipped as `.opencode/command/foo.md` has no switch. The `command.execute.before` plugin hook fires *after* the expansion and cannot veto it **[measured]** — a plugin that throws produces HTTP 500 with the file already written. The asymmetry worth naming: the model-invoked `skill` tool *is* gated (`tool/skill.ts:26-32`), and `Command` builds its list from `skill.all()` rather than `skill.available()` (`command/index.ts:134`) **[read-only]†** — two doors to the same asset, one locked.

**3.2 — `POST /mcp` is not durable.** (C08) **[measured]** on 1.18.25 without `--pure`, with pre-existing `mcp` keys at both global and project scope.

> **Restate as:** it is a hot-add, not a save. Worse than "vanishes at the next reload": it is invisible to `GET /config` too, so a read-modify-write client silently drops it, and it is wiped by *any* config write from any client (`PATCH /config` and `PATCH /global/config` both measured), by `POST /instance/dispose`, and by restart — but **not** by an on-disk edit to `opencode.jsonc`, which triggers nothing until an API config call happens. And the durable capability is not missing: there are three shipped destinations, none named `mcp` — `PATCH /global/config` (writes the file *and* takes effect with no restart), the `opencode mcp add` CLI (writes, not live), and a plugin `config` hook that injects `mcp` entries at every start with zero config bytes **[measured]** — plus `OPENCODE_CONFIG` / `OPENCODE_CONFIG_CONTENT` at startup.

**3.3 — OpenCode has no scheduler.** (C05)

> **Restate as ownership of the clock:** OpenCode never holds a timer that fires user work — no `opencode schedule` command, no schedule key in `opencode.json`, no scheduling endpoint in the 162-path OpenAPI document, no timed hook in the plugin API, no scheduling tool. What it has is first-class support for being triggered by someone else's clock. So goose's scheduler *does* have a destination on the OpenCode side; it is an **inbound trigger, not a peer scheduler.**

**3.4 — The gateway forwards paths verbatim.** (C15)

> **Restate as:** no *path* allowlist inside the proxy prefix — `/chat/<known-id>/<anything>` is forwarded unnormalised (`--path-as-is /chat/<id>/../../api/health` reached the upstream as the literal string) **[measured]**. It is not an open pipe: requests are authenticated first and the client's own `Authorization` is stripped and replaced; only `/chat/…` proxies at all; the chat id must match `[a-zA-Z0-9-]+` and exist; the verb set is a compiled-in allowlist of GET/POST/PUT/PATCH/DELETE; and `upgrade`/`connection` are stripped as hop headers, which is *why* the PTY WebSocket cannot be reached.

**3.5 — OpenCode's skill loader keeps only `name` and `description`.** (C07 — survives; see §4) The misleading part is the consequence.

> **Restate as:** it is a **regression, not a boundary**. The original skills commit `8fe0715928` kept `license`, `compatibility` and `metadata`; `046e351140` deleted all three two days later; and both `packages/web/src/content/docs/skills.mdx` and the built-in `customize-opencode` skill still tell users those fields are recognised **[measured]** — the doc line was observed being served live from the 1.18.25 container. The upstream fix is three lines. And the *capability* metadata serves is not missing, it is declared in the body: skills are registered as commands with the body as `template` (`command/index.ts:134-149`) **[read-only]†**, and invoking one expands `$1..$N`, `$ARGUMENTS` and executes `` !`shell` `` **[measured]** — `first=alpha second=beta gamma delta` and `Shell: SUBSTITUTED-BY-SHELL`. What genuinely has no destination is narrower than "metadata": goose's *named* placeholders (`$pr`), and `argument-hint` (whose counterpart `Command.Info.hints` exists but is hard-coded `[]` for skill-sourced commands).

**3.6 — The `/tui/*` routes acknowledge operations that did not happen.** (C12 — the underlying observation, not the claim)

> **Restate as:** the boolean is a **publish receipt, not a delivery receipt**, and OpenCode exposes no way at all to detect an attached TUI (no subscriber count, no attachment flag anywhere in the tree). The sharpest genuine instance is `/tui/execute-command`: an unrecognised command name maps to `undefined` through the alias table and the server answers `200 true` while emitting a command-less event **[measured]**. And `open-themes` is not a no-op but a **mis-wire**: `handlers/tui.ts:51-54` publishes `session.list`, byte-identical to `open-sessions`, so it opens the sessions dialog — with a TUI attached or not. That is an upstream bug worth reporting, not a headless artefact.

---

## 4. What survived

| Claim | Verdict | Confidence | How |
|---|---|---|---|
| **C07** — OpenCode's skill loader keeps only `name`/`description` and discards every other frontmatter key | **Holds** | High — two independent passes, one against production bytes | **[measured]** `{name, description, location, content}` from live 1.18.25 with twelve extra keys declared; frontmatter also stripped from `content`; the write site read out of the shipped bundle: `j.skills[z.data.name]={name:…,description:…,location:J,content:z.content}` |
| **C08** — `POST /mcp` writes zero bytes and does not survive | **Holds** | High — two passes, second on 1.18.25 without `--pure` | **[measured]** `grep -rl` over an isolated config/data/state/project tree returned nothing; md5 of both config files unchanged; gone after restart |
| **C13** — the command route expands shell and file refs with no permission evaluation | **Holds** | High — reproduced on the production image | **[measured]** see §3.1 |

Plus these **surviving residues of refuted claims** — the parts that were right inside claims that were wrong overall. Each is a real asymmetry and should be carried forward in place of the claim it came from.

- **goose has no per-tool deny that reaches a subagent.** Subagents are pinned to `GooseMode::Auto` (`summon.rs:1291-1300, 1850-1859`), and `Auto` returns `InspectionAction::Allow` before `get_user_permission` is consulted (`permission_inspector.rs:157-161`), so `NeverAllow` never fires. Subagents are also constructed with an empty `HookManager` (`agents/agent.rs:427-433`), so `PreToolUse` policy hooks never run for them. The only deny plane that reaches a subagent is the global, LLM-judged, fail-open `~/.config/goose/adversary.md`. **[read-only]**
- **goose's agent files have no `permission:` key** and cannot bind their own permissions. **[read-only]†**
- **OpenCode has no `activities` equivalent, declarative or otherwise.** Searched: command schema, agent schema (unknown keys land in `options`, nothing consumes them as suggestions), skill schema, top-level config, integration prompts (OAuth form fields), TUI tips (hardcoded). **[read-only]**
- **OpenCode has no success-criteria retry.** `session/retry.ts` is provider rate-limit backoff. The one adjacent knob — `retryCount` on the json_schema format — defaults to 2 in the schema and is **never read by the runtime** (`prompt.ts:1310-1313` hardcodes `retries: 0`); verified absent from both the 1.18.5 source and the shipped 1.18.20 bundle. **[measured]**
- **OpenCode has no OS-keychain integration.** grep for keytar/libsecret/`security find-generic-password`/Credential Manager over `packages/` is empty. Its indirection targets are the process environment, a file on disk, and `auth.json` at 0600. "Resolve a named secret from the OS keyring" is genuinely goose-only. **[read-only]**
- **OpenCode's `permission.ask` plugin hook is inert.** Declared at `packages/plugin/src/index.ts:261`; no trigger site found in the 1.18.5 tree or the 1.18.20 binary — only `permission.asked` events. **[read-only]**
- **MCP OAuth is loopback-only on both sides, and goose additionally refuses URL-mode elicitation at the ACP bridge** (`acp/server/elicitation.rs:59-77`). The phone client's existing note (`src/extensions.rs:38-42`) is correct and now source-proven. **[read-only]**
- **goose has no path-boundary check anywhere in its tool layer** — no `.gooseignore`, no worktree containment. The container is the directory restriction. **[read-only]**
- **The PTY WebSocket is structurally unreachable through the gateway.** 400 through the manager, 101 direct to the container. **[measured]**

---

## 5. The corrected picture

Three capabilities, each stated as a problem first, then each system's *shape*, then the difference that actually matters, then the hazard.

### 5.1 Restriction — what may this tool call touch?

**The problem.** Between a model emitting a tool call and that call touching the world, something decides run / refuse / ask. The axes are: which tool, which arguments, which directory, which subagent, and at which of several moments the decision is made.

**OpenCode's shape: one ordered rule table, consulted at every gate.** A rule is `{permission, pattern, action}`, both fields globs, action ∈ allow|ask|deny, **last match wins**, default `ask` (`permission/index.ts:28-38`). The design move that makes it expressive: **every tool nominates its own most restrictable argument as the pattern.** **[read-only]†**

| tool | the resource it submits |
|---|---|
| `bash` | the source text of **each** command node, tree-sitter-parsed out of pipelines and `&&` chains (`tool/shell.ts:392-411`) |
| `read` / `write` / `edit` | path relative to worktree |
| `webfetch` | the URL |
| `task` | the subagent's name |
| `skill` | the skill's name |
| `external_directory` | `<dir>/*` |
| **any MCP tool** | **`"*"`** (`session/tools.ts:408`) |

A hard deny does not merely block the call — it **removes the tool from the schema sent to the model**, but only when the last matching rule has `pattern === "*"` **and** `action === "deny"` (`permission/index.ts:204-214`) **[read-only]†**. So `{"*":"deny", "X_a":"allow"}` is a true allowlist; `"ask"` leaves the tool advertised, and a path-scoped deny blocks the call without hiding the tool.

**goose's shape: a pipeline of inspectors under a master switch, over a surface composed at load time.** Four layers with no shared vocabulary: **composition** (`available_tools`, exact string match, empty = allow-all, enforced at listing `extension_manager.rs:1549` and again at dispatch `:1928-1941` **[read-only]†**); **consent** (`permission.yaml`, bare tool name → `always_allow|ask_before|never_allow`, `Vec::contains` on the exact string, one global file); **the master switch** (`GOOSE_MODE`); and **five inspectors** run in order with Deny winning. Plus **`PreToolUse` hooks** — external programs receiving the tool name, full arguments and working dir on stdin, exit 2 or `{"decision":"block"}` to deny — which is goose's real argument-level policy engine.

**The difference that matters.**

- **OpenCode is the only one of the two that can say *"this tool, with these arguments"* declaratively.** `{"bash": {"*":"allow","git push *":"ask","rm -rf *":"deny"}}` has no goose equivalent that is not a program.
- **goose is the only one that can restrict MCP tool *arguments* at all** — OpenCode's MCP tools ask with `patterns: ["*"]`, i.e. all-or-nothing; goose reaches them through `PreToolUse`, which sees the full argument object for any tool.
- **goose is the only one with OS-level confinement in-tree** (`goose run --container` runs every stdio *and builtin* extension as `docker exec -i`, which is also the only place either system gives an MCP server anything other than the full parent environment).
- **goose is the only one with egress observability** — `EgressInspector` extracts URLs, `git@host:`, `s3://`, registry targets from tool arguments, classifies direction, emits a tracing event, and **always returns `Allow`** with confidence 0.0. Telemetry, deliberately not enforcement.
- **neither has per-destination network policy that is usable today.** OpenCode's engine could express it — the URL is already the resource — but the v1 config schema types `webfetch` and `websearch` as bare `Action`, not `Rule` **[read-only]†** (confirmed at `packages/core/src/v1/config/permission.ts:28-29`), so `{"https://api.github.com/*":"allow","*":"deny"}` is rejected at parse **[measured]**.

**Hazards, all measured or read on disk, all live in this deployment.**

1. **A user wildcard defeats a mode.** OpenCode merges user config last and `evaluate` uses `findLast`, so a global `{"permission":{"*":"allow"}}` puts a trailing allow after plan mode's compiled-in `edit: deny`. **[measured]**
2. **`GOOSE_MODE=auto` silently overrides an explicit `never_allow`** — and `scripts/common/run-recipe.sh:91` exports it for every scheduled run. **[read-only]**
3. **A repo-committed `.opencode/opencode.json` overrides the container template** (later wins through `mergeDeep`) — measured flipping `git push*: ask` → `allow`, `external_directory: deny` → `allow`, `share: disabled` → `auto`. **[measured]**, already in `code-plane-library.md §4b`; it now compounds with §3.1, because the same repo file plus a repo command template is unpermissioned shell.
4. **`external_directory` fails in both directions at once.** Too strict for the read tool: the template's `{"*":"deny"}` revokes OpenCode's auto-grants for discovered skill directories, so `code-review`'s `checklists.md` is advertised in `<skill_files>` and denied to `read` **[measured]**. Entirely absent on the command path: the same key is bypassed outright by `bypassCwdCheck: true` **[measured]**.
5. **goose's egress inspector does not match its own URL-fetching tool.** `is_web_tool()` matches `web_fetch|fetch|browser_navigate|http_request` and does not match `read_image`. **[read-only]**

### 5.2 Packaging — how is reusable work carried?

**"Reusable work" is six separable problems:** procedure, standing context, run configuration, delegation, triggering, and distribution + shape of the result. **Neither system has one concept covering all six; each fuses a different subset into its headline artifact, and that is the whole story.**

**The one place they genuinely agree is `SKILL.md`** — same format, overlapping discovery roots, lazy-load-by-name, and both register a built-in that a disk skill may shadow. **[measured]** — `goose skills list` and `opencode debug skill` enumerate the same eleven directories out of `~/.agents/skills`, with zero adaptation. Even here they diverge on three axes: goose's `load_skill` takes arguments and supports named `$name` placeholders resolved against `metadata.arguments`; OpenCode's `skill` tool takes no arguments at all and supports only `$1..$N`/`$ARGUMENTS`; and OpenCode has a per-skill-name allow/ask/deny fence with globs while goose has none.

**goose's cut is *whole run* vs *paragraph*.** Its runtime union is `SourceType = {Skill, BuiltinSkill, Recipe, Subrecipe, Agent}` and the `summon` extension exposes exactly two verbs over it: `load` (read it into my context) and `delegate` (run it as a subagent with its own extensions, model, turn budget and working dir). **The artifact kind barely matters; the verb does.** A recipe can be read like a skill; an agent can be run like a recipe.

**OpenCode's cut is *what the human types* vs *what the model delegates to*.** `Command.Info` with `source: "command" | "mcp" | "skill"` is everything sayable with a slash; `Agent.Info` is everything delegatable. **[measured]** — `GET /command` on a scratch project returns config commands and every disk skill in one list.

**Three asymmetries that are load-bearing for the product:**

1. **Invoking the reusable thing does opposite things to the session.** goose `/deploy` **mutates the current session in place** — renders the recipe, attaches it to the session record, installs the `recipe__final_output` tool from `response.json_schema`, injects the prompt. OpenCode `/deploy` **forks** — the prompt becomes a `subtask` part run by that agent in its own session. *Reconfigures where you are* vs *goes somewhere else and reports back.*
2. **The tool surface is scoped at opposite layers.** goose **doesn't start it**: a recipe's `extensions:` list replaces the session's configured extensions, and `available_tools` narrows further, so `manage_event` is not in the process. OpenCode **doesn't show it**: MCP servers are global config and the agent's permission ruleset filters what it sees.
3. **Typed return exists in both, in different places.** goose: `response.json_schema` on the recipe → a `recipe__final_output` tool the model must call before finishing. **A recipe is a callable function with a typed return.** OpenCode: `format: {type:"json_schema", schema}` on the *prompt*, → a synthetic `StructuredOutput` tool with `toolChoice:"required"` → `assistant.structured`. **[measured]** Same capability; goose binds it to the artifact, OpenCode binds it to the call.

**Where they genuinely do not meet:** OpenCode has no `activities` and no success-criteria `retry` (§4). goose has no per-skill permission fence. Neither lets a skill file declare which engine it belongs to — OpenCode *documents* `compatibility` and drops it at parse **[measured]**; goose preserves a free-form `metadata` bag and reads only `argument-hint` and `arguments` from it.

**The fence that does exist is not on the skill.** It is (a) `permission.skill` in OpenCode — allow/ask/deny, glob-patterned, per-agent, enforced twice (denied skills are filtered out of `<available_skills>` before the system prompt is built, and the `skill` tool asks again at load) **[measured]** — a `build` agent and a `plan` agent were configured to allow and deny opposite skills and `opencode debug agent` resolved exactly that; (b) **placement**, in both engines, whose discovery roots are disjoint for their private directories (`.goose/skills` is invisible to OpenCode, `.opencode/skills` invisible to goose) **[measured]**; and (c) all-or-nothing kill switches (`OPENCODE_DISABLE_EXTERNAL_SKILLS` cut all eleven shared skills in one measured run; goose's `extensions.skills.enabled: false`).

**The summary sentence.** goose's recipe is OpenCode's agent and command fused into one file, with a cron and a return type bolted on; OpenCode's agent is goose's `settings` block promoted to a first-class, permission-scoped identity that a recipe's procedure never gets to be. **goose asks "what runs, and can I read it or delegate it?"; OpenCode asks "who is running, and what may they see?"**

### 5.3 Reach — how does each touch the outside world, and hold the keys?

**Secret injection into an MCP server.** goose: *named-key resolution against a secret store, fail-closed per extension.* `env_keys` → `config.get(key, true)` → uppercased env var, then keyring, then a 0600 file; a key that resolves to nothing **kills that extension** with a warning and the child is never spawned **[measured]**. OpenCode: *textual interpolation over the config document, fail-open to empty.* `{env:}` / `{file:}` reach `args`, `command`, `url`, `headers` — everywhere goose leaves literals — and a missing variable becomes `""` silently **[measured]**: the owner's real `TOGETHER_API_KEY` resolved to an empty string in a probe run, which in production means a 401 at the first model call rather than a startup error.

Two things that follow and were not in the shipped docs:
- **goose's `${VAR}` substitution is not uniform.** Applied at spawn to `streamable_http`'s `uri`/`headers`/`socket`; **not** applied to a `stdio` server's `cmd`, `args` or `cwd` — the child literally received `--token ${MY_TEST_SECRET}` **[measured]**. For a stdio server goose's only secret channel is the child's environment.
- **goose refuses to let an extension override 31 hijackable env vars** (`PATH`, `LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, `NODE_OPTIONS`, `PYTHONPATH`, …). OpenCode has no such list. **[read-only]**
- **Both leak the entire parent environment into a local MCP server** — measured 74 vars (OpenCode) and 73 (goose), both carrying an unrelated canary; goose's additionally carried `OPENAI_API_KEY`. goose's `--container` path is the only per-server isolation in either system.

**Fetching a URL from inside the model loop.** OpenCode ships `webfetch` (GET, 5 MB, HTML→markdown) and `websearch` (itself an MCP call out to `mcp.exa.ai` or `search.parallel.ai`), both permission-gated with the URL/query as the pattern — and both crippled by the v1 schema typing them as bare actions (§5.1).

**goose's web reach is three things the shipped docs miss.** **[measured]†** for the first, **[read-only]** for the rest:
1. `computercontroller`'s **`web_scrape`** and **`automation_script`** exist in the installed 1.46.0 binary and in no line of the checkout. If that extension is ever enabled without an `available_tools` list, the deployment gets an unrestricted URL fetcher and a second shell — on the same enum variant whose empty-means-allow-all polarity `check-security.sh:349-357` exists to catch.
2. **`read_image`** is compiled-in, default-enabled, and performs a reqwest GET at any model-supplied `http(s)` URL (`developer/image.rs:168-214`), annotated `open_world_hint: true` by a goose test literally named `read_image_annotations_reflect_network_access`. It decodes as png/jpeg/gif/webp, so it is a web fetch for pixels, not for pages — but it is real, ungated egress, and `config/goose/config.yaml:77-81` enables `developer` with no allowlist.
3. **`shell`**, which goose's own extension instructions recommend for "accessing web sites or APIs" and its egress inspector explicitly classifies as network egress.

**Credential store.** goose: keyring-first, degrading to a 0600 `secrets.yaml` — and it auto-degrades on any keyring error, which on a headless VPS is always. OpenCode: file-only, `auth.json` + `mcp-auth.json` at 0600, with a **whole-blob env override** (`OPENCODE_AUTH_CONTENT`) that is also what OpenCode's own workspace-spawn path uses — exporting *every* provider credential, with no selective export. In a container the two converge to the same thing.

**What the containerised code agent can actually reach.** No `--network` flag, so unrestricted egress at the network layer; `permission.bash` is `{"*":"allow","git push*":"ask"}`, so `curl`/`nc`/`ssh`/`pip install` are unprompted; `GH_TOKEN` is in the environment, so `curl -H "Authorization: bearer $GH_TOKEN"` reaches every repo the PAT can, with the push *prompt* bypassed entirely. Structurally out of reach regardless of config: anything inbound; any OAuth needing a browser on the agent's host; an OS keyring; the host filesystem outside `/chat`; another chat's state; per-MCP-server credential isolation; and per-destination network policy of any kind — that one has to come from outside both agents.

---

## 6. What this means for the product decision

**The earlier recommendation stands in shape. Three of its supporting arguments were wrong, two scope items flip from "impossible" to "available", and one new safety statement is required.**

### Stands, unchanged

- **One skills store.** Do not split `~/.agents/skills`. Strengthened, not weakened: both engines enumerate it identically **[measured]**, and the intersection of the two wire formats really is `{name, description, body, location}`.
- **One shared type for skills only; no unified command abstraction.** Still right, but for a *different reason* — not "OpenCode commands are impoverished" but "the two systems cut the space at different joints" (§5.2). A common type would still buy nothing.
- **The code library is one list with a source badge, inside a code chat, read-first.** Strengthened: after §3.1, read-first is a safety requirement rather than a taste. `GET /command` is genuinely the union of what goose splits four ways.
- **Do not merge the Extensions screens.** goose's extension list contains the shell; OpenCode's MCP list never will. Still true — but the *argument* changes (below).
- **The scheduler is ours to build, in the gateway.** Narrowed: OpenCode is a first-class cron *target*, and `code-plane-library.md §4d`'s option 1 (GitHub Actions cron) is stronger than it was written — `opencode github` has a dedicated `schedule` code path, so that is a supported product path, not a CI hack.
- **Do not build a plugin UI.** Reason changes from "impossible" to "possible and deliberately declined" — arbitrary in-process JS with `Bun.$` and the `Auth` service in scope is the widest single hole in either system.

### Changes

1. **The Extensions/MCP port is possible, and the code-plane Extensions screen becomes implementable rather than aspirational.** `available_tools` translates to `permission: {"X_*":"deny","X_a":"allow"}` and `env_keys` to `{env:VAR}`. The earlier doc's recommendation — "the honest code-plane counterpart of the Extensions screen is a *permissions and tools* screen, not an MCP screen" — was right, and is now buildable: read the ruleset out of `GET /config`, write it with `PATCH /global/config`, both of which the gateway already forwards and one of which the client already has a generic verb for.
2. **Local stdio MCP servers run in the container.** "Remote-only" is wrong. `opencode x <npm-package>` and `opencode run <script>` work because `opencode` *is* Bun and OpenCode injects `BUN_BE_BUN=1` when `argv[0] === "opencode"`; the install cache lands in `/chat/home`, the persisted volume **[measured]**. Two real residual limits: a CPython server (`uvx mcp-server-*`, which is exactly what `workspace-mcp` is) still needs a runtime added, and the injection keys on the literal string `opencode`, so an absolute path needs `"environment": {"BUN_BE_BUN":"1"}` explicitly.
3. **Structured output is available on the code plane.** `POST /session/{id}/message` with `format: {type:"json_schema", schema}` gives the recipe's typed-return semantics to an OpenCode run **[measured]**. If the app ever wants "run this and give me a structured result" it now works on both planes — goose via `response.json_schema` on the recipe, OpenCode via `format` on the call.
4. **The plane badge gains an enforcement half on the code side.** The earlier doc was right that declaration is advisory (goose reads `metadata`, OpenCode drops it). What it missed is that OpenCode has a real, enforced, per-agent, glob-patterned skill fence — `permission.skill` — that both hides a denied skill from `<available_skills>` and rejects it at load **[measured]**. So the badge can become an **action**: "Deny on this plane" is one key in a `PATCH /global/config`.
5. **goose's `read`: reframe, do not fix.** goose has a `read` tool; the phone declines it. **The refutation pass's own restatement — "flipping that one boolean in the phone client is the whole change" — is wrong for this deployment.** ACP filesystem capabilities mean *the client performs the read on the agent's behalf*; this client is a phone on the other side of a WebSocket from the brain, with no access to the brain's disk. `client.rs:182` declines `readTextFile`, `writeTextFile` **and** `terminal` together, consistently and correctly **[read-only]†**. The honest statement is: *on a remote-client ACP deployment, goose's file reads go through shell+cat, because the read tool is defined to run on the client.* If a server-side read is ever wanted, the implementation is already written and unbound — `EditTools::file_read_with_cwd` at `developer/edit.rs:47` has only its own tests as callers **[read-only]†**.
6. **The gateway already has a method allowlist.** `code-plane-library.md §7` assumption 1 should be updated: a *method* allowlist exists (and is why the PTY upgrade dies); only the *path* half is open.
7. **The `/tui/*` routes are not inert and plugins are HTTP-manageable.** Both exclusions can stand on other grounds; the stated reasons are wrong.

### The one new safety statement the UI must make

The earlier hazard marker — flag a skill body containing `` !`…` ``, a bare `@path`, or a bare `$1` — is correct and **too narrow**. Widen it to:

> **On the code plane, running *anything* through the command route executes embedded shell and reads referenced files with no permission check** — whether it arrived as a skill, a `.opencode/command/*.md` in the repo clone, or an MCP server's prompt. This is not specific to skills, and `external_directory: deny` does not stop it. The same is true of `POST /session/:id/shell` and of any `@file` attached to any message.

And the corollary for the row, not just the detail view: **the misleading artifact is the one you never open**, so the marker belongs on the list row, and "Run" on the code plane must never be offered bare.

---

## 7. Standing corrections to the shipped docs

### `docs/shared-artifacts.md`

**§1, line 20.**
> ~~"the two fields the owner's entire security posture rests on — `available_tools` and `env_keys` — have no destination field on the OpenCode side at all"~~

Replace with: *"the two fields the owner's entire security posture rests on — `available_tools` and `env_keys` — have no counterpart **field**, but both have real destinations in other subsystems: `available_tools` becomes an ordered `permission` ruleset over the flattened `<server>_<tool>` key (`permission/index.ts:204-219`, applied to MCP tools at `tool/registry.ts:281`), and `env_keys` becomes `{env:VAR}` / `{file:path}` substitution over the config text (`config/variable.ts:33`). The porting cost is a translation, not an impossibility."*

**§2.2 table, `response.json_schema` row.**
> ~~"goose builds a synthetic tool the model must call; a command emits chat text"~~

Replace with: *"no destination on a **command** — but an exact one on the **call**: `format: {type:'json_schema', schema}` on `POST /session/{id}/message` makes OpenCode build the same synthetic tool with `toolChoice:'required'` and return the validated object on `assistant.structured` (measured). goose binds the schema to the artifact; OpenCode binds it to the caller."*

**§2.3 table, two rows.**
> ~~"**env by reference** | **`env_keys`** | **absent** | **no**"~~
> ~~"**tool allowlist** | **`available_tools`** | **absent as a field** | **no**"~~

Replace the "portable?" cells with **"yes, by translation"** and the OpenCode cells with `{env:}` / `{file:}` and `permission: {"<server>_*":"deny", …allows}` respectively. Keep the caveat that only a `pattern:"*"` deny *hides* a tool; `ask` leaves it advertised.

**§2.3, line 146.**
> ~~"`env_keys` has **no** OpenCode equivalent."~~

Replace with: *"`env_keys` has a functional equivalent and three real asymmetries. **Backing store**: goose resolves through the OS keyring (degrading to a 0600 file); OpenCode has no keychain integration at all, so its targets are the process env, a file, and `auth.json`. **Discovery**: `env_keys` is a declared list of secret names that drives interactive setup (`configure.rs:1118-1144`) and recipe pre-flight checks (`secret_discovery.rs:73`); `{env:X}` is a bare reference with no registry, so nothing can be enumerated or prompted for. **Failure**: goose warns and skips or hard-errors; OpenCode substitutes the empty string silently. In exchange OpenCode's form is syntactically more general — it works inside a larger string, in argv, and in a URL, none of which `env_keys` can do."*

**§2.3, line 172.**
> ~~"**Zero of ten are both portable and desirable.**"~~

Replace with: *"**Two of the ten port cleanly and are worth having** — `skills` (same directory, same files, measured on both listers) and `workspace-mcp` (goose's exact pinned command line reported `✓ connected` under OpenCode, publishing exactly the ten tools `available_tools` pins). **Four more are redundant** because OpenCode ships the capability natively (developer, todo, summon, extensionmanager). **Four have no direct equivalent** and would need redesign (memory, tom, analyze, scheduler). Note the residual on workspace-mcp: it is a `uvx` server, so it specifically needs a Python runtime added to the image — but that is a fact about CPython, not about local MCP servers in general (see the C10 correction below)."*

**§4, line 218.**
> ~~"**goose has no file-read tool.** `DeveloperClient` advertises `write`, `edit`, `shell`, `tree`, `read_image` and nothing else"~~

Replace with: *"**goose has a file-read tool that this client declines.** The static list really is those five (`developer/mod.rs:108-179`), but `AcpTools` inserts a sixth — `read`, 'Read a text file from disk' (`acp/fs.rs:107`) — into the developer extension's list whenever the ACP client advertises `fs.readTextFile` (`acp/fs.rs:406-408`, gated at `acp/server.rs:866`). This client declares `{"readTextFile": false, "writeTextFile": false}` (`crates/goose-acp-client/src/client.rs:182`), and that is correct rather than an oversight: the capability means the **client** performs the read, and this client is a phone with no access to the brain's disk. So on this deployment 'read file X' does become shell+cat — because of the ACP filesystem model, not because goose lacks the tool. A server-side read is ~10 lines away: `EditTools::file_read_with_cwd` (`developer/edit.rs:47`) is written and has only test callers."*

**§4, line 220.**
> ~~"**goose has no web tool at all** — and this kills the skill everyone named first. There is no `webfetch`, no `websearch`, nothing in `crates/goose-mcp` (`computercontroller` is xlsx/docx/pdf/computer_control). Web access on the brain exists only via the `tavily` MCP extension"~~

Replace with: *"**goose has no general web-fetch and no web-search tool, and three real network paths.** (1) The **installed 1.46.0 binary's `computercontroller` ships `web_scrape` and `automation_script`** — measured by driving `goose mcp computercontroller`; the checkout removed them afterwards in `0d0d130d9`. If that extension is ever enabled without an `available_tools` list, the deployment gets an unrestricted URL fetcher and a second shell. (2) `read_image` is default-enabled and performs a GET at any model-supplied `http(s)` URL (`developer/image.rs:168-214`), annotated `open_world_hint: true` — a web fetch for pixels, not for pages, and one goose's own egress inspector does not match. (3) `shell`, which goose's own extension instructions recommend for 'accessing web sites or APIs'. `deep-research`'s text-and-citations contract still has nothing clean to bind to, so the Tier-1 conclusion stands — but 'no web tool at all' does not, and 'only via tavily' is wrong twice: tavily is not a goose extension (zero occurrences in the binary), and the owner's template disables **two** web routes, tavily and playwright."*

**§4, line 222.**
> ~~"so `model:` and `permission: {edit: deny}` are read and discarded — **there is no per-subagent read-only on goose**"~~

Replace with: *"so `permission: {edit: deny}` is discarded — `parse_agent_content` builds the entry with `properties: HashMap::new()` (`summon.rs:156`) — but **`model:` is not**. `build_recipe_from_agent` re-parses the frontmatter at delegate time and turns it into `Settings { goose_model }` (`summon.rs:1504-1548`), applied with precedence delegate-arg > agent-file > `GOOSE_SUBAGENT_MODEL` > session model (`:1626-1634`). Read-only **is** achievable, just not declarable in the agent file: `delegate(extensions: [])` gives a subagent no tools at all, `available_tools` is enforced at both listing and dispatch, and `goose review` runs each check as `goose run --no-profile`. What is genuinely missing is per-tool denial that reaches a subagent — subagents are pinned to `GooseMode::Auto`, which short-circuits `NeverAllow`, and are built with an empty `HookManager`, so `PreToolUse` never runs for them. So `code-review:61` is unenforceable **as declared in the agent file**; the equivalent guarantee has to be made by the caller."*

**§2.1, line 64.**
> ~~"**neither engine has a field that says which plane a skill belongs to** — the only fence today is prose in the `description`."~~

Replace with: *"**neither engine lets the skill file itself say where it belongs** — OpenCode documents `compatibility` and drops it at parse (measured), goose keeps a `metadata` bag and reads only `argument-hint`/`arguments` from it. But prose is not the only fence. OpenCode has a real one, config-side and per-agent: `permission.skill` with allow/ask/deny and glob patterns, which both hides a denied skill from `<available_skills>` and rejects it at load (measured on `build` vs `plan`). Both engines also fence by **placement** — their private discovery roots are disjoint — and OpenCode ships `OPENCODE_DISABLE_EXTERNAL_SKILLS` to cut the shared pool wholesale (measured: eleven skills to zero). goose alone has nothing at per-skill granularity, which is notable given that its own `checks` source type enforces frontmatter-declared `tools:`/`model`/`scope_dir`."*

**§5.3, line 285 (hazard marker).** Widen per §6 above: the hazard is the **command route**, not the skill; it covers repo-provided commands and MCP prompts equally; `external_directory: deny` does not stop it; and `POST /session/:id/shell` and any `@file` on any message are the same.

**§6, line 304.** Keep the sentence, add: *"On OpenCode this is a regression rather than a design boundary — commit `8fe0715928` kept `license`/`compatibility`/`metadata` and `046e351140` removed them two days later, while `web/src/content/docs/skills.mdx` and the built-in `customize-opencode` skill still advertise all three (observed served live from the 1.18.25 container). The upstream fix is three lines, so this claim should be re-checked on every version bump."*

**§6, line 305.**
> ~~"**A web tool on goose.** ... Today the only path is enabling `tavily` with a proper `available_tools` list."~~

Replace with: *"**A web tool on goose.** Today the paths are: enable `tavily` or `playwright` with a proper `available_tools` list; select the shipped Perplexity provider, whose declarative definition states 'built-in real-time web search grounding' and needs no extension at all; or discover that the installed binary's `computercontroller` already has `web_scrape` — which is a finding to act on, not a plan."*

### `docs/code-plane-library.md`

**§1, line 22.**
> ~~"It has **no scheduler at all** — that one is genuinely goose-only."~~

Replace with: *"It has **no scheduler of its own** — no `opencode schedule`, no schedule key in config, no scheduling endpoint in the 162-path OpenAPI doc, no timed plugin hook. It is, however, a **first-class cron target**: `schedule` is one of six supported GitHub Action events with a dedicated code path (`cli/cmd/github.handler.ts:149, 402, 421, 528, 728`) and a documented `- cron: \"0 9 * * 1\"` example, all present in the shipped binary. So goose's scheduler has a destination — an inbound trigger, not a peer."*

**§2 inventory, row 12 (Plugins), Gateway cell.**
> ~~"n/a — **no HTTP route at all**"~~

Replace with: *"**Yes, via `/global/config`** — no dedicated route, but `GET /config` lists the `plugin[]` array and `PATCH /global/config` adds or removes, with the server npm-installing on the next instance build (measured: a package appeared under `~/.cache/opencode/packages/` and its tool entered `/experimental/tool/ids`). Excluded on risk, not reachability."*

**§2 inventory, row 13 (Themes/keybinds), Gateway cell.**
> ~~"Routes proxy, and lie"~~

Replace with: *"Routes proxy and publish. Excluded as TUI-only."*

**§5, "Local (stdio) MCP servers", line 120.**
> ~~"**remote MCP is the only viable kind for a phone-driven library**"~~

Replace with: *"The nine binary names are indeed absent — but the tenth is a runtime. `opencode` is a Bun 1.3.14 single-file executable, and OpenCode injects `BUN_BE_BUN=1` for any local MCP server whose `argv[0]` is `opencode` (`mcp/index.ts:354`). Measured in the real code-agent image: two off-the-shelf npm stdio MCP servers reached `connected` via `opencode x -y @modelcontextprotocol/server-…`, with the install cache landing in `/chat/home` and surviving spin-down. **JS/TS stdio servers are viable today; CPython servers (`uvx …`, which is what `workspace-mcp` is) still need a runtime added** — via the repo's `setup` field, `apk add python3` (the container is uid 0 and not read-only), or an image change the Containerfile already documents at `:30-33`. Note also that the injection keys on the literal string `opencode`, so an absolute path needs `\"environment\": {\"BUN_BE_BUN\":\"1\"}` explicitly."*

**§5, "Themes and keybinds", line 126.**
> ~~"Worse than inert: measured, `POST /chat/<id>/tui/open-themes` returns **200 `true`** with no TUI attached. Any UI built on a `/tui/*` call shows a green tick for something that never happened."~~

Replace with: *"The boolean is a **publish receipt, not a delivery receipt** — the routes are not inert. Every fire-and-forget `/tui/*` route emits a real `tui.*` event on the general bus, delivered over `GET /event` to any subscriber (measured: a plain curl SSE client on a TUI-less server received `tui.command.execute` and `tui.toast.show`). OpenCode has no way at all to detect an attached TUI, which is the real caveat. Two routes do not return blanket success (`/tui/select-session` validates; `GET /tui/control/next` blocks forever headless), and `open-themes` is an upstream **mis-wire**, not a no-op: `handlers/tui.ts:51-54` publishes `session.list`, so it opens the sessions dialog — TUI attached or not. Worth filing upstream. Still excluded: TUI-only, and we run headless."*

**§5, "Plugins", line 128.**
> ~~"There is **no plugin list/install/uninstall HTTP route anywhere** in the httpapi groups."~~

Replace with: *"There is no **dedicated** plugin route — but list, install and uninstall all work through the config routes, measured end to end. Exclude on the merits (arbitrary in-process JS with `Bun.$` and the `Auth` service in scope), not on reachability. Note that a dedicated `GET /api/plugin` (`v2.plugin.list`) already exists in the repo's generated OpenAPI fixture and on unmerged branches, so this will become a first-class route."*

**§5, "Structured output / `response.json_schema`, `retry`, `activities`", line 136.**
> ~~"OpenCode's `ConfigCommandV1.Info` is only `{template, description, agent, model, variant, subtask}`. Recipes-only, permanently."~~

Replace with: *"Split them; they do not have the same answer. **Structured output has an exact destination**, just not on a command: `format: {type:'json_schema', schema, retryCount}` on `POST /session/{id}/message` or `/prompt_async` registers a synthetic `StructuredOutput` tool whose input schema is the caller's, forces `toolChoice:'required'`, and returns the validated object on `assistant.structured` — measured live. **`retry` (success criteria) and `activities` are genuinely recipes-only**; the one adjacent knob, `retryCount`, is declared, defaults to 2, and is never read by the runtime (verified in source and in the shipped bundle). Also correct the premise: the live `Command.Info` is eight fields, and commands are synthesised from config markdown, MCP prompts and skills alike."*

**§7, assumption 1, line 160.**
> ~~"Everything 'free today' depends on `ROUTE_CHAT` being matched first with no path or method allowlist."~~

Replace with: *"...with no **path** allowlist. A **method** allowlist already exists and is not a config field — it is the set of `do_*` methods on the handler class (`code-agent-manager.py:1543-1556`), enforced by `BaseHTTPRequestHandler` via `hasattr`. Measured: HEAD, OPTIONS, TRACE and arbitrary verbs get 501 before authentication and never reach the container. This costs no declared OpenCode endpoint (all 127 declared endpoints use GET/POST/DELETE/PATCH/PUT) but it does cost CORS preflight — and, together with `upgrade`/`connection` being stripped as hop headers, it is the real explanation for the PTY block in §5. If a path allowlist is ever added, allowlist `/command`, `/skill`, `/agent`, `/mcp`, `/global/config` and `/api/permission/saved`; blacklist `/config` and `/tui/*`."*

**§5, "`POST /mcp` as a durable add", line 122.** Keep the finding; extend: *"Also invisible to `GET /config`, so a read-modify-write client silently drops it; wiped by **any** config write from any client (both `PATCH /config` and `PATCH /global/config` measured), by `POST /instance/dispose`, and by restart — but **not** by an on-disk edit to `opencode.jsonc`. And the durable capability has three destinations, none named `mcp`: `PATCH /global/config` (durable **and** live), the `opencode mcp add` CLI (durable, not live), and a plugin `config` hook (live at every start, zero config bytes). goose draws exactly the same line — `_goose/unstable/session/extensions/add` vs `_goose/unstable/config/extensions/add` — so `POST /mcp` is the peer of goose's session add, not its config add."*

**§3, the `PATCH /config` warning (line 72) and the 404 warning (line 73).** Both survive and were independently re-hit by a second attacker: a project-scoped `PATCH /config` wrote `<project>/config.json`, a filename outside the discovery chain, and the key never appeared in `GET /config` or `GET /mcp`, before or after a restart — **durable-but-inert, now confirmed** rather than an open gap. And "unknown paths answer 200 `text/html`" is the general form of a trap that caught a second investigation: on OpenCode, **the absence signal is a 200 HTML body, not a 404**.

**§2 inventory, row 8 (Permission policy).** No change — "incl. `permission.skill` globs" was already right, and it is now the enforcement half of the plane badge in `shared-artifacts.md §5.3`. Promote it from an inventory cell to a design item.

---

## 8. Unverifiable here

- **OpenCode 1.18.21–1.18.24 drift.** Production is 1.18.25 and behaviour claims were executed against 1.18.20 and 1.18.25; static reads are 1.18.5. The permission path (`resolveTools`, `Permission.disabled`), the skill write site and the command expansion block are byte-identical across all three. **Settled by** re-running the §3 probes against the pinned production digest after any image bump.
- **Whether the deployed brain's goose is the Homebrew 1.46.0 measured here.** It matters, because that build's `computercontroller` ships `web_scrape` and `automation_script` and no 1.46.0 source tree does. **Settled by** running `printf '…initialize…tools/list…' | goose mcp computercontroller` on the brain — the exact command in §1(d).
- **Whether `permission.webfetch` is still rejected as a pattern map on 1.18.25.** Rejection proven on 1.18.20. **Settled by** `opencode debug config` against a 1.18.25 container with that key present.
- **Whether an externally proxied `redirect_uri` completes an MCP OAuth flow in OpenCode.** The code paths are consistent with it (client metadata takes the configured URI; the listener takes only its port and still binds `127.0.0.1`). **Settled by** pointing `redirect_uri` at a tailnet host reverse-proxying into the container's `127.0.0.1:19876` and running `opencode mcp auth <name>`.
- **Whether a live model, as opposed to a forced tool call, changes any "the model does not see this tool" claim.** Every such claim rests on source plus compiled binary plus `opencode debug agent`, not on an observed request body. **Settled by** running either agent under a proxy that logs the `tools` array.
- **The `/api/integration/*` and `/api/credential/*` v2 surface.** Still entirely unexercised, and still the one plausible candidate that could displace MCP as the Extensions analogue. **Settled by** one probe.

**Repository state.** No repository was modified. `git status --porcelain` returns zero lines in `/Users/phillipchaffee/git/opencode`, `/Users/phillipchaffee/git/goose`, `/Users/phillipchaffee/git/personal-ai-setup` and `/Users/phillipchaffee/git/goose-phone-app/.claude/worktrees/testing`. **[measured]†**