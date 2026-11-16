# Kotlin & Gradle IDE Pack (`kotlin-ide`)

Reference runtime IDE extension pack for Otto IDE (`otto-ide-plugin.json`), demonstrating zero-rebuild IDE customization.

## What It Contributes

- **Languages**: `kotlin` (`.kt`, `.kts`, `build.gradle.kts`, `settings.gradle.kts`) with `//` line comments and `ktlint -F` formatting.
- **LSP Server**: `kotlin-language-server` over stdio (local and Remote SSH).
- **DAP Debugger**: `kotlin-debug-adapter` for JVM/Kotlin breakpoint debugging.
- **Gutter Detectors**:
  - `fun main(...)` / `suspend fun main(...)` → `./gradlew run`
  - `fun testXxx()` / ``fun `should ...`()`` → `./gradlew test --tests *<name>*`
- **Keymaps, Snippets & Panels**: `funmain` / `ktest` editor snippets and Gradle verification panel metadata.

## Installing in Otto IDE

1. Open **IDE** (`#/ide`) → `⋯` → **IDE Plugins**.
2. Choose **Catalog** → click **Install** on `Kotlin & Gradle IDE Pack`, **or** choose **Install from Path / Git** and enter:
   ```text
   examples/ide-plugins/kotlin-ide
   ```
3. All `.kt` / `.kts` files immediately gain language detection, Structure outline symbols, gutter Run/Debug buttons, and `kotlin-language-server` attachment without restarting or rebuilding Otto.
