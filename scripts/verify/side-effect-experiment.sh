#!/usr/bin/env bash
# side-effect-experiment.sh — when the round is discarded, does the WORK go too?
#
# NOT YET RUN. This is the open half of docs/permission-durability.md: §7.6.
# Its sibling, scripts/verify/permission-loss-experiment.sh, is answered — it
# established that a pending permission ask costs you the whole round within 75
# seconds, whether or not the socket ever closed (§0). What it could not touch
# is the claim that sets the SEVERITY of that loss, in §3:
#
#     "the side effects of tools that already ran remain on disk, with nothing
#      in the session file to say they happened"
#
# That run cannot speak to it, because the tool never executed: it was blocked
# on the permission for the entire experiment. Nothing has yet observed a goose
# round that both DID something and lost the record of it. This script is the
# one that looks.
#
# THE SHAPE OF THE THING
#
# The turn has to contain two tool calls: a first that completes and lands a
# mark on the server's disk, and a second that is still asking when the client
# dies. Getting that is not a matter of asking nicely, because of how goose
# orders the two:
#
#   * Approved tools are dispatched into `tool_futures`
#     (goose crates/goose/src/agents/agent.rs:865-910), but those futures are
#     not POLLED until the approval stream has finished (agent.rs:2654-2683),
#     and `handle_approval_tool_requests` awaits each confirmation one at a
#     time (crates/goose/src/agents/tool_execution.rs:183). A tool approved in
#     the same LLM message as a tool that is still asking therefore does not
#     run at all.
#   * A round's messages are persisted at the BOTTOM of each loop iteration
#     (agent.rs:3339, in the loop opened at :2331). A tool that finished in an
#     EARLIER round of the same turn was already written to the session file.
#
# So the two shapes are different experiments, and both are run:
#
#   SEQUENTIAL — write in round 1, ask in round 2. Source predicts the write
#                survives in the transcript, which would SOFTEN §3 a lot.
#   BATCHED    — both calls in one message. Source predicts the write never
#                executes, so there is no orphaned side effect to fear.
#
# Two source reads. Section 0 falsified two source reads. That is the entire
# reason this is a script and not a paragraph.
#
# HOW THE SIDE EFFECT IS OBSERVED WITHOUT THE TRANSCRIPT
#
# The probe is a file on the SERVER's disk — /tmp/perm-loss-probe-<nonce>.txt,
# on the machine running goose, which over a tailnet is not this machine. Two
# ways to look, and the script does both when it can:
#
#   1. PROBE_CHECK_CMD, if you set it: any command that prints the file, e.g.
#        PROBE_CHECK_CMD='ssh ai-brain cat' ./scripts/verify/side-effect-experiment.sh
#      This is the authoritative one. The script appends the path.
#   2. A `readback` session otherwise: a SEPARATE, throwaway goose session on
#      the same server that runs `cat` on the probe. A fresh session has its
#      own history, so this does not disturb the transcript under test — the
#      trap §7.6 warns about is asking the SAME session, and this is not that.
#
# The readback is a model reporting on a file, so it is nonce-guarded. Two
# nonces: one in the PATH, which the readback prompt must name, and one in the
# CONTENTS, which it is never told. A model that invents a successful read
# cannot invent the content nonce. If the content nonce comes back, the file is
# real; if it does not, believe nothing and use route 1.
#
# WHY YOU HAVE TO RUN IT: the server's secret is in your Keychain and exported
# into your interactive shell. This reads it as $GOOSE_SERVER__SECRET_KEY and
# passes it to the harness THROUGH THE ENVIRONMENT, not argv — `sideeffect`
# parks for over a minute and argv is visible to `ps`. Nothing here echoes a
# secret value; no `set -x`.
#
#   ./scripts/verify/side-effect-experiment.sh [base_url]
#
# COST. Two real turns of two tool calls each, plus a readback turn each: about
# six provider calls on a small prompt, so cents. Wall clock is ~4 minutes per
# case, nearly all of it the 75-second wait and the model's own latency. It
# writes two files to the server's /tmp and creates four throwaway sessions
# (two under test, two readback — the readback ones delete themselves). The
# session ids and the `rm` commands are printed at the end; nothing is deleted
# for you, because the sessions under test ARE the result.
#
# FREE, IF YOU ARE THERE ANYWAY: capture the SERVER's log across this run and
# §7.1 falls out of it too. `error!("permission request failed")` at
# /Users/phillipchaffee/git/goose/crates/goose/src/acp/server.rs:1313 present
# means the Err arm ran and its Permission::Cancel was thrown away with the
# round; absent means the task was aborted before it could. That is the last
# thing standing between §5.1 and a pull request, and it costs one extra
# terminal.
set -uo pipefail

