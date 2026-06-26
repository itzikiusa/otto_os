# Agent context injection (out-of-tree bundles)

How Otto delivers a workspace's **soul, skills, memory, repo-rules and activity
hooks** to an agent CLI **without writing anything into the user's working tree**,
and the **checklist for adding a new provider**.

> TL;DR — at spawn, Otto materializes the active context into an Otto-owned
> *bundle* under `~/.otto/context/<provider>/<enc(cwd)>/` and appends the
> per-CLI **launch flags** that make the CLI load it (`--add-dir`,
> `--append-system-prompt-file`, codex `-c developer_instructions=…`). The repo
> is never touched.

---

## Why out-of-tree

Otto used to merge an `<!-- OTTO:START -->…<!-- OTTO:END -->` region into the
working-tree `CLAUDE.md` / `AGENTS.md` and write `.claude/skills/` into the repo.
For a repo that **tracks** those files (Otto's own, for one) the injection showed
up as a git diff and could be committed by accident, and the full-skill dump blew
past codex's hard **150 000-char `AGENTS.md` limit**. The bundle approach keeps the
repo pristine and scales with the skill library.

## Architecture

```
SessionManager.create()                      crates/otto-sessions/src/manager.rs
  └─ PreSpawnHook::before_spawn(ws,cwd,prov)  crates/otto-core/src/hooks.rs
        └─ Provisioner (otto-context)         crates/otto-context/src/provisioner.rs
             └─ materialize::provision(…, ctx_root)   crates/otto-context/src/materialize.rs
                  • writes the bundle under ~/.otto/context/<provider>/<enc(cwd)>
                  • returns SpawnInjection { args, env }
  └─ spec.args.extend(injection.args); spec.env.extend(injection.env)
```

- **`SpawnInjection { args, env }`** (`crates/otto-core/src/hooks.rs`) — the launch
  flags + env the CLI needs to load the bundle. The manager appends them to the
  spawn command.
- **`before_spawn`** runs on a fresh `create()`; **`resume_injection`** runs on
  `restart()` (resume) — it has no `Workspace`, so it just reads the persisted
  bundle and reconstructs the same flags. Both append to the spawn `spec`
  (`manager.rs` `create()` and `restart()`).
- The bundle path is **deterministic** (`<ctx_root>/<provider>/<enc(cwd)>`) so
  resume needs no re-materialize. `ctx_root` defaults to `~/.otto/context`
  (`materialize::default_context_root`) and is injectable for tests.

## Per-provider reference (verified against the live CLIs, 2026-06-26)

| Provider | Context delivery | Context file in bundle | Skills dir in bundle | Activity hooks | Launch flags Otto adds |
|---|---|---|---|---|---|
| **claude** (2.1.x) | `--append-system-prompt-file` (— `--add-dir` does **not** load `CLAUDE.md`) | `CONTEXT.md` | `.claude/skills/<name>/SKILL.md` (first-class, auto-loaded by `--add-dir`) | `settings.json` via `--settings` | `--add-dir=<b>` `--append-system-prompt-file=<b>/CONTEXT.md` `--settings=<b>/settings.json` |
| **agy** (Antigravity Gemini 1.0.x) | `--add-dir` **auto-loads `AGENTS.md`** | `AGENTS.md` | `.agents/skills/<name>/SKILL.md` (first-class) | none | `--add-dir=<b>` |
| **codex** (OpenAI 0.142.x) | `-c developer_instructions=<text>` (**no** flag loads an out-of-tree instructions *file*) | `CONTEXT.md` (carries a skills **index**) | `skills/<name>/SKILL.md` (read **on demand** via the index) | none | `--add-dir=<b>` `-c developer_instructions=<CONTEXT.md text>` |

Notes that bit us, so you don't have to rediscover them:
- claude's `--help` *claims* `--add-dir` loads "CLAUDE.md dirs" — it does **not**.
  Use `--append-system-prompt-file` (the *append* form, so Claude's own default
  system prompt survives).
- codex `--add-dir` is **sandbox-writability only**; it grants read access to the
  skill files the index points at, but carries no instructions. codex reads
  `AGENTS.md` only from the cwd→git-root tree and global `~/.codex`. A `CODEX_HOME`
  swap *can* deliver first-class skills + a global `AGENTS.md` but needs an
  `auth.json` symlink + `config.toml` copy and rewires session-id capture — we
  deliberately chose the simpler `developer_instructions` channel.
- agy reads `AGENTS.md`/`GEMINI.md` from an added dir but **not** `CLAUDE.md`, and
  scans `.agents/skills` — **not** `.claude/skills`.

---

## How to add a new provider

Work through this checklist. The two code touch-points are
**`crates/otto-sessions/src/providers.rs`** (how to *launch* it) and
**`crates/otto-context/src/materialize.rs`** (what *bundle* to build + which
*flags* to inject). Nothing else needs editing for context injection.

### 1. Probe the CLI first (don't trust the `--help`)

Before writing any code, empirically answer these against the real binary — the
help text lies (claude's did). Build a probe bundle in `/tmp` with sentinel
files and run the CLI from a **neutral, empty cwd** so you know the context can
only have come from your flags:

- **Instruction/memory file:** which filename does it auto-load from an added
  dir, and via which flag? Try `--add-dir`, `--append-system-prompt-file`, a
  `-c key=value` config, or a `*_HOME` env. Put a sentinel line in candidate
  files (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, `CONTEXT.md`) and ask the model to
  echo any sentinel it sees. **Use a behavioral test** (e.g. "begin every reply
  with `MARK:`") — the model *reading* a file with a tool is not the same as the
  file being *loaded into context*.
- **Skills dir:** where does it scan for `SKILL.md`? (`.claude/skills`,
  `.agents/skills`, a plain `skills/`, or only a global `~/.<tool>/skills`?) Are
  skills **first-class** (auto-registered/triggerable) or only readable on demand?
- **Hooks/settings:** does it support activity hooks, and how are they supplied
  out-of-tree (a `--settings <file>`-style flag)?
- **Arg form:** does it accept the `--flag=value` (equals) form? Otto emits
  equals-form. (claude/agy/codex all do.)

### 2. Register the launch in `providers.rs`

Add a `ProviderSpec` to `ProviderRegistry::build_map`
(`crates/otto-sessions/src/providers.rs`): `cmd`, baseline `args`, `resume_args`
(use the `{sid}` / `{cwd}` templates), `update_command`, and `captures_session_id`
(`true` only if the CLI mints its own session id and Otto must scrape it from disk
after spawn — see codex/agy; `false` if Otto assigns it via a launch flag). **Do
not** hard-code the context flags here — those come from the injection in step 4.

If the CLI mints its own id, also teach the post-spawn capture
(`spawn_session_id_capture` in `manager.rs`) where its rollout/conversation file
lives.

### 3. Decide the bundle layout in `materialize.rs`

Add the provider to the three small helpers:
- **`plan()` dispatch** — add the name to the `"claude" | "codex" | "agy"` match
  arm so it routes to `plan_provider`.
- **`context_file_name(provider)`** — the bundle filename for the context block
  (`AGENTS.md` if the CLI auto-loads that name, else `CONTEXT.md`).
- **`skills_subdir(provider)`** — where the CLI scans skills inside the bundle
  (`.claude/skills` / `.agents/skills` / `skills`).
- In `plan_provider`, choose the **`SkillIndex`**: `SkillIndex::None` when skills
  are **first-class auto-loaded** (the block omits them); `SkillIndex::Codex { … }`
  when they're **read on demand** (the block emits a name+description+path index).
  Never inline full skill bodies — that is what broke the 150k limit.
- Set `hooks` only if the CLI consumes activity hooks (currently claude-only via
  `plan_claude_hooks`).

### 4. Emit the launch flags in `injection_for`

Add a match arm to **`injection_for(provider, dir)`** returning the
`SpawnInjection` your step-1 probe proved works — e.g. `--add-dir=<dir>` plus
whatever loads the context (`--append-system-prompt-file=…`, `-c key=value`, or an
env var in `SpawnInjection.env`). Guard file-pointing flags with `is_file()` so a
missing bundle file never makes the CLI error. This same function is reused on
resume, so it must be reconstructable from the on-disk bundle alone.

### 5. (Optional) surface it in preview

Add the provider to the default lists in the materialize/preview endpoints
(`crates/otto-context/src/http.rs`) so the Context preview UI includes it.

### 6. Verify — unit + a real session

- **Unit:** add a test like `provision_never_writes_into_the_working_tree` and a
  per-provider test asserting the bundle layout + the exact `injection.args`
  (see the `tests` module in `materialize.rs`). Tests pass a temp `ctx_root` so
  they never touch the real `~/.otto`.
- **End-to-end (required):** materialize a real bundle and launch the CLI with the
  **exact `=`-form args Otto generates**, from a neutral cwd, and confirm the
  context sentinel and a probe skill are visible. Recipe:

  ```bash
  # build a bundle mirroring Otto's layout for <provider>, then:
  cd /tmp/neutral && <cli> <Otto's exact injection args> -p \
    'Echo the OTTO_E2E sentinel from your context; is skill otto-probe available?'
  ```

  This catches the gaps unit tests can't: equals-form arg parsing, interactive vs
  `-p` differences, and whether skills are actually *registered* vs merely
  readable.

### What you do NOT need to touch

`manager.rs` (create/restart already append the injection), `hooks.rs`
(`SpawnInjection` is generic), and the spawn plumbing are provider-agnostic. The
**1 000-line cap** is also automatic: every provider's block flows through
`build_block` → `enforce_line_budget`, so a new provider inherits the budget
without extra code. If your provider needs an **env var** instead of a flag (e.g.
a `*_HOME` swap), return it in `SpawnInjection.env` — the manager already extends
`spec.env` with it.

---

## Capabilities & limitations

- **No working-tree writes.** Verified by `provision_never_writes_into_the_working_tree`.
  The repo's own `CLAUDE.md`/`AGENTS.md` stay as the user authored them and are
  still read by the CLI as legit project docs (codex still honors an in-repo
  `AGENTS.md`).
- **Capped at 1 000 lines.** The assembled block Otto injects into *every* session
  is hard-capped at `MAX_CONTEXT_LINES` (1 000) by `enforce_line_budget` in
  `materialize.rs`, so a growing memory/soul/repo-rules set can never bloat the
  agent's context window. Over budget, the lowest-priority tail (memory → repo
  rules → context) is dropped first — soul and the skills index at the head always
  survive — and a one-line marker records what was trimmed. The cap is asserted by
  `provisioned_bundle_never_exceeds_the_line_budget` + `enforce_line_budget_caps_oversized_blocks`,
  so it's enforced on every `cargo test` run (the commit/CI gate). This bounds
  **only** Otto's injected block — a user's own in-repo `CLAUDE.md`/`AGENTS.md` is
  theirs and uncapped.
- **Resume** reuses the persisted bundle (no re-materialize, no `Workspace`
  needed). If skills/soul changed between spawn and resume, the resumed session
  keeps the spawn-time bundle — consistent with prior behavior.
- **codex skills are read-on-demand**, not first-class — by design (codex never
  had first-class Otto skills; first-class would require a `CODEX_HOME` swap).
- **Shell** and unknown providers are skipped (empty injection).
- The bundle is **per `(provider, cwd)`**, shared across workspaces that open the
  same cwd; the last spawn's context wins (matches the old cwd-write behavior).
