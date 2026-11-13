# IDE

A full multi-language development environment inside Otto (`#/ide`) uniting
**Local** and **Remote SSH** workspaces, OS/architecture-aware **SDK &
Toolchain installation**, per-project **Environments & Run Configurations**,
inline **Gutter Run/Debug**, a full **Debug Adapter Protocol (DAP) debugger**,
**LSP intelligence**, a side-by-side **AI Coding Agent dock (`⌘L`) & Inline
Edit (`⌘I`)**, **Share with Friends** live collaborative editing, and
zero-rebuild **runtime IDE plugins**.

- Route: `#/ide`. Sidebar entry **IDE** (Build group). ⌘K: "Go to IDE",
  "Toggle IDE AI Agent", "IDE: Inline Edit with Agent".
- RBAC:
  - Protected `/api/v1/ide/*` routes require **Agents: Edit** (`Require(Agents, Edit)`).
  - `/api/v1/ide/fs/*` is additionally guarded by `is_host_files_route`, so an
    agent-session token cannot escape its Seatbelt sandbox through the daemon's
    IDE filesystem routes.
  - Toolchain installation (`/ide/toolchains/install`, `/ide/toolchains/uninstall`)
    and mutating plugin routes (`/ide/plugins*`) carry `RouteClass::Admin`
    (human credentials only); minting a collab share room (`POST /ide/collab/rooms`)
    carries `RouteClass::Secret` (human credentials only).
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
    `ui/src/lib/components/cm-ide-git-and-agent.ts`, `ui/src/lib/api/ide.ts`.

## Walkthrough

### 1. Local & Remote SSH targets

Every IDE subsystem operates over a unified `IdeTargetSpec`:

- **Local target (`kind: "local"`):** Reads and writes inside the selected
  workspace or folder root on the macOS host, rejecting path traversal (`..`),
  Otto's own data directory, `~/.ssh`, and credential/key files for non-root
  callers.
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
- **Keymap presets:** Switch between **JetBrains macOS** (`⌘B` go to
  definition / usages, `⌥⏎` quick-fix, `⇧F6` rename symbol, `⌥⌘L` format,
  `⌘F8` toggle breakpoint, `⌘D` duplicate line/selection, `⌘⌫` delete line)
  and **Standard** (`F12`, `⌘.`, `F2`, `⇧⌥F`, `F9`, `⌘D` next occurrence).

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
| **Go** | Official Go SDK tarball, `gopls`, Delve (`dlv`), `gofumpt` |
| **Node / TypeScript** | Node LTS (22), `pnpm`, `typescript-language-server`, `vscode-langservers-extracted`, `prettier`, `js-debug` |
| **C / C++** | `clangd`, `clang-format`, `CodeLLDB` |

Installed `<data_dir>/toolchains/**/bin` directories are automatically prepended
to `PATH` (and managed JDKs set `JAVA_HOME`) across IDE runs, debug sessions,
and `/ws/lsp`.

### 4. Environments & Run Configurations

Per-project configuration is persisted in SQLite (`0214_ide_configs.sql`), with
secret environment variable values stored in the macOS Keychain via
`otto-keychain` (`ide-env:<project_hash>:<key>`):

- Select SDK/toolchain overrides per project (`java_home`, `go_root`,
  `rust_toolchain`, `node_home`, `python_interpreter`, `dotenv_path`,
  `extra_path`).
- Create project-local Python virtual environments (`POST /ide/environments/python-venv`
  via `uv venv` or `python3 -m venv`), discover `.env*` files, and configure
  environment variables (marking secrets so plaintext never touches SQLite).
- Define reusable **Run Configurations** (`go_test`, `go_run`, `java_junit`,
  `java_main`, `maven`, `gradle`, `cargo_test`, `cargo_run`, `node`, `npm`,
  `vitest`, `pytest`, `python`, `custom`), launch them in the **Run** bottom
  dock (`POST /ide/runs`) with live output and structured test-result trees, or
  stop running jobs (`POST /ide/runs/{run_id}/stop`).