BASE="${1:-https://ai-brain.tail5ac550.ts.net:3284}"
: "${GOOSE_SERVER__SECRET_KEY:?not set — run this in your own shell, where the profile exports it from the Keychain}"
export GOOSE_SERVER__SECRET_KEY

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$HERE/target/debug/examples/perm_loss"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/side-effect.XXXXXX")"
echo "== work dir: $WORK"

if [ ! -x "$BIN" ]; then
  echo "== building the harness"
  (cd "$HERE" && cargo build -q -p goose-acp-client --example perm_loss) || exit 1
fi

# `-` tells the harness to take the secret from the environment.
run() { "$BIN" "$BASE" - "$@"; }

# Only `kill -STOP` here. Section 0 already measured that STOP and KILL give the
# same answer for the round, so paying for both again buys nothing; STOP is the
# one that is shaped like a phone, which suspends a backgrounded app with its
# socket still open.
run_case() {
  local shape="$1" log="$WORK/$1.log"
  local path_nonce content_nonce probe
  path_nonce="$(openssl rand -hex 6)"
  content_nonce="probe-$(openssl rand -hex 8)"
  probe="/tmp/perm-loss-probe-$path_nonce.txt"

  echo
  echo "=============================================================="
  echo "== CASE: $shape"
  echo "== probe file:    $probe"
  echo "== probe content: $content_nonce   (never said to the readback)"
  echo "=============================================================="

  run sideeffect "$path_nonce" "$content_nonce" "$shape" > "$log" 2>&1 &
  local wrapper=$!

  local pid="" sid=""
  for _ in $(seq 1 120); do
    if grep -q "PARKED" "$log" 2>/dev/null; then
      pid="$(grep -m1 -o 'PARKED pid=[0-9]*' "$log" | cut -d= -f2)"
      sid="$(grep -m1 -o 'PARKED .*session=.*' "$log" | sed 's/.*session=//')"
      break
    fi
    if grep -q "PROMPT RESOLVED" "$log" 2>/dev/null; then
      echo "!! the turn finished without a second ask. Either the model made only"
      echo "!! one tool call, or the server is not in 'approve' mode. Last lines:"
      tail -8 "$log"
      kill "$wrapper" 2>/dev/null
      return 1
    fi
    sleep 2
  done

  if [ -z "$pid" ]; then
    echo "!! no parked ask within 240s. Last lines:"; tail -10 "$log"
    kill "$wrapper" 2>/dev/null; return 1
  fi

  echo "-- SETUP VERDICT (from the live stream, before anything died):"
  grep -E 'ASK:|ANSWERING|WRITE COMPLETED|SETUP |SHAPE OBSERVED' "$log" | sed 's/^/     /'
  echo "-- session=$sid client pid=$pid"

  echo "-- freezing the client with kill -STOP (this is the phone going away)"
  kill -STOP "$pid" 2>/dev/null
  echo "-- waiting 75s"
  sleep 75
  kill -CONT "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
  kill -9 "$wrapper" 2>/dev/null

  echo
  echo "-- OBSERVATION 1: the transcript. Does it admit the write happened? --"
  local replay="$WORK/$shape.replay"
  run inspect "$sid" > "$replay" 2>&1
  tail -60 "$replay"
  echo
  # NOT a grep for the nonce. The user's prompt names the probe path and the
  # content nonce, and the user's prompt is precisely the thing section 0 found
  # survives — so "the nonce is in the replay" is true even when nothing else
  # is. The discriminator is whether a TOOL CALL was replayed at all, which in
  # section 0 was zero.
  local tool_lines write_lines
  tool_lines="$(grep -cE '^REPLAY ToolCall' "$replay")"
  # And of those, the ones that are the WRITE rather than the second tool. A
  # replayed shell call and a replayed write mean opposite things.
  write_lines="$(grep -E '^REPLAY ToolCall' "$replay" | grep -c "$path_nonce")"
  echo "   TRANSCRIPT: $tool_lines replayed tool-call update(s), $write_lines naming the probe."
  grep -E '^REPLAY ToolCall' "$replay" | sed 's/^/     /'
  local in_transcript=no
  if [ "$write_lines" -gt 0 ]; then in_transcript=yes; fi
  if [ "$tool_lines" -gt 0 ] && [ "$write_lines" -eq 0 ]; then
    echo "   !! tool calls were replayed but none names the probe. Read the lines"
    echo "   !! above by hand before believing the verdict: perm_loss's inspect"
    echo "   !! truncates each update at 160 chars, so a long title can hide the"
    echo "   !! path (crates/goose-acp-client/examples/perm_loss.rs, fn short)."
  fi

  echo
  echo "-- OBSERVATION 2: the server's disk. Did the write happen? --"
  local disk="$WORK/$shape.disk"
  if [ -n "${PROBE_CHECK_CMD:-}" ]; then
    # shellcheck disable=SC2086
    ${PROBE_CHECK_CMD} "$probe" > "$disk" 2>&1
    echo "   (via PROBE_CHECK_CMD)"
  else
    echo "   (no PROBE_CHECK_CMD set — using a separate readback session)"
    run readback "$probe" > "$disk" 2>&1
  fi
  tail -25 "$disk"
  echo
  local on_disk=no
  if grep -q "$content_nonce" "$disk"; then on_disk=yes; fi
  echo "   DISK: file present with the right contents? $on_disk"

  echo
  echo "-- WHAT THIS CASE MEANS --"
  case "$on_disk/$in_transcript" in
    yes/no)
      echo "   PREDICTED-WORST. The work is on disk and the session denies it."
      echo "   docs/permission-durability.md section 3 stands as written."
      ;;
    yes/yes)
      echo "   SOFTER. The finished round WAS flushed; only the in-flight round"
      echo "   is lost. Section 3's severity claim needs rewriting: the user"
      echo "   loses the pending ask, not the record of completed work."
      ;;
    no/no)
      echo "   NO ORPHAN. The write never ran, so there is nothing to be"
      echo "   orphaned. Check the SETUP VERDICT above before concluding: if it"
      echo "   said DEGRADED or INVALID this is a setup failure, not a result."
      echo "   If it said SETUP OK, section 3's fear is unfounded for this shape."
      ;;
    no/yes)
      echo "   FIFTH WORLD. The transcript claims the write and the file is not"
      echo "   there. Check the DISK output first — a readback that never showed"
      echo "   the content nonce proves nothing, so re-check with PROBE_CHECK_CMD"
      echo "   before believing this. If it holds, write it up on its own;"
      echo "   nothing in section 7.6 predicted it."
      ;;
    *) echo "   unclassified: on_disk=$on_disk in_transcript=$in_transcript" ;;
  esac
  echo
  echo "   session under test (keep it, it is the evidence): $sid"
  echo "   probe to clean up on the server later: rm -f $probe"
}

run_case sequential
run_case batched

echo
echo "== done. Logs, replays and disk checks in $WORK"
echo "== Read the SETUP VERDICT of each case first. A case whose write never"
echo "== completed measures nothing about side effects; it just re-runs the"
echo "== experiment that is already answered."
