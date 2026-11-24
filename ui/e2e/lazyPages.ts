import { expect, type Locator, type Page } from '@playwright/test';

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
  page.on('request', (r) => {
    if (isModule(new URL(r.url()))) fetched.push(r.url());
  });
  // Hold the module until the placeholder has been seen, so the check does
  // not depend on how fast the dev server answers.
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  await page.route(isModule, async (route) => {
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
