---
id: ide
title: IDE
group: Build
route: ide
summary: Multi-language IDE with rectangular & multi-cursor editing, LSP navigation and refactoring, full DAP debugging, SDK downloads, remote SSH workspaces and live peer collaboration.
---

## What it's for

**IDE** brings JetBrains-grade code editing, navigation, testing and step-through debugging directly into Otto for **Java & Spring Boot**, **Kotlin**, **Go**, **Rust**, **Zig**, **C / C++ & Unreal Engine**, **.NET (C# / F# / Unity / Unreal)**, **JavaScript / TypeScript & Web Frameworks** and **Python & Jupyter Notebooks** — locally or over an SSH connection, with zero external window switching.

## Getting started

1. Open **IDE** from the **Build** section of the sidebar (or click **Open in IDE** from any file tab in **Threads**). Otto attaches your active workspace folder automatically.
2. Browse files, symbol structure or breakpoints in the left rail, press `⌥O` to switch between C/C++ headers and sources, or press `⇧⇧` (**Search everywhere**) to jump to any file, symbol, text match or IDE action.
3. Click any gutter **Run** icon next to a `main` function or unit test (`@Test`, `func Test*`, `#[test]`, `test "..."`, `TEST(...)`, `TEST_CASE(...)`, `[Fact]`/`[Test]`/`[TestMethod]`, `def test_*`, `it(...)`) to run or debug it in one click.
4. Need a missing compiler, language server or debugger? Open **⋯ → Toolchains & SDKs** to download architecture-matched JDKs, Kotlin (`kotlin-language-server`), Go, Rust, Zig (`zls`), C/C++ (`clangd`, `CodeLLDB`), .NET (`csharp-ls` / `netcoredbg`), Node.js or Python toolchains directly onto your Mac or remote SSH host.

## Everything it can do

- **Multi-cursor & column editing** — Hold `⌥` and drag vertically across lines for rectangular box selection, `⌥`-click to place carets, press `⌥⇧I` to split a multi-line selection into per-line cursors, or open the **Prefix / suffix** bar in the editor header to wrap every selected line with custom text before and after in a single undoable step.
- **LSP intelligence & refactoring** — Hover types, completions, inline diagnostics, **Go to definition** (`⌘B`), **Find usages** (`⌥F7`), **Workspace rename** (`⇧F6`) with multi-file diff preview, **Quick fixes** (`⌥⏎`) and **Format file** (`⌥⌘L`).
- **Full DAP debugger** — Line, conditional, hit-count and logpoint breakpoints (`⌘F8`), step over/into/out, call stack frames, expandable local/global scopes, watch expressions, an interactive **Evaluate** REPL and inline ghost variable values next to paused source lines.
- **Environments, Jupyter & ML workflow** — Bind per-project SDK homes, `.venv` / `conda` / `uv` / `poetry` interpreters (or create a `.venv` in one click), `.env` files, Keychain-backed secret variables and named run/debug configurations (including auto-detected `train.py` / `finetune.py` unbuffered training runs, loopback TensorBoard and Jupyter Lab). Open `.ipynb` notebooks directly in Otto on a real Jupyter kernel — any installed kernelspec (ipykernel, R, Bash, …) or the ipykernel in your project's Python env — with `%magic`, `!shell` and `?help`, outputs that stream while the cell runs, `input()` prompts, interrupt/restart, a kernel status chip, rich inline outputs (matplotlib images, HTML tables, coloured tracebacks), a live Variables inspector and GPU/Apple Silicon MPS detection. A Python env without ipykernel falls back to Otto's built-in kernel (no IPython; **⋯ → Install ipykernel…** upgrades it).
- **Remote SSH & live sharing** — Switch the target badge in the header to any saved SSH host to edit, search, build and debug remotely, or click **Share** to invite a teammate with viewer or co-debugger permissions and live multi-cursor presence.
- **Debugging with an agent** — Ask an agent to "use Otto to debug …": it opens the project, sets its breakpoints, and after your **Debug** confirm (one per start, showing the exact command) runs the test or service, sends the request and explains each stop. Reply or press **Proceed** on the driving bar; your own Step/Continue always win.
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
- **Zig, Kotlin, Vue and .NET language servers wait for trust** — `zls` runs `build.zig`, `kotlin-language-server` imports the Gradle / Maven build, `vue-language-server` loads the plugins `tsconfig.json` names and `csharp-ls` / `fsautocomplete` load the MSBuild project (its design-time build runs the project's targets), so they start only after you trust the project: click **Trust project…** in the notice above the editor (or the "needs trust" item in the status strip). Turn it off again with **Edit run configurations… → Project environment → Trust this project**. (For scripts: that switch is `OTTO_IDE_TRUST_PROJECT_EXECUTION=1`; `OTTO_IDE_TRUST_ZIG_BUILD`, `OTTO_IDE_TRUST_KOTLIN_LSP`, `OTTO_IDE_TRUST_VUE_LSP` and `OTTO_IDE_TRUST_DOTNET_LSP` set to `1` still work per language.)
- **What Debug does not do** — Playwright and Karma test runs can be Run but not debugged; a .NET Debug of all tests is refused when the folder or solution holds several test projects (debug one test from the gutter, or a ".NET Test" configuration that targets one project — add it in Launch settings when none is listed); Unity Editor attach is not supported.
- **C/C++ compilers come from your system** — Install Clang or GCC with Xcode Command Line Tools, Homebrew or your Linux package manager; Otto detects them and downloads only `clangd` and CodeLLDB.
- **Notebook kernels** — Pick the kernel in the notebook toolbar (Jupyter kernels first, the built-in one only for envs without ipykernel); the notebook remembers it in its metadata. A kernelspec stored inside the project asks once before its first start. Each open notebook runs one kernel (12 at most), which stops when its tab closes. Debug Cell works on the built-in kernel only for now; widgets and JavaScript outputs show as text.

## Related

- **Threads** — Inspect agent changes and jump straight into **IDE** at the exact file and line via **Open in IDE**.
- **Git** — Review diffs, stage hunks and open pull requests once your changes pass tests.
- **Connections** — Manage SSH hosts available as remote IDE targets.
