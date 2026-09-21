import { test, expect } from '@playwright/test';

test('folder errors identify the attempted path and keep the last folder recoverable', async ({ page }) => {
  let denied = true;
  const visited: string[] = [];
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    if (!url.pathname.endsWith('/fs/browse')) return route.fulfill({json:{content:'fixture',version:'1',exists:true,path:'/fixture/notes.md'}});
    const path = url.searchParams.get('path') ?? '';
    visited.push(path);
    if (path === '/fixture/.ssh' && denied) return route.fulfill({status:403,json:{code:'forbidden',message:'OS permission denied while listing directory: /fixture/.ssh'}});
    const target = path === '/fixture/.ssh' ? path : '/fixture';
    return route.fulfill({json:{path:target,parent:'/',is_git_repo:false,entries:target==='/fixture'?[{name:'.ssh',path:'/fixture/.ssh',is_dir:true,is_git_repo:false}]:[]}});
  });
  await page.goto('/e2e/fixtures/personal-documents.html');
  await page.getByTitle('Browse folders').click();
  const picker = page.getByRole('dialog');
  await picker.getByLabel('Show hidden').check();
  await picker.getByRole('button',{name:'.ssh',exact:true}).click();
  await expect(picker.getByRole('alert')).toContainText('/fixture/.ssh');
  await expect(picker.getByTestId('attempted-folder')).toHaveText('/fixture/.ssh');
  await expect(picker.getByText('Last opened folder', {exact:true})).toBeVisible();
  await expect(picker.getByRole('button',{name:'Use this folder',exact:true})).toBeDisabled();
  await picker.getByRole('button',{name:'Return to last opened folder'}).click();
  await expect(picker.getByRole('alert')).toHaveCount(0);
  await picker.getByRole('button',{name:'.ssh',exact:true}).click();
  denied = false;
  await picker.getByRole('button',{name:'Retry',exact:true}).click();
  await expect(picker.locator('.crumb')).toHaveAttribute('data-path','/fixture/.ssh');
  await picker.getByRole('button',{name:'Use this folder',exact:true}).click();
  await expect(page.getByLabel('Working directory')).toHaveValue('/fixture/.ssh');
  expect(visited.filter((path) => path === '/fixture/.ssh')).toHaveLength(3);
});
