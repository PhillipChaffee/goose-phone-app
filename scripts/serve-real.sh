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
REMOTE_WORKDIR="${REMOTE_WORKDIR:-/data/goose}"

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

GOOSE_SECRET="$(keyfetch GOOSE_SERVER__SECRET_KEY)"
CODE_SECRET="$(keyfetch OPENCODE_SERVER_PASSWORD)"

[ -n "$GOOSE_SECRET" ] || die "GOOSE_SERVER__SECRET_KEY not in keychain service '$KEYCHAIN_SERVICE'"
[ -n "$CODE_SECRET" ]  || die "OPENCODE_SERVER_PASSWORD not in keychain service '$KEYCHAIN_SERVICE'"
note "secrets: both present (values not read)"

# ---- 2. the certificate fingerprint ----------------------------------------
# Not a secret — it is a hash of a certificate the server hands to anyone who
# connects. Taken from the server's own startup line so it cannot drift from
# the certificate actually in use.
FP="${GOOSE_DEV_FINGERPRINT:-}"
if [ -z "$FP" ]; then
  note "fetching the certificate fingerprint from $BRAIN"
  FP="$(tailscale ssh "agent@$BRAIN" \
        "journalctl -u goose-serve 2>/dev/null | grep -o 'GOOSED_CERT_FINGERPRINT=[A-Fa-f0-9:]*' | tail -1" \
        2>/dev/null | cut -d= -f2)"
fi
if [ -z "$FP" ]; then
  printf 'warn: no fingerprint found.\n' >&2
  printf '      If goose is fronted by `tailscale serve` its certificate is a real\n' >&2
  printf '      one and no pin is needed — carry on. If goose holds its own TLS the\n' >&2
  printf '      certificate is self-signed and the connection WILL be refused; get\n' >&2
  printf '      the line yourself and re-run with GOOSE_DEV_FINGERPRINT=... :\n' >&2
  printf '        tailscale ssh agent@%s "journalctl -u goose-serve | grep -i fingerprint"\n' "$BRAIN" >&2
else
  note "fingerprint: found (${#FP} chars)"
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
printf '\n  goose  https://%s:%s\n  code   https://%s:%s\n  cwd    %s\n\n' \
  "$BRAIN" "$GOOSE_PORT" "$BRAIN" "$CODE_PORT" "$REMOTE_WORKDIR"
printf '  The app starts DISCONNECTED by design — press Save & Connect once.\n\n'

cd "$(dirname "$0")/.." || die "cannot reach the repo root"

exec env \
  GOOSE_DEV_SERVER_URL="https://$BRAIN:$GOOSE_PORT" \
  GOOSE_DEV_SECRET_KEY="$GOOSE_SECRET" \
  GOOSE_DEV_FINGERPRINT="$FP" \
  GOOSE_DEV_WORKING_DIR="$REMOTE_WORKDIR" \
  GOOSE_DEV_CODE_URL="https://$BRAIN:$CODE_PORT" \
  GOOSE_DEV_CODE_PASSWORD="$CODE_SECRET" \
  dx serve --desktop
