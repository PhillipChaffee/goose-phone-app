#!/usr/bin/env python3
"""Generate docs/style-gallery.html from the DOM the running app emits.

The gallery used to be a hand-written copy of the views' markup, and it
drifted far enough that a review pass examined states the app no longer
produced. This reads the real thing instead: build a debug binary (which
includes src/domdump.rs), drive the app to each state you want, and every
screen change prints its `.app` subtree to the console.

    xcrun simctl launch --console-pty booted com.goosemobile.app > /tmp/applog.txt
    ...drive the app...
    cargo run > /tmp/desktop.log      # the other shell, in the same session
    ...drive that too...
    scripts/capture-gallery.py /tmp/applog.txt /tmp/desktop.log

A RUN REPLACES THE GALLERY, and a run may be SEVERAL LOGS. Keeping states you
did not visit is how the old hand-written gallery went stale in the first
place — a screen captured on another branch survives every later run,
`docs/audit.js` keeps reporting it clean, and nothing ever says the markup no
longer exists. That guarantee is why several logs had to be one invocation
rather than two: the app ships two shells now, they write into one store, and
a phone run followed by a desktop run would each delete the other's states.
Pass --merge if you really do want to build the set up over several runs; it
names every key it carries over so a stale one cannot pass unnoticed.

Which shell a state came from is in its KEY — `src/shell/mod.rs` gives the
desktop's dumps a `desktop-` prefix and the phone's none — and that one string
does both jobs it has to do: it keeps the two sets from colliding here, and it
is what `docs/audit.js` partitions on when it decides which stylesheets and
which viewport sizes a state is measured against.
"""

from __future__ import annotations

import html
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GALLERY = ROOT / "docs" / "style-gallery.html"
STORE = ROOT / "docs" / "gallery-states.json"

# key -> the label the gallery shows. Keys come from `Destination::key` in
# src/nav.rs — the same table the app navigates by, so a screen that is not in
# it is a screen the gallery never sees.
LABELS = {
    "chats": "Chats",
    "chat": "Chat transcript",
    "code-list": "Code sessions",
    "code-new": "New code session",
    "code-chat": "Code agent transcript",
    "code-diff": "Code review",
    "code-pulls": "Pull requests",
    "extensions": "Extensions",
    "extensions-detail": "Extension detail",
    "skills": "Skills",
    "skill": "Skill detail",
    "recipes-list": "Recipes",
    "recipes-detail": "Recipe detail",
    "scheduler": "Scheduler",
    "scheduler-detail": "Scheduled job",
    "settings": "Settings",
}

# The prefix `src/shell/mod.rs` puts on a desktop dump's key. One string, two
# jobs: it keeps a desktop `chats` from overwriting the phone's in the store,
# and it is what decides which stylesheets and which frame a state gets below.
DESKTOP = "desktop-"

# EVERY stylesheet the app embeds, in the order `src/css.rs` concatenates them.
#
# main.css alone was a live bug, and the same one `docs/audit.js` documents at
# its own top ("Linking main.css alone would rebuild every feature screen
# unstyled and then measure the result, which passes"). The audit fixed it for
# itself; this file, which is generated alongside it and read by a human rather
# than by a walk, never got the same fix — so every recipes, skills, scheduler,
# extensions and session-history frame in the gallery has been rendering
# against 149 lines of missing CSS. Enumerated rather than listed for the
# reason `src/css.rs` states: a feature brings a sheet of its own, and a list
# here would be a second place to remember.
FEATURES = sorted(p.name for p in (ROOT / "assets" / "features").glob("*.css"))

