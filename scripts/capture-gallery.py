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

    scripts/capture-gallery.py --only desktop- /tmp/desktop.log

is the third mode and the one to reach for when the change is confined to one
shell. It DECLARES A SCOPE: inside the prefix the run is authoritative exactly
as a full run is — a state matching it that the app no longer emits is dropped
and named — and outside it every key is carried over untouched, counted rather
than listed. See the note on `--only` in `main` for why that is not `--merge`
under another name.

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
# shared.css alone was a live bug, and the same one `docs/audit.js` documents at
# its own top ("Linking shared.css alone would rebuild every feature screen
# unstyled and then measure the result, which passes"). The audit fixed it for
# itself; this file, which is generated alongside it and read by a human rather
# than by a walk, never got the same fix — so every recipes, skills, scheduler,
# extensions and session-history frame in the gallery has been rendering
# against 149 lines of missing CSS. Enumerated rather than listed for the
# reason `src/css.rs` states: a feature brings a sheet of its own, and a list
# here would be a second place to remember.
FEATURES = sorted(p.name for p in (ROOT / "assets" / "features").glob("*.css"))

# And the sheets that are deliberately NOT in that directory, because the audit
# links the whole of it into phone frames (`src/css.rs` explains all of them).
# They belong to the desktop frames alone.
#
# ORDER IS NOT FREE: `src/app.rs` emits STYLES, then SHELL (`assets/desktop/`),
# then PLATFORM (macos.css) — and it carries the write-up of what getting that
# wrong did, since the two sides set `--chrome-h`/`--traffic-w` on the same
# `.app > .shell` selector at equal specificity and the later one wins. Linked
# the other way round the reservation is always zero and the gallery would show
# a nav toggle sitting on the macOS close button, which is a window nobody
# ships.
#
# AND THE SHELL HALF IS ITSELF ORDERED. `src/css.rs` `concat!`s
# `assets/desktop/`'s fifteen region files in filename order, so the sort below
# IS the cascade: the prefixes are zero-padded so that Python's `sorted`,
# `readdirSync().sort()` in `docs/audit.js` and the const list in `src/css.rs`
# all agree, and a sheet that lands in a different slot here is a different
# cascade from the one that ships. macos.css is appended after the sort, not
# swept up by it — PLATFORM comes last by `src/app.rs`'s order, and its name
# would sort it into the middle.
SHELL_REGIONS = sorted(p.name for p in (ROOT / "assets" / "desktop").glob("*.css"))
if not SHELL_REGIONS:
    # A glob that comes back empty is the quiet version of a missing sheet: the
    # desktop frames would still render, still fill the page and still look like
    # a gallery, with every one of them unstyled. Nothing downstream would say
    # so — `docs/audit.js` measures the app's own sheets and never opens this
    # file.
    print(
        "assets/desktop/ has no .css in it — every desktop frame would render "
        "with the whole shell sheet missing",
        file=sys.stderr,
    )
    raise SystemExit(1)
SHELL_SHEETS = [f"../assets/desktop/{f}" for f in SHELL_REGIONS] + [
    "../assets/platform/macos.css"
]


