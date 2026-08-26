#!/usr/bin/env bash
# Verify that a goose server is in the shape Goose Mobile needs.
#
# Run this ON the VPS that runs goose:
#   ./scripts/check-server.sh
#
# Or point it at an already-known URL (from anywhere on the tailnet):
#   ./scripts/check-server.sh https://my-box.tailnet-name.ts.net 'my-secret'
#
# It never prints your secret key.

set -uo pipefail

BASE_URL="${1:-}"
SECRET="${2:-${GOOSE_SERVER__SECRET_KEY:-}}"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  R=$'\e[31m'; G=$'\e[32m'; Y=$'\e[33m'; B=$'\e[1m'; N=$'\e[0m'
else
  R=""; G=""; Y=""; B=""; N=""
fi

FAILED=0
ok()      { printf '%s  ok  %s %s\n' "$G" "$N" "$1"; }
warn()    { printf '%s warn %s %s\n' "$Y" "$N" "$1"; }
bad()     { printf '%s fail %s %s\n' "$R" "$N" "$1"; FAILED=1; }
section() { printf '\n%s== %s ==%s\n' "$B" "$1" "$N"; }
fix()     { printf '        %s→%s %s\n' "$Y" "$N" "$1"; }

# Processes matching a pattern, excluding this script and its shell.
find_pids() {
  pgrep -f "$1" 2>/dev/null | grep -vx -e "$$" -e "${PPID:-0}" || true
}

# The pid of the goose binary itself (not a wrapper shell whose command line
# happens to mention "goose serve"). Prefers an exact executable-name match.
find_goose_pid() {
  local pid
  for pid in $(pgrep -x goose 2>/dev/null); do
    if [ -r "/proc/$pid/cmdline" ]; then
      tr '\0' ' ' < "/proc/$pid/cmdline" | grep -q ' serve' && { printf '%s' "$pid"; return; }
    else
      printf '%s' "$pid"; return          # non-Linux: no /proc to confirm with
    fi
  done
  # Fall back to a full-command-line match, skipping obvious shell wrappers.
  for pid in $(find_pids 'goose serve'); do
    if [ -r "/proc/$pid/cmdline" ]; then
      tr '\0' ' ' < "/proc/$pid/cmdline" | grep -qE '(^|/)(ba|z|da|k)?sh( |$)' && continue
    fi
    printf '%s' "$pid"; return
  done
}

# --------------------------------------------------------------------------
section "1. goose process"

GOOSE_PID=""
if command -v pgrep >/dev/null 2>&1; then
  GOOSE_PID=$(find_goose_pid)
fi

if [ -n "$GOOSE_PID" ]; then
  ok "\`goose serve\` is running (pid $GOOSE_PID)"
  # /proc first because it is exact, `ps -o args=` second because it is what
  # macOS and the other BSDs have instead. The fallback is the point: this
  # block used to be inside the `/proc` test alone, so on a Mac — the platform
  # the README's quick start is written for — it printed nothing at all, and
  # the --enable-scheduler check below never ran on the one machine whose flag
  # it is about.
  if [ -r "/proc/$GOOSE_PID/cmdline" ]; then
    ARGS=$(tr '\0' ' ' < "/proc/$GOOSE_PID/cmdline")
  else
    ARGS=$(ps -o args= -p "$GOOSE_PID" 2>/dev/null)
  fi
  if [ -n "$ARGS" ]; then
    printf '        args: %s\n' "$ARGS"
    # A warn and never a failure: chat, recipes, skills and extensions all work
    # without it. It is checked at all because the Scheduler screen is
    # otherwise a sentence with no way to act on it — goose answers -32601 to
    # every schedules/* method, the app states the fact, and nothing anywhere
    # tells you the fix is a flag on this machine.
    if printf '%s' "$ARGS" | grep -q -- '--enable-scheduler'; then
      ok "server was started with --enable-scheduler"
    else
      warn "server has no --enable-scheduler — the Scheduler screen will be empty"
      fix "restart with: goose serve --enable-scheduler ..."
    fi
  fi
elif [ -n "$(find_pids 'goose')" ]; then
  warn "a goose process is running, but not \`goose serve\`"
  fix "the app needs the server mode: goose serve --host 127.0.0.1 --port 3284"
else
  warn "no local \`goose serve\` process found"
  fix "if goose runs in Docker/systemd elsewhere, skip to section 4 and test the URL directly"
fi

