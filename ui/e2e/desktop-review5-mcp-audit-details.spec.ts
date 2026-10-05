import {test, expect} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';
import {expectFullyInViewport} from './helpers';

test.use({serviceWorkers: 'block'});
test('audit full identities and failure details are reachable by keyboard and phone disclosure', async ({page}, info) => {
  const {ctx, base} = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
  const server = 'Shared-prefix-'.repeat(15) + 'server-final-identity';
  const tool = 'Shared-prefix-'.repeat(15) + 'tool-final-identity';
  const error = 'The destination rejected this request. ' + 'Retry after checking the named destination. '.repeat(12);
  await page.route('**/api/v1/mcp/audit?*', route => route.fulfill({json: [{
    id: 'audit-detail', workspace_id: workspace, server_id: null, server_name: server, tool,
    direction: 'inbound', caller_user_id: null, caller_kind: 'agent', args_redacted_json: '{}',
    decision: 'allowed', decision_reason: 'Read access is allowed', risk_label: 'read', injection_risk: 'low',
    dry_run: false, ok: false, error, latency_ms: 3, bytes: 120, rows: null, approval_id: null,
    caller_session_id: null, created_at: '2026-10-05T00:00:00Z',
  }]}));
  await page.goto('/#/mcp/activity');
  const details = page.getByTestId('mcp-audit').locator('details').filter({has: page.getByText('Call details', {exact: true})});
  const disclosure = details.locator('summary');
  await expect(disclosure).toBeVisible();
  await disclosure.focus(); await page.keyboard.press('Enter');
  await expect(details.getByText(server, {exact: true})).toBeVisible();
  await expect(details.getByText(tool, {exact: true})).toBeVisible();
  await expect(details.getByText(error, {exact: true})).toBeVisible();
  await page.setViewportSize({width: 390, height: 844});
  await disclosure.click(); await expect(details).not.toHaveAttribute('open', '');
  await disclosure.click();
  for (const scheme of ['light', 'dark'] as const) {
    await page.evaluate(next => {document.documentElement.dataset.scheme = next;}, scheme);
    await details.getByText(server, {exact: true}).scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, details.getByText(server, {exact: true}));
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.screenshot({path: info.outputPath(`audit-phone-${scheme}.png`), fullPage: true});
    await details.getByText(error, {exact: true}).scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, details.getByText(error, {exact: true}));
    await page.screenshot({path: info.outputPath(`audit-error-phone-${scheme}.png`), fullPage: true});
  }
});