def head(key: str) -> str:
    sheets = ["../assets/shared.css"] + [f"../assets/features/{f}" for f in FEATURES]
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
# at; 1440x860 is `with_inner_size` at src/main.rs:108 — the window the desktop
# build actually opens, and the only source of truth for the number below. A
# desktop state in a phone bezel is a 1440pt layout folded into 402, which is
# not a state this app has.
#
# THE DESKTOP NUMBER HAS TO TRACK src/main.rs AND ONCE DID NOT. It stayed at
# the 1180x820 the window opened at before the re-scale raised it, so every
# visual review of a desktop state after that happened in a window 260pt
# narrower than the product while `docs/audit.js` went on measuring the real
# 1440x860. Nothing catches a drift like that on its own: a too-narrow frame
# renders a perfectly legal layout, just not the shipped one, and the reviewer
# sees a page that looks fine. Naming the line it comes from is the whole
# defence, so keep the citation with the number.
FRAMES = {
    False: {"w": 402, "h": 874, "radius": 44},
    # macOS rounds a window's own corners at about 10pt, and there is no bezel:
    # the chrome is inside the app on this shell (`src/main.rs` hides the
    # titlebar), so what the frame has to show is the app's whole surface.
    True: {"w": 1440, "h": 860, "radius": 10},
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
  feature sheet; a <strong>desktop</strong> frame is the 1440×860 window
  <code>src/main.rs</code> opens (its <code>with_inner_size</code>, and
  the size <code>docs/audit.js</code> takes as its reference cell), and links
  every region sheet in <code>../assets/desktop/</code>, in filename order,
  then <code>../assets/platform/macos.css</code> — the order
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


def parse_args(argv: list[str]) -> tuple[list[str], bool, str | None] | None:
    """`(log paths, --merge, --only prefix)`, or `None` after printing why not.

    Hand-rolled rather than `argparse` because the positional half is "every
    remaining argument is a log" and the flag half is two flags; what argparse
    would add here is a dependency on its opinion about `--only=` versus
    `--only `, both of which the loop below takes.
    """
    args: list[str] = []
    merge = False
    only: str | None = None
    pending = list(argv)
    while pending:
        arg = pending.pop(0)
        if arg == "--merge":
            merge = True
        elif arg == "--only":
            if not pending:
                print("--only takes a key prefix, e.g. --only desktop-", file=sys.stderr)
                return None
            only = pending.pop(0)
        elif arg.startswith("--only="):
            only = arg.split("=", 1)[1]
        else:
            args.append(arg)

    # Both at once is a contradiction rather than a combination: --merge keeps
    # every key it did not capture and --only exists to DROP the ones inside
    # its scope. Answering "which wins" would give the freshness guarantee a
    # spelling under which it quietly does not hold.
    if merge and only is not None:
        print(
            "--merge and --only contradict each other: --merge keeps every key "
            "it did not capture, --only drops the ones inside its prefix",
            file=sys.stderr,
        )
        return None
    # An empty prefix matches every key, so `--only ''` is a plain run wearing a
    # flag that says a scope was declared. Rejected rather than accepted as a
    # no-op, because the report it prints would claim a scope it does not have.
    if only == "":
        print(
            "--only needs a non-empty prefix; an empty one matches every key, "
            "which is what a run with no --only already does",
            file=sys.stderr,
        )
        return None
    return args, merge, only


def main() -> int:
    parsed = parse_args(sys.argv[1:])
    if parsed is None:
        return 2
    args, merge, only = parsed
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

    # THE SCOPE, checked before anything is carried. A run that says
    # `--only desktop-` and hands over a log full of phone dumps has not
    # narrowed a capture, it has mistyped one — and the shape of that mistake
    # is the reason this is an error and not a warning: a prefix that matches
    # nothing (`--only destkop-`) would otherwise carry the WHOLE previous
    # store over, drop nothing, and print a success line, which is the silent
    # staleness this flag was added to prevent wearing the flag that prevents
    # it.
    if only is not None:
        if outside := sorted(k for k in states if not k.startswith(only)):
            print(
                f"--only {only} claims this run is authoritative for that prefix, "
                f"but the log holds {len(outside)} state(s) outside it: "
                f"{', '.join(outside)}. Drop --only, or fix the prefix",
                file=sys.stderr,
            )
            return 1
        if not states:
            print(
                f"--only {only} captured nothing — no dump in the log has that "
                "prefix, so this run would carry the whole store over unchanged "
                "and call it a capture",
                file=sys.stderr,
            )
            return 1

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
    elif only is not None:
        # HOW THIS DIFFERS FROM --merge, which is the whole question about it.
        #
        # --merge carries over every key the run did not visit, so a key that
        # the app has STOPPED emitting survives; that is why it names all of
        # them, and why the block above calls it "not the answer" for a routine
        # capture. --only carries over exactly the keys the run never claimed:
        # the operator states a scope up front, the scope is checked against
        # the log above, and inside it this run is as authoritative as a full
        # one — `dropped` below is the same drop a plain run makes, on the same
        # evidence. So the carried set is by construction the half of the store
        # this capture said nothing about, which is why it is counted rather
        # than listed: a stale key cannot hide in it that could not equally
        # hide in a store nobody ran the script over at all.
        #
        # What it buys is that the two shells stop being one indivisible
        # capture. A desktop-only change needs a desktop window and an
        # operator; without this it also needs a booted simulator and a phone
        # driven through 49 screens, and the honest way to avoid that was
        # --merge, which gives up the freshness guarantee for the desktop half
        # as well as the phone's.
        kept = [k for k in carried if not k.startswith(only)]
        dropped = [k for k in carried if k.startswith(only)]
        for key in kept:
            states[key] = previous[key]
        print(
            f"--only {only}: authoritative for {len(states) - len(kept)} captured "
            f"state(s); carried {len(kept)} outside that prefix over unchanged",
            file=sys.stderr,
        )
        if dropped:
            print(
                f"dropped (matches {only}, not visited this run): "
                f"{', '.join(dropped)}",
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
