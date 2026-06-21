# Skills Library

Otto ships a curated, **versioned library of first-party "skills"** — multi-file
instruction packs that teach an agent a method (how to grill a diff, hunt a
correctness bug, write a usage-insights report). The Skills Library in
**Settings → Skills** lets you browse that bundled catalog and **install or
update** each skill into your personal Otto library, with **drift detection**
(bundled-vs-installed `version:`) and a **never-overwrite-without-a-backup**
guarantee. Once installed, skills are materialized into agent sessions and drive
the **Review** lenses, **Product** analysis, and **Insights** reports — and the
self-improvement engine refines your installed copies from your real sessions.

This is the definitive end-user + operator guide. It documents what the code in
`crates/otto-skills/` and `ui/src/modules/settings/SkillsLibrary.svelte` actually
does — the on-disk install paths, the versioning/drift model, and how the catalog
relates to your own library entries.

> Related docs: **[Skills Evaluator](./skills-evaluator.md)** (an A/B harness that
> *evolves and promotes* skills — a separate feature), **[AI code review](./code-review.md)**
> (where `review`-category skills become one-click lenses), **[Product](./product.md)**
> (story analysis driven by `product`-category skills), and
> **[Self-improvement](./self-improvement.md)** (which edits your *installed* skill
> copies). The Skills Library is the **catalog + install primitive**; it is not the
> evaluator and not your own hand-authored skills (those live under
> **Settings → Context Library → Skills**).

---

## 1. Summary

| | |
|---|---|
| **What it is** | A bundled, versioned catalog of first-party skills + a drift-aware install/update tool. |
| **Where it lives** | **Settings → Skills** (root-only page). |
| **What a skill is** | A multi-file instruction pack (`SKILL.md` + `references/` + optional `assets/`/`scripts/`) that gives an agent a method. |
| **Bundled vs installed** | The catalog is **embedded in the daemon binary** (read-only); installing copies a skill into your **on-disk Otto library** where it becomes editable + self-improvable. |
| **Install target on disk** | `<data_dir>/library/skills/<name>/` — on macOS `~/Library/Application Support/Otto/library/skills/<name>/`. |
| **Versioning** | Each skill carries a `version:` integer in `SKILL.md` frontmatter; the catalog compares bundled vs installed and shows the drift state. |
| **Backups** | Updating an existing copy first copies it aside to `<data_dir>/library/skills-backup/<name>-<unix_secs>/` (default `backup=true`). Nothing is destroyed silently. |
| **Auto-install?** | **No.** Bundled skills are *never* auto-installed; they appear in the catalog and install only on explicit user action. (The separate `product`/`swarm` skill sets *are* auto-seeded — see §6.) |
| **Where installed skills are used** | Materialized into agent sessions; `review` skills → Review lenses, `product` skills → Product analysis, `insights` skill → Insights reports. |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1`. |
| **Crate / UI** | `crates/otto-skills/` (catalog + install) · `ui/src/modules/settings/SkillsLibrary.svelte`. |
| **Auth** | Listing the catalog is root; installing/updating/removing is **root-only**. |

---

## 2. Overview & where it lives

The Skills Library is two things glued together:

1. **The bundled catalog** — Otto's curated, first-party skills, *compiled into the
   `ottod` binary* (`include_dir!("$CARGO_MANIFEST_DIR/assets/skills")` in
   `crates/otto-skills/src/lib.rs`). This catalog is read-only and ships with the
   app; you cannot edit it in place.
2. **Your Otto library** — a real directory tree on disk
   (`<data_dir>/library/`) holding the skills, souls, and context you have
   installed/authored. *This* is what agents read at session spawn, and what the
   self-improvement engine edits.

Installing a bundled skill copies it (full multi-file tree) from #1 into #2. From
that moment it is **your copy** — editable, removable, refinable — and the catalog
tracks whether your copy still matches the bundled one.

### Where things live

| Thing | Path / location |
|---|---|
| Bundled catalog (read-only) | Embedded in the `ottod` binary; source at `crates/otto-skills/assets/skills/<category>/<name>/`. |
| Your Otto library root | `<data_dir>/library/` — macOS default `~/Library/Application Support/Otto/library/` (override via `$OTTO_DATA_DIR`). |
| Installed skill | `<data_dir>/library/skills/<name>/` (flat by name — see §5). |
| Backup of a replaced skill | `<data_dir>/library/skills-backup/<name>-<unix_secs>/`. |
| Souls / context (sibling stores) | `<data_dir>/library/souls/<name>.md`, `<data_dir>/library/context/<name>.md`. |
| Catalog UI | **Settings → Skills** (`ui/src/modules/settings/SkillsLibrary.svelte`). |
| Your-own-skills UI | **Settings → Context Library → Skills** (edit/remove arbitrary library skills). |
| Catalog routes | `GET /library/bundled`, `POST /library/bundled/{name}/install`, `POST /library/bundled/install-all`. |
| Library CRUD routes | `GET/PUT/DELETE /library/skills[/{name}]` (and souls/context). |

> **Data-dir note.** The library root is resolved by the daemon as
> `<data_dir>/library` (`crates/ottod/src/main.rs`), where `data_dir` is
> `$OTTO_DATA_DIR` or `~/Library/Application Support/Otto`
> (`crates/ottod/src/config.rs`). Everything below is relative to that root.

---

## 3. What a skill is (and where it's used)

A **skill** in Otto is a directory — not a single file. The required and optional
parts (see `crates/otto-skills/SKILL_AUTHORING.md`):

```
skills/<name>/
  SKILL.md            # required — YAML frontmatter + the method body
  references/         # the depth: checklists, catalogues, heuristics, good-vs-bad
  assets/             # output/report templates the skill fills in
  scripts/            # runnable helpers (a .sh gets +x on install)
