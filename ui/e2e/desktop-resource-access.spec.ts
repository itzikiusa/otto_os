import { test, expect } from '@playwright/test';

// Contract mocks must receive requests directly, including mobile WebKit.
test.use({ serviceWorkers: 'block' });

test('root creates reusable access group', async ({ page }) => {
  let created = false;
  await page.route('**/api/v1/access/groups', async (route) => {
    if (route.request().method() === 'POST') {
      expect(route.request().postDataJSON().name).toBe('Database readers');
      created = true;
      await route.fulfill({
        json: { id: 'group-readers', name: 'Database readers', description: null },
      });
    } else
      await route.fulfill({
        json: created ? [{ id: 'group-readers', name: 'Database readers', description: null }] : [],
      });
  });
  await page.route('**/api/v1/access/roles', (route) => route.fulfill({ json: [] }));
  await page.route('**/api/v1/access/groups/group-readers/members', (route) =>
    route.fulfill({ json: [] }),
  );
  await page.goto('/#/settings/access-groups');
  await expect(page.getByRole('heading', { name: 'Groups & access roles' })).toBeVisible();
  await page.getByLabel('Group name').fill('Database readers');
  await page.getByRole('button', { name: 'Create group', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Database readers', exact: true })).toBeVisible();
  expect(created).toBeTruthy();
});

// Browser contract test: real isolated workspace/connection, mocked new policy
// endpoint. This tests preview-token binding and editor behavior, not backend auth.
import { apiCtx, seedWorkspace } from './seed';
import type { AccessPolicy } from '../src/lib/api/types';

test('policy activation binds preview to exact rules, shows child impact and saves user deny', async ({
  page,
}, testInfo) => {
  test.setTimeout(90_000);
  const fixtureName = `Access fixture ${testInfo.project.name}`;
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, {
    data: {
      name: fixtureName,
      kind: 'ssh',
      params: { host: 'example.invalid', user: 'reader' },
      secret: null,
      environment: 'dev',
      read_only: false,
    },
  });
  expect(response.ok()).toBeTruthy();
  const connection = (await response.json()).id;
  await ctx.dispose();
  let policy: AccessPolicy = {
    kind: 'connection',
    resource_id: connection,
    mode: 'legacy',
    revision: 0,
    rules: [],
  };
  let lastPreview = '';
  let saved = false;
  let authorized = true;
  const decision = (allowed: boolean) => ({
    allowed,
    reason: allowed ? 'Matching group grant' : 'Direct user deny',
    matched_rule_ids: [],
    mode: 'enforced',
  });
  await page.route('**/api/v1/access/connection/**', async (route) => {
    if (!authorized)
      return route.fulfill({
        status: 403,
        json: { code: 'forbidden', message: 'Access management denied' },
      });
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/subjects'))
      return route.fulfill({
        json: {
          users: [{ id: 'alice', username: 'alice', display_name: 'Alice' }],
          groups: [{ id: 'readers', name: 'Readers' }],
          roles: [
            {
              id: 'reader-role',
              name: 'Reader preset',
              kind: 'connection',
              operations: ['discover', 'db_query'],
              grantable_operations: [],
            },
          ],
        },
      });
    if (url.pathname.endsWith('/capabilities'))
      return route.fulfill({
        json: {
          kind: 'connection',
          resource_id: connection,
          user_id: 'root',
          mode: 'legacy',
          child: null,
          operations: {},
        },
      });
    if (url.pathname.endsWith('/effective'))
      return route.fulfill({
        json: {
          kind: 'connection',
          resource_id: connection,
          user_id: 'alice',
          mode: 'enforced',
          child: 'shop',
          operations: { db_query: decision(true), db_schema: decision(false) },
        },
      });
    if (url.pathname.endsWith('/preview')) {
      lastPreview = JSON.stringify(route.request().postDataJSON().policy);
      return route.fulfill({
        json: {
          token: 'exact-preview',
          revision: 0,
          issues: [],
          changes: [
            {
              user_id: 'alice',
              display_name: 'Alice',
              before: {},
              after: {},
              children: [
                {
                  child: 'shop',
                  before: { db_query: decision(false) },
                  after: { db_query: decision(true) },
                },
              ],
            },
          ],
        },
      });
    }
    if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      expect(body.preview_token).toBe('exact-preview');
      expect(JSON.stringify(body.policy)).toBe(lastPreview);
      policy = { ...body.policy, revision: 1 };
      saved = true;
      return route.fulfill({ json: policy });
    }
    return route.fulfill({ json: policy });
  });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_connhub_filter', 'all');
  }, workspaceId);
  await page.goto('/#/connections');
  await expect(
    page.getByRole('button', { name: `Access for ${fixtureName}`, exact: true }),
  ).toBeVisible();
  const backdrop = page.locator('.drawer-backdrop');
  if (await backdrop.isVisible()) {
    const bounds = await backdrop.boundingBox();
    await backdrop.click({ position: { x: (bounds?.width ?? 430) - 5, y: 10 } });
  }
  await page.getByRole('button', { name: `Access for ${fixtureName}`, exact: true }).click();
  const editor = page.getByRole('region', { name: 'Resource access' });
  await editor.getByLabel('Access mode').selectOption('enforced');
  await editor.getByRole('button', { name: 'Add rule', exact: true }).click();
  const first = editor.getByRole('article', { name: 'Rule 1', exact: true });
  await first.getByLabel('Copy role preset').selectOption('reader-role');
  await expect(first.getByLabel('Query data', { exact: true }).first()).toBeChecked();
  await first.getByLabel('Scope', { exact: true }).selectOption('selected');
  await first.getByLabel('Named children').fill('shop');
  await editor.getByRole('button', { name: 'Review changes', exact: true }).click();
  await expect(editor.getByText('Alice · shop', { exact: true })).toBeVisible();
  await expect(editor.getByRole('button', { name: 'Activate enforced access' })).toBeEnabled();
  await editor.getByRole('button', { name: 'Add rule', exact: true }).click();
  await expect(editor.getByRole('button', { name: 'Activate enforced access' })).toBeDisabled();
  const second = editor.getByRole('article', { name: 'Rule 2', exact: true });
  await second.getByLabel('Subject type').selectOption('user');
  await second.getByLabel('Subject', { exact: true }).selectOption('alice');
  await second.getByLabel('Effect').selectOption('deny');
  await second.getByLabel('Modify schema', { exact: true }).check();
  await editor.getByRole('button', { name: 'Review changes', exact: true }).click();
  await editor.getByRole('button', { name: 'Activate enforced access' }).click();
  await expect(editor.getByRole('status')).toHaveText('Access saved.');
  expect(saved).toBeTruthy();
  expect(policy.rules[1]).toMatchObject({
    subject_kind: 'user',
    subject_id: 'alice',
    effect: 'deny',
    operations: ['db_schema'],
  });
  await editor.getByRole('button', { name: 'Effective access', exact: true }).click();
  await editor.getByLabel('Child scope').fill('shop');
  await editor.getByRole('button', { name: 'Check access', exact: true }).click();
  await expect(editor.getByText('Direct user deny', { exact: true })).toBeVisible();
  authorized = false;
  await page.evaluate(async () => {
    const path = '/src/lib/api/client.ts';
    const { setToken } = await import(path);
    setToken('narrowed-access-test-token');
  });
  await expect(editor.getByRole('article')).toHaveCount(0);
  await expect(editor).toHaveCount(0);
});