if command -v goose >/dev/null 2>&1; then
  VERSION=$(goose --version 2>/dev/null | tr -d '\n')
  MAJOR_MINOR=$(printf '%s' "$VERSION" | grep -oE '[0-9]+\.[0-9]+' | head -1)
  if [ -n "$MAJOR_MINOR" ]; then
    MAJ=${MAJOR_MINOR%%.*}; MIN=${MAJOR_MINOR##*.}
    if [ "$MAJ" -gt 1 ] || { [ "$MAJ" -eq 1 ] && [ "$MIN" -ge 42 ]; }; then
      ok "goose version $VERSION (>= 1.42, speaks ACP)"
    else
      bad "goose version $VERSION is older than 1.42 — it predates the ACP API this app uses"
      fix "upgrade: curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | bash"
    fi
  else
    warn "could not parse goose version output: $VERSION"
  fi
fi

# --------------------------------------------------------------------------
section "2. secret key"

if [ -n "$GOOSE_PID" ] && [ -r "/proc/$GOOSE_PID/environ" ]; then
  if tr '\0' '\n' < "/proc/$GOOSE_PID/environ" | grep -q '^GOOSE_SERVER__SECRET_KEY=.'; then
    ok "GOOSE_SERVER__SECRET_KEY is set for the running server"
  elif tr '\0' ' ' < "/proc/$GOOSE_PID/cmdline" | grep -q -- '--dangerously-unauthenticated'; then
    warn "server runs with --dangerously-unauthenticated (no secret)"
    fix "anyone on your tailnet can drive the agent; prefer setting GOOSE_SERVER__SECRET_KEY"
  else
    warn "GOOSE_SERVER__SECRET_KEY not visible in the server's environment"
    fix "if it starts via systemd, check the unit's Environment=/EnvironmentFile= lines"
    fix "the authoritative check is the /acp test in section 5 below"
  fi
elif [ -n "$GOOSE_PID" ]; then
  warn "cannot read the server's environment (try: sudo $0)"
fi

if [ -n "$SECRET" ]; then
  ok "a secret was supplied to this script (${#SECRET} chars) — will use it to test auth"
else
  warn "no secret available to this script"
  fix "re-run as: $0 '${BASE_URL:-<url>}' 'your-secret'   (or export GOOSE_SERVER__SECRET_KEY)"
fi

# --------------------------------------------------------------------------
section "3. listening socket"

PORT=""
SOCK_TOOL=""
for t in ss netstat lsof; do
  command -v "$t" >/dev/null 2>&1 && { SOCK_TOOL="$t"; break; }
done

case "$SOCK_TOOL" in
  ss)      LISTEN=$(ss -lntp 2>/dev/null | grep -i goose) ;;
  netstat) LISTEN=$(netstat -lntp 2>/dev/null | grep -i goose) ;;
  lsof)    LISTEN=$(lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null | grep -i goose) ;;
  *)       LISTEN="" ;;
esac

if [ -n "$LISTEN" ]; then
  printf '%s\n' "$LISTEN" | sed 's/^/        /'
  # Last :NNNN on the local-address field of the first line.
  PORT=$(printf '%s\n' "$LISTEN" | sed -n 1p | grep -oE ':[0-9]{2,5}' | tr -d ':' | sed -n 1p)
  if printf '%s' "$LISTEN" | grep -qE '127\.0\.0\.1:|\[::1\]:|localhost:'; then
    ok "bound to localhost — correct when \`tailscale serve\` fronts it (checked next)"
  else
    warn "bound to a non-loopback address — rely on tailnet ACLs/firewall to limit access"
  fi
elif [ -z "$SOCK_TOOL" ]; then
  warn "no ss/netstat/lsof available — skipping the socket check"
else
  warn "no listening socket owned by goose found (try: sudo $0)"
  fix "if goose runs in Docker or another namespace, check inside that container instead"
fi

[ -z "$PORT" ] && PORT=3284

# --------------------------------------------------------------------------
section "4. tailscale"

