# Walkthrough videos — update plan (audit of 2026-09-02)

Every composition in `src/compositions/` was read caption-by-caption and checked
against `docs/features/*.md` (authoritative), `ui/src/lib/sidebar.ts` and the
code. This file lists, per composition, **what to change, why, and which doc
proves it**, then a scene outline for each walkthrough that does not exist yet.
Nothing here has been rendered; follow `AGENT_BRIEF.md` when implementing.

Legend: **OK** = accurate as-is · **STALE** = true when filmed, missing major
capabilities since · **WRONG** = a caption states something the product does
not do (or names it wrongly).

| Composition | Verdict | One-liner |
|---|---|---|
| Intro | STALE | Pillar list omits Browser / AWS / Kubernetes / Personal Agents / Run with Otto; "Knowledge Vault" pillar name predates Vault v3. |
| Sessions | STALE | Four fixed providers → custom providers + per-session model picker + name themes; "nudge" overclaims. |
| MissionControl | WRONG | "Scheduled · daily-digest" node kind does not exist; the eight kinds are different; click → detail panel, not "the work". |
| Git | STALE | No conflict resolver / WIP folder staging / Focus tab / watchable PR-draft session; "Conventional Commits title" for PRs is wrong. |
| Review | OK | Minor: lenses are skill-driven (not a fixed four); summarizer is claude-only. |
| ProofPacks | OK | Minor: gates are env knobs; PR gate only fires when a `proof_pack_id` is attached. |
| Product | STALE | Lens/bucket names are invented; "tasks get owners" is false; Inject spawns a *new* session; misses test cases → Confluence, Learnings, Mockups, Discovery Chat. |
| Canvas | WRONG | "Many scenes per canvas · tab between · Present" is unwired legacy; D2 mode missing. |
| Swarm | WRONG | Org ladder, board columns, "leader goal-verify loop" and "integration branch merged back" are not the product; presets are six. |
| GoalLoops | STALE | "Validates the criteria command before the first iteration" — only presence is validated. |
| Connections | STALE | Kind list omits PostgreSQL, Kafka, Custom CLI; "drag and drop" upload in SFTP unproven. |
| Database | STALE | "Ask in English" → DB Assistant; misses index editor, mongosh scripts, detached queries, batched row UPDATEs, Explain/ERD/Format; "cancel any query" overclaims. |
| Brokers | OK | Missing newer tabs (topic configs, Replay, Lag Alerts, broker CPU/RAM). |
| Channels | STALE | Telegram inbound files are not downloaded; no mention of `Action: Workflow` / `run <name>:` chat triggers. |
| Workflows | STALE | "Scheduled triggers are being wired" is false (they ship); "concurrently where possible" is false (strictly sequential); retry / stall watchdog / run queue / versioning / chat triggers missing. |
| ScheduledTasks | STALE | Cadence list omits cron + timezone; misses any-provider, worktree sandbox, retry, notify-on-change, presets, proof attach. |
| Mcp | WRONG | "8 otto.* tools" → 101 (incl. `aws_*`/`k8s_*`); pipeline has more stages; reads default on. |
| Vault | WRONG | "Keyword + semantic (vector) hybrid recall" — Vault v3 has **no embeddings**; "Product/Swarm/Review pull from the Vault" is false; whole video predates the file-backed docs home + OKF. |
| Skills | OK | Minor: memory edits are claude-only; add pointer to Skills Lab. |
| SkillsEval | STALE | Module is now **Skills Lab** with tabs Skills · Review · Evaluator; window titles say "Skills Evaluator". |
| UsageInsights | STALE | "Monthly budget ring" → opt-in budgets rendered as rows + banner; per-feature rollups missing; tailer is Claude+Codex only. |
| Api | OK | Missing GraphQL, cookie jar, OpenAPI export, git sync, Automations; gRPC is unary + server-streaming only. |
| Plugins | OK | — |
| TeamMobile | STALE | "24-hour TTL" → 1h/4h/12h/24h fixed at mint (OTP shares: ≤12h window); grant matrix is 25 features incl. AWS/K8s; misses workspace roles "By user". |
| Platform | WRONG | "⌘T jump" — ⌘T is **New session**; jump is ⌃1…⌃9 / ⌘[ ⌘]. Multi-window ⌘⇧N and Snip ⌘⌃⇧2 missing. |
| Outro | OK | Pillar list shares Intro's omissions. |

