#!/usr/bin/env bash
# Capture the README screenshots from a running app on a booted simulator.
#
# These are real device captures, not renders: the previous set came from
# docs/style-gallery.html, which is a hand-maintained copy of the DOM and had
# drifted from what the app actually produced. A capture of the running app
# cannot drift.
#
#   1. cargo run -p mock-goose-server 3285
#   2. build + install with the GOOSE_DEV_* seeds (see docs/design.md)
#   3. drive the app to the screen you want
#   4. scripts/shoot-simulator.sh <name>
#
# Writes docs/images/<name>.png at the simulator's native scale.
set -euo pipefail
name=${1:?usage: shoot-simulator.sh <name>}
out="$(cd "$(dirname "$0")/.." && pwd)/docs/images/$name.png"
xcrun simctl io booted screenshot "$out" >/dev/null
echo "$out  ($(du -h "$out" | cut -f1))"
