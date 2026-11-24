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
      new URL('../src/lib/stores/branchPrs.svelte.ts', import.meta.url),
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
    ) as typeof import('../src/lib/stores/branchPrs.svelte.ts');

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
  const { branchPrs: prs1 } = createHarness();
  assert.equal(prs1.prFor(makeRepo({ id: 'repo_local', provider: null, remote_url: null }), 'feat/ide'), null);
  await flushMicrotasks();
  assert.deepEqual(prListCalls, []);

  // 2. GitHub remote repo with `git_account_id: null` and zero GitHub accounts configured: skips `/repos/{id}/prs` (prevents 400).
  accountsList = [];
  const { branchPrs: prs2 } = createHarness();
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

test('IdeProjectsBridge and fetchNamedIdeProjectsFromServer coalesce fetches and never loop on IDE_PROJECTS_CHANGED_EVENT', async () => {
  const flushMicrotasks = () => new Promise((r) => setTimeout(r, 15));
  const storage = new Map<string, string>();
  const winListeners = new Map<string, Set<(ev: Event) => void>>();
  let getProjectsCalls = 0;

  class FakeCustomEvent<T = unknown> extends Event {
    detail: T;
    constructor(type: string, init?: { detail?: T }) {
      super(type);
      this.detail = init?.detail as T;
    }
  }
  const fakeStorage = {
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => {
      storage.set(k, v);
    },
    removeItem: (k: string) => {
      storage.delete(k);
    },
  };
  const fakeWin = {
    localStorage: fakeStorage,
    addEventListener(type: string, fn: (ev: Event) => void) {
      if (!winListeners.has(type)) winListeners.set(type, new Set());
      winListeners.get(type)!.add(fn);
    },
    removeEventListener(type: string, fn: (ev: Event) => void) {
      winListeners.get(type)?.delete(fn);
    },
    dispatchEvent(ev: Event) {
      for (const fn of [...(winListeners.get(ev.type) ?? [])]) fn(ev);
      return true;
    },
  };

  const sharedCache = new Map<string, Record<string, unknown>>();
  const sharedImports = {
    svelte: { untrack: <T>(fn: () => T) => fn() },
    'lib/api/client': {
      api: {
        async get() {
          return {};
        },
        bg: {
          async get(path: string) {
            assert.equal(path, '/ide/projects');
            getProjectsCalls += 1;
            await Promise.resolve();
            return {
              projects: [
                {
                  id: 'proj_1',
                  name: 'Otto',
                  primary_root: '/Users/dev/otto',
                  folders: [{ path: '/Users/dev/otto', label: 'otto' }],
                },
              ],
            };
          },
        },
      },
    },
    'lib/ideWorkspacePort.svelte': {
      ideWorkspacePort: {
        current: { root: null, additionalRoots: [], activePath: null, tabs: [] },
        lastRoot: null,
        rootsForLocal: () => [],
      },
    },
    'lib/router.svelte': { router: { module: 'threads' } },
    'lib/stores/git.svelte': { git: { repos: [] } },
    'lib/stores/workspace.svelte': {
      SCRATCH_WORKSPACE_ID: 'scratch',
      ws: { currentId: 'ws_1', current: { id: 'ws_1', root_path: '/Users/dev/otto' }, list: [] },
    },
  };
  const sharedGlobals = {
    window: fakeWin,
    localStorage: fakeStorage,
    CustomEvent: FakeCustomEvent,
  };

  const model = loadSource(
    new URL('../src/lib/ideProjectsModel.ts', import.meta.url),
    sharedImports,
    sharedGlobals,
    { relative: true },
    sharedCache,
  ) as typeof import('../src/lib/ideProjectsModel.ts');

  const bridgeMod = loadSource(
    new URL('../src/modules/threads/ideProjects.svelte.ts', import.meta.url),
    sharedImports,
    sharedGlobals,
    { relative: true },
    sharedCache,
  ) as typeof import('../src/modules/threads/ideProjects.svelte.ts');

  const bridge = bridgeMod.ideProjects;
  // Trigger lazy hydration + concurrent fetchNamedIdeProjectsFromServer calls.
  void bridge.namedProjects;
  const [a, b] = await Promise.all([
    model.fetchNamedIdeProjectsFromServer(),
    model.fetchNamedIdeProjectsFromServer(),
  ]);
  await flushMicrotasks();
  await flushMicrotasks();

  assert.equal(a?.length, 1);
  assert.equal(b?.length, 1);
  // Must have hit GET /ide/projects ONCE total — zero infinite event-feedback loop!
  assert.equal(getProjectsCalls, 1);
  assert.equal(bridge.namedProjects.length, 1);
});

