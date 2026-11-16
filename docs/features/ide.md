# IDE

A full multi-language development environment inside Otto (`#/ide`) uniting
**Local** and **Remote SSH** workspaces, OS/architecture-aware **SDK &
Toolchain installation**, per-project **Environments & Run Configurations**,
inline **Gutter Run/Debug**, a full **Debug Adapter Protocol (DAP) debugger**,
**LSP intelligence**, a side-by-side **AI Coding Agent dock (`⌘L`) & Inline
Edit (`⌘I`)**, **Share with Friends** live collaborative editing, and
zero-rebuild **runtime IDE plugins**.

- Route: `#/ide`. The project switcher always lists **Scratches** first — the
  user's own scratch folder, the same in every workspace ([Scratches](./scratches.md));
  `#/workbench` opens it. Sidebar entry **IDE**
  (Build group). ⌘K: "Go to IDE", "Toggle IDE AI Agent", "IDE: Inline Edit with Agent".
- RBAC:
  - Protected workspace `/api/v1/ide/*` routes require **Agents: Edit** (`Require(Agents, Edit)`),
    and are additionally guarded by `is_host_files_route` so an agent-session
    token cannot escape its Seatbelt sandbox through the daemon's IDE filesystem,
    runner, debugger, or history routes.
  - Toolchain installation and local SDK mutations (`/ide/toolchains/install`,
    `/ide/toolchains/local`) and mutating plugin routes (`/ide/plugins*`) require
    **Settings: Admin**, carry `RouteClass::Admin` (human credentials only), and
    enforce root authority (`GET /ide/toolchains/jobs/{job_id}` requires
    **Agents: Edit** + root).
  - Shell environment routes (`/ide/shell-env*`) carry `RouteClass::Secret` and
    require human credentials + root authority; project environment routes
    (`GET/PUT /ide/environments`) and minting a collab share room
    (`POST /ide/collab/rooms`) carry `RouteClass::Secret` (human credentials only);
    and subprocess-spawning workspace mutations (`python-venv`, `deps/action`,
    `POST /ide/runs`, `POST /ide/debug/sessions`) carry `RouteClass::Admin`
    (human credentials only).
  - Public guest routes (`/api/v1/ide/collab/guest/*`) sit outside the bearer
    chokepoint and authenticate each request with the room's capability share
    token (`viewer` vs `editor`).
- Code:
  - Engine & routers: `crates/otto-ide/` (`target.rs`, `target/fs_ops.rs`,
    `toolchains_catalog.rs`, `toolchains.rs`, `environments.rs`,
    `environments/runner.rs`, `gutter.rs`, `dap/`, `lsp_service.rs`,
    `lsp_host/`, `collab.rs`, `plugins.rs`, `http.rs`).
  - Host wiring & `/ws/lsp` bridge: `crates/otto-server/src/lsp/mod.rs`,
    `crates/otto-server/src/modules/ctx_impls.rs`, `crates/otto-server/src/policy.rs`.
  - UI: `ui/src/modules/ide/` (`IdePage.svelte`, `IdeAgentPanel.svelte`,
    `IdeInlinePromptBar.svelte`, `store/ideAgentController.svelte.ts`,
    `store/ideDiffApplier.ts`, `store/ideBufferPerf.ts`),
    `ui/src/lib/components/CodeEditor.svelte`,
    `ui/src/lib/components/cm-ide-extensions.ts`,
    `ui/src/lib/components/cm-ide-git-and-agent.ts`,
    `ui/src/lib/api/types/ide.ts`, `ui/src/lib/api/types/ide-intelligence.ts`.

## Walkthrough

### 1. Local & Remote SSH targets

Every IDE subsystem operates over a unified `IdeTargetSpec`:

