import { expect, type Page } from '@playwright/test';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// An app's (MCP) approval in a thread's dock (MOCK mode; the pending card is
// seeded as `window.__mockMcpAsks`): a plain sentence first, the reason, the
// arguments folded behind "Details", and the same buttons as every other
// approval — Allow once (primary) · Always allow · Deny. Used by
// desktop-threads-mcp-ask.spec.ts and threads-mcp-ask-mobile.spec.ts.
// ─────────────────────────────────────────────────────────────────────────────

const CARD = {
  id: 'apr_1', workspace_id: 'wsp_otto', kind: 'tool_call', server_id: null, server_name: 'otto', tool: 'otto.create_pr',
  title: 'otto MCP server → create_pr', detail: 'Opens a pull request on itzik/otto from fix/cache.',
  args_redacted_json: JSON.stringify({ repo: 'itzik/otto', head: 'fix/cache', base: 'main', title: 'Cache the plan' }),
  risk_label: 'dangerous', status: 'pending', requested_by: 'u1', requested_by_kind: 'mcp_server', requested_by_session_id: 'rt_mcp',
  decided_by: null, decision_note: null, created_at: '2026-11-23T10:00:00Z', decided_at: null, consumed_at: null, expires_at: null,
};

async function open(page: Page): Promise<void> {
  await page.addInitScript((card) => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
    (window as unknown as { __mockMcpAsks: unknown[] }).__mockMcpAsks = [card];
  }, CARD);
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: { seed?: unknown } }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    (window as unknown as { __mockRuntime: { seed: (r: Record<string, unknown>) => string } }).__mockRuntime.seed({ id: 'rt_mcp', title: 'Open the PR', provider: 'claude', status: 'working', last_active_at: '2026-11-23T10:00:00Z' });
    location.hash = '#/threads/rt_mcp';
  });
}

/** The whole check; `tag` names the screenshots (`1600` / `phone`). */
export async function checkMcpAsk(page: Page, tag: string): Promise<void> {
  await open(page);
  const card = page.getByTestId('runtime-approval-dock-mcp:apr_1');
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('runtime-attention')).toContainText('otto · create_pr wants to create pr');
  await expect(card).toContainText('Opens a pull request on itzik/otto');
  const args = card.locator('.ra-detail');
  await expect(args).toBeHidden();
  const buttons = card.locator('.ra-actions button');
  // An MCP admin (the mock user) may also add a workspace rule.
  await expect(buttons).toHaveText([/^Allow once/, /^Always allow$/, /^Deny/]);
  await expect(card.getByRole('button', { name: 'Always allow' })).toHaveAttribute('title', 'Always allow in this workspace');
  await expect(card.getByRole('button', { name: 'Allow once' })).toHaveClass(/primary/);
  await expectFullyInViewport(page, card, 'MCP approval card');
  const dir = process.env.OTTO_E2E_SHOTS;
  if (dir) await page.screenshot({ path: `${dir}/mcp-ask-${tag}-light.png`, animations: 'disabled' });
  await card.getByText('Details', { exact: true }).click();
  await expect(args).toBeVisible();
  await expect(args).toContainText('"head": "fix/cache"');
  if (dir) {
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.screenshot({ path: `${dir}/mcp-ask-${tag}-details-dark.png`, animations: 'disabled' });
  }
}
