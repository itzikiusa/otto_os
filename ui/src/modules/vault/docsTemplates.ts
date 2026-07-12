// Prepared prompts for the Docs agent — curated, parameterized prompt bodies
// the user picks instead of writing a request from scratch. Each template can
// require a repo path (substituted into the body) and declares the library
// skills the run should stage (server: RunReq.skills → staged for claude,
// inlined for codex/agy).
//
// Authoring principles (learned from real runs that came out too thin):
// - Demand an explicit FLOW INVENTORY first, then ONE NOTE PER FLOW — "cover
//   everything" alone yields a single shallow overview note.
// - State the anti-bloat bar in the same breath: dense tables/diagrams/
//   examples, never prose-padding — 1M lines of code must NOT become 500k
//   lines of docs nobody reads.
// - Require file-path citations so claims trace to code, not imagination.

export interface DocsTemplate {
  id: string;
  label: string;
  /** One-liner shown under the picker. */
  hint: string;
  /** Library skills the run stages (visible as chips in the form). */
  skills: string[];
  /** Whether the template needs a repository path. */
  needsRepo: boolean;
  build: (repo: string) => string;
}

/** The shared quality bar — appended to every repo template. */
const QUALITY_BAR = `
Quality bar (read twice):
- ELABORATE but NOT BLOATED: every sentence must carry information a reader acts on. Prefer tables, real examples and diagrams over paragraphs. No boilerplate, no restating code line-by-line — turning 1M lines of code into 500k lines of docs is failure; nobody reads that.
- A note a reader can't consume in ~5 minutes must be split or tightened.
- Every factual claim cites the source file path (backticked, relative to the repo).
- Realistic examples lifted from code/tests/fixtures — sanitize secrets.
- Cross-link liberally with markdown links; every note must be reachable from the bundle's index.md.

Diagrams (rendered live in the vault — a broken diagram shows as a red parse error, so VERIFY each one):
- Flows/sequences: mermaid fences (flowchart / sequenceDiagram). Data models: PREFER d2 fences with its sql_table shape (one table per store entity, columns + types) — d2 is stronger for schemas.
- Mermaid safety rules (these cause most parse failures): quote any label containing (){}[]|@:; or double underscores — write A["SetNX AUTO_LOGOUT_{playerId}"], never bare; one statement per line; no raw HTML/markdown inside labels; keep node ids alphanumeric.
- VERIFY before moving on: re-read every fence you wrote and mentally parse it line by line (balanced brackets/quotes, every edge references a defined node, first line is a valid diagram header). Rewrite or simplify anything doubtful — a simpler diagram that renders beats a rich one that errors.`;

const FLOW_RULES = `
FLOWS — the core requirement:
1. FIRST enumerate ALL flows by reading the code: every API operation, every consumed AND produced message, every scheduled worker / reconciliation cycle, every startup/shutdown routine. List them in flows/index.md as a table (flow → trigger → one-line purpose).
2. Then write ONE NOTE PER FLOW under flows/ — ALL of them, no sampling. Each flow note: trigger, step-by-step path through the code (with file citations), data read/written per step, side effects outside the service, failure modes + retry/idempotency semantics, and ONE mermaid sequence or flowchart diagram.`;