test('buildDedupedBulkRepos deduplicates symlinked sibling repositories sharing a canonical path', async () => {
  const { buildDedupedBulkRepos } = await import('../src/modules/ide/shell/bulkRepoModel.ts');
  const built = buildDedupedBulkRepos([
    {
      name: 'support_agent',
      path: '/Users/dev/supprt_agent',
      kind: 'go',
      has_go_mod: true,
      has_pom_or_gradle: false,
      has_git: true,
    },
    {
      name: 'supprt_agent',
      path: '/Users/dev/supprt_agent',
      kind: 'go',
      has_go_mod: true,
      has_pom_or_gradle: false,
      has_git: true,
    },
    {
      name: 'go_utilities',
      path: '/Users/dev/go_utilities',
      kind: 'go',
      has_go_mod: true,
      has_pom_or_gradle: false,
      has_git: true,
    },
  ]);

  assert.equal(built.entries.length, 2, 'duplicate canonical path must be collapsed to one keyed entry');
  assert.deepEqual(
    built.entries.map((e) => ({ name: e.name, path: e.path })),
    [
      { name: 'go_utilities', path: '/Users/dev/go_utilities' },
      { name: 'supprt_agent', path: '/Users/dev/supprt_agent' },
    ],
  );
  assert.deepEqual(built.techByPath['/Users/dev/supprt_agent'], ['go']);
});

test('networkBridgeModel serializes K8s cluster + namespace + SSH tunnel + host mappings and formats toolbar badge', async () => {
  const {
    emptyNetworkBridgeDraft,
    formatBridgeToolbarLabel,
    mergeClusterNamespaceSuggestions,
    serializeNetworkBridgeDraft,
  } = await import('../src/modules/ide/shell/networkBridgeModel.ts');

  const draft = emptyNetworkBridgeDraft({
    enabled: true,
    k8s_cluster_id: '01M3PSPFMHKRJ5RGDNKBC90ECH',
    k8s_namespace: 'cbo',
    ssh_tunnel_connection_id: 'conn_bastion',
    inject_k8s_service_env: true,
    containerized: true,
    host_mappings: [
      { from_host: 'localhost', from_port: 9090, to_host: 'confsrv', to_port: 9090 },
    ],
  });
  assert.equal(draft.enabled, true);
  assert.equal(draft.k8s_namespace, 'cbo');
  assert.equal(draft.mappings.length, 1);

  const ok = serializeNetworkBridgeDraft(draft);
  assert.equal(ok.error, null);
  assert.deepEqual(ok.config, {
    enabled: true,
    k8s_cluster_id: '01M3PSPFMHKRJ5RGDNKBC90ECH',
    k8s_namespace: 'cbo',
    ssh_tunnel_connection_id: 'conn_bastion',
    inject_k8s_service_env: true,
    containerized: true,
    host_mappings: [
      { from_host: 'localhost', from_port: 9090, to_host: 'confsrv', to_port: 9090 },
    ],
  });
  assert.equal(formatBridgeToolbarLabel(ok.config), 'K8s · cbo + SSH');
  assert.equal(
    formatBridgeToolbarLabel({ ...ok.config, ssh_tunnel_connection_id: null }),
    'K8s · cbo',
  );
  assert.equal(formatBridgeToolbarLabel({ ...ok.config, enabled: false }), null);

  // Validation catches enabling with zero cluster, zero SSH, and zero mappings:
  const invalidEmpty = serializeNetworkBridgeDraft({
    ...emptyNetworkBridgeDraft(null),
    enabled: true,
  });
  assert.ok(invalidEmpty.error?.includes('Select a Kubernetes cluster'));

  // Validation catches invalid port:
  const invalidPort = serializeNetworkBridgeDraft({
    ...draft,
    mappings: [{ rowId: '1', from_host: 'localhost', from_port: '99999', to_host: 'confsrv', to_port: '9090' }],
  });
  assert.ok(invalidPort.error?.includes('1–65535'));

  // Namespace suggestions deduplicate default_namespace + known_namespaces + live namespaces:
  const nsSuggestions = mergeClusterNamespaceSuggestions(
    {
      id: '01M3',
      name: 'STG-M',
      source: 'kubeconfig',
      context_name: 'stgm',
      default_namespace: 'cbo',
      known_namespaces: ['cbo', 'mscasino', 'payments'],
      environment: 'staging',
      params: {},
      created_by: null,
      created_at: '2026-11-15T00:00:00Z',
      updated_at: '2026-11-15T00:00:00Z',
    },
    [
      { name: 'mscasino', status: 'Active', age_seconds: 100 },
      { name: 'auth', status: 'Active', age_seconds: 100 },
    ],
  );
  assert.deepEqual(nsSuggestions, ['cbo', 'mscasino', 'payments', 'auth']);
});
