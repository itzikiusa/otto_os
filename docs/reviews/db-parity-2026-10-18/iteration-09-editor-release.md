# Iteration 09 — root L21–L24: package and release boundaries

Inspected vendor/sqlx-mysql/README-OTTO.md, preserved licenses, root Cargo patch,
standalone desktop workspace and packaging/deploy.sh. Vendor changes preserve
upstream provenance and document completion metadata, protocol status and
one-command LOCAL bytes grants. The daemon consumes this patch; the independent
Tauri shell does not acquire an unrelated SQLx dependency. Deployment refuses a
dirty source tree and validates a build receipt before installation. It must run
after the requested single PR is merged and main is synchronized.

| Lens | Score | Evidence |
|---|---:|---|
| L21 routing | 9.6 | MySQL ANSI_QUOTES edge reproduced; owner fixing result-boundary fidelity. |
| L22 dialog | 9.8 | Optional height remains isolated to NewSession. |
| L23 contracts | 9.7 | Explicit CH native JSON/Dynamic/Variant limitation, no universal parity claim. |
| L24 release | 9.2 | Current UI build/check/1996 units/4 browser tests pass; LOC ratchet passes 130844/130939. Rust aggregate gate, required CI and install pending. |

This pass does not certify release completion or convert pending checks into a
passing score. No destructive git operation or user-data cleanup is authorized
by this report.
