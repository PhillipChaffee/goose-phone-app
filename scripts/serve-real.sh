#!/usr/bin/env bash
# Serve the desktop shell against a REAL brain instead of the mocks.
#
#   ./scripts/serve-real.sh                 # BRAIN_HOST from the environment
#   ./scripts/serve-real.sh my-brain.ts.net
#   ./scripts/serve-real.sh --fresh         # ...and let the seeds actually win
#
# CLAUDE.md's development recipe points at the two mock servers. This is the
# same recipe pointed at a real one, and it exists because four of the six
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
#   * A SEED IS ONLY THE DEFAULT, AND THE SAVED FILE OUTRANKS IT (#279). Since
#     #220 `settings` is fs-backed; it is loaded before `dev_seed!` can apply
#     and it wins. So this script used to print a host it could not promise —
#     it said `ai-brain...:3284` while the app connected to `127.0.0.1:3285`,
#     a mock left over from a capture run the day before. Section 3 below reads
#     the saved file and says so; `--fresh` moves it aside for good.
#
# SECRETS ARE MOVED BY REFERENCE AND NEVER PRINTED. Both are read straight out
# of the login keychain into the child's environment by shell expansion. This
# script has no branch that echoes one, and it does not write one to a file.
# What it reports is presence, never value.
#
# Two consequences of the seeds being environment variables, stated because
# they are easy not to think about and this is a real credential rather than a
# mock's: the values are visible in `ps -E` output for the `dx serve` process
# while it runs, and `option_env!` bakes them into the unstripped binary under
# `target/`. Neither can ride into a release build — `dev_seed!` expands to an
# empty string there — but both are on this disk until you rebuild. If that is
# not a trade you want, leave the seeds off and type the fields into Settings
# instead. Those DO survive a restart now — this paragraph used to end "they
# will not, because `use_persistent` is in-memory `SessionStorage` on every
# non-wasm target", which #220 made false in the same stroke that created the
# #279 above. The two secrets are still not written, by construction: they are
# `#[serde(skip_serializing)]` and `the_saved_settings_file_holds_no_credential`
# holds the serializer to it, so typing them in really is per-launch.

set -uo pipefail

# `--fresh` may appear anywhere; the first non-flag word is the brain host.
# Written without arrays because macOS still ships bash 3.2, where `set -u`
# treats `${arr[0]}` of an empty array as unbound and kills the script.
BRAIN=""
FRESH="${GOOSE_DEV_RESET:-0}"
for arg in "$@"; do
  case "$arg" in
    --fresh) FRESH=1 ;;
    *) [ -n "$BRAIN" ] || BRAIN="$arg" ;;
  esac
done
BRAIN="${BRAIN:-${BRAIN_HOST:-}}"
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

# ---- 3. the saved settings file, which outranks every seed below ------------
# THIS SECTION EXISTS BECAUSE EVERYTHING BELOW IT IS ONLY A DEFAULT. `dev_seed!`
# is an `option_env!` read into `Settings::default()`, and since #220 a saved
# `settings` file is loaded in preference to that default. The seeds therefore
# decide only on a machine that has never pressed Save — which, after three
# capture sessions, this one is not.
#
# The failure #279 reported is not that the app is wrong; it is that this script
# was CONFIDENTLY wrong, printing `ai-brain...:3284` while the app used a mock
# from the day before. It failed loudly only because that mock was dead. A live
# one would have connected to the wrong server and looked entirely correct.
#
# READING IT NEEDS A DECODER, WHICH IS THE OTHER THING NOBODY EXPECTS. The file
# is hex-encoded zlib-compressed CBOR, all three layers dioxus-sdk-storage's
# choice — `grep server_url` over it finds nothing, and a launcher that took
# that silence for "no saved value" would print the opposite of the truth. The
# decode is `scripts/read-saved-settings.py`, which emits three named fields and
# refuses to dump the rest of the map.
SETTINGS_FILE="${GOOSE_SETTINGS_FILE:-$HOME/Library/Application Support/goose-mobile/settings}"
READER="$(dirname "$0")/read-saved-settings.py"
PY="${PYTHON:-python3}"

if [ "$FRESH" = 1 ] && [ -f "$SETTINGS_FILE" ]; then
  # Moved, not deleted: the four persisted fields are the ones somebody typed,
  # and the app rewrites this path on the next Save anyway.
  ASIDE="$SETTINGS_FILE.superseded.$(date +%Y%m%d-%H%M%S)"
  if mv "$SETTINGS_FILE" "$ASIDE"; then
    note "saved settings: moved aside — the seeds below now decide"
    note "                $ASIDE"
  else
    die "could not move $SETTINGS_FILE aside"
  fi
fi

# Print the value the app has saved for one field, or return 1 if it has none.
# Presence is the line existing, not the value being non-empty: a saved EMPTY
# working directory is the most confusing contradiction of the lot, because the
# app connects, lists history and looks healthy right up until the first message
# fails with "invalid directory path".
saved_field() {
  printf '%s\n' "$SAVED" | awk -F'\t' -v k="$1" '$1 == k { print $2; f = 1 } END { exit !f }'
}