- **Local target (`kind: "local"`, plus IntelliJ-style multi-folder projects):**
  Reads and writes inside the selected workspace or folder root **plus any
  user-attached project folders** (`POST / DELETE /ide/projects/folders`,
  persisted in `additional_roots` and rendered alongside the top-level primary
  project folder node in `IdeProjectTree`), plus read-only inspection of
  external SDK and dependency sources (`$GOROOT/src`, `$GOMODCACHE`,
  `node_modules`, JDK `src.zip` / `jar:file://!...` / `javap`, Rust `rustup`
  stdlib & Cargo registry, Python stdlib & `.venv`), while rejecting path
  traversal (`..`), Otto's own data directory, `~/.ssh`, and credential/key
  files for non-root callers.
- **Remote SSH target (`kind: "ssh"`, `connection_id`):** Reuses saved SSH
  connections from **Connections** (`ConnectionKind::Ssh`, verified through
  workspace/global RBAC plus `discover` and `shell` resource access and SSH
  option-injection checks). File tree browsing, reads, atomic writes,
  project-wide search & replace, gutter runs, DAP debugging, and `/ws/lsp`
  all run transparently over SSH (`otto_ssh` SFTP + POSIX-escaped `ssh -T`
  remote commands).

### 2. Multi-cursor editor, prefix/suffix & JetBrains/Standard keymaps

`CodeEditor.svelte` lazy-loads `cm-ide-extensions.ts` when IDE features are
active:

- **Multi-cursor:** `Option + Click` adds/removes cursors; `Option + Drag`
  draws rectangular column selections; `⌥⇧I` splits a multi-line selection into
  one cursor per line; `⌥⇧↑` / `⌥⇧↓` (`⌘⌥↑` / `⌘⌥↓`) adds cursors above/below;
  `⌃G` selects the next occurrence and `⌃⌘G` selects all occurrences.
- **Add Prefix / Suffix to Lines:** Wraps every selected line (or every cursor
  line) with a chosen prefix and/or suffix in a single undoable CodeMirror
  transaction.
- **Keymap presets:** Switch between **JetBrains macOS** (`⌘,` IDE Settings &
  Project Structure, `⌘B` go to definition / usages, `⌥⏎` instant `<10ms`
  deterministic quick-fix, `⇧F6` rename symbol, `⌥⌘L` format, `⌘F8` or 1-click
  gutter breakpoint toggle, `⌘D` duplicate line/selection / compare files,
  `⌘⇧V` clipboard history, `⌘⌫` delete line) and **Standard** (`F12`, `⌘.`,
  `F2`, `⇧⌥F`, `F9`, `⌘D` next occurrence).
- **6 Color Schemes, Semantic Coloring & Rainbow Brackets:** Choose between
  **Follow Otto Dark/Light**, **JetBrains Islands Dark**, **JetBrains Darcula**,
  **JetBrains Islands Light**, **VS Code Dark Modern**, and **VS Code Light Modern**
  in `⌘,` (`Editor → Color Scheme`), with distinct semantic token coloring
  (`package` vs exported struct field vs function call in Go, `@Annotation` gold
  vs instance field purple in Java) and 4-stop depth-cycled **Rainbow Brackets**
  (`()`, `[]`, `{}`).

### 3. SDKs & Toolchains Manager

Detects `os` (`darwin` / `linux`) and `arch` (`aarch64` / `x86_64`) on the
active Local or Remote SSH target and manages isolated SDKs, language servers,
debuggers, and formatters under `<data_dir>/toolchains/<ecosystem>/` locally
or `~/.otto/toolchains/<ecosystem>/` on SSH targets — without polluting
global system directories:

