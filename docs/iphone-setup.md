# Getting Goose Mobile onto your iPhone

For the case where goose is already running on a cloud VPS that is already on
your tailnet. Every command and file path below was checked against the
Dioxus CLI source at tag v0.7.10 and Apple's current documentation.

**Time:** about 45 minutes the first time, most of it Xcode downloading.
**You need:** a Mac with Xcode (see [Appendix A](#appendix-a--no-mac) if you
don't have one), your iPhone and a cable, and an Apple ID (free is fine — see
[Appendix B](#appendix-b--the-7-day-expiry) for the catch).

---

## Step 1 — Confirm the server is app-ready

On the VPS:

```bash
git clone -b claude/goose-ai-mobile-app-dbwibi https://github.com/PhillipChaffee/goose-phone-app.git
cd goose-phone-app
./scripts/check-server.sh
```

It reports on the goose process and version, the secret key, the listening
address, `tailscale serve`, and the two HTTP signals that actually matter
(`/status` → 200, `/acp` with the secret → **406**). When everything passes it
prints the three values to type into the app. Fix anything marked `fail`
before continuing.

If `tailscale serve` is not yet fronting goose, this is the recommended shape —
goose stays on localhost and gets a real Let's Encrypt certificate, tailnet-only:

```bash
sudo tailscale serve --bg 3284      # then: tailscale serve status
```

That requires **MagicDNS** and **HTTPS Certificates** to be enabled for the
tailnet in the [admin console → DNS](https://login.tailscale.com/admin/dns).

> **ACL note:** with `tailscale serve`, phones connect to the node on port
> **443**, not 3284. If you restrict access with ACLs, allow `tcp:443` to the
> goose node — a rule that only permits 3284 will silently drop everything.
> Also confirm the VPS isn't in shields-up mode (`tailscale set --shields-up=false`),
> which refuses all inbound traffic regardless of ACLs.

---

## Step 2 — Prepare the Mac

Install Xcode from the App Store (the full app — the Command Line Tools alone
lack `xcrun devicectl` and the iOS device SDK), launch it once, then:

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
xcode-select -p                      # must print the path above
sudo xcodebuild -license accept
xcodebuild -runFirstLaunch
xcodebuild -downloadPlatform iOS     # the iOS device SDK

rustup target add aarch64-apple-ios          # add -sim too if you want the simulator
cargo install dioxus-cli@0.7.10 --locked     # or: cargo binstall dioxus-cli@0.7.10
dx --version
```

Clone the repo on the Mac as well:

```bash
git clone -b claude/goose-ai-mobile-app-dbwibi https://github.com/PhillipChaffee/goose-phone-app.git
cd goose-phone-app
```

**Optional but recommended dry run.** Before dealing with signing, prove the
app works against your server from the Mac:

```bash
dx serve --desktop
```

Enter your `https://<machine>.<tailnet>.ts.net` URL, secret, and working
directory, and tap Test connection. (The Mac must be on the tailnet.) If this
works, everything after here is purely an iOS packaging problem.

---

## Step 3 — Pair the iPhone and enable Developer Mode

1. Connect the iPhone by USB, unlock it, tap **Trust This Computer**, enter your passcode.
2. On the iPhone: **Settings → Privacy & Security → Developer Mode → on**, then
   tap **Restart** in the alert. After it reboots, unlock and tap **Enable**.
3. Back on the Mac, confirm the phone is visible to the exact tool `dx` uses:

```bash
xcrun devicectl list devices
```

> Developer Mode only appears in Settings after the phone has been paired with
> a Mac. If you don't see it, complete step 1 first.

---

## Step 4 — Create the signing assets (the step everything else depends on)

`dx` never creates signing material. It only *looks for* two things that must
already exist:

- a keychain identity whose name begins `Apple Development:`
  (`security find-identity -v -p codesigning`)
- a provisioning profile for the app's bundle id **`com.goosemobile.app`**,
  listing your iPhone, in
  `~/Library/Developer/Xcode/UserData/Provisioning Profiles`

Only Xcode's automatic signing can mint those on a free Apple ID, and it only
runs against an Xcode target. So create one small Xcode project once:

1. Xcode → **Settings… → Accounts → + → Apple ID** → sign in.
2. **File → New → Project… → iOS → App**. Name it anything (`GooseSigning` is fine).
   Set **Bundle Identifier** to exactly `com.goosemobile.app`.
3. Open **Signing & Capabilities**, tick **Automatically manage signing**, choose
   your Team (your name + "Personal Team" on a free account).
4. Select your iPhone in the run-destination dropdown at the top, then
   **Product → Build** (use **Product → Run** if Build alone doesn't produce a profile).

That registers your phone, creates the App ID, and writes the profile to disk.
Verify both prerequisites:

```bash
security find-identity -v -p codesigning | grep "Apple Development"
ls ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles
```

**Keep this Xcode project.** On a free Apple ID the profile expires after 7
days, and re-running Build here is how you renew it.

If you'd rather use a different bundle id (e.g. `com.yourname.goosemobile`),
change it in **both** places: the Xcode project and `identifier` under
`[bundle]` in `Dioxus.toml`. They must match exactly.

---

## Step 5 — Install the app

From the repo root on the Mac, with the iPhone connected and unlocked:

```bash
dx serve --ios --device
```

`dx` builds for `aarch64-apple-ios`, enables the crate's `mobile` feature,
signs the bundle with your Apple Development identity and profile, then
installs and launches via `xcrun devicectl`. Useful variants:

```bash
dx serve --ios --device "Phillip's iPhone"   # pick a specific device
dx serve --ios --device --release            # optimized build
dx serve --ios --device --verbose            # shows why signing failed
```

On first launch, if iOS says **Untrusted Developer**, go to
**Settings → General → VPN & Device Management → (your developer app) → Trust**.

> **Device mode is deploy-only.** Hot reload and app logs do not work over
> `devicectl` in dx 0.7.10 — the dev-server address is never passed to the
> device. Iterate with `dx serve --desktop` or the simulator, and read device
> logs in **Console.app** with the iPhone selected in the sidebar.

---

## Step 6 — Tailscale on the iPhone

1. Install **Tailscale** from the App Store and sign in with the same identity
   your tailnet uses.
2. Turn it on. iOS shows **"Tailscale" Would Like to Add VPN Configurations** —
   tap **Allow** and confirm with Face ID / passcode. This installs the tunnel
   plus MagicDNS system-wide.
3. Verify before blaming the app: open
   `https://<machine>.<tailnet>.ts.net/status` in Safari on the phone. You
   should see `ok`.

> iOS allows only one VPN active at a time — another VPN app will displace
> Tailscale.

---

## Step 7 — Connect

Open Goose Mobile and fill in Settings:

| Field | Value |
|---|---|
| Server URL | `https://<machine>.<tailnet>.ts.net` (whatever `check-server.sh` printed) |
| Secret key | your `GOOSE_SERVER__SECRET_KEY` |
| Working directory | an absolute path that exists on the VPS, e.g. `/home/you/projects` |
| TLS fingerprint | leave empty (you have a real certificate) |

Tap **Test connection** → *"Server reachable, secret accepted ✓"*, then
**Save & Connect**. You land on Sessions; tap **New chat**.

---

## Troubleshooting

**"Failed to find Apple Development in `security find-identity`"** — `dx`
matches the literal string `Apple Development:`. Old-style `iPhone Developer:`
certificates and `Apple Distribution:` certificates are not accepted. Let Xcode
create a modern development certificate (Step 4).

**"No provisioning profile found matching bundle identifier"** — the profile
either doesn't exist or is in the wrong folder. `dx` checks
`~/Library/Developer/Xcode/UserData/Provisioning Profiles` and only falls back
to `~/Library/MobileDevice/Provisioning Profiles` if the first folder does not
exist at all — if it exists but holds no matching profile, the fallback never
happens. Copy the profile into the Xcode 16 folder. Run with `--verbose` to see
which profiles were skipped and why.

**`codesign` says the identity is ambiguous** — you have more than one Apple
Development certificate. `dx` takes the first match. Delete stale ones in
Keychain Access, or pin it:
`dx serve --ios --device --apple-team-id "Apple Development: you@example.com (XXXXXXXXXX)"`.

**The app launched last week and won't open now** — the free 7-day profile
expired. Rebuild in the Xcode project from Step 4, then `dx serve --ios --device`.

**"Server reachable, secret accepted ✓" on Wi-Fi but not on cellular** — a
known Tailscale-on-iOS issue after a network switch: the app looks connected
but nothing routes. Toggle Tailscale off and on.

**Safari can't resolve the `.ts.net` name** — MagicDNS isn't reaching the
phone. Check "Use Tailscale DNS Settings" in the Tailscale app; if you have a
DNS profile installed (NextDNS, AdGuard, etc.) under
**Settings → General → VPN & Device Management → DNS**, it can override
Tailscale. As an interim, use `http://100.x.y.z:3284` (the raw tailnet address)
in the app — the app's networking is pure Rust, so iOS ATS does not block it.

**A TLS error against a `100.x` address** — the `tailscale serve` certificate
covers the `.ts.net` hostname only. Use the hostname. (If you instead run
`goose serve --tls` with its self-signed certificate, paste the
`GOOSED_CERT_FINGERPRINT` value into the app's fingerprint field.)

**The app says "Connection lost" after the phone was in your pocket** — this is
expected and handled. While an iOS app is suspended no code runs and the system
may reclaim its sockets, so the WebSocket dies. The app pings every 30s, treats
two unanswered pings as a dead connection, and reconnects (2/4/8/15s, then
every 30s), replaying the open session's history.

---

## Appendix A — No Mac

macOS is required to *compile* (`dx` shells out to `codesign`, `security`, and
`xcrun`, and links against the iOS SDK), and an Apple account is required to
*install*. But you don't have to own the Mac:

1. Build an unsigned `.app` on a free GitHub Actions `macos-latest` runner:
   ```bash
   dx bundle --platform ios --package-types ios
   ```
   Note `--package-types ios`, not the default. The default (`ipa`) runs
   `codesign --verify` and fails with *"iOS .app bundle must be codesigned
   before creating an .ipa"* when nothing signed it.
2. Zip the result as an IPA: put `YourApp.app` inside a `Payload/` directory and
   zip that to `GooseMobile.ipa`.
3. Download the artifact and sideload from Windows or Linux with **Sideloadly**
   or **AltStore / AltServer-Linux**, which re-sign it with your free Apple ID.

Same 7-day expiry applies. Renting a cloud Mac (MacStadium, Scaleway, AWS EC2
Mac) is the alternative if you want the normal `dx serve --ios --device` loop.

## Appendix B — The 7-day expiry

A free Apple ID ("Personal Team") gets provisioning profiles valid for **7
days**, a limit of **3 sideloaded apps**, and 10 App IDs at a time. When it
expires the app simply stops launching — rebuild in the Xcode project from
Step 4 and re-run `dx serve --ios --device`.

The Apple Developer Program ($99/yr) raises this to roughly one-year profiles,
100 devices, wildcard App IDs, and TestFlight. For an app you want on your
phone permanently, that is the real fix.