---

## Per-composition changes

### Intro.tsx (STALE)
- **PILLARS** (l.240–262): add `Run with Otto` (icon `play`), `Browser` (`globe`),
  `AWS` (`cloud`*), `Kubernetes` (`helm`*), `Personal Agents` (`user`); rename
  `Knowledge Vault` → `Vault · Docs`, `Skills` → `Skills Lab`. (*icons not in the
  kit yet — port from `ui/src/lib/components/Icon.svelte`, which is a shared-file
  change; otherwise reuse `box`/`grid`.) Proof: `ui/src/lib/sidebar.ts`,
  `docs/features/README.md`.
- Caption l.233: "Claude Code, Codex, Antigravity & shell" → "Claude Code, Codex,
  Antigravity, any custom CLI & shell". Proof: `agent-sessions.md` §1–2,
  `ui/src/lib/providers.ts`.

### Sessions.tsx (STALE)
- Scene 1 sub "claude, codex, agy, or a plain shell" → add "…or any provider you
  add in Settings → Providers — with a per-session model pick". Show the
  `ModelPicker` dropdown in the New Session form. Proof: `personal-agents.md` §5–6.
- Scene 2 pane titles ("fix auth middleware"…) → the shipping default is the
  name-theme ("Ronaldo", "Messi"…) plus provider-derived titles; address a session
  by name in Broadcast (`"ronaldo: run the tests"`). Proof:
  `crates/otto-sessions/src/names.rs`, `ui/src/modules/settings/SessionNames.svelte`.
- Scene 4 sub: drop "stuck agents get a nudge" (only a bounded reminder for
  headless automation agents — `crates/otto-server/src/agent_session.rs:53`).
  Replace with "resume works for claude, codex and agy". Proof:
  `crates/otto-sessions/src/providers.rs` (`resume_args`).
- Optional beat: split panes + activity trail (`agent-sessions.md` §6).

### MissionControl.tsx (WRONG)
- Node list (l.207–217, 441–459): remove `Scheduled · daily-digest`; kinds are
  **Session · Swarm Project · Goal Loop · Workflow Run · PR Review · Product
  Story · Pull Request · External Trigger**. Replace with e.g. `Workflow · release-guard`,
  `PR #482`, `Trigger · slack #support`. Proof: `mission-control.md` §"The eight
  kinds", `ui/src/modules/mission-control/lib.ts` (`KIND_LABEL`).
- Caption l.377: "Sessions, swarms, goal loops, reviews, product, scheduled" →
  "…reviews, product stories, workflows, PRs & external triggers".
- Caption l.479–480: "Click any node to jump straight to the work" → "Click any
  node for its detail panel — sessions and triggers open the live terminal".
  Proof: `mission-control.md` ("No outward actions").

### Git.tsx (STALE)
- Scene 2 (graph) keep; add lane hover tooltip + "Stash & switch" checkout from
  the graph (`GraphView.svelte`, `git.md` §5).
- Scene 3 (commit): rename to the **WIP panel** — per-file / per-folder / "Stage
  all" checkboxes and the trash (discard) at all three levels. Proof `git.md` §5.
- **New scene** "Conflict resolver": a temporary **Conflict** tab → Resolver view
  with Ours/Theirs per file, **Complete / Abort**; works for merge, rebase,
  cherry-pick, stash-pop. Proof `git.md` §6, `ConflictResolverView.svelte`.