| Ecosystem | Components |
|-----------|------------|
| **Java / JVM** | Adoptium Temurin JDK (17, 21 LTS), Maven, Gradle, Eclipse `jdtls`, `java-debug-adapter` |
| **Python** | `uv`, standalone CPython (3.11 / 3.12 / 3.13), `pyright`, `ruff`, `debugpy` |
| **Rust** | `rustup` (`rustc` + `cargo`), `rust-analyzer`, `CodeLLDB` (`codelldb`) |
| **Go** | Official Go SDK tarball, local SDK registration (`POST /ide/toolchains/local`), `gopls`, Delve (`dlv`), `gofumpt` |
| **Node / TypeScript** | Node LTS (22), `pnpm`, `typescript-language-server`, `vscode-langservers-extracted`, `prettier`, `js-debug` |
| **Shell (Bash / Zsh)** | `bash-language-server`, `shellcheck`, `shfmt`, built-in `otto-sh-dap` (`DEBUG` trap DAP debugger) |
| **JSON & CSV / TSV** | `vscode-json-language-server`, live JSONPath breadcrumb, Pretty / Minify / Sort Keys, `$ref` navigation, virtualized `IdeCsvView` (`Table ↔ Text` toggle, rainbow columns, sort/filter, inline cell edit + undo, column stats) |

Installed `<data_dir>/toolchains/**/bin` directories and custom registered local
SDKs (`POST /ide/toolchains/local`) are automatically prepended to `PATH` (setting
`GOROOT` / `JAVA_HOME` / `NODE_HOME` as appropriate) across IDE runs, debug
sessions, terminals, and `/ws/lsp`.

### 4. Cached Shell Environment, Environments & Run Configurations

Because `ottod` runs under macOS `launchd` / `Otto.app`, `std::env` lacks user
variables exported in `~/.zshenv`, `~/.zprofile`, or `~/.zshrc` (such as
`GOPRIVATE` or app-specific debug params). `otto-ide` captures a **cached
login+interactive `$SHELL -lic` environment snapshot** once (`3s` timeout,
viewable at `GET /ide/shell-env` and reloadable via `POST /ide/shell-env/reload`,
never spawned per request) and merges:

`daemon base env → cached login shell env → selected SDK env → project .env file → Run/Debug config overrides`

into **all** Run executions, DAP Debug sessions, IDE Terminals, and local LSP
servers (`gopls`, `jdtls`, `rust-analyzer`, `pyright`, `tsserver`).

Per-project configuration is persisted in SQLite (`0214_ide_configs.sql`), with
secret environment variable values stored in the macOS Keychain via
`otto-keychain` (`ide-env:<project_hash>:<key>`):

- Select SDK/toolchain overrides per project (`java_home`, `go_root`,
  `rust_toolchain`, `node_home`, `python_interpreter`, `dotenv_path`,
  `extra_path`, `inherit_shell_env`, `additional_roots`).
- Create project-local Python virtual environments (`POST /ide/environments/python-venv`
  via `uv venv` or `python3 -m venv`), discover `.env*` files, and configure
  environment variables (marking secrets so plaintext never touches SQLite).
- Instant (`< 25ms`, pure file-stat/manifest scan with zero subprocess spawns)
  runner discovery across primary and attached folders plus reusable **Run
  Configurations** (`go_test`, `go_run`, `java_junit`, `java_main`, `maven`,
  `gradle`, `cargo_test`, `cargo_run`, `node`, `npm`, `vitest`, `pytest`,
  `python`, `shell`, `compound`, `custom`), launched in the **Run** bottom dock
  (`POST /ide/runs`) with live output and structured test-result trees, or
  stopped via `POST /ide/runs/{run_id}/stop`.

### 5. Gutter Run / Debug

`gutter.rs` scans open source files and emits runnable entry points and test
items directly into the editor gutter:

- **Rust:** `fn main()`, `#[test]`, `#[tokio::test]`, `mod tests`
- **Go:** `func main()`, `func TestXxx(t *testing.T)`, `func BenchmarkXxx(b *testing.B)`
- **Python:** `if __name__ == "__main__":`, `def test_*`, `class Test*`
- **Java:** `public static void main`, `@Test`, `@ParameterizedTest`
- **JS / TS:** `describe(...)`, `it(...)`, `test(...)`
- **Shell (Bash / Zsh):** runnable scripts and functions (plus custom `gutter_detectors` contributed by runtime IDE plugins)

