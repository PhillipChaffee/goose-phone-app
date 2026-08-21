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
cargo check --workspace              # must be warning-free
cargo test -p goose-acp-client       # protocol wire-shape tests
cargo run -p mock-goose-server       # fake server on :3285 (secret "mock-secret")
dx serve --desktop                   # run the app for development
```