- Scene 4 caption l.406: drop "Conventional Commits title" — the PR title is
  `<JIRA-KEY> <imperative summary>` ≤72 chars (`pull-request` skill). Show the
  **`PR draft · <branch>`** session with its elapsed counter and the "PR & commit
  draft model" (default haiku) from Settings → Providers. Proof `git.md` §7.
- Add a beat on the **Focus** tab (My Pull Requests across forges + My Work from
  Jira) — `RepoView.svelte:186`, `FocusView.svelte` (undocumented; verify live).

### Review.tsx (OK, polish)
- Caption l.164: "Security, Correctness, Performance, Tests" → "…each lens is an
  installed `review` skill" (`code-review.md` §lenses). Note the summarizer runs
  on claude.
- Scene 3 sub l.378: use the real ladder `open · accepted · false_positive ·
  fixed · verified · waived` and "re-review never resets triage"
  (`review-findings.md`).

### ProofPacks.tsx (OK, polish)
- Caption l.477–478: "Require a PR, a goal loop, or auto-tests" → name the five
  gates (goal loops opt-in, PR creation default ON, review/workflows/sessions
  package evidence) and that they are env knobs `OTTO_PROOF_REQUIRE_*`
  (`proof-packs.md` §gates). Add the 0-100 score + HTML report export.

### Product.tsx (STALE)
- Caption l.274 buckets "Risk · Edge Cases · Dependencies · Acceptance" → real
  lenses **PO Overview · Architecture · Clarifying Questions**; findings
  sections **Related repos · Functionalities · Integration points · Risks · Open
  questions · Suggested learnings**; question categories Scope/Data/UX/Edge
  Case/Dependency/Other. Post-back works for Confluence too. Proof `product.md`
  §4.2–4.3.
- Caption l.440: drop "tasks get owners" (plan items are todo→in-progress→done;
  owners exist only after Send to Swarm). Proof `product.md` §4.6, `PlanTab.svelte`.
- Caption l.560: "inject into a running session" → "spawn a preloaded agent
  session" (`POST …/inject-session`). Proof `product.md` §4.10.
- **Add a scene**: Test cases (Happy Path / Validation / Error / Edge; approve →
  publish "Test Cases — <story>" to Confluence, full-width) + the Learnings base
  (Patterns to follow / Cases to avoid). Proof `product.md` §4.5, §4.8,
  `crates/otto-issues/src/confluence.rs:238`.
- Mention Discovery **Chat** action cards with Undo (`discovery-chat.md`).

### Canvas.tsx (WRONG)
- Scene 4 (l.581–582 "Many scenes per canvas · Tab between diagrams · Present")
  → **replace** with the third mode **D2** (`canvas.d2`, in-browser WASM render,
  Sketch toggle, PNG/SVG export) and the scene rail with Search + Sections.
  Present mode and multi-scene tabs are unmounted legacy. Proof `canvas.md` §3, §9.
- Caption l.273 "open in real Excalidraw any time" → "a real `.excalidraw` scene
  on disk at `<data_dir>/canvas/<id>/canvas.json`" (no hand-off action; export
  menu exists). Proof `canvas.md` §9–10.

### Swarm.tsx (WRONG)
- Caption l.198: ladder is **CEO → CTO → VP → Team Lead → Devs / QA** (generic
  `reports_to`). Proof `agent-swarm.md` §tree.
- Caption l.407: board is **backlog → todo → in_progress → in_review → done**;
  only `todo` with deps done is picked up. Proof `agent-swarm.md` §board.
- Caption l.569: delete "Leader goal-verify loop" and "Integration branch merged
  back" (neither exists — zero hits in doc or `swarm_runtime.rs`). Replace with
  "recruiter drafts roles · per-agent schedules · run & cost budgets".
- Presets: six (`po-team.yaml` added). Proof `crates/otto-swarm/assets/presets/`.

