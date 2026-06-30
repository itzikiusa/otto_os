import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// ─────────────────────────────────────────────────────────────────────────────
// Vault v2 — "Repo Brain" end-to-end against the real isolated daemon.
//
// Covers the full code-intelligence vertical through the live HTTP API:
//   index a repo → symbol index → dependency graph (http_call/db_call/import/
//   test_of) → hybrid search WITH reasons → repo brain → linked doc → unified
//   full graph → remote backend config + install PLAN. Then a UI smoke that
//   drives every Vault tab and confirms the indexed repo + symbols + graph render.
//
// A tiny go_admission-style fixture (login → GetLimits → http(LIMITS) +
// db(MdlGm_tblLimits)) is written to a temp dir the daemon can read. No coding
// agent is ever spawned. Desktop-only (runs once).
// ─────────────────────────────────────────────────────────────────────────────

test.describe.configure({ mode: 'serial' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let repoDir = '';
let repoId = '';

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

function writeFixture(): string {
  const dir = mkdtempSync(join(tmpdir(), 'otto-vault-e2e-'));
  mkdirSync(join(dir, 'app'), { recursive: true });
  writeFileSync(
    join(dir, 'app', 'login.go'),
    `package app
import (
    "context"
    "bitbucket.org/gamescale-rnd/go_casino_kit/clients"
)
func Login(ctx context.Context, brandId int) error {
    _ = GetLimits(ctx, brandId)
    return nil
}
`,
  );
  writeFileSync(
    join(dir, 'app', 'limits.go'),
    `package app
import "context"
func GetLimits(ctx context.Context, brandId int) int {
    url, _ := serviceLocator.GetBrandService(ctx, brandId, "LIMITS")
    resp, _ := restClient.GetRequest(ctx, url)
    _ = resp
    row, _ := conn.GetContext(ctx, "SELECT max_limit FROM MdlGm_tblLimits WHERE brand_id = ?")
    _ = row
    return 0
}
`,
  );
  writeFileSync(join(dir, 'app', 'login_test.go'), `package app
func TestLogin(t *testing.T) {}
`);
  return dir;
}

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  repoDir = writeFixture();
});

test('indexes a repo and builds the symbol index + dependency graph', async () => {
  const started = await postJson(`/api/v1/workspaces/${wsId}/vault/repos/index`, {
    root: repoDir,
    name: 'go_admission_fixture',
  });
  repoId = started.repo_id;
  expect(repoId).toBeTruthy();

  // Indexing runs in the background — poll the repo status to completion.
  let repo: any;
  for (let i = 0; i < 60; i++) {
    const repos = await getJson(`/api/v1/workspaces/${wsId}/vault/repos`);
    repo = repos.find((r: any) => r.id === repoId);
    if (repo && (repo.status === 'ready' || repo.status === 'error')) break;
    await new Promise((r) => setTimeout(r, 1000));
  }
  expect(repo?.status).toBe('ready');
  expect(repo.symbols).toBeGreaterThanOrEqual(2);
  expect(repo.edges).toBeGreaterThanOrEqual(3);
  expect(repo.chunks).toBeGreaterThanOrEqual(1);

  const syms = await getJson(`/api/v1/workspaces/${wsId}/vault/symbols?q=limits`);
  expect(syms.some((s: any) => s.name === 'GetLimits')).toBeTruthy();
});

test('dependency graph has http_call / db_call / import / test_of edges', async () => {
  const g = await getJson(`/api/v1/workspaces/${wsId}/vault/graph?repo_id=${repoId}`);
  const rels = new Set(g.edges.map((e: any) => e.rel));
  expect(rels.has('http_call')).toBeTruthy();
  expect(rels.has('db_call')).toBeTruthy();
  expect(rels.has('imports')).toBeTruthy();
  expect(rels.has('test_of')).toBeTruthy();
  expect(g.nodes.some((n: any) => n.kind === 'service' && n.key === 'LIMITS')).toBeTruthy();
  expect(g.nodes.some((n: any) => n.kind === 'db_table' && n.key === 'MdlGm_tblLimits')).toBeTruthy();
  expect(g.nodes.some((n: any) => n.kind === 'service' && n.key === 'go_casino_kit')).toBeTruthy();
});

