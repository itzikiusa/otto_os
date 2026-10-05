# Integration Points Checklist

Fill in this template when producing a Story Architecture Overview. Replace each
`[ ]` item with a short answer and a file citation, or mark it `N/A — verified`
with evidence that you checked and it does not apply.

---

## Story

> (Paste story title / Jira key here)

---

## 1. Related repos and modules

List every codebase, service, or module this story touches or depends on.

| Name / path | Role in this story | In scope (yes/no/unclear) |
|---|---|---|
| | | |
| | | |

---

## 2. Functionalities touched

Existing features, endpoints, jobs, screens, or flows that are affected or reused.

- [ ] **Endpoint / handler:** `path/to/handler.go:line` — describe what changes
- [ ] **Service method:** `path/to/service.go:line` — describe what changes
- [ ] **Background job / ETL:** `path/to/job.go:line` — describe what changes
- [ ] **UI screen / route:** `path/to/page.svelte:line` — describe what changes
- [ ] **Shared utility / library:** `path/to/util.go:line` — describe what changes

---

## 3. Integration and contract points

Each boundary this story crosses. Mark each one as **additive** (safe) or
**breaking** (needs coordination with all consumers).

| Boundary type | Name / path | Change type | Consumers affected |
|---|---|---|---|
| REST endpoint | | additive / breaking | |
| Event / message schema | | additive / breaking | |
| Shared type / struct | | additive / breaking | |
| DB table / column | | additive / breaking | |
| Feature flag / config key | | additive / breaking | |
| External API | | additive / breaking | |

---

## 4. Data impact

- [ ] **New tables:** (list table names + migration file path)
- [ ] **Changed columns:** (column name, table, nature of change)
- [ ] **New indexes:** (column(s), table, estimated row count)
- [ ] **Migration required:** yes / no — file: `path/to/migration.sql`
- [ ] **Migration backward-compatible:** yes / no / unknown — explain if no
- [ ] **Backfill required:** yes / no — rows affected, estimated time
- [ ] **Retention / TTL:** applicable / not applicable — rule: ___

---

## 5. Technical risks

For each risk, give the category (from `references/risk-catalog.md`), description,
evidence (file path), and severity (high / medium / low).

| # | Category | Risk description | Evidence | Severity |
|---|---|---|---|---|
| 1 | | | | |
| 2 | | | | |
| 3 | | | | |

---

## 6. Exists today vs. this story adds

Be explicit for each item above. Use this table to summarize the delta.

| Item | Exists today (cite path) | This story adds / changes |
|---|---|---|
| | | |
| | | |

---

## 7. Open questions

Things the story implies that the code does not yet support, or gaps that would
block safe estimation.

- [ ] **Q1:** (question) — needs answer from: (PO / tech lead / platform team / ...)
- [ ] **Q2:** (question) — needs answer from: ___
- [ ] **Q3:** (question) — needs answer from: ___

---

## Evidence log

Brief record of what you searched and where. Reviewers use this to spot gaps.

| What you searched | Where you looked | Finding |
|---|---|---|
| | | |
| | | |
