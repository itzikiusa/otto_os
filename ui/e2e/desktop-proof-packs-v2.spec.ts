import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Proof Packs v2 E2E. Drives the new v2 surface (done contract, snapshots,
// media/api/db/kafka evidence, PR-consistency check, repo proof-config, report
// export, approver-gated waiver) against the isolated E2E daemon — the
// status/badges/score are DERIVED server-side, so the API responses are the
// contract — then renders the v2 done-contract meter in the desktop browser.
// Runs in the desktop-browser project only.

const V1 = '/api/v1';

// A 1×1 transparent PNG — the smallest valid image payload for media evidence.
const TINY_PNG_B64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

test.describe('proof packs v2', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  async function mkPack(ctx: any, base: string, ws: string, title: string): Promise<string> {
    const r = await ctx.post(`${base}${V1}/workspaces/${ws}/proof-packs`, {
      data: { work_item_kind: 'manual', work_item_id: `wi-${title}-${Date.now()}`, title },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    return (await r.json()).id as string;
  }

  test('done contract: score + itemized checklist + list done_score', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'contract');

    // Add a diff + a passing test → contract items get satisfied + score rises.
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'diff', title: 'Working tree diff', content: 'diff --git a b\n+1', status: 'info' },
    });
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'cargo test --workspace', content: 'ok. 10 passed', status: 'passed' },
    });

    // Detail carries the explainable done contract.
    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    expect(detail.done_contract).toBeDefined();
    expect(typeof detail.done_contract.score).toBe('number');
    expect(detail.done_contract.score).toBeGreaterThan(0);
    expect(Array.isArray(detail.done_contract.items)).toBeTruthy();
    expect(detail.done_contract.items.length).toBeGreaterThan(0);
    const item = detail.done_contract.items[0];
    expect(item).toHaveProperty('key');
    expect(item).toHaveProperty('label');
    expect(item).toHaveProperty('required');
    expect(item).toHaveProperty('satisfied');

    // The list row exposes the compact done_score.
    const list = await (await ctx.get(`${base}${V1}/workspaces/${ws}/proof-packs`)).json();
    const row = list.find((p: any) => p.id === packId);
    expect(row).toBeTruthy();
    expect(typeof row.done_score).toBe('number');
    expect(row.done_score).toBeGreaterThan(0);

    await ctx.dispose();
  });

  test('snapshot: hashed copy + frozen report + listing + bundle', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'snapshot');
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'cargo test', content: 'ok', status: 'passed' },
    });

    // Freeze a snapshot → sha256 + md/html reports come back.
    let r = await ctx.post(`${base}${V1}/proof-packs/${packId}/snapshot`, {
      data: { note: 'release candidate' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    const snap = await r.json();
    expect(typeof snap.sha256).toBe('string');
    expect(snap.sha256.length).toBeGreaterThan(0);
    expect(snap.report_md).toContain('Proof Pack');
    expect(typeof snap.report_html).toBe('string');
    expect(snap.report_html.length).toBeGreaterThan(0);
    expect(typeof snap.done_score).toBe('number');

    // It's listed (newest first) …
    const snaps = await (await ctx.get(`${base}${V1}/proof-packs/${packId}/snapshots`)).json();
    expect(Array.isArray(snaps)).toBeTruthy();
    expect(snaps.length).toBeGreaterThanOrEqual(1);
    expect(snaps[0].id).toBe(snap.id);
    expect(snaps[0].note).toBe('release candidate');

    // … and fetchable as a full bundle.
    r = await ctx.get(`${base}${V1}/proof-snapshots/${snap.id}`);
    expect(r.ok(), await r.text()).toBeTruthy();
    const full = await r.json();
    expect(full.sha256).toBe(snap.sha256);
    expect(full.bundle).toBeDefined();

    await ctx.dispose();
  });

  test('media: PNG upload → screenshot artifact + blob round-trip', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'media');

    const r = await ctx.post(`${base}${V1}/proof-packs/${packId}/media`, {
      data: { kind: 'screenshot', title: 'home page', mime: 'image/png', data_base64: TINY_PNG_B64 },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    const shot = detail.artifacts.find((a: any) => a.kind === 'screenshot');
    expect(shot, 'a screenshot artifact should exist').toBeTruthy();
    expect(shot.title).toBe('home page');

    // The blob is fetchable and served as an image.
    const blob = await ctx.get(`${base}${V1}/proof-artifacts/${shot.id}/blob`);
    expect(blob.ok(), await blob.text()).toBeTruthy();
    expect(blob.headers()['content-type']).toContain('image');

    await ctx.dispose();
  });

  test('evidence: api/db/kafka artifacts + db_api_verified badge', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'evidence');

    // API evidence with a 2xx status → a passed api artifact.
    let r = await ctx.post(`${base}${V1}/proof-packs/${packId}/evidence/api`, {
      data: { title: 'GET /health', method: 'GET', url: 'https://api.example.com/health', status: 200 },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/evidence/db`, {
      data: { title: 'orders count', engine: 'mysql', query: 'SELECT count(*) FROM orders', row_count: 1, sample: '42' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/evidence/kafka`, {
      data: { title: 'orders.events', topic: 'orders.events', message_count: 3, sample: '{"id":1}' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    const api = detail.artifacts.find((a: any) => a.kind === 'api');
    expect(api, 'an api artifact should exist').toBeTruthy();
    expect(api.status).toBe('passed');
    expect(detail.artifacts.some((a: any) => a.kind === 'db')).toBeTruthy();
    expect(detail.artifacts.some((a: any) => a.kind === 'kafka')).toBeTruthy();
    expect(detail.badges).toContain('db_api_verified');

    await ctx.dispose();
  });

  test('pr-check: false "tests pass" claim → failed + pr_inconsistent', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'prcheck');

    // A failing test artifact contradicts a description that claims tests pass.
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'go test ./...', content: 'FAIL', status: 'failed' },
    });
    const r = await ctx.post(`${base}${V1}/proof-packs/${packId}/pr-check`, {
      data: {
        title: 'PR description check',
        description: 'All tests pass and the change is fully covered.',
      },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    const prc = detail.artifacts.find((a: any) => a.kind === 'pr_check');
    expect(prc, 'a pr_check artifact should exist').toBeTruthy();
    expect(prc.status).toBe('failed');
    expect(detail.badges).toContain('pr_inconsistent');

    await ctx.dispose();
  });

  test('repo proof-config: GET/PUT round-trip', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const { repoId } = await seedGitRepo(ctx, base, ws);

    // Strengthen the repo's requirements …
    let r = await ctx.put(`${base}${V1}/repos/${repoId}/proof-config`, {
      data: { require_ci: true, require_test: true, test_cmd: 'cargo test --workspace' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    // … and read them back.
    r = await ctx.get(`${base}${V1}/repos/${repoId}/proof-config`);
    expect(r.ok(), await r.text()).toBeTruthy();
    const cfg = await r.json();
    expect(cfg.repo_id).toBe(repoId);
    expect(cfg.require_ci).toBe(true);
    expect(cfg.require_test).toBe(true);
    expect(cfg.test_cmd).toBe('cargo test --workspace');

    // NOTE: capping a repo-linked pack at `partial` isn't seeded here — the
    // create endpoint doesn't accept a repo_id, so a pack can't be linked to this
    // repo over the REST surface in isolation. The GET/PUT round-trip is covered.

    await ctx.dispose();
  });

  test('report: markdown export contains "Proof Pack"', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'report');
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'cargo test', content: 'ok', status: 'passed' },
    });

    const r = await ctx.get(`${base}${V1}/proof-packs/${packId}/report?format=md`);
    expect(r.ok(), await r.text()).toBeTruthy();
    const md = await r.text();
    expect(md).toContain('Proof Pack');

    await ctx.dispose();
  });

  test('waiver: short reason rejected (400); real reason → waived + approval', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'waiver');

    // A too-short reason is rejected by the backend.
    let r = await ctx.post(`${base}${V1}/proof-packs/${packId}/waive`, { data: { reason: 'nope' } });
    expect(r.status()).toBe(400);

    // A real reason waives the gate.
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/waive`, {
      data: { reason: 'verified manually in the E2E run' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    const pack = await r.json();
    expect(pack.status).toBe('waived');
    expect(pack.badges).toContain('waived');
    expect(pack.waived_at).toBeTruthy();

    // The waiver leaves an approval artifact behind.
    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    expect(detail.artifacts.some((a: any) => a.kind === 'approval')).toBeTruthy();

    await ctx.dispose();
  });

  test('Proof page renders the done-contract meter', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const created = await (
      await ctx.post(`${base}${V1}/workspaces/${ws}/proof-packs`, {
        data: { work_item_kind: 'manual', work_item_id: `wi-meter-${Date.now()}`, title: 'Meter Proof Pack' },
      })
    ).json();
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'command', title: 'cargo test', content: 'ok', status: 'passed' },
    });
    await ctx.dispose();

    await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
    await page.goto('/#/proof');

    // Open the pack from the list, then the v2 done-contract meter must render.
    await expect(page.getByText('Meter Proof Pack').first()).toBeVisible({ timeout: 30_000 });
    await page.getByText('Meter Proof Pack').first().click();
    await expect(page.getByText('Done contract').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText('/100').first()).toBeVisible({ timeout: 10_000 });
  });
});