test('hybrid search returns hits annotated with WHY they were selected', async () => {
  const hits = await postJson(`/api/v1/workspaces/${wsId}/memory/search`, {
    text: 'limits service',
    k: 5,
  });
  expect(hits.length).toBeGreaterThan(0);
  // Every hit carries structured reasons (the explainability surface).
  expect(hits.every((h: any) => Array.isArray(h.reasons) && h.reasons.length > 0)).toBeTruthy();
});

test('repo brain assembles a context block + a doc links into the graph', async () => {
  const brain = await postJson(`/api/v1/workspaces/${wsId}/vault/brain`, {
    focus: 'login limits',
    cwd: repoDir,
  });
  expect(brain.markdown).toContain('Repo Brain');
  expect(brain.sections.some((s: any) => s.heading.includes('Indexed repos'))).toBeTruthy();

  // Link a doc to the Login symbol node.
  const nodeId = (await getJson(`/api/v1/workspaces/${wsId}/vault/graph?repo_id=${repoId}`)).nodes.find(
    (n: any) => n.kind === 'symbol' && n.key.endsWith('#Login'),
  )?.id;
  expect(nodeId).toBeTruthy();
  const doc = await postJson(`/api/v1/workspaces/${wsId}/vault/docs`, {
    repo_id: repoId,
    title: 'Login Flow',
    body: 'Brief: login authenticates then loads limits via go_casino_kit.',
    documents: [nodeId],
  });
  expect(doc.collection).toBe('docs');

  const full = await getJson(`/api/v1/workspaces/${wsId}/vault/fullgraph?repo_id=${repoId}`);
  expect(full.nodes.some((n: any) => n.kind === 'doc' && n.label === 'Login Flow')).toBeTruthy();
});

test('remote backends: config + health + install plan', async () => {
  const list0 = await getJson(`/api/v1/workspaces/${wsId}/vault/backends`);
  expect(Array.isArray(list0)).toBeTruthy();

  // Configure a (not-running) Qdrant backend — upsert returns a row with status.
  const b = await ctx.put(`${base}/api/v1/workspaces/${wsId}/vault/backends/qdrant`, {
    data: { enabled: true, url: 'http://127.0.0.1:6333', role: 'vector' },
  });
  expect(b.ok()).toBeTruthy();
  const row = await b.json();
  expect(row.kind).toBe('qdrant');
  // Not running → status error (graceful), never a 500.
  expect(['error', 'ok', 'unknown']).toContain(row.status);

  // Install PLAN is a side-effect-free preview.
  const plan = await postJson(`/api/v1/workspaces/${wsId}/vault/backends/qdrant/install/plan`);
  expect(plan.kind).toBe('qdrant');
  expect(plan.health_url).toContain('6333');
});

test('UI: every Vault tab renders; Repos + Symbols + Graph reflect the index', async ({ page }) => {
  await page.goto('/#/vault');
  await expect(page.getByTestId('vault-tabs')).toBeVisible();
  for (const tab of ['knowledge', 'graph', 'repos', 'symbols', 'backends', 'brain']) {
    await expect(page.getByTestId(`vault-tab-${tab}`)).toBeVisible();
  }

  // Repos tab → the indexed fixture repo card appears.
  await page.getByTestId('vault-tab-repos').click();
  await expect(page.getByText('go_admission_fixture')).toBeVisible({ timeout: 10_000 });

  // Symbols tab → search finds GetLimits.
  await page.getByTestId('vault-tab-symbols').click();
  const search = page.getByPlaceholder(/Search symbols/i);
  await search.fill('GetLimits');
  await expect(page.getByText('GetLimits').first()).toBeVisible({ timeout: 10_000 });

  // Graph tab → the force graph renders nodes (svg or canvas).
  await page.getByTestId('vault-tab-graph').click();
  await expect(page.locator('.vp-body svg, .vp-body canvas').first()).toBeVisible({ timeout: 10_000 });
});
