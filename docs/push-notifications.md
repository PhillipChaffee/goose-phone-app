# Push notifications

How a phone in a pocket finds out that a turn finished, or that an agent is
parked waiting for permission. This is a design, not a changelog: nothing here
is implemented yet. Read it before writing any of it, because the two most
expensive mistakes in this area are both invisible at build time — an
entitlement that is only rejected at install, and a payload that is only wrong
after it has already left the tailnet.

Where something is uncertain it says so, with the experiment that settles it in
[§9](#9-what-would-falsify-this).

## 1. The feature, from the user's side

You start a long task from your phone, lock it, put it in your pocket. Twenty
minutes later the phone buzzes once. The lock screen says either **"Goose: a
turn finished"** or **"Goose: 1 agent is waiting on you"** — the second is the
urgent one, because a blocked agent is doing nothing at all until you answer.
You unlock, the app opens, and the ask (or the finished transcript) is there.

Two events, and only two:

| event | urgency | why |
|---|---|---|
| turn ended | ordinary | the work is done; nothing is waiting on you |
| blocked on a permission ask | urgent | the agent has stopped, and it stays stopped |

The second row is a claim about the **code plane only**, and that is not a
scoping convenience — it is measured. On the goose plane a blocked agent does
not stay stopped; the round it was building is destroyed and the ask stops
existing (§2, and `docs/permission-durability.md` §0). This document used to
assert something different and wrong about the goose plane, and §2 records the
correction rather than hiding it.

The notification carries **no session title, no repo name, no tool arguments,
no model output**. It says a count and a kind, and the app fetches the truth
over the tailnet once it is open. That is not timidity — §7 shows that every
field a designer instinctively reaches for is contaminated, and §2 of
`docs/privacy.md`'s delivery-channel rule ("counts and neutral titles only")
already governs this exact channel.

Tapping it opens the app. On the first stage it opens the app's default screen
and you navigate; deep-linking to the right session is a later stage and is
[blocked on something specific](#the-deep-link-is-dead-today).

## 2. Why this does not work today

Three independent reasons, and fixing only the third fixes nothing.

**The socket dies.** The client learns about progress from
`_goose/unstable/session/update` on a WebSocket. iOS suspends a backgrounded
app's process shortly after it leaves the foreground, at which point its file
descriptors stop being serviced and the connection drops. The app already
assumes this — `reconnect_loop` (`src/state.rs:688-707`) exists precisely
because of it, and `docs/iphone-setup.md`'s troubleshooting section documents
"the app says Connection lost after the phone was in your pocket" as expected
behaviour. Tailscale does not help: the tunnel runs in a NetworkExtension
process with its own lifecycle and survives your suspension, but it grants your
app no runtime it would not otherwise have. (Apple's framing in DTS forum
answers is running-vs-suspended rather than foreground-vs-background; the
mechanism is not in doubt, the exact wording of those threads is secondhand.)

**goose has no push endpoint and no notion of a device.** The ACP surface has
one agent→client notification and, in goose's own `_goose/unstable` namespace,
one agent→client request (`crates/goose-acp-client/tests/fixtures/acp-meta.json`,
checked by `crates/goose-acp-client/tests/contract.rs:11-20`). Base ACP adds a
second agent→client request, `session/request_permission`. None of them carries
a device token, and there is nothing to register one with.

**And — the finding that reframes the whole feature — the work does not survive
the socket either.** On the goose plane a pending permission ask has no
server-side existence at all. It is a closure living inside an in-flight
JSON-RPC call:

```rust
// goose/crates/goose/src/acp/server.rs:1300-1325
cx.send_request(permission_request)
    .on_receiving_result(move |result| async move {
        match result {
            Ok(response)  => agent.handle_confirmation(request_id, outcome_to_confirmation(&response.outcome)).await,
            Err(e)        => agent.handle_confirmation(request_id, PermissionConfirmation {
                                 principal_type: PrincipalType::Tool,
                                 permission: Permission::Cancel,
                             }).await,
        }
        Ok(())
    })?;
```

> **CORRECTION — this section was wrong, and it was written down as fact.**
>
> It used to read, with an arrow drawn at the `Permission::Cancel` line: "When
> the phone's transport dies, that request fails, and goose answers its own
> question with `Permission::Cancel`. […] **It silently denies the tool and
> kills the run.**" It cited `src/state.rs:670-673`'s comment as corroboration,
> and that comment was wrong too — it has since been corrected.
>
> That account has been **falsified by measurement**. See
> `docs/permission-durability.md` §0. goose 1.46.0 over the tailnet, a session
> in `mode = approve`, a client that received the ask and never answered, killed
> both by `kill -STOP` (frozen, socket still open — what iOS does to a
> backgrounded app) and by `kill -9` (fd closed). Sessions `20260827_3` and
> `20260827_4`. On reconnect, `session/load` replayed exactly four things:
>
> ```
> REPLAY UserMessageChunk(... "Run the shell command `uname -a` ...")
> REPLAY GooseUpdate usage_update
> REPLAY UsageUpdate
> REPLAY AvailableCommandsUpdate
> ```
>
> No `ToolCall`. No assistant message. No "declined". No tool response of any
> kind. There is no denied tool to notify anyone about, because there is no tool
> call left at all.

What actually happens is worse for this feature, not better: **the whole
provider round is discarded.** The user's prompt survives, and so does the
generated session title — the measured session is called "Run uname command" and
contains only the request to run it. Everything the round produced is gone.

So locking the phone mid-ask today does not merely lose the notification. **It
destroys the reply the agent was composing, and leaves a session named after
work with no record that the work was ever attempted.** A notification delivered
twenty minutes later would be describing a turn that stopped existing nineteen
minutes ago. No transport — APNs, ntfy, Telegram, email — changes that.

The conclusion this section was drawing is therefore unchanged and if anything
firmer. Only its mechanism was wrong, and it is worth noticing *how* it was
wrong: the source read was confident, quoted, line-cited and annotated with an
arrow, and none of that made it true.

The same is true of the ordinary event for a different reason: "turn ended" on
the goose plane is the resolution of the client's own `session/prompt` call
(`src/state.rs:1329-1336`). There is no server-side fact to query afterwards.

**The code-agent plane is the exact opposite, and that asymmetry is the plan.**
There, a pending ask is durable server state: `pending_permissions()` fans out
to every running container's `/permission` and returns the parked asks
(`personal-ai-setup/scripts/vps/code-agent-manager.py:335-378`), and the app
already treats it as authoritative, reconciling on connect and on a 10s timer
(`src/code.rs:389-411`). Turn-end is detectable too — `chat_busy()` polls each
container's `/session/status` and `reaper_loop()` already calls it for every
running chat on a 60-second cadence (`code-agent-manager.py:1060-1098`). A
busy→idle edge *is* "turn ended", and the poll that would detect it is already
running.

Ship against the plane that already has the property.

## 3. The architecture, and the two that were rejected

### Chosen: a notifier on the brain, content-free payload, pluggable delivery

One long-lived service on the box the user already runs, with the transport
behind a single `deliver(event)` seam so the ntfy and APNs eras are the same
code. Three properties, in priority order:

1. **The event is computed server-side, from state that outlives the phone.**
2. **The payload is content-free by construction** (§7), so the privacy question
   is answered once rather than per field.
3. **The transport is the last decision, not the first**, because it is the one
   gated on $99 and an `unsafe` carve-out.

It arrives in stages (§8), and stage 0 needs no new process at all.

### Rejected: a second WebSocket client that watches

The obvious shape — a watcher that connects to `goose serve` alongside the
phone and pushes what it sees — does not work, because a second connection is
blind. `create_acp_router_inner` hands the server a *per-connection* factory
(`goose/crates/goose/src/acp/transport/mod.rs:189`) and `create_agent` builds a
fresh `GooseAcpAgent` for every connection
(`goose/crates/goose/src/acp/server_factory.rs:64-107`), each with its own
`sessions` map, its own `active_prompt_runs` and its own `client_cx`
(`server.rs:210-212`). Every update goes out on the *owning* connection's
context (`server.rs:1084`, `:1167`, `:1711-1716`), so a second client receives
nothing about the first one's turn. `session/load` is not a live attach either —
it replays persisted history into the loading connection only
(`goose/crates/goose/src/acp/server/load_session.rs:25-61`). There is no
observe, no subscribe and no takeover primitive to build on.

And even if there were, §2's third reason stands: watching a turn that dies with
the phone tells you about a corpse.

### Rejected: change goose upstream

This is the durable end state and it is on the user's own roadmap
(`personal-ai-setup/docs/roadmap.md:98-111`), but the minimum useful change is
large: detach the run from the connection, share agents across connections, and
park permission asks server-side so any attached client can answer. That is a
substantial PR against a fast-moving upstream the user consumes as a pinned
release.

Worth correcting the roadmap while you are there: it cites goose's mobile-access
document as evidence that upstream is heading toward remote ACP plus push. That
page is now `unlisted: true` and says the feature was **removed** — "Mobile
access via secure tunneling is no longer available in current goose Desktop
builds" (`goose/documentation/docs/experimental/remote-access/mobile-access.md:1-11`),
and the Remote Access index now lists exactly one card, the Telegram Gateway
(`.../remote-access/index.md:11-19`). Upstream retreated *from* remote-ACP-plus-push
*toward* a messaging gateway. Do not plan around goose-native push arriving.

### Also rejected, briefly

- **BGAppRefreshTask / BGProcessingTask.** `earliestBeginDate` is a floor, not a
  schedule; Apple's docs do not guarantee the task runs at all, and reported
  observations on the Developer Forums have 60-second requests firing after 12
  to 21 minutes, or never. Even at its best it is strictly worse than a push at
  higher cost. Skip.
- **Live Activities.** The lifetime fits (8h active + 4h stale) and the Dynamic
  Island is the right surface, but it needs a SwiftUI WidgetKit *extension
  target*, and `dx` builds one app bundle from one Rust crate — there is no
  extension-target concept anywhere in `dioxus-cli-0.7.10`'s `src/build/apple.rs`
  or `src/config/manifest.rs`. It also needs ActivityKit push tokens to update
  while suspended, which is the same paid-account gate. Post-APNs polish at the
  earliest.
- **Silent (`content-available`) push as the delivery mechanism.** Apple states
  delivery is not guaranteed, the system discards held background notifications,
  and "don't try to send more than two or three per hour"
  (*Pushing background updates to your app*). It is an optimisation — "your
  transcript is stale, refresh when convenient" — never the thing that buzzes.
- **Email via the ntfy gateway.** Already wired (`notify.sh:65-68`), already
  free, and already capped: ntfy.sh's free tier forwards roughly 5/day, which
  `personal-ai-setup/docs/automations.md:141-143` names as "exactly why content
  does NOT travel this channel". Route turn-ends through it and the cap is gone
  in an afternoon, taking the failure-alert backstop with it. Keep it as the
  backstop it is; do not scale it.

## 4. The client half

### What is small

More of this is cheap than it looks.

- **tao's iOS delegate implements neither push selector**, so nothing needs
  swizzling. `create_delegate_class()` registers an ObjC class literally named
  `"AppDelegate"` with exactly eight methods
  (`tao-0.34.8/src/platform_impl/ios/view.rs:657-694`);
  `application:didRegisterForRemoteNotificationsWithDeviceToken:` is not among
  them, so there is no existing IMP to preserve — a plain `class_addMethod`
  graft is enough.
- **The dependencies are already here.** `objc2`, `objc2-foundation`,
  `objc2-ui-kit` and `block2` are in `Cargo.lock` transitively via wry
  (lines 3031, 3090, 3102, 226). Only `objc2-user-notifications` would be new.
- **`UIApplication::sharedApplication(mtm)` and `registerForRemoteNotifications()`
  are safe `pub fn`** in `objc2-ui-kit-0.3.2` (`src/generated/UIApplication.rs:222`
  and `:524`), as is the entire local-notification call path in
  `objc2-user-notifications-0.3.2`.
- **The bundle identifier is stable.** `bundle_identifier()`
  (`dioxus-cli-0.7.10/src/build/request.rs:2688-2707`) reads `[bundle] identifier`
  and nothing else — no per-build or debug/release mangling. `com.goosemobile.app`
  (`Dioxus.toml:8`) is the single source, it lands in `CFBundleIdentifier`, and it
  is therefore what `apns-topic` must be.
- **`dx` can emit the entitlement without a fork.** `aps_environment` is a
  first-class field (`dioxus-cli/src/config/manifest.rs:475-477`), emitted at
  `src/build/apple.rs:436-440`. One line in `Dioxus.toml`; no Xcode project edit,
  no checked-in plist.
- **Nothing here is a DOM event.** `design.md` rule on the native renderer's
  synchronous XHR is scoped to *listened-to DOM events* — that is why swipe-tray
  close and pull-to-refresh are JS strings injected through `document::eval`
  rather than Rust handlers (`src/viewport.rs:125-158`). UIKit and ObjC-runtime
  callbacks never touch the webview IPC and pay nothing. **No CSS changes and no
  gallery re-capture are implicated by anything in this document.**

### What needs `unsafe`, and what the carve-out is

Two things, and only two: `objc2::ffi::class_addMethod` (raw FFI,
`objc2-0.6.4/src/ffi/class.rs:78`) and decoding the token `NSData`. Everything
else on the path is a safe wrapper.

`unsafe_code = "forbid"` (`Cargo.toml:26`) cannot be lifted locally — that is
the whole difference between `forbid` and `deny`, and it is the mechanism the
lint policy in `CLAUDE.md` rests on. But `[workspace.lints]` is inert unless a
package opts in, and all four current members do so *explicitly*
(`Cargo.toml:82-83`, `crates/goose-acp-client/Cargo.toml:28-29`,
`crates/mock-goose-server/Cargo.toml:21-22`, `crates/opencode-client/Cargo.toml:20-21`).

**The carve-out is therefore a new workspace member, `crates/ios-push/`, that
omits `lints.workspace = true` and states its own table** with
`unsafe_code = "allow"` and a written reason in the style of the exceptions in
the root `Cargo.toml`. `forbid` stays intact on the app and the three existing
crates. The unsafe surface is one small crate with a safe Rust API — an
`install()` and an `UnboundedReceiver<PushEvent>` — and every objc2 dependency
goes under `[target.'cfg(target_os = "ios")'.dependencies]` so the Linux CI job
builds it empty.

This is a deliberate relaxation of a stated policy and should be its own commit
with its own reason, not a line buried in a feature branch.

**The sharp edge:** CI's clippy gate is Linux-only
(`.github/workflows/ci.yml:37`) and the iOS job runs `cargo check --package
goose-mobile` (`ci.yml:127`), not clippy and not the whole workspace. As
written, **the one crate holding every unsafe line in the repo would be the one
crate no lint gate ever inspects.** Extend the iOS job to
`cargo clippy --target aarch64-apple-ios ... -- -D warnings` and drop the
`--package` filter in the same commit that adds the crate.

### The token capture, and why it is fragile

The UNUserNotificationCenter delegate — tap handling and foreground
presentation — is an object we own entirely, and it **must** be installed in
`fn main()` before `launch()`. `main()` already runs two statements there
(`src/main.rs:52-57`), and at that point `UIApplicationMain` has not been
called, which satisfies Apple's rule ("assign your delegate to the shared
UNUserNotificationCenter object before your app finishes launching",
*Local and Remote Notification Programming Guide*) with room to spare. That
placement is what makes a cold-start tap deliver.

The APNs token has no delegate other than `UIApplicationDelegate`, and tao owns
that with no seam anywhere:

- `EventLoop::new` calls `create_delegate_class()`
  (`tao/src/platform_impl/ios/event_loop.rs:112-124`) and `EventLoop::run` calls
  `UIApplicationMain(..., "AppDelegate")` (`event_loop.rs:159-165`);
  `dioxus-desktop-0.7.10/src/launch.rs:15-18` invokes them on two consecutive
  statements. `Config::with_custom_event_handler` only fires *inside* the run
  loop.
- Pre-declaring our own class named `"AppDelegate"` makes tao's
  `ClassDecl::new(b"AppDelegate\0", ...).expect(...)` return `None` and **panic
  at startup** (`view.rs:650-654`). The app would not launch at all.
- `did_finish_launching` discards the launchOptions dictionary (`view.rs:554-559`),
  so `UIApplicationLaunchOptionsRemoteNotificationKey` is permanently
  unreachable.

So the only route is `class_addMethod` against tao's live class, from a
`spawn_forever` at the app root (after `create_delegate_class()` has run), before
we call `registerForRemoteNotifications()` — which we control. Adding methods to
an already-registered class is legal in the ObjC runtime; only `class_addIvar`
is restricted to the pre-registration window. **I have not tested this against a
class whose instance is already live and whose method caches are warm**; see §9.

This is a load-bearing dependency on a private, unversioned class-name string
literal in tao, and `view.rs:55-75` carries TODOs saying tao intends to migrate
to objc2's `define_class!`. That migration would break the graft **with no
compile error** — just a token that never arrives. Mitigations, all cheap and all
required:

- `AnyClass::get(c"AppDelegate")` returning `None` is an *error*, not a no-op.
- `class_addMethod` returning `NO` is an error.
- `registerForRemoteNotifications()` is called only if both succeeded.
- Failure surfaces as a visible in-app "push unavailable" state, so a tao bump
  shows up as a UI regression rather than silence.
- Pin tao's version explicitly; treat a tao bump as requiring a device re-test.

### What the build needs

`Dioxus.toml` gains one line:

```toml
[ios.entitlements]
aps-environment = "development"
```

No Xcode project edit, no checked-in `.plist` or `.entitlements`. `[background]
remote-notifications = true` is *not* needed unless silent push is later used as
an optimisation.

Three build-side traps, all silent, all worth a preflight script:

1. **`dx --entitlements <path>` silently disables the config.**
   `enrich_entitlements_from_config` — the only thing that emits
   `aps-environment` — runs inside `if entitlements_file.is_none()`, where
   `entitlements_file` starts as the value of the `--entitlements` flag
   (`dioxus-cli/src/build/apple.rs:295-311`, flag at `src/cli/target.rs:135`).
   Pass that flag once and the app builds, signs, installs and runs while
   registration just fails. **Never use `--entitlements` on this project.** Assert
   it: after a build, run `codesign -d --entitlements - <app>` and fail if
   `aps-environment` is absent.

2. **`dx` never reads a provisioning profile's expiry.** Its `ProvisioningProfile`
   struct deserializes four keys — TeamIdentifier, Entitlements,
   ApplicationIdentifierPrefix, ProvisionedDevices — and there is no
   `ExpirationDate` anywhere in the CLI source (`apple.rs:653-663`). Selection
   ranks by exact-app-ID match, then by provisioned-device count, then keeps
   whichever `read_dir()` yielded first (`apple.rs:769-780`). On a free Personal
   Team the profile expires every 7 days (`docs/iphone-setup.md:275-284`), so the
   folder accumulates expired copies for the same bundle ID and `dx` can
   deterministically prefer a stale one. Worse: the day Push is enabled on the
   App ID and the profile re-minted, the old pre-push profile is still there and
   does not allowlist `aps-environment`.
   **Prune the profiles folder to exactly one profile for `com.goosemobile.app`
   before every device build**, and preflight it with `security cms -D -i <profile>`,
   failing on a past `ExpirationDate` or a missing `aps-environment` entitlement.
   Upstream, `dx` should skip expired profiles — worth an issue.

3. **Install failure is loud; runtime failure is not.** `dx` installs via
   `xcrun devicectl device install app` (`src/build/builder.rs:1256-1262`), which
   rejects an entitlement the embedded profile does not allowlist. So a mismatched
   entitlement most likely fails at *install*, not at
   `didFailToRegisterForRemoteNotifications`. Good — but it means the install log
   is the thing to read, and the chosen profile path should be recorded in the
   build log.

### The deep link is dead today

`dioxus-desktop` never matches `tao::event::Event::Opened` — grepping "Opened"
across `dioxus-desktop-0.7.10/src/` returns nothing. tao raises it for custom URL
schemes and universal links only (`view.rs:562-612`), and the only way to see it
is `Config::with_custom_event_handler` (`config.rs:216-221`), which the iOS
branch of `launch()` cannot reach because it calls bare `dioxus::launch(app::App)`
with no `Config` (`src/main.rs:158-159`).

Consequence: **any URL-scheme route into the app is dead as written**, which
specifically kills the `Click: goosemobile://...` header that would make an
ntfy notification land on the right session instead of inside ntfy's own app.
Fixing it means rewriting the iOS `launch()` as
`dioxus::LaunchBuilder::mobile().with_cfg(Config::new().with_custom_event_handler(...))`
and matching `Event::Opened { urls }` there, pushing onto the same channel the
push pump drains. (`dioxus::mobile` is a re-export alias of `dioxus_desktop` —
`dioxus-0.7.10/src/lib.rs:113` — so `Config` is the same type.) Plus
`CFBundleURLTypes` via `[ios.raw] info_plist`. Ship stage 0 without a `Click`
header rather than pretending.

### The tap, once it works

Reuse the shape `state.rs` already has: the ObjC callback (main thread) pushes
onto an `UnboundedSender` held in a `OnceLock`, and a `spawn_forever` pump at the
app root drains it exactly the way `pump(&ctx, events)` drains `AcpEvent`
(`src/state.rs:614`, `:633`).

Two things that are not one-liners:

**`open_session` cannot be called with an id.** Its signature is
`open_session(ctx: &AppCtx, info: SessionInfo)` (`src/state.rs:1150`) and it reads
`info.cwd`, `info.display_title()` and `ctx.running_sessions`. On a cold start
from a tap — the primary case — `ctx.sessions` is empty, so the pump needs a
fetch-then-open path with its own loading and not-found states. That path does
not exist today. Note the payload cannot help here (§7 forbids carrying the
title), so it is a real refresh-then-find.

**Foreground presentation must be decided synchronously on the main thread.**
iOS suppresses the banner for a foregrounded app unless
`userNotificationCenter(_:willPresent:withCompletionHandler:)` calls the
completion block with non-empty options; `objc2-user-notifications-0.3.2`
exposes exactly that, with `UNNotificationPresentationOptionNone` as the
suppress value (`src/generated/UNUserNotificationCenter.rs:216-232`). That
callback cannot read a Dioxus `Signal`, so mirror the state out: a
`Mutex<Option<String>>` holding the currently-open session id, updated by a
`use_effect` watching `ctx.chat` and `ctx.screen`. This changes once per session
switch, not per frame, so it does not offend the rule it superficially
resembles.

## 5. The server half

### Stage 0 lives inside the manager; stage 1 is a new unit

For the code plane, the watcher is already running. `reaper_loop()` wakes every
60 seconds, and for each running chat it already calls `chat_busy()`, which
talks straight to `127.0.0.1:<chat.port>` (`code-agent-manager.py:1088-1098`).
The permission side has an identical fan-out that also goes direct
(`pending_permissions()`, `:335-378`). **Going direct is not an implementation
detail — it is required**, because reaching a chat through the proxy calls
`touch_maybe` (`:1140`), which defeats the idle spin-down the whole code plane
rests on. `pending_permissions`'s own docstring explains this. Any notifier that
polls `/chat/<id>/...` on a cadence under `IDLE_SECONDS` (900, `:91`) pins every
container forever and starts returning 409 at `MAX_ACTIVE` (2, `:92`).

So stage 0 is a previous-state map in the reaper plus a call to a sender. No new
process, no new client, and — importantly — no second HTTP client racing the
phone through `proxy()`, which matters because of the next paragraph.

**Fix the deadlock first.** `_lock = threading.Lock()` (`:291`) is
non-reentrant. `wake_chat` acquires it at `:1021` and, still inside the `with`
block, calls `touch(chat_id)` at `:1028`, which re-acquires at `:1052`. That is
an unconditional self-deadlock on the `state == "running"` path, and the lock is
then held forever: every later `touch`, `create_chat` (`:933`), `route_delete_chat`
(`:1393`) and `wake_chat` blocks, including the reaper's own `touch(cid)` at
`:1094`. One hit wedges the manager until restart and idle spin-down stops with
it. Today it is reachable only through the check-then-wake race in `proxy()`;
`CodeClient::wake_chat` (`crates/opencode-client/src/lib.rs:1192`) POSTs
`/api/chats/<id>/wake` unconditionally, which is a 100% deadlock on an
already-running chat, and nothing in `src/` calls it yet — that is the only
reason this has not fired. **A push synchronises clients**: the notification
lands, the user taps, and everything hits the same chat within the same second.
Make it impossible before adding any second reader: `threading.RLock()`, or
inline touch's two lines into `wake_chat`'s critical section.

For the goose plane, stage 1 is a new unit, `goose-notifier.service`, a Rust
binary reusing `crates/goose-acp-client` verbatim — it is already UI-independent
(tokio + tungstenite + rustls, fingerprint pinning in
`crates/goose-acp-client/src/tls.rs`), and `crates/mock-goose-server` is the
precedent for a non-app binary in that workspace. It cannot live inside
`code-agent-manager.py`: that service is deliberately stdlib-only under
`mypy --strict` and the full ruff rule set (`code-agent-manager.py:55-84`), and an
ACP client means TLS-pinned WebSocket plus JSON-RPC.

Its socket hygiene is already solved by the crate it reuses: a 30s keepalive
ping with drop-after-missed-pongs (`client.rs:296-318`), no timeout on
`session/prompt` (`:213-214` — correct, a turn is unbounded), and pending
requests drained on close (`:330`), so a goose restart surfaces as an error the
notifier can act on rather than a hang. Reusing the crate is also safe on
capabilities: it advertises `fs: {readTextFile:false, writeTextFile:false},
terminal:false` (`client.rs:181-185`), so `apply_acp_extension_overrides`
(`server.rs:830-840`) early-returns and goose keeps running its tools
server-side. The notifier will not find itself asked to read files on the
brain's behalf.

### The notifier must own the prompt, not watch it

Because of §2, "watch and push" is not available. The notifier is the connection
that calls `session/prompt` on the phone's behalf:

- `POST /api/sessions/<id>/prompt {deviceId, blocks[]}` returns immediately.
- `GET /api/sessions/<id>/events` (SSE) mirrors `session/update` to a
  foregrounded phone.
- `session/request_permission` asks are parked in the notifier's memory and
  answered by `POST /api/permissions/<request_id> {optionId}`, which the notifier
  turns into the JSON-RPC result on the socket it still holds.

Because the notifier never disconnects, the ask survives the phone sleeping
instead of being auto-cancelled. This is the same architecture the user already
runs for code agents, deliberately.

**One prompt path, not two.** The only guard against two concurrent turns on one
session is per-connection: `start_active_run` checks only its own
`active_prompt_runs` map (`server.rs:1592-1620`, map at `:210-212`). So the
notifier's connection and a directly-connected phone can both start a run on the
same session id with no error from either, and both then append to the same
session row (`goose/crates/goose/src/agents/agent.rs:1734`) — two interleaved
conversations in one transcript, no diagnostic anywhere. If the phone keeps a
direct WebSocket for interactive latency, **it must lose its own prompt path**;
half-and-half is the one configuration that corrupts transcripts.

### The Monday-morning hole

The notifier buys durability against the *phone* sleeping. It buys nothing
against *goose* restarting, and goose is restarted on a schedule:
`tls-cert-renew.timer` fires `OnCalendar=Mon *-*-* 05:00:00` with
`Persistent=true`, and `renew-tls-cert.sh:27` runs an unconditional
`systemctl restart goose-serve` with no busy check. `goose-serve.service` is also
`Restart=always`.

A task started Sunday evening, or an ask parked overnight, is dead by Monday
05:00. Two cheap fixes, both worth doing: gate the restart on a `/api/busy`
endpoint the notifier can answer (it owns every prompt, so it knows), and scope
the promise in the docs — a parked ask survives the phone, not the server.
Anything stronger needs the upstream change in §3.

### The events, precisely

**Code plane.** `permission.updated` / `permission.asked` and `session.idle`, the
tags the client already models (`crates/opencode-client/src/lib.rs:928-947`), or
at stage 0 the reaper's own `pending_permissions()`/`chat_busy()` diff.

**goose plane, turn ended.** A `session/update` with
`sessionUpdate: "session_info_update"` whose `_meta.goose.activeRunId` is null.
`send_active_run_update(cx, id, Some(run_id))` fires when the turn starts
(`server.rs:1815`) and `..., None)` when it ends (`:1960`), both built by
`active_run_meta` (`:1692-1704`).

**And the naive reading of that fires mid-turn.** Three different producers emit
`SessionInfoUpdate` on the same connection:

| producer | `_meta` shape | line |
|---|---|---|
| `active_run_meta` | `{goose: {activeRunId: <id>\|null}}` | `server.rs:1692-1704` |
| `send_queued_steer_update` | `{goose: {queuedSteer: {...}}}` — no `activeRunId` | `server.rs:1720-1740` |
| `spawn_session_name_update_notifier` | `{messageCount, userSetName}` — no `goose` key | `server.rs:265-296` |

The client parses `_meta` as a raw `Option<Value>`
(`crates/goose-acp-client/src/types/session.rs:9-16`), and in `serde_json`
indexing a missing key yields `Value::Null`. So
`meta["goose"]["activeRunId"].is_null()` is `true` for **all three** — a watcher
written that way pushes "turn ended" seconds after the turn starts, when the
auto-generated title lands (`agents/agent.rs:1745-1756` spawns it on the first
turn), and again on every steer.

Match presence-and-null explicitly:

```rust
meta.get("goose").and_then(|g| g.get("activeRunId")) == Some(&Value::Null)
```

Never `is_null()` on an indexed path. Latch the run id from the matching
`Some(String)` update and clear it on the null one — the end-of-turn update
carries a literal null, so the `(sessionId, runId)` dedupe key has nothing to key
on otherwise. Worth a fixture test against all three shapes, because all three
are silent false positives rather than errors.

**goose plane, permission.** The agent→client request
`session/request_permission` (`server.rs:1264-1300`), which the client already
models and dispatches (`crates/goose-acp-client/src/client.rs:520-545`).

**Scheduled runs emit nothing, and that is fine.** Scheduled sessions build their
agent directly with no `ConnectionTo<Client>` (`goose/crates/goose/src/scheduler.rs:1025-1032`),
so a connection-owning notifier never sees them — the feared "every cron job
pushes too" is false. The inverse trap is real, though: `session/list` *does*
surface them, because `ACP_SESSION_LIST_TYPES` defaults to
`[User, Scheduled, Acp]` (`goose/crates/goose/src/acp/server/list_sessions.rs:13-14`),
so **never fall back to polling `session/list`** without passing
`sessionTypes: ["user","acp"]`. And do not promise permission notifications for
scheduled work at all: those sessions are forced `GooseMode::Auto`
(`scheduler.rs:1030`, `:1065`), so they auto-approve and there are no asks to
deliver. `goose serve` runs with `--enable-scheduler` today
(`goose-serve.service:41`).

### Dedupe, and not sending five pushes for a five-tool turn

Key on identifiers the protocols already mint, so it is not a heuristic.

- **Turn ended:** one push per `(sessionId, runId)`, fired on the
  `activeRunId → null` transition. A run id spans the whole multi-tool turn by
  construction, so a five-tool turn is one push for free — no timer, no debounce.
- **Permission:** one push per `(sessionId, toolCallId)`, never coalesced and
  never suppressed by a quiet window. A blocked agent is doing nothing.
- **Suppress turn-ended** if that device currently has an SSE stream attached to
  that session — the notifier serves that stream, so it knows the user is
  looking at the screen. Plus a per-device floor (30s) on turn-ended only.

### State, and where it lives

One atomically-written JSON file on the LUKS volume, `/data/notifier/devices.json`,
following `Index.load()/save()`'s convention exactly — write `.tmp`, then
`tmp.replace(path)` (`code-agent-manager.py:191-217`) — with `RequiresMountsFor=/data`
in the unit so it cannot start against an unmounted volume.

Four things:

1. **Devices:** `{token, platform, environment, appBuild, registeredAt, lastSeen,
   profileExpiresAt}`. A push token is a capability to buzz that handset, so the
   encrypted volume is the right home.
2. **Handle map:** the opaque per-notification handle → real session/chat id
   (§7). Handles expire.
3. **Session→device ownership**, so a push goes to the phone that started the
   work; unknown-origin sessions fan out to all registered devices.
4. **Delivered keys** for dedupe, and per-device quiet state.

**Key the registry on the token, not on a client-minted UUID.** The tempting
design is a stable `deviceId` the phone mints and stores, but the app's settings
go through `use_persistent("settings", ...)` (`src/state.rs:490`), which
`dioxus-sdk-storage` writes as a plaintext file in the platform data dir — on
iOS, inside the app container's Library/Application Support, which Apple includes
in iCloud and encrypted backups by default. A restored backup would carry that
UUID onto a different handset, and a registry that upserts by `deviceId` would
silently hand the new device the old one's registration and session ownership.
Treat an unseen token as a new device requiring fresh registration, never as a
re-registration. (Reasoned from Apple's documented backup exclusion rules, not
tested on a device — see §9.) If a stable device identity is ever genuinely
needed it belongs in the Keychain with `ThisDeviceOnly` accessibility, which is
objc2 territory and belongs in the same carve-out crate.

Worth noting in passing, since push adds a third item to that same backed-up
file: `secret_key` and `code_password` are already in it
(`src/state.rs:53-62`).

### Registration, and revocation

`POST /api/devices {token, platform, environment}` over HTTP Basic on the
tailnet, TLS from the same `/data/tls/{cert,key}.pem` the manager uses. Reuse
`OPENCODE_SERVER_PASSWORD` from `/data/secrets.env` rather than minting a new
secret — same box, same trust boundary, and the app already stores and sends it
(`crates/opencode-client/src/lib.rs:1001`; the manager compares with
`secrets.compare_digest`, `code-agent-manager.py:1148-1165`). The unavoidable
cost is one new base-URL field in `Settings` (`src/state.rs:53-62` holds exactly
two pairs today), because the manager's router only matches `/api/*` and
`/chat/<id>/*` on its own port (`:1204-1221`).

Register on **every app launch**, not once: Apple's *Registering your app with
APNs* says the token can change and the app must forward it to the provider each
launch. The server upserts.

**Revocation must ship with the feature, not after it.** Every credential in
`personal-ai-setup/docs/security.md:130-145`'s rotation table is a *pull*
credential — rotate it and the old client can no longer fetch. **A push token is
the opposite:** delivery happens over the public internet to Apple or ntfy, never
over the tailnet, and nothing re-checks authentication at send time. The
documented lost-phone drill ("Admin console → Machines… remove stale devices") does
not stop the leak, and rotating `OPENCODE_SERVER_PASSWORD` only gates
registration. On the ntfy path the one lever is global — rotate the topic, kill
every device at once. On APNs there is no lever short of hand-editing the file.

So: `DELETE /api/devices/<token-hash>`, an "Unregister this device" button in the
app's settings, and a `--forget-device` operator command usable from an SSH shell
without the phone in hand. Add a row to `security.md`'s rotation table whose
Notes column says explicitly that removing the tailnet device is **not**
sufficient. APNs `410 Unregistered` is cleanup, never revocation — it never fires
for a phone that is merely lost.

Also: **the 7-day profile expiry looks exactly like a push bug.** APNs neither
knows nor cares about provisioning profiles. Once the profile expires the app
stops launching, but the server keeps sending and the phone keeps buzzing — you
tap, and nothing opens, for up to a week, and specifically during the week you
have not been at the Mac, which is the window this feature exists to cover.
Mitigation: read `ExpirationDate` from `embedded.mobileprovision` at build time,
inject it as a compile-time constant, send it at registration, and have the
notifier stop pushing past it. At minimum, document it in the troubleshooting
section so a dead tap is not debugged as an APNs problem.

### Failure, and the rule that delivery never touches the work

`notify.sh` always exits 0 by design — "Notification loss must never fail the job
that produced the real work" (`scripts/common/notify.sh:56-63`) — and
`notify_failure` wraps its call in `contextlib.suppress`
(`code-agent-manager.py:1106-1118`). The notifier inherits that verbatim: log to
journald and carry on, never cancel a turn or an ask.

APNs specifics: `410 Unregistered` with a `timestamp` body field, or
`400 BadDeviceToken` — delete the device **only** when that timestamp is later
than `registeredAt`, or a re-registration racing a stale push deletes a live
device. `403 ExpiredProviderToken` → regenerate the JWT and retry once. **Log the
`apns-id` and the JSON `reason` on every non-200**, or a revoked key, a wrong
environment and "push doesn't work" are indistinguishable.

### Deployment

Same conditional-enable pattern as the two units already there
(`deploy-vps.sh:193-195`, `:222-226`): install unconditionally with
`install -m 644`, `systemctl enable --now` only when the credential is present.
Unit body copies `code-agent-manager.service`: `User=agent`,
`EnvironmentFile=/data/secrets.env`, `RequiresMountsFor=/data`,
`ExecStartPre=/usr/bin/mountpoint -q /data`, `Restart=always`. It must resolve
its bind address from `tailscale ip -4` and exit nonzero until one exists —
`docs/security.md:28-45` requires the socket to exist on the tailnet interface or
not at all. Outbound to Apple is fine; ufw is default-deny *incoming*.

**The APNs key does not fit the gate.** Both existing units are enabled by
`grep -q '^VAR=..*' /data/secrets.env`, i.e. the pattern assumes every credential
is a single-line env var. A `.p8` signing key is a file. Storage has precedent —
`renew-tls-cert.sh:22-25` writes `/data/tls/key.pem` 600 agent:agent on the LUKS
volume — but the gate, the `secrets.env.example` names-only file, and the
rotation table have no entry for a file-shaped credential. Gate on
`[ -s /data/apns/AuthKey_*.p8 ]` instead; keep the 10-character Key ID and Team
ID in `secrets.env` as ordinary vars so the `.p8` is the only file-shaped item;
add a rotation row. Two properties differ from everything already there: a
team-scoped key can send push for every app in the team and cannot be scoped
narrower, and it does not expire, so "annually as routine" has no forcing
function.

## 6. What Apple requires

**A paid Apple Developer Program membership ($99/yr). This is a hard
prerequisite for APNs and it gates stage 2 entirely.**

Apple's *Supported capabilities (iOS)* reference table has three membership
columns — ADP (paid), ADEP (paid Enterprise), and "Apple Developer" (free /
Personal Team). The **Push notifications** row carries a checkmark for ADP and
ADEP and an **empty cell** for the free column, while Background modes and
Keychain sharing carry checkmarks in all three — so the empty cell is meaningful,
not a rendering gap. (That table renders its checkmarks as `<figure class="icon
icon-checksolid">` elements, which is why a plain page fetch looks blank in all
three columns and why this question is so often answered wrongly from memory.)
Apple DTS states it directly on the Developer Forums: push notifications and
iCloud capabilities are part of Apple Developer Program membership. Xcode's own
refusal is "Personal development teams… do not support the Push Notifications
capability."

This repo is built end-to-end around a free Apple ID: `docs/iphone-setup.md:105-124`
walks through creating the signing assets on a Personal Team, and Appendix B
(`:275-284`) documents the 7-day profile expiry that comes with it.

**There is no workaround.** The gate is enforced twice, both server-side at
Apple: the App ID cannot have the push service enabled, and TN3125 makes the
profile's entitlements an **allowlist** the device enforces against the
signature — so hand-writing `aps-environment` into the signature (which `dx` will
happily do, `apple.rs:806-826` reads only four keys out of the profile) produces a
bundle that will not install. Sideloading via Sideloadly or AltStore re-signs with
the same free Apple ID and inherits the same restriction.

**How to check in five minutes, without spending anything:** open the
`GooseSigning` Xcode project from `docs/iphone-setup.md` Step 4, go to Signing &
Capabilities with the Personal Team selected, click **+ Capability** and add
**Push Notifications**. Read the error. If it says personal teams do not support
the capability, the answer above is confirmed for this machine and this account.
A second check, also five minutes: sign in at developer.apple.com → Certificates,
Identifiers & Profiles → Identifiers → register an App ID and see whether the
Push Notifications checkbox is enabled.

**While you are there, check that `com.goosemobile.app` is available.** Free
Personal Team bundle IDs are never registered globally; explicit App IDs on a
paid program *are* globally unique across all Apple accounts. A collision is
invisible today and appears only on the day you enrol. If it is taken, the bundle
ID must change — and the bundle ID *is* the `apns-topic`, so every stored token
becomes `400 DeviceTokenNotForTopic` and the app has to be deleted and
reinstalled. Even without a collision, enrolling changes the Team ID, and iOS
will not install an app signed by a different team over the existing one;
deleting to proceed invalidates the token. **Either way the token registry is
flushed at exactly the moment the feature goes live** — budget it in the
enrolment runbook, and prefer a reverse-DNS identifier under a domain the user
actually controls.

Other Apple-side facts that shape the design:

- **Environment.** A `dx serve --ios --device` build signed with a development
  profile is in the **sandbox** environment; the sender must use
  `api.sandbox.push.apple.com`. TestFlight and production profiles use
  `production` (*APS Environment Entitlement*). Mismatch yields `400 BadDeviceToken`
  and silence. Store the environment per token — they are indistinguishable by
  inspection.
- **Credential.** Prefer the `.p8` authentication key over a per-app certificate:
  stateless, usable from multiple provider servers, and it does not expire
  annually (*Establishing a token-based connection to APNs*). One Sandbox-scoped
  team key is the right shape while on a development build.
- **Wire.** HTTP/2, `POST /3/device/<hex-token>`, headers `apns-topic` (the bundle
  id), `authorization: bearer <JWT>`, `apns-push-type: alert`, `apns-priority: 10`
  for a blocked ask, `apns-id` (generate it, so errors are traceable),
  `apns-expiration` nonzero so it survives the pocket, `apns-collapse-id` so a
  later turn-end replaces rather than stacks. APNs stores exactly **one**
  notification per bundle id, so without a collapse id a turn-end can silently
  displace a permission ask. JWT is ES256 only; refresh no more than once every
  20 minutes and no less than once every 60.
- **Sender library.** Python's stdlib has no HTTP/2 client and APNs dropped the
  legacy binary protocol in 2021, so a stdlib-only sender is impossible — which is
  a second reason the sender does not belong inside `code-agent-manager.py`.
  `httpx[http2]` + `PyJWT[crypto]` is closest to the existing hand-rolled style;
  `aioapns` is the maintained batteries-included option.

**Android/FCM is a separate, larger project and is out of scope here.** There is
no paid-tier gate, but there is a build gate: `dx 0.7.10` has no field that places
`google-services.json`, and the embedded root `build.gradle.kts` pins a buildscript
classpath (AGP + Kotlin only) that cannot resolve the
`com.google.gms.google-services` plugin id. It also needs a Kotlin
`FirebaseMessagingService` bridged back to Rust over JNI and a second
credential type (service-account OAuth2). And it may be solving a problem Android
does not have: Android does not suspend processes the way iOS does, so a
foreground service holding the existing WebSocket is a legitimate design there.
Nothing Android-specific has been exercised in this repo anyway — CI has no
Android job and `design.md` already says the Android text-scaling answer is not
wired up.

## 7. Payload and privacy

**The payload is content-free by construction:**

```json
{ "kind": "ask" | "turn", "handle": "<opaque random>", "count": 1 }
```

with a fixed neutral title — "1 agent is waiting on you", "A turn finished".
Nothing else. The tap opens the app; the app exchanges the handle for the real
session or chat id over the tailnet, then fetches the truth.

This is not a degraded compromise, it is the correct design, and it is *already
functionally sufficient*: `refresh_code_permissions` treats the manager's
`/api/permissions` as authoritative and reconciles on every poll
(`src/code.rs:388-411`). The push only has to say "go look".

### Why every obvious field is contaminated

- **Ask metadata** is an arbitrary `Value` (`crates/opencode-client/src/lib.rs:357`)
  that the app renders verbatim with `serde_json::to_string_pretty`
  (`src/views/code.rs:541`). For a bash ask that is the literal shell command —
  the crate's own fixtures show `{"command": "git push"}` (`lib.rs:1871`,
  `:2418`) — and for an edit ask, the file path and the diff.