```

`SKILL.md` opens with frontmatter that the catalog parses (`frontmatter_value` in
`crates/otto-skills/src/lib.rs`):

```yaml
---
description: <one or two sentences — what it does + when to use it (the selector signal)>
category: review            # product | project | development | review | design | insights
version: 1                  # integer; bump on content change → drives drift detection
---
```

There is **no `name:` field** — the directory name *is* the skill's globally-unique
name (the library is flat by name; `kebab-case`).

**What skills do.** A skill is *advisory* guidance an agent reads — a method, not a
hook. At session spawn Otto materializes the active skills into the agent's working
copy (for `claude`, under `.claude/skills/`), so the agent can load `SKILL.md`, pull
in `references/` as needed, run `scripts/`, and fill `assets/` templates. Concretely
they power three features:

- **Review** — the `review` category becomes one-click PR/local review **lenses**
  (`grill`, `correctness-review`, `security-review`, …). See §7.
- **Product** — `product`-category skills (story overview, clarifying questions,
  architecture overview, test cases, PRD/RFC, task breakdown) are the prompts the
  Product analysis fan-out runs. See §7.
- **Insights** — the `insights` skill drives the daily/weekly/monthly usage-insights
  report. See §7.

> **Advisory vs enforced.** Skills (and instruction files like `CLAUDE.md`/`AGENTS.md`)
> are labeled **advisory** in the context preview — the model reads them and *may
> ignore* them. Only runtime hooks/settings (`.claude/settings.local.json`) are
> **enforced** by the daemon. Skills shape behavior; they do not constrain it. See
> the context-preview shapes in `docs/contracts/api.md`.

---

## 4. Browsing the bundled catalog

Open **Settings → Skills** (root-only). The page loads `GET /library/bundled`
(`contextApi.listBundled()`), which returns every bundled skill plus its drift
state against your installed copy.

Skills are shown **grouped by category**, in this preferred order, then
alphabetically for anything else (`SkillsLibrary.svelte`):

```
product → project → development → review → design → insights
```

Each row shows the skill's **`name`** (monospace), a **state chip**, and the
**`description`** (the selector signal). A per-category **"Install all in category"**
button appears in each section header; it is disabled when every skill in that
category is already up to date.

### State chips

The chip text/color is driven by the per-skill `state` (`badge()` in the UI):

| `state` | Chip | Meaning |
|---|---|---|
| `not_installed` | **Not installed** | No copy in your library. |
| `up_to_date` | **Installed v{n}** (ok) | Installed at the same version as the bundle. |
| `update_available` | **Update available v{i}→v{b}** (accent) | Installed, but the bundle is newer. |
| `ahead` | **Edited (ahead)** (warn) | Your installed copy's version is ≥ the bundle — it was hand-edited / is ahead. **Never auto-touched.** |

For `update_available` the row notes "Your installed copy is backed up first." For
`ahead` it warns that updating backs up then replaces your edited copy, and that
*doing nothing keeps your edited copy*.

### What ships today (catalog snapshot)

The bundled crate (`crates/otto-skills/assets/skills/`) currently ships:

| Category | Skills (version) |
|---|---|
| **review** | `grill` (v1), `correctness-review` (v1), `security-review` (v1), `performance-review` (v1), `architecture-review` (v1), `test-review` (v1), `devex-review` (v1) |
| **insights** | `insights` (v2) |

> **Catalog vs target.** `SKILL_AUTHORING.md` describes a five-category *target*
> catalog (`product | project | development | review | design`) with many more
> skills (`tdd`, `triage`, `codebase-design`, …). Those are an authoring roadmap;
> the **`review` + `insights` skills above are what the bundled crate ships**. The
> seven **`product`** skills are *not* part of this catalog — they ship in the
> `otto-product` crate and are **auto-seeded** into your library on daemon start
> (see §6), so they show up in your Context Library and drive Product analysis
> without ever appearing on the **Settings → Skills** catalog page.

---

## 5. Installing & updating

### Installing one skill

Click **Install** (state `not_installed`) or **Update** (state `update_available`)
on a row. The UI calls `POST /library/bundled/{name}/install`
(`contextApi.installBundled(name)`); the backend:

1. If a copy already exists **and** `backup=true` (the default), copies the existing
   tree to `<data_dir>/library/skills-backup/<name>-<unix_secs>/` (`install_one` →
   `copy_tree` in `crates/otto-skills/src/http.rs`).
2. Removes the old installed dir and **copies the full bundled tree** into
   `<data_dir>/library/skills/<name>/` — `SKILL.md`, `references/`, `assets/`, and
   `scripts/` (a `.sh` gets the executable bit on Unix) (`install_into` → `seed_dir`
   in `crates/otto-skills/src/lib.rs`).

The response (`InstallResult`) reports `{ name, installed, backed_up, backup_path }`.
The UI toasts **"Backed up & updated"** with the backup path when a copy was
replaced, or **"Installed"** otherwise, then reloads the catalog.

### Updating an "ahead" (edited) skill

If your installed copy is **`ahead`** — its version is ≥ the bundle, meaning you (or
self-improvement) edited it — the UI **first asks for confirmation**: *"Updating
backs it up first, then replaces it with the bundled skill. Do nothing to keep your
edited copy."* (`confirmer.ask`, danger style, **"Back up & replace"**). This is the
explicit keep-old-vs-sync choice; the bundle is never silently pushed over your edits.

### Install all in a category

The section header's **"Install all in category"** runs `POST /library/bundled/install-all?category=<cat>`
(`contextApi.installAllBundled(category)`). It iterates every bundled skill in that
category, backing up + installing each (still `backup=true` by default), and returns
`InstallAllResult { installed: string[], backed_up: string[] }`. The toast reports
how many were installed and how many existing copies were backed up; if nothing
changed it says *"All `<cat>` skills are already up to date."*

### Versioning & drift model

- **Bundled version** — the `version:` integer in the embedded `SKILL.md`
  (`bundled_version`). Authors bump it whenever the skill's content changes.
- **Installed version** — the `version:` parsed from
  `<data_dir>/library/skills/<name>/SKILL.md` (`installed_version`); a present-but-
  unparseable/absent version defaults to `1`.
- **State** is computed by comparing the two (`install_state`):

| Condition | State |
|---|---|
| not on disk | `NotInstalled` |
| installed == bundled | `UpToDate` |
| installed < bundled | `UpdateAvailable { installed, bundled }` |
| installed ≥ bundled | `Ahead { installed, bundled }` |

There is **no automatic update** — a newer bundle only *surfaces* an
"Update available" chip; you decide whether to sync. This is deliberate: your
installed copy is the source of truth that agents read and that self-improvement
edits, so the catalog never reaches in and overwrites it on its own.

> **Diff/preview.** The catalog does not show a line-level bundled-vs-installed
> diff. Drift is expressed as the version delta (`v{i}→v{b}`). To inspect actual
> content before updating, read the installed `SKILL.md` (Context Library) or rely
> on the automatic backup — the previous copy is preserved under `skills-backup/`
> so any update is reversible by hand.

---

## 6. The Context Library (skills / souls / context)

The Skills Library catalog feeds into the broader **Context Library**, the on-disk
store under `<data_dir>/library/` (`crates/otto-context/src/library.rs`) with three
sibling kinds:

| Kind | On disk | What it is |
|---|---|---|
| **Skills** | `skills/<name>/SKILL.md` (+ refs/assets/scripts) | Methods (the catalog installs into here). |
| **Souls** | `souls/<name>.md` | Agent personas/voice; one is the **default soul** (`default-soul.txt`). |
| **Context** | `context/<name>.md` | Reusable context snippets injected into sessions. |

Entry names are validated as single safe path segments (alphanumeric / `-` / `_`,
not `.`/`..`) to prevent path traversal. Library reads/writes are **root**; the
per-workspace *selection* of which skills/soul/context to use is workspace-scoped.

**How it reaches a session.** At session spawn Otto **materializes** the workspace's
active context for the target provider (`crates/otto-context/src/materialize.rs`,
`provision`). Multi-file library skills are copied verbatim; legacy single-body
skills are written as a lone `SKILL.md`. A workspace can use *all* library skills
(default) or an explicit subset. You can **preview** exactly what a spawn would
write — every file, the chosen soul, generated `CLAUDE.md`/`AGENTS.md`, and hooks —
without spawning anything, via `POST /workspaces/{id}/context/preview` (a byte-for-
byte dry-run of the real `plan()`; see `docs/contracts/api.md`).

**Seeded (auto-installed) skills — distinct from the catalog.** Two crates seed
their own skills into this library on daemon start, *write-if-absent / version-gated*
so your edits are never clobbered on routine restart:

- **`otto-product`** seeds 7 product skills (`po-story-overview`,
  `story-clarifying-questions`, `story-architecture-overview`, `story-test-cases`,
  `jira-story-writer`, `rfc-writer`, `story-task-breakdown`), gated by a
  `.product-skills-version` marker (`crates/otto-product/src/skills.rs`).
- **`otto-swarm`** seeds role skills + preset souls.

These appear in your Context Library (editable, self-improvable) but **not** in the
**Settings → Skills** catalog page — that page only lists the `otto-skills` bundle.

---

## 7. How skills drive Review, Product & Insights

### Review lenses (`review` category)

The **Review** tab (`ui/src/modules/git/ReviewPanel.svelte`) builds its one-click
review lenses **data-drivenly from your installed `review`-category skills**. Each
installed `review` skill becomes a preset whose focus is *"Apply the `<skill>` skill
(it is available to you): <description>"*, and selecting it spawns a real review
agent **with the skill materialized in-session**, so the agent follows the skill's
full method (e.g. `grill`'s twelve-pass adversarial sweep). The panel also runs a
**pre-check**: if `review` skills are *missing* or have a *newer bundled version*, it
nudges you toward **Settings → Skills** to install/update them. (See
**[code-review.md](./code-review.md)**.)

### Product analysis (`product` category)

The Product feature runs a per-skill agent fan-out. For each analysis lens it
resolves the skill body **library-first, then the bundled `otto-product` body, then
empty** (`crates/otto-server/src/product_run.rs`: `context_library.get_skill(...)`
`.or_else(|| otto_product::skill_body(...))`), builds the prompt from that skill +
context + an output contract, and runs it as a real session. So editing the installed
copy of, e.g., `po-story-overview` changes how Product analysis behaves. (See
**[product.md](./product.md)**.)

### Insights (`insights` category)

The daily/weekly/monthly usage-insights run materializes and follows the `insights`
skill (`crates/otto-skills/assets/skills/insights/insights/`) — its
`scripts/collect_insights.py`, `references/`, and `assets/report-skeleton.html`
produce the HTML report. Install/update it from the catalog the same way as any
other skill.

---

## 8. Self-improvement refines your installed skills

Otto's self-improvement engine (`otto-improve`) watches your sessions and proposes
edits to skills. Crucially, it **targets the installed *library* copy**, not the
bundled source: `resolve_target(..., ImprovementTarget::Skill, name, Some(library_root))`
prefers `<data_dir>/library/skills/<name>/SKILL.md` when it exists
(`crates/otto-improve/src/pathsafe.rs`, `engine.rs`). The library is the source of
truth.

Two consequences for the catalog:

1. A self-improvement edit **bumps your installed copy's content** (and may bump its
   `version:`), which can flip the skill's catalog state to **`ahead`** — exactly the
   "your copy was edited" case in §5. The catalog will then refuse to silently push a
   bundled update over it.
2. Reinstalling/updating from the catalog **discards** those refinements (after the
   automatic backup). The confirmation dialog in §5 exists precisely so you choose
   between keeping the refined copy and resyncing to the bundle.

See **[self-improvement.md](./self-improvement.md)** for how edits are proposed,
queued, and applied (and the autonomy levels that gate auto-apply).

---

## 9. API / contract reference

Catalog endpoints (mounted at `/api/v1`, from `crates/otto-skills/src/http.rs`;
contract: `docs/contracts/api.md`, "Bundled skills"):

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `GET /library/bundled` | root | — | `BundledView[]` — the catalog + per-skill drift state |
| `POST /library/bundled/{name}/install` | root | `?backup=<bool>` (default `true`) | `InstallResult` |
| `POST /library/bundled/install-all` | root | `?category=<cat>&backup=<bool>` | `InstallAllResult` |

Library CRUD used by the catalog UI (Remove) and the Context Library
(`docs/contracts/api.md`, "Context library"):

| Method & path | Auth | Notes |
|---|---|---|
| `GET /library/skills` | root | List installed library skills (`SkillEntry[]`). |
| `GET /library/skills/{name}` | root | Read an installed skill body. |
| `PUT /library/skills/{name}` | root | Write/edit an installed skill. |
| `DELETE /library/skills/{name}` | root | **Remove** an installed skill (the catalog's "Remove" button → `contextApi.deleteSkill`). |
| `GET/PUT/DELETE /library/souls[/{name}]`, `…/default-soul` | root | Souls store. |
| `GET/PUT/DELETE /library/context[/{name}]` | root | Context store. |
| `POST /workspaces/{id}/context/preview` | ws viewer | Byte-for-byte dry-run of what a spawn materializes. |

### Wire shapes

```ts
// GET /library/bundled → BundledView[]
interface BundledView {
  name: string;
  category: string;          // product | project | development | review | design | insights
  version: number;           // bundled version
  description: string;       // selector signal
  installed_version: number | null;             // null when not installed
  state: 'not_installed' | 'up_to_date' | 'update_available' | 'ahead';
}

