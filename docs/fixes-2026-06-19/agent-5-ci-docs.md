# Agent 5 — CI, docs (AGENTS/CLAUDE/RELEASE), Svelte warnings

Hardening pass on `/Users/itziklavon/otto_os`. Files owned exclusively by this
agent; no Rust sources, swarm files, or other agents' UI files were touched.

## T9a — CI pipeline — [x]

- **Created** `.github/workflows/ci.yml`
  - `rust` job (ubuntu-latest): `dtolnay/rust-toolchain@stable` (rustfmt +
    clippy) + `Swatinem/rust-cache@v2`, then `cargo fmt --all --check`,
    `cargo clippy --workspace --all-targets -- -D warnings`,
    `cargo test --workspace`.
  - `ui` job: `actions/setup-node@v4` (Node 20, npm cache keyed on
    `ui/package-lock.json`), `npm ci`, `npm run check`, `npm run build`.
  - `audit` job: installs `cargo-audit` and runs `cargo audit` as
    `continue-on-error: true` (advisory-only — a new RustSec advisory warns,
    doesn't block).
  - `concurrency` cancels superseded runs per ref.
  - Verified script names against `ui/package.json` (`check`, `build` exist) and
    that `ui/package-lock.json` is tracked (so `npm ci` works). The Tauri desktop
    app is a **separate, macOS-only workspace** (`apps/desktop/src-tauri` has its
    own `[workspace]` + `Cargo.lock`) and is intentionally excluded — root
    `cargo build --workspace` does not build it. No system deps needed: crates
    have no `build.rs`/`protoc` (protox is pure-Rust) and reqwest uses rustls.

## T9b — Release checklist — [x]

- **Created** `docs/RELEASE.md` — gated, ordered checklist:
  pre-flight (version sync across root `Cargo.toml`, desktop `Cargo.toml`,
  `tauri.conf.json`; contracts/types in sync) → verify gates (`cargo fmt --check`,
  `clippy -D warnings`, `cargo check`, `cargo test`, `npm run check`) → build
  (UI → `ui/dist`; `cargo build --release -p ottod`; sidecar copy to
  `binaries/ottod-$(rustc -vV | sed -n 's/host: //p')`; Tauri build via
  `npx --yes @tauri-apps/cli@^2 build`) → sign (`packaging/make-cert.sh`,
  `packaging/sign.sh`) → DMG (`packaging/dmg.sh`) → install & smoke test (ditto to
  `/Applications`, app self-deploys daemon, `curl …/health`) → publish.
  Commands verified against `README.md`, `packaging/` scripts, and the
  build/deploy memory (no Makefile; `cargo-tauri` not installed → npx; stable
  self-signed identity "Otto Dev Signing").

## T10 — AGENTS.md + CLAUDE.md — [x]

- **Created** `AGENTS.md` (repo root): what Otto is (1–2 lines), the
  architecture diagram, a full crate-map table (all 19 crates from `Cargo.toml`),
  the UI module areas (real list from `ui/src/modules/`), real build/test
  commands (cargo + npm, how `ottod` runs), conventions (contracts-first,
  append-only migrations, Keychain secrets), and an explicit **"Do NOT damage
  user work"** section (no deleting user data/DBs, no force-push/history rewrite,
  ask before irreversible/outward-facing actions, inspect before overwrite,
  report faithfully, don't weaken the loopback-only default).
- **Created** `CLAUDE.md` (repo root): thin bridge — points to AGENTS.md and uses
  the `@AGENTS.md` import so Claude Code picks it up.

## T12 — Svelte `npm run check` warnings — [x]

- `ui/src/modules/database/RedisKeyFilter.svelte`: `draft` was seeded with
  `$state(database.nodeFilter(node.id))`, reading the reactive `node` prop in the
  initializer (captured once). Now `let draft = $state('')` + an `$effect` that
  re-syncs `draft` from the derived `active` filter, so a reused component never
  shows a stale prefix.
- `ui/src/modules/database/TableDesigner.svelte`: `rows` was seeded with
  `$state(columns.map(...))`, capturing the reactive `columns` prop once.
  Extracted `rowsFromColumns()`; `rows` starts `[]` and is re-seeded via an
  `$effect` keyed on `table` (with a `seededFor` guard) so reopening on a
  different table doesn't carry over prior edits, while edits within one open are
  preserved.
- `ui/src/modules/database/ResultsGrid.svelte`: removed the unused `.tb-note` CSS
  selector (present only in `<style>`, never in markup).

**Verification (`cd ui && npm run check`):**

```
COMPLETED 480 FILES 0 ERRORS 0 WARNINGS 0 FILES_WITH_PROBLEMS
```

All three target warnings resolved; no new warnings introduced.