- **The chat label** is `ChatMeta.title`, which the manager defaults to
  `(request.task or chat_id)[:80]` (`code-agent-manager.py:947`) — the first 80
  characters of the user's raw prompt, verbatim.
- **A goose session title** is `generate_session_name(provider, ...)`
  (`goose/crates/goose/src/session/session_manager.rs:617`), i.e. it *is* model
  output summarising the conversation.
- **And `chatId` is not opaque.** `chat_id = re.sub(r"[^a-zA-Z0-9-]", "-",
  f"{request.repo}-{suffix}")` (`code-agent-manager.py:941-942`) — it embeds the
  repository name. This is the specific trap: a careful implementer picking the
  safest-looking field still ships the contents of the private repo allowlist,
  one repo name per notification.

Hence the opaque handle. **Never put `chatId` or `sessionId` on the wire.**

### The bar is the lock screen, not the app

`personal-ai-setup/docs/security.md:24-26` accepts "a compromised phone or Mac"
as a risk, but everything sensitive today sits behind the app container and the
tailnet. A notification renders on a **locked** screen, and iOS's Show Previews
setting is per-device and per-user — the server cannot read it or enforce it. A
body that is safe inside the app is not safe on a lock screen, and no server-side
header makes it so. Write that bar into `docs/privacy.md` as the rule for the
channel, and add a line to `security.md`'s accepted-risk list acknowledging the
lock screen is now part of the surface.

