import { type APIRequestContext } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { join } from 'node:path';

// ── Seed a COMPLETE product story so every Product sub-flow renders with real,
// representative content on the isolated OFFLINE test daemon.
//
// Two seeding paths are used:
//   • HTTP API for everything that has an offline-safe create endpoint
//     (draft + body, questions+answers, notes, learnings, transcripts, to-swarm).
//   • Direct SQLite INSERTs for the surfaces that are otherwise LLM-gated and
//     would hang on an offline daemon (analysis + agent findings, the
//     source/suggested/plan versions, and the test-case run + cases).
//
// The product_* tables declare no FOREIGN KEYs, so opaque UUID ids + a matching
// story_id are all that's needed. We reuse the story's own created_by so authored
// rows attribute correctly.

function dbPath(): string {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const dir = join(process.cwd(), 'e2e', `.auth-${slot}`);
  const meta = JSON.parse(readFileSync(join(dir, 'daemon.json'), 'utf8')) as {
    dataDir: string;
  };
  return join(meta.dataDir, 'otto.db');
}

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

async function patchJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.patch(url, { data });
  if (!r.ok()) throw new Error(`PATCH ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

/** SQL single-quote literal (doubles embedded quotes); use `null` for NULL. */
function sql(s: string | null): string {
  return s === null ? 'NULL' : `'${s.replace(/'/g, "''")}'`;
}

function iso(offsetMin = 0): string {
  return new Date(Date.now() + offsetMin * 60_000).toISOString();
}

export interface SeededStory {
  storyId: string;
  createdBy: string;
}

