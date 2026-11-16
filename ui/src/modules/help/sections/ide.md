---
id: ide
title: IDE
group: Build
route: ide
summary: Multi-language IDE with rectangular & multi-cursor editing, LSP navigation and refactoring, full DAP debugging, SDK downloads, remote SSH workspaces and live peer collaboration.
---

## What it's for

**IDE** brings JetBrains-grade code editing, navigation, testing and step-through debugging directly into Otto for **Java**, **Go**, **Rust**, **JavaScript / TypeScript** and **Python** — locally or over an SSH connection, with zero external window switching.

## Getting started

1. Open **IDE** from the **Build** section of the sidebar (or click **Open in IDE** from any file tab in **Threads**). Otto attaches your active workspace folder automatically.
2. Browse files, symbol structure or breakpoints in the left rail, or press `⇧⇧` (**Search everywhere**) to jump to any file, symbol, text match or IDE action.
3. Click any gutter **Run** icon next to a `main` function or unit test (`@Test`, `func Test*`, `#[test]`, `def test_*`, `it(...)`) to run or debug it in one click.
4. Need a missing compiler, language server or debugger? Open **⋯ → Toolchains & SDKs** to download architecture-matched JDKs, Go, Rust, Node.js or Python toolchains directly onto your Mac or remote SSH host.

## Everything it can do

- **Multi-cursor & column editing** — Hold `⌥` and drag vertically across lines for rectangular box selection, `⌥`-click to place carets, press `⌥⇧I` to split a multi-line selection into per-line cursors, or open the **Prefix / suffix** bar in the editor header to wrap every selected line with custom text before and after in a single undoable step.
- **LSP intelligence & refactoring** — Hover types, completions, inline diagnostics, **Go to definition** (`⌘B`), **Find usages** (`⌥F7`), **Workspace rename** (`⇧F6`) with multi-file diff preview, **Quick fixes** (`⌥⏎`) and **Format file** (`⌥⌘L`).
- **Full DAP debugger** — Line, conditional, hit-count and logpoint breakpoints (`⌘F8`), step over/into/out, call stack frames, expandable local/global scopes, watch expressions, an interactive **Evaluate** REPL and inline ghost variable values next to paused source lines.
- **Environments & run configurations** — Bind per-project SDK homes, `.venv` interpreters (or create a `.venv` in one click), `.env` files, Keychain-backed secret variables and named run/debug configurations.
- **Remote SSH & live sharing** — Switch the target badge in the header to any saved SSH host to edit, search, build and debug remotely, or click **Share** to invite a teammate with viewer or co-debugger permissions and live multi-cursor presence.
- **Runtime IDE plugins** — Install extension packs from a local folder, Git URL or curated catalog via **⋯ → Plugins** without rebuilding Otto.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `⇧⇧` | Search everywhere (files, symbols, text, actions) |
| `⌘S` | Save active file (prompts on external disk conflict) |
| `⌘E` | Recent files menu |
| `⌘B` | Go to definition or usages |
| `⇧F6` | Rename symbol across workspace |
| `⌥⏎` | Quick fixes and code actions |
| `⌥⌘L` | Format active file |
| `⌥ + drag` | Rectangular / column multi-line selection |
| `⌥⇧↑` / `⌥⇧↓` | Clone cursor above / below |
| `⌃G` / `⌃⌘G` | Select next / all occurrences |
| `⌥⇧I` | Split selection into cursors at end of each line |
| `⌃R` / `⌃D` | Run / Debug selected configuration |
| `⌘F8` | Toggle line breakpoint |
| `F8` / `F7` / `⇧F8` | Step over / Step into / Step out |
| `⌥⌘R` | Resume program execution |

## Tips and limits

- **JetBrains vs Standard keymap** — Toggle between JetBrains macOS bindings and Standard bindings anytime from **⋯ → Keymap**.
- **Safe conflict handling** — Every save verifies the file's SHA-256 digest on disk first so concurrent agent edits are never silently overwritten.
- **Secrets stay in Keychain** — Secret environment variables are stored in macOS Keychain (`otto-keychain`); only opaque key references live in SQLite.

## Related

- **Threads** — Inspect agent changes and jump straight into **IDE** at the exact file and line via **Open in IDE**.
- **Git** — Review diffs, stage hunks and open pull requests once your changes pass tests.
- **Connections** — Manage SSH hosts available as remote IDE targets.
