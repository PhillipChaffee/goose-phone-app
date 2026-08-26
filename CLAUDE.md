# Goose Mobile — project guide

A Rust mobile client (Dioxus 0.7) for a remote goose AI agent server, targeting
iOS and Android from one codebase and reaching the server over Tailscale.

- `src/` — the Dioxus app (`state.rs` holds connection lifecycle + transcript folding)
- `crates/goose-acp-client/` — UI-independent ACP protocol library (tokio + tungstenite + rustls)
- `crates/mock-goose-server/` — protocol-faithful fake server for testing without an API key
- `scripts/check-server.sh` — verifies a goose server is in the shape the app needs
- `docs/iphone-setup.md` — end-to-end iPhone deployment walkthrough

Common commands:

```bash
cargo clippy --workspace --all-targets -- -D warnings   # the CI lint gate
cargo test --workspace               # all tests
cargo fmt --all -- --check           # formatting gate
cargo run -p mock-goose-server       # fake server on :3285 (secret "mock-secret")
dx serve --desktop                   # run the app for development
```

Lint policy: `[workspace.lints]` in the root `Cargo.toml` turns on clippy's
pedantic, nursery and cargo groups plus restriction picks (`unwrap_used`,
`expect_used`, `panic`, `print_stdout`, ...). Every blanket exception is
justified in that table; one-off exceptions go in the code as
`#[expect(lint, reason = "...")]` — `expect`, not `allow`, so an exception
that stops being needed fails the build instead of rotting.

Styling: [`docs/design.md`](docs/design.md) is the design guide — where the
look comes from and the rules that produce it (floating chrome, tiered
rounding, borders vs shadows, tap targets). Read it before changing
`assets/main.css`, which is the whole design system: semantic tokens, light
and dark, with `data-theme` on the root element overriding the system
preference.

Every size in it is a `rem`, because the root font-size is the reader's — on
iOS `assets/platform/ios.css` sets it to `-apple-system-body` and the whole
scale follows Dynamic Type. That sheet is the one platform-conditional
stylesheet (`#[cfg(target_os = "ios")]` in `src/css.rs`), because macOS is
WKWebView too and resolves the same keyword to a flat 13px. `docs/audit.js`
and `docs/measure-composer.js` both walk four text sizes; design.md rule 14
is the whole story.

`docs/style-gallery.html` renders every state in a 402x874 frame against that
stylesheet: open it in a browser after a CSS change and all of them are
visible at once, with no build and no device. It is **generated** from the
running app by `scripts/capture-gallery.py` — never hand-edited — and
`node docs/audit.js both` plus `node docs/measure-composer.js 360` are the
checks that read it. See the end of `docs/design.md` for how to re-capture.

The toolchain is pinned in `rust-toolchain.toml` and rustup honours it
automatically, so a local `cargo clippy` sees exactly the lints CI sees.
Bumping the channel is a deliberate change: raise it, re-run the gate, and
fix what the newer lints found in the same commit.
