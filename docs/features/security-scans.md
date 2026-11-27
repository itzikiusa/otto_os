# IDE security scans

Otto runs Trivy against local named IDE projects. A project can span several
folders. They share one settings record, one scan history and one queue entry.

## Set up a project

Create or select a named project in the IDE. The security API uses its project
ID. Open Project Settings to see the folders that belong to it. The
[IDE security API](../contracts/api.md#ide-project-security-scans) accepts settings,
starts and cancels scans, and returns their findings.

By default Otto checks dependencies for known vulnerabilities, scans for
credential patterns and checks infrastructure files for misconfigurations.
Enable license checking in the project's settings. Each folder uses the same
root-relative excludes. Otto skips common build and dependency directories.

## Coverage

| IDE language | Trivy dependency coverage |
| --- | --- |
| Go | Go modules |
| Rust | Cargo lockfiles |
| JavaScript and TypeScript | npm, Yarn, pnpm and other supported lockfiles |
| Python | Supported pip, Poetry, Pipenv and uv dependency files |
| Java and Kotlin | Maven and Gradle dependency files that Trivy can resolve offline |
| C and C++ | Supported Conan lockfiles |
| .NET | Supported NuGet dependency files |
| Zig | No native advisory analysis for Zig package manifests |

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

## Measured resource use

A full Trivy 0.76.2 scan of the Otto worktree on Apple Silicon ran in 18.392
seconds with one worker, a sampled peak of 100.2% of one CPU core and 241 MiB
peak RSS. These are engine measurements. The acceptance test also downloaded
and unpacked the engine and database into temporary tool state.

```sh
CARGO_INCREMENTAL=0 cargo test -j 2 -p otto-security --lib live_trivy_on_otto_worktree -- --ignored --nocapture
```

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
