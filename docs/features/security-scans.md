# IDE security scans

Otto runs Trivy, Opengrep, and (opt-in) Nuclei against local named IDE projects. A project can span several
folders. They share one settings record, one scan history and one queue entry.

## Scan a project

Select a named local project in the IDE and open **Security** from the toolbar.
Select **Scan now**, or open **Settings** to choose checks, excludes and resource
limits. The IDE project switcher controls which folders the scan includes.

Enter a five-field cron and an IANA timezone to schedule scans. Select
**Preview next runs** to see the next five occurrences. Clear the cron field
and save to disable it. Otto's scheduled-task supervisor serves the latest
missed occurrence when it wakes after sleep.
For legacy projects without an owner, scheduled scans run as the person who
saved the schedule. Re-save any older schedule on an ownerless project to
activate it under your account.

Choose a completed scan to review findings by severity and location. Open
a finding's file, suppress it with a reason, or restore it from **Show
suppressed**. Select **Set baseline** to keep a completed scan as a reference.
Each finding shows whether it existed in the preceding scan and whether it
appears in the baseline. The latest unsuppressed file findings also appear in
Problems and the editor gutter. **Ask an agent to fix** opens a draft for
review in the IDE agent composer.

Use **Import SARIF** and **Export SARIF** to exchange SARIF 2.1.0 results.
Imports use project-relative file locations. The [IDE security API](../contracts/api.md#ide-project-security-scans)
describes the settings and scan routes.

## Audit a running HTTP service (passive and opt-in active DAST)

For a saved local project run configuration, set `OTTO_SECURITY_URL` in its
environment overrides to the exact HTTP or HTTPS URL it serves. Start that
configuration. In **Security → Settings → HTTP audit & DAST**, select the
configuration and review the destination before confirming it. Public
destinations require HTTPS and an explicit public-target confirmation.

**Run passive audit** (the default) sends one HEAD request without credentials,
cookies, a query string, or a body. The audit stays at the confirmed URL and
checks the response's browser security headers and cookie attributes. The audit
stores rule descriptions and fix hints without saving cookie values. It uses a
ten-second deadline and the project scan queue.
Choose a URL that accepts unauthenticated HEAD requests and returns a success
status. If the URL redirects, confirm the final URL instead.

**Run active scan (Nuclei)…** (available from the **⋯** menu and the HTTP audit
settings panel once a target is confirmed and running) is an explicit per-scan
opt-in. Before starting, Otto confirms what active scanning does (sending test
HTTP requests to the confirmed target). On macOS, Otto installs the latest
ProjectDiscovery Nuclei release into `<data_dir>/toolchains/security-nuclei/`,
verifies its archive against GitHub's published SHA-256 digest, and has Nuclei
fetch templates into that engine directory (never your home directory). Active
scans run under the same resource governor (CPU, RSS, worker count, wall-time
limit, and whole-process-group cancellation) and macOS Seatbelt with outbound
traffic confined to an `otto-netguard`-pinned loopback proxy for the single
confirmed host and port. Dev-safe defaults apply: HTTP exposure templates only
(`http/exposures/configs`, `http/exposures/files`, `http/exposures/apis`, and
`http/exposures/logs`), severities `low` through `critical`,
DoS/fuzzing/intrusive/brute-force tags excluded, Interactsh disabled,
rate-limited to 20 req/s, and raw matched responses, extracted tokens, and URL
query strings stripped or redacted before persistence.

Enable **Include passive audit in scheduled scans** when confirming the URL to
allow future scheduled passive audits while the same configuration is running.
Scheduled active checks (**Include active Nuclei checks in scheduled scans**)
are off by default and require explicit opt-in alongside scheduled passive
audits. A changed URL, changed live configuration, or revoked confirmation
prevents the previous approval from being used. Scheduled source scans can
proceed when the service is unavailable.

## Coverage

| IDE language | Trivy dependency coverage | Opengrep static analysis |
| --- | --- | --- |
| Go | Go modules | Community security rules |
| Rust | Cargo lockfiles | Community security rules |
| JavaScript and TypeScript | npm, Yarn, pnpm and other supported lockfiles | Community security rules |
| Python | Supported pip, Poetry, Pipenv and uv dependency files | Community security rules |
| Java and Kotlin | Maven and Gradle dependency files that Trivy can resolve offline | Community security rules for both languages |
| C and C++ | Supported Conan lockfiles | Community C rules and a focused C++ unsafe-input check |
| .NET | Supported NuGet dependency files | Community C# rules |
| Zig | Package manifests outside Trivy coverage | Focused runtime-safety check |

Secret and infrastructure checks can report findings in any project. Trivy
also provides the optional license check. The existing IDE dependency
inspector gives you the manifest-level package and update view.

## Engine and resource limits

On macOS, Otto fetches the latest Trivy release into the IDE toolchain
directory and verifies the archive against the release asset's SHA-256 digest.
Otto checks for a release once per 24 hours. It downloads the public
vulnerability database separately from the source scan, verifies its OCI
layer digest, and caches the expanded database. The database can use about
2 GiB of disk, with additional space needed during a refresh. Temporary
downloads and raw reports are discarded after the run. The scan runs with
network access denied, reads the project and its required tool files, and
writes within the scan cache and output directories. It discards matched
secret text and snippets before saving findings.

Across projects Otto runs one scan at a time and waits for machine load and
memory pressure to fall before starting and between scan steps. By default
the engine runs at background priority with one worker, a sampled 1 GiB RSS limit and a
180-second timeout per folder. Change the worker count, memory limit or timeout
in project settings. Cancelling a scan stops the scanner's process group.

Remote SSH project scans and Java archive scanning are outside the current
coverage. Java archive scanning requires Trivy's separate Java index download.

## Incremental scans and measured resource use

Otto verifies Opengrep against the upstream release digest, refreshes its
community rules daily, and runs it offline. The selection favors high-confidence
community security rules. Source analysis covers files up to 256 KiB with a
five-second per-rule/file time budget. Remote rule options and cross-file mode
are disabled.

The result cache uses file contents, engine versions, rule and database
versions, and scan settings. For small Opengrep edits Otto scans changed files
and drops results from deleted files. Trivy reuses results for unchanged
content and reruns a full scan after changes. Broad edits, rule and ignore-file
changes, and a missing snapshot trigger full scans. Links to regular files
already included in the snapshot reuse those files’ digests. A changed or
deleted link triggers a full Opengrep scan when the project has an ignore file.
Links to other targets, unreadable files, and scopes beyond 100,000 files or
512 MiB bypass result reuse.

Measured on the Otto worktree on Apple Silicon with one engine worker. CPU
percentages are sampled relative to one core. The total includes cache lookup
and, for the first run, engine and rules or database preparation.

| Engine and mode | Engine wall time | Total wall time | Peak CPU | Peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Opengrep 2.1.0, full | 23.082 s | 42.762 s | 99.9% | 186 MiB |
| Opengrep 2.1.0, one-file incremental | 0.683 s | 2.205 s | 96.0% | 94 MiB |
| Opengrep 2.1.0, unchanged cache | 0 s | 0.820 s | — | — |
| Trivy 0.76.2, full | 14.442 s | 64.421 s | 99.7% | 211 MiB |
| Trivy 0.76.2, unchanged cache | 0 s | 0.939 s | — | — |
| Otto HTTP audit, one loopback request | 0.002 s | — | — | — |

Each HTTP audit reads the current response. The loopback fixture measured
request and header analysis time. Per-audit CPU and RSS were unmeasured
because the audit runs inside the daemon.

```sh
CARGO_INCREMENTAL=0 cargo test -j 2 -p otto-security --lib live_incremental_engines_on_otto_worktree -- --ignored --nocapture
```

[Opengrep releases](https://github.com/opengrep/opengrep/releases) · [Community rules](https://semgrep.dev/p/default)

## Troubleshooting

- A queued scan may be waiting for another project or for the Mac to become idle.
- If the first launch reports a download error, check access to GitHub releases
  and GitHub Container Registry. Once an engine and database are installed,
  Otto can use them when an update service is unavailable.
- If the scanner reaches a resource limit, narrow the project folder set or
  add excludes for generated or vendored content.
- A finding may identify a file without a line number when the engine reports
  only the dependency or artifact location.

[Trivy filesystem reference](https://trivy.dev/docs/latest/guide/references/configuration/cli/trivy_filesystem/)
