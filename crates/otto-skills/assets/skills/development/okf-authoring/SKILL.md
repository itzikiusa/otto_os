---
description: Author, maintain, and consume documentation in OKF (Open Knowledge Format) — markdown + YAML frontmatter knowledge bundles. Use when writing docs into the Otto Vault, creating/updating a knowledge bundle, documenting a repo/service/dataset/decision, converting Obsidian/Notion notes to OKF, or validating a bundle. OKF is the documentation standard for the Otto Vault.
category: development
version: 1
---

# OKF — Open Knowledge Format authoring

OKF (spec v0.1, `GoogleCloudPlatform/knowledge-catalog/okf/SPEC.md`) represents
knowledge as **a directory of markdown files with YAML frontmatter**. No
registry, no required tooling: `cat` reads it, `git clone` ships it. The Otto
Vault treats every OKF-enabled vault as a bundle and validates it
deterministically.

## The format in one screen

- **Concept** = one `.md` file. **Concept ID** = path minus `.md`
  (`services/auth-api.md` → `services/auth-api`). No other ID scheme.
- **Frontmatter** (YAML between `---` lines):

| Field | Required | Rule |
|---|---|---|
| `type` | **YES — the only required field** | free-form kind: `Service`, `Metric`, `Playbook`, `Decision`, `Runbook`, `API Endpoint`, `BigQuery Table`, `Reference`, … |
| `title` | recommended | display name; falls back to filename |
| `description` | recommended | **exactly one sentence** — reused verbatim in `index.md` and search |
| `resource` | when applicable | canonical URI of the real asset; **omit for abstract concepts**; never a doc page URL (those go in Citations) |
| `tags` | optional | YAML list, short strings |
| `timestamp` | optional | ISO 8601, time of last meaningful change |

  Extra keys are allowed (`severity`, `method`, …). **Preserve unknown keys on
  every edit.**
- **Reserved files** (never concepts): `index.md` — directory listing of
  `* [Title](url) - description` bullets grouped under headings, **no
  frontmatter** (exception: the bundle-root `index.md` may carry only
  `okf_version: "0.1"`); `log.md` — newest-first history, `## YYYY-MM-DD`
  headings (ISO is a MUST), bullets like `* **Update**: … [concept](path)`.
  Lead words: `**Creation**`, `**Update**`, `**Deprecation**`,
  `**Initialization**`.
- **Links are standard markdown links** — `[customers](/tables/customers.md)`
  (bundle-absolute, preferred inside tools) or `[users](users.md)`
  (file-relative, preferred when GitHub renders the bundle). Never wikilinks
  in OKF output. Link *kind* lives in prose; a broken link is legal — it means
  not-yet-written knowledge.
- Conventional body headings: `# Overview`, `# Schema`, `# Examples`,
  `# Common query patterns`, `# Joins`, `# Metrics`, `# Citations` (numbered
  `[1] [Title](url)`); ADRs: `# Context` / `# Decision` / `# Consequences`.

## Conformance — never eyeball it

A bundle is conformant iff: (1) every non-reserved `.md` has parseable YAML
frontmatter; (2) every frontmatter has non-empty `type`; (3) reserved files
follow their structure. Everything else is a warning, never a rejection.

Validate with the vault: `otto_vault_okf_validate` MCP tool or
`POST /api/v1/workspaces/{ws}/vault/vaults/{id}/okf/validate`.
Errors (fix always): E1 no/unparseable frontmatter · E2 missing/empty `type` ·
E3 reserved-file structure. Warnings (fix when editing anyway): W1 missing
title/description · W2 broken internal link · W3 no timestamp · W4 directory
missing `index.md` · W5 log dates not ISO. Never "fix" a warning by inventing
facts.

## Authoring doctrine

1. **One sentence description.** It is the index entry and the search snippet.
2. **State the grain** for data assets: "One row per completed order." Add
   time range and caveats.
3. **Body order**: 1–3 paragraphs of prose → `# Schema` → examples/queries →
   `# Citations`. Favor tables/lists/fenced code over prose walls.
4. **Facts get their own reference docs.** A metric's formula lives ONCE in
   `references/metrics/<slug>.md`; tables link to it under `# Metrics`. Join
   paths live in `references/joins/<a>__<b>.md` (names alphabetical, double
   underscore) with the fenced `ON` clause. Never duplicate the SQL.
5. **Four-gate test before minting a reference doc**: nameable topic; not
   bundle-meta (skip overview/intro/getting-started/faq/changelog shapes);
   passes the citation test ("See the [X reference](…) for …"); ≥2 concepts
   would cite it. When in doubt, don't mint.
6. **Augment, don't rewrite.** When enriching an existing concept: every
   existing `#` heading survives in order and wording; extend prose, add
   bullets/`##` subsections, append new `#` headings at the end; union-merge
   `tags`; copy `type`/`title`/`resource` verbatim; refresh `timestamp`.
7. **Deprecate, don't delete.** Retiring knowledge = a `**Deprecation**` log
   entry + a note in the concept; removal only when truly wrong.
8. **Never invent** URLs, columns, joins, or enum values. Cite what you read.
9. **Cross-link discipline**: link a concept once per section at first
   mention; never from headings, fenced code, or schema field cells; never
   self-link.
10. **Update the neighborhood**: after creating/renaming concepts, refresh the
    directory's `index.md` (or run the vault's index generator) and append a
    dated `log.md` entry.

## Modes

- **produce** — new knowledge: choose the directory by domain (plural nouns:
  `services/`, `datasets/`, `decisions/`, `runbooks/`, `references/`…), write
  concepts from the template below, add indexes + log entry, validate.
- **maintain** — code/system changed: find affected concepts (by `resource`,
  path, topic), update bodies + `timestamp`, fix links, add concepts for new
  assets, log, validate. Touch every affected file in one pass.
- **consume** — need knowledge: read the root `index.md` first, follow links
  only into task-relevant concepts (progressive disclosure). If you learn
  something durable, switch to maintain and write it back.

## Concept template

```markdown
---
type: <Service | Metric | Playbook | Decision | …>
title: <Display name>
description: <One sentence.>
resource: <canonical URI — omit for abstract concepts>
tags: [tag, tag]
timestamp: 2026-07-11T10:00:00Z
---

# Overview

<What this is and why it matters. Grain/time-range for data assets.>

# Schema

| Field | Type | Description |
|-------|------|-------------|

# Citations

[1] [<source>](<url>)
```

## Converting existing notes

- **Obsidian**: `[[Note]]` → `[Note](./note.md)` (also `|alias` → link text,
  `#heading` → anchor); inline `#tags` → frontmatter `tags`; `![[embed]]` →
  link; callouts → blockquotes; MOC notes → `index.md`; add `type` (daily
  note→`Log`, permanent/literature→`Reference`, project→`Playbook`).
- **Notion**: properties→frontmatter (Name→title, Tags→tags, URL→resource,
  Last edited→timestamp); strip UUID suffixes from filenames/links.

## Otto wiring

- Write/update: `otto_vault_write` (path + full content). Read:
  `otto_vault_read`. Find: `otto_vault_search` (FTS + `tag:`/`type:` filters)
  or `otto_vault_dir`. Links: `otto_vault_backlinks`, `otto_vault_graph`.
- Regenerate indexes: `POST …/okf/indexes`.
- Always finish a produce/maintain session with `otto_vault_okf_validate` and
  fix every error before reporting done.