### GoalLoops.tsx (STALE)
- Caption l.154 "validates the criteria command before the first iteration" →
  "Launch is blocked until every criterion has a description and a verify
  command; commands run in the VERIFY phase". Proof `goal-loops.md` §criteria.
- Add "Retry" on a stuck executor + pause/resume with worktree re-attach.

### Connections.tsx (STALE)
- Caption l.253: "SSH, MySQL, Redis, Mongo, ClickHouse" → add **PostgreSQL,
  Kafka clusters, Custom CLI**. Proof `connections-ssh-sftp.md` §1.
- Caption l.450: drop "drag and drop" unless verified in `SftpBrowser` (doc only
  describes drag in the sidebar tree).
- Mention access = the **Connections** feature grant (Settings → Users → Feature
  grants). Proof `rbac-multiuser-sharing.md`.

### Database.tsx (STALE)
- Caption l.298 "Natural-language → SQL" → **DB Assistant** (file-backed agent:
  writes `SCHEMA.md`, read-only `./q` tool, final `ANSWER.sql`). Proof
  `docs/contracts/api.md` §DB Assistant, `db_assist.rs`.
- Caption l.172 "cancel any running query" → "cancel (engine-native on MySQL /
  Postgres / ClickHouse-HTTP)". Proof `database-explorer.md` §4.
- Caption l.299: add "batched cell edits → one reviewed UPDATE per row".
- **Add a scene** "Structure & Mongo": Indexes block (New / Edit / Drop, partial
  index Condition, nested Mongo paths) + a mongosh script in the query tab with
  the "mongosh script detected" notice. Proof `database-explorer.md` §3–4.
- Add: queries survive navigation (detached run + re-attach), Explain, Format,
  Mask, ERD tab, Saved queries/History, Postgres in the engine list.

### Brokers.tsx (OK, polish)
- Scene 4 caption: add the **Replay** and **Lag Alerts** tabs, topic config
  editing, offset reset and broker CPU/RAM via Prometheus scrape. Proof
  `message-brokers.md` §1.

### Channels.tsx (STALE)
- Caption l.330 "attach files in, receive files out" → "Slack files in; uploads
  out on both (Telegram inbound files are text-only)". Proof
  `channels-slack-telegram.md` §files.
- **Add a beat**: start a workflow from the thread — `Action: Workflow` /
  `run <name>: <prompt>` with in-thread `status / skip / abort / help`. Proof
  `workflows.md` §5 (the Channels doc itself does not mention it yet — cross-doc gap).

### Workflows.tsx (STALE)
- Caption l.283 "concurrently where possible" → delete; runs are strictly
  sequential (`workflow_engine.rs:1504`). Proof `workflows.md` §run loop.
- Caption l.480 "Scheduled triggers are being wired" → "manual, webhook, event,
  **schedule** (cron + timezone) and **chat** triggers". Proof `workflows.md`
  §"Schedule triggers fire at boot now".
- Caption l.374: approval pauses until approve/reject **or `NODE_AGENT_TIMEOUT`**;
  survives daemon restart.
- **Add a scene**: per-node retry `{max_attempts, backoff_ms}` + stall watchdog,
  the 2-parallel run queue (`OTTO_WF_MAX_PARALLEL_RUNS`), graph versioning with
  restore. Proof `workflows.md` §retry, §queue.
- Node palette: include `condition`, `loop`, `git_pr`, `api_run`, `budget_gate`.

### ScheduledTasks.tsx (STALE)
- Caption l.155 "interval, daily, or weekly" → "+ **cron**, per-task timezone".
- Scene 2: any provider (claude/codex/agy/shell/custom) + `ModelPicker`,
  worktree sandbox, retry policy, `notify_on_change`, `attach_proof`, presets
  (`weekly-security-scan`, …). Proof `scheduled-tasks.md` §summary table.
- Keep "seven otto.* tools" (still exactly seven for this feature).

