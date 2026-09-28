# CodeQL extraction-cache experiment

Status, 2026-09-27: local proposal only. [The workflow template](codeql-performance.workflow.yml) is deliberately outside `.github/workflows/`. Default setup remains enabled; no settings, runner purchases, workflow dispatches or result uploads were made. The template is dispatch-only and is **not a production scanning replacement** until the activation steps below are completed.

## Baseline and hypothesis

[Run 36219378962](https://github.com/itzikiusa/otto_os/actions/runs/36219378962), main `4ad7475d`, took 20m34 wall time. The Rust job used CodeQL 2.27.1, `build-mode: none`, four threads and 14,575 MB. Approximate phase times from its log:

| Rust phase | Baseline | Cache-off / cold / warm experiment |
| --- | ---: | --- |
| Extraction | 7m32 | Pending |
| Database finalization | 1m17 | Pending |
| Query execution | 10m21 | Pending |
| Restore/save + total job | See run log | Pending |

The other five jobs finish in roughly 1–3 minutes. The actual six-language matrix is Actions, Go, Java/Kotlin, JavaScript/TypeScript, Python and Rust; API aliases do not represent additional jobs. Rust already avoids building the full application, so `build-mode: none` is preserved behavior, not a new optimization. Java/Kotlin currently uses `none`, Go uses `autobuild`, and interpreted languages use `none`.

The supported Rust extractor option `cargo_target_dir` permits reuse of Cargo artifacts from its internal commands. It normally uses scratch storage. Both [GitHub's extractor declaration](https://github.com/github/codeql/blob/main/rust/codeql-extractor.yml) and the baseline run's `resolve languages --format=betterjson` output declare it. The [extractor implementation](https://github.com/github/codeql/blob/main/rust/extractor/src/config.rs) reads `CODEQL_EXTRACTOR_RUST_OPTION_CARGO_TARGET_DIR`; the template sets this only for cache-enabled Rust extraction. It leaves features, cfg settings and targets at their existing defaults. This is a hypothesis about extraction savings, not a query-result cache or a promise that the 10-minute query phase becomes faster.

The template checks the installed bundle's option metadata before proceeding, following GitHub's [extractor-options discovery interface](https://docs.github.com/en/code-security/reference/code-scanning/codeql/codeql-cli/extractor-options). The Action's generic `dependency-caching` switch is not used to claim Rust caching: GitHub currently documents that integration for Java, Go and C#, not Rust. See [compiled languages and dependency caching](https://docs.github.com/en/code-security/concepts/code-scanning/codeql/codeql-for-compiled-languages).

## What the template preserves

- All six language jobs, their observed build modes, repository-root extraction and the default security suite. There are no file, test, feature, cfg or query exclusions. Omitting query customization keeps default queries; [init's inputs](https://github.com/github/codeql-action/blob/v4/init/action.yml) also expose the resolved CLI version used in cache keys. The fixed 2.27.1 bundle makes the initial comparison reproducible; remove the temporary pin or update it deliberately after the experiment so security queries continue to evolve.
- Standard `ubuntu-latest` runners and normal resource selection. No larger paid runner or guessed RAM allocation. Record the actual image, CPU, thread and RAM values on each run; reject comparisons across different resources.
- Read-only repository contents and job-scoped `security-events: write` for [analysis uploads](https://github.com/github/codeql-action/blob/v4/analyze/action.yml). Checkout does not persist credentials. This public repository needs neither repository-write nor package-write permissions; private-repository activation may need `actions: read` and `packages: read` according to its configuration.
- [Concurrency cancellation](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency) is scoped to this workflow's ref. It avoids spending minutes on superseded runs; it does not make a completed individual scan faster. Run benchmark trials sequentially so cancellation does not invalidate a trial.

Java `none` retains the measured configuration; it does not analyze Kotlin. If Kotlin is introduced, review that language's build mode before claiming coverage parity. This proposal adds no further reduction of coverage.

## Cache lifetime and invalidation

The only custom cached directory is `$RUNNER_TEMP/codeql-rust-cargo-target`, separate from ordinary CI `target/`, source files, credentials and CodeQL databases. Every run still extracts source and executes the queries. The namespace includes schema version, OS/architecture, resolved CodeQL bundle version (including its extractor), installed Rust/Cargo/toolchain fingerprints, runner image version, all manifests/lockfiles/toolchain files/Cargo configs/build scripts and the workflow contents. A commit-SHA suffix permits refreshed snapshots; the restore prefix drops only that suffix. Cargo still validates changed source inputs. There is no fallback across dependency, toolchain, extractor or configuration changes.

Only successful default-branch runs save new snapshots; PRs restore without saving. Exact-key hits are immutable. Changing the `v1` namespace forces a fresh experiment without deleting anyone's cache. [GitHub's cache rules](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching) still apply: branch scoping, cache quotas and eviction can produce misses. Measure archive size and transfer time; a large archive can cost more than the work it saves. Do not store secrets in caches. This directory can be read by eligible PR runs.

## Activation boundary — requires separate approval

GitHub [disables competing CodeQL workflows and blocks their uploads while default setup is enabled](https://docs.github.com/en/code-security/reference/code-scanning/troubleshoot-analysis-errors/two-codeql-workflows). Merely adding this template as an active workflow is not a supported side-by-side benchmark. Keep default setup until a coordinated replacement is ready.

1. Review the complete candidate locally, copying it to the proposed `.github/workflows/codeql.yml` only in a future approved change. Before production activation, add these events alongside `workflow_dispatch` (include **every** currently protected branch in `push.branches`, not just main):

   ```yaml
   push:
     branches: [main] # extend to all protected branches before activation
   pull_request:
     branches: [main]
   schedule:
     - cron: '23 4 * * 6' # weekly; select the agreed UTC window
   ```

   Recheck the current default setup languages, suite, protected branches and required checks. The template preserves the September 27 inventory, not automatic future language detection. Update the matrix when new languages enter the repository.
2. With explicit approval to publish the workflow and change repository settings, use Settings → Advanced Security → CodeQL analysis → Switch to advanced. GitHub's [switching instructions](https://docs.github.com/en/code-security/how-tos/find-and-fix-code-vulnerabilities/configure-code-scanning/configuring-advanced-setup-for-code-scanning) disable default setup as part of this operation. Coordinate the workflow merge/enablement and first run immediately; do not leave the dispatch-only experiment as the repository's sole scanner.
3. Verify one complete default-branch analysis, a PR analysis, all six uploads, query inventory and file coverage on the tool status page. Match existing scan categories/required checks where needed; inspect unresolved prior configurations rather than assuming a green workflow means complete coverage.
4. If activation or coverage checks fail, restore the previously recorded default setup and disable the candidate workflow through the approved settings change. Retain logs and results for diagnosis. Do not remove historical alerts or weaken checks to obtain a green result.

No part of that remote sequence was performed in this change.

## Benchmark and acceptance

After an approved advanced-setup switch, on the same default-branch commit and fixed CodeQL bundle, dispatch `rust_extractor_cache=false` to measure the uncached control, then `true` for the initial cache fill and repeated warm runs. Use at least three uncached and three warm measurements. Confirm a warm run actually restores the exact key; compare only equal toolchain/image/query inventories. Record median wall time and range, extraction/finalization/query durations, cache hit/size/restore/save time, actual CPU/RAM, warnings and scanned-file counts. Include initialization, uploads and post-job time in the total. The historical default run above is context, not a controlled before/after trial.

The cache is accepted only if all six analyses succeed with equivalent query and file coverage and total elapsed time improves after transfer overhead. Compare alert fingerprints and extraction diagnostics on the same source; investigate missing alerts rather than treating fewer results as a gain. If it does not improve total time or creates extraction differences, leave the extractor option/cache disabled and keep the full scanning workflow. Do not exclude tests, split away cross-crate analysis or disable security queries to shorten the run. Larger-runner experiments remain separately budgeted and approved.

Local validation covers YAML/Actions syntax and the template's intended cache/coverage structure. Actual extractor reuse, GitHub cache transfer performance and Code Scanning upload parity remain unverified until those approved remote trials.
