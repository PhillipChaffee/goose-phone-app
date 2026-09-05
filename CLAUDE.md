# Goose Mobile — project guide

A Rust client (Dioxus 0.7) for a remote goose AI agent server, targeting iOS,
Android and a macOS desktop shell from one codebase and reaching the server
over Tailscale. The phone is the product; the desktop shell is the same views
worn a second way (see the two-shells paragraph at the end).

- `src/` — the Dioxus app (`state.rs` holds connection lifecycle + transcript folding)
- `crates/goose-acp-client/` — UI-independent ACP protocol library (tokio + tungstenite + rustls)
- `crates/opencode-client/` — the other backend: the code-agent manager and the per-chat `OpenCode` servers behind it (reqwest + rustls)
- `crates/mock-goose-server/` — protocol-faithful fake goose, for testing without an API key
- `crates/mock-opencode-server/` — the same for the other backend: the
  code-agent manager and the per-chat `OpenCode` servers behind it, so the
  whole Code plane runs with no container engine and no paid key
- `scripts/check-server.sh` — verifies a goose server is in the shape the app needs
- `docs/iphone-setup.md` — end-to-end iPhone deployment walkthrough

Common commands:

```bash
cargo clippy --workspace --all-targets -- -D warnings   # the CI lint gate
cargo test --workspace               # all tests
cargo fmt --all -- --check           # formatting gate
cargo run -p mock-goose-server       # fake goose on :3285 (secret "mock-secret")
cargo run -p mock-opencode-server    # fake code agents on :4399 ("mock-code-secret")
dx serve --desktop                   # the desktop shell (the phone's is iOS/Android only)
```

**Testing the whole app locally.** Start both fakes, then serve with the
`GOOSE_DEV_*` seeds so a rebuild does not mean retyping five fields. They are
read by `dev_seed!` in `src/state.rs` and expand to nothing in a release build,
so a development endpoint cannot ride along into one:

```bash
GOOSE_DEV_SERVER_URL=http://127.0.0.1:3285 \
GOOSE_DEV_SECRET_KEY=mock-secret \
GOOSE_DEV_WORKING_DIR=$PWD \
GOOSE_DEV_CODE_URL=http://127.0.0.1:4399 \
GOOSE_DEV_CODE_PASSWORD=mock-code-secret \
  dx serve --desktop
```

The fields arrive filled; press **Save & Connect** once per launch, because the
app deliberately starts disconnected. `MOCK_FIXTURES=empty` on either fake
gives the connected-and-empty state every empty-list message is written for,
and the code fake takes `slow`, `ask`, `fail` and `notool` as prompt keywords
the way the goose one takes `slow` and `notool`.

Lint policy: `[workspace.lints]` in the root `Cargo.toml` turns on clippy's
pedantic, nursery and cargo groups plus restriction picks (`unwrap_used`,
`expect_used`, `panic`, `print_stdout`, ...). Every blanket exception is
justified in that table; one-off exceptions go in the code as
`#[expect(lint, reason = "...")]` — `expect`, not `allow`, so an exception
that stops being needed fails the build instead of rotting.

Styling: [`docs/design.md`](docs/design.md) is the design guide — where the
look comes from and the rules that produce it (floating chrome, tiered
rounding, borders vs shadows, tap targets). Read it before changing
`assets/shared.css`, which is the whole design system: semantic tokens, light
and dark, with `data-theme` on the root element overriding the system
preference. It is called *shared* because **both shells link it** — `src/app.rs`
emits it into the phone and the desktop alike, and only about 4% of it (`.ptr`
and the drawer panel head, both named in its header) is the phone's alone. A
rule there lands on the desktop too; to change how it lands there, add an
override in `assets/desktop/` rather than editing it.

Every size in it is a `rem`, because the root font-size is the reader's — on
iOS `assets/platform/ios.css` sets it to `-apple-system-body` and the whole
scale follows Dynamic Type. `assets/platform/` is the one directory `src/css.rs`
picks from by target rather than concatenating whole: `PLATFORM` is
`ios.css` under `#[cfg(target_os = "ios")]` and `macos.css` under
`#[cfg(target_os = "macos")]`, at most one per binary and never both. The iOS
arm is a `cfg` rather than an unconditional line because macOS is WKWebView too
and resolves the same keyword to a flat 13px; the macOS arm exists because
`src/main.rs` hides the titlebar there and nowhere else, so only that build may
reserve room for the traffic lights. `docs/audit.js`
and `docs/measure-composer.js` both walk four text sizes; design.md rule 14
is the whole story. The audit walks a second axis as well — six phone sizes,
320x568 to 440x956, width *and* height, because the failures it found there
were content taller than the space it was given. Five are iPhones and 360x800
is Android's modal size, since this app ships to both.

