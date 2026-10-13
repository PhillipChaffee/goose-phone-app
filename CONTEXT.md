# goose-phone-app

A Dioxus/Rust phone client for a user-hosted goose agent, speaking ACP over the user's Tailscale tailnet. This glossary holds the project's language.

## Language

**Connect surface**:
The phone screen holding the connection fields — server URL, secret key, working directory, optional fingerprint — plus connect/test controls. An unconfigured device opens here.
_Avoid_: Settings screen, login screen

**Pairing**:
First-time entry of the server URL and secret on the phone. Manual entry with clipboard paste; no QR or deep link (those would need a server affordance).
_Avoid_: Login, sign-in, provisioning

**Fingerprint pin**:
The SHA-256 of the server's TLS certificate, entered on the connect surface. It is the trust decision for the `goose serve --tls` path and replaces certificate-chain and hostname validation.
_Avoid_: Cert pinning, TOFU