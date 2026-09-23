import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Design Hall co-design (desktop-browser) — the Otto panel, the variants tray,
// the lobby's Generate hand-off and "What Otto learned", against the REAL
// /design/* assist routes. The isolated daemon runs with OTTO_E2E=1, so every
// agent turn is answered by the offline stub (crates/otto-orchestrator/src/
// e2e_stub.rs → `design_assist_reply`): an HTML fence titled "E2E design"
// for generate/refine/a11y/variants, a one-line "Top finding: …" for a
// critique, citing `[R1]` whenever a reference was offered. Needs a daemon
// built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).
//
//   • lobby Generate → the draft opens on its Otto tab, the turn goes live and
//     lands as a new (Otto) version with a verified R1 provenance chip;
//   • Check accessibility → a review that edits nothing;
//   • 3 variants → a tray of 3 cards; ✕ asks why and posts variant_rejected
//     with the reason chip; Apply fast-forwards main (daemon-recorded accept);
//   • What Otto learned → seeded reject signals become a pending rule, Approve
//     asks first and moves it to Rules; Roll back asks first; Settings writes
//     the workspace `design_learning` setting.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';
const stamp = Date.now().toString(36);
let wsId = '';
let refId = '';
let busyId = '';
const REF_TITLE = `Spring promo landing ${stamp}`;

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}
async function getJson(ctx: APIRequestContext, url: string): Promise<any> {
  const r = await ctx.get(url);
  expect(r.ok(), `${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}
async function html(ctx: APIRequestContext, base: string, title: string): Promise<string> {
  const res = await postJson(ctx, `${base}${V1}/design/artifacts`, {
    workspace_id: wsId,
    format: 'html',
    studio: 'frames',
    title,
    content: `<!doctype html><html><body><h1>${title}</h1></body></html>`,
  });
  return res.artifact.id as string;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  // A shipped library design the "Use references" preview finds for the brief.
  refId = await html(ctx, base, REF_TITLE);
  await postJson(ctx, `${base}${V1}/design/artifacts/${refId}/approve`, {});
  busyId = await html(ctx, base, `Checkout hero ${stamp}`);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function open(page: Page, route: string): Promise<void> {
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

test('lobby Generate opens the draft on Otto and the turn lands as a version', async ({ page }) => {
  test.setTimeout(120_000);
  await open(page, 'design');
  await page.getByTestId('design-prompt').fill(`Spring promo landing refresh ${stamp}`);
  // "Use references" is on by default and previews the team's closest match.
  await expect(page.getByTestId('design-prompt-ref-chips')).toContainText(REF_TITLE, { timeout: 15_000 });
  await page.getByTestId('design-prompt-generate').click();

  await expect(page).toHaveURL(/#\/design\/a\/[^/]+\/otto$/);
  const panel = page.getByTestId('design-otto-panel');
  await expect(panel).toBeVisible();
  const turn = panel.getByTestId('design-assist-turn').last();
  await expect(turn).toContainText('Spring promo landing refresh'); // the ask, remembered
  await expect(turn.getByTestId('design-assist-status')).toHaveText(/New version/, { timeout: 60_000 });
  await expect(turn).toContainText('Built the screen');
  // The stub cites [R1] — the reference the lobby offered first — and the
  // server verified it, so it is a clickable provenance chip.
  await expect(turn.getByTestId('design-prov-chip').first()).toContainText('R1');
  await expect(page.getByTestId('design-version-chip')).toHaveCount(2);
  await expectNoHorizontalOverflow(page);
});

test('Check accessibility is a review that changes nothing', async ({ page }) => {
  test.setTimeout(90_000);
  await open(page, `design/a/${busyId}/otto`);
  const panel = page.getByTestId('design-otto-panel');
  await expect(panel).toBeVisible();
  await expect(page.getByTestId('design-version-chip')).toHaveCount(1);
  await page.getByTestId('design-quick-a11y_check').click();
  const turn = panel.getByTestId('design-assist-turn').last();
  await expect(turn.getByTestId('design-assist-status')).toHaveText(/Review ready/, { timeout: 60_000 });
  await expect(turn).toContainText('Top finding');
  await expect(page.getByTestId('design-version-chip')).toHaveCount(1);
});

test('3 variants: reject asks why, Apply makes the pick the new version', async ({ page }) => {
  test.setTimeout(150_000);
  const { ctx, base } = await apiCtx();
  const id = await html(ctx, base, `Hero variants ${stamp}`);
  await open(page, `design/a/${id}/otto`);
  await page.getByTestId('design-quick-variants').click();

  const tray = page.getByTestId('design-variants-tray').last();
  await expect(tray.getByTestId('design-variant-card')).toHaveCount(3, { timeout: 30_000 });
  await expect(tray.locator('[data-testid="design-variant-card"][data-state="ready"]')).toHaveCount(3, { timeout: 90_000 });

  // ✕ on B asks why (a clamped global menu) and records the reason.
  await tray.getByTestId('design-variant-reject').nth(1).click();
  const menu = page.locator('.ctx-menu');
  await page.waitForTimeout(100);
  await expectFullyInViewport(page, menu, 'reject reasons');
  await menu.getByRole('menuitem', { name: 'Too busy' }).click();
  await expect(tray.getByTestId('design-variant-card').nth(1)).toContainText('Rejected · Too busy');
  await expect
    .poll(async () => {
      const s = await getJson(ctx, `${base}${V1}/design/signals?artifact_id=${id}&kind=variant_rejected`);
      return s.some((x: any) => x.payload?.reason === 'too_busy' && x.payload?.source === 'variant_tray');
    })
    .toBe(true);

  // Apply A → a new main version; the daemon records the accept.
  await tray.getByTestId('design-variant-apply').first().click();
  await expect(page.getByTestId('design-otto-panel')).toContainText(/Applied .* as v\d+/);
  await expect(tray.getByTestId('design-variant-card').first()).toHaveAttribute('data-state', 'accepted');
  await expect
    .poll(async () => (await getJson(ctx, `${base}${V1}/design/signals?artifact_id=${id}&kind=variant_accepted`)).length)
    .toBe(1);
  await ctx.dispose();
});

test('What Otto learned: a proposal is approved and rolled back only after asking', async ({ page }) => {
  test.setTimeout(90_000);
  const { ctx, base } = await apiCtx();
  // Three "too busy" rejects across two designs → a ready candidate.
  const a = await html(ctx, base, `Learn A ${stamp}`);
  const b = await html(ctx, base, `Learn B ${stamp}`);
  for (const art of [a, a, b]) {
    await postJson(ctx, `${base}${V1}/design/signals`, { artifact_id: art, kind: 'variant_rejected', payload: { reason: 'too_busy', source: 'variant_tray' } });
  }
  await open(page, 'design/learned/pending');
  await page.getByTestId('design-learned-extract').click();
  const card = page.getByTestId('design-pending-rule').first();
  await expect(card).toContainText(/too busy/i, { timeout: 20_000 });

  await card.getByTestId('design-rule-approve').click();
  const confirm = page.getByRole('dialog', { name: 'Approve team rule' });
  await expect(confirm).toContainText('roll it back');
  await confirm.getByRole('button', { name: 'Approve rule' }).click();
  await page.getByTestId('design-learned-tab-rules').click();
  const rule = page.getByTestId('design-active-rule').first();
  await expect(rule).toContainText(/too busy/i);

  // Evidence opens beside the rule.
  await rule.locator('.row-btn').click();
  await expect(page.getByTestId('design-learned-evidence')).toContainText('Variant rejected');

  // Roll back asks first.
  await rule.getByRole('button', { name: 'Rule actions' }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Roll back…' }).click();
  const rb = page.getByRole('dialog', { name: 'Roll back team rule' });
  await rb.getByRole('button', { name: 'Roll back' }).click();
  await expect(page.getByTestId('design-active-rule')).toHaveCount(0);

  // Settings → Off writes the workspace setting; back to Suggest only.
  await page.getByTestId('design-learned-tab-settings').click();
  const settings = page.getByTestId('design-learned-settings');
  await settings.getByRole('button', { name: 'Off' }).click();
  await expect
    .poll(async () => (await getJson(ctx, `${base}${V1}/workspaces/${wsId}`)).settings?.design_learning)
    .toBe('off');
  await settings.getByRole('button', { name: 'Suggest only' }).click();
  await expect
    .poll(async () => (await getJson(ctx, `${base}${V1}/workspaces/${wsId}`)).settings?.design_learning)
    .toBe('suggest');
  await ctx.dispose();
});
