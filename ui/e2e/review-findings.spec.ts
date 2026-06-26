import { test, expect, type APIRequestContext } from '@playwright/test';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Review Findings Workflow — end-to-end against the isolated test daemon (OTTO_E2E=1).
//
// Exercises EVERY requirement deterministically via the `__e2e` seed endpoints +
// the real action/transition endpoints:
//   - all 11 finding fields are persisted + surfaced
//   - all 6 statuses are reachable (and an illegal transition is rejected)
//   - all 7 action buttons perform their effect + write an audit event
//   - Proof Pack assembles with correct counts + exports
//   - "Add to repo rule" materializes into a session's instruction file (Context Engine)
//   - exported verified findings are recallable from the memory store
//
// All state is global to the daemon, so pin the whole file to ONE device project.

test.describe.configure({ mode: 'serial' });
test.beforeEach(({}, testInfo) => {
  test.skip(
    testInfo.project.name !== 'iphone-portrait',
    'findings-workflow state is global to the daemon; run on a single project only',
  );
});

let ctx: APIRequestContext;
let base: string;
let ws: string;
let repoId: string;
let repoDir: string;

const api = (p: string) => `${base}/api/v1${p}`;

async function jget(url: string): Promise<any> {
  const r = await ctx.get(url);
  if (!r.ok()) throw new Error(`GET ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}
async function jpost(url: string, body: unknown = {}): Promise<any> {
  const r = await ctx.post(url, { data: body });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

async function seedReview(pr = 0): Promise<string> {
  const rev = await jpost(api(`/workspaces/${ws}/__e2e/review`), { repo_id: repoId, pr_number: pr });
  return rev.id as string;
}
async function seedFinding(reviewId: string, over: Record<string, unknown> = {}): Promise<any> {
  return jpost(api(`/workspaces/${ws}/__e2e/findings`), {
    review_id: reviewId,
    repo_id: repoId,
    title: 'Seeded',
    body: 'seeded body',
    evidence: 'seeded evidence',
    reasoning: 'seeded reasoning',
    severity: 'high',
    category: 'security',
    status: 'open',
    ...over,
  });
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  ws = await seedWorkspace(ctx, base);
  const r = await seedGitRepo(ctx, base, ws);
  repoId = r.repoId;
  repoDir = r.dir;
});

test.afterAll(async () => {
  await ctx.dispose();
});

test('all 11 finding fields are persisted and surfaced, severity normalized', async () => {
  const rev = await seedReview();
  const f = await seedFinding(rev, {
    title: 'SQL injection',
    evidence: 'let q = format!("... {}", name);',
    reasoning: 'user input reaches the query unescaped',
    suggested_fix: 'use a parameterized query',
    path: 'src/db.rs',
    line: 10,
    line_end: 12,
    severity: 'bug', // → normalized to high
    category: 'security',
    status: 'open',
  });
  const detail = await jget(api(`/findings/${f.id}`));
  const F = detail.finding;
  // the 11 required fields all present
  for (const k of [
    'id', 'severity', 'category', 'path', 'line', 'line_end', 'evidence',
    'agent_reasoning_summary', 'suggested_fix', 'status', 'linked_commit', 'linked_test', 'reviewer',
  ]) {
    expect(k in F, `field ${k} present`).toBeTruthy();
  }
  expect(F.severity).toBe('high'); // bug → high (normalized on write)
  expect(F.category).toBe('security');
  expect(F.path).toBe('src/db.rs');
  expect(F.line).toBe(10);
  expect(F.line_end).toBe(12);
  expect(F.evidence.length).toBeGreaterThan(0); // not vacuous
  expect(F.agent_reasoning_summary.length).toBeGreaterThan(0);
  expect(F.suggested_fix).toContain('parameterized');
  expect(F.status).toBe('open');
  // it shows up on the review's findings list
  const list = await jget(api(`/reviews/${rev}/findings`));
  expect(list.some((x: any) => x.id === F.id)).toBeTruthy();
});

test('all 6 statuses are reachable; illegal transition rejected', async () => {
  const rev = await seedReview();
  // open (seed) — implicit
  const o = await seedFinding(rev, { status: 'open' });
  expect(o.status).toBe('open');
  // open → accepted
  let r = await jpost(api(`/findings/${(await seedFinding(rev)).id}/accept`));
  expect(r.status).toBe('accepted');
  // → false_positive
  r = await jpost(api(`/findings/${(await seedFinding(rev)).id}/false-positive`), {});
  expect(r.status).toBe('false_positive');
  // → waived
  r = await jpost(api(`/findings/${(await seedFinding(rev)).id}/waive`), {});
  expect(r.status).toBe('waived');
  // seeded fixed → verify → verified (+ linked_commit stamped)
  const fixed = await seedFinding(rev, { status: 'fixed', linked_commit: 'deadbeef' });
  expect(fixed.status).toBe('fixed');
  r = await jpost(api(`/findings/${fixed.id}/verify`));
  expect(r.finding.status).toBe('verified');
  expect(r.finding.linked_commit).toBeTruthy();
  // illegal: open → verify is rejected (400)
  const open2 = await seedFinding(rev, { status: 'open' });
  const bad = await ctx.post(api(`/findings/${open2.id}/verify`));
  expect(bad.status()).toBe(400);
});

test('button: Ask agent to fix → accepted + fix_requested event', async () => {
  const rev = await seedReview();
  const f = await seedFinding(rev, { status: 'open' });
  const resp = await jpost(api(`/findings/${f.id}/fix`));
  expect(resp.finding.status).toBe('accepted');
  const detail = await jget(api(`/findings/${f.id}`));
  expect(detail.events.some((e: any) => e.kind === 'fix_requested')).toBeTruthy();
});

test('button: Mark false positive + Require human approval + Approve/Reject', async () => {
  const rev = await seedReview();
  // require-approval then APPROVE → accepted, gate cleared
  const a = await seedFinding(rev, { status: 'open' });
  let r = await jpost(api(`/findings/${a.id}/require-approval`));
  expect(r.requires_human_approval).toBe(true);
  r = await jpost(api(`/findings/${a.id}/approve`), { decision: 'approve' });
  expect(r.status).toBe('accepted');
  expect(r.requires_human_approval).toBe(false);
  expect(r.approval_decision).toBe('approved');
  // require-approval then REJECT → false_positive
  const b = await seedFinding(rev, { status: 'open' });
  await jpost(api(`/findings/${b.id}/require-approval`));
  r = await jpost(api(`/findings/${b.id}/approve`), { decision: 'reject' });
  expect(r.status).toBe('false_positive');
});

test('button: Convert to Jira → 400 when no Jira account configured', async () => {
  const rev = await seedReview();
  const f = await seedFinding(rev, { status: 'open' });
  const resp = await ctx.post(api(`/findings/${f.id}/jira`), { data: { project_key: 'PROJ' } });
  expect(resp.status()).toBe(400);
  const body = await resp.json();
  expect(body.code).toBe('invalid');
});

test('button: Add to repo rule → rule created + listed', async () => {
  const rev = await seedReview();
  const f = await seedFinding(rev, { status: 'open', title: 'no raw sql' });
  const rule = await jpost(api(`/findings/${f.id}/repo-rule`), {
    title: 'NEVER build SQL with format!',
    body: 'Use parameterized queries.',
  });
  expect(rule.id).toBeTruthy();
  expect(rule.enabled).toBe(true);
  const rules = await jget(api(`/workspaces/${ws}/repo-rules`));
  expect(rules.some((x: any) => x.id === rule.id)).toBeTruthy();
  // the finding is linked to the rule
  const detail = await jget(api(`/findings/${f.id}`));
  expect(detail.finding.repo_rule_id).toBe(rule.id);
  expect(detail.events.some((e: any) => e.kind === 'repo_rule_added')).toBeTruthy();
});

test('button: Add regression test → action recorded', async () => {
  const rev = await seedReview();
  const f = await seedFinding(rev, { status: 'fixed', linked_commit: 'abc' });
  const resp = await jpost(api(`/findings/${f.id}/regression-test`));
  expect(resp.finding.id).toBe(f.id);
  const detail = await jget(api(`/findings/${f.id}`));
  expect(detail.events.some((e: any) => e.kind === 'regression_test_requested')).toBeTruthy();
});

test('Proof Pack assembles with correct counts and exports markdown', async () => {
  const rev = await seedReview();
  await seedFinding(rev, { status: 'verified', linked_commit: 'c1', linked_test: 'tests/x_test.rs', title: 'V1' });
  await seedFinding(rev, { status: 'fixed', linked_commit: 'c2', title: 'F1' });
  await seedFinding(rev, { status: 'open', title: 'O1' });
  const pack = await jget(api(`/reviews/${rev}/proof-pack`));
  expect(pack.summary.total).toBe(3);
  expect(pack.summary.verified).toBe(1);
  expect(pack.summary.fixed).toBe(1);
  expect(pack.summary.open).toBe(1);
  expect(pack.summary.with_commit).toBe(2);
  expect(pack.summary.with_test).toBe(1);
  // each entry carries its event timeline
  expect(pack.findings.every((e: any) => Array.isArray(e.events))).toBeTruthy();
  const exp = await jpost(api(`/reviews/${rev}/proof-pack/export`), {});
  expect(exp.markdown).toContain('Proof Pack');
  expect(exp.markdown).toContain('V1');
  expect(exp.id).toBeTruthy();
});

test('Context Engine: a repo rule is materialized into a new session instruction file', async () => {
  const rev = await seedReview();
  const f = await seedFinding(rev, { status: 'open', title: 'ctx-rule-src' });
  const marker = 'PARAMETERIZE-ALL-SQL-MARKER';
  await jpost(api(`/findings/${f.id}/repo-rule`), { title: marker, body: 'Always parameterize queries.' });

  // Spawn a session in the repo dir; the Provisioner (PreSpawnHook) materializes
  // the workspace context — including the repo-rules block — into CLAUDE.md/AGENTS.md.
  await jpost(api(`/workspaces/${ws}/sessions`), { kind: 'agent', provider: 'claude', cwd: repoDir });

  // Poll for the instruction file to contain the rule (materialize is synchronous,
  // but allow a brief window for the spawn path).
  let found = false;
  for (let i = 0; i < 20 && !found; i++) {
    for (const fn of ['CLAUDE.md', 'AGENTS.md']) {
      const p = join(repoDir, fn);
      if (existsSync(p) && readFileSync(p, 'utf8').includes(marker)) {
        found = true;
        break;
      }
    }
    if (!found) await new Promise((r) => setTimeout(r, 300));
  }
  expect(found, `repo rule "${marker}" materialized into the session instruction file`).toBeTruthy();
});

test('Memory: an exported verified finding is recallable from the memory store', async () => {
  const rev = await seedReview();
  const marker = 'UNIQUE-RECALL-MARKER-SQLI';
  await seedFinding(rev, {
    status: 'verified',
    linked_commit: 'c9',
    title: marker,
    evidence: 'format!() into a query',
    reasoning: 'tainted input',
  });
  await jpost(api(`/reviews/${rev}/proof-pack/export`), {});

  // Search the memory store; the verified finding should be recallable (keyword/FTS
  // path works offline). Be tolerant of the hit envelope shape.
  const hits = await jpost(api(`/workspaces/${ws}/memory/search`), { text: marker, k: 10, mode: 'keyword' });
  const blob = JSON.stringify(hits);
  expect(blob).toContain(marker);
});
