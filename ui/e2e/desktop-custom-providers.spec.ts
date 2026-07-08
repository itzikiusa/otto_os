import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage } from './helpers';

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

  // Open the ⌘T sheet; fall back to the TabBar + button if the shortcut doesn't
  // reach the app in this browser build (mirrors desktop-new-session-keys).
  const dialog = page.locator('.sheet[role="dialog"][aria-label="New Session"]');
  await page.keyboard.press('Meta+t');
  if (!(await dialog.isVisible().catch(() => false))) {
    await page.getByTitle('New session (⌘T)').click();
  }
  await expect(dialog).toBeVisible();

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

test('Run with Otto (single-agent) offers the custom provider', async ({ page }) => {
  await openPage(page, 'run-with-otto');

  // Single-agent mode is the default; its Provider <select> used to be a fixed
  // "claude" label and now comes from the live registry (both modes honor it).
  const providerSelect = page.locator('select[aria-label="Provider"]');
  await expect(providerSelect).toBeVisible({ timeout: 10_000 });
  await expect(providerSelect.locator('option', { hasText: /^grok$/ })).toHaveCount(1);
});

test('Insights settings offers a custom provider for report generation', async ({ page }) => {
  await page.goto('/#/settings/insights');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.waitForLoadState('networkidle').catch(() => {});

  const providerSelect = page.locator('.agent-row select');
  await expect(providerSelect).toBeVisible({ timeout: 10_000 });
  await expect(providerSelect.locator('option', { hasText: /^grok$/ })).toHaveCount(1);
});

test('Workflow agent node offers provider + model from the registry', async ({ page }) => {
  await openPage(page, 'workflows');

  // Start a blank workflow to open the editor (empty state otherwise).
  await page.getByRole('button', { name: 'Start blank' }).click();

  // Open the node palette and add an Agent node.
  const nodeBtn = page.locator('.menu-wrap button', { hasText: 'Node' });
  await expect(nodeBtn).toBeVisible({ timeout: 10_000 });
  await nodeBtn.click();
  await page.locator('.pal-item', { hasText: 'Agent' }).first().click();

  // The new node is auto-selected → its inspector shows the Provider select +
  // Model input (agent_prompt was Model-only before, no provider).
  const providerSelect = page.locator('#np-provider');
  await expect(providerSelect).toBeVisible({ timeout: 10_000 });
  await expect(providerSelect.locator('option', { hasText: /^grok$/ })).toHaveCount(1);
  await expect(page.locator('#np-model')).toBeVisible();
});

test('Workflow node inspector docks to a resizable side panel', async ({ page }) => {
  await openPage(page, 'workflows');
  await page.getByRole('button', { name: 'Start blank' }).click();

  // Add + select a node so the inspector shows.
  const nodeBtn = page.locator('.menu-wrap button', { hasText: 'Node' });
  await nodeBtn.click();
  await page.locator('.pal-item', { hasText: 'Agent' }).first().click();

  const inspector = page.locator('.inspector');
  await expect(inspector).toBeVisible({ timeout: 10_000 });
  // Bottom dock by default (no .side).
  await expect(inspector).not.toHaveClass(/\bside\b/);

  // Toggle to the side dock → the inspector becomes a right column.
  await page.getByRole('button', { name: 'Dock' }).click();
  await expect(inspector).toHaveClass(/\bside\b/);
  const box = await inspector.boundingBox();
  const vw = page.viewportSize()!.width;
  // It sits on the right half of the viewport (a real side column).
  expect(box!.x).toBeGreaterThan(vw / 2);

  // Drag the vertical grip left to widen it; width grows and persists.
  const before = box!.width;
  const grip = page.locator('.insp-grip.side');
  const gb = (await grip.boundingBox())!;
  await page.mouse.move(gb.x + gb.width / 2, gb.y + 200);
  await page.mouse.down();
  await page.mouse.move(gb.x - 140, gb.y + 200, { steps: 6 });
  await page.mouse.up();
  const after = (await inspector.boundingBox())!.width;
  expect(after).toBeGreaterThan(before + 80);

  // Toggle back to the bottom dock.
  await page.getByRole('button', { name: 'Dock' }).click();
  await expect(inspector).not.toHaveClass(/\bside\b/);
});

test('Dock opens a persistent side panel with a working close button', async ({ page }) => {
  await openPage(page, 'workflows');
  await page.getByRole('button', { name: 'Start blank' }).click();

  // Press Dock with NOTHING selected → the side panel appears immediately with a
  // centered placeholder (previously nothing happened, which is what the user hit).
  const inspector = page.locator('.inspector.side');
  await expect(inspector).toHaveCount(0);
  await page.getByRole('button', { name: 'Dock' }).click();
  await expect(inspector).toBeVisible({ timeout: 10_000 });
  await expect(inspector.locator('.insp-blank')).toContainText(/select a node/i);

  // The panel header hosts the notification bell (the shell's floating bell is
  // hidden in this layout) + the close (×).
  await expect(inspector.locator('.insp-side-head .bell-wrap')).toBeVisible();
  await expect(page.locator('.bell-anchor')).toHaveCount(0);

  // The panel reaches the right edge of the viewport — no gutter black-strip.
  const box = (await inspector.boundingBox())!;
  const vw = page.viewportSize()!.width;
  expect(box.x + box.width).toBeGreaterThan(vw - 4);

  // The × close button in the side header dismisses the dock; the floating shell
  // bell returns.
  await inspector.getByRole('button', { name: 'Close panel' }).click();
  await expect(page.locator('.inspector.side')).toHaveCount(0);
  await expect(page.locator('.bell-anchor')).toBeVisible();
  // Dock button returns to inactive (bottom mode).
  await expect(page.getByRole('button', { name: 'Dock' })).not.toHaveClass(/\bactive\b/);
});