export const DOCS_TEMPLATES: DocsTemplate[] = [
  {
    id: 'repo-full',
    label: 'Repo deep-dive (full documentation)',
    hint: 'Complete linked bundle: overview, one note per flow, API+OpenAPI, messaging, workers, datastores, side effects.',
    skills: ['vault-repo-docs'],
    needsRepo: true,
    build: (repo) => `Document the repository at ${repo} into this vault as a complete, linked bundle in a folder named after the repo.

Ground rules: read the actual code FIRST; never invent endpoints, topics, tables or workers — if it is not in the code, it is not in the docs.
${FLOW_RULES}

Deliverables (all cross-linked; index.md links everything):
1. overview.md — purpose, place in the platform, runtime topology, upstream/downstream dependencies, config surface. Mermaid context diagram (callers, callees, datastores, brokers).
2. flows/ — the flow inventory + one note per flow (see above). This is the heart of the bundle.
3. api.md — the FULL HTTP surface as a table (method, path, auth, purpose, → link to its flow note). ALSO write a complete OpenAPI 3 spec to api-openapi.yaml: real request/response schemas and at least one realistic example per operation — not stubs.
4. messaging.md — EVERY producer and consumer: topic/queue name, when it fires, full example payloads (incoming AND outgoing), delivery/retry/poison semantics, → flow links.
5. workers.md — EVERY scheduled job and reconciliation: schedule, scan scope, what it fixes, idempotency, failure behavior, → flow links.
6. data.md — EVERY datastore the code touches, grouped (brand MySQL, MS SQL, MongoDB, ClickHouse, Redis brand/MS). Per table/collection/key pattern: every column/field (name, type, purpose) AND the code paths that read or write it (file citations) — the goal is IMPACT ANALYSIS: from any column, a reader must see what a change to it would affect. d2 sql_table diagram per relational store.
7. side-effects.md — one table: operation → everything it changes outside this service (calls to other services, events emitted, cache invalidations, balance mutations).
${QUALITY_BAR}

Scan marker (REQUIRED — incremental updates depend on it): record in overview.md's frontmatter the absolute repo path, the current git commit hash (\`git rev-parse HEAD\` in the repo), and the scan date, e.g. \`repo:\`, \`commit:\`, \`scanned_at:\`.

Finish: verify every link resolves and flows/index.md covers every flow you enumerated.`,
  },
  {
    id: 'repo-update',
    label: 'Update docs (changes since last scan)',
    hint: 'Incremental: diff the repo against the commit recorded in the bundle and touch only affected notes.',
    skills: ['vault-repo-docs'],
    needsRepo: true,
    build: (repo) => `UPDATE the existing documentation bundle for the repository at ${repo} in this vault — incrementally. Only changes since the last scan are relevant; do NOT rewrite what is still accurate.

Method:
1. Locate the repo's bundle in the vault (folder named after the repo); read its index.md and overview.md. The overview frontmatter records the last scanned commit (\`commit:\`).
2. In the repo, list what changed since then: \`git log --oneline <commit>..HEAD\` and \`git diff --stat <commit>..HEAD\`. If no marker exists, say so in your summary and scope by comparing the docs against the current code only where they disagree.
3. Map each change to the affected notes: flows added → NEW flow note (one per flow, all of them); flows removed → delete/mark the note; changed routes/payloads/schemas/workers → update ONLY the affected sections and examples; new tables/topics → extend data.md / messaging.md. Regenerate api-openapi.yaml only if the API surface changed.
4. Keep every touched note's citations and mermaid diagrams in sync with the new code.
5. Refresh overview.md's \`commit:\` and \`scanned_at:\` to the new HEAD, and update index.md if notes were added/removed.
${QUALITY_BAR}

Finish: one-line summary listing which notes you added / updated / removed and the commit range covered.`,
  },
  {
    id: 'repo-flows',
    label: 'Flow catalog (one note per flow)',
    hint: 'Just the flows — inventory table plus one detailed note per flow, all of them.',
    skills: ['vault-repo-docs'],
    needsRepo: true,
    build: (repo) => `Catalog ALL flows of the repository at ${repo} into this vault, under <repo-name>/flows/.
${FLOW_RULES}

No other deliverables — flows only, but ALL of them (API, messaging, workers, reconciliation, startup). flows/index.md is the entry point.
${QUALITY_BAR}`,
  },
  {
    id: 'repo-api',
    label: 'API surface + OpenAPI spec',
    hint: 'Every route with examples, plus a real OpenAPI 3 YAML file.',
    skills: ['vault-repo-docs'],
    needsRepo: true,
    build: (repo) => `Document the COMPLETE HTTP API of the repository at ${repo} into this vault under <repo-name>/.

Deliverables:
1. api.md — every route as a table (method, path, auth, purpose), then one subsection per route: request/response shapes with a realistic JSON example each (from code/tests), validation rules, error responses, side effects (cite handler file paths).
2. api-openapi.yaml — a COMPLETE OpenAPI 3 spec: real component schemas (not stubs), every operation with parameters, requestBody, responses and at least one example each. It must be valid YAML that a swagger viewer renders.
3. index.md linking both.
${QUALITY_BAR}`,
  },
  {
    id: 'repo-data',
    label: 'Datastores audit',
    hint: 'Every table/collection/key across MySQL, MS SQL, Mongo, ClickHouse, Redis — schemas and access patterns.',
    skills: ['vault-repo-docs'],
    needsRepo: true,
    build: (repo) => `Audit EVERY datastore the repository at ${repo} touches and document them into this vault under <repo-name>/data/.

One note per store kind (brand MySQL, MS SQL, MongoDB, ClickHouse, Redis brand/MS — only those actually used): every table/collection/key pattern with purpose, a full column/field table (name, type, purpose), the actual queries/access patterns in the code, indexes, TTLs — and for EVERY column/field the code paths that read or write it (file citations). The goal is IMPACT ANALYSIS: from any column a reader must see what a change to it would affect. Add a d2 sql_table diagram per relational store and a data/index.md tying it together with a store → tables overview table.
${QUALITY_BAR}`,
  },
  {
    id: 'repo-messaging',
    label: 'Messaging map (producers/consumers)',
    hint: 'Topics, payloads in/out with examples, retry semantics, sequence diagrams.',
    skills: ['vault-repo-docs'],
    needsRepo: true,
    build: (repo) => `Map ALL messaging of the repository at ${repo} into this vault under <repo-name>/messaging/.

For EVERY consumer and producer: topic/queue name, trigger, FULL example payload (incoming and outgoing — realistic, from code/tests), schema of the payload, delivery guarantees, retry/backoff/poison handling, idempotency, and downstream effects (cite file paths). One mermaid sequenceDiagram per main message flow. messaging/index.md is the entry point with a topic → direction → handler table.
${QUALITY_BAR}`,
  },
];
