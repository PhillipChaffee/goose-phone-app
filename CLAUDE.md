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

Styling: `assets/main.css` is the whole design system — semantic tokens,
light and dark, with `data-theme` on the root element overriding the system
preference. Its header says where the token values come from and why they are
not free to adjust to taste; read it before changing a colour.

`docs/style-gallery.html` renders every screen in a 390x844 frame against
that stylesheet: open it in a browser after a CSS change and every state is
visible at once, with no build and no device.

The toolchain is pinned in `rust-toolchain.toml` and rustup honours it
automatically, so a local `cargo clippy` sees exactly the lints CI sees.
Bumping the channel is a deliberate change: raise it, re-run the gate, and
fix what the newer lints found in the same commit.