### 5. Gutter Run / Debug

`gutter.rs` scans open source files and emits runnable entry points and test
items directly into the editor gutter:

- **Rust:** `fn main()`, `#[test]`, `#[tokio::test]`, `mod tests`
- **Go:** `func main()`, `func TestXxx(t *testing.T)`, `func BenchmarkXxx(b *testing.B)`
- **Python:** `if __name__ == "__main__":`, `def test_*`, `class Test*`
- **Java:** `public static void main`, `@Test`, `@ParameterizedTest`
- **JS / TS:** `describe(...)`, `it(...)`, `test(...)`
- **C / C++:** `int main(...)`, `TEST(...)`, `TEST_F(...)`

Clicking the gutter play button offers **Run** or **Debug** for that exact item.

### 6. Full DAP Debugger

`crates/otto-ide/src/dap/` speaks framing-accurate `Content-Length` JSON-RPC
Debug Adapter Protocol over stdio (or over `ssh -T` for Remote SSH targets)
against `codelldb` / `lldb-dap` (Rust, C/C++), `dlv dap` (Go), `debugpy.adapter`
(Python), Node inspector / `js-debug` (JS/TS), and `java-debug-adapter` + JDWP
(Java), plus a deterministic built-in mock adapter when testing without an
external debugger binary:

- **Breakpoints:** Line breakpoints, conditional breakpoints (`condition`), hit
  counts (`hit_condition`), logpoints (`log_message`), and exception filters
  (`caught` / `uncaught`).
- **Execution controls:** Continue (`F5` / `⌥⌘R`), Pause, Step Over (`F8` /
  `F10`), Step Into (`F7` / `F11`), Step Out (`⇧F8` / `⇧F11`), Restart, Stop,
  and **Run to Cursor**.
- **Inspection:** Threads & Call Stack frames, expandable Scopes & Variables
  trees (`variablesReference`), Watch expressions, an interactive Debug Console
  (REPL `evaluate`), and IntelliJ-style **inline ghost variable values** rendered
  at the end of source lines while paused.

### 7. LSP Intelligence

Combines two complementary paths:

- **Live WebSocket bridge (`WS /ws/lsp?lang=&root=&connection_id=`):** Multiplexes
  live `didOpen` / `didChange` diagnostics, hover, and completions per
  `(user, lang, root, ssh_target)`, automatically discovering binaries in
  `<data_dir>/toolchains/**/bin` and active runtime IDE plugins, and spawning
  over SSH when `connection_id` is set.
- **Structured IDE intelligence (`/api/v1/ide/lsp/*`):** Provides document
  outline symbols, workspace symbol search, go-to-definition, find usages
  (feeding the **Usages** bottom dock), cross-file symbol rename, formatting,
  diagnostics, and quick-fix code actions — backed by LSP when available with a
  fast lexical/regex fallback so navigation works immediately even before a
  language server is installed.

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

- Contributes custom `languages`, `lsp_servers` (automatically picked up by
  `/ws/lsp` and `/lsp/capabilities`), `dap_adapters`, `gutter_detectors`,
  `keymaps`, `snippets`, and `panels`.
- Reference plugins ship under `examples/ide-plugins/kotlin-ide` and
  `examples/ide-plugins/makefile-docker-pack`.
- Enable, disable (`POST /ide/plugins/{slug}/toggle`), or uninstall
  (`DELETE /ide/plugins/{slug}`) at runtime.

### 10. Side-by-Side AI Coding Agent (`⌘L`) & Inline Edit (`⌘I`)

