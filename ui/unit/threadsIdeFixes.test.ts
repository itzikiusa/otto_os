import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { GitAccount, Repo } from '../src/lib/api/types';
import {
  IDE_NAMED_PROJECTS_KEY,
  IDE_PROJECTS_CHANGED_EVENT,
  parseNamedIdeProjects,
} from '../src/modules/threads/ideProjectModel.ts';
import {
  IDE_PROJECTS_CHANGED_EVENT as LIB_PROJECTS_CHANGED_EVENT,
  LS_IDE_PROJECTS,
  parseNamedIdeProjectsJson,
} from '../src/lib/ideProjectsModel.ts';
import { loadSource } from './sourceHarness.ts';

test('threads ideProjectModel delegates Named-Project parsing and constants to lib/ideProjectsModel', () => {
  assert.equal(IDE_NAMED_PROJECTS_KEY, LS_IDE_PROJECTS);
  assert.equal(IDE_PROJECTS_CHANGED_EVENT, LIB_PROJECTS_CHANGED_EVENT);

  const rawJson = JSON.stringify([
    {
      id: 'proj_1',
      name: 'Core Services',
      primary_root: '/private/var/folders/demo/core/',
      folders: [{ path: '/private/var/folders/demo/core/' }, { path: '/ws/shared/' }],
    },
  ]);

  const fromThreads = parseNamedIdeProjects(rawJson);
  const fromLib = parseNamedIdeProjectsJson(rawJson);
  assert.equal(fromThreads.length, 1);
  assert.equal(fromThreads[0].pinned, true, 'omitted pinned defaults to true identically across IDE and Threads');
  assert.equal(fromThreads[0].primary_root, '/var/folders/demo/core');
  assert.deepEqual(fromThreads[0].folders, ['/var/folders/demo/core', '/ws/shared']);
  assert.equal(fromThreads[0].primary_root, fromLib[0].primaryRoot);
  assert.deepEqual(fromThreads[0].folders, fromLib[0].folders);
});

test('fetchNamedIdeProjectsFromServer delegates to api.bg.get("/ide/projects") and persists via storage helpers', async () => {
  const calls: string[] = [];
  const mod = loadSource(
    new URL('../src/lib/ideProjectsModel.ts', import.meta.url),
    {
      'lib/api/client': {
        api: {
          bg: {
            get: async (path: string) => {
              calls.push(path);
              return {
                projects: [
                  {
                    id: 'proj_srv',
                    name: 'Backend Suite',
                    primary_root: '/ws/backend',
                    folders: ['/ws/backend', { path: '/ws/common' }],
                  },
                ],
              };
            },
          },
        },
      },
    },
    {},
    { relative: true },
  ) as typeof import('../src/lib/ideProjectsModel.ts');

  const loaded = await mod.fetchNamedIdeProjectsFromServer();
  assert.deepEqual(calls, ['/ide/projects']);
  assert.ok(loaded);
  assert.equal(loaded.length, 1);
  assert.equal(loaded[0].id, 'proj_srv');
  assert.deepEqual([...loaded[0].folders], ['/ws/backend', '/ws/common']);
  assert.equal(loaded[0].pinned, true);

  const roundTripped = mod.loadNamedIdeProjectsLocal();
  assert.equal(roundTripped.length, 1);
  assert.equal(roundTripped[0].name, 'Backend Suite');
});

test('threadPrs skips gitApi.prs.list when repo has no provider or no configured GitAccount', async () => {
  let accountsList: GitAccount[] = [];
  const prListCalls: string[] = [];

  const createHarness = () =>
    loadSource(
      new URL('../src/modules/threads/threadPrs.svelte.ts', import.meta.url),
      {
        'lib/api/client': {
          api: {
            get: async (path: string) => {
              assert.equal(path, '/git/accounts');
              return accountsList;
            },
          },
        },
        'lib/api/git': {
          gitApi: {
            prs: {
              list: async (repoId: string) => {
                prListCalls.push(repoId);
                return {
                  items: [
                    {
                      number: 42,
                      title: 'feat: add IDE smart indent',
                      state: 'open',
                      source_branch: 'feat/ide',
                      target_branch: 'main',
                      url: 'https://github.com/org/repo/pull/42',
                    },
                  ],
                  has_more: false,
                  page: 1,
                  per_page: 50,
                };
              },
            },
          },
        },
      },
      {},
      { relative: true },
    ) as typeof import('../src/modules/threads/threadPrs.svelte.ts');

  const flushMicrotasks = () => new Promise((r) => setTimeout(r, 15));

  const makeRepo = (overrides: Partial<Repo>): Repo => ({
    id: overrides.id ?? 'repo_1',
    workspace_id: 'wsp_1',
    name: 'demo',
    path: '/ws/demo',
    remote_url: 'remote_url' in overrides ? (overrides.remote_url ?? null) : 'git@github.com:org/demo.git',
    provider: 'provider' in overrides ? (overrides.provider ?? null) : 'github',
    git_account_id: overrides.git_account_id ?? null,
    created_at: '2026-11-14T00:00:00Z',
  });

  // 1. Local-only repo (`provider: null`): never calls `/repos/{id}/prs`.
  const { threadPrs: prs1 } = createHarness();
  assert.equal(prs1.prFor(makeRepo({ id: 'repo_local', provider: null, remote_url: null }), 'feat/ide'), null);
  await flushMicrotasks();
  assert.deepEqual(prListCalls, []);

  // 2. GitHub remote repo with `git_account_id: null` and zero GitHub accounts configured: skips `/repos/{id}/prs` (prevents 400).
  accountsList = [];
  const { threadPrs: prs2 } = createHarness();
  assert.equal(prs2.prFor(makeRepo({ id: 'repo_no_acct', provider: 'github', git_account_id: null }), 'feat/ide'), null);
  await flushMicrotasks();
  assert.deepEqual(prListCalls, []);
  assert.equal(prs2.checked(makeRepo({ id: 'repo_no_acct', provider: 'github', git_account_id: null }), 'feat/ide'), true);

  // 3. Same repo once a matching GitHub account is configured: fetches open PRs cleanly!
  accountsList = [
    {
      id: 'acct_gh',
      user_id: 'usr_1',
      provider: 'github',
      label: 'GitHub',
      username: 'dev',
      token_ref: 'keychain:gh',
      api_base_url: null,
      namespace: null,
      token_expires_at: null,
      created_at: '2026-11-14T00:00:00Z',
    },
  ];
  prs2.forget(makeRepo({ id: 'repo_no_acct', provider: 'github', git_account_id: null }), 'feat/ide');
  prs2.prFor(makeRepo({ id: 'repo_no_acct', provider: 'github', git_account_id: null }), 'feat/ide');
  await flushMicrotasks();
  assert.deepEqual(prListCalls, ['repo_no_acct']);
  assert.equal(
    prs2.prFor(makeRepo({ id: 'repo_no_acct', provider: 'github', git_account_id: null }), 'feat/ide')?.number,
    42,
  );
});
