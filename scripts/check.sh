#!/usr/bin/env bash
# scripts/check.sh — the everyday local gate: CI's checks, scoped to what you
# changed. Mirrors .github/workflows/ci.yml (fmt · clippy -D warnings · nextest ·
# doc-tests · npm run check · npm run test:unit) but only for:
#   * Rust: the workspace crates whose files changed (fmt + clippy) and those
#     crates plus every crate that depends on them (tests — an API change can
#     break a dependent), found via `cargo metadata`;
#   * UI: `npm run check` + `npm run test:unit` + `npm run build` + the bundle
#     budget (scripts/bundle-budget.mjs), only when ui/ changed;
#   * guard inputs: docs/contracts/** and the sidebar/sidePane/uiCommands UI
#     files also select the Rust crates whose tests read or mirror them.
# A change to a root build file (Cargo.toml, Cargo.lock, .cargo/, .config/,
# rust-toolchain) selects the whole workspace — exactly the CI gate.
#
# "Changed" = everything that differs from the merge-base with the base branch
# (committed on this branch, staged, unstaged and untracked).
#
# Usage: scripts/check.sh [options]
#   --base REF     compare against REF's merge-base (default: main)
#   --all          whole workspace + UI (what CI runs)
#   --check        rustfmt --check instead of reformatting changed files
#   --no-clippy    skip clippy          --no-test   skip nextest + doc-tests
#                  (--no-test also skips the nextest filter guard)
#   --no-ui        skip the UI gates    --dry-run   print the plan, run nothing
# Env: CARGO=<wrapper> to route cargo builds through a throttle wrapper;
#      NEXTEST_PROFILE (default: default) e.g. `ci` for CI's 4 test processes.
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
CARGO=${CARGO:-cargo}
BASE=main ALL=0 FMT_CHECK=0 CLIPPY=1 TEST=1 UI=1 DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --base) BASE=$2; shift ;;
    --all) ALL=1 ;;
    --check) FMT_CHECK=1 ;;
    --no-clippy) CLIPPY=0 ;;
    --no-test) TEST=0 ;;
    --no-ui) UI=0 ;;
    --dry-run) DRY=1 ;;
    -h | --help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "check.sh: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
run() {
  printf '+ %s\n' "$*"
  [ "$DRY" = 1 ] || "$@"
}

# ── What changed ──────────────────────────────────────────────────────────────
if [ "$ALL" = 0 ]; then
  mb=$(git merge-base HEAD "$BASE" 2>/dev/null) || {
    echo "check.sh: no merge-base with '$BASE' — pass --base REF or --all" >&2
    exit 2
  }
  CHANGED=$( { git diff --name-only "$mb"; git ls-files --others --exclude-standard; } | sort -u)
else
  CHANGED=""
fi

# Changed .rs files that still exist (deleted files have nothing to format).
RS_FILES=()
while IFS= read -r f; do
  [ -n "$f" ] && [ -f "$f" ] && RS_FILES+=("$f")
done < <(printf '%s\n' "$CHANGED" | grep -E '^crates/.*\.rs$' || true)

# Map changed paths → workspace packages (changed) and add reverse dependents
# (tested). Prints two lines: "changed: a b" / "tested: a b c" (or "all").
SCOPE=$(
  printf '%s\n' "$CHANGED" | ALL=$ALL python3 -c '
import json, os, subprocess, sys
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"]))
root = meta["workspace_root"] + "/"
members = set(meta["workspace_members"])
pkgs = [p for p in meta["packages"] if p["id"] in members]
dirs = {p["name"]: os.path.dirname(p["manifest_path"])[len(root):] + "/" for p in pkgs}
changed_paths = [l for l in sys.stdin.read().splitlines() if l]
build_files = ("Cargo.toml", "Cargo.lock", ".cargo/", ".config/", "rust-toolchain", "vendor/")
if os.environ["ALL"] == "1" or any(c.startswith(build_files) for c in changed_paths):
    print("changed: all"); print("tested: all"); sys.exit()
changed = sorted({n for n, d in dirs.items() for c in changed_paths if c.startswith(d)})
# Non-crate inputs that Rust guard tests read or mirror: the route/policy
# inventories and ws.md drift guard (otto-server) and the ui-commands catalog
# (include_str! in otto-mcp, whose paneKeys mirror sidebar.ts / sidePane.ts).
# A docs- or UI-only change must still run those tests (S12-311).
GUARD_INPUTS = {
    "docs/contracts/": ("otto-server", "otto-mcp"),
    "ui/src/lib/sidebar.ts": ("otto-mcp",),
    "ui/src/lib/sidePane.ts": ("otto-mcp",),
    "ui/src/lib/uiCommands": ("otto-mcp",),
}
guarded = {p for pre, ps in GUARD_INPUTS.items() for c in changed_paths if c.startswith(pre) for p in ps if p in dirs}
# reverse-dependency closure over workspace path dependencies
rdeps = {n: set() for n in dirs}
for p in pkgs:
    for d in p["dependencies"]:
        if d["name"] in rdeps and d.get("path"):
            rdeps[d["name"]].add(p["name"])
seen, todo = set(changed) | guarded, sorted(set(changed) | guarded)
while todo:
    for r in rdeps[todo.pop()]:
        if r not in seen:
            seen.add(r); todo.append(r)
print("changed:", *changed); print("tested:", *sorted(seen))
'
)
RUST_CHANGED=$(printf '%s\n' "$SCOPE" | sed -n 's/^changed: *//p')
RUST_TESTED=$(printf '%s\n' "$SCOPE" | sed -n 's/^tested: *//p')
UI_CHANGED=0
if [ "$ALL" = 1 ] || printf '%s\n' "$CHANGED" | grep -q '^ui/'; then UI_CHANGED=1; fi