warn_if_saved_differs() { # $1 label, $2 field name, $3 what we are about to seed
  local was
  was="$(saved_field "$2")" || return 0
  [ "$was" = "$3" ] && return 0
  MISMATCH=$((MISMATCH + 1))
  printf 'warn: the app will use its SAVED %s, not the one below.\n' "$1" >&2
  printf '        saved    %s\n' "${was:-(empty)}" >&2
  printf '        seeding  %s\n' "$3" >&2
}

MISMATCH=0
if [ ! -f "$SETTINGS_FILE" ]; then
  # Silent after `--fresh`: the line above already said what happened, and
  # saying "none" straight afterwards reads like a second, different fact.
  [ "$FRESH" = 1 ] || note "saved settings: none — the seeds below are the whole story"
elif ! command -v "$PY" >/dev/null 2>&1; then
  printf 'warn: a saved settings file OUTRANKS the seeds below, and no python3\n' >&2
  printf '      is on PATH to say which of its fields differ.\n' >&2
  printf '        %s\n' "$SETTINGS_FILE" >&2
  printf '      Re-run with --fresh to move it aside.\n' >&2
elif ! SAVED="$("$PY" "$READER" "$SETTINGS_FILE" \
      server_url code_server_url working_dir 2>/dev/null)"; then
  printf 'warn: a saved settings file OUTRANKS the seeds below, and it did not\n' >&2
  printf '      decode — so this cannot say which of its fields differ.\n' >&2
  printf '        %s\n' "$SETTINGS_FILE" >&2
  printf '      Re-run with --fresh to move it aside.\n' >&2
else
  warn_if_saved_differs "server URL" server_url "$GOOSE_URL"
  warn_if_saved_differs "code URL" code_server_url "$CODE_URL"
  warn_if_saved_differs "working directory" working_dir "$REMOTE_WORKDIR"
  if [ "$MISMATCH" -eq 0 ]; then
    note "saved settings: agree with the seeds below"
  else
    printf '      saved in %s\n' "$SETTINGS_FILE" >&2
    printf '      Re-run with --fresh to move it aside, or edit the field in\n' >&2
    printf '      Settings and press Save & Connect.\n' >&2
  fi
fi

# ---- 4. clear the field ----------------------------------------------------
# Every running `dx serve` relaunches the same app, so a stale one silently
# replaces the build this script is about to make. Killing them is the
# difference between configuring the window you are looking at and configuring
# a different one. Two on the SAME profile also write the same target
# directory, so the last writer wins there as well; since this script serves
# `--profile fast` (section 5) and a bare `dx serve --desktop` is `dev`, that
# second collision is now only between two runs of this script — the window
# problem is the one that always applies, which is why the kill is
# unconditional.
if pgrep -f "dx serve" >/dev/null 2>&1; then
  note "stopping $(pgrep -f 'dx serve' | wc -l | tr -d ' ') running dx serve process(es)"
  pkill -f "dx serve" 2>/dev/null
  sleep 2
fi

# ---- 5. serve --------------------------------------------------------------
printf '\n  goose  %s\n  code   %s\n  cwd    %s\n\n' \
  "$GOOSE_URL" "$CODE_URL" "$REMOTE_WORKDIR"
printf '  The app starts DISCONNECTED by design — press Save & Connect once.\n\n'

cd "$(dirname "$0")/.." || die "cannot reach the repo root"

# `--profile fast` is `Cargo.toml`'s own profile: `inherits = "dev"` plus
# `opt-level = 2`. It is here and not `--release` because the six lines above
# would all become empty strings in a release build — `dev_seed!` is
# `#[cfg(debug_assertions)]` on purpose — and the two secrets are
# `#[serde(skip_serializing)]`, so nothing would refill them. Inheriting `dev`
# keeps `debug-assertions = true`, so they still arrive; verified by reading
# `-C debug-assertions=on` and `-C opt-level=2` back out of the rustc arguments
# dx captured for this build.
#
# The first run after this landed pays a cold build of the whole graph at
# `opt-level = 2` (2m31s here against 49s for `dev`, `cargo build -p
# goose-mobile`); after that a save costs what it did, because `inherits =
# "dev"` keeps `incremental = true` and only the changed unit is re-optimised
# (2.0-2.7s either way). What it buys is 6.6x — a 600-item markdown re-parse
# goes 13.67 ms to 2.08 ms. The profile's own comment in `Cargo.toml` has the
# table and the two cheaper options it beat.
exec env \
  GOOSE_DEV_SERVER_URL="$GOOSE_URL" \
  GOOSE_DEV_SECRET_KEY="$GOOSE_SECRET" \
  GOOSE_DEV_FINGERPRINT="$FP" \
  GOOSE_DEV_WORKING_DIR="$REMOTE_WORKDIR" \
  GOOSE_DEV_CODE_URL="$CODE_URL" \
  GOOSE_DEV_CODE_PASSWORD="$CODE_SECRET" \
  dx serve --desktop --profile fast
