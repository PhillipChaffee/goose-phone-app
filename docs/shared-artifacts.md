<!-- Produced by a research workflow: four agents compared the artifact pairs
(skills, recipes/commands, extensions/MCP) and the owner's own corpus, then two
more cross-loaded real artifacts into the PRODUCTION container image and
enumerated the tool inventories on both sides. 41 findings were established by
running things rather than reading. The permission-bypass in 2.1 was measured
with a purpose-built probe skill and then independently confirmed against
opencode packages/opencode/src/session/prompt.ts:1397-1407, where
Process.text() runs before getModel() and agents.get() and no permission check
appears in the path. Static line numbers are against OpenCode 1.18.5; every
behavioural claim was executed against 1.18.25, which is what production runs. -->

# Can the two planes share artifacts?

*A three-level answer — file shape, semantics, product — for the decision about whether the chat half and the code half of this app share a library or keep separate ones.*

---

## 1. The answer in a paragraph

**The answer is different for each of the three pairs, and that split is the finding.** Skills are genuinely shared — not in theory, but on this machine today: `goose skills list` and `opencode debug skill` each enumerate the same eleven `SKILL.md` files out of `~/.agents/skills/`, byte-identical, because both engines deliberately scan that directory (`goose/crates/goose/src/skills/mod.rs:38-40, 316-335`; `opencode/packages/opencode/src/skill/index.ts:21-23, 184-193`), and the owner wrote that overlap down as design intent (`personal-ai-setup/scripts/mac/bootstrap-mac.sh:122-125`: *"One skills target serves both tools: `~/.agents/skills/` is read by OpenCode ... AND by goose >= 1.16's built-in skills support"*). Recipes and commands are not shared and should not be: a goose `Recipe` is a session bundle with extensions, provider, temperature, retry checks, a response schema and sub-recipes (`goose/crates/goose/src/recipe/mod.rs:41-85`), while an OpenCode command is six fields — `{template, description, agent, model, variant, subtask}` (`opencode/packages/core/src/v1/config/command.ts:5-12`) — so translating one *decomposes* it across two or three files on the other side, and both parsers silently swallow the other's fields, which makes a half-finished port look finished. Extensions and MCP servers overlap on exactly two of goose's seven variants, and the two fields the owner's entire security posture rests on — `available_tools` and `env_keys` — have no destination field on the OpenCode side at all. **So: one shared store and one shared type for skills; no shared type for recipes/commands; a deliberately unshared model for extensions.** And the sharp caveat that should drive the UI: at level 1 everything passes, at level 2 most of it fails, and the failures are *silent on both sides*. The dangerous artifact is not the one that errors — it is the one that loads.

---

## 2. Pair by pair

### 2.1 Skills

| | goose 1.46.0 | OpenCode 1.18.25 |
|---|---|---|
| file | `SKILL.md`, YAML frontmatter | `SKILL.md`, YAML frontmatter |
| parser | `parse_frontmatter` → typed struct `SkillFrontmatter { name: Option<String>, description: String, metadata: HashMap<String, Value> }` (`skills/mod.rs:24-36`) | `gray-matter`, untyped, then a shape predicate `isSkillFrontmatter` requiring only `name: string` (`skill/index.ts:53-59`) |
| shared discovery dirs | `~/.agents/skills`, `~/.claude/skills` (`skills/mod.rs:316-335`) | `~/.agents/skills`, `~/.claude/skills` (`skill/index.ts:21-22, 186-193`) |
| model-chosen entry | `load_skill(name, args?)` from the `skills` platform extension | `skill` core tool (`tool/skill.ts`) |
| user-run entry | every skill is also a slash command (`slash_commands/skill_slash_command.rs:7-9, 42-59`) | every skill is also in `/command`, tagged `source: "skill"` (`command/index.ts:132-152`) |
| extra fields read | `metadata.argument-hint`, `metadata.arguments` | none |
| everything else | dropped silently | dropped silently |

**Level 1 — file shape: YES, verified live in both directions.** All twelve of the owner's skills parse on both parsers. In a container built from `personal-ai-setup/config/code-agents/Containerfile` running the production image (`ghcr.io/anomalyco/opencode:latest`, which reports 1.18.25), five owner skills loaded from four different roots — `~/.agents/skills`, `~/.claude/skills`, `$workspace/.claude/skills`, and `$XDG_CONFIG/opencode/skill/` (singular, a fifth root nobody had documented). `GET /skill` returned entries with exactly the keys `['content','description','location','name']`, and a byte diff against the on-disk files showed the body identical for all five — only the frontmatter block is stripped. On the goose side, the real 1.46.0 binary's `goose skills list` printed the same eleven directories plus the built-in `goose-doc-guide`.

Three asymmetries worth knowing:

