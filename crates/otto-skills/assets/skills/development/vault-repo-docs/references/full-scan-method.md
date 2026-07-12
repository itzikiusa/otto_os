# Full-scan method

## Phase 1: establish the baseline

Record the absolute repository path, repository name, current commit, scan
timestamp, requested focus, and prior scan commit. Exclude generated/vendor
trees only when repository metadata or a well-known convention supports it.

## Phase 2: create a candidate inventory

Run `inventory_repo.py`, then manually inspect entrypoints and registration
sites. The inventory deliberately reports lexical candidates. A match is not a
fact until code reading confirms it; absence is not proof that a concept does
not exist.

Candidate kinds are `api`, `data`, `messaging`, `worker`, and `runtime`. Keep
the original stable ID and `path:line` evidence in `coverage.md`.

## Phase 3: reconcile before writing

Create a coverage table:

| Candidate | Kind | Evidence | Status | Document | Reason |
| --- | --- | --- | --- | --- | --- |
| `api:src/http.rs:42:post:/orders` | API | `src/http.rs:42` | documented | [Create order](endpoints/create-order.md) | Handler and DTO verified |

Allowed statuses:

- `documented`: a linked concept satisfies its completion contract.
- `generated`: generated source is authoritative and documented with provenance.
- `irrelevant`: confirmed false positive or outside an explicit focused scan.
- `uncertain`: evidence is incomplete or contradictory; never call the scan
  complete without disclosing these rows.

No candidate may disappear between inventory and completion. Do not mark a
row documented merely because an overview mentions its name.

## Phase 4: breadth, then depth

First create the bundle skeleton and one destination per confirmed concept.
Then complete APIs, stores, and runtime flows using their reference contracts.
This prevents early deep dives from hiding missed surfaces.

## Phase 5: deterministic audit and source verification

Run both OKF validation and `audit_repo_bundle.py`. Then re-read registration
sites, payload types, queries, migrations, triggers, retries, and tests. Update
incorrect claims and coverage rows before recording the final commit marker.

Incremental scans use the same phases but scope the candidate inventory to
changed files plus directly affected registration/contract files.
