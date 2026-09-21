import { test, expect } from '@playwright/test';

test('memory edits preserve notes, report conflicts, and folder selection never submits', async ({ page }) => {
  let saved = { content: '# Existing notes\nKeep this knowledge.', version: 'v1', exists: true, path: '/fixture/memory/notes.md' };
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.includes('/memory')) {
      if (route.request().method() === 'PUT') {
        const body = route.request().postDataJSON();
        if (body.version !== saved.version) return route.fulfill({ status: 409, json: { code: 'conflict', message: 'Memory changed since you opened it. Reload and reconcile your edits.' } });
        saved = { ...saved, content: body.content, version: 'v2' };
      }
      return route.fulfill({ json: saved });
    }
    if (url.pathname === '/api/v1/fs/browse') return route.fulfill({ json: {
      path: '/fixture/chosen', parent: '/fixture', is_git_repo: false, entries: [{ name: 'child', path: '/fixture/chosen/child', is_dir: true, is_git_repo: false }],
    } });
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/personal-documents.html');
  await expect(page.getByTestId('agent-document')).toContainText('Keep this knowledge.');
  await page.getByRole('button', { name: 'Edit memory' }).click();
  await page.getByLabel('Memory Markdown').fill('Manual notes retained');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByTestId('agent-document')).toContainText('Manual notes retained');
  await page.getByRole('button', { name: 'Edit memory' }).click();
  await page.getByLabel('Memory Markdown').fill('Stale editor text');
  saved = { ...saved, content: 'Agent updated this concurrently', version: 'v3' };
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Memory changed');
  await expect(page.getByLabel('Memory Markdown')).toHaveValue('Stale editor text');
  await page.getByRole('button', { name: 'Reload saved version' }).click();
  await expect(page.getByTestId('agent-document')).toContainText('Agent updated this concurrently');
  await page.getByTitle('Browse folders').click();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByLabel('Working directory')).toHaveValue('~/original');
  await page.getByTitle('Browse folders').click();
  await page.getByRole('button', { name: 'child', exact: true }).click();
  await expect(page.locator('output')).toHaveText('not submitted');
  await page.keyboard.press('Escape');
  await expect(page.getByTitle('Browse folders')).toBeFocused();
  await page.getByTitle('Browse folders').click();
  await page.getByRole('button', { name: 'Use this folder', exact: true }).click();
  await expect(page.getByLabel('Working directory')).toHaveValue('/fixture/chosen');
  await expect(page.locator('output')).toHaveText('not submitted');
  await page.goto('/e2e/fixtures/personal-documents.html?viewer');
  await expect(page.getByTestId('agent-document')).toContainText('Agent updated this concurrently');
  await expect(page.getByRole('button', { name: 'Edit memory' })).toHaveCount(0);
});


test('context stays separate, cancels edits, and inserts file and Vault references', async ({ page }) => {
  let saved = { content: 'User background', version: 'c1', exists: true, path: null };
  const browsed: string[] = [];
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/context')) {
      if (route.request().method() === 'PUT') saved = { ...saved, content: route.request().postDataJSON().content, version: 'c2' };
      return route.fulfill({ json: saved });
    }
    if (url.pathname.endsWith('/vaults')) return route.fulfill({ json: [{ id: 'v', name: 'Knowledge', root_path: '/vault', workspace_id: 'w' }] });
    if (url.pathname === '/api/v1/fs/browse') {
      browsed.push(url.searchParams.get('path') ?? '');
      return route.fulfill({ json: { path: '/vault', parent: '/', is_git_repo: false, entries: [{ name: 'notes.md', path: '/vault/notes.md', is_dir: false, is_git_repo: false }] } });
    }
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/personal-documents.html?kind=context');
  await page.getByRole('button', { name: 'Edit context' }).click();
  await page.getByLabel('Context Markdown').fill('Discard this');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByTestId('agent-document')).toContainText('User background');
  await page.getByRole('button', { name: 'Edit context' }).click();
  await page.getByRole('button', { name: 'Add file reference' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  await page.getByRole('button', { name: 'notes.md select', exact: true }).click();
  await expect(page.getByLabel('Context Markdown')).toHaveValue(/Reference: \/vault\/notes.md/);
  await page.getByRole('button', { name: 'Add Vault reference' }).click();
  await page.getByRole('button', { name: 'Choose note' }).click();
  await page.getByRole('button', { name: 'notes.md select', exact: true }).click();
  expect(browsed).toContain('/vault');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByTestId('agent-document')).toContainText('User background');
  expect(saved.content.match(/Reference: \/vault\/notes.md/g)).toHaveLength(2);
  await page.reload();
  await expect(page.getByTestId('agent-document')).toContainText('Reference: /vault/notes.md');
});