Clicking the gutter play button offers **Run** or **Debug** for that exact item.

### 6. Full DAP Debugger & Tabbed/Split Dock

`crates/otto-ide/src/dap/` speaks framing-accurate `Content-Length` JSON-RPC
Debug Adapter Protocol over stdio (or over `ssh -T` for Remote SSH targets)
plus zero-install built-in bridges across every major ecosystem: `dlv dap` (Go),
`java-debug-adapter` or built-in `otto-jdi-jdwp` (`jdb`/JDWP for Java),
`debugpy.adapter` or built-in `otto-py-bdb` (Python stdlib `bdb`), `js-debug`
or built-in `otto-node-cdp` (`node --inspect-brk=0` CDP), `codelldb` /
`lldb-dap` or built-in `otto-lldb-dap` (`/usr/bin/lldb` for Rust), and
`otto-sh-dap` (`DEBUG` trap for Bash/Zsh), plus a deterministic mock adapter:

- **1-Click Gutter Breakpoints:** Clicking anywhere beside or on line numbers in
  `.cm-gutters` toggles verified breakpoints in 1 click (persisted across tab
  switches and restarts); right-clicking edits conditions (`condition`), hit
  counts (`hit_condition`), logpoints (`log_message`), and exception filters.
- **Execution controls & Tabbed/Split layout:** Switch between `Debugger`
  (`Frames` | `Variables & Watches` with draggable column resizer and hideable
  panes), full-width `Console` (with automatic multi-line JSON log pretty
  formatting), or side-by-side **Split** mode alongside a draggable bottom-dock
  height resizer, inline ghost variable values, and an integrated **Git** dock
  tab reusing Otto's `DiffViewer` + stage/commit controls.

### 7. LSP Intelligence, Quick-Doc, Manifest Inspector & Local History

Combines live LSP multiplexing with zero-roundtrip editor intelligence:

- **Live WebSocket bridge (`WS /ws/lsp?lang=&root=&connection_id=`):** Multiplexes
  live `didOpen` / `didChange` diagnostics, hover, and completions per
  `(user, lang, root, ssh_target)`, automatically discovering binaries in
  `<data_dir>/toolchains/**/bin` and active runtime IDE plugins, and spawning
  over SSH when `connection_id` is set.
- **Structured IDE intelligence (`/api/v1/ide/lsp/*`):** Provides document
  outline symbols, workspace symbol search, go-to-definition, find usages
  (feeding the **Usages** bottom dock), cross-file symbol rename, formatting,
  Lombok/`src.zip`/Maven/Gradle-aware Java & polyglot diagnostics
  (`POST /ide/lsp/diagnostics`), compact JetBrains-style **Quick-Doc hover**
  (`POST /ide/lsp/hover`, isolated strictly to the hovered declaration with zero
  neighbor-comment bleed), and `<10ms` deterministic `⌥⏎` **Quick Fixes**
  applied as native `⌘Z`-undoable CodeMirror transactions.
- **Settings (`⌘,`), Named Multi-Repo Projects, Build Tools & Dependency Inspector:**
  Two-pane IntelliJ-style Settings & Project Structure modal (`Project` /
  per-module SDKs with auto-detected host SDKs, `Languages & Frameworks`,
  `Build Tools`), named multi-repo workspace projects (`/ide/projects*`), a
  collapsible right-hand **Maven / Gradle** tool window (`POST /ide/build-tools/model`)
  with Maven & Spring Boot profile activation, and inline **Dependency &
  Vulnerability Intelligence** (`/ide/deps/*`) with `[Text | Dependency Analyzer]`
  tabs on `go.mod`, `pom.xml`, `build.gradle*`, `Cargo.toml`, `package.json`,
  and `pyproject.toml`.