// POST /library/bundled/{name}/install → InstallResult
interface InstallResult {
  name: string;
  installed: boolean;        // false only when {name} is not a bundled skill
  backed_up: boolean;
  backup_path: string | null; // …/library/skills-backup/<name>-<unix_secs>/
}

// POST /library/bundled/install-all → InstallAllResult
interface InstallAllResult {
  installed: string[];
  backed_up: string[];
}
```

Errors use the standard `Problem { code, message }` shape: `404` when `{name}` is
not a bundled skill, `403` (`"installing skills requires root"`) for a non-root
caller on an install route.

---

## 10. Capabilities & limitations

**Capabilities**
- One curated, version-stamped, multi-file skill catalog embedded in the app.
- Per-skill and per-category install/update with drift detection.
- Automatic backup of any replaced copy (reversible by hand).
- Installed skills are real, editable files that drive Review/Product/Insights and
  are refined by self-improvement.
- Catalog respects user/AI edits: an "ahead" copy is never silently overwritten.

**Limitations**
- **No bundled content diff.** Drift is a version delta, not a line diff.
- **No catalog rollback button.** Reverting a bad update means manually restoring
  from `skills-backup/<name>-<ts>/` (the backups are never auto-pruned, so they can
  accumulate).
- **No "create a bundled skill" from the UI.** The catalog is fixed at build time;
  authoring new bundled skills is a code change (`crates/otto-skills/assets/skills/`,
  per `SKILL_AUTHORING.md`). You *can* author arbitrary **library** skills via
  `PUT /library/skills/{name}` (Context Library), but those aren't part of the catalog.
- **Catalog ≠ full target list.** Only the `review` + `insights` skills ship in the
  bundle today; `product` skills are auto-seeded by a different crate; the broader
  five-category list in `SKILL_AUTHORING.md` is a roadmap.
- **Skills are advisory.** They guide the model and can be ignored; only hooks are enforced.

---

## 11. Security & permissions

- **Root-only mutations.** `require_root` gates every install route
  (`crates/otto-skills/src/http.rs`); a non-root caller gets `403`. The policy layer
  also classifies these (`crates/otto-server/src/policy.rs`): `GET /library/bundled`
  is a **View**-level read, the install/install-all POSTs are **Edit**-level, and the
  rest of `/library/*` is library-admin. The **Settings → Skills** page is gated to
  root in the UI as well.
- **Never overwrite without a safety copy.** The backend *always* keeps the backup
  (`backup=true` default); the UI obtains explicit consent for the destructive
  "ahead" replace. This is the "do not damage user work" guarantee in code.
- **Path-traversal safe.** Skill names are validated as single safe segments
  (`is_safe_segment` in `library.rs`); names like `..` or with slashes are rejected,
  so installs/edits can't escape the library tree.
- **No secrets in skills.** Skills are plain Markdown + scripts checked into the
  repo/library; they carry no tokens. Keychain-backed secrets are never part of a
  skill.
- **Local-only surface.** All routes are served by `ottod` on loopback
  (`127.0.0.1:7700`) unless you explicitly enable a network listener.

---

## 12. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| **"No bundled skills"** empty state | Catalog failed to load, or the build shipped no skills. Confirm `ottod` is running and you're authenticated as root; reload the page. |
| Install/Update button → **403 / "installing skills requires root"** | You are not a root user. Catalog mutations are root-only; sign in as root. |
| Skill stuck at **"Edited (ahead)"** and won't auto-update | By design — your copy was hand-edited or refined by self-improvement. Click **Update → "Back up & replace"** to sync to the bundle (your copy is backed up first), or leave it to keep your edits. |
| Update **lost my hand-edits** | Expected — Update overwrites. Restore from `<data_dir>/library/skills-backup/<name>-<unix_secs>/`. |
| A `review` skill **doesn't appear as a lens** in the Review tab | It isn't *installed* (only installed `review`-category skills become lenses), or its `category` isn't `review`. Install it from **Settings → Skills**; the Review tab pre-check flags missing/outdated ones. |
| Review lens "out of date" pre-check banner | A `review` skill has a newer bundled version. Go to **Settings → Skills** and Update it. |
| `product` skills aren't on the **Settings → Skills** page | Correct — they're auto-seeded by `otto-product`, not part of this catalog. Find/edit them under **Settings → Context Library → Skills**. |
| Edited a skill's `version:` but state didn't change | The catalog re-reads on reload; refresh the page. An unparseable `version:` is treated as `1`. |
| A skill's `scripts/*.sh` isn't executable | The executable bit is set on install on Unix only; reinstall the skill, or `chmod +x` it manually. |
| `skills-backup/` is growing large | Backups are never auto-pruned. Delete old `skills-backup/<name>-<ts>/` dirs you no longer need. |

---

## 13. Related docs

- **[Skills Evaluator](./skills-evaluator.md)** — the A/B harness that *evolves and
  promotes* skills into the library (`POST /skill-evaluations/.../promote`). Distinct
  from this catalog: the evaluator generates/scores candidate skills; the Skills
  Library installs *bundled, first-party* ones.
- **[AI code review](./code-review.md)** — how installed `review`-category skills
  become one-click review lenses.
- **[Product](./product.md)** — how `product`-category skills drive story analysis.
- **[Self-improvement](./self-improvement.md)** — how your *installed* skill copies
  are refined from real sessions (and why that flips skills to "Edited (ahead)").
