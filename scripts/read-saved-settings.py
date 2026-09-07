#!/usr/bin/env python3
"""Print named fields of the app's SAVED settings file, and nothing else.

Why this exists: since #220 `settings` is fs-backed, it is loaded before
`dev_seed!` can apply, and it WINS. So `scripts/serve-real.sh` can print a host
it does not actually get, which is issue #279. The launcher calls this to
compare the two and say so.

THE FILE IS NOT TEXT, which is the thing everyone gets wrong about it. Issue
#279 says "have it read the settings file" as if a grep would do. Three layers
sit between the disk and a URL, all of them dioxus-sdk-storage's choice rather
than this app's (`serde_to_string`, dioxus-sdk-storage 0.7.0 `src/lib.rs:551`):

    hex ASCII  ->  zlib stream  ->  CBOR map  ->  {"server_url": "https://..."}

Measured on a real file: 230 bytes on disk, 115 compressed, 128 of CBOR. A
`grep server_url` over it finds nothing, and a launcher that concluded from
that silence that there was no saved value would report the exact opposite of
the truth.

ONLY THE KEYS IN `PRINTABLE` ARE EVER EMITTED, and that is the whole of
"paths only, never values". The file cannot hold a credential today —
`Settings::secret_key` and `Settings::code_password` are
`#[serde(skip_serializing)]` and `the_saved_settings_file_holds_no_credential`
holds the serializer to it — but this reader is one attribute away from being
the thing that prints one, so it refuses to dump the map. A field added to
`Settings` is invisible here until someone adds it below on purpose.

Usage:  read-saved-settings.py <file> <key>...
Output: one `key<TAB>value` line per requested key that is present and is text.
Exits:  0 decoded (even if it printed nothing), 3 no such file,
        4 present but not decodable in the shape above.
"""

import sys
import zlib

# The allowlist. Hosts and a path: the three fields `serve-real.sh` seeds and
# can therefore be contradicted about. `fingerprint` is deliberately absent —
# it is not a secret, but the launcher reports it by length and never by value,
# and this file should not be the one place that habit breaks.
PRINTABLE = ("server_url", "code_server_url", "working_dir")


def read_item(b, i):
    """Read one CBOR item at `b[i:]`. Returns `(value, next_index)`.

    Enough of RFC 8949 to walk a serde struct and skip anything unexpected
    without desyncing: every major type consumes its argument correctly, so a
    field that is one day a number or a list costs a `None`, not a wrong
    answer for the field after it.
    """
    init = b[i]
    i += 1
    major, ai = init >> 5, init & 0x1F
    if ai < 24:
        arg = ai
    elif ai == 24:
        arg = b[i]
        i += 1
    elif ai == 25:
        arg = int.from_bytes(b[i : i + 2], "big")
        i += 2
    elif ai == 26:
        arg = int.from_bytes(b[i : i + 4], "big")
        i += 4
    elif ai == 27:
        arg = int.from_bytes(b[i : i + 8], "big")
        i += 8
    else:
        # 28-30 are reserved; 31 is indefinite length, which ciborium does not
        # emit for a serde struct. Either means this is not the file we think.
        raise ValueError(f"unsupported CBOR additional information {ai}")

    if major == 0:
        return arg, i
    if major == 1:
        return -1 - arg, i
    if major == 2:
        return b[i : i + arg], i + arg
    if major == 3:
        return b[i : i + arg].decode("utf-8"), i + arg
    if major == 4:
        out = []
        for _ in range(arg):
            v, i = read_item(b, i)
            out.append(v)
        return out, i
    if major == 5:
        out = {}
        for _ in range(arg):
            k, i = read_item(b, i)
            v, i = read_item(b, i)
            if isinstance(k, str):
                out[k] = v
        return out, i
    if major == 6:
        return read_item(b, i)  # a tag; the value is the item it wraps
    return None, i  # major 7: simple values and floats, argument already eaten


def main(argv):
    if len(argv) < 3:
        print("usage: read-saved-settings.py <file> <key>...", file=sys.stderr)
        return 2
    path, wanted = argv[1], argv[2:]

    try:
        with open(path, encoding="ascii") as fh:
            hexed = fh.read().strip()
    except OSError:
        return 3

    try:
        fields, _ = read_item(zlib.decompress(bytes.fromhex(hexed)), 0)
    except (ValueError, IndexError, zlib.error, UnicodeDecodeError):
        return 4
    if not isinstance(fields, dict):
        return 4

    for key in wanted:
        if key not in PRINTABLE:
            print(f"refusing to print {key!r}: not in the allowlist", file=sys.stderr)
            return 2
        value = fields.get(key)
        if isinstance(value, str):
            print(f"{key}\t{value}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