- **Interactive Comparison Tool & Backend Local History (`/ide/history/*`):**
  Side-by-side diff modal (`IdeComparisonModal`) with per-hunk `>>` / `<<`
  apply/revert arrows and an editable right-hand buffer, backed by a content-addressed
  daemon Local History journal that snapshots every save, refactor, and
  deletion so deleted files can be restored with 1 click from folder **Local
  History (`Deleted`)**.

### 8. Share with Friends (Live Collaboration)

Hosts start a collaborative room (`POST /api/v1/ide/collab/rooms`) scoped to a
target root with a chosen default guest role (`viewer` or `editor`) and copy a
capability invite link (`#/ide?collab=<room_id>&token=<invite_token>`):

- Guests join through `/api/v1/ide/collab/guest/{room_id}/join` without needing
  an Otto user account, authenticating subsequent calls via `X-Ide-Collab-Token`
  or `?token=`.
- Live colored peer cursors, multi-range selections, active file indicators,
  **Follow peer**, real-time buffer edits, and shared debug session snapshots
  stream over `GET /api/v1/ide/collab/guest/{room_id}/events` (`text/event-stream`).
- **Viewers** can browse files and follow cursors; any edit or debug action from
  a `viewer` peer is rejected with `403 Forbidden`. Hosts can promote/demote
  individual peers (`POST /ide/collab/rooms/{room_id}/role`) or close the room
  (`DELETE /ide/collab/rooms/{room_id}`, revoking all guest tokens).

### 9. Runtime IDE Plugins

Install an `otto-ide-plugin.json` extension into `<data_dir>/ide-plugins/<slug>/`
(from a local path, `https://` Git repository, or the built-in catalog:
`catalog:kotlin-ide`, `catalog:makefile-docker-pack`, `catalog:graphql-ide`,
`catalog:protobuf-grpc-ide`) or declare an `"ide"` contribution block inside
`~/otto-plugins/*/otto-plugin.json` — with zero app rebuild:

- Contributes custom `languages`, `lsp_servers` (such as Kotlin
  `kotlin-language-server` or C/C++ `clangd`, automatically picked up by
  `/ws/lsp` and `/lsp/capabilities`), `dap_adapters`, `gutter_detectors`,
  `keymaps`, `snippets`, and `panels`.
- Reference plugins ship under `examples/ide-plugins/kotlin-ide` and
  `examples/ide-plugins/makefile-docker-pack`.
- Enable, disable (`POST /ide/plugins/{slug}/toggle`), or uninstall
  (`DELETE /ide/plugins/{slug}`) at runtime.

### 10. Unified Otto Threads AI Coding Agent (`⌘L`) & Inline Edit (`⌘I`)

`IdeAgentPanel.svelte` and `IdeInlinePromptBar.svelte` share the **exact same
Otto Threads pool** as `#/threads` (`threadsSessionStore`), uniting structured
runtime sessions (`/api/v1/workspaces/{id}/runtime-sessions` across `claude`,
`codex`, `gemini`, `agy`, `grok`, `opencode`, `pi` and `muse`) with 1:1 attachment and
composer capability parity, the shared `ModelPicker`, and the thread switcher:

- **6-source context marking (`ideAgentController.svelte.ts`):**
  1. **Editor selection (`⌘L` / contextual selection bar / right-click menu):**
     attaches `@relPath:Lstart-Lend` with exact line offsets.
  2. **Project Explorer & open tabs:** attaches full files (`@relPath`, bounded
     per chip and capped at 12 chips / 28 KB total `<ide_context>` payload).
  3. **Structure outline symbols:** attaches `@relPath#SymbolName` slices.
  4. **Problems panel (`Fix with Agent`):** attaches `⚠ relPath:line` with
     surrounding source context and pre-fills the repair prompt.
  5. **Run & Tests panel (`Fix` on failed tests):** attaches `✗ TestName` with
     extracted failure output.
  6. **DAP Debugger (`Ask Agent` while paused):** attaches `⏸ func @ file:line`
     with active stack frames and local scope variables.
