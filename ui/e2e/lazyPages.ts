import { expect, type Locator, type Page } from '@playwright/test';
import { apiCtx, seedVaultDir, seedWorkspace } from './seed';
import { seedProductStory } from './seed-product';

// Shared check for the page diets (effort 2, D1–D7): a heavy view is NOT
// fetched with its page; it loads on first use behind a calm placeholder (the
// shared LoadState skeleton) and the page chrome does not move when it lands.
// Runs in MOCK mode (ui/src/lib/api/mock.ts), so no backend fixtures.

export async function mockMode(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
  });
}

export interface LazyCase {
  /** Hash route that shows the page WITHOUT the heavy view. */
  overview: string;
  /** Visible once that page has painted. */
  painted: (page: Page) => Locator;
  /** Source path suffix of the lazy module (Vite dev serves one URL per module). */
  module: string;
  /** Opens the heavy view (a click/tap or a hash change). */
  open: (page: Page) => Promise<void>;
  /** The placeholder's accessible name: LoadState says "Loading {what}". */
  placeholder: string;
  /** Visible once the lazy view has rendered. */
  ready: (page: Page) => Locator;
  /** Chrome that must not move when the view lands (default: the page header). */
  anchor?: (page: Page) => Locator;
}

export async function expectLazyOnFirstUse(page: Page, c: LazyCase): Promise<void> {
  const fetched: string[] = [];
  const isModule = (u: URL) => u.pathname.endsWith(c.module);
  // Hold the module until the placeholder has been seen, so the check does
  // not depend on how fast the dev server answers. Recorded here, not from
  // page 'request' events: WebKit reports a held request only once it goes on.
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  await page.route(isModule, async (route) => {
    fetched.push(route.request().url());
    await gate;
    await route.continue();
  });

  await page.goto('/' + c.overview);
  await expect(c.painted(page)).toBeVisible({ timeout: 20_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  expect(fetched, `${c.module} must not load with the page`).toEqual([]);

  const anchor = (c.anchor ?? ((p: Page) => p.getByTestId('page-header').first()))(page);
  const before = await anchor.boundingBox();
  await c.open(page);
  await expect(page.getByRole('status', { name: c.placeholder })).toBeVisible();
  expect(fetched.length, `${c.module} loads on first use`).toBeGreaterThan(0);
  release();
  await expect(c.ready(page)).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('status', { name: c.placeholder })).toHaveCount(0);
  const after = await anchor.boundingBox();
  expect(after, 'the page chrome stays where it was').toEqual(before);
}

/** A click on desktop, a tap on the touch projects. */
export type Act = (target: Locator) => Promise<void>;

/** One case per dieted page. */
export const LAZY_CASES: Record<string, (act: Act) => LazyCase> = {
  aws: () => ({
    overview: '#/aws',
    painted: (p) => p.getByTestId('aws-account-card').first(),
    module: '/src/modules/aws/S3Browser.svelte',
    open: (p) => p.evaluate(() => (location.hash = '#/aws/aws_sandbox/s3')).then(() => undefined),
    placeholder: 'Loading the S3 view',
    ready: (p) => p.locator('.vt-title', { hasText: 'S3' }),
  }),
  kubernetes: (act) => ({
    overview: '#/kubernetes/k8s_dev/pods/payments/payments-api-7d9f8b6c4-x2kqp',
    painted: (p) => p.getByRole('tab', { name: /^Logs/ }),
    module: '/src/modules/kubernetes/LogsView.svelte',
    open: (p) => act(p.getByRole('tab', { name: /^Logs/ })),
    placeholder: 'Loading the logs',
    ready: (p) => p.getByTestId('k8s-logs-copy-kubectl'),
  }),
};

/** Vault runs on the real test daemon (no vault mock): a seeded vault, then
 *  the note editor (CodeMirror) loads on the first Edit, not with the page. */
export async function seedVault(page: Page): Promise<void> {
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const { vaultId } = await seedVaultDir(ctx, base, wsId);
  await ctx.dispose();
  await page.addInitScript(
    ({ wsId, vId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem(`otto_vault_last:${wsId}`, String(vId));
      localStorage.setItem(`otto_vault_view:${vId}`, 'read');
    },
    { wsId, vId: vaultId },
  );
}

export const vaultCase = (act: Act): LazyCase => ({
  overview: '#/vault',
  painted: (p) => p.locator('.tree').getByText('services', { exact: true }),
  module: '/src/lib/components/CodeEditor.svelte',
  open: async (p) => {
    const tree = p.locator('.tree');
    await act(tree.getByText('services', { exact: true }));
    await act(tree.getByText('auth-api', { exact: true }));
    await expect(p.locator('.read').getByRole('heading', { name: 'Overview' })).toBeVisible();
    await act(p.locator('.mode-btn[title^="Edit"]'));
  },
  placeholder: 'Loading the editor',
  ready: (p) => p.locator('.cm-editor'),
});

/** Product: each story tab is its own chunk, loaded when it is opened.
 *  Stories are global across workspaces, so the row is found by a unique title. */
export async function seedStory(page: Page): Promise<string> {
  const title = `E2E Lazy ${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const { storyId } = await seedProductStory(ctx, base, wsId);
  const r = await ctx.patch(`${base}/api/v1/product/stories/${storyId}/draft`, { data: { title, body_md: `# ${title}\n\nSeeded.` } });
  if (!r.ok()) throw new Error(`rename story → ${r.status()}`);
  await ctx.dispose();
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsId);
  return title;
}

export const productCase = (act: Act, title: string): LazyCase => ({
  overview: '#/product',
  painted: (p) => p.locator('.story-row', { hasText: title }).first(),
  module: '/src/modules/product/ChatTab.svelte',
  open: async (p) => {
    await act(p.locator('.story-row', { hasText: title }).first());
    await expect(p.locator('.overview')).toBeVisible({ timeout: 20_000 });
    await act(p.getByRole('tab', { name: 'Discover', exact: true }));
    await act(p.getByRole('tab', { name: 'Chat' }));
  },
  placeholder: 'Loading the chat',
  ready: (p) => p.locator('.chat-tab'),
});

/** Skills Lab: the multi-file editor (CodeMirror) loads on the Files tab. */
export const skillsCase = (act: Act): LazyCase => ({
  overview: '#/skills-eval',
  painted: (p) => p.getByTestId('skill-row').first(),
  module: '/src/modules/skills-lab/SkillEditor.svelte',
  open: async (p) => {
    await act(p.getByTestId('skill-row').first());
    await act(p.getByRole('tab', { name: 'Files' }));
  },
  placeholder: 'Loading the editor',
  ready: (p) => p.locator('.cm-editor'),
});