# And the two sheets that are deliberately NOT in that directory, because the
# audit links the whole of it into phone frames (`src/css.rs` explains both).
# They belong to the desktop frames alone.
#
# ORDER IS NOT FREE: `src/app.rs` emits STYLES, then SHELL (desktop.css), then
# PLATFORM (macos.css) — and it carries the write-up of what getting that wrong
# did, since the two sheets set `--chrome-h`/`--traffic-w` on the same
# `.app > .shell` selector at equal specificity and the later one wins. Linked
# the other way round the reservation is always zero and the gallery would show
# a nav toggle sitting on the macOS close button, which is a window nobody
# ships.
SHELL_SHEETS = ["../assets/desktop.css", "../assets/platform/macos.css"]


def head(key: str) -> str:
    sheets = ["../assets/main.css"] + [f"../assets/features/{f}" for f in FEATURES]
    if key.startswith(DESKTOP):
        sheets += SHELL_SHEETS
    links = "".join(f'<link rel="stylesheet" href="{href}">' for href in sheets)
    return (
        '<meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1, '
        'maximum-scale=1, viewport-fit=cover">' + links
    )


# The frame a state is rendered in, which is a fact about the shell that drew
# it. 402x874 is an iPhone 17 Pro and the size every phone state was captured
# at; 1180x820 is `with_inner_size` in src/main.rs — the window the desktop
# build actually opens. A desktop state in a phone bezel is a 1180pt layout
# folded into 402, which is not a state this app has.
FRAMES = {
    False: {"w": 402, "h": 874, "radius": 44},
    # macOS rounds a window's own corners at about 10pt, and there is no bezel:
    # the chrome is inside the app on this shell (`src/main.rs` hides the
    # titlebar), so what the frame has to show is the app's whole surface.
    True: {"w": 1180, "h": 820, "radius": 10},
}

PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>goose mobile — style gallery</title>
<style>
  /* Gallery chrome only. Nothing here reaches inside a frame: each state is
     an <iframe srcdoc>, so no font, colour or inherited property crosses the
     boundary and what you see is the app's own stylesheet doing all of it. */
  body {{
    margin: 0;
    padding: 24px;
    background: #8a8a8a;
    font: 13px -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    color: #fff;
  }}
  h1 {{ margin: 0 0 4px; font-size: 18px; font-weight: 600; }}
  .note {{ margin: 0 0 24px; max-width: 62ch; opacity: 0.85; line-height: 1.5; }}
  .grid {{ display: flex; flex-wrap: wrap; gap: 24px; align-items: flex-start; }}
  .label {{ margin-bottom: 6px; font-weight: 600; }}
  .shell {{ overflow: hidden; box-shadow: 0 10px 30px rgb(0 0 0 / 0.35); }}
  iframe.gallery-frame {{ border: 0; display: block; }}
</style>
</head>
<body>
<h1>goose mobile — style gallery</h1>
<p class="note">
  <strong>Generated, not written.</strong> Every frame below is the real
  <code>.app</code> subtree captured out of the running app by
  <code>src/domdump.rs</code> and written here by
  <code>scripts/capture-gallery.py</code>. Do not edit this file by hand — a
  hand-maintained copy is what drifted from the app last time. A phone frame is
  a 402×874 viewport (iPhone 17 Pro) linking the design system and every
  feature sheet; a <strong>desktop</strong> frame is the 1180×820 window
  <code>src/main.rs</code> opens, and links
  <code>../assets/desktop.css</code> and
  <code>../assets/platform/macos.css</code> after them, in the order
  <code>src/app.rs</code> emits them. So the styling is exactly what ships, on
  either shell. Frames render in whichever colour scheme your OS is set to;
  check both.
</p>
<p class="note">
  The traffic lights are AppKit's, not the app's. A desktop frame shows an
  empty <code>.traffic-slot</code> where macOS paints them, so this can say
  nothing covers that reservation and nothing about whether the reservation is
  where the lights really are. That stays a device check.
</p>
<p class="note">
  Safe-area insets are zero here — a browser has no notch — so the floating
  chrome sits higher than it does on a device. Positions are not what this is
  for; colour, type, spacing and state are.
