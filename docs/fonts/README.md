# Fonts the audit measures with

**Nothing here ships in the app.** These five files exist so that
`docs/audit.js` lays text out in the same glyphs on every machine. Before they
existed, `-apple-system` / `ui-serif` / `ui-monospace` resolved to `.SF NS`,
Charter and Menlo on a Mac and to Liberation Sans, Serif and Mono on
`ubuntu-latest`, and the same commit came out Clean here and 24 findings in
CI. The long version is the font block at the top of `docs/audit.js`; the
design-level version is *The audit brings its own fonts* at the end of
`docs/design.md`.

They are stand-ins for San Francisco, New York and SF Mono, which are Apple's
and cannot be vendored. `node docs/audit.js fonts` measures how good the
stand-in is, against the real files in `/System/Library/Fonts`, over every
string this app puts on screen — macOS only, since only macOS has them.

| file | face | stands for | licence |
| --- | --- | --- | --- |
| `inter-latin-standard-normal.woff2` | Inter | San Francisco (`--font-sans`) | OFL 1.1 |
| `literata-latin-standard-normal.woff2` | Literata | New York (`--font-serif`) | OFL 1.1 |
| `jetbrains-mono-latin-wght-normal.woff2` | JetBrains Mono | SF Mono (`--font-mono`) | OFL 1.1 |
| `noto-sans-math-marks.woff2` | Noto Sans Math, three glyphs | nothing — see below | OFL 1.1 |
| `noto-sans-symbols2-keys.woff2` | Noto Sans Symbols 2, two glyphs | nothing — see below | OFL 1.1 |

Licence texts are beside them, as OFL 1.1 requires.

## Where they came from

The two proportional faces are the `-standard-` builds, not the `-wght-` ones
that are a third of the size, because those carry the `opsz` axis. San
Francisco tightens as it grows; a face that does not runs 15% wide at AX5 and
reports spills no iPhone has.

```sh
base=https://cdn.jsdelivr.net/npm
curl -L -o inter-latin-standard-normal.woff2 \
  $base/@fontsource-variable/inter@5.3.0/files/inter-latin-standard-normal.woff2
curl -L -o literata-latin-standard-normal.woff2 \
  $base/@fontsource-variable/literata@5.3.0/files/literata-latin-standard-normal.woff2
curl -L -o jetbrains-mono-latin-wght-normal.woff2 \
  $base/@fontsource-variable/jetbrains-mono@5.3.0/files/jetbrains-mono-latin-wght-normal.woff2
```

Committed rather than pulled through `npm`, because `.github/workflows/ci.yml`
runs `npm install` against caret ranges with no lockfile: a font that can move
under the gate is a gate that can change its mind about a commit it already
passed.

```
2c295d99e26dcf357d4d01bcf270fd6924b600c9a13dd8c363ef114f4c6976fa  inter-latin-standard-normal.woff2
29de894c768689feef6ab4ef274a9a16d19bfc5b0c3cbfcdac80ef220816210c  literata-latin-standard-normal.woff2
18be452724bfdc236c074ca94a249a7f41a86752c7d04ab258ce9ed5651f6a7e  jetbrains-mono-latin-wght-normal.woff2
88601da0cf3262c1cff28df0459bdf606f7e7e535b8970e3bd46faf3bbe98122  noto-sans-math-marks.woff2
df10042bb34dfcc527ac7b2ad063d07da690bd53f0a20b5facb10f5dcc1b51a3  noto-sans-symbols2-keys.woff2
```

## The two leftover files

The three above are Latin subsets, and this app puts five characters outside
that on screen. `⋯` (U+22EF) is the "N unchanged lines" marker on the review
screen; `⌘` (U+2318) and `⌥` (U+2325) are the inspector's keyboard legend; and
`→` (U+2192) and `✓` (U+2713) are the inspector's branch arrow and its reviewed
tick, which only reached a captured state once the Code plane was driven
connected. Measured: none of Inter, Literata, JetBrains Mono, Source Serif 4,
Noto Serif, Charis SIL or Liberation has `⋯` even in its full build, so
shipping bigger files would not have helped. Left alone each is a glyph
answered by the host — PingFang SC here, something else on a runner — which is
the whole problem in miniature, so they are named as the next families after
each of the three and the run fails if anything reaches past them.

Under 2KB for both: Google Fonts' `css2` API will cut a face down to a given
string. No one free family has all five — Noto Sans Math has `⋯`, `→` and `✓`
and neither key; Noto Sans Symbols 2 has both keys and `✓` but not `→`.

```sh
ua='Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
  AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36'
curl -H "User-Agent: $ua" \
  'https://fonts.googleapis.com/css2?family=Noto+Sans+Math&text=%E2%8B%AF%E2%9C%93%E2%86%92'
curl -H "User-Agent: $ua" \
  'https://fonts.googleapis.com/css2?family=Noto+Sans+Symbols+2&text=%E2%8C%98%E2%8C%A5'
# then fetch the src: url() each answers with
```

**Check the file, not the CSS.** `text=` is answered with a `unicode-range`
listing the codepoints you *asked for*, whether or not the family has them: a
Symbols 2 subset requested with `→` comes back declaring `U+2192` and still
renders it out of the host's font. The question is only answerable by
rendering — Chrome DevTools' `CSS.getPlatformFontsForNode` reports
`isCustomFont` per family, which is the same call `docs/audit.js` makes.

If `node docs/audit.js both` ever stops with *"the measurement faces do not
cover this app's text"*, a new character has appeared on a screen. Regenerate
with the new string in `text=`; do not silence the check.
