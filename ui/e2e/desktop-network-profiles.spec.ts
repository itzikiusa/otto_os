import { test, expect } from '@playwright/test';

test('network profiles preserve conflict drafts, select saved endpoints, and show tunnel failures', async ({ page }) => {
  let profiles: Record<string, unknown>[] = [];
  let conflict = false;
  let failed = false;
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    const method = route.request().method();
    if (path.endsWith('/connections')) return route.fulfill({ json: [{id: 'ssh', name: 'Bastion', kind: 'ssh'}, {id: 'db', name: 'Database', kind: 'postgres'}] });
    if (path.endsWith('/network-profiles') || path.endsWith('/network-profiles/p')) {
      if (method === 'PUT' && conflict) return route.fulfill({status:409, json:{code:'conflict',message:'Profile changed. Reload before saving.'}});
      if (method === 'POST' || method === 'PUT') profiles = [{...route.request().postDataJSON(), id:'p', workspace_id:'w', version:2}];
      return route.fulfill({json: method === 'POST' || method === 'PUT' ? profiles[0] : path.includes('/other/') ? [] : profiles});
    }
    if (path.endsWith('/network')) return route.fulfill({json:{profile_id:'p',profile_name:'Private DB',profile_version:1,selected_profile_id:'p',restart_required:true,status:failed?'error':'connected',error:failed?'SSH exited; restart this session':null,endpoints:failed?[]:[{name:'DB',host:'127.0.0.1',port:43123,remote_host:'db.internal',remote_port:5432,host_env:'PGHOST',port_env:'PGPORT'}]}});
    return route.fulfill({json:[]});
  });
  await page.goto('/e2e/fixtures/network-profiles.html');
  await page.getByRole('button', {name:'Manage network profiles'}).click();
  await page.getByLabel('Network profile name').fill('Private DB');
  await page.getByLabel('SSH connection').selectOption('ssh');
  await page.getByLabel('Endpoint 1 name').fill('DB');
  await page.getByLabel('Endpoint 1 remote host').fill('db.internal');
  await page.getByLabel('Endpoint 1 remote port').fill('5432');
  await page.getByLabel('Endpoint 1 host environment').fill('PGHOST');
  await page.getByLabel('Endpoint 1 port environment').fill('PGPORT');
  await page.getByRole('button', {name:'Save network profile', exact:true}).click();
  await expect(page.getByTestId('selected')).toHaveText('p');
  expect(profiles[0].endpoints).toEqual([{name:'DB',remote_host:'db.internal',remote_port:5432,host_env:'PGHOST',port_env:'PGPORT'}]);
  await page.getByRole('button', {name:'Edit Private DB'}).click();
  await page.getByLabel('Network profile name').fill('Unsaved draft');
  conflict = true;
  await page.getByRole('button', {name:'Save network profile', exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('Profile changed');
  await expect(page.getByLabel('Network profile name')).toHaveValue('Unsaved draft');
  await page.getByText('Network: Private DB', {exact:false}).click();
  await expect(page.getByTestId('network-status')).toContainText('127.0.0.1:43123');
  await expect(page.getByTestId('network-status')).toContainText('PGHOST');
  await expect(page.getByTestId('network-status')).toContainText('Restart required');
  failed = true;
  await page.getByRole('button', {name:'Refresh network status'}).click();
  await expect(page.getByTestId('network-status')).toContainText('SSH exited');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('button', {name:'Other workspace'}).click();
  await expect(page.getByTestId('selected')).toHaveText('');
  await expect(page.getByRole('option', {name:'Private DB',exact:true})).toHaveCount(0);
});

test('workspace changes ignore delayed profile responses and archived profiles cannot be selected', async ({ page }) => {
  let release: (() => void) | undefined;
  const held = new Promise<void>((resolve) => release = resolve);
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/w/network-profiles')) {
      await held;
      return route.fulfill({json:[{id:'old',name:'Old workspace',archived:false}]}).catch(() => {});
    }
    if (path.endsWith('/other/network-profiles')) return route.fulfill({json:[{id:'archived',name:'Archived profile',archived:true}]});
    if (path.endsWith('/network')) return route.fulfill({json:{profile_id:null,profile_name:null,profile_version:null,selected_profile_id:'p',status:'stopped',restart_required:true,error:null,endpoints:[]}});
    return route.fulfill({json:[]});
  });
  await page.goto('/e2e/fixtures/network-profiles.html');
  await page.getByRole('button', {name:'Other workspace'}).click();
  await expect(page.getByLabel('Network profile', {exact:true})).toBeEnabled();
  const staleResponse = page.waitForResponse((response) => response.url().endsWith('/w/network-profiles'));
  release!();
  await staleResponse;
  await expect(page.getByRole('option', {name:'Old workspace',exact:true})).toHaveCount(0);
  await expect(page.getByRole('option', {name:'Archived profile',exact:true})).toHaveCount(0);
  await expect(page.getByTestId('network-status')).toContainText('stopped');
});
