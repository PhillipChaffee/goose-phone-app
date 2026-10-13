# goose-phone-app

Terms of the goose phone client's domain — a phone client for a user-hosted goose agent. A glossary and nothing else: decisions live on the tracker, implementation in the code.

## Language

### Connection and trust

**Connect surface**:
The phone screen holding the connection fields — server URL, secret key, working directory, optional fingerprint — plus connect/test controls. An unconfigured device opens here.
_Avoid_: Settings screen, login screen

**Pairing**:
First-time entry of the server URL and secret on the phone. Manual entry with clipboard paste; no QR or deep link (those would need a server affordance).
_Avoid_: Login, sign-in, provisioning

**Fingerprint pin**:
The SHA-256 of the server's TLS certificate, entered on the connect surface. It is the trust decision for the `goose serve --tls` path and replaces certificate-chain and hostname validation.
_Avoid_: Cert pinning, TOFU

**Tailscale-blind**:
The app takes a plain server URL and a secret; no network technology is a concept inside the app. Tailscale is how the phone typically reaches that URL, not an app feature.

### The keeper and its notifications

**Keeper**:
The host-side daemon that owns the prompt connection to the goose server on the user's host, parks permission asks durably, and computes notification events. The phone speaks the keeper's HTTP API, never ACP directly.

**Prompt run**:
One turn of agent work on the server, owned by exactly one connection; on current goose it dies when that connection does.

**Single prompt path**:
Invariant: only one owner of a session's prompt path at a time. Two owners interleave one transcript with no diagnostic anywhere.

**Parked ask**:
A permission request the keeper holds as durable server-side state until the user answers. Contrast: goose's native pending ask is an in-flight request that self-cancels when the transport fails, silently denying the tool.

**Turn finished**:
The ordinary notification event: a run ended and nothing is waiting on the user.

**Blocked on ask**:
The urgent notification event: a run has stopped and stays stopped until a permission decision arrives.

**Content-free payload**:
A notification carries only a count and a kind; the truth is fetched from the keeper after the app opens. The lock screen is the exposure bar the payload must clear.
