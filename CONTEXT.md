# Glossary

Terms of the goose phone client effort. A glossary and nothing else: decisions live on the tracker, implementation in the code.

**Keeper** — the host-side daemon that owns the prompt connection to the goose server on the user's host, parks permission asks durably, and computes notification events. The phone speaks the keeper's HTTP API, never ACP directly. Thin and replaceable by design: if goose detaches runs server-side, the keeper's reason to exist disappears.

**Prompt run** — one turn of agent work on the server. Owned by exactly one connection; on current goose it dies when that connection does.

**Parked ask** — a permission request held as durable server-side state by the keeper until the user answers. Contrast: goose's native pending ask is an in-flight request that self-cancels when the transport fails, silently denying the tool.

**Turn finished** — the ordinary notification event: a run ended and nothing is waiting on the user.

**Blocked on ask** — the urgent notification event: a run has stopped and stays stopped until a permission decision arrives.

**Single prompt path** — invariant: only one owner of a session's prompt path at a time. Two owners interleave one transcript with no diagnostic anywhere.

**Tailscale-blind** — the app takes a plain server URL and a secret; no network technology is a concept inside the app. Tailscale is how the phone typically reaches that URL, not an app feature.

**Content-free payload** — a notification carries only a count and a kind; the truth is fetched from the keeper after the app opens. The lock screen is the exposure bar the payload must clear.