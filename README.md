# Goose Mobile

A mobile client for [goose](https://github.com/aaif-goose/goose) — the open-source AI agent from the Agentic AI Foundation (originally created by Block) — written entirely in **Rust** with [Dioxus](https://dioxuslabs.com). One codebase builds for **iOS**, **Android**, and desktop, and connects to a remote goose server running on your own machine or cloud box, reached privately over **Tailscale**.

```
┌─────────────┐     Tailscale (WireGuard)      ┌──────────────────────┐
│  Phone       │  wss://goose-box.tailnet…/acp │  Cloud / home server │
│  Goose Mobile│ ─────────────────────────────▶│  goose serve (ACP)   │
│  + Tailscale │      JSON-RPC over WebSocket  │  + tailscaled        │
└─────────────┘                                └──────────────────────┘
```

## How it works

Modern goose (≥ 1.42) exposes a single API: the [Agent Client Protocol](https://agentclientprotocol.com) (JSON-RPC 2.0) served by `goose serve` at `/acp`. This app speaks it over a WebSocket — the same transport the official goose Desktop app uses:

- `initialize` → `session/new` / `session/load` / `session/list` → `session/prompt`
- streamed `session/update` notifications render live assistant text, thinking, and tool calls
- `session/request_permission` requests from the agent pop a native approval sheet (allow once / always / reject)
- `session/cancel` stops a running turn; token usage is shown from goose's usage notifications
- disconnects (phone sleeping, network blips) auto-reconnect — quick backoff ramp, then a steady 30 s retry until the server is back — and the open session's history is replayed

The protocol layer lives in [`crates/goose-acp-client`](crates/goose-acp-client) — a UI-independent tokio + tungstenite + rustls library you can reuse in other Rust clients.

## 1. Server setup

On the machine that will run goose (any always-on Linux/macOS box):

```bash
# Install goose and configure your AI provider once
curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | bash
goose configure

# Run the server (default port 3284). The secret key is the app's password.
GOOSE_SERVER__SECRET_KEY='pick-a-long-random-secret' \
  goose serve --platform desktop --enable-scheduler --host 127.0.0.1 --port 3284
```

Verify it locally: `curl http://127.0.0.1:3284/status` → `ok`.

goose's own guide for this setup: [Remote goose server](https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/remote-goose-server.md).

## 2. Tailscale setup (recommended path)

1. Install [Tailscale](https://tailscale.com) on the server (`tailscaled` + `tailscale up`) and sign in to your tailnet.
2. In the [admin console → DNS](https://login.tailscale.com/admin/dns): enable **MagicDNS** and **HTTPS Certificates**.
3. Front goose with tailnet-only HTTPS (a real Let's Encrypt certificate, no self-signed anything):

   ```bash
   sudo tailscale serve --bg 3284
   ```

   Your server is now `https://<machine-name>.<tailnet-name>.ts.net` — reachable **only** from your tailnet, with goose still bound to localhost.
4. On your phone, install the Tailscale app (App Store / Play Store), sign in to the same tailnet, and toggle the VPN on.

In Goose Mobile's settings, enter:

- **Server URL**: `https://<machine-name>.<tailnet-name>.ts.net`
- **Secret key**: the `GOOSE_SERVER__SECRET_KEY` value
- **Working directory**: an absolute path on the server where new sessions start, e.g. `/home/you/projects`

Tap **Test connection**, then **Save & Connect**.

### Alternative: goose's own TLS (self-signed + pinning)

If you'd rather not use `tailscale serve`, run goose with TLS directly:

```bash
GOOSE_SERVER__SECRET_KEY='…' goose serve --host 0.0.0.0 --port 3284 --tls
```

goose prints `GOOSED_CERT_FINGERPRINT=AA:BB:…` at startup. Use `https://<tailnet-ip-or-name>:3284` as the server URL and paste that fingerprint into the app's **TLS certificate fingerprint** field — the app pins that exact certificate (same scheme as goose Desktop).

### Alternative: plain HTTP over the tailnet

`http://<machine-name>.<tailnet-name>.ts.net:3284` (with `--host 0.0.0.0`, no `--tls`) also works — Tailscale already encrypts the path end-to-end, and the app's networking is pure Rust, so iOS ATS / Android cleartext policies don't intercept it. Keep the secret key set either way; the tailnet ACL plus the secret are your two layers of defense.

## 3. Building the app

Install the Dioxus CLI once:

```bash
curl -sSL https://dioxus.dev/install.sh | bash   # or: cargo install dioxus-cli
dx doctor                                        # verifies platform toolchains
```

### Desktop (development)

```bash
dx serve
```

### iOS (requires a Mac with Xcode)

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
open -a Simulator && dx serve --ios          # simulator
dx serve --ios --device                      # physical device (needs a provisioning profile)
dx bundle --ios                              # release .ipa
```

### Android (requires Android Studio + NDK)

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi \
                  i686-linux-android x86_64-linux-android
# In Android Studio's SDK Manager install: SDK, command-line tools, NDK (side by side), CMake.
# Export JAVA_HOME / ANDROID_HOME / NDK_HOME per the Dioxus mobile guide.
dx serve --android                           # emulator
dx serve --android --device                  # physical device via adb
dx bundle --android                          # release .aab (use --package-types apk for an APK)
```

Bundle identifiers and Android signing are configured in [`Dioxus.toml`](Dioxus.toml). See the [Dioxus mobile guide](https://dioxuslabs.com/learn/0.7/guides/platforms/mobile) for toolchain details.

## Project layout

```
├── src/                     # Dioxus app (UI, state, event pump)
│   ├── main.rs              #   entry point
│   ├── app.rs               #   root component + screen switching
│   ├── state.rs             #   settings, connection lifecycle, chat transcript fold
│   ├── markdown.rs          #   sanitized markdown → HTML for chat bubbles
│   └── views/               #   Settings / Sessions / Chat screens + permission modal
├── assets/main.css          # mobile-first dark theme
├── crates/goose-acp-client/ # reusable ACP protocol library (tokio + tungstenite + rustls)
└── Dioxus.toml              # bundle config (identifiers, Android SDK levels)
```

## Development notes

- `cargo test -p goose-acp-client` runs protocol unit tests (wire-shape fidelity against goose 1.47 frames).
- The client advertises no `fs`/`terminal` capabilities, so the agent always uses its own server-side tools — the phone only ever approves or rejects them.
- Settings persist in the app-private data directory (`dioxus-sdk-storage`); the secret never leaves the device except as the `X-Secret-Key` header to your server.
- The WebSocket sends a ping every 30 s to keep NAT/VPN paths alive; `session/prompt` deliberately has no timeout (agent turns can run for minutes).
- Session history replays through the same streaming path as live updates (`session/load`), so one rendering pipeline covers both.

## Security

- Prefer the `tailscale serve` setup: tailnet-only exposure, real certificates, goose bound to loopback.
- Use a long random `GOOSE_SERVER__SECRET_KEY`; the app sends it only to the configured server.
- Restrict which tailnet devices may reach the goose port with [Tailscale ACLs](https://tailscale.com/kb/1018/acls).
- Tool permission prompts are your last line of defense — leave goose in approval mode for remote use.

## Roadmap ideas

- Embedded Tailscale (`libtailscale`) for one-tap onboarding without the separate VPN app
- Image attachments (the protocol layer already models `ContentBlock::Image`)
- Mid-turn steering via `_goose/unstable/session/steer`
- Session rename/archive, recipes, and model switching from the phone
