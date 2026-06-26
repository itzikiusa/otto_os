import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';

// Proof Packs E2E. Drives the proof API against the isolated E2E daemon (the
// status/badges are DERIVED server-side, so the API responses are the contract),
// exercises the session done-gate via the tasks endpoint (no live agent needed),
// and renders the Proof page in the desktop browser. Runs in the desktop-browser
// project only.

const V1 = '/api/v1';

test.describe('proof packs', () => {
  test.beforeEach(({ }, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  });

  test('derived status + badges over the artifact lifecycle', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);

    // 1. Create a manual pack → missing / no_proof.
    let r = await ctx.post(`${base}${V1}/workspaces/${ws}/proof-packs`, {
      data: { work_item_kind: 'manual', work_item_id: `wi-${Date.now()}`, title: 'E2E proof' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    let pack = await r.json();
    const packId: string = pack.id;
    expect(pack.status).toBe('missing');
    expect(pack.badges).toContain('no_proof');

    // 2. Add a diff artifact + a passing test command → passed + tests_passed.
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'diff', title: 'Working tree diff', content: 'diff --git a b\n+1', status: 'info' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'cargo test --workspace', content: 'ok. 10 passed', status: 'passed' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    pack = await r.json();
    // Manual pack is lenient, but the badge logic recognizes the passing test.
    expect(pack.badges).toContain('tests_passed');
    expect(pack.status).toBe('passed');

    // 3. Add a FAILED test command → failed + tests_failed.
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: { kind: 'command', title: 'go test ./...', content: 'FAIL', status: 'failed' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    pack = await r.json();
    expect(pack.status).toBe('failed');
    expect(pack.badges).toContain('tests_failed');

    // 4. Detail shows the artifacts with previews.
    r = await ctx.get(`${base}${V1}/proof-packs/${packId}`);
    expect(r.ok()).toBeTruthy();
    const detail = await r.json();
    expect(detail.artifacts.length).toBe(3);
    const cmd = detail.artifacts.find((a: any) => a.title === 'cargo test --workspace');
    expect(cmd.preview).toContain('10 passed');

    // 5. Waive → waived + waived badge.
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/waive`, {
      data: { reason: 'verified manually in E2E' },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    pack = await r.json();
    expect(pack.status).toBe('waived');
    expect(pack.badges).toContain('waived');

    // 6. Redaction: a secret in artifact content must not be stored verbatim.
    r = await ctx.post(`${base}${V1}/proof-packs/${packId}/artifacts`, {
      data: {
        kind: 'log', title: 'secret log',
        content: 'token Authorization: Bearer abcdef0123456789ABCDEF0123456789 end',
        status: 'info',
      },
    });
    const withSecret = await r.json();
    const secretArt = (await (await ctx.get(`${base}${V1}/proof-packs/${packId}`)).json()).artifacts
      .find((a: any) => a.title === 'secret log');
    const full = await (await ctx.get(`${base}${V1}/proof-artifacts/${secretArt.id}/content`)).json();
    expect(String(full.content)).not.toContain('abcdef0123456789ABCDEF0123456789');
    expect(withSecret.status).toBeDefined();

    await ctx.dispose();
  });

  test('session done-gate auto-creates a proof pack', async () => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const sid = await seedShellSession(ctx, base, ws);

    // Drive the tasks list to "all complete" → trips the session proof gate.
    const r = await ctx.put(`${base}${V1}/workspaces/${ws}/sessions/${sid}/tasks`, {
      data: { tasks: [{ title: 'do the thing', status: 'completed' }] },
    });
    expect(r.ok(), await r.text()).toBeTruthy();

    // The gate runs in the background; poll for the session's pack to appear.
    let found = false;
    for (let i = 0; i < 30 && !found; i++) {
      const list = await (
        await ctx.get(`${base}${V1}/workspaces/${ws}/proof-packs?work_item_kind=session&work_item_id=${sid}`)
      ).json();
      if (Array.isArray(list) && list.length > 0) {
        found = true;
        expect(list[0].work_item_kind).toBe('session');
        break;
      }
      await new Promise((res) => setTimeout(res, 500));
    }
    expect(found, 'session gate should auto-create a proof pack on all-done').toBeTruthy();

    await ctx.dispose();
  });

  test('Proof page renders packs with badges', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const wid = `wi-page-${Date.now()}`;
    const created = await (
      await ctx.post(`${base}${V1}/workspaces/${ws}/proof-packs`, {
        data: { work_item_kind: 'manual', work_item_id: wid, title: 'Page Proof Pack' },
      })
    ).json();
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'command', title: 'cargo test', content: 'ok', status: 'passed' },
    });
    await ctx.post(`${base}${V1}/proof-packs/${created.id}/artifacts`, {
      data: { kind: 'diff', title: 'Working tree diff', content: 'diff', status: 'info' },
    });
    await ctx.dispose();

    await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
    await page.goto('/#/proof');
    // The page should render and show our pack title somewhere.
    await expect(page.getByText('Page Proof Pack').first()).toBeVisible({ timeout: 30_000 });
    // A passed/tests badge should appear.
    await expect(page.getByText(/Tests/i).first()).toBeVisible({ timeout: 10_000 });
  });
});