`IdeAgentPanel.svelte` and `IdeInlinePromptBar.svelte` connect the IDE editor,
explorer, diagnostics, test runner, and debugger directly to Otto's structured
runtime sessions (`/api/v1/workspaces/{id}/runtime-sessions` across Claude,
Codex, and Gemini):

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
  `VecDeque` ring with incremental $O(1)$-per-line test tracking and optional
  `GET /ide/runs/{run_id}?tail_lines=`), finished runs (`MAX_FINISHED_RUNS = 16`),
  debug output (`1,000` entries) and terminated DAP sessions (`6`), collab rooms
  (`32` rooms, `2h` idle TTL, `32` buffers / `16 MiB` per room, zero buffer
  clones on cursor moves), and pooled LSP servers (`MAX_SERVERS_PER_OWNER = 3`,
  `45s` idle eviction, `rg --files-with-matches` reference pre-filtering, and a
  `2s` `otto_git_cmd::hardened_command()` git-status cache) are all strictly
  bounded.

## API & WebSocket summary

Full schemas and route tables live in [`docs/contracts/api.md`](../contracts/api.md)
(**IDE** section) and [`docs/contracts/ws.md`](../contracts/ws.md) (§1c).

- Target, FS, Search & Gutter: `GET /ide/fs/tree`, `GET /ide/fs/file`,
  `PUT /ide/fs/file`, `POST /ide/fs/mutate`, `POST /ide/search`,
  `POST /ide/replace`, `POST /ide/gutter`.
- Toolchains & Environments: `GET /ide/toolchains`, `POST /ide/toolchains/install`,
  `GET /ide/toolchains/jobs/{job_id}`, `GET /ide/environments`,
  `PUT /ide/environments`, `POST /ide/environments/python-venv`,
  `POST /ide/run-configs`, `DELETE /ide/run-configs/{id}`.
- Run & DAP Debugger: `POST /ide/runs`, `GET /ide/runs/{run_id}`,
  `POST /ide/runs/{run_id}/stop`, `POST /ide/debug/sessions`,
  `GET /ide/debug/sessions`, `GET /ide/debug/sessions/{id}`,
  `GET /ide/debug/sessions/{id}/events`,
  `PUT /ide/debug/sessions/{id}/breakpoints`,
  `POST /ide/debug/sessions/{id}/command`,
  `POST /ide/debug/sessions/{id}/variables`,
  `POST /ide/debug/sessions/{id}/evaluate`.
- LSP Intelligence: `POST /ide/lsp/symbols`, `POST /ide/lsp/references`,
  `POST /ide/lsp/rename`, `POST /ide/lsp/format`, `POST /ide/lsp/code-actions`,
  plus `WS /ws/lsp?lang=&root=&connection_id=`.
- Collaboration: `POST /ide/collab/rooms`, `GET /ide/collab/rooms`,
  `POST /ide/collab/rooms/{room_id}/role`, `DELETE /ide/collab/rooms/{room_id}`,
  and public guest routes `/ide/collab/guest/{room_id}/join`,
  `/ide/collab/guest/{room_id}/state`, `/ide/collab/guest/{room_id}/events`,
  `/ide/collab/guest/{room_id}/cursor`, `/ide/collab/guest/{room_id}/edit`,
  `/ide/collab/guest/{room_id}/debug-action`.
- Runtime IDE Plugins: `GET /ide/plugins`, `POST /ide/plugins/install`,
  `POST /ide/plugins/{slug}/toggle`, `DELETE /ide/plugins/{slug}`.

## Troubleshooting

- **"Language server unavailable" banner:** Click **Toolchains** in the IDE
  toolbar and install the matching language server (for example `rust-analyzer`,
  `gopls`, `pyright`, `jdtls`, `typescript-language-server`, or `clangd`). The
  editor reconnects automatically once the install job finishes.
- **Remote SSH target fails to connect:** Verify the SSH connection succeeds in
  **Connections** first (`BatchMode=yes` key auth is used for non-interactive
  LSP/DAP/command execution) and confirm your role grants `discover` and `shell`
  on enforced connections.
- **Remote guest cannot reach a Share with Friends link:** Like Session Rooms,
  `ottod` listens on loopback by default; remote guests outside your Mac need an
  active tunnel or tailnet listener (see [Remote / mobile access](./remote-mobile-access.md)).