- **goose is stricter.** OpenCode carries an explicit Claude-compat hack — `packages/opencode/src/config/markdown.ts:18-19`: *"other coding agents like claude code allow invalid yaml in their frontmatter, we need to fallback to a more permissive parser for those cases"*. An unquoted colon in `description:` is rescued on OpenCode and dropped with a `warn!()` on goose. **Author against goose and OpenCode takes it; author against OpenCode and goose may silently lose it.** The owner's `config/opencode/AGENTS.md:379-395` already encodes the strict discipline, which is why all twelve survive.
- **Neither engine's own built-in is portable.** OpenCode's `customize-opencode.md` has no frontmatter at all (name and description are TS constants, `skill/index.ts:32-35`), so both engines reject the file. goose's `goose_doc_guide.md` is a valid SKILL.md whose body contains `{{GOOSE_DOCS_ROOT}}`, substituted only by goose for that one name (`skills/mod.rs:109-115`).
- **Duplicate resolution differs.** goose: first-wins in a fixed precedence order (`skills/mod.rs:453-471`). OpenCode: last-write-wins, under `Effect.forEach(..., { concurrency: "unbounded" })` (`skill/index.ts:125-131, 240-243`) — i.e. non-deterministic. `~/.claude/skills/` currently holds six stale *empty* directories, so no live collision; refill them and the two engines can disagree about which `code-review` you got.

**Level 2 — semantics: NO, and the sharpest failure is on a single engine.** The same `SKILL.md` means two different things *on OpenCode alone*, depending on how it is invoked. A purpose-built probe skill containing three markers was installed and driven both ways in the production container:

```markdown
Marker A (shell): !`echo PWNED_SHELL_RAN`
Marker B (file):  @/etc/alpine-release
Marker C (posix): $1 and $ARGUMENTS
```

