# Makefile, Dockerfile & Compose Pack (`makefile-docker-pack`)

Reference runtime IDE extension pack for Otto IDE (`otto-ide-plugin.json`) adding build-file and container authoring capabilities at runtime.

## What It Contributes

- **Languages**:
  - `makefile` (`Makefile`, `GNUmakefile`, `.mk`, `.mak`)
  - `dockerfile` (`Dockerfile`, `Containerfile`, `.dockerfile`)
- **LSP Servers**:
  - `docker-langserver --stdio` for Dockerfiles
  - `autotools-language-server` for Makefiles
- **Gutter Target Runners**:
  - Detects Makefile targets (`build:`, `test:`, `lint:`, `deploy:`, etc.) and adds one-click gutter `▶ make <target>` runners.
- **Snippets & Keymaps**: Multi-stage Dockerfile (`ruststage`) and `.PHONY` Makefile templates.

## Installing in Otto IDE

Install directly from the built-in **IDE Plugins → Catalog** (`catalog:makefile-docker-pack`) or from local path `examples/ide-plugins/makefile-docker-pack` with zero daemon/UI rebuild.
