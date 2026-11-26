import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openNewSessionSheet, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Custom agent providers are first-class EVERYWHERE (desktop-browser only).
//
// A custom provider (`grok`) registered in Settings → Providers must appear in
// every provider picker and reach the ⌘K planner — not just the built-in
// claude/codex/agy. This proves the single shared registry drives all surfaces:
//   • GET /meta.providers            → includes grok
//   • POST /orchestrate "open grok…" → plans a spawn_sessions grok (dynamic
//     enum + validation, via the deterministic E2E stub)
//   • New Session ⌘T sheet           → a grok provider card (allProviders)
//   • Scheduled Tasks new-task form   → grok in the provider <select> (allProviders;
//     this surface used to hardcode claude/codex/agy/shell)
// ─────────────────────────────────────────────────────────────────────────────

let wsId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  // Register a custom provider — reloads the live registry immediately.
  const r = await ctx.put(`${base}/api/v1/settings`, {
    data: { providers: { grok: { cmd: 'grok', args: ['--session-id', '{sid}'] } } },
  });
  if (!r.ok()) throw new Error(`register provider failed: ${r.status()} ${await r.text()}`);
  wsId = await seedWorkspace(ctx, base);
  // A session so the Agents page renders panes + TabBar (not the first-run coach).
  await seedShellSession(ctx, base, wsId);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
});

test('custom provider appears in /meta and the ⌘K planner', async () => {
  const { ctx, base } = await apiCtx();

  const meta = await (await ctx.get(`${base}/api/v1/meta`)).json();
  expect(meta.providers, 'meta.providers includes the custom provider').toContain('grok');

  // The planner (E2E stub) must offer grok because the prompt enum is now the
  // live registry — "open grok session" plans exactly a grok spawn.
  const resp = await ctx.post(`${base}/api/v1/workspaces/${wsId}/orchestrate`, {
    data: { text: 'open grok session', optimize: false, ai_fallback: false },
  });
  const body = await resp.text();
  expect(resp.ok(), `orchestrate → ${resp.status()} ${body}`).toBeTruthy();
  const plan = (JSON.parse(body) as { plan: { action: string; provider?: string }[] }).plan;
  expect(plan.length).toBeGreaterThan(0);
  expect(plan[0].action).toBe('spawn_sessions');
  expect(plan[0].provider).toBe('grok');

  await ctx.dispose();
});

test('New Session sheet lists the custom provider', async ({ page }) => {
  await openPage(page, 'agents');

  // Open the sheet via ⌘T (falls back to the TabBar + button).
  const dialog = await openNewSessionSheet(page);

  // The custom provider has its own selectable card…
  await expect(
    dialog.locator('.provider-card', { hasText: 'grok' }),
  ).toHaveCount(1, { timeout: 10_000 });
  // …and the built-ins are still there too (custom EXTENDS, never replaces).
  await expect(dialog.locator('.provider-card', { hasText: 'claude' })).toHaveCount(1);
});

test('Scheduled-task form offers the custom provider', async ({ page }) => {
  await openPage(page, 'scheduled-tasks');

  // "New task" reveals the form; its provider <select> used to be a hardcoded
  // claude/codex/agy/shell list and now comes from the live registry.
  await page.getByRole('button', { name: 'New task' }).click();
  const providerSelect = page.locator('select').filter({ has: page.locator('option', { hasText: /^claude$/ }) }).first();
  await expect(providerSelect).toBeVisible({ timeout: 10_000 });
  await expect(providerSelect.locator('option', { hasText: /^grok$/ })).toHaveCount(1);
});

test('Self-Improvement lists the custom provider and NOT non-agent tools', async ({ page }) => {
  await page.goto('/#/settings/self-improvement');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.waitForLoadState('networkidle').catch(() => {});

  const chips = page.locator('.provider-grid .provider-chip .mono');
  // The custom provider is offered…
  await expect(chips.filter({ hasText: /^grok$/ })).toHaveCount(1, { timeout: 10_000 });
  await expect(chips.filter({ hasText: /^claude$/ })).toHaveCount(1);
  // …and the earlier bug is gone: git/clickhouse are TOOLS, never providers.
  await expect(chips.filter({ hasText: /^git$/ })).toHaveCount(0);
  await expect(chips.filter({ hasText: /^clickhouse$/ })).toHaveCount(0);
});

