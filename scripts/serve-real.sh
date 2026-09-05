#!/usr/bin/env bash
# Serve the desktop shell against a REAL brain instead of the mocks.
#
#   ./scripts/serve-real.sh                 # BRAIN_HOST from the environment
#   ./scripts/serve-real.sh my-brain.ts.net
#
# CLAUDE.md's development recipe points at the two mock servers. This is the
# same recipe pointed at a real one, and it exists because three of the five
# differences are easy to get wrong and none of them fails loudly:
#
#   * the ports are NOT the mocks'. Real `goose serve` is 3284; the goose mock
#     is 3285. The code-agent gateway is 4300; its mock is 4399.
#   * a real `goose serve` holding its own TLS is SELF-SIGNED. Without a pinned
#     fingerprint the client falls back to WebPKI chain validation and refuses
#     the certificate. `Settings.fingerprint` has a seed for exactly this.
#   * `option_env!` is read by the compiler, so the seeds are baked from the
#     environment of the process that BUILDS. A `dx serve` already running keeps
#     handing out whatever it was launched with, and a second one started later
#     shares the same target directory — so the last writer wins and the window
#     you are looking at may not be the one you just configured.
#
# SECRETS ARE MOVED BY REFERENCE AND NEVER PRINTED. Both are read straight out
# of the login keychain into the child's environment by shell expansion. This
# script has no branch that echoes one, and it does not write one to a file.
# What it reports is presence, never value.
#
# Two consequences of the seeds being environment variables, stated because
# they are easy not to think about and this is a real credential rather than a
# mock's: the values are visible in `ps -E` output for the `dx serve` process
# while it runs, and `option_env!` bakes them into the debug binary under
# `target/`. Neither can ride into a release build — `dev_seed!` expands to an
# empty string there — but both are on this disk until you rebuild. If that is
# not a trade you want, leave the seeds off and type the fields into Settings
# instead; they will not survive a restart, because `use_persistent` is
# in-memory `SessionStorage` on every non-wasm target.

set -uo pipefail

BRAIN="${1:-${BRAIN_HOST:-}}"
KEYCHAIN_SERVICE="${KEYCHAIN_SERVICE:-personal-ai}"
GOOSE_PORT="${GOOSE_PORT:-3284}"
CODE_PORT="${CODE_PORT:-4300}"
# A directory that EXISTS ON THE SERVER and that new chats start in. goose
# rejects one that does not with "invalid directory path", and it says so at
# session creation rather than at connect — so the app connects, lists history
# and looks entirely healthy right up until the first message fails.
#
# It is not `GOOSE_PATH_ROOT`. That is where goose keeps sessions and schedules;
# the two are unrelated, and reading a path root out of a deployment doc and
# passing it here is how this default was wrong to begin with. The honest source
# is the server's own history — on the tailnet this was written against,
# `SELECT working_dir, COUNT(*) FROM sessions GROUP BY 1` answers /home/agent
# (70) and /home/agent/personal-ai-setup (61), so the home directory it is.
REMOTE_WORKDIR="${REMOTE_WORKDIR:-/home/agent}"

# THE TWO HALVES DO NOT HAVE TO LIVE ON THE SAME BOX, and today they usually do
# not. `goose serve` is one systemd unit on the brain; the code-agent manager is
# a container host that needs podman and two paid API keys, so a tailnet can
# easily carry a real goose and no real code plane at all. The app already keeps
# a separate URL and secret per plane, so point them wherever each one is:
#
#   CODE_URL=http://127.0.0.1:4399 CODE_PASSWORD_PLAIN=mock-code-secret \
#     BRAIN_HOST=brain.tailnet.ts.net ./scripts/serve-real.sh
#
# `CODE_PASSWORD_PLAIN` exists ONLY for a mock whose password is a published
# constant. Never pass a real credential through it — it would be visible in
# your shell history and in `ps`. A real code plane's password comes from the
# keychain like goose's does, which is what happens when you leave this unset.
GOOSE_URL="${GOOSE_URL:-}"
CODE_URL="${CODE_URL:-}"

die() { printf 'error: %s\n' "$1" >&2; exit 1; }
note() { printf '  %s\n' "$1"; }

[ -n "$BRAIN" ] || die "no brain host. Pass one, or set BRAIN_HOST=<name>.<tailnet>.ts.net"
command -v dx >/dev/null 2>&1 || die "dx is not on PATH (cargo install dioxus-cli)"

# ---- 1. the secrets, by reference ------------------------------------------
# `-w` prints the password on stdout, which is why it is consumed by an
# assignment and never by a command that could log it. If the keychain is
# locked, macOS prompts; that prompt is the intended interaction.
keyfetch() {
  security find-generic-password -s "$KEYCHAIN_SERVICE" -a "$1" -w 2>/dev/null
}

