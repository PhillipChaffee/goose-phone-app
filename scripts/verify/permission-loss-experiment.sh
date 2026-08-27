#!/usr/bin/env bash
# permission-loss-experiment.sh — what happens to a pending permission ask when
# the client stops answering?
#
# Three accounts of this failure have been written into this repository, all
# from reading source, and they disagree:
#
#   1. goose answers its own question with Permission::Cancel, so the tool is
#      DECLINED and the transcript says the user declined it.
#   2. That arm never runs on a WebSocket, because teardown is abortive and
#      kills the actor that would poll it — so the whole ROUND is discarded,
#      including tool calls that already ran and changed the disk.
#   3. Nothing happens, because the server never notices: there is no
#      server-side keepalive, so a quiet client looks like a thinking one.
#
# Each leaves a different fingerprint in the transcript, so one run tells them
# apart. It has to run against a REAL goose with a provider configured — the
# mock parks a pending ask forever on a oneshot and would report health.
#
# WHY YOU HAVE TO RUN IT: the server's secret is in your Keychain and exported
# into your interactive shell. This script reads it as $GOOSE_SERVER__SECRET_KEY
# and never prints it. Nothing here echoes a secret value; no `set -x`.
#
#   ./scripts/verify/permission-loss-experiment.sh [base_url]
#
# It creates ONE throwaway session on the server, in /tmp, and asks the agent to
# run `uname -a`. It never answers the permission ask. Delete the session
# afterwards if you like; the id is printed.
set -uo pipefail

BASE="${1:-https://ai-brain.tail5ac550.ts.net:3284}"
: "${GOOSE_SERVER__SECRET_KEY:?not set — run this in your own shell, where the profile exports it from the Keychain}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$HERE/target/debug/examples/perm_loss"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/perm-loss.XXXXXX")"
echo "== work dir: $WORK"

if [ ! -x "$BIN" ]; then
  echo "== building the harness"
  (cd "$HERE" && cargo build -q -p goose-acp-client --example perm_loss) || exit 1
fi

# How the client dies. `-STOP` is the one that matters: iOS SUSPENDS a
# backgrounded app, so the process freezes with its socket still open and
# nobody reading it. `-9` is the different case where the fd is closed and the
# peer sees a FIN. Both are run, because which one goose sees changes the
# answer and only one of them is what a phone does.
run_case() {
  local signal="$1" label="$2" log="$WORK/$2.log"
  echo
  echo "=============================================================="
  echo "== CASE: $label  (kill -$signal)"
  echo "=============================================================="

  "$BIN" "$BASE" "$GOOSE_SERVER__SECRET_KEY" ask > "$log" 2>&1 &
  local wrapper=$!

  local pid="" sid=""
  for _ in $(seq 1 90); do
    if grep -q "PARKED" "$log" 2>/dev/null; then
      pid="$(grep -m1 -o 'PARKED pid=[0-9]*' "$log" | cut -d= -f2)"
      sid="$(grep -m1 -o 'PARKED .*session=.*' "$log" | sed 's/.*session=//')"
      break
    fi
    if grep -q "PROMPT RESOLVED" "$log" 2>/dev/null; then
      echo "!! the turn finished without ever asking. Server mode may be 'auto',"
      echo "!! or the model chose not to call a tool. Last lines:"
      tail -5 "$log"
      kill "$wrapper" 2>/dev/null
      return 1
    fi
    sleep 2
  done

  if [ -z "$pid" ]; then
    echo "!! no permission ask within 180s. Last lines:"; tail -8 "$log"
    kill "$wrapper" 2>/dev/null; return 1
  fi

  echo "-- ask received; session=$sid client pid=$pid"
  echo "-- freezing the client with kill -$signal (this is the phone going away)"
  kill -"$signal" "$pid" 2>/dev/null

  echo "-- waiting 75s for the server to notice, or not"
  sleep 75

  # Let a STOPped process go, so it does not linger holding a socket.
  if [ "$signal" = "STOP" ]; then kill -CONT "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null; fi
  kill -9 "$wrapper" 2>/dev/null

  echo
  echo "-- RECONNECTING and replaying the transcript --"
  "$BIN" "$BASE" "$GOOSE_SERVER__SECRET_KEY" inspect "$sid" 2>&1 | tail -60

  echo
  echo "-- WHAT TO LOOK FOR --"
  echo "   'declined' / status Failed on the tool  -> account 1 (goose answered for you)"
  echo "   the tool call MISSING from the replay   -> account 2 (the round was discarded)"
  echo "   the tool call present and still pending -> account 3 (server never noticed)"
  echo "   session id for cleanup: $sid"
}

run_case STOP suspended
run_case KILL closed

echo
echo "== done. Logs in $WORK"