test('revocation discards cached results and rejects late cloud responses; token swaps clear identity data', async ({
  page,
}) => {
  let allowed = true;
  await page.route('**/api/v1/access/*/*/capabilities', (route) => {
    const parts = new URL(route.request().url()).pathname.split('/');
    const kind = parts.at(-3),
      id = parts.at(-2);
    return route.fulfill({
      json: {
        kind,
        resource_id: id,
        user_id: 'root',
        child: null,
        mode: 'enforced',
        operations: Object.fromEntries(
          ['discover', 'db_browse', 'db_query', 's3_list'].map((op) => [
            op,
            { allowed, reason: 'Test policy decision', matched_rule_ids: [], mode: 'enforced' },
          ]),
        ),
      },
    });
  });
  let release: () => Promise<void> = async () => {};
  let started: () => void = () => {};
  const inFlight = new Promise<void>((resolve) => (started = resolve));
  await page.route('**/api/v1/aws/accounts/cache-account/s3/buckets', async (route) => {
    release = () => route.fulfill({ json: { buckets: [{ name: 'late-private-bucket' }] } });
    started();
  });
  await page.goto('/#/settings/appearance');
  await expect(page.locator('.shell')).toBeVisible();
  await page.evaluate(async () => {
    const accessPath = '/src/lib/stores/resource-access.svelte.ts',
      dbPath = '/src/lib/stores/database.svelte.ts',
      awsPath = '/src/lib/stores/aws.svelte.ts';
    const { resourceAccess } = await import(accessPath),
      { database } = await import(dbPath),
      { aws } = await import(awsPath);
    await resourceAccess.load('connection', 'cache-db');
    await resourceAccess.load('aws_account', 'cache-account');
    database.selectedConnId = 'cache-db';
    database.openConnIds = ['cache-db'];
    database.tabs[0].statement = 'unfinished draft';
    database.tabs[0].result = { columns: [], rows: [['private-result']] };
    database.schemaRoot = [{ id: 'db:private' }];
    aws.s3Buckets = { 'cache-account': [{ name: 'private-bucket' }] };
    void aws.loadS3Buckets('cache-account');
  });
  await inFlight;
  allowed = false;
  await page.evaluate(async () => {
    const path = '/src/lib/stores/resource-access.svelte.ts';
    const { resourceAccess } = await import(path);
    await resourceAccess.refresh();
  });
  await release();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const d = '/src/lib/stores/database.svelte.ts',
          a = '/src/lib/stores/aws.svelte.ts';
        const { database } = await import(d),
          { aws } = await import(a);
        return {
          result: database.tabs[0].result,
          schema: database.schemaRoot,
          buckets: aws.s3Buckets,
          draft: database.tabs[0].statement,
        };
      }),
    )
    .toEqual({ result: null, schema: [], buckets: {}, draft: 'unfinished draft' });
  await page.evaluate(async () => {
    const path = '/src/lib/api/client.ts';
    const { setToken } = await import(path);
    setToken('test-different-bearer');
  });
  const identity = await page.evaluate(async () => {
    const p = '/src/lib/stores/database.svelte.ts';
    const { database } = await import(p);
    return { statement: database.tabs[0].statement, selected: database.selectedConnId };
  });
  expect(identity).toEqual({ statement: '', selected: null });
});