### Mcp.tsx (WRONG)
- Caption l.371 "8 otto.* tools" → "100+ otto.* tools across every feature —
  incl. `aws_*` and `k8s_*`" (101 in `mcp_outward.rs`). Proof
  `mcp-control-plane.md` §3.3 category table.
- Caption l.172 pipeline → "enabled → per-tool permission → allowlist → policy →
  approval (single-use, args-hash bound) → dry-run → fail-closed audit → stats".
- "opt-in gateway": reads default on, mutating tools DANGEROUS/approval-gated;
  server itself off by default. Add the gateway (Otto's own agents' downstream
  MCP calls) and the two transports (stdio, Streamable-HTTP).

### Vault.tsx (WRONG — re-shoot)
- Kill Scene 3 entirely ("Keyword + semantic (vector) hybrid recall"): Vault v3
  has **no embeddings, no vector stores** (`vault.md` §1, §7, §8;
  `crates/otto-vault/src/lib.rs:5`). Replace with **FTS5 search with `tag:` /
  `path:` / `type:` operators + the ⌘O quick switcher**.
- Scene 2 caption: "workspace knowledge store" → "a **docs home**: register a
  local markdown folder (an Obsidian vault works as-is); wikilinks resolve,
  backlinks are derived". Vaults are global, not per-workspace. Proof `vault.md` §1–3.
- Scene 4 caption l.630–631: drop "Product, Swarm, and Review pull from the
  Vault" (false — `product.md` §4.11 "recall not yet auto-used"). Replace with
  "graph focus by service / type / tag / hops + service rollup; agents read &
  write through `otto_vault_*` MCP tools". Proof `vault/GraphView.svelte:14`,
  `vault.md` §MCP.
- **Add a scene**: the **OKF** card (validator E1–E3 / W1–W5, templates, index
  generation) and the Docs agents (write / review / revise). Proof `vault.md` §4.6.
- Title kicker: "Vault · Docs home".

### Skills.tsx (OK, polish)
- Scene 1: note the library is also browsable/editable in the **Skills Lab**
  sidebar module (tabs Skills · Review · Evaluator). Proof
  `ui/src/modules/skills-lab/SkillsLabPage.svelte`.
- Caption l.660: "reflects across providers" → "analysis runs on every configured
  provider (merged); memory edits target claude's project memory". Proof
  `self-improvement.md`.

### SkillsEval.tsx (STALE)
- All window titles "Otto — Skills Evaluator" → "Otto — Skills Lab · Evaluator";
  `Navigator active="skills-eval"` label is **Skills Lab**. Proof
  `ui/src/lib/sidebar.ts:78`.
- Add a short beat on the other two tabs (Skills viewer/editor with zip import;
  multi-agent skills **Review** with an apply-fixes agent). Proof
  `skills-evaluator.md` intro, docs index.

### UsageInsights.tsx (STALE)
- Scene 2 "monthly budget ring" → budgets are **opt-in**, rolling `window_days`,
  rendered as status rows + a live `budget_exceeded` banner (no ring). Proof
  `usage-and-cost.md` §budgets, `UsagePage.svelte:348`.
- Add "per-feature rollups"; footnote that the tailer parses Claude + Codex
  transcripts (agy/custom not captured).

### Api.tsx (OK, polish)
- Transport chips: add **GraphQL**; gRPC = unary + server-streaming.
- Scene 4: add Automations (collection runner with assertions), cookie jar,
  OpenAPI export, git sync of collections. Proof `api-client.md`.

### TeamMobile.tsx (STALE)
- Caption l.353 "24-hour TTL" → "1h / 4h / 12h / 24h, fixed at mint; OTP shares
  keep a ≤12h attach window and re-pend". Proof `session-sharing.md` §TTL.
- Scene 1 grant matrix: 25 features incl. `aws_*` and `kubernetes`; add the
  **Workspace roles → By user** tab (grant one account several workspaces).
  Proof `rbac-multiuser-sharing.md` §matrix, §workspaces.
