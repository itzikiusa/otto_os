# IDE security scans

Otto runs Trivy and Opengrep against local named IDE projects. A project can span several
folders. They share one settings record, one scan history and one queue entry.

## Set up a project

Create or select a named project in the IDE. The security API uses its project
ID. Open Project Settings to see the folders that belong to it. The
[IDE security API](../contracts/api.md#ide-project-security-scans) accepts settings,
starts and cancels scans, and returns their findings.

By default Otto checks dependencies for known vulnerabilities, scans for
credential patterns, checks infrastructure files for misconfigurations and runs static security analysis.
Enable license checking in the project's settings. Each folder uses the same
root-relative excludes. Otto skips common build and dependency directories.

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
memory pressure to fall before starting. By default the engine runs at
background priority with one worker, a sampled 1 GiB RSS limit and a
180-second timeout per folder. Change the worker count, memory limit or timeout
in project settings. Cancelling a scan stops the scanner's process group.

Remote SSH project scans and Java archive scanning are outside the current
coverage. Java archive scanning requires Trivy's separate Java index download.

## Incremental scans and measured resource use

Otto verifies Opengrep against the upstream release digest, refreshes its
community rules daily, and runs it offline. The selection favors high-confidence
community security rules. Source analysis covers files up to 256 KiB with a
five-second per-rule/file time budget. Remote rule options and cross-file mode
are disabled. Project users can suppress findings through the Security view
once available.

The result cache uses file contents, engine versions, rule and database
versions, and scan settings. For small Opengrep edits Otto scans changed files
and drops results from deleted files. Trivy reuses results for unchanged
content and reruns a full scan after changes. Broad edits, rule and ignore-file
changes, and a missing snapshot trigger full scans. Symlinks, unreadable files,
and scopes beyond 100,000 files or 512 MiB bypass result reuse.

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