GOOSE_URL="${GOOSE_URL:-https://$BRAIN:$GOOSE_PORT}"
CODE_URL="${CODE_URL:-https://$BRAIN:$CODE_PORT}"

GOOSE_SECRET="$(keyfetch GOOSE_SERVER__SECRET_KEY)"
[ -n "$GOOSE_SECRET" ] || die "GOOSE_SERVER__SECRET_KEY not in keychain service '$KEYCHAIN_SERVICE'"

if [ -n "${CODE_PASSWORD_PLAIN:-}" ]; then
  CODE_SECRET="$CODE_PASSWORD_PLAIN"
  note "code password: taken from CODE_PASSWORD_PLAIN (use this for a mock only)"
else
  CODE_SECRET="$(keyfetch OPENCODE_SERVER_PASSWORD)"
  [ -n "$CODE_SECRET" ] || die "OPENCODE_SERVER_PASSWORD not in keychain service '$KEYCHAIN_SERVICE'"
fi
note "secrets: present (values not read)"

# ---- 2. the certificate pin, only if one is needed --------------------------
# ASK THE CERTIFICATE, do not assume. A pin is needed when the certificate does
# not chain to a public root, and that is a question with a one-command answer:
# curl WITHOUT -k either completes the handshake or it does not.
#
# Guessing it wrong is expensive in both directions. Assume self-signed and you
# go hunting for a `GOOSED_CERT_FINGERPRINT` line that a CA-issued deployment
# never printed. Assume CA-issued and the client falls back to chain validation
# and refuses to connect, with nothing on screen naming the certificate.
#
# Tailscale hands out real Let's Encrypt certificates for `*.ts.net`, so a goose
# configured with them validates on its own port with no `tailscale serve` in
# front of it — which is the case this branch exists to detect.
FP="${GOOSE_DEV_FINGERPRINT:-}"
if [ -n "$FP" ]; then
  note "fingerprint: supplied (${#FP} chars)"
elif curl -sS -o /dev/null --max-time 10 "$GOOSE_URL/status" 2>/dev/null; then
  note "certificate: chains to a public root — no pin needed"
elif curl -sS -o /dev/null --max-time 10 -k "$GOOSE_URL/status" 2>/dev/null; then
  note "certificate: self-signed — fetching the pin from $BRAIN"
  FP="$(tailscale ssh "agent@$BRAIN" \
        "journalctl -u goose-serve 2>/dev/null | grep -o 'GOOSED_CERT_FINGERPRINT=[A-Fa-f0-9:]*' | tail -1" \
        2>/dev/null | cut -d= -f2)"
  [ -n "$FP" ] && note "fingerprint: found (${#FP} chars)" || {
    printf 'warn: self-signed certificate and no GOOSED_CERT_FINGERPRINT in the log.\n' >&2
    printf '      The client will refuse this certificate. Re-run with\n' >&2
    printf '      GOOSE_DEV_FINGERPRINT=... once you have the line.\n' >&2
  }
else
  printf 'warn: %s answered nothing at all, with or without certificate checks.\n' "$GOOSE_URL" >&2
  printf '      Check the service before blaming the app:\n' >&2
  printf '        tailscale ssh agent@%s "systemctl is-active goose-serve"\n' "$BRAIN" >&2
  printf '      After a reboot the stack stays down until luks-unlock.sh runs.\n' >&2
fi

# ---- 3. clear the field ----------------------------------------------------
# Every running `dx serve` writes the same target directory and relaunches the
# same app, so a stale one silently replaces the build this script is about to
# make. Killing them is the difference between configuring the window you are
# looking at and configuring a different one.
if pgrep -f "dx serve" >/dev/null 2>&1; then
  note "stopping $(pgrep -f 'dx serve' | wc -l | tr -d ' ') running dx serve process(es)"
  pkill -f "dx serve" 2>/dev/null
  sleep 2
fi

# ---- 4. serve --------------------------------------------------------------
printf '\n  goose  %s\n  code   %s\n  cwd    %s\n\n' \
  "$GOOSE_URL" "$CODE_URL" "$REMOTE_WORKDIR"
printf '  The app starts DISCONNECTED by design — press Save & Connect once.\n\n'

cd "$(dirname "$0")/.." || die "cannot reach the repo root"

exec env \
  GOOSE_DEV_SERVER_URL="$GOOSE_URL" \
  GOOSE_DEV_SECRET_KEY="$GOOSE_SECRET" \
  GOOSE_DEV_FINGERPRINT="$FP" \
  GOOSE_DEV_WORKING_DIR="$REMOTE_WORKDIR" \
  GOOSE_DEV_CODE_URL="$CODE_URL" \
  GOOSE_DEV_CODE_PASSWORD="$CODE_SECRET" \
  dx serve --desktop