`docs/privacy.md:106-125` already states the general rule for this channel —
counts and neutral titles only, one choke point, "recipes never assemble their
own ntfy requests" — and the manager already honours it in `notify_failure`
("Component + failure class only — never model output", `:1104-1107`). Formatting
a notification belongs in **exactly one function**, the way `notify.sh` is the one
place today. `docs/privacy.md` needs a row for the new channel.

One happy consequence: `docs/app-privacy-policy.md` — a Google OAuth verification
artifact, not a courtesy document — states the app "never shares data with third
parties beyond the processing above". A goose turn-end push naming an
inbox-triage session would carry an LLM-generated summary of Gmail content to
Apple or ntfy.sh, a party the policy does not name, and the policy would need a
new processor and a new effective date. **With the content-free payload, nothing
Google-derived leaves and the policy stands unchanged.**

### The ntfy channel specifically

Two rules, both learned from the existing deployment:

- **A separate topic**, stored separately from `NTFY_TOPIC`, so the failure
  channel and the interactive channel can be burned independently. The topic name
  is a shared secret in both directions — `notify.sh:11-12` says so — and
  `docs/automations.md:118-119` notes the current design rests on nothing being
  subscribed. Subscribing the phone turns the topic from a read-only leak into a
  **write channel onto your lock screen**: anyone who learns it can plant "code
  agent wants to push to main" there.