Through the `skill` tool the body arrived at the model verbatim — `!` echo`, `@/etc/...`, `$1` all literal, nothing executed, nothing read. Through `POST /session/{id}/command` the shell ran (`PWNED_SHELL_RAN` present in the upstream request body), the file was inlined, and `$1` was substituted. This is not a "trap"; it is a **permission bypass**. Under the owner's own container template, which sets `"external_directory": {"*": "deny"}` (`config/code-agents/opencode.json:25-27`), the `read` tool refuses external paths — and `@/etc/alpine-release` inside a skill body was read and inlined anyway, with no permission evaluation. The shell command likewise ran with no bash-permission check. Expansion happens in `session/prompt.ts` before the model exists.

goose is the mirror image: its template environment registers no shell function (`recipe/template_recipe.rs:117-133`), so `` !`cmd` `` and `@file` pass through as inert literals. But goose has its own hazard: `PLACEHOLDER_RE` (`skills/arguments.rs:6-11`) is

```rust
r"\$ARGUMENTS\[(?P<idx>\d+)\]|\$ARGUMENTS\b|\$(?P<pos>\d+)|\$(?P<name>[A-Za-z_][A-Za-z0-9_-]*)"
```

and it runs over the **entire skill body** whenever arguments are supplied. Undeclared `$name` stays literal (`is_resolvable`, `:13-17`), but `$1`/`$2` always substitute — so a body containing `awk '{print $1}'` is clobbered by `/skill some-arg` on goose and untouched on OpenCode. The corpus is clean today (the only `$` in twelve skills is `` `$CI_*` `` at `ci-lint-test/SKILL.md:58`, undeclared and therefore literal).

**The loads-but-misleads case for skills, stated plainly:** `connect-service/SKILL.md` — whose entire subject is goose's ACP config, pinned to *"goose 1.46.0"* (`:28`) and requiring `_goose/unstable/config/extensions/{list,add,remove,set-enabled}` and `_goose/unstable/config/upsert` (`:56-61`) — parses perfectly on OpenCode, loads from `$XDG_CONFIG/opencode/skill/`, and was observed **in the code-agent container's system prompt**, inside `<available_skills>`, with its home-directory path leaked in a `<location>` element, and in `GET /command`. The container has no goose, no tailnet, no `_goose/unstable/*` and `GET /mcp` returns `{}`. The offer cannot be fulfilled. Sharing one directory is bidirectional, and **neither engine has a field that says which plane a skill belongs to** — the only fence today is prose in the `description`.

**Level 3 — product: YES, for a narrow, specific set, and almost entirely in one direction.** See §3.

---

### 2.2 Recipes vs commands

| goose `Recipe` field (`recipe/mod.rs`) | OpenCode command | Lost |
|---|---|---|
| `version` :44 | — | cosmetic |
| `title` :47 | — | collapses into `description` |
| `description` :49 | `description` | survives |
| `instructions` :54 + `prompt` :57 | `template` | two fields merge into one blob |
| `extensions: Vec<ExtensionConfig>` :64 | — | **no per-command tool set.** Nearest is `agent: <name>` → a second file |
| `settings.goose_provider` / `goose_model` | `model` | survives syntactically; `opencode/kimi-k2.5` does not resolve on goose |
| `settings.temperature`, `max_turns` | — | dropped (they are *agent* fields, `config/agent.ts:18, 34`) |
| `activities` :70 | — | nothing anywhere |
| `parameters: Vec<RecipeParameter>` :76 | — (`hints` is derived) | **the crux, below** |
| `response.json_schema` :79 | — | goose builds a synthetic tool the model must call; a command emits chat text |
| `sub_recipes` :82 | `subtask: bool` | boolean only; no fan-out, no bound `values:` |
| `retry: {max_retries, checks[], on_failure}` :85 | — | nothing. `health-followups.yaml:114-119` re-runs unless `grep -q '^## Follow-ups'` exits 0 |
| — | `` !`cmd` `` / `@file` | goose has no author-time shell; a *guarantee* becomes a *request* |

**Parameters are not a subset either way.** goose declares and enforces a bijection: every `{{ var }}` needs a `parameters` entry **and** every entry must be used — both directions are hard errors. Executed against the real binary:

```
$ goose recipe validate t1.yaml    # prompt has {{ target }}, no parameters block
Error: ✗ recipe file is invalid: Missing definitions for parameters in the recipe file: target.
$ goose recipe validate t2.yaml    # declares `unused`, never referenced
Error: ✗ recipe file is invalid: Unnecessary parameter definitions: unused.
```

OpenCode never declares: `hints` is regex-derived after the fact (`command/index.ts:36-44`), values are positional, a missing `$N` becomes the empty string silently (`session/prompt.ts:1385`), and the highest-numbered placeholder swallows the tail — measured: template `"$1 and $ARGUMENTS"` with arguments `"ARG1 ARG2"` rendered as `"ARG1 ARG2 and ARG1 ARG2"`. The strongest evidence that positional is OpenCode's ceiling is what it does to MCP prompts, which *do* carry named, described, required arguments: it throws the names away — `Object.fromEntries(prompt.arguments.map((argument, i) => [argument.name, `$${i + 1}`]))` (`command/index.ts:113-131`).

**Loads but misleads — this pair is the worst offender, and it was demonstrated end to end.** OpenCode's own `/init` and `/review` commands, pulled verbatim from a running container's `GET /command`, are rejected outright as `.md` (*"Recipe file ... is not a json or yaml file"*). Hand-translate to YAML and:

```
$ goose recipe validate review.yaml
✓ recipe file is valid

$ goose run --recipe review.yaml --render-recipe | grep -n ARGUMENTS
 9:  Input: $ARGUMENTS
23:     - Run: `git show $ARGUMENTS`
26:     - Run: `git diff $ARGUMENTS...HEAD`
29:     - Run: `gh pr view $ARGUMENTS` to get PR context

$ goose run --recipe review.yaml --params ARGUMENTS=HEAD --render-recipe | grep -c ARGUMENTS
5          # unchanged, no error
```

The literal token reaches the model inside the git commands the command exists to run. Adding `subtask: true`, `agent: build`, `model: opencode/kimi-k2.5`, `variant: x` and `template: IGNORED` to a valid recipe still validates and renders **byte-identically** — every OpenCode-only field is silently dropped. And the inverse holds: a recipe whose prompt contains `` !`git diff --stat` `` and `@README.md` validates fine and renders those as literal text.

One correction to a claim made earlier in this investigation: goose's bijection is between template variables and the `parameters` block, **not** between supplied `--params` and declarations. `goose run --recipe review.yaml --params NOPE=1 --render-recipe` against a recipe with zero declared parameters renders fine and exits 0.

**Product verdict: mostly no, in both directions.** Six of the eight real commands in `opencode/.opencode/command/` begin by shelling into git; on a plane with no repo they have no subject. Five of the seven owner recipes are unattended cron jobs (`morning-brief`, `inbox-triage`, `weekly-review`, `health-followups`, `budget-checkin`) registered via `goose schedule add --recipe-source`; OpenCode has no scheduler, so there is nothing to attach them to. The one recipe you would want as a command is `connect-service` — and the owner already resolved that case himself, in the file: *"From the phone there is no recipe launcher — ask the brain to run the connect-service skill instead; the skill IS the procedure, and this file is only its launchable form"* (`recipes/connect-service.yaml:12-14`).

---

### 2.3 Extensions vs MCP servers

goose's `ExtensionConfig` is a seven-variant tagged union (`goose/crates/goose/src/agents/extension.rs`): `Sse` :165, `Stdio` :176, `Builtin` :199, `Platform` :215, `StreamableHttp` :230, `Frontend` :280, `InlinePython` :298. OpenCode's is two: `Local` and `Remote` (`opencode/packages/core/src/v1/config/mcp.ts:6, 44`).

| concept | goose `stdio` | OpenCode `local` | portable? |
|---|---|---|---|
| program | `cmd` + `args` | `command: string[]` | mechanical rewrite |
| env, literal | `envs` | `environment` | yes |
| **env by reference** | **`env_keys`** | **absent** | **no** |
| **tool allowlist** | **`available_tools`** | **absent as a field** | **no** |
| timeout | `Option<u64>` — **seconds** | `PositiveInt` — **milliseconds** (`mcp.ts:20-22`) | `timeout: 300` becomes 300 ms |
| on/off | `enabled` on the wrapper entry | `enabled?` inside the entry | rewrite |
| UDS `socket`, `client_secret_key` | present on `streamable_http` | absent | dropped |
| `oauth: {clientId, scope, callbackPort, redirectUri}` | partial | present | no goose home |

`available_tools` polarity is empty-means-allow-all (`extension.rs:465-466`):

```rust
available_tools.is_empty() || available_tools.contains(&tool_name.to_string())
```

OpenCode's nearest equivalent is the `permission` map, which reaches the same effect by the **opposite** construction: a deny-star followed by explicit allows, where order is load-bearing (`Permission.evaluate` uses `findLast`) and the key is the *flattened* name `workspace-mcp_search_gmail_messages`, not the raw server tool name. Same open failure mode, weaker binding: rename the server key or let a server rename a tool and the deny-star stops matching, with no error.

`env_keys` has **no** OpenCode equivalent. goose resolves it from its secret store and hard-fails the extension if the store errors; over ACP it is *enforced* — `goose_extension_to_config_without_secrets` rejects inline env outright (`acp/server/extensions.rs:350-360`), which is what this app's credential design rests on (`src/extensions.rs`). OpenCode's only credential store is `mcp-auth.json`, and it holds OAuth tokens only.

The clearest real example is in the owner's own repo — the same server, both planes:

```yaml
# personal-ai-setup/config/goose/config.yaml:244-268
  todoist:
    type: streamable_http
    enabled: false
    uri: https://ai.todoist.net/mcp
    env_keys: [TODOIST_API_KEY]
    headers: { Authorization: "Bearer ${TODOIST_API_KEY}" }
    available_tools: [get-overview, find-tasks, find-tasks-by-date, find-projects,
                      add-tasks, update-tasks, complete-tasks, reschedule-tasks]
    timeout: 300
```

```json
// personal-ai-setup/config/opencode/opencode.json — the entire entry
"todoist-example": { "type": "remote", "url": "https://ai.todoist.net/mcp", "enabled": false }
```

Same endpoint, same protocol. The OpenCode entry has no credential and no allowlist — not carelessness, but inexpressibility. `scripts/verify/check-security.sh:349-357` fails the build on any *enabled* goose extension without a non-empty `available_tools`; `check-code-agents.sh` has no MCP check at all, because the code plane has no MCP servers to check.

**"Extension" is not the same concept.** goose's `platform` variant exists *because goose has no built-in tools*: `developer`, `todo`, `summon`, `skills`, `analyze` are in-process fake MCP clients registered in `PLATFORM_EXTENSIONS`. On goose the shell is an extension you can toggle; on OpenCode it is a compiled-in tool governed by `permission.bash`. That is why goose's Extensions screen is a real product surface and OpenCode's `mcp` block is a config detail — **the goose list contains the shell, and the OpenCode list never will.**

Of the ten extensions goose actually shows on this setup: four are already built into OpenCode (developer, skills, todo, summon — nothing to port), four are brain-side by nature (memory, top-of-mind, apps, extension-manager), one is a real gap with no port path (`analyze`, tree-sitter symbol graphs, compiled Rust), and one is a true MCP server (`workspace-mcp`) that cannot run in the container (`uvx` is absent) and that you would not want there anyway. **Zero of ten are both portable and desirable.**

---

## 3. What the owner's own corpus says

Twelve skills, seven recipes, thirty OpenCode agents. Small, but it is a real sample and it says three things.

**(a) The skill is the portable unit; the launcher is not.** This is not inference — it is written down. `recipes/connect-service.yaml:75-93`:

> **THE PROCEDURE IS NOT IN THIS FILE.** Before anything else, read in full and then execute:
> `/home/agent/personal-ai-setup/config/skills/connect-service/SKILL.md`
> ... This recipe supplies the launch parameters and the guardrails below; **the skill owns the steps.** Where the two appear to disagree, the SKILL wins, and this file must never grow a copy of its procedure — **they stay in sync by reference, not by duplication.**

The recipe is packaging. The skill is the work. And the mechanism by which the skill reaches the chat plane today is `cat`-by-absolute-path, because *"`scripts/vps/deploy-vps.sh` installs no skills at all"* (`:83-86`) — the recipe documents its own workaround.

**(b) Ten of twelve are code-shaped, but only about half irreducibly so.** The line that binds each skill to the code plane is one of three things: (a) a diff, a checkout, or a green test run; (b) a *path*, `~/.config/opencode/agents/<name>.md`; (c) a *string*, `opencode/kimi-k2.6` or `opencode/claude-sonnet-5`. (b) and (c) are addressing, not capability. Strip them and ask which skills still need (a):

| Skill | Class | The deciding line |
|---|---|---|
| `connect-service` | chat | `:56` *"A running `goose serve` on the brain at the pinned version"*; `:57-61` the `_goose/unstable/*` methods |
| `plan-review` | chat-shaped in disguise | `:22-25` accepts *"User pastes plan"* and *"the plan discussed in conversation"*. Its only real bindings are `~/.config/opencode/agents/` (`:65, 72, 100, 111`) and `opencode/*` slugs (`:42, 92-94`) |
| `looping-plan-review` | chat-shaped in disguise | budget is a plan line count; git is opt-in; Phase 1 is *"wait until the user declares the architecture aligned"* — a conversation currently locked in a terminal |
| `clean-plan` | chat-shaped in disguise | pure markdown editing, no subagents, no MCP, no model pins; already degrades (*"If no plan-steps rule is accessible, skip this pass"*) |
| `deep-research` | **contested — see §4** | description claims *"across the codebase, the web, or connected tools"* (`:9-10`), and the web half does not exist on goose |
| `ci-lint-test`, `pre-mr-checklist` | code, by data not wiring | no subagents, no model pins, no MCP — just a checkout, a shell, and a toolchain |
| `code-review`, `looping-code-review`, `refactor-planner` | code | `git diff main...HEAD` (`code-review:15`), *"`git push` to the existing remote branch so the MR updates in place"*, *"confirm green"* |
| `mr-review`, `ship` | code, and aspirational everywhere | `mr-review:18` *"GitLab MCP (required) — ... Without one configured, this skill cannot run"*; `ship:28-29` *"Linear MCP"* + *"Any working GitLab MCP"* |

`mr-review` and `ship` deserve a line of their own: **no GitLab MCP and no Linear MCP exists on any of the three machines.** The Mac's live `opencode.json` has one MCP entry (`todoist-example`, disabled); the container template has no `mcp` key at all; goose's config has `workspace-mcp` enabled and three disabled servers. These two skills are not a portability question — they are unrunnable everywhere today.

**(c) The chat→code direction is empty.** All seven recipes are chat-shaped. Five are cron jobs with delivery obligations and privacy rules; `vault-qa` is foreclosed by the owner's own policy (*"The life vault never goes in `repos.json`"*); `connect-service` is the pair discussed above. `health-followups` is the useful negative: it *uses git* (`git -C /data/life-vault pull --ff-only`, then add/commit/push) and is still unambiguously chat-shaped. **"Touches git" is not the discriminator. "Consumes a diff and produces a commit against reviewed code" is.**

---

## 4. The environment gap

This is where "same file format" stops being enough.

**The tool lists barely overlap.** Driving `goose acp` over stdio against a byte-copy of the owner's config (workspace-mcp disabled to stay offline) and calling `_goose/unstable/tools/list` returned 16 tools; `GET /experimental/tool/ids` on OpenCode returned 14 ids.

- goose: `analyze, delegate, edit, extensionmanager__*(2), load, load_skill, memory__*(4), read_image, shell, todo__todo_write, tree, write` (+ `workspace-mcp__*` ×10 and `scheduler__manage_schedule` on the brain).
- OpenCode: `invalid, question, bash, read, glob, grep, edit, write, task, webfetch, todowrite, websearch, skill, apply_patch`.
- **The two lists share exactly two literal names: `edit` and `write`.**
- Renamed but equivalent: `shell` ↔ `bash`, `delegate` ↔ `task`, `load_skill` ↔ `skill`, `todo__todo_write` ↔ `todowrite`. The exposed OpenCode id really is `bash`, not `shell` — `packages/opencode/src/tool/shell/id.ts:14-16`: *"Keep the exposed tool ID and permission key as \"bash\" for compatibility ... Rename with opencode 2.0."* A skill that names its tool in prose names a different string on each side, and the owner's container config keys its push guard on `permission.bash."git push*"`.

**goose has no file-read tool.** `DeveloperClient` advertises `write`, `edit`, `shell`, `tree`, `read_image` and nothing else; `file_read_with_cwd` exists in `developer/edit.rs:47` but its only callers are that file's own unit tests. So every *"Read the file X"* instruction in the corpus — `ship:35`, `plan-review:111`, `refactor-planner`'s eight `references/*.md` — becomes `shell` + `cat` on goose: a shell permission decision instead of a read.

**goose has no web tool at all — and this kills the skill everyone named first.** There is no `webfetch`, no `websearch`, nothing in `crates/goose-mcp` (`computercontroller` is xlsx/docx/pdf/computer_control). Web access on the brain exists only via the `tavily` MCP extension, which ships `enabled: false` with no `available_tools` — and `check-security.sh:349-357` would fail the build if it were enabled without one. `deep-research`'s citation contract (`:199` *"URLs for web"*) and `:169` *"Researchers keep their web and MCP access"* have nothing to bind to. **The genuinely portable, web-free skills are `plan-review`, `looping-plan-review` and `clean-plan`; `deep-research` ports only in its Tier-1, no-collector mode.**

**The subagent registries are not shared the way the skill directories are.** goose scans `<wd>/{.goose,.claude,.agents}/agents` and `~/{.goose,.agents,.claude}/agents` plus `<config>/agents`. `~/.config/opencode/agents` — where all 30 agents live, and where nine of the twelve skills address their reviewers by path — is not on that list. All four goose agent directories on this Mac are empty. Worse, the file contract differs: goose's `AgentMetadata.name` is non-optional (`agents/platform_extensions/summon.rs:114-116`) and a file missing it is dropped **silently** (`:126-131`, *"Missing fields means this file has valid YAML but isn't an agent — skip silently"*). All 30 OpenCode agent files lack `name:` because OpenCode derives it from the filename. Copying two into goose's `.agents/agents/` produced *"No sources available for load/delegate"*; adding one line `name: <stem>` to each made both delegable immediately. And even then goose builds the entry with `properties: HashMap::new()` (`summon.rs:146-156`), so `model:` and `permission: {edit: deny}` are read and discarded — **there is no per-subagent read-only on goose**, because `delegate`'s only isolation knob is `extensions: [...]` and `developer` bundles shell+write+edit together. `code-review:61` (*"its agent `permission` denies edits — it cannot modify files"*) is unenforceable on the chat plane.

**The most decisive fact: the code skills do not run on the owner's own code plane either.** `render_chat_config` (`scripts/vps/code-agent-manager.py:889-903`) writes exactly one file into a chat volume — `opencode.json`. `seed_auth` writes `auth.json`. `grep -ci skill scripts/vps/code-agent-manager.py` returns 0; nothing writes `agents/`, `skills/`, or a global `AGENTS.md`. A fresh container therefore exposes only the native agents (build, plan, general, explore, compaction, summary, title) — verified: `GET /agent` returned 7. Dispatching `task(subagent_type="cr-planner")` returned *"Unknown agent type: cr-planner is not a valid agent type"*. The documented fallback (*"use a general-purpose subagent with `cr-planner.md` inlined"*, `code-review:38-39`) also fails, because the file is not on disk to inline.

**The three-machine matrix:**

| | skills | agents | global rules |
|---|---|---|---|
| Mac | 11 in `~/.agents/skills` | 30 in `~/.config/opencode/agents` | `AGENTS.md`, 408 lines |
| Brain | **0** (`grep -ci skill deploy-vps.sh` = 0) | **0** (all four dirs empty) | `.goosehints`, 62 lines — owner/privacy/routing, not the engineering rules the skills cite |
| Container | **0** beyond the repo's own `.claude/` | **0** beyond the natives | **none** |

Every skill that says *"per the `minimal-changes` rule in `~/.config/opencode/AGENTS.md`"* or *"plan-steps conventions in your global rules"* is reaching for a file that exists on exactly one of the three machines.

**Two container-specific gaps that change the cost estimate.** First, the fix on the code plane is a copy, not a rewrite: dropping the eleven `cr-*.md` files into `$CHAT/home/.config/opencode/agents/` took `GET /agent` from 7 to 18, with model pins and permission maps intact, and the `task` tool then advertised all eleven by name. (The pinned models did not resolve in the probe — *"Model not found: opencode/claude-sonnet-5"* — but that container was unauthenticated and the config template's own default, `opencode/deepseek-v4-flash`, was equally unresolvable, so the catalog rather than the pin is what differed. **Whether the pins resolve in production is unverified.**) Second, an ordering bug worth fixing regardless of any of this: OpenCode auto-allowlists every discovered skill's directory under `external_directory`, then `Permission.merge(defaults, ..., user)` appends the user's config last and `evaluate` uses `findLast` — so the container template's `"external_directory": {"*": "deny"}` **revokes every auto-grant**. Observed live: `read` on `/chat/home/.agents/skills/code-review/checklists.md` was refused, `read` on a skill inside the workspace succeeded, and `bash head` on the identical refused path succeeded. So `code-review`'s `checklists.md` and `examples.md` and `refactor-planner`'s eight `references/*.md` are advertised to the model in `<skill_files>` and then denied to the tool that should open them.

Two places where the chat plane is the *better* one, incidentally: goose enumerates **all** of a skill's supporting files with resolved absolute paths and lets `load_skill(name: "skill-name/path")` address any of them, with no read-permission question; OpenCode samples with `ripgrep.find({ limit: 10 })` and always prints *"Note: file list is sampled."* — even when the list is complete, and even when it is empty (`tool/skill.ts:36-56`).

---

## 5. The recommendation

**One store. One shared type for skills only. Two views. A portability badge.**

### 5.1 Storage: keep the single directory

Do not split `~/.agents/skills`. It already works, both engines already read it, the owner already documented the intent, and splitting would mean maintaining two copies of files that are byte-identical today. The cost of sharing is that each engine advertises the other's skills — accept that and solve it in the UI (§5.4), not by forking the directory.

The two real storage gaps are deployment, not design, and both are cheap:

1. Ship skills to the brain. `deploy-vps.sh` installs `config.yaml`, `.goosehints`, `memory/`, `secrets.yaml` and nothing else; the copy loop already exists at `bootstrap-mac.sh:133-142`.
2. Ship skills **and agents and a rules file** into chat volumes. `render_chat_config` is the only writer; adding two more copy steps there makes `code-review`'s fan-out real (proven: 7 agents → 18).

### 5.2 Library: one skills crate, no unified command abstraction

- **Skills: share.** The intersection of the two wire formats is `{name, description, body, location}` — which is exactly what both `GET /skill` (keys `['content','description','location','name']`) and goose's `sources/list` return. That intersection is not a lowest-common-denominator compromise; it *is* the artifact. Model it once. This app already has the goose half (`crates/goose-acp-client/src/goose/skills.rs`, 605 lines; `src/skills.rs`, 215 lines); the code half is one new endpoint on `crates/opencode-client/src/lib.rs`, which today has `GET /agent` but no `/skill`.
- **Recipes and commands: do not share.** Any common type would have to be the intersection of a 13-field bundle and a 6-field template — which is `{name, description, body}`, i.e. the skill again. Building a "unified command" type would buy nothing and would invite exactly the silent-swallow failure demonstrated above. Keep `goose::recipes` as it is; if the code plane ever gets commands, give them their own type.
- **Extensions and MCP: do not share, and do not merge the screens.** goose's extension list contains the shell; OpenCode's MCP list is `{}` in every container the owner runs. The honest code-plane counterpart of the Extensions screen is a *permissions and tools* screen, not an MCP screen.

### 5.3 The badge, concretely

Three states on the skill row, plus a hazard marker:

- **Runs here** — every proper noun in the body resolves on the connected plane.
- **Reads here** — it will load, and its prerequisites are missing. Show what is missing: *"Needs: 11 agents, GitLab MCP"*.
- **Not for this plane** — declared, or structurally impossible (e.g. a skill requiring `_goose/unstable/*` shown in a code chat).

Two inputs, because neither alone is enough:

**Declared.** `metadata.plane: [chat, code]` in the frontmatter. This survives goose's parser: `SkillFrontmatter.metadata` is a free-form `HashMap<String, Value>` (`skills/mod.rs:32-36`) copied straight into `SourceEntry.properties` (`skills/mod.rs:379`), which is on the ACP wire (`goose-sdk-types/src/custom_requests.rs:1409-1412`) and **is already modelled in this app** (`crates/goose-acp-client/src/goose/skills.rs:105-108`). So the chat plane can read a declared plane today with zero server change. The code plane cannot: OpenCode's loader keeps only `name` and `description`, and `GET /skill` serves the body with frontmatter stripped. Declaration is therefore advisory, not sufficient.

**Derived.** Cheap, exact, and computed client-side against what the connected plane actually reports:

| probe | chat plane source | code plane source |
|---|---|---|
| tool names in the body | `_goose/unstable/tools/list` | `GET /experimental/tool/ids` |
| named subagents | `load`/`delegate` source list | `GET /agent` |
| `provider/model` slugs | goose provider config | `GET /provider` |
| MCP requirements | extensions list | `GET /mcp` |
| git commands | is the session working dir a checkout? | always yes |

A body that says `` `cr-planner` agent via the task tool `` against a `GET /agent` that returns seven natives is a *Reads here* with a precise missing-list. That is the badge earning its keep: it turns "this might not work" into "copy eleven files".

**Hazard marker, plane-independent.** Flag a skill whose body contains `` !`...` `` or a bare `@path` — because on the code plane, running it as a command executes shell and reads files with no permission check (demonstrated in §2.1) — or a bare `$1`/`$2`, because goose substitutes those over the whole body whenever arguments are supplied. This is the one piece of UI that is genuinely about safety rather than convenience.

### 5.4 What the UI should say about the difference

The Skills screen's existing stance is the right one and should be extended, not changed. `src/skills.rs:1-9` already says the detail view exists because *"reading a skill before invoking it is the difference between trusting the agent and checking it."* Add to that:

- **Never offer a bare "Run this skill" on the code plane** without saying it will execute embedded shell. The two entry points are not equivalent and the UI is currently the only place a person could learn that.
- **Show the plane badge on the list row**, not just the detail — the misleading case is the one you never open.
- **Show origin.** A skill loaded from `~/.agents/skills` is shared; one from `$workspace/.claude/skills` came with the repo. Different trust, different lifetime.

### 5.5 Two small corrections in this repo, while you are here

- `crates/goose-acp-client/src/goose/skills.rs:91` documents `content` as *"The body of `SKILL.md`, frontmatter included."* goose strips it: `parse_skill_content` sets `content: body` (`goose/crates/goose/src/skills/mod.rs:345-380`). The frontmatter arrives separately, as `properties`.
- `sources/list` filters to `SourceType::Skill` (`goose/crates/goose/src/sources.rs:879-883`), so `BuiltinSkill` entries never reach the phone — `goose-doc-guide` appears in `goose skills list` and will never appear in the app's Skills list. Worth a doc line so it does not read as a bug later.

---

## 6. What would change this

- **A `plane:` or `compatibility:` field that both engines read.** Today neither does; the owner's `AGENTS.md:381` mentions `compatibility` as an allowed field but no reader exists on either side. If OpenCode's loader kept unknown frontmatter keys (it copies only four fields, `skill/index.ts:134-139`) and served them on `GET /skill`, the declared half of the badge would work on both planes and the derived half would become a cross-check rather than the primary signal.
- **A web tool on goose.** It is the single change that would move `deep-research` from "Tier 1 only" to fully portable, and it would make `mr-review`-shaped MCP status skills the highest-value chat artifacts in the corpus. Today the only path is enabling `tavily` with a proper `available_tools` list.
- **A shared name for "spawn a subagent," or a goose agent directory that overlaps OpenCode's.** goose reads `~/.agents/agents/`; OpenCode's 30 files live in `~/.config/opencode/agents/`. One directory over, plus one `name:` line per file, and (b)-class bindings mostly dissolve. What would *not* dissolve: model slugs and `permission: {edit: deny}`, both discarded by goose's agent parser.
- **Naming a tier instead of a slug.** If the skills said `tier: deep` rather than `opencode/claude-sonnet-5`, most of the (c)-class bindings vanish — both planes already run the same three-tier routing and `docs/model-routing.md` is already the shared source of truth.
- **Deployment.** Skills on the brain, and skills + agents + a rules file in chat volumes. Until then the portability question is partly academic: the code skills do not run on the code plane either.
- **Version drift.** Production runs OpenCode 1.18.25; the checkout read here is 1.18.5. Every behavioural claim above was executed against 1.18.25 in the production image, but static line numbers in `packages/opencode/src/**` are from 1.18.5 and may have moved. There is a second, plugin-driven pipeline forming in `packages/core` (`SkillV2`, `config/plugin/skill.ts`); if a future release cuts over, re-verify the frontmatter predicate and duplicate resolution.

---

## Appendix: what is verified and what is not

**Executed:** both engines' own listers on this machine (`goose skills list`, `opencode debug skill`) enumerating the same eleven directories; five skills loading from four roots in a container built from the production Containerfile running `ghcr.io/anomalyco/opencode:latest` (1.18.25), with byte-diffed bodies; the shell/`@file`/`$1` expansion split between the `skill` tool and the command path, including the `external_directory` bypass; `GET /agent` going 7 → 18 after copying eleven agent files; goose's parameter bijection in both directions; goose silently swallowing `subtask`/`agent`/`model`/`variant`/`template` and rendering byte-identically; `$ARGUMENTS` never substituting in a translated recipe; goose's live 16-tool list over `goose acp`; OpenCode's 14 ids over `GET /experimental/tool/ids`; goose dropping nameless agent files and accepting them after a one-line `name:`.

**Static reads only** (I re-opened and confirmed the citations in `goose/crates/goose/src/{skills/mod.rs, skills/arguments.rs, recipe/mod.rs, agents/extension.rs, agents/platform_extensions/summon.rs, sources.rs, slash_commands/skill_slash_command.rs, acp/server/sources.rs}`, `opencode/packages/{core/src/v1/config/{command,mcp}.ts, opencode/src/{skill/index.ts, command/index.ts, tool/skill.ts, tool/shell/id.ts}}`, and `personal-ai-setup/{config/goose/config.yaml, config/code-agents/opencode.json, config/skills/**, recipes/connect-service.yaml, scripts/mac/bootstrap-mac.sh, scripts/vps/{deploy-vps.sh, code-agent-manager.py}, scripts/verify/check-security.sh}`): the OpenCode permission ordering derivation, the timeout unit mismatch, the `mcp-auth.json` OAuth-only claim, the per-model tool filtering that removes `websearch`/`apply_patch`, and the claim that goose's `sub_recipes` mechanism is reachable from inside a skill body.

**Explicitly unverified:** whether `opencode/claude-sonnet-5` and `opencode/kimi-k2.6` resolve in a *production* container (the probe was unauthenticated by choice — seeding `auth.json` would have meant reading the owner's `OPENCODE_ZEN_API_KEY`); whether a real model, as opposed to a scripted mock endpoint, would choose to follow any of these skills; whether `websearch` is offered to an Anthropic-family model in the container (it was absent with `together/gpt-oss-120b`); goose's up-walk behaviour and duplicate-name resolution at runtime.

**Repository state:** no repository was modified. `git status --porcelain` is empty in `/Users/phillipchaffee/git/goose`, `/Users/phillipchaffee/git/opencode`, and `/Users/phillipchaffee/git/personal-ai-setup`; `/Users/phillipchaffee/git/goose-phone-app` shows only its pre-existing untracked `.probe-tmp/`.