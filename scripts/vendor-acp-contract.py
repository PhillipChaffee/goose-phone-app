#!/usr/bin/env python3
"""Vendor goose's own declaration of its wire protocol into this repo.

`crates/goose-acp-client/tests/contract.rs` checks every method this app sends
against what goose says it accepts. That check has to run offline, in
milliseconds, on a machine that has never cloned goose — so the declaration is
vendored rather than read from a sibling checkout.

Two files come out, into `crates/goose-acp-client/tests/fixtures/`:

  acp-meta.json          goose's file, verbatim (~21 KB). It is the list of
                         methods and the request/response type name of each.

  acp-request-keys.json  derived. `acp-schema.json` is 246 KB of JSON Schema,
                         which is too much to carry in a phone app's
                         repository for the one question the contract test
                         asks it: which keys does this method's request
                         declare, and which are required. So the schema is
                         resolved down to that, here, by a script that is
                         checked in — a regenerable index rather than a
                         hand-trimmed blob that rots.

Both carry `_source`, the goose commit they were taken from, because a
vendored copy with no provenance is a copy nobody can tell is stale.

Usage:

    scripts/vendor-acp-contract.py [PATH_TO_GOOSE_CHECKOUT]

defaulting to ~/git/goose. Re-run it when goose releases; the diff is the
review.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
OUT = REPO / "crates" / "goose-acp-client" / "tests" / "fixtures"


def source_stamp(goose: pathlib.Path) -> str:
    """The goose commit these came from, or a note that it is unknown."""
    try:
        rev = subprocess.run(
            ["git", "-C", str(goose), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
        return rev.stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def request_keys(schema: dict, type_name: str) -> dict | None:
    """The declared request keys for one `$defs` entry.

    `None` for a method whose request type is not an object with properties —
    `ListSchedulesRequest_unstable` is `{"type": "object"}` and nothing else,
    which is goose saying "this method takes no parameters". The contract test
    reads that as "any key you send is one goose does not declare", which is
    exactly what it should mean.
    """
    entry = schema.get("$defs", {}).get(type_name)
    if entry is None:
        return None
    return {
        "keys": sorted(entry.get("properties", {})),
        "required": sorted(entry.get("required", [])),
    }


def main() -> int:
    goose = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "~/git/goose")
    goose = goose.expanduser()
    meta_path = goose / "crates" / "goose" / "acp-meta.json"
    schema_path = goose / "crates" / "goose" / "acp-schema.json"
    for path in (meta_path, schema_path):
        if not path.is_file():
            print(f"not found: {path}", file=sys.stderr)
            print(f"usage: {sys.argv[0]} [PATH_TO_GOOSE_CHECKOUT]", file=sys.stderr)
            return 1

    meta = json.loads(meta_path.read_text())
    schema = json.loads(schema_path.read_text())
    stamp = source_stamp(goose)

    OUT.mkdir(parents=True, exist_ok=True)
    meta["_source"] = stamp
    (OUT / "acp-meta.json").write_text(json.dumps(meta, indent=1) + "\n")

    index: dict[str, dict] = {}
    for entry in meta["methods"]:
        keys = request_keys(schema, entry["requestType"])
        if keys is not None:
            index[entry["method"]] = keys
    (OUT / "acp-request-keys.json").write_text(
        json.dumps({"_source": stamp, "methods": index}, indent=1) + "\n"
    )

    print(f"vendored {len(meta['methods'])} methods from goose {stamp[:12]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
