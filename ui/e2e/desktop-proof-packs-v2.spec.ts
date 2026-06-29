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

    // The end-to-end capping behaviour (a repo-linked pack held at `partial` until
    // its CI requirement is met) is covered by the dedicated
    // "repo policy: a repo-linked pack is capped to partial until CI is green" test.

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

  test('ci status: ci_passed / ci_failed / ci_pending badges (R2)', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);

    // A green CI artifact → ci_passed. (A `ci` artifact can be attached via the
    // generic artifacts endpoint; the dedicated ci-refresh route fetches it from
    // the git provider for a PR-linked pack.)
    const green = await mkPack(ctx, base, ws, 'ci-green');
    await ctx.post(`${base}${V1}/proof-packs/${green}/artifacts`, {
      data: { kind: 'ci', title: 'CI status', content: 'all checks green', status: 'passed' },
    });
    let detail = await (await ctx.get(`${base}${V1}/proof-packs/${green}`)).json();
    expect(detail.badges).toContain('ci_passed');
    expect(detail.badges).not.toContain('ci_failed');

    // A red CI artifact → ci_failed (and it fails the pack).
    const red = await mkPack(ctx, base, ws, 'ci-red');
    await ctx.post(`${base}${V1}/proof-packs/${red}/artifacts`, {
      data: { kind: 'ci', title: 'CI status', content: '2 checks failed', status: 'failed' },
    });
    detail = await (await ctx.get(`${base}${V1}/proof-packs/${red}`)).json();
    expect(detail.badges).toContain('ci_failed');
    expect(detail.pack.status).toBe('failed');

    // A pending CI artifact → ci_pending.
    const pend = await mkPack(ctx, base, ws, 'ci-pending');
    await ctx.post(`${base}${V1}/proof-packs/${pend}/artifacts`, {
      data: { kind: 'ci', title: 'CI status', content: 'queued', status: 'pending' },
    });
    detail = await (await ctx.get(`${base}${V1}/proof-packs/${pend}`)).json();
    expect(detail.badges).toContain('ci_pending');

    // A diff with no CI at all → ci_missing.
    const miss = await mkPack(ctx, base, ws, 'ci-missing');
    await ctx.post(`${base}${V1}/proof-packs/${miss}/artifacts`, {
      data: { kind: 'diff', title: 'Working tree diff', content: 'diff --git a b\n+1', status: 'info' },
    });
    detail = await (await ctx.get(`${base}${V1}/proof-packs/${miss}`)).json();
    expect(detail.badges).toContain('ci_missing');

    await ctx.dispose();
  });

  test('repo policy: a repo-linked pack is capped to partial until CI is green (R3)', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const { repoId } = await seedGitRepo(ctx, base, ws);

    // Repo *requires* green CI + a passing test (strengthen-only policy).
    let r = await ctx.put(`${base}${V1}/repos/${repoId}/proof-config`, {
      data: { require_ci: true, require_test: true },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    // Create a pack LINKED to that repo (repo_id on create) so its policy applies.
    const created = await (
      await ctx.post(`${base}${V1}/workspaces/${ws}/proof-packs`, {
        data: {
          work_item_kind: 'manual',
          work_item_id: `wi-repocap-${Date.now()}`,
          title: 'repo cap',
          repo_id: repoId,
        },
      })
    ).json();
    expect(created.repo_id).toBe(repoId);

    // A diff + a passing test would normally pass a manual pack — but the repo
    // requires green CI too, so the policy caps it to `partial`.
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'diff', title: 'Working tree diff', content: 'diff --git a b\n+1', status: 'info' },
    });
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'command', title: 'cargo test --workspace', content: 'ok. 9 passed', status: 'passed' },
    });
    let detail = await (await ctx.get(`${base}${V1}/proof-packs/${created.id}`)).json();
    expect(detail.pack.status, 'capped by require_ci until CI is green').toBe('partial');
    // The done-contract names the unmet CI requirement.
    const ci = detail.done_contract.items.find((i: any) => i.key === 'ci');
    expect(ci).toBeTruthy();
    expect(ci.required).toBe(true);
    expect(ci.satisfied).toBe(false);

    // Attach green CI → the requirement is met → the pack reaches `passed`.
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'ci', title: 'CI status', content: 'green', status: 'passed' },
    });
    detail = await (await ctx.get(`${base}${V1}/proof-packs/${created.id}`)).json();
    expect(detail.pack.status, 'CI now green → released').toBe('passed');

    await ctx.dispose();
  });

  test('media: unsupported mime → 415, oversized blob → 413 (R4)', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'media-reject');

    // A disallowed MIME is rejected with 415 Unsupported Media Type.
    let r = await ctx.post(`${base}${V1}/proof-packs/${packId}/media`, {
      data: { kind: 'screenshot', title: 'evil', mime: 'application/x-msdownload', data_base64: TINY_PNG_B64 },
    });
    expect(r.status()).toBe(415);

    // A blob over the 25 MiB cap is rejected with 413 Payload Too Large. (36e6
    // base64 'A's decode to 27e6 bytes > 25 MiB, while staying under the 40 MiB
    // request-body limit so the cap — not the body limit — is what fires.)
    const oversized = 'A'.repeat(36_000_000);
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/media`, {
      data: { kind: 'screenshot', title: 'huge', mime: 'image/png', data_base64: oversized },
    });
    expect(r.status()).toBe(413);

    await ctx.dispose();
  });

  test('evidence failure states: api 5xx → failed, kafka empty → not verified, db error → failed (R5/R6)', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'evidence-fail');

    // A 500 API response → a FAILED api artifact (which fails the pack).
    let r = await ctx.post(`${base}${V1}/proof-packs/${packId}/evidence/api`, {
      data: { title: 'POST /checkout', method: 'POST', url: 'https://api.example.com/checkout', status: 500 },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    // A Kafka read that returned ZERO messages is neutral (info), not "verified".
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/evidence/kafka`, {
      data: { title: 'empty topic', topic: 'orders.events', message_count: 0 },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    // A DB read that errored → a FAILED db artifact.
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/evidence/db`, {
      data: { title: 'bad query', engine: 'mysql', query: 'SELECT * FROM missing', error: 'table not found' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    const api = detail.artifacts.find((a: any) => a.kind === 'api');
    expect(api.status).toBe('failed');
    const kafka = detail.artifacts.find((a: any) => a.kind === 'kafka');
    expect(kafka.status).toBe('info');
    const db = detail.artifacts.find((a: any) => a.kind === 'db');
    expect(db.status).toBe('failed');
    // A failing api/db read fails the pack and there's no db_api_verified badge.
    expect(detail.pack.status).toBe('failed');
    expect(detail.badges).not.toContain('db_api_verified');

    await ctx.dispose();
  });

  test('report: HTML export carries the done-contract + evidence sections (R9)', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'html-report');
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'cargo test --workspace', content: 'ok. 12 passed', status: 'passed' },
    });

    const r = await ctx.get(`${base}${V1}/proof-packs/${packId}/report?format=html`);
    expect(r.ok(), await r.text()).toBeTruthy();
    expect(r.headers()['content-type']).toContain('text/html');
    const html = await r.text();
    expect(html).toContain('<!doctype html>');
    expect(html).toContain('Done contract');
    expect(html).toContain('Evidence');
    expect(html).toContain('cargo test'); // the artifact title is rendered

    await ctx.dispose();
  });

  test('pr-check: a consistent description → passed (no false claim) (R7)', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const packId = await mkPack(ctx, base, ws, 'prcheck-ok');

    // A passing test artifact backs an honest, substantive description.
    await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'cargo test --workspace', content: 'ok. 30 passed', status: 'passed' },
    });
    // The route returns the recomputed pack (ProofPackResp, per api.md #134); the
    // consistency report itself is stored on the pr_check artifact's
    // metadata.report (which is what the UI's structured render reads).
    const r = await ctx.post(`${base}${V1}/proof-packs/${packId}/pr-check`, {
      data: {
        title: 'Add evidence export to proof packs',
        description:
          'This change adds an exportable evidence report. Testing: ran cargo test --workspace and all tests pass.',
      },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    const detail = await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json();
    const prc = detail.artifacts.find((a: any) => a.kind === 'pr_check');
    expect(prc, 'a pr_check artifact should exist').toBeTruthy();
    expect(prc.status).toBe('passed');
    expect(prc.metadata.report.passed).toBe(true);
    expect(prc.metadata.report.hard_fail).toBe(false);
    expect(detail.badges).not.toContain('pr_inconsistent');

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

  test('Proof page renders the structured PR-consistency report (R7)', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const created = await (
      await ctx.post(`${base}${V1}/workspaces/${ws}/proof-packs`, {
        data: { work_item_kind: 'manual', work_item_id: `wi-prview-${Date.now()}`, title: 'PR View Pack' },
      })
    ).json();
    // A failing test + a "tests pass" claim → a hard-fail pr_check artifact whose
    // structured report the UI must render as a per-check breakdown.
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'command', title: 'go test ./...', content: 'FAIL', status: 'failed' },
    });
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/pr-check`, {
      data: { title: 'Fix it', description: 'All tests pass and CI is green now.' },
    });
    await ctx.dispose();

    await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
    await page.goto('/#/proof');
    await expect(page.getByText('PR View Pack').first()).toBeVisible({ timeout: 30_000 });
    await page.getByText('PR View Pack').first().click();
    // The structured report header + at least one itemized check line render.
    await expect(page.getByText(/Consistency/).first()).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText(/No false 'tests pass' claim/).first()).toBeVisible({ timeout: 10_000 });
  });
});