if command -v tailscale >/dev/null 2>&1; then
  if tailscale status >/dev/null 2>&1; then
    DNSNAME=$(tailscale status --json 2>/dev/null | grep -oE '"DNSName": *"[^"]+"' | head -1 | sed -E 's/.*"DNSName": *"([^"]+)".*/\1/' | sed 's/\.$//')
    TSIP=$(tailscale ip -4 2>/dev/null | head -1)
    [ -n "$TSIP" ] && ok "this machine is on the tailnet at $TSIP"
    [ -n "$DNSNAME" ] && ok "MagicDNS name: $DNSNAME"

    SERVE=$(tailscale serve status 2>/dev/null)
    if [ -n "$SERVE" ] && ! printf '%s' "$SERVE" | grep -qi 'no serve config'; then
      ok "tailscale serve is active:"
      printf '%s\n' "$SERVE" | sed 's/^/        /'
      SERVED=$(printf '%s' "$SERVE" | grep -oE 'https://[^ ]+' | head -1)
      [ -z "$BASE_URL" ] && [ -n "$SERVED" ] && BASE_URL="${SERVED%/}"
    else
      warn "tailscale serve is NOT fronting anything"
      fix "recommended (real HTTPS cert, tailnet-only): sudo tailscale serve --bg $PORT"
      if [ -z "$BASE_URL" ] && [ -n "$DNSNAME" ]; then
        BASE_URL="http://$DNSNAME:$PORT"
        fix "without it, the app must use plain http: $BASE_URL (works, but see README)"
      fi
    fi
  else
    bad "tailscale is installed but not connected"
    fix "sudo tailscale up"
  fi
else
  warn "tailscale CLI not found on this machine"
fi

# --------------------------------------------------------------------------
section "5. HTTP checks"

if [ -z "$BASE_URL" ]; then
  BASE_URL="http://127.0.0.1:$PORT"
  warn "no tailnet URL determined; testing locally at $BASE_URL"
fi
printf '        testing: %s\n' "$BASE_URL"

CURL_OPTS=(-s -o /dev/null -w '%{http_code}' --max-time 12)
case "$BASE_URL" in
  https://*.ts.net*) : ;;                       # real cert from tailscale serve
  https://*) CURL_OPTS+=(-k) ;;                 # self-signed goose --tls
esac

STATUS_CODE=$(curl "${CURL_OPTS[@]}" "$BASE_URL/status" 2>/dev/null)
case "$STATUS_CODE" in
  200) ok "GET /status → 200 (server is up)" ;;
  000) bad "GET /status → no response (unreachable: wrong host/port, firewall, or server down)" ;;
  *)   bad "GET /status → HTTP $STATUS_CODE (expected 200 — is this really a goose server?)" ;;
esac

if [ -n "$SECRET" ]; then
  ACP_CODE=$(curl "${CURL_OPTS[@]}" -H "X-Secret-Key: $SECRET" "$BASE_URL/acp" 2>/dev/null)
  case "$ACP_CODE" in
    406) ok "GET /acp with secret → 406 (auth accepted — this is the expected success code)" ;;
    401|403) bad "GET /acp with secret → HTTP $ACP_CODE (secret rejected)" ;;
    000) bad "GET /acp → no response" ;;
    *)   bad "GET /acp → HTTP $ACP_CODE (expected 406; an ACP endpoint may not be present)" ;;
  esac
else
  NOAUTH=$(curl "${CURL_OPTS[@]}" "$BASE_URL/acp" 2>/dev/null)
  case "$NOAUTH" in
    401) ok "GET /acp without a secret → 401 (auth is enabled, as it should be)" ;;
    406) warn "GET /acp without a secret → 406 (server accepts UNAUTHENTICATED clients)" ;;
    *)   warn "GET /acp without a secret → HTTP $NOAUTH" ;;
  esac
fi

# --------------------------------------------------------------------------
section "6. provider + working directory"

if command -v goose >/dev/null 2>&1; then
  CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/goose/config.yaml"
  if [ -f "$CONFIG" ]; then
    ok "goose config found at $CONFIG"
    grep -iE '^(GOOSE_PROVIDER|GOOSE_MODEL|provider|model):' "$CONFIG" 2>/dev/null | sed 's/^/        /'
  else
    warn "no goose config at $CONFIG — if the server has no provider, chats will fail"
    fix "run \`goose configure\` as the user that runs the server"
  fi
fi

printf '\n%sWorking directory for the app:%s pick an absolute path that exists on THIS machine,\n' "$B" "$N"
printf 'e.g. %s — new chats start there.\n' "${HOME:-/root}"

# --------------------------------------------------------------------------
section "Summary"

if [ "$FAILED" -eq 0 ]; then
  printf '%sAll critical checks passed.%s Put this in the app:\n\n' "$G" "$N"
  printf '  Server URL:        %s\n' "$BASE_URL"
  printf '  Secret key:        your GOOSE_SERVER__SECRET_KEY\n'
  printf '  Working directory: %s\n' "${HOME:-/root}"
  exit 0
else
  printf '%sSome checks failed%s — fix the items marked "fail" above, then re-run.\n' "$R" "$N"
  exit 1
fi
