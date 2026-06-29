import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDirtyRepo } from './seed';

// These tests form ONE dependent flow (a run is created, then gated, promoted,
// turned into a regression, then surfaced in the UI) and share module state, so
// they must run in order in a single worker (the project default is parallel).
test.describe.configure({ mode: 'serial' });

// ─────────────────────────────────────────────────────────────────────────────
// Eval Lab — full end-to-end against the real isolated daemon (no agents).
//
// Exercises every new capability through the live HTTP API + a UI smoke:
//   R1 run repo test commands · R2 attach proof pack · R3 provider×skill×prompt
//   matrix · R4 failed eval → regression case · R5 golden tasks per repo ·
//   R6 multi-signal score (tests/lint/diff/review/human) · R7 promote iff score+proof.
//
// The pipeline runs in `score_only` mode against a seeded DIRTY git repo (a
// real working-tree diff), so no coding-agent CLI is ever spawned — the harness
// runs with CLAUDE_BIN=/nonexistent. Commands are RECOGNIZED test runners (their
// string contains a needle like "cargo test") so a green one earns proof `passed`;
// `true # cargo test` is green, `false # cargo test` is red.
//
// Desktop-only spec (runs once) → no cross-project data duplication.
// ─────────────────────────────────────────────────────────────────────────────

const GREEN = 'true # cargo test';
const RED = 'false # cargo test';
const GREEN_LINT = 'true # cargo clippy';

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

// Shared across ordered tests in this file.
let passEvalId = '';
let passIterId = '';
let failEvalId = '';
let failIterId = '';
let failGoldenId = '';

