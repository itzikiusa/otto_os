# Cross-repo dependencies: deep-dive and linking

A repository is not an island. Flows routinely cross into internal/platform
libraries and called services; a flow note that stops at the library boundary
("gets a DB connection") hides the part the reader actually needs. Two
obligations follow: **deep-dive** into the dependency's source, and **link**
the dependency's vault bundle.

## Resolve dependency sources locally — ALWAYS

Internal dependencies are usually checked out on this machine. For EVERY
internal dependency, attempt local resolution before treating it as opaque:

1. Take the repo name from the module path's last segment
   (`<host>/<org>/<repo>` → `<repo>`).
2. Look for that name as a sibling of the scanned repo, in the user's home
   directory (`~/<repo>`), and in obvious workspace roots.
3. Also honor explicit redirects: Go `go.mod` `replace` directives and
   `vendor/`; Node workspace/`file:` deps; equivalent mechanisms elsewhere.
4. Only when nothing resolves, document the contract you can see (signature,
   config, observed effects) and state that the source was unavailable.

Which dependencies count as internal: same org/host in the module path, or
clearly platform-owned libraries the repo's team also maintains. Stdlib and
generic third-party packages (web frameworks, drivers) are NOT deep-dive
targets.

## Deep-dive rule

When a flow step calls into a resolved internal dependency, open the
implementation and document what actually happens inside as sub-steps of THIS
flow — the reader must not need to reverse-engineer the library:

```markdown
2. Opens the tenant DB connection via
   [<dep-repo> multi-tenant SQL](../<dep-repo>/data.md)
   (`dao/player_dao.go:41`): resolves the tenant's DB endpoint through
   service discovery (`<dep-repo>:sql/multi_conn.go:88`), reuses the cached
   pool per tenant, falls back to a fresh dial on eviction.
```

- Cite dependency code as `<dep-repo>:relative/path:line` — the repo prefix
  distinguishes it from same-repo citations.
- Depth is flow-relevant only: what the call does for THIS flow (lookup,
  caching, retries, transactionality, side effects) — not a tour of the
  library.

## Linking rule — forward links are required

Vault bundles are folders named after the repo, so the target path is
deterministic BEFORE the dependency is ever scanned:

- Dependency already documented in the vault → link the most specific note
  (package / flow / data note), falling back to its `index.md`.
- Dependency NOT yet documented → still link `../<dep-repo>/index.md` and mark
  it "not scanned yet" in `dependencies.md`. The link is intentionally dangling
  and resolves the moment that repo is scanned — never omit it.

## dependencies.md (required deliverable)

One table, linked from `index.md`: import path → local source path (or
"unresolved") → what THIS repo uses it for (with a same-repo citation) →
vault bundle link (live or forward). Every internal dependency the code
actually uses appears exactly once.

## Reverse direction (infra/library scans)

When scanning a library, `consumers.md` links back to every documented app
bundle that imports it — check the vault's existing top-level bundles
(`otto_vault_list`) for importers. The app→infra links written by earlier app
scans already point here; the backlinks graph completes when both sides exist.
