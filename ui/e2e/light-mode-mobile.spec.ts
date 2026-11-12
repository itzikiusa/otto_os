import { test, expect, type Page } from '@playwright/test';
import { openPage, expectNoHorizontalOverflow, expectContentHasHeight } from './helpers';

// The phone's light version (lib/lightMode.svelte.ts): opt-in per device; the
// full app stays the default. On: exactly five tabs (Threads, Connections,
// Git, API, Usage) and a compact ⋯, Threads is the landing page, any other
// module is a calm "not in the light version" page whose chunk never loads,
// and one tap goes back to the full app. A phone on its side keeps the tablet
// layout with the same five modules.

// Every chunk must reach the network listener: a service worker answering
// from its cache would hide a hidden page's chunk from `page.on('request')`.
test.use({ serviceWorkers: 'block' });

const LIGHT = ['threads', 'connections', 'git', 'api', 'usage'];
/** Page chunks of modules the light version hides (dev: source path; build: hashed name). */
const HIDDEN_CHUNK = /\/(AgentsPage|HomePage|VaultPage|WorkflowsPage|KubernetesPage|AwsPage|McpPage|ProductPage|InsightsPage|LoopsPage)[.-]/;

async function phoneSized(page: Page): Promise<boolean> {
  return page.evaluate(
    () => window.innerWidth <= 640 || (window.innerWidth > window.innerHeight && window.innerHeight <= 500),
  );
}
const portrait = (page: Page) => page.evaluate(() => window.innerWidth <= 640);

/** Boot with the light version already chosen on this device. */
async function bootLight(page: Page, route: string, hidden: string[] = []): Promise<void> {
  page.on('request', (r) => {
    if (HIDDEN_CHUNK.test(new URL(r.url()).pathname)) hidden.push(r.url());
  });
  await page.addInitScript(() => localStorage.setItem('otto_phone_light', '1'));
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

const bottomNav = (page: Page) => page.locator('nav.bottomnav');
const more = (page: Page) => bottomNav(page).getByRole('button', { name: /^More/ });
const sheet = (page: Page) => page.getByRole('dialog', { name: 'More' });
const navIds = (page: Page) =>
  bottomNav(page).locator('[data-nav-id]').evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.navId));

