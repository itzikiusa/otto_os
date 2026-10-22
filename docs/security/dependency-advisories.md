# Dependency advisories

The advisories in Otto's dependency tree that are **not** fixed by an upgrade,
and why each one is acceptable for now. Every ignore entry anywhere in the repo
(CI's `cargo audit --ignore`, `deny.toml [advisories] ignore`, an npm override)
must have a row here with its reason. When an upstream release fixes one, take
the upgrade and delete its row.

Sources checked: GitHub Dependabot alerts (`gh api
repos/itzikiusa/otto_os/dependabot/alerts?state=open`, npm and cargo
ecosystems), `npm audit` in `ui/`, and the RustSec database for the root
`Cargo.lock` (CI's advisory `audit` job runs `cargo audit`).

## Last triage: 2026-10-22

Fixed by an upgrade in this pass:

| Advisory | Package | Change |
|----------|---------|--------|
| GHSA-9jrq-w75r-8gcw, GHSA-rq7h-c2jc-7f22, GHSA-vfpm-58rq-9qcg, GHSA-587h-7wmj-h5pm, GHSA-r825-w8mm-5pxw, GHSA-698v-4r3x-3vcv (Dependabot #56, #58, #60–#63) | `vite` (UI dev server) | 8.3.2 → 8.3.3 |
| GHSA-cg2g-xh64-v4q7, GHSA-mrv2-3pqr-j5jr, GHSA-c6gr-qvv4-5v3f (`npm audit`, critical) | `dompurify` (pinned through `overrides`, used by mermaid) | 3.4.16 → 3.4.17 |
| RUSTSEC-2026-0382 (distrusted intermediates accepted) | `rustls-webpki` | 0.103.15 → 0.103.16 |
| RUSTSEC-2026-0381, RUSTSEC-2026-0383 (distrusted emSign roots bundled) | `webpki-roots`, `webpki-root-certs` | 1.0.9 → 1.0.10 (`webpki-roots` 0.26.11 is already the fixed 0.26 line) |

## Open, no compatible fix

| Advisory | Package | Reaches us through | Why it stays |
|----------|---------|--------------------|--------------|
| RUSTSEC-2023-0071 (Marvin timing attack) | `rsa` 0.10.0-rc | `sqlx-mysql` (`caching_sha2_password` auth) | No fixed `rsa` release exists. The path only *encrypts* the password to the server's public key; the attack needs a party doing RSA *decryption*, which Otto never does. This is the one ignored advisory (`.github/workflows/ci.yml`, `cargo audit --ignore RUSTSEC-2023-0071`). |
| RUSTSEC-2024-0436 (unmaintained) | `paste` | `klickhouse` → `otto-dbviewer` | Informational, no vulnerability. A compile-time macro with no runtime surface; goes away when `klickhouse` drops it. Not ignored (cargo audit only warns). |
| RUSTSEC-2024-0449 (unmaintained) | `unsafe-libyaml` | `serde_yaml` 0.9 → `otto-k8s`, `otto-aws`, `otto-server`, `otto-swarm` | Informational. Replacing `serde_yaml` (itself archived) is a cross-crate migration to a maintained YAML crate, tracked separately. Not ignored. |
| RUSTSEC-2025-0134 (unmaintained) | `rustls-pemfile` | `ottod` (`main.rs`, network-listener TLS cert loading) | Informational. The fix is to read PEM through `rustls-pki-types`' `PemObject` and drop the crate; that is a code change in `ottod`, outside the dependency pass. Not ignored. |
| RUSTSEC-2026-0370, -0371, -0372 (unmaintained) | `zvariant`, `zbus_names`, `zvariant_derive` | `keyring` → `secret-service` → `zbus` → `otto-keychain` | Informational. Linux Secret Service backend only; the macOS app uses the Keychain backend and never runs this code. Goes away with a `keyring` release on a maintained `zbus`. Not ignored. |
