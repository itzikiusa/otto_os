import { test, expect } from '@playwright/test';

// Personal-agent autonomy (modes, goals, rules) + the memory inspector, mounted
// in a fixture page against routed APIs — no daemon needed.

test('autonomy: proactive budget, goals and rules save; enforced rules are labelled', async ({ page }) => {
  let saved = {
    proactive: { enabled: false, runs_per_day: 4, max_minutes: 15 },
    goals: [] as { id: string; text: string; enabled: boolean; last_run_at: string | null }[],
    rules: [] as { id: string; text: string; enforce: { kind: string; terms: string[] } | null }[],
    primary: false,
  };
  let lastBody: Record<string, unknown> | null = null;
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/autonomy')) {
      if (route.request().method() === 'PUT') {
        const body = route.request().postDataJSON();
        lastBody = body;
        saved = {
          proactive: body.proactive,
          goals: body.goals.map((g: { id?: string; text: string; enabled?: boolean }, i: number) => ({ id: g.id ?? `g${i}`, text: g.text, enabled: g.enabled ?? true, last_run_at: null })),
          rules: body.rules.map((r: { id?: string; text: string }, i: number) => ({
            id: r.id ?? `r${i}`,
            text: r.text,
            enforce: /ask before/i.test(r.text) && /prod/i.test(r.text) ? { kind: 'approval', terms: ['prod'] } : null,
          })),
          primary: body.primary,
        };
      }
      return route.fulfill({ json: saved });
    }
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/personal-autonomy.html');
  await expect(page.getByRole('heading', { name: 'Permission modes' })).toBeVisible();
  await expect(page.getByText('can’t send, post, write or change anything')).toBeVisible();
  await expect(page.getByText('Account, credential and sharing actions always ask you first')).toBeVisible();
  const save = page.getByRole('button', { name: 'Save', exact: true });
  await expect(save).toBeDisabled();
  await page.getByLabel('Work on standing goals in the background').check();
  await page.getByLabel('Runs per day (max)').fill('6');
  await page.getByRole('button', { name: 'Add goal' }).click();
  await page.getByLabel('Goal', { exact: true }).fill('Watch release CI');
  await page.getByRole('button', { name: 'Add rule' }).click();
  await page.getByLabel('Rule', { exact: true }).fill('Ask before touching prod');
  await save.click();
  await expect(page.getByText('Enforced: asks you before actions mentioning “prod”')).toBeVisible();
  expect(lastBody).toMatchObject({ proactive: { enabled: true, runs_per_day: 6 }, goals: [{ text: 'Watch release CI' }] });
  await expect(save).toBeDisabled();
  // Read-only viewers see the settings but can't change them.
  await page.goto('/e2e/fixtures/personal-autonomy.html?viewer');
  await expect(page.getByLabel('Work on standing goals in the background')).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toHaveCount(0);
});

test('memory inspector: sources, filter, edit and forget are content-checked', async ({ page }) => {
  let doc = '# Scout\n- [slack] Dana owns billing\n- [run] CI flaky on mac\n- Prefers short summaries\n';
  let version = 'v1';
  const parse = () => doc.split('\n').flatMap((raw, line) => {
    const m = /^- (?:\[(\w+)\] )?(.*)$/.exec(raw);
    return m ? [{ line, raw, text: m[2], source: m[1] ?? 'notes', section: 'Scout' }] : [];
  });
  const edits: unknown[] = [];
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/memories/edit')) {
      const body = route.request().postDataJSON();
      edits.push(body);
      if (body.version !== version) return route.fulfill({ status: 409, json: { code: 'conflict', message: 'changed' } });
      const lines = doc.split('\n');
      if (body.text === null) lines.splice(body.line, 1);
      else lines[body.line] = body.raw.replace(/\] .*$/, `] ${body.text}`);
      doc = lines.join('\n');
      version = 'v2';
    }
    if (url.pathname.includes('/memories')) return route.fulfill({ json: { version, exists: true, path: '/x/memory/notes.md', items: parse() } });
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/personal-autonomy.html?view=memory');
  await expect(page.getByText('Dana owns billing')).toBeVisible();
  await expect(page.getByRole('group', { name: 'Filter by source' })).toBeVisible();
  await page.getByRole('button', { name: 'Slack', exact: true }).click();
  await expect(page.getByText('CI flaky on mac')).toHaveCount(0);
  await page.getByRole('button', { name: 'All', exact: true }).click();
  await page.getByRole('button', { name: 'Edit memory' }).first().click();
  await page.getByLabel('Memory text').fill('Dana owns payments');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Dana owns payments')).toBeVisible();
  await page.getByRole('button', { name: 'Forget memory' }).nth(1).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Forget' }).click();
  await expect(page.getByText('CI flaky on mac')).toHaveCount(0);
  expect(edits).toHaveLength(2);
  expect(edits[1]).toMatchObject({ raw: '- [run] CI flaky on mac', text: null });
});