- Scene 3 sub: mention the alternative opt-in TLS network listener (off by default).

### Platform.tsx (WRONG)
- Caption l.371 "⌘T jump" → **⌘T new session**; jump = **⌃1…⌃9**, prev/next =
  **⌘[ / ⌘]**; add **⌘U** (update CLIs now), **⌘⇧N** new window, **⌘⌃⇧2** snip.
  Proof `ui/src/lib/keys.ts`, `multi-window.md`, `snipping-tool.md`.
- Add a beat: Multi-window (independent workspace surfaces restored on relaunch)
  and the Snipping Tool (image on clipboard at every step).

### Outro.tsx
- Mirror the Intro pillar additions.

---

## Missing walkthroughs — proposed scene outlines

Each follows the house structure: `TitleCard` → 3–4 app scenes with a
`<Caption step>` lower-third → `WalkOutro`. Target ~600–700 frames. Use
`Navigator active=` with the new module ids (`aws`, `kubernetes`, `browser`,
`personal-agents`) — `Nav.tsx` must learn these ids first (shared-file change,
do it in its own commit). Sidebar labels: **AWS**, **Kubernetes**, **Browser**,
**Personal Agents** (`ui/src/lib/sidebar.ts`).

### Aws.tsx — "AWS console" (`docs/features/aws-console.md`)
1. **Title** (80f) — kicker "AWS console", title "S3 · SQS · EC2 · Athena · EKS",
   sub "Through the aws CLI v2, per saved account — secrets in the Keychain."
2. **Accounts** (170f) — Settings-style account cards: `prod (SSO profile)`,
   `staging (access keys)`, region + environment color chip; a **Sign in** button
   that opens `aws sso login` in a PTY tab; first-run "Install now" panel with a
   log tail. Caption: "Accounts, not credentials — SSO profile or access keys;
   `~/.aws` is never written."
3. **S3 + SQS** (220f) — split: left an S3 bucket → prefix breadcrumb → object
   preview (JSON, ranged 64 KiB) + Download; right an SQS queue with **Peek 10**
   (visibility-timeout 0), Send with FIFO fields, ⋯ → Purge with typed
   confirmation. Caption: "Browse S3 read-only · peek, send, redrive & purge SQS
   — destructive verbs need a typed confirm."
4. **Athena + EC2** (220f) — Athena three-pane workbench (catalog tree, editor
   with ⌘↵ Run, results in the DB Explorer grid, "scanned 1.2 GB · $0.006"
   status bar, History); an EC2 table with state pills and Stop → `confirm_id`.
   Caption: "Athena results land in the DB Explorer grid · EC2 start / stop /
   reboot — every action audited."
5. **EKS → Kubernetes** (150f) — cluster row → **Open in Kubernetes** → the
   Kubernetes module opens with the imported context. Caption: "One click
   imports an EKS cluster into the Kubernetes console — your kubeconfig is never
   touched."
6. **WalkOutro** — pills S3 · SQS · EC2 · Athena · EKS · Keychain · Audited.

### Kubernetes.tsx — "Kubernetes console" (`docs/features/kubernetes-console.md`)
1. **Title** (80f) — kicker "Kubernetes console", title "k9s in a tab", sub
   "Any kubeconfig context — workloads, logs, exec, Argo — through plain kubectl."
2. **Clusters** (150f) — register: pick a context from `~/.kube/config`, paste a
   kubeconfig, or import from EKS; capability probe chips (metrics-server ✓,
   Argo Rollouts ✓, Argo CD ✓); auto-install kubectl / k9s card. Caption:
   "Register a context, paste a kubeconfig, or import EKS — missing tools
   install themselves, never sudo."