pkg_args() { # "a b" → -p a -p b ; "all" → --workspace
  if [ "$1" = all ]; then echo --workspace; else for p in $1; do printf -- '-p %s ' "$p"; done; fi
}

if [ "$ALL" = 1 ]; then echo "base:          (all)"; else echo "base:          $BASE @ $(git rev-parse --short "$mb")"; fi
echo "rust changed:  ${RUST_CHANGED:-none}"
echo "rust tested:   ${RUST_TESTED:-none}"
echo "ui changed:    $( [ "$UI_CHANGED" = 1 ] && echo yes || echo no)"

# ── Rust ──────────────────────────────────────────────────────────────────────
if [ -n "$RUST_CHANGED" ] || [ -n "$RUST_TESTED" ]; then
  step "rustfmt"
  if [ "$RUST_CHANGED" = all ]; then
    if [ "$FMT_CHECK" = 1 ]; then run "$CARGO" fmt --all --check; else run "$CARGO" fmt --all; fi
  elif [ "${#RS_FILES[@]}" -gt 0 ]; then
    if [ "$FMT_CHECK" = 1 ]; then
      run rustfmt --edition 2021 --check "${RS_FILES[@]}"
    else
      run rustfmt --edition 2021 "${RS_FILES[@]}"
    fi
  else
    echo "(no changed .rs files)"
  fi

  if [ "$CLIPPY" = 1 ] && [ -n "$RUST_CHANGED" ]; then
    step "clippy (changed crates, all targets, -D warnings)"
    # shellcheck disable=SC2046
    run "$CARGO" clippy $(pkg_args "$RUST_CHANGED") --all-targets -- -D warnings
  fi

  if [ "$TEST" = 1 ]; then
    step "tests (changed crates + reverse dependents)"
    if cargo nextest --version >/dev/null 2>&1; then
      # shellcheck disable=SC2046
      run "$CARGO" nextest run $(pkg_args "$RUST_TESTED") --profile "${NEXTEST_PROFILE:-default}" --no-fail-fast
    else
      echo "(cargo-nextest not installed — falling back to cargo test; see AGENTS.md)"
      # shellcheck disable=SC2046
      run "$CARGO" test $(pkg_args "$RUST_TESTED") --tests --no-fail-fast
    fi
    # CI's separate doc-test gate. Only packages with a library have doc-tests.
    if [ "$RUST_TESTED" = all ]; then
      run "$CARGO" test --workspace --doc
    else
      LIBS=$(cargo metadata --format-version 1 --no-deps --offline | python3 -c '
import json, sys
want = set(sys.argv[1:])
for p in json.load(sys.stdin)["packages"]:
    if p["name"] in want and any("lib" in t["kind"] for t in p["targets"]):
        print(p["name"])
' $RUST_TESTED | xargs)
      # shellcheck disable=SC2046
      [ -z "$LIBS" ] || run "$CARGO" test $(pkg_args "$LIBS") --doc
    fi
    # CI's nextest override-filter guard (Rust job) whenever the config it
    # checks changed — every override disjunct must still match a test.
    if [ "$ALL" = 1 ] || printf '%s\n' "$CHANGED" | grep -q '^\.config/nextest\.toml$'; then
      if cargo nextest --version >/dev/null 2>&1; then
        run python3 scripts/check-nextest-filters.py
      else
        echo "(cargo-nextest not installed — skipping scripts/check-nextest-filters.py)"
      fi
    fi
  fi
fi

# God-module LOC ratchet (CI: Rust job) — cheap, so whenever Rust changed.
if [ -n "$RUST_CHANGED" ]; then
  step "rust LOC ratchet"
  run python3 scripts/loc-ratchet.py
fi

# ── UI ────────────────────────────────────────────────────────────────────────
if [ "$UI" = 1 ] && [ "$UI_CHANGED" = 1 ]; then
  step "ui: npm run check + test:unit + build + bundle budget"
  run npm --prefix ui run check
  run npm --prefix ui run test:unit
  # CI's UI job also builds and checks the gzip bundle budget; a deleted or
  # grown chunk fails there, so catch it here too (S12-303).
  run npm --prefix ui run build
  run node ui/scripts/bundle-budget.mjs
fi

if [ -z "$RUST_CHANGED" ] && [ "$UI_CHANGED" = 0 ]; then
  echo "nothing to check (no Rust or UI changes vs $BASE)"
fi
step "done"