</p>
<div class="grid">
{cells}
</div>
</body>
</html>
"""

CELL = """  <div class="cell" style="width: {w}px">
    <div class="label">{label}</div>
    <div class="shell" style="width: {w}px; height: {h}px; border-radius: {radius}px">
      <iframe class="gallery-frame" title="{label}" width="{w}" height="{h}"
              style="width: {w}px; height: {h}px"
              srcdoc="{srcdoc}"></iframe>
    </div>
  </div>"""


def main() -> int:
    args = [a for a in sys.argv[1:] if a != "--merge"]
    merge = "--merge" in sys.argv
    # SEVERAL logs, unioned, and the store still replaced wholesale from the
    # union. That is what keeps "a run replaces the gallery" true now that
    # there are two shells writing into one store: --merge is not the answer,
    # because it exists precisely to SHOUT that a carried-over key may be
    # stale, and using it on every routine capture would turn the freshness
    # guarantee into noise nobody reads.
    logs = [Path(a) for a in args] or [Path("/tmp/applog.txt")]
    if missing := [p for p in logs if not p.exists()]:
        for path in missing:
            print(f"no console log at {path}", file=sys.stderr)
        return 1

    previous: dict[str, str] = {}
    if STORE.exists():
        previous = json.loads(STORE.read_text())

    found: list[tuple[str, str]] = []
    for path in logs:
        text = path.read_text(errors="replace")
        found += re.findall(r"@@DOM@@(.*?)@@(.*?)@@ENDDOM@@", text, re.DOTALL)
    states: dict[str, str] = {}
    for key, markup in found:
        if markup.strip():
            states[key] = markup.strip()

    carried = sorted(set(previous) - set(states))
    if merge:
        for key in carried:
            states[key] = previous[key]
        if carried:
            print(
                "carried over from a previous run (NOT captured now, may be "
                f"stale): {', '.join(carried)}",
                file=sys.stderr,
            )
    elif carried:
        print(f"dropped (not visited this run): {', '.join(carried)}", file=sys.stderr)

    if not states:
        print("no DOM dumps in the log — is this a debug build?", file=sys.stderr)
        return 1

    STORE.write_text(json.dumps(states, indent=1, sort_keys=True))

    # The phone's screens in `LABELS` order, then whatever else it captured,
    # then the desktop's — so the two shells read as two blocks rather than
    # interleaving `chat` with `desktop-chat`.
    order = list(LABELS)
    phone = [k for k in states if not k.startswith(DESKTOP)]
    ordered = (
        [k for k in order if k in phone]
        + [k for k in sorted(phone) if k not in order]
        + sorted(k for k in states if k.startswith(DESKTOP))
    )

    def label_for(key: str) -> str:
        if not key.startswith(DESKTOP):
            return LABELS.get(key, key)
        rest = key[len(DESKTOP) :]
        return f"Desktop · {LABELS.get(rest, rest)}"

    cells = "\n".join(
        CELL.format(
            label=html.escape(label_for(key)),
            srcdoc=html.escape(
                f"<!doctype html><html lang=en><head>{head(key)}</head>"
                f"<body>{states[key]}</body></html>",
                quote=True,
            ),
            **FRAMES[key.startswith(DESKTOP)],
        )
        for key in ordered
    )
    GALLERY.write_text(PAGE.format(cells=cells))
    # Counted per shell rather than in one number, for the reason
    # `docs/audit.js` states about its own summary line: an unnamed count is
    # exactly how a coverage claim rots, and "49 states" over a store holding
    # two shells says nothing about either.
    desktop = len(ordered) - len(phone)
    print(
        f"{GALLERY.relative_to(ROOT)}: {len(ordered)} states "
        f"({len(phone)} phone, {desktop} desktop) "
        f"({', '.join(ordered)}); {len(found)} dumps read from "
        f"{len(logs)} log{'s' if len(logs) > 1 else ''}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