test('full app by default; one tap to the light version and one tap back', async ({ page }) => {
  await openPage(page, 'home');
  test.skip(!(await portrait(page)), 'the bottom nav is the phone-portrait shell');

  // Default: the full app's bar (four tabs + a labelled More), unchanged.
  await expect(more(page)).toContainText('More');
  expect((await navIds(page)).length).toBeGreaterThan(0);

  // More → Light version (the first row): lands on Threads, five tabs, a compact ⋯.
  await more(page).click();
  const toggle = sheet(page).getByTestId('light-toggle');
  await expect(toggle).toContainText('Light version');
  await toggle.click();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/threads');
  await expect(page.getByTestId('threads-page')).toBeVisible();
  await expect.poll(() => navIds(page)).toEqual(LIGHT);
  await expect(more(page)).not.toContainText('More');
  expect(await page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('1');
  await expectNoHorizontalOverflow(page);

  // The bare route lands on Threads after a reload (remembered per device).
  await page.goto('/#/');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/threads');
  await expect.poll(() => navIds(page)).toEqual(LIGHT);

  // The ⋯ sheet: back to the full app, notifications, Commands, Settings, sign out.
  await more(page).click();
  await expect(sheet(page).getByTestId('light-toggle')).toContainText('Full app');
  await expect(sheet(page).getByTestId('more-notifications').getByRole('button', { name: /^Notifications/ })).toBeVisible();
  for (const name of ['Commands', 'Settings', 'Sign out']) {
    await expect(sheet(page).getByRole('button', { name, exact: true })).toBeVisible();
  }
  await expect(sheet(page).getByRole('button', { name: 'Help', exact: true })).toHaveCount(0);
  await sheet(page).getByTestId('light-toggle').click();
  await expect(more(page)).toContainText('More');
  await expect.poll(async () => (await navIds(page)).length).toBeLessThanOrEqual(4);
  expect(await page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('0');
});

test('a hidden module is a calm page, its code never loads, and "Open the full app" shows it', async ({ page }) => {
  test.setTimeout(90_000);
  const hidden: string[] = [];
  await bootLight(page, 'kubernetes', hidden);
  test.skip(!(await phoneSized(page)), 'phone-sized screens only');

  const notice = page.getByRole('heading', { name: 'Kubernetes isn’t in the light version' });
  await expect(notice).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('button', { name: 'Go to Threads' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(hidden, 'no hidden page chunk is requested').toEqual([]);

  // Full app: the same route now shows the real page.
  await page.getByRole('button', { name: 'Open the full app' }).click();
  await expect(notice).toHaveCount(0);
  await expect(page.getByTestId('k8s-page')).toBeVisible({ timeout: 20_000 });
  expect(await page.evaluate(() => location.hash)).toBe('#/kubernetes');
  expect(await page.evaluate(() => localStorage.getItem('otto_phone_light'))).toBe('0');
});

test('the five pages open, fit the width and never pull in a hidden page', async ({ page }) => {
  // Five cold page loads: on its side (the tablet layout) it ran past the default 45 s.
  test.setTimeout(120_000);
  const hidden: string[] = [];
  await bootLight(page, '', hidden);
  test.skip(!(await phoneSized(page)), 'phone-sized screens only');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/threads');

  if (await portrait(page)) {
    await expect.poll(() => navIds(page)).toEqual(LIGHT);
  } else {
    // On its side: the tablet layout (sidebar, no bottom nav) with the same five.
    await expect(bottomNav(page)).toHaveCount(0);
    const sidebar = page.locator('.sidebar');
    await expect(sidebar.locator('[data-nav-id="git"]').first()).toBeVisible();
    for (const id of ['agents', 'home', 'kubernetes', 'vault', 'workflows']) {
      await expect(sidebar.locator(`[data-nav-id="${id}"]`)).toHaveCount(0);
    }
  }

  for (const id of LIGHT) {
    await openPage(page, id);
    await expect(page.getByText('isn’t in the light version')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Page not found' })).toHaveCount(0);
    await expectContentHasHeight(page);
    await expectNoHorizontalOverflow(page);
  }
  expect(hidden, 'no hidden page chunk is requested').toEqual([]);
});

test('⌘K "Go to" lists only the five modules', async ({ page }) => {
  await bootLight(page, 'git');
  test.skip(!(await portrait(page)), 'opened from the bottom nav’s ⋯');
  await more(page).click();
  await sheet(page).getByRole('button', { name: 'Commands', exact: true }).click();
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  const search = palette.getByLabel('Search commands');
  // A command whose title is exactly the module's Go-to: not the palette's
  // `Ask Otto: "Go to …"` row, a search hit, or a "Go to <x> settings" section.
  const goTo = (label: string) =>
    palette.locator('.pal-item-title').filter({ hasText: new RegExp(`^Go to ${label}$`) });
  for (const label of ['Threads', 'Connections', 'Git', 'API', 'Usage']) {
    await search.fill(`Go to ${label}`);
    await expect(goTo(label).first()).toBeVisible();
  }
  for (const label of ['Agents', 'Home', 'Kubernetes', 'Vault', 'Workflows', 'Help']) {
    await search.fill(`Go to ${label}`);
    // The palette has re-ranked for this query once the Ask Otto row echoes it.
    await expect(palette.getByRole('option', { name: `Ask Otto: "Go to ${label}"` })).toBeVisible();
    await expect(goTo(label)).toHaveCount(0);
  }
  // Settings sections that moved into a hidden module are not offered either.
  for (const moved of ['Bundled skills (Skills Lab)', 'Evaluator defaults', 'MCP servers (Control Plane)']) {
    await search.fill(`Go to ${moved}`);
    await expect(palette.getByRole('option', { name: `Ask Otto: "Go to ${moved}"` })).toBeVisible();
    await expect(palette.locator('.pal-item-title').filter({ hasText: `Go to ${moved}` }).filter({ hasNotText: 'Ask Otto' })).toHaveCount(0);
  }
});