3. **Workspace** (230f) — left rail of kinds (Pods … Argo Rollouts, ArgoCD Apps),
   namespace filter, a Pods table with health-coloured status
   (`CrashLoopBackOff` red, `Init:1/2` progressing), cpu/mem from metrics-server;
   press `l` → drawer **Logs** tab streaming with **Follow** on. Caption: "16
   kinds, health colouring, live metrics · `l` `s` `d` open logs, shell,
   describe — k9s muscle memory."
4. **Exec + actions** (200f) — **Terminal** tab: a real Otto session titled
   `api-7d9f… · payments`; right-click a Deployment → Restart / Scale / Rollout
   undo (typed confirm); an Argo Rollout with Promote / Abort / Retry; an Argo CD
   app **Sync** dialog (revision + prune). Caption: "Shell in a pod is a normal
   Otto session · restart, scale, rollout, promote, sync — each one audited
   kubectl."
5. **WalkOutro** — pills kubectl-only · Argo Rollouts · Argo CD · Logs -f · Exec
   · RBAC ×2.

### Browser.tsx — "Browser" (`docs/features/browser.md`)
1. **Title** (80f) — kicker "Browser", title "The web, inside your workspace",
   sub "Reader & live tabs, marks you hand to an agent, credentials agents may
   borrow."
2. **Reader tab** (180f) — paste a URL → markdown render with title; the
   Reader / **Live** toggle; `degraded: true` chip explained as Lightpanda
   best-effort. Caption: "Reader mode fetches → markdown (JS via the Lightpanda
   sidecar, plain-fetch fallback) · every URL passes the SSRF guard."
3. **Marks** (220f) — select a paragraph → add a comment → mark appears in the
   notes rail keyed by URL; **Send to session** drops a fenced `[Browser mark]`
   block into a live claude session; **Save to Vault** writes an OKF note with
   `## Mark 1`. Caption: "Annotate any page · send a mark into a running agent ·
   save page + marks as a Vault note."
4. **Live + credentials** (200f) — Live toggle hosts a real page (desktop app
   only); crosshair element picker; Site Credentials card with
   `allow_agent_use` toggle; an agent calling `browser_login`. Caption: "Live
   tabs are native webviews · Keychain-backed site credentials an agent may use
   only when you opt in."
5. **WalkOutro** — pills Reader · Live · Marks · Summarize · Vault · MCP tools.

### PersonalAgents.tsx — "Personal Agents" (`docs/features/personal-agents.md`)
1. **Title** (80f) — kicker "Personal Agents", title "Agents with a name and a
   schedule", sub "A persona, a pinned provider + model, memory, and rooms you
   can always read."
2. **Agent cards** (160f) — Sidebar **Personal Agents** → cards: Personal
   Assistant, Daily Recap (2 schedules), Casino Reviewer; provider·model chip
   (`claude · sonnet`), next run, **Run now**. Caption: "Ships with four
   editable examples, disabled until you switch them on."
3. **Agent page** (230f) — tabs **Overview / Schedules / Runs / Chat / Memory**:
   Schedules shows `daily 09:00 Europe/Tel_Aviv → "recap yesterday"` and `every
   15m → "anything needing me?"`, each with its own directive; the `ModelPicker`
   pinning `--model`; Memory shows `memory/notes.md`. Caption: "1..N schedules,
   each its own directive & timezone · the model is pinned per agent, never
   globally."
4. **Runs + Rooms** (200f) — a run row → its report + delivery state (Slack ✓);
   **Chat** tab embeds the persona's live terminal; the **Rooms** view: two
   agents exchanging `otto.room_post` messages and a user post box. Caption:
   "Every run is a fresh session with a report · rooms are the only agent-to-
   agent channel — and you see all of it."
5. **WalkOutro** — pills Persona · Pinned model · Schedules · Memory · Rooms ·
   Delivery.

### Also un-filmed (lower priority)
Run with Otto (`run-with-otto.md` — one-button pipeline, best shown as a Slack
`Action: Run` → approval gate → PR), Snipping Tool, Multi-window, Model catalog
(Settings → Providers → Refresh; could be a beat inside Sessions).