async function getJson(url: string): Promise<any> {
  const r = await ctx.get(`${base}${url}`);
  if (!r.ok()) throw new Error(`GET ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}
async function postJson(url: string, data: unknown = {}): Promise<any> {
  const r = await ctx.post(`${base}${url}`, { data });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

/** Poll a run until it leaves "running" (or time out). */
async function pollEval(evalId: string, timeoutMs = 25_000): Promise<any> {
  const deadline = Date.now() + timeoutMs;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const e = await getJson(`/api/v1/skill-evaluations/${evalId}`);
    if (e.status !== 'running') return e;
    if (Date.now() > deadline) throw new Error(`eval ${evalId} still running after ${timeoutMs}ms`);
    await new Promise((r) => setTimeout(r, 350));
  }
}

/** Start a score-only run from a golden task against the seeded repo dir. */
async function scoreGolden(goldenId: string): Promise<any> {
  const eval0 = await postJson(`/api/v1/workspaces/${wsId}/skill-evaluations`, {
    source: { kind: 'library', reference: '' },
    task: '',
    impl_cli: '',
    validations: [],
    iterations: 1,
    mode: 'score_only',
    golden_task_id: goldenId,
    target: { kind: 'path', path: dir },
  });
  return pollEval(eval0.id);
}

async function createGolden(name: string, test_cmd: string, lint_cmd = ''): Promise<string> {
  const g = await postJson(`/api/v1/workspaces/${wsId}/golden-tasks`, {
    name,
    prompt: 'Implement the feature and keep the test command green.',
    skill: 'demo-skill',
    test_cmd,
    lint_cmd,
  });
  return g.id as string;
}

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  wsId = await seedWorkspace(ctx, base);
  const repo = await seedDirtyRepo(ctx, base, wsId);
  dir = repo.dir;
});

test.afterAll(async () => {
  await ctx?.dispose().catch(() => {});
});

// R1 + R2 + R6 — a recognized GREEN test command yields a passing proof pack and
// a positive composite score with the right signals.
test('score-only run with a green test command → proof passed + composite', async () => {
  const goldenId = await createGolden('green case', GREEN, GREEN_LINT);
  const e = await scoreGolden(goldenId);

  expect(e.status).toBe('done');
  expect(e.iterations.length).toBe(1);
  const it = e.iterations[0];
  expect(it.scoring).toBeTruthy();
  const s = it.scoring;

  // R1: the repo test command ran and is a 0/100 gate signal.
  expect(s.tests.ran).toBe(true);
  expect(s.tests.score).toBe(100);
  expect(s.lint.ran).toBe(true);
  expect(s.lint.score).toBe(100);
  // R6: diff-quality signal collected from the working-tree change.
  expect(s.diff.ran).toBe(true);
  expect(s.diff.files_changed).toBeGreaterThan(0);
  // R2: proof pack assembled + derived "passed" (diff + recognized passing test).
  expect(s.proof_status).toBe('passed');
  expect(it.proof_pack_id).toBeTruthy();
  expect(s.composite).toBeGreaterThan(0);
  expect(e.composite_score).toBeGreaterThan(0);

  // R2: the proof pack endpoint returns assembled evidence artifacts.
  const pack = await getJson(`/api/v1/skill-evaluations/${e.id}/iterations/${it.id}/proof-pack`);
  expect(pack.exists).toBe(true);
  expect(pack.status).toBe('passed');
  const kinds = (pack.artifacts ?? []).map((a: { kind: string }) => a.kind);
  expect(kinds).toContain('command'); // the test command
  expect(kinds).toContain('diff');

  passEvalId = e.id;
  passIterId = it.id;
});

// R6 — a recognized RED test command yields a failing proof pack.
test('score-only run with a red test command → proof failed', async () => {
  failGoldenId = await createGolden('red case', RED);
  const e = await scoreGolden(failGoldenId);

  expect(e.status).toBe('done');
  const it = e.iterations[0];
  expect(it.scoring.tests.ran).toBe(true);
  expect(it.scoring.tests.score).toBe(0);
  expect(it.scoring.proof_status).toBe('failed');

  failEvalId = e.id;
  failIterId = it.id;
});

// R7 — promotion is gated on composite score + proof: blocked for the red run,
// allowed for the green run; a forced promote (root) bypasses + is recorded.
test('promote gate: blocks the failed run, allows the passing run, force overrides', async () => {
  const passGate = await getJson(
    `/api/v1/skill-evaluations/${passEvalId}/promote-gate?iteration_id=${passIterId}`,
  );
  expect(passGate.proof_ok).toBe(true);
  expect(passGate.score_ok).toBe(true);
  expect(passGate.allowed).toBe(true);

  const failGate = await getJson(
    `/api/v1/skill-evaluations/${failEvalId}/promote-gate?iteration_id=${failIterId}`,
  );
  expect(failGate.proof_ok).toBe(false);
  expect(failGate.allowed).toBe(false);
  expect(failGate.reasons.length).toBeGreaterThan(0);

  // Promoting the failed run WITHOUT force is rejected (409 conflict).
  const blocked = await ctx.post(`${base}/api/v1/skill-evaluations/${failEvalId}/promote`, {
    data: { iteration_id: failIterId, source: 'tested', name: 'eval-lab-blocked' },
  });
  expect(blocked.status()).toBe(409);

  // Forcing it through succeeds (root) and marks the run promoted.
  const forced = await ctx.post(`${base}/api/v1/skill-evaluations/${failEvalId}/promote`, {
    data: { iteration_id: failIterId, source: 'tested', name: 'eval-lab-forced', force: true },
  });
  expect(forced.ok()).toBe(true);
  const after = await getJson(`/api/v1/skill-evaluations/${failEvalId}`);
  expect(after.promoted).toBe(true);
});

// R4 + R5 — a failed iteration is captured as a per-repo regression golden task,
// it appears in the corpus, and recapture is idempotent (deduped by source iter).
test('failed eval → regression case (deduped) shows up in the golden corpus', async () => {
  const reg = await postJson(
    `/api/v1/skill-evaluations/${failEvalId}/iterations/${failIterId}/regression`,
    {},
  );
  expect(reg.origin).toBe('regression');
  expect(reg.test_cmd).toBe(RED); // carries the run's command forward
  expect(reg.source_iter_id).toBe(failIterId);

  // R5: it's in the per-repo golden corpus.
  const list = await getJson(`/api/v1/workspaces/${wsId}/golden-tasks`);
  const regs = list.filter((g: { origin: string }) => g.origin === 'regression');
  expect(regs.length).toBeGreaterThanOrEqual(1);

  // Dedup: a second capture returns the same task, not a duplicate.
  const reg2 = await postJson(
    `/api/v1/skill-evaluations/${failEvalId}/iterations/${failIterId}/regression`,
    {},
  );
  expect(reg2.id).toBe(reg.id);
});

// R3 — a provider × skill × prompt matrix fans out to score-only cells and each
// cell gets a composite + proof.
test('matrix: provider × skill × prompt fans out to scored cells', async () => {
  const m0 = await postJson(`/api/v1/workspaces/${wsId}/eval-matrices`, {
    name: 'claude-vs-codex',
    mode: 'score_only',
    providers: ['claude', 'codex'],
    skills: [{ kind: 'library', reference: 'demo-skill' }],
    prompts: [
      { label: 'P1', task: 'do thing one' },
      { label: 'P2', task: 'do thing two' },
    ],
    target: { kind: 'path', path: dir },
    test_cmd: GREEN,
    iterations: 1,
  });

  // Poll the matrix until all cells settle.
  let m = m0;
  const deadline = Date.now() + 40_000;
  while (m.status === 'running') {
    if (Date.now() > deadline) throw new Error('matrix still running');
    await new Promise((r) => setTimeout(r, 600));
    m = await getJson(`/api/v1/eval-matrices/${m0.id}`);
  }

  expect(m.status).toBe('done');
  expect(m.cells.length).toBe(4); // 2 providers × 1 skill × 2 prompts
  for (const cell of m.cells) {
    expect(['done', 'error']).toContain(cell.status);
    expect(cell.composite_score).not.toBeNull();
  }
  // Every green cell should reach a passing proof.
  expect(m.cells.some((c: { proof_status: string }) => c.proof_status === 'passed')).toBe(true);
});

// UI smoke — the eval lab renders its tabs, the golden corpus, and a scorecard.
test('UI: eval-lab tabs, golden tasks, and scorecard render', async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);

  await page.goto('/#/skills-eval');
  await expect(page.locator('[data-testid="eval-tabs"]')).toBeVisible({ timeout: 30_000 });

  // A run auto-selects on the Runs tab; it carries a multi-signal scorecard.
  await expect(page.locator('[data-testid="scorecard"]').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-testid="scorecard-composite"]').first()).toBeVisible();

  // Golden Tasks tab lists the per-repo corpus (incl. the regression case).
  await page.locator('[data-testid="tab-golden"]').click();
  await expect(page.locator('[data-testid="golden-tasks"]')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('[data-testid="golden-card"]').first()).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('[data-testid="golden-regression-badge"]').first()).toBeVisible();

  // Matrix tab renders its grid for the matrix created above.
  await page.locator('[data-testid="tab-matrix"]').click();
  await expect(page.locator('[data-testid="matrix-view"]')).toBeVisible({ timeout: 15_000 });
});