The audit walks a **second grid** for the desktop shell: seven window sizes at
the one root a macOS build has, with three shell states — nav open, nav closed
and fullscreen — in place of the text axis. A state says which grid it is on through its own key — a desktop dump
carries a `desktop-` prefix from `src/shell::DUMP_PREFIX` — and that is also
what decides whether `assets/desktop/` and `assets/platform/macos.css` are
linked, so a desktop rule cannot reach a phone frame. Both shells write into
one `docs/gallery-states.json`, so a capture takes **both logs in one
invocation**: `scripts/capture-gallery.py /tmp/applog.txt /tmp/desktop.log` —
or `--only desktop- /tmp/desktop.log`, which declares a scope, stays
authoritative inside it and carries the phone's half over untouched. A store
that stops describing the app is caught by
`every_class_the_desktop_shell_renders_is_in_the_captured_store`, not by the
audit: the audit renders what it is given.

`docs/style-gallery.html` renders every phone state in a 402x874 frame and
every desktop state in the 1440x860 window `src/main.rs` opens, each against
the sheets that shell actually ships: open it in a browser after a CSS change
and all of them are visible at once, with no build and no device. It is
**generated** from the running app by `scripts/capture-gallery.py` — never
hand-edited — and
`node docs/audit.js both` plus `node docs/measure-composer.js` (no argument:
that sweeps its whole default list of six widths in ~2min, and naming one
narrows the run to that width) are the checks that read it. See the end of
`docs/design.md` for how to re-capture.

Two shells, chosen by `target_os` in `src/shell/`: `mobile.rs` is today's
phone (swipe trays, pull-to-refresh, drawer over the page), and
`src/shell/desktop/` is **three columns** — a sidebar holding the plane's own
session list, a content column, and an inspector. It splits at the top into a
**Chat** half and a **Code** half (`nav::Plane`), chosen by the segmented
control at the top of the sidebar; the two share no data and no vocabulary.
Platform decides affordances; width decides only how many columns, entirely
inside `assets/desktop/` — so nothing in Rust listens to a **DOM** resize,
which is the one that costs a synchronous XHR per frame. There is exactly one
Rust resize listener in the tree and it is not that: `use_fullscreen` in
`src/shell/desktop/mod.rs` takes a `tao` `Resized` as a trigger and reads the
window's own fullscreen flag back off it, because tao publishes no fullscreen
event. It decides nothing about columns.

The desktop's window chrome is inside the app: one band across the top holding
the traffic lights' reservation, the nav toggle, the plane badge, a crumb
naming whatever is open with the half's counts beside it, the plane's own
connection, and the inspector toggle. `crumb_parts` is TOTAL — every screen
has a name — so `assets/desktop/`'s `:has(.chrome-title)` takes that
heading back out of the pane unconditionally and there is one title per
window. The name travels as data on `nav::Destination` (`Detail`'s `Crumb`),
because Dioxus has no portal.

**The desktop's sheet is a directory, and the sort is the cascade.**
`assets/desktop/` holds fifteen region files — `00-tokens.css` through
`98-home-sched.css` — that `src/css.rs`'s `SHELL` `concat!`s in filename order,
the same way `STYLES` assembles the phone's. One 4278-line sheet was one file
every branch of a wide redesign appended to at once, and parallel appends had
already left it carrying the same declaration twice; a region that needs rules
of its own now brings its own file and nothing else in the list moves. The
prefixes are zero-padded because **three** places produce that order and all
three must agree: the `concat!` in `src/css.rs`, `readdirSync().sort()` in
`docs/audit.js`, and `sorted()` in `scripts/capture-gallery.py`. A sheet that
sorts into a different slot in any one of them is a different cascade from the
one that ships. `assets/platform/macos.css` is appended after that sort, never
swept into it — `src/app.rs` emits STYLES, then SHELL, then PLATFORM, and the
two sides set `--chrome-h`/`--traffic-w` on the same selector, so the later one
has to be the platform's.

**The desktop has a type scale of its own**, declared on `.app > .shell` in
`assets/desktop/00-tokens.css`: the mockups' ladder, body 13px on 1.45 against
the phone's 16 on 24, with a 10px floor. It is all `rem`, so the root is still
the reader's. **Three** traps are written up there and all three are
load-bearing: a custom property declared on `:root` substitutes AT `:root`, so
`body { font-size: var(--text-md) }`, `body { color: var(--text-primary) }` and
`--text-code` never see a shell override, and the shell restates all three. The
third is the one that nearly shipped as a no-op — remapping the ink tokens
changes almost nothing in this window unless `.app > .shell` also says
`color: var(--text-primary)`, and no gate in the repo can tell that apart from
a change that worked.

**Nothing may display a number no server sends.** Dollar figures, latency
percentiles, container counts, queue depth, per-turn durations and the user's
own name are all absent by decision, each recorded where it would have gone.
`src/shell/desktop/inspector.rs` has the fullest version of that list.

The desktop section at the end of `docs/design.md` is the whole story.

The toolchain is pinned in `rust-toolchain.toml` and rustup honours it
automatically, so a local `cargo clippy` sees exactly the lints CI sees.
Bumping the channel is a deliberate change: raise it, re-run the gate, and
fix what the newer lints found in the same commit.