test('Insights settings offers a custom provider for report generation', async ({ page }) => {
  await page.goto('/#/settings/insights');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.waitForLoadState('networkidle').catch(() => {});

  const providerSelect = page.locator('.agent-row select');
  await expect(providerSelect).toBeVisible({ timeout: 10_000 });
  await expect(providerSelect.locator('option', { hasText: /^grok$/ })).toHaveCount(1);
});

// Workflows: a step is added from the editor's "+ Step" menu on a blank
// workflow (New workflow → Start blank). Agent steps choose provider · model ·
// effort with the Threads picker (StepModel); loop sub-steps and Self-Improve
// keep their own provider controls.
async function addWfNode(page: import('@playwright/test').Page, label: string): Promise<void> {
  await page.goto('/#/workflows/all');
  await page.getByRole('button', { name: 'New workflow' }).first().click();
  await page.getByRole('button', { name: 'Start blank' }).click();
  await expect(page.getByTestId('wf-editor-canvas')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('page-header').first().getByRole('button', { name: 'Step' }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: label }).first().click();
  await expect(page.getByTestId('wf-inspector')).toBeVisible({ timeout: 10_000 });
}

test('Workflow agent step offers provider + model through the Threads picker', async ({ page }) => {
  await addWfNode(page, 'Agent');
  // Unset reads the ACTUAL runtime default (claude — resolve_provider's
  // fallback), never the alphabetically first provider.
  const chip = page.getByTestId('wf-step-model');
  await expect(chip).toHaveAttribute('aria-label', /Claude, model Default/);
  await chip.click();
  await expect(page.getByRole('dialog').or(page.locator('.ctx-menu, [role="listbox"]')).first()).toBeVisible();
});

test('prepare_context step exposes the agent picker', async ({ page }) => {
  await addWfNode(page, 'Prepare relevant data');
  await expect(page.getByTestId('wf-inspector').getByTestId('wf-step-model')).toHaveAttribute('aria-label', /Claude/);
});

test('loop sub-steps expose a Provider selector (not raw JSON only)', async ({ page }) => {
  await addWfNode(page, 'Repeat (Until)');
  // Add an agent sub-step → its Provider select appears with the default label.
  await page.getByRole('button', { name: 'Add step' }).click();
  const prov = page.getByTestId('wf-inspector').locator('.ls-prov').first();
  await expect(prov).toBeVisible();
  await expect(prov.locator('option[value=""]')).toHaveText('default (claude)');
  await expect(prov.locator('option', { hasText: /^grok$/ })).toHaveCount(1);
});

test('self_improve step exposes provider chips', async ({ page }) => {
  await addWfNode(page, 'Self-Improve (offer)');
  const chips = page.getByTestId('wf-inspector').locator('.rv-chip');
  // One chip per registered agent provider (claude + the custom grok present).
  await expect(chips.filter({ hasText: 'claude' })).toHaveCount(1);
  await expect(chips.filter({ hasText: 'grok' })).toHaveCount(1);
});

test('Excluding a provider hides it from /meta and every picker', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  try {
    // Exclude agy.
    const put = await ctx.put(`${base}/api/v1/settings`, {
      data: { disabled_providers: ['agy'] },
    });
    expect(put.ok(), `disable agy → ${put.status()}`).toBeTruthy();

    // /meta.providers no longer lists agy, but keeps the others.
    const meta = await (await ctx.get(`${base}/api/v1/meta`)).json();
    expect(meta.providers).not.toContain('agy');
    expect(meta.providers).toContain('claude');
    expect(meta.providers).toContain('grok');

    // The New Session sheet drops the agy card while keeping claude + grok.
    await openPage(page, 'agents');
    // Open the sheet via ⌘T (falls back to the TabBar + button).
    const dialog = await openNewSessionSheet(page);
    await expect(dialog.locator('.provider-card', { hasText: 'claude' })).toHaveCount(1);
    await expect(dialog.locator('.provider-card', { hasText: 'grok' })).toHaveCount(1);
    await expect(dialog.locator('.provider-card', { hasText: /^agy$/ })).toHaveCount(0);
  } finally {
    // Re-enable so the shared test daemon is left clean.
    await ctx.put(`${base}/api/v1/settings`, { data: { disabled_providers: [] } });
    await ctx.dispose();
  }
});

