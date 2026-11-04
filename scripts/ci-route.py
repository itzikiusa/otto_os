#!/usr/bin/env python3
"""Decide which CI jobs a change needs ("Route (what changed)").

Reads changed paths (one per line) on stdin and writes `key=true|false`
lines for $GITHUB_OUTPUT. Only pull requests are routed; every other event
(push to main, schedule, dispatch) passes `--all` and runs everything, so
a scoped-out job still runs on the merge commit.

Outputs:
  rust    Rust lint / nextest shards / doctests / cargo deny + audit / daemon
          perf budget. True unless EVERY path is UI-, docs- or desktop-only.
  code    The UI job, the test daemon build and the E2E smoke gate. False
          only when every path is documentation / marketing.
  plugin  The Team Performance plugin suites (examples/plugins/
          team-performance), which also read ui/src/lib/tokens.css.
  daemon  macOS daemon crates (macos.yml): anything outside apps/desktop/.
  desktop macOS Tauri app (macos.yml): apps/desktop/ or macos.yml itself.

Unknown paths count as Rust + code (fail open: a new top-level directory
runs the full gate until it is classified here).

    git diff --name-only BASE...HEAD | scripts/ci-route.py
    scripts/ci-route.py --all
"""

from __future__ import annotations

import re
import sys

# Paths that can never change what the Rust jobs build or test.
NOT_RUST = re.compile(
    r"^(?:"
    r"ui/|examples/|marketing/|apps/desktop/|dev/|packaging/"
    r"|docs/(?!contracts/)"  # docs/contracts/ui-commands.json is include_str!'d
    r"|\.github/(?!workflows/ci\.yml$)"
    r"|[^/]+\.md$"
    r")"
)
# Pure documentation / assets: nothing in the build or the tests reads them.
DOCS_ONLY = re.compile(r"^(?:marketing/|docs/(?!contracts/)|[^/]+\.md$|LICENSE)")
PLUGIN = re.compile(
    r"^(?:examples/plugins/team-performance/|ui/src/lib/tokens\.css$"
    r"|ui/package(?:-lock)?\.json$|\.github/workflows/ci\.yml$)"
)
DESKTOP = re.compile(r"^(?:apps/desktop/|\.github/workflows/macos\.yml$)")

KEYS = ("rust", "code", "plugin", "daemon", "desktop")


def route(paths: list[str]) -> dict[str, bool]:
    paths = [p.strip() for p in paths if p.strip()]
    if not paths:
        # An empty diff (e.g. a revert of a revert) still deserves a gate.
        return {k: True for k in KEYS}
    return {
        "rust": any(not NOT_RUST.match(p) for p in paths),
        "code": any(not DOCS_ONLY.match(p) for p in paths),
        "plugin": any(PLUGIN.match(p) for p in paths),
        "daemon": any(not p.startswith("apps/desktop/") for p in paths),
        "desktop": any(DESKTOP.match(p) for p in paths),
    }


def main(argv: list[str]) -> int:
    result = {k: True for k in KEYS} if "--all" in argv else route(sys.stdin.read().splitlines())
    for k in KEYS:
        print(f"{k}={'true' if result[k] else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
