import { test, expect, request, type APIRequestContext } from '@playwright/test';
import { randomUUID } from 'node:crypto';
import { apiCtx } from './seed';

// Regression for per-token rotation. Rotating one root-owned token must leave a
// sibling root token and a token owned by another user valid; the legacy status
// route must likewise leave scoped tokens alone.

interface TokenInfo {
  id: string;
  user_id: string;
  username: string;
  label?: string | null;
}

interface MintedToken {
  token: string;
  info: TokenInfo;
}

interface OutwardStatus {
  enabled: boolean;
  tools: { name: string; enabled: boolean }[];
}

async function createSecondUser(ctx: APIRequestContext, base: string): Promise<string> {
  const username = `mcp-rotate-${Date.now()}-${randomUUID().slice(0, 8)}`;
  const created = await ctx.post(`${base}/api/v1/users`, {
    data: { username, password: 'mcp-user-pw-123456', display_name: 'MCP Rotate User' },
  });
  expect(created.ok(), `create user → ${created.status()} ${await created.text()}`).toBeTruthy();
  return ((await created.json()) as { id: string }).id;
}

async function mint(
  ctx: APIRequestContext,
  base: string,
  data: { user_id?: string; label: string; scope: { allow_writes: boolean } },
): Promise<MintedToken> {
  const response = await ctx.post(`${base}/api/v1/mcp/tokens`, { data });
  expect(
    response.ok(),
    `mint ${data.label} → ${response.status()} ${await response.text()}`,
  ).toBeTruthy();
  return (await response.json()) as MintedToken;
}

async function toolsListStatus(base: string, token: string): Promise<number> {
  const tokenRequest = await request.newContext({
    extraHTTPHeaders: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
  });
  try {
    const response = await tokenRequest.post(`${base}/api/v1/mcp/http`, {
      data: { jsonrpc: '2.0', id: 1, method: 'tools/list' },
    });
    return response.status();
  } finally {
    await tokenRequest.dispose();
  }
}

test('rotating one token leaves every other scoped token working', async ({ page }) => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const cleanupIds = new Set<string>();
  let legacyBeforeIds: Set<string> | null = null;
  let originalStatus: OutwardStatus | null = null;

  try {
    const statusResponse = await ctx.get(`${base}/api/v1/mcp/otto-server`);
    expect(statusResponse.ok()).toBeTruthy();
    originalStatus = (await statusResponse.json()) as OutwardStatus;
    const enable = await ctx.patch(`${base}/api/v1/mcp/otto-server`, {
      data: { enabled: true, tools: ['otto.get_context_packet'] },
    });
    expect(
      enable.ok(),
      `enable outward server → ${enable.status()} ${await enable.text()}`,
    ).toBeTruthy();

    const secondUserId = await createSecondUser(ctx, base);
    const a1 = await mint(ctx, base, { label: 'rot-a1', scope: { allow_writes: false } });
    cleanupIds.add(a1.info.id);
    const a2 = await mint(ctx, base, { label: 'rot-a2', scope: { allow_writes: false } });
    cleanupIds.add(a2.info.id);
    const b1 = await mint(ctx, base, {
      user_id: secondUserId,
      label: 'rot-b1',
      scope: { allow_writes: false },
    });
    cleanupIds.add(b1.info.id);

    await page.goto('/#/mcp');
    await expect(page.locator('.otto')).toBeVisible({ timeout: 30_000 });
    await page.locator('[data-testid="mcp-expose-toggle"]').click();
    await expect(page.locator('[data-testid="mcp-tokens"]')).toBeVisible();

    const row = page.locator(
      `[data-testid="mcp-token-row"][data-token-id="${a1.info.id}"]`,
    );
    await row.locator('[data-testid="mcp-token-rotate"]').click();
    const confirm = page.locator('.sheet[role="dialog"][aria-label="Rotate token"]');
    await expect(confirm).toBeVisible();
    await confirm.getByRole('button', { name: 'Rotate', exact: true }).click();

    const rotatedCode = page.locator('[data-testid="mcp-rotated-token"] code.token:not(.cmd)');
    await expect(rotatedCode).toBeVisible({ timeout: 20_000 });
    const rotatedSecret = (await rotatedCode.innerText()).trim();
    expect(rotatedSecret).toBeTruthy();

    const listedResponse = await ctx.get(`${base}/api/v1/mcp/tokens`);
    expect(listedResponse.ok()).toBeTruthy();
    const listed = (await listedResponse.json()) as { tokens: TokenInfo[] };
    const ids = listed.tokens.map((token) => token.id);
    expect(ids).toContain(a2.info.id);
    expect(ids).toContain(b1.info.id);
    expect(ids).not.toContain(a1.info.id);
    const replacement = listed.tokens.find(
      (token) => token.label === 'rot-a1' && token.id !== a1.info.id,
    );
    if (replacement) cleanupIds.add(replacement.id);
    expect(replacement, 'the replacement keeps the selected token label').toBeTruthy();
    expect(ids).toContain(replacement!.id);

    expect(await toolsListStatus(base, a1.token), 'old selected token is revoked').toBe(401);
    expect(await toolsListStatus(base, rotatedSecret), 'replacement token works').toBe(200);
    expect(await toolsListStatus(base, a2.token), 'same-user sibling token still works').toBe(
      200,
    );
    expect(await toolsListStatus(base, b1.token), 'other-user token still works').toBe(200);

    legacyBeforeIds = new Set(listed.tokens.map((token) => token.id));
    const legacy = await ctx.patch(`${base}/api/v1/mcp/otto-server`, {
      data: { rotate_token: true },
    });
    expect(legacy.ok(), `legacy rotate → ${legacy.status()} ${await legacy.text()}`).toBeTruthy();

    const afterLegacyResponse = await ctx.get(`${base}/api/v1/mcp/tokens`);
    expect(afterLegacyResponse.ok()).toBeTruthy();
    const afterLegacy = (await afterLegacyResponse.json()) as { tokens: TokenInfo[] };
    for (const token of afterLegacy.tokens) {
      if (!legacyBeforeIds.has(token.id) && token.label === 'otto-mcp-server') cleanupIds.add(token.id);
    }

    expect(
      await toolsListStatus(base, a2.token),
      'legacy rotate leaves same-user scoped token',
    ).toBe(200);
    expect(
      await toolsListStatus(base, b1.token),
      'legacy rotate leaves other-user scoped token',
    ).toBe(200);
  } finally {
    const currentResponse = await ctx.get(`${base}/api/v1/mcp/tokens`);
    if (currentResponse.ok()) {
      const current = (await currentResponse.json()) as { tokens: TokenInfo[] };
      for (const token of current.tokens) {
        if (token.label === 'rot-a1' || token.label === 'rot-a2' || token.label === 'rot-b1') {
          cleanupIds.add(token.id);
        }
        if (
          legacyBeforeIds &&
          token.label === 'otto-mcp-server' &&
          !legacyBeforeIds.has(token.id)
        ) {
          cleanupIds.add(token.id);
        }
      }
    }
    for (const id of cleanupIds) {
      await ctx.delete(`${base}/api/v1/mcp/tokens/${id}`);
    }
    if (originalStatus) {
      await ctx.patch(`${base}/api/v1/mcp/otto-server`, {
        data: {
          enabled: originalStatus.enabled,
          tools: originalStatus.tools.filter((tool) => tool.enabled).map((tool) => tool.name),
        },
      });
    }
    await ctx.dispose();
  }
});