- **Do not call `notify.sh`**, which is shaped for rare failures and
  unconditionally attaches the `Email:` header (`:65-68`), burning the ~5/day
  forwarding cap. The notifier gets its own sender with no Email header.

And a rule that follows from the previous point and holds for APNs too: **a
notification is never itself answerable.** No Allow/Deny action buttons on the
notification. The tap only opens the app, and the app re-reads the real pending
ask over the tailnet before showing anything actionable.

On self-hosting ntfy later: a self-hosted server with `upstream-base-url` set
publishes only a **poll request** carrying the message id upstream — ntfy's own
configuration docs say "the self-hosted server literally sends the message `New
message` for every message". So the third party sees no content at all, which is
stronger than `docs/roadmap.md:112-124` currently claims. The cost is that the
iOS app must reach the brain to fetch the body, i.e. Tailscale up at
notification-tap time — a real argument for keeping stage 0 on public ntfy.sh
under the content-free rule.

## 8. Staging

### Stage 0 — the code plane, over ntfy. No Apple account, no `unsafe`.

Prerequisites, in order, both of which are bug fixes worth doing anyway:

1. Make `_lock` reentrant (or inline `touch` into `wake_chat`) —
   `code-agent-manager.py:1021-1028`, `:1051-1057`.
2. Settle whether a permission-blocked opencode session reports busy (§9,
   experiment 1) and, whatever the answer, teach `chat_busy` a third state:
   **a chat with a pending ask counts as busy** and is not reaped. Bound it — an
   ask outstanding longer than N minutes stops counting, or blocked chats stop
   counting toward `MAX_ACTIVE`, or two ignored asks lock the plane at
   `MAX_ACTIVE = 2`.

Then, inside `reaper_loop()` (60-second cadence, already talking direct to
127.0.0.1, already never touching the proxy): a previous-state map per chat,
firing the sender on a busy→idle edge and on a newly-seen ask id. Roughly thirty
lines, reusing the existing `pending_permissions()` fan-out.

On the phone: install the ntfy app, subscribe to a **new** topic. The topic value
is a secret — read it from Keychain Access.app, service `personal-ai`; it must not
be printed into a terminal or a conversation.

**What it gives up:** the goose plane entirely; up to 60 seconds of latency; and
the tap lands in ntfy's app rather than ours, because the deep link is dead
(§4). No Allow/Deny from the lock screen.

**Done looks like:** start a code-agent task that needs to push, lock the phone,
and the phone buzzes within a minute of the ask appearing; unlock, open the app,
and the ask is still there and answerable — which today it might not be.

**Adjacent and worth doing at the same time, for the best ratio in the whole
plan:** pre-approve the tools you would always allow.
`_goose/unstable/tools/permissions/set` exists in the ACP surface
(`crates/goose-acp-client/tests/fixtures/acp-meta.json`). Turning "blocked on
permission" into "turn ended" for routine tools is configuration, not code, and it
makes the cheap transports sufficient for more of the day. A mitigation, not a
fix.

### Stage 1 — the goose plane gets durability

`goose-notifier.service`: the Rust binary of §5, owning the prompt, parking asks,
mirroring updates over SSE. New base-URL field in `Settings`. Restart gate in
`renew-tls-cert.sh`. The phone loses its own `session/prompt` path.

**What it gives up:** still ntfy, still no in-app tap target.

**Done looks like:** start a goose turn from the phone, lock it, and twenty
minutes later the ask is *still parked* — the tool has not been auto-cancelled
and the run has not died. That is the correctness fix; the notification is the
part that tells you about it.

### Stage 2 — APNs

Only after §6's five-minute check and the purchase. Order: enrol; create the
explicit App ID with Push enabled (checking the bundle id is free); one
Sandbox-scoped team key; `[ios.entitlements] aps-environment = "development"`;
`crates/ios-push/` with the lint carve-out and the CI clippy extension; then the
sender, **with its error logging written first**. Budget a full token flush at
enrolment.

**Done looks like:** the lock screen banner is ours, the tap opens our app, and
`deliver()` has two implementations behind one signature so ntfy remains the
fallback for an APNs outage.

### Stage 3 — the tap lands on the ask

The `LaunchBuilder::mobile().with_cfg(...)` rewrite, `CFBundleURLTypes`, the
handle exchange, and the fetch-then-open path `open_session` needs. Live
Activities, if ever, after this.

## 9. What would falsify this

Ranked by how much of the plan each one moves.

**1. Does a permission-blocked opencode session report busy on
`/session/status`?** `chat_busy` greps the JSON blob for `"busy"` or `"retry"`
(`code-agent-manager.py:1078-1085`). If a blocked session does not match, the
reaper kills the container with the ask in its memory at ~15 minutes — the push
says "waiting on you", the tap wakes a *fresh* container, `catch_up_permissions`
gets an empty `/permission` (`src/code.rs:1051`), and the card clears. The turn is
not parked, it is dead. If it *does* match, `touch(cid)` pins the container
indefinitely and two unanswered asks lock the plane at `MAX_ACTIVE = 2`. Both
answers are broken, differently. Note that `pending_permissions`'s docstring
asserts the second branch as fact (`:338-341`), and
`crates/opencode-client/src/lib.rs:1089-1092` repeats the claim — two load-bearing
comments resting on an untested assumption.
**Experiment:** park an ask, wait past `IDLE_SECONDS`, then read
`GET /session/status` and `container_state`. Five minutes of waiting, one HTTP
call. Settles stage 0 outright.

**2. Does `class_addMethod` against tao's live `AppDelegate` actually deliver a
token?** The ObjC runtime restricts only `class_addIvar` to the pre-registration
window, but the delegate instance is already alive and its method caches warm by
the time our `spawn_forever` runs. **Experiment:** a spike branch that grafts the
selector and logs the token *length* (never the token) — device-only, cannot be
tested in the simulator, and requires stage 2's entitlement. Falsifies the entire
client half if it fails; the fallback would be forking tao or dioxus-desktop to
add a delegate hook, which is a materially different project.

**3. Is `UNUserNotificationCenter::currentNotificationCenter()` usable before
`UIApplicationMain` has run?** The design puts the delegate assignment in `main()`
because that is the only place early enough. I believe it is fine — it is not a
`UIApplication`-derived singleton — but I found no Apple statement either way.
**Experiment:** a local-notification-only spike, which needs no `unsafe` and no
paid account (every binding on that path is a safe `pub fn` in
`objc2-user-notifications-0.3.2`). This is also independently useful: it lets the
notification UI be built and tested before Apple is paid anything.

**4. Does `com.goosemobile.app` survive enrolment?** §6. **Experiment:** the
developer portal Identifiers page, five minutes, free. Falsifying it costs a
bundle-id change and a full reinstall.

**5. Is there a resumable connection identity on the HTTP (non-WebSocket) ACP
side?** `goose/crates/goose/src/acp/transport/mod.rs:161-170` exposes
`acp-connection-id` and `acp-session-id` response headers, which hints that the
`agent-client-protocol-http` crate (a git dependency at rev `c97a520`, not in any
local cargo cache, so it could not be read) has one. If it does, a
reattach-after-suspend path might exist that is cheaper than the whole
notifier-owns-the-prompt design. **Experiment:** one look at the crate's transport
before building stage 1. Worth doing first; it is the only thing here that could
make stage 1 substantially smaller.

**6. Do two per-connection `SessionManager`s over one SQLite file behave?**
Probably fine and probably not a blocker — `session_manager.rs:906-907` sets WAL
plus a 30-second busy timeout — but the notifier holds a connection while the
phone may hold another. **Experiment:** smoke test with two connections prompting
different sessions. (Related and already settled the reassuring way: a turn killed
mid-flight *is* resumable, because `fix_conversation`
(`goose/crates/goose/src/agents/reply_parts.rs:337-340`) repairs the dangling tool
request before every provider call. A half-written session does not poison the
next turn.)

**7. Can an ntfy iOS notification deep-link into a custom URL scheme?** ntfy's
`Click` header takes a URL and the app hands it to iOS, so custom schemes should
work — but ntfy's per-feature support matrix renders as icons and could not be
read directly, so this rests on release notes. Moot until §4's `Event::Opened`
problem is fixed either way. **Experiment:** send one message with a `Click`
header to a test topic and tap it.

**8. Is `Library/Application Support` really in the iCloud backup on iOS?** The
device-identity argument in §5 depends on it. This is reasoned from Apple's
documented backup exclusion rules (Caches and `NSURLIsExcludedFromBackupKey`
items are excluded; everything else is not), not tested. **Experiment:** a device
backup inspection, or simply adopt the token-as-identity rule anyway — it costs
nothing and removes the dependency on the answer.

---

Two things this document deliberately does not claim. It does not claim that
APNs is worth $99/yr — a Telegram gateway is already deployed on the brain
(`personal-ai-setup/scripts/vps/systemd/goose-telegram-gateway.service:1-4`,
described in its own header as "the phone's agentic surface") and ntfy is already
publishing into the void, so the honest framing is that **APNs buys a lock-screen
banner that is ours and a tap that lands in our app**, on top of a buzz that stage
0 delivers for free. And it does not claim the notification is the feature. On the
goose plane the notification is a *report* on a durability fix that does not exist
yet; ship the fix, then the report.