- **Proposal diff preview & native `⌘Z`-undoable apply (`ideDiffApplier.ts`):**
  Strips echoed `<ide_context>` blocks before parsing fenced code proposals
  (`path=...`, positional path, `title="..."`, `:Lstart-Lend`, or active-file /
  selection fallback), renders `+added` / `-removed` unified diffs, and applies
  accepted edits through a single CodeMirror transaction (`applyAgentEdit`) so
  `⌘Z` or the card's **Undo** button reverts the change in one step.
- **Inline selection edit (`⌘I`):** Opens `IdeInlinePromptBar` anchored above
  the active editor with multi-line prompt support, compact unified diff
  preview, **Accept** / **Discard**, and 1-click **In Chat `⌘L`** handoff.

### 11. Performance, Virtualization & Memory Bounds

- **Zero idle polling & zero closed-dock overhead:** `IdeAgentPanel`,
  `IdeInlinePromptBar`, and all six IDE modals are lazy-loaded via
  `lazyComponent` + `<LazyMount>`; runtime event subscriptions mount only while
  the Agent dock or inline bar is open, and debugger polling runs strictly while
  an active non-terminal session exists (`0` background `/api/` requests at idle).
- **Virtualization & keystroke path:** `IdeProjectTree` windows visible rows
  past `150` entries (`< 140` DOM rows on 2,500+ file trees); `ideBufferPerf.ts`
  stores live keystrokes in a non-reactive `Map` (`250ms` debounced sync +
  abortable `280ms` gutter/symbol loads); `CodeEditor` caps cached IDE editor
  states (`8`, skipping docs `> 200 KB`) and renders modified-line gutter bars
  (`cm-ide-diff-gutter`) under `3,000` lines.
- **Bounded daemon state (`crates/otto-ide`):** Run stdout (`2,000`-line
  `VecDeque` ring with incremental $O(1)$-per-line test tracking, live
  `GET /ide/runs/{run_id}/events` SSE streaming, and optional
  `GET /ide/runs/{run_id}?tail_lines=`), finished runs (`MAX_FINISHED_RUNS = 8`),
  debug output (`400` entries / `256 KiB`), terminated DAP sessions (`4`),
  external incremental Go/Java build caches outside user repositories, collab
  rooms (`32` rooms, `2h` idle TTL, `32` buffers / `16 MiB` per room, zero buffer
  clones on cursor moves), and pooled module-scoped LSP servers
  (`MAX_SERVERS_PER_OWNER = 3`, `MAX_GLOBAL_SERVERS = 3`, `1.2 GiB` combined LSP
  RSS budget with inactive-first `active_clients == 0` LRU eviction, observable
  via `GET /ide/lsp/status`) are all strictly bounded with zero stale FS/tree
  caches.

## API & WebSocket summary

Full schemas and route tables live in [`docs/contracts/api.md`](../contracts/api.md)
(**IDE** section) and [`docs/contracts/ws.md`](../contracts/ws.md) (§1c).

- Target, Named Projects, Multi-Folder, FS, Local History, Search & Gutter:
  `GET /ide/fs/tree`, `GET /ide/fs/file`, `PUT /ide/fs/file`,
  `POST /ide/fs/mutate`, `POST /ide/fs/complete`, `GET /ide/history/list`,
  `GET /ide/history/entry`, `POST /ide/history/restore`,
  `POST /ide/history/snapshot`, `GET /ide/projects`, `POST /ide/projects`,
  `DELETE /ide/projects/{id}`, `POST /ide/projects/scan-repos`,
  `POST /ide/projects/folders`, `DELETE /ide/projects/folders`,
  `POST /ide/projects/folders/bulk`, `POST /ide/search`, `POST /ide/replace`,
  `POST /ide/gutter`.