export async function seedProductStory(
  ctx: APIRequestContext,
  base: string,
  workspaceId: string,
): Promise<SeededStory> {
  const ws = workspaceId;

  // 1. ── Draft story (offline-safe "create a new one" path) ──────────────────
  const draftDetail = await postJson(ctx, `${base}/api/v1/workspaces/${ws}/product/drafts`, {
    title: 'Wallet: instant withdrawals',
  });
  const storyId: string = draftDetail.story.id;
  const createdBy: string = draftDetail.story.created_by;

  const bodyMd = [
    '# Wallet: instant withdrawals',
    '',
    '**As a** player **I want** my withdrawals to settle instantly to my linked card',
    '**so that** I can access my winnings without waiting for the overnight batch.',
    '',
    '## Background',
    '',
    'Today withdrawals queue into a nightly settlement run. Competitors clear funds',
    'in seconds via push-to-card rails. This story moves us to real-time payouts for',
    'eligible players and rails, with a safe fallback to the batch path.',
    '',
    '## Acceptance criteria',
    '',
    '- Eligible withdrawals under the per-rail limit settle in < 30s.',
    '- Ineligible / over-limit requests fall back to the batch path transparently.',
    '- Every payout writes a ledger entry and emits a `payout.settled` event.',
    '- Players see a live status (queued → sent → settled) in the cashier.',
    '',
    '## Out of scope',
    '',
    '- New KYC flows (reuse existing verification).',
    '- Crypto rails (tracked separately).',
  ].join('\n');

  await patchJson(ctx, `${base}/api/v1/product/stories/${storyId}/draft`, {
    title: 'Wallet: instant withdrawals',
    body_md: bodyMd,
  });

  // Tag the story so the tag-filter row + chips render.
  await patchJson(ctx, `${base}/api/v1/product/stories/${storyId}`, {
    tags: 'wallet,payments,mvp',
  });

  // 2. ── Transcripts (Overview right panel for drafts) ───────────────────────
  for (const t of [
    {
      title: 'Kickoff with payments',
      body: 'Payments confirmed push-to-card is live for Visa/MC. Daily limit is $2k per player; over that we batch. They want an idempotency key per request.',
    },
    {
      title: 'Risk review',
      body: 'Risk wants a velocity check: max 5 instant payouts/hour/player, and a hard stop if the account is flagged. Fallback to batch must be silent to the player.',
    },
  ]) {
    await postJson(ctx, `${base}/api/v1/product/stories/${storyId}/transcripts`, t);
  }

  // 3. ── Questions (varied categories + statuses) ────────────────────────────
  const qDefs: { text: string; rationale: string; category: string; answer?: string; status?: string }[] = [
    {
      text: 'What is the per-transaction and daily limit for instant payouts?',
      rationale: 'Drives the eligibility check and the fallback threshold.',
      category: 'scope',
      answer: '$500 per transaction, $2,000 per player per day. Over either limit → batch.',
      status: 'answered',
    },
    {
      text: 'Which card networks are in scope for v1?',
      rationale: 'Determines which rails we integrate first.',
      category: 'dependency',
      answer: 'Visa Direct and Mastercard Send for v1. Others tracked separately.',
      status: 'answered',
    },
    {
      text: 'What status states should the cashier surface to the player?',
      rationale: 'Affects the live-status UX and the event contract.',
      category: 'ux',
    },
    {
      text: 'How do we reconcile a payout that succeeds on the rail but fails to write the ledger entry?',
      rationale: 'Edge case with real money — needs an explicit recovery path.',
      category: 'edge-case',
    },
    {
      text: 'Should flagged accounts get a generic "processing" message or an explicit decline?',
      rationale: 'Compliance + anti-fraud disclosure tradeoff.',
      category: 'other',
      status: 'discarded',
    },
  ];
  for (const qd of qDefs) {
    const created = await postJson(ctx, `${base}/api/v1/product/stories/${storyId}/questions`, {
      text: qd.text,
      rationale: qd.rationale,
      category: qd.category,
    });
    if (qd.status) {
      await patchJson(ctx, `${base}/api/v1/product/questions/${created.id}`, {
        status: qd.status,
        ...(qd.answer ? { answer: qd.answer } : {}),
      });
    }
  }

  // 4. ── Notes (multiple sections) ───────────────────────────────────────────
  for (const n of [
    {
      section: 'analysis',
      body: 'The settlement service already exposes a `submit_payout` RPC — we can reuse it and add a `mode: instant|batch` flag rather than a new endpoint.',
    },
    {
      section: 'implementation',
      body: 'Idempotency: hash (player_id, request_id) into the payout key. The rail SDK dedupes on that key for 24h, which covers our retry window.',
    },
    {
      section: 'clarifications',
      body: 'Confirmed with risk: the velocity cap is 5/hour/player. Exceeding it routes to batch, it does NOT hard-decline.',
    },
  ]) {
    await postJson(ctx, `${base}/api/v1/product/stories/${storyId}/notes`, n);
  }

  // 5. ── Learnings (patterns + avoid, active + suggested/inactive) ───────────
  const learnDefs: { kind: string; title: string; body: string; tags: string; active: boolean }[] = [
    {
      kind: 'pattern',
      title: 'Always attach an idempotency key to money-moving calls',
      body: 'Every payout/refund RPC must carry a deterministic idempotency key derived from (player_id, request_id). Retries then become safe and the rail dedupes for us.',
      tags: 'payments,reliability',
      active: true,
    },
    {
      kind: 'pattern',
      title: 'Write the ledger entry before calling the external rail',
      body: 'Record intent (pending ledger row) first, then call the rail, then settle. A crash after the rail call is recoverable because the intent already exists.',
      tags: 'payments,ledger',
      active: true,
    },
    {
      kind: 'avoid',
      title: 'Do not hard-decline on a velocity breach',
      body: 'Silently routing to the batch path keeps the player experience smooth and avoids leaking anti-fraud thresholds. Hard declines train abusers to probe limits.',
      tags: 'risk,ux',
      active: true,
    },
    {
      kind: 'avoid',
      title: 'Avoid a separate endpoint per payout mode',
      body: 'Forking instant vs batch into two endpoints doubles the surface and the tests. Use a single endpoint with a mode flag and a shared validation path.',
      tags: 'architecture',
      active: true,
    },
    {
      // Inactive → renders as an AI "suggested" learning with an Accept button.
      kind: 'pattern',
      title: 'Emit a domain event for every settled payout',
      body: 'Downstream (cashier UI, analytics, comms) should react to a `payout.settled` event rather than polling. Suggested from this story\'s analysis.',
      tags: 'events',
      active: false,
    },
  ];
  // Learnings are a GLOBAL library (not workspace-scoped), so they persist across
  // the per-project beforeAll seedings that share one test daemon. Only seed when
  // the library is still empty, so the screenshots show a clean 5 — not N×5.
  const existingLearnings = await (
    await ctx.get(`${base}/api/v1/workspaces/${ws}/product/learnings`)
  ).json();
  if (!Array.isArray(existingLearnings) || existingLearnings.length === 0) {
    for (const l of learnDefs) {
      const created = await postJson(ctx, `${base}/api/v1/workspaces/${ws}/product/learnings`, {
        kind: l.kind,
        title: l.title,
        body: l.body,
        tags: l.tags,
        source_story_id: storyId,
      });
      // New learnings default active=true; flip the "suggested" one to inactive.
      if (!l.active) {
        await patchJson(ctx, `${base}/api/v1/product/learnings/${created.id}`, { active: false });
      }
    }
  }

  // 6. ── Direct SQLite: versions, analysis, test cases, extra events ─────────
  const srcVersionId = randomUUID();
  const suggestedVersionId = randomUUID();
  const planVersionId = randomUUID();
  const analysisId = randomUUID();
  const runId = randomUUID();

  const sourceBody = bodyMd; // the original
  const suggestedBody = [
    '# Wallet: instant withdrawals',
    '',
    '**As a** verified player **I want** eligible withdrawals to settle to my linked',
    'card in under 30 seconds **so that** I get my winnings immediately, with a',
    'transparent fallback when a request is not eligible.',
    '',
    '## Acceptance criteria',
    '',
    '1. A withdrawal ≤ $500 and within the $2,000 daily cap settles in < 30s via',
    '   Visa Direct / Mastercard Send.',
    '2. Requests over a limit, on an unsupported rail, or from a flagged account',
    '   fall back to the nightly batch **silently** (player sees "processing").',
    '3. Each payout writes a pending ledger row *before* the rail call and settles',
    '   it after, and emits `payout.settled` exactly once.',
    '4. The cashier shows a live status: queued → sent → settled.',
    '5. Velocity cap: 5 instant payouts/hour/player; breaches route to batch.',
    '',
    '## Notes',
    '',
    'Reuses the existing `submit_payout` RPC with a new `mode` flag — no new',
    'endpoint. Idempotency key = hash(player_id, request_id).',
  ].join('\n');

  const planBody = [
    '# Implementation plan — Instant withdrawals',
    '',
    '### Task 1: Eligibility + limits service',
    '',
    '- [x] Add `is_instant_eligible(player, amount, rail)` with per-tx + daily caps',
    '- [x] Wire the velocity counter (5/hour/player) into the check',
    '- [ ] Unit-test the boundary cases (exactly at limit, just over)',
    '',
    '### Task 2: Payout pipeline',
    '',
    '- [~] Add `mode: instant|batch` to `submit_payout`',
    '- [ ] Write pending ledger row before the rail call',
    '- [ ] Settle ledger row + emit `payout.settled` after success',
    '- [ ] Silent fallback to batch on ineligibility',
    '',
    '### Task 3: Cashier live status',
    '',
    '- [ ] Subscribe the cashier to payout status updates',
    '- [ ] Render queued → sent → settled with timestamps',
    '',
    '### Task 4: Rollout',
    '',
    '- [ ] Feature-flag instant payouts per brand',
    '- [ ] Dashboards: settle latency p95, fallback rate, failure rate',
  ].join('\n');

  // Lens-agent findings (shape mirrors AnalysisTab.Findings).
  const poFindings = JSON.stringify({
    summary:
      'Clear PO value: move eligible withdrawals from an overnight batch to sub-30s push-to-card, with a safe, silent fallback. Scope is well-bounded (no new KYC, no crypto).',
    functionalities: [
      'Eligibility check (per-tx $500, daily $2,000, supported rails)',
      'Instant settlement via Visa Direct / Mastercard Send',
      'Silent fallback to nightly batch when ineligible',
      'Live cashier status (queued → sent → settled)',
    ],
    risks: [
      'Money-movement edge cases (rail success + ledger write failure) need an explicit recovery path',
      'Disclosing anti-fraud thresholds via error copy could be abused',
    ],
    open_questions: [
      { text: 'What status states should the cashier surface?', rationale: 'Defines the event contract + UX', category: 'ux' },
      { text: 'Generic "processing" vs explicit decline for flagged accounts?', rationale: 'Compliance tradeoff', category: 'other' },
    ],
    suggested_learnings: [
      { kind: 'pattern', title: 'Emit a domain event for every settled payout', body: 'Let downstream react to payout.settled rather than polling.' },
    ],
  });
  const archFindings = JSON.stringify({
    summary:
      'Reuse the existing settlement service. Add a mode flag to submit_payout rather than a parallel endpoint; keep the ledger as the source of truth with a write-ahead intent row.',
    related_repos: ['casino/go_wallet_gateway', 'casino/go_settlement', 'casino/cashier-ui'],
    integration_points: [
      'WALLET_GATEWAY.submit_payout (add mode flag)',
      'Visa Direct / Mastercard Send rail SDK',
      'Ledger service (pending → settled)',
      'Event bus (payout.settled)',
    ],
    risks: [
      'Idempotency must be enforced at the rail key level to survive retries',
      'Velocity counter is shared state — needs an atomic increment (Redis INCR + TTL)',
    ],
    functionalities: [
      'mode: instant|batch on submit_payout',
      'Write-ahead ledger intent row',
      'Atomic per-player velocity counter',
    ],
  });
  const clarFindings = JSON.stringify({
    summary: 'A handful of clarifications block a clean implementation — mostly around limits, rails in scope, and the reconciliation path for partial failures.',
    open_questions: [
      { text: 'Per-transaction and daily limits for instant payouts?', rationale: 'Drives eligibility + fallback threshold', category: 'scope' },
      { text: 'Which card networks are in scope for v1?', rationale: 'Determines rail integrations', category: 'dependency' },
      { text: 'Reconciliation when the rail succeeds but the ledger write fails?', rationale: 'Real-money recovery path', category: 'edge-case' },
    ],
    risks: ['Without a defined reconciliation path, a partial failure could double-pay or lose an entry'],
  });

  const synthSummary =
    'Instant withdrawals is a high-value, well-scoped story. Implementation should reuse submit_payout with a mode flag, write a ledger intent row before calling the rail, enforce idempotency at the rail key, and fall back to batch silently on ineligibility or velocity breach. Main risks are partial-failure reconciliation and atomic velocity counting. Three clarifications (limits, in-scope rails, reconciliation) should be answered before build.';

  // Test cases (steps_json = {preconditions[], steps[], expected}).
  const tc = (
    title: string,
    category: string,
    priority: string,
    preconditions: string[],
    steps: string[],
    expected: string,
    status: string,
    order: number,
  ) => ({
    id: randomUUID(),
    title,
    category,
    priority,
    steps_json: JSON.stringify({ preconditions, steps, expected }),
    status,
    order,
  });
  const cases = [
    tc(
      'Eligible withdrawal settles instantly',
      'happy',
      'high',
      ['Verified player', 'Linked Visa card', 'Balance ≥ $100'],
      ['Request a $100 withdrawal to the linked card', 'Observe the cashier status'],
      'Funds settle in < 30s; status shows queued → sent → settled; a payout.settled event is emitted.',
      'approved',
      0,
    ),
    tc(
      'Over-limit withdrawal falls back to batch',
      'validation',
      'high',
      ['Verified player', 'Daily instant total already $1,950'],
      ['Request a $200 withdrawal (would exceed the $2,000 daily cap)'],
      'Request is accepted but routed to the nightly batch; player sees "processing"; no instant settle.',
      'approved',
      1,
    ),
    tc(
      'Unsupported rail falls back to batch',
      'validation',
      'medium',
      ['Verified player', 'Linked card on an unsupported network'],
      ['Request a $50 withdrawal'],
      'Eligibility check fails on rail; request routes to batch silently.',
      'draft',
      2,
    ),
    tc(
      'Rail succeeds but ledger write fails → reconciled, not double-paid',
      'error',
      'high',
      ['Instant payout in flight', 'Ledger write fails after rail success'],
      ['Trigger the failure', 'Run reconciliation'],
      'Exactly one ledger entry exists after reconciliation; no double payout; alert raised.',
      'changes_requested',
      3,
    ),
    tc(
      'Velocity breach routes to batch',
      'edge',
      'medium',
      ['Verified player', '5 instant payouts already this hour'],
      ['Request a 6th instant withdrawal'],
      '6th request routes to batch silently; no hard decline; counter resets after the hour.',
      'approved',
      4,
    ),
    tc(
      'Flagged account does not settle instantly',
      'edge',
      'high',
      ['Player account flagged by risk'],
      ['Request any withdrawal'],
      'No instant settlement; request handled per risk policy; thresholds not disclosed in copy.',
      'draft',
      5,
    ),
  ];

  const stmts: string[] = [];
  stmts.push('PRAGMA busy_timeout=15000;');
  stmts.push('BEGIN IMMEDIATE;');

  // Versions: source (v2), suggested (v3), plan (v4). The draft already holds v1.
  stmts.push(
    `INSERT INTO product_story_versions (id, story_id, version_no, kind, title, body_md, raw_json, change_notes, created_by, created_at) VALUES (${sql(srcVersionId)}, ${sql(storyId)}, 2, 'source', ${sql('Wallet: instant withdrawals')}, ${sql(sourceBody)}, NULL, ${sql('Imported source baseline')}, ${sql(createdBy)}, ${sql(iso(-50))});`,
  );
  stmts.push(
    `INSERT INTO product_story_versions (id, story_id, version_no, kind, title, body_md, raw_json, change_notes, created_by, created_at) VALUES (${sql(suggestedVersionId)}, ${sql(storyId)}, 3, 'suggested', ${sql('Wallet: instant withdrawals (rewrite)')}, ${sql(suggestedBody)}, NULL, ${sql('Tightened acceptance criteria; added fallback + velocity rules')}, ${sql(createdBy)}, ${sql(iso(-30))});`,
  );
  stmts.push(
    `INSERT INTO product_story_versions (id, story_id, version_no, kind, title, body_md, raw_json, change_notes, created_by, created_at) VALUES (${sql(planVersionId)}, ${sql(storyId)}, 4, 'plan', ${sql('Implementation plan')}, ${sql(planBody)}, NULL, ${sql('Generated plan')}, ${sql(createdBy)}, ${sql(iso(-20))});`,
  );

  // Analysis + agents.
  stmts.push(
    `INSERT INTO product_analyses (id, story_id, source_version_id, status, summary, created_by, created_at, finished_at) VALUES (${sql(analysisId)}, ${sql(storyId)}, ${sql(srcVersionId)}, 'done', ${sql(synthSummary)}, ${sql(createdBy)}, ${sql(iso(-40))}, ${sql(iso(-38))});`,
  );
  const agentRows: { name: string; skill: string; findings: string | null }[] = [
    { name: 'PO Overview', skill: 'po-story-overview', findings: poFindings },
    { name: 'Architecture', skill: 'story-architecture-overview', findings: archFindings },
    { name: 'Clarifying Questions', skill: 'story-clarifying-questions', findings: clarFindings },
    { name: 'Summarizer', skill: 'summarizer', findings: null },
  ];
  for (const a of agentRows) {
    stmts.push(
      `INSERT INTO product_analysis_agents (id, analysis_id, name, skill, provider, model, status, findings_json, error, started_at, finished_at) VALUES (${sql(randomUUID())}, ${sql(analysisId)}, ${sql(a.name)}, ${sql(a.skill)}, 'claude', ${sql('claude-opus-4-8')}, 'done', ${a.findings ? sql(a.findings) : 'NULL'}, NULL, ${sql(iso(-40))}, ${sql(iso(-38))});`,
    );
  }

  // Test-case run + cases.
  stmts.push(
    `INSERT INTO product_testcase_runs (id, story_id, status, confluence_page_id, confluence_url, created_by, created_at) VALUES (${sql(runId)}, ${sql(storyId)}, 'draft', NULL, NULL, ${sql(createdBy)}, ${sql(iso(-15))});`,
  );
  for (const c of cases) {
    stmts.push(
      `INSERT INTO product_testcases (id, run_id, story_id, title, category, priority, steps_json, status, review_note, order_idx, created_at, updated_at) VALUES (${sql(c.id)}, ${sql(runId)}, ${sql(storyId)}, ${sql(c.title)}, ${sql(c.category)}, ${sql(c.priority)}, ${sql(c.steps_json)}, ${sql(c.status)}, NULL, ${c.order}, ${sql(iso(-15))}, ${sql(iso(-15))});`,
    );
  }

  // A few extra history events across sections so the History tab is rich.
  const events: { section: string; kind: string; summary: string; off: number }[] = [
    { section: 'analysis', kind: 'analysis_done', summary: 'Analysis completed: 3 lenses, 1 summarizer', off: -38 },
    { section: 'rewrite', kind: 'rewrite_done', summary: 'Suggested rewrite generated (v3)', off: -30 },
    { section: 'tests', kind: 'tests_generated', summary: 'Generated 6 test cases', off: -15 },
    { section: 'plan', kind: 'plan_done', summary: 'Implementation plan generated (v4)', off: -20 },
  ];
  for (const e of events) {
    stmts.push(
      `INSERT INTO product_events (id, story_id, section, kind, summary, actor_id, meta_json, created_at) VALUES (${sql(randomUUID())}, ${sql(storyId)}, ${sql(e.section)}, ${sql(e.kind)}, ${sql(e.summary)}, ${sql(createdBy)}, NULL, ${sql(iso(e.off))});`,
    );
  }

  stmts.push('COMMIT;');

  execFileSync('sqlite3', [dbPath()], { input: stmts.join('\n'), stdio: ['pipe', 'ignore', 'pipe'] });

  // 7. ── Plan → Swarm (offline-safe now a kind='plan' version exists) ────────
  try {
    await postJson(ctx, `${base}/api/v1/product/stories/${storyId}/to-swarm`, {
      name: 'Instant withdrawals',
    });
  } catch {
    // best-effort: the SwarmLinkCard simply won't show if this fails.
  }

  return { storyId, createdBy };
}