- Toolchains, Shell Env, Build Tools, Dependencies & Environments:
  `GET /ide/toolchains`, `POST /ide/toolchains/install`,
  `POST /ide/toolchains/local`, `DELETE /ide/toolchains/local`,
  `GET /ide/toolchains/jobs/{job_id}`, `POST /ide/toolchains/python-venv`,
  `GET /ide/shell-env`, `POST /ide/shell-env/reload`, `POST /ide/deps/inspect`,
  `POST /ide/deps/action`, `POST /ide/build-tools/model`,
  `GET /ide/environments`, `PUT /ide/environments`,
  `POST /ide/environments/python-venv`, `POST /ide/run-configs`,
  `DELETE /ide/run-configs/{id}`.
- Run & DAP Debugger: `POST /ide/runs`, `GET /ide/runs/{run_id}`,
  `GET /ide/runs/{run_id}/events`, `POST /ide/runs/{run_id}/stop`,
  `POST /ide/debug/sessions`,
  `GET /ide/debug/sessions`, `GET /ide/debug/sessions/{id}`,
  `GET /ide/debug/sessions/{id}/events`,
  `PUT /ide/debug/sessions/{id}/breakpoints`,
  `POST /ide/debug/sessions/{id}/command`,
  `POST /ide/debug/sessions/{id}/variables`,
  `POST /ide/debug/sessions/{id}/evaluate`.
- LSP Intelligence: `GET /ide/lsp/status`, `POST /ide/lsp/symbols`, `POST /ide/lsp/definition`,
  `POST /ide/lsp/implementation`, `POST /ide/lsp/references`,
  `POST /ide/lsp/rename`, `POST /ide/lsp/format`, `POST /ide/lsp/code-actions`,
  `POST /ide/lsp/diagnostics`, `POST /ide/lsp/hover`,
  plus `WS /ws/lsp?lang=&root=&connection_id=`.
- Collaboration: `POST /ide/collab/rooms`, `GET /ide/collab/rooms`,
  `POST /ide/collab/rooms/{room_id}/role`, `DELETE /ide/collab/rooms/{room_id}`,
  and public guest routes `/ide/collab/guest/{room_id}/join`,
  `/ide/collab/guest/{room_id}/state`, `/ide/collab/guest/{room_id}/events`,
  `/ide/collab/guest/{room_id}/cursor`, `/ide/collab/guest/{room_id}/edit`,
  `/ide/collab/guest/{room_id}/debug-action`.
- Runtime IDE Plugins: `GET /ide/plugins`, `POST /ide/plugins/install`,
  `POST /ide/plugins/{slug}/toggle`, `DELETE /ide/plugins/{slug}`.
- Live Database Watch & Component-Test Watcher: `POST /ide/db-watch/detect`,
  `POST /ide/db-watch/snapshot` (scans code for MySQL, PostgreSQL, MongoDB,
  Redis, and ClickHouse tables/collections/key prefixes, pairs with saved DB
  connections or live local Testcontainers / `miniredis` instances, and computes
  read-only `Before -> After` row/field diffs across debug steps and test runs).

## Troubleshooting

- **"Language server unavailable" banner:** Click **Toolchains** in the IDE
  toolbar and install the matching language server (for example `rust-analyzer`,
  `gopls`, `pyright`, `jdtls`, or `typescript-language-server`, or install/enable
  a runtime IDE plugin for additional languages such as Kotlin or C/C++). The
  editor reconnects automatically once the install job finishes.
- **Remote SSH target fails to connect:** Verify the SSH connection succeeds in
  **Connections** first (`BatchMode=yes` key auth is used for non-interactive
  LSP/DAP/command execution) and confirm your role grants `discover` and `shell`
  on enforced connections.
- **Remote guest cannot reach a Share with Friends link:** Like Session Rooms,
  `ottod` listens on loopback by default; remote guests outside your Mac need an
  active tunnel or tailnet listener (see [Remote / mobile access](./remote-mobile-access.md)).
