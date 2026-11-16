import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import * as cmAutocomplete from '@codemirror/autocomplete';
import * as cmCommands from '@codemirror/commands';
import * as cmLanguage from '@codemirror/language';
import * as cmState from '@codemirror/state';
import * as cmView from '@codemirror/view';
import { componentFunctions } from './componentFunctions.ts';
import { loadSource } from './sourceHarness.ts';
import * as lineDiff from '../src/lib/lineDiff.ts';
import * as livePaths from '../src/lib/gitLivePaths.ts';
import * as latest from '../src/lib/latest.ts';
import {
  fileUriToPath,
  isBroadNonProjectRootClient,
  pathToFileUri,
  percentDecodeUtf8,
} from '../src/lib/fileUri.ts';
import {
  extractGoBuildTagsFromSource,
  mergeGoTagsCliArgs,
  resolveEffectiveRepoGoBuildTags,
  sanitizeGoBuildTags,
} from '../src/lib/goBuildTags.ts';
import {
  attachmentSupport,
  inferProviderFamily,
  isRuntimeProvider,
  orderThreadHarnesses,
  providerSupportsSteer,
  resolveIdeHarnesses,
  resolveIdeInlineProviders,
  sessionProviderSlug,
  takesFile,
} from '../src/lib/providerRegistry.ts';
import {
  effortsForModel,
  modelsForProvider,
  providerFamily,
  supportsEffortSelection,
  supportsModelSelection,
} from '../src/lib/providerModels.ts';

const { computeIdeDiffHunks, parseSavedLines } = loadSource(
  new URL('../src/lib/components/cm-ide-git-and-agent.ts', import.meta.url),
  {
    '@codemirror/view': cmView,
    '@codemirror/state': cmState,
    '../clipboard': { copyWithToast() {} },
    '../contextmenu.svelte': { ctxMenu: { show() {}, showAt() {} } },
    '../lineDiff.ts': lineDiff,
  },
);
const { computeMinimalTextUpdate } = loadSource(
  new URL('../src/lib/components/cm-editor-helpers.ts', import.meta.url),
  {
    '@codemirror/autocomplete': cmAutocomplete,
    '@codemirror/commands': cmCommands,
    '@codemirror/language': cmLanguage,
    '@codemirror/state': cmState,
    '@codemirror/view': cmView,
    '../editor-history.ts': {
      forgetEditorState() {},
      loadEditorState: () => null,
      PERSIST_MAX_BYTES: 65536,
      prepareEditorHistory() {},
      saveEditorState() {},
    },
  },
);
const { detectTextEol, normalizeEditorLf, serializeTabDiskContent } = loadSource(
  new URL('../src/modules/ide/store/ideTypes.ts', import.meta.url),
  {
    '../../../lib/api/client': { getToken: () => 'tok', laneFetch: async () => ({ ok: false, body: null }) },
  },
);

describe('Round 5 — Bug U1: Editor Git HEAD diff gutter & persisted baseline across reopen', () => {
  it('computes add, mod, and del hunks against Git HEAD even after saving and normalizes CRLF baselines', () => {
    const headBase = 'line 1\r\nline 2\r\nline 3\r\nline 4\r\n';
    const savedEditedDoc = 'line 1\nline 2 modified\nline 4\nline 5 added\n';

    assert.deepEqual([...(parseSavedLines(headBase) ?? [])], ['line 1', 'line 2', 'line 3', 'line 4', '']);

    // Even when `docText === tab.savedContent` (already saved on disk or reopened after leaving IDE),
    // diffing against `headBase` marks every uncommitted modification, deletion, and addition!
    const hunks = computeIdeDiffHunks(savedEditedDoc, headBase);
    assert.ok(hunks.length >= 2, 'should report modified/deleted and added hunks vs Git HEAD');
    assert.ok(hunks.some((h: { kind: string }) => h.kind === 'mod'));
    assert.ok(hunks.some((h: { kind: string }) => h.kind === 'add'));

    // Untracked file inside a Git repo (`headBase === ''`) marks all lines as added (`add`).
    const untrackedHunks = computeIdeDiffHunks('package demo\n\nfunc New() {}\n', '');
    assert.equal(untrackedHunks.length, 1);
    assert.equal(untrackedHunks[0].kind, 'add');
    assert.equal(untrackedHunks[0].markerStartLine, 1);

    // Clean file matching HEAD (`docText === normalizeEditorLf(headBase)`) has zero hunks.
    assert.deepEqual([...computeIdeDiffHunks(normalizeEditorLf(headBase), headBase)], []);
  });

  it('fetches /ide/fs/git-base via runResolveIdeGitBaseContent and propagates transient errors without poisoning cache', async () => {
    const calls: string[] = [];
    let mode: 'tracked' | 'untracked' | 'outside' | 'binary' | 'error' = 'tracked';
    const apiStub = {
      api: {
        bg: {
          get: async (url: string) => {
            calls.push(url);
            if (mode === 'error') throw new Error('daemon restarting');
            if (mode === 'binary') return { in_repo: true, head_exists: true, binary: true, content: null };
            if (mode === 'outside') return { in_repo: false, head_exists: false, binary: false, content: null };
            if (mode === 'untracked') return { in_repo: true, head_exists: false, binary: false, content: null };
            return {
              in_repo: true,
              head_exists: true,
              binary: false,
              content: 'first\r\nsecond\r\n',
            };
          },
        },
      },
    };
    const ideStub = {
      ide: {
        target: { kind: 'local', root: '/Users/dev/koala-svc' },
        projectRoots: [{ path: '/Users/dev/koala-svc', name: 'koala-svc', isPrimary: true }],
        additionalRoots: [],
        tabs: [],
        activePath: null,
        targetQuery: (extra: Record<string, string>) => new URLSearchParams(extra).toString(),
      },
      ideBasename: (p: string) => p.split('/').pop() ?? p,
    };

    const lazyMod = loadSource(
      new URL('../src/modules/ide/editor/ideEditorActionsLazy.ts', import.meta.url),
      {
        '../../../lib/api/client': apiStub,
        '../../../lib/clipboard': { copyWithToast() {} },
        '../../../lib/contextmenu.svelte': { ctxMenu: { show() {}, showAt() {} } },
        '../git/ideGitController.svelte': { ideGit: {} },
        '../ideStore.svelte': ideStub,
        'modules/ide/ideStore.svelte': ideStub,
        '../store/ideAgentController.svelte': { ideAgent: {} },
        '../../../lib/idePreferences.svelte': { idePreferences: { current: { formatOnSave: false }, update() {} } },
      },
      {},
      { relative: true },
    );

    const trackedBase = await lazyMod.runResolveIdeGitBaseContent(
      '/Users/dev/koala-svc/main.go',
      'first\nsecond edited\n',
    );
    assert.equal(trackedBase, 'first\nsecond\n');
    assert.ok(calls[0]?.startsWith('/ide/fs/git-base?'));

    mode = 'untracked';
    assert.equal(
      await lazyMod.runResolveIdeGitBaseContent('/Users/dev/koala-svc/new.go', 'package main\n'),
      '',
    );

    mode = 'outside';
    assert.equal(
      await lazyMod.runResolveIdeGitBaseContent('/Users/dev/scratch/notes.txt', 'disk text'),
      undefined,
    );

    mode = 'binary';
    assert.equal(
      await lazyMod.runResolveIdeGitBaseContent('/Users/dev/koala-svc/logo.png', 'raw'),
      null,
    );

    mode = 'error';
    await assert.rejects(() =>
      lazyMod.runResolveIdeGitBaseContent('/Users/dev/koala-svc/main.go', 'disk text'),
    );
  });

  it('diffs 5,000-line files with UTF-8 BOM, CRLF, and bare CR accurately without marking all rows changed', () => {
    const baseRows = Array.from({ length: 5000 }, (_, idx) => `property.key.${idx}=value_${idx}`);
    const headWithBomAndCrlf = `\uFEFF${baseRows.join('\r\n')}\r\n`;
    const editedRows = [...baseRows];
    editedRows[2499] = 'property.key.2499=updated_value';
    const editorLf = `${editedRows.join('\n')}\n`;

    const hunks = computeIdeDiffHunks(editorLf, headWithBomAndCrlf);
    assert.equal(hunks.length, 1, 'editing 1 line in a 5,000-line BOM+CRLF file must produce exactly 1 hunk');
    assert.equal(hunks[0].kind, 'mod');
    assert.equal(hunks[0].markerStartLine, 2500);
    assert.equal(hunks[0].markerEndLine, 2500);

    // Trailing-newline-only difference marks only the final line, never all 5,000 rows!
    const noTrailingNl = baseRows.join('\n');
    const nlHunks = computeIdeDiffHunks(noTrailingNl, headWithBomAndCrlf);
    assert.ok(nlHunks.length <= 1);
    if (nlHunks.length === 1) {
      assert.equal(nlHunks[0].markerStartLine, 5000);
    }
  });
});

describe('Round 5 — Bugs U2 & U3: Broad $HOME root guard, fresh status bypass, and Open in Git Module tab preservation', () => {
  it('blocks $HOME and system roots while allowing real project repositories', () => {
    assert.equal(isBroadNonProjectRootClient('/'), true);
    assert.equal(isBroadNonProjectRootClient('/Users'), true);
    assert.equal(isBroadNonProjectRootClient('/Users/itziklavon'), true);
    assert.equal(isBroadNonProjectRootClient('/Users/itziklavon/'), true);
    assert.equal(isBroadNonProjectRootClient('/home/ubuntu'), true);
    assert.equal(isBroadNonProjectRootClient('/tmp'), true);
    assert.equal(isBroadNonProjectRootClient('/private/tmp'), true);

    assert.equal(isBroadNonProjectRootClient('/Users/itziklavon/koala-common-go'), false);
    assert.equal(isBroadNonProjectRootClient('/Users/itziklavon/claude_ade-ide'), false);
    assert.equal(isBroadNonProjectRootClient('/home/ubuntu/work/service'), false);
  });

  it('bypasses STATUS_SHARE_MS on refreshStatus({ force: true }) with ?fresh=true, records statusErrorById, rejects $HOME, and preserves persisted Git tabs', async () => {
    const urls: string[] = [];
    let shouldFail = false;
    let branchSeq = 0;

    const client = {
      api: {
        get: async (url: string) => {
          urls.push(url);
          if (shouldFail) throw new Error('git status timed out after 30s');
          branchSeq += 1;
          return {
            branch: 'feat/ide',
            upstream: 'origin/feat/ide',
            ahead: branchSeq,
            behind: 0,
            changes: [{ path: 'main.go', kind: 'modified', staged: false, unstaged: true }],
          };
        },
        post: async () => ({}),
      },
    };

    const storageMap = new Map<string, string>([
      [
        'otto_git_open_tabs',
        JSON.stringify({
          openRepoIds: ['repo_existing_1', 'repo_existing_2'],
          activeRepoId: 'repo_existing_1',
          sub: { repo_existing_1: 'prs' },
        }),
      ],
    ]);

    const { git } = loadSource(
      new URL('../src/lib/stores/git.svelte.ts', import.meta.url),
      {
        '../api/client': client,
        'lib/api/client': client,
        '../gitLivePaths': livePaths,
        '../latest': latest,
      },
      {
        localStorage: {
          getItem: (k: string) => storageMap.get(k) ?? null,
          setItem: (k: string, v: string) => storageMap.set(k, v),
          removeItem: (k: string) => storageMap.delete(k),
        },
        document: { hidden: false, hasFocus: () => true, addEventListener() {}, removeEventListener() {} },
      },
      { relative: true },
    );

    // 1. $HOME dotfile repo must NEVER enter git.repos / git.allRepos via upsertRepo
    git.upsertRepo({ id: 'repo_home', name: 'itziklavon', path: '/Users/itziklavon', workspace_id: 'ws1' });
    assert.equal(git.allRepos.length, 0);

    // Real project repo registers cleanly
    git.upsertRepo({ id: 'repo_koala', name: 'koala-svc', path: '/Users/itziklavon/koala-svc', workspace_id: 'ws1' });
    assert.equal(git.allRepos.length, 1);

    // 2. Open in Git Module BEFORE initializeOpenTabs() finishes must preserve existing persisted tabs!
    git.openRepoTab('repo_koala', 'graph');
    assert.deepEqual([...git.openRepoIds], ['repo_existing_1', 'repo_existing_2', 'repo_koala']);
    assert.equal(git.activeRepoId, 'repo_koala');
    assert.equal(git.subTab.repo_existing_1, 'prs');
    assert.equal(git.subTab.repo_koala, 'graph');

    // 3. Normal status() followed immediately (<300ms) by refreshStatus({ force: true }) must hit ?fresh=true!
    await git.refreshStatus('repo_koala');
    assert.equal(urls.at(-1), '/repos/repo_koala/status');
    assert.equal(git.statusById.repo_koala.ahead, 1);
    assert.equal(git.liveRev.repo_koala, undefined);

    await git.refreshStatus('repo_koala', { force: true });
    assert.equal(urls.at(-1), '/repos/repo_koala/status?fresh=true');
    assert.equal(git.statusById.repo_koala.ahead, 2);
    assert.equal(git.liveRev.repo_koala, 1);

    // 4. Status failure records statusErrorById instead of leaving the Git dock silently blank!
    shouldFail = true;
    await git.refreshStatus('repo_koala', { force: true });
    assert.match(git.statusErrorById.repo_koala ?? '', /timed out after 30s/);
  });
});

describe('Round 5 — Bug U4: CRLF line-ending preservation across open, edit, undo, and save', () => {
  it('normalizes CRLF to LF in editor, clears dirty on undo back to saved LF, and writes CRLF back to disk', () => {
    const rawDiskCrlf = 'package main\r\n\r\nfunc main() {\r\n\tprintln("hi")\r\n}\r\n';
    const eol = detectTextEol(rawDiskCrlf);
    assert.equal(eol, '\r\n');

    const normalizedLf = normalizeEditorLf(rawDiskCrlf);
    assert.equal(normalizedLf.includes('\r'), false);

    // Minimal text update treats CRLF vs LF of identical lines as a no-op (`null`) so history isn't polluted.
    assert.equal(computeMinimalTextUpdate(rawDiskCrlf, normalizedLf, 0, 0), null);

    // Editing one line and serializing back to disk preserves `\r\n` on every row so Git diff shows 1 changed line!
    const editedLf = normalizedLf.replace('println("hi")', 'println("hello")');
    const serialized = serializeTabDiskContent({ eol }, editedLf);
    assert.equal(
      serialized,
      'package main\r\n\r\nfunc main() {\r\n\tprintln("hello")\r\n}\r\n',
    );

    // Plain LF files stay LF on save.
    const lfFile = 'a\nb\nc\n';
    assert.equal(detectTextEol(lfFile), '\n');
    assert.equal(serializeTabDiskContent({ eol: '\n' }, 'a\nb2\nc\n'), 'a\nb2\nc\n');
  });
});

describe('Round 5 — Bugs U3, U5 & U6: Zero-shift IDE gutter, Open in Git Module handler, & Debug/Run replay fidelity', () => {
  it('executes IdeGitToolPanel openInGitModule() navigating via router.go("git") while rejecting $HOME roots', async () => {
    const upserted: string[] = [];
    const openedTabs: Array<[string, string]> = [];
    const routes: string[] = [];
    const infos: string[] = [];
    const selectedWs: string[] = [];

    const makePanel = (repo: { id: string; name: string; path: string; workspace_id?: string } | null) =>
      componentFunctions(
        new URL('../src/modules/ide/git/IdeGitToolPanel.svelte', import.meta.url),
        ['openInGitModule'],
        {
          ide: { activePath: repo?.path ?? '/Users/itziklavon', target: { kind: 'local', root: '/Users/itziklavon' } },
          ideGit: {
            activeRoot: null,
            resolveRepoForPath: async () => repo,
          },
          activeRepo: repo,
          isBroadNonProjectRootClient,
          SCRATCH_WORKSPACE_ID: '__scratch__',
          ws: {
            workspaces: [{ id: 'ws_main' }, { id: 'ws_other' }],
            currentId: 'ws_main',
            select: (id: string) => selectedWs.push(id),
          },
          toasts: {
            info: (title: string) => infos.push(title),
          },
          git: {
            initializeOpenTabs: async () => {},
            upsertRepo: (r: { id: string }) => upserted.push(r.id),
            openRepoTab: (id: string, sub: string) => openedTabs.push([id, sub]),
          },
          router: {
            go: (mod: string) => routes.push(mod),
          },
        },
      );

    // Broad `$HOME` repo must NOT be upserted or opened as a tab, surfaces an info toast, yet still routes directly to `git` (never `focus`).
    await makePanel({ id: 'r_home', name: 'itziklavon', path: '/Users/itziklavon' }).openInGitModule();
    assert.deepEqual(upserted, []);
    assert.deepEqual(openedTabs, []);
    assert.deepEqual(infos, ['No Git repository detected for the current project folder.']);
    assert.deepEqual(routes, ['git']);

    // Real project repo (including cross-workspace) upserts, opens `'graph'` tab additively, switches workspace, and navigates via `router.go('git')`.
    await makePanel({ id: 'r_svc', name: 'koala-svc', path: '/Users/itziklavon/koala-svc', workspace_id: 'ws_other' }).openInGitModule();
    assert.deepEqual(upserted, ['r_svc', 'r_svc']);
    assert.deepEqual(openedTabs, [['r_svc', 'graph']]);
    assert.deepEqual(selectedWs, ['ws_other']);
    assert.deepEqual(routes, ['git', 'git']);
  });

  it('keeps IDE gutter columns fixed-width and replays Run/Debug launch memos faithfully across terminated sessions', async () => {
    const extSrc = readFileSync(
      new URL('../src/lib/components/cm-ide-extensions.ts', import.meta.url),
      'utf8',
    );
    const helpersSrc = readFileSync(
      new URL('../src/lib/components/cm-editor-helpers.ts', import.meta.url),
      'utf8',
    );
    assert.match(extSrc, /\.cm-ide-bp-gutter[\s\S]*?inlineSize:\s*'34px'[\s\S]*?minInlineSize:\s*'34px'[\s\S]*?maxInlineSize:\s*'34px'/);
    assert.match(extSrc, /\.cm-ide-bp-cell[\s\S]*?gridTemplateColumns:\s*'14px 15px'/);
    assert.match(helpersSrc, /\.cm-foldGutter[\s\S]*?inlineSize:\s*'14px'[\s\S]*?minInlineSize:\s*'14px'[\s\S]*?maxInlineSize:\s*'14px'/);
    assert.match(helpersSrc, /cm-ide-bp-gutter[\s\S]*?inlineSize:\s*'34px'/);

    const posted: Array<{ url: string; body: any }> = [];
    const debugEnvMod = loadSource(
      new URL('../src/modules/ide/store/ideDebugAndEnvController.ts', import.meta.url),
      {
        '../../../lib/api/client': {
          api: {
            post: async (url: string, body: any) => {
              posted.push({ url, body });
              if (url === '/ide/runs') {
                return { run: { id: `run_${posted.length}`, status: 'completed', output: [] } };
              }
              return { session: { id: `dbg_${posted.length}`, state: 'running', stack_frames: [], output: [], inline_values: [], breakpoints: [] } };
            },
            get: async () => ({}),
          },
          ApiError: class ApiError extends Error { status = 500; },
          getToken: () => 'tok',
          laneFetch: async () => ({ ok: false, body: null }),
        },
        '../../../lib/idePreferences.svelte': {
          idePreferences: { current: { moduleSdkMap: {} } },
        },
        '../../../lib/stores/auth.svelte': { auth: { isRoot: true } },
        '../../../lib/toast.svelte': { toasts: { success() {} } },
        '../../../lib/toastError': { toastError() {} },
        '../../../lib/confirm.svelte': { confirmer: { ask: async () => true } },
        '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [], saveRootsForLocal() {} } },
        '../../../lib/storage': { lsGet: () => null, lsSet() {} },
        '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false, maxOpenTabs: 20 }) },
        '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft() {}, writeIdeUndoDraftImmediate() {} },
        svelte: { tick: async () => {} },
      },
      {},
      { relative: true },
    );

    const recordedRuns = new Map<string, any>();
    let lastRunMemo: any = null;
    const host: any = {
      target: { kind: 'local', root: '/ws/promotions' },
      targetPayload: { kind: 'local', root: '/ws/promotions' },
      projectRoots: [{ path: '/ws/promotions', name: 'promotions', isPrimary: true }],
      activePath: '/ws/promotions/main.go',
      activeTab: { path: '/ws/promotions/main.go', language: 'go', content: 'package main\nfunc main() {}\n' },
      projectEnv: { go_build_tags: ['component'], env_vars: [] },
      runConfigs: [],
      detectedConfigs: [],
      selectedConfigId: null,
      activeMavenProfiles: [],
      activeSpringProfiles: [],
      buildSkipTests: false,
      buildOffline: false,
      effectiveBreakpoints: [{ path: '/ws/promotions/main.go', line: 12, enabled: true }],
      keepTestcontainersAlive: false,
      lastDebugLaunch: null,
      recordRunLaunch(runId: string, memo: any) {
        lastRunMemo = memo;
        recordedRuns.set(runId, memo);
      },
      openBottomTab() {},
      resolvePath: (p: string) => p,
      targetQuery: () => 'kind=local&root=%2Fws%2Fpromotions',
    };

    // 1. Launch an adhoc Run with coverage + keepTestcontainers, then replay via recorded runId memo!
    const adhocSpec = { name: 'TestMainFlow', runner: 'go_test', cwd: '/ws/promotions', target_path: './...' };
    const r1 = await debugEnvMod.launchIdeRun(
      host,
      { adhoc: adhocSpec, coverage: true, keepTestcontainers: true },
      null,
      null,
    );
    assert.equal(posted.length, 1);
    assert.equal(posted[0].url, '/ide/runs');
    assert.equal(posted[0].body.adhoc.coverage, true);
    assert.equal(posted[0].body.adhoc.env_overrides.TESTCONTAINERS_RYUK_DISABLED, 'true');

    const replayMemo = recordedRuns.get('run_1') ?? lastRunMemo;
    assert.ok(replayMemo, 'launchIdeRun must record run launch memo by runId');
    const r2 = await debugEnvMod.launchIdeRun(host, replayMemo, r1.poller, r1.sseCtl);
    r2.poller?.stop();
    r2.sseCtl?.abort();
    assert.equal(posted.length, 2);
    assert.equal(posted[1].body.adhoc.name, 'TestMainFlow');
    assert.equal(posted[1].body.adhoc.coverage, true);
    assert.equal(posted[1].body.adhoc.env_overrides.TESTCONTAINERS_RYUK_DISABLED, 'true');

    // 2. Launch a Go `main.go` Debug session, terminate it, and verify `lastDebugLaunch` replays cleanly!
    const d1 = await debugEnvMod.launchDebugSession(
      host,
      {
        language: 'go',
        launch: { program: '/ws/promotions/main.go', cwd: '/ws/promotions', args: ['--port=8080'] },
      },
      null,
      () => {},
    );
    assert.equal(posted.length, 3);
    assert.equal(posted[2].url, '/ide/debug/sessions');
    assert.equal(posted[2].body.launch.program, '/ws/promotions/main.go');
    assert.equal(posted[2].body.launch.test_name, undefined);
    assert.deepEqual(posted[2].body.launch.build_flags ? [...posted[2].body.launch.build_flags] : [], ['-tags=component']);

    // Replay `host.lastDebugLaunch` (what `ide.restartDebug()` executes after termination)
    assert.ok(host.lastDebugLaunch, 'launchDebugSession must record lastDebugLaunch');
    const d2 = await debugEnvMod.launchDebugSession(host, host.lastDebugLaunch, d1, () => {});
    d2?.abort();
    assert.equal(posted.length, 4);
    assert.equal(posted[3].url, '/ide/debug/sessions');
    assert.equal(posted[3].body.launch.program, '/ws/promotions/main.go');
    assert.deepEqual(posted[3].body.launch.args ? [...posted[3].body.launch.args] : [], ['--port=8080']);

    // 3. Toolbar `{}` launch (no explicit opts) followed by switching activeTab to README.md:
    // Rerun MUST still replay the resolved `main.go` launch + `break_on_exceptions`, NEVER `{}` -> README.md!
    host.breakOnExceptions = true;
    host.activePath = '/ws/promotions/cmd/api/server.go';
    host.activeTab = {
      path: '/ws/promotions/cmd/api/server.go',
      name: 'server.go',
      language: 'go',
      content: 'package main\nfunc main() {}\n',
    };
    const rToolbar = await debugEnvMod.launchIdeRun(host, {}, null, null);
    rToolbar.poller?.stop();
    rToolbar.sseCtl?.abort();
    const dToolbar = await debugEnvMod.launchDebugSession(host, {}, null, () => {});
    dToolbar?.abort();
    assert.equal(posted[5].body.break_on_exceptions, true);
    assert.equal(host.lastDebugLaunch?.launch?.program, '/ws/promotions/cmd/api/server.go');

    // User switches editor tab to `README.md` before pressing Rerun:
    host.activePath = '/ws/promotions/README.md';
    host.activeTab = {
      path: '/ws/promotions/README.md',
      name: 'README.md',
      language: 'markdown',
      content: '# Docs\n',
    };
    const rReplay = await debugEnvMod.launchIdeRun(host, lastRunMemo, null, null);
    rReplay.poller?.stop();
    rReplay.sseCtl?.abort();
    assert.equal(posted[6].body.adhoc.target_path, '/ws/promotions/cmd/api/server.go');
    const dReplay = await debugEnvMod.launchDebugSession(host, host.lastDebugLaunch, null, () => {});
    dReplay?.abort();
    assert.equal(posted[7].body.launch.program, '/ws/promotions/cmd/api/server.go');
    assert.equal(posted[7].body.language, 'go');
  });
});

describe('Round 5 — Custom provider parity (`claudework`) & token-boundary safety', () => {
  const meta = {
    providers: ['claude', 'claudework', 'codex', 'agy', 'pi', 'pipeline'],
    model_flags: { claude: true, claudework: true },
    provider_models: {
      claude: [
        {
          id: 'claude-sonnet-4-6',
          label: 'Sonnet 4.6',
          description: '',
          default_effort: 'medium',
          supported_efforts: ['low', 'medium', 'high'],
        },
      ],
      claudework: [
        {
          id: 'claude-opus-4-6',
          label: 'Opus 4.6 (Work)',
          description: '',
          default_effort: 'high',
          supported_efforts: ['low', 'medium', 'high', 'max'],
        },
      ],
    },
  };

  it('resolves claudework across structured runtime, model catalogs, steer, attachments, and session slugs without false-matching pi', () => {
    assert.equal(inferProviderFamily('claudework', meta), 'claude');
    assert.equal(providerFamily('claudework', meta), 'claude');
    assert.equal(isRuntimeProvider('claudework', meta), true);
    assert.equal(isRuntimeProvider('CLAUDE', meta), false);
    assert.equal(providerSupportsSteer('claudework', meta), true);
    assert.equal(providerSupportsSteer('CLAUDE', meta), false);
    assert.deepEqual(attachmentSupport('claudework', meta), { images: true, files: true });
    assert.equal(takesFile('claudework', { type: 'application/pdf' }, meta), true);
    assert.equal(supportsModelSelection('claudework', meta), true);
    assert.equal(supportsEffortSelection('claudework', meta), true);
    assert.deepEqual(effortsForModel('claudework', 'claude-opus-4-6', meta), ['low', 'medium', 'high', 'max']);
    assert.equal(modelsForProvider('claudework', meta)[0]?.id, 'claude-opus-4-6');
    assert.equal(
      sessionProviderSlug({ provider: 'claude', meta: { provider_slug: 'claudework' } }),
      'claudework',
    );

    // Token-boundary check: `pipeline` and `api-agent` must NEVER false-match `pi`!
    assert.equal(inferProviderFamily('pipeline'), 'pipeline');
    assert.equal(inferProviderFamily('api-agent'), 'api-agent');
    assert.equal(isRuntimeProvider('pipeline', meta), false);
    assert.equal(inferProviderFamily('pi-fast'), 'pi');

    // IDE harness ordering & inline provider filtering include `claudework` while honoring disabled_providers (`[]`).
    assert.deepEqual(resolveIdeHarnesses([]), []);
    assert.ok(orderThreadHarnesses(meta.providers).includes('claudework'));
    assert.deepEqual(resolveIdeInlineProviders(meta.providers, meta.providers), [
      'claude',
      'claudework',
      'codex',
      'agy',
      'pi',
    ]);
  });
});

describe('Round 5 — Queued message Edit attachment preservation, MAX_DRAFT_IMAGES cap, and model override', () => {
  it('restores screenshots and PDFs with filenames on queued Edit, deduplicates existing items, and caps at 20', async () => {
    const toastErrors: string[] = [];
    const composerMod = loadSource(
      new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url),
      {
        '../../../lib/providerRegistry': { sessionProviderSlug },
        '../../../lib/toastError': {
          toastError: (title: string, err: unknown) => toastErrors.push(`${title}: ${String(err)}`),
        },
        '../../../lib/telemetry': {
          startMeasurement: () => null,
        },
        '../../../lib/stores/runtimeDrafts.svelte': {
          runtimeDrafts: { owns: () => true, for: () => ({ freshContext: false }) },
        },
        './hostCommands': { runHostSlash: () => false },
        './timeline': { errorText: (e: string) => e },
      },
    );

    const existing = [{ name: 'already.png', mime: 'image/png', data_base64: 'AAA=' }];
    const poppedFromQueue = [
      { name: 'already-dup.png', mime: 'image/png', data_base64: 'AAA=' },
      { name: 'bug-shot.png', mime: 'image/png', data_base64: 'BBB=' },
      { name: 'spec.pdf', mime: 'application/pdf', data_base64: 'CCC=' },
    ];
    const merged = JSON.parse(JSON.stringify(composerMod.mergeQueuedAttachments(existing, poppedFromQueue)));
    assert.deepEqual(merged, [
      { name: 'already.png', mime: 'image/png', data_base64: 'AAA=' },
      { name: 'bug-shot.png', mime: 'image/png', data_base64: 'BBB=' },
      { name: 'spec.pdf', mime: 'application/pdf', data_base64: 'CCC=' },
    ]);
    assert.equal(toastErrors.length, 0);

    // Exceeding MAX_DRAFT_IMAGES (20) caps cleanly and emits a clear toast error.
    const twentyTwo = Array.from({ length: 22 }, (_, idx) => ({
      name: `shot-${idx}.png`,
      mime: 'image/png',
      data_base64: `DATA_${idx}`,
    }));
    const capped = composerMod.mergeQueuedAttachments([], twentyTwo);
    assert.equal(capped.length, 20);
    assert.equal(toastErrors.length, 1);

    // Explicit "Default model" (`null`) vs unchosen (`undefined`) model override resolution:
    assert.equal(composerMod.resolveEffectiveDraftModel(undefined, 'claude-opus-4-6'), 'claude-opus-4-6');
    assert.equal(composerMod.resolveEffectiveDraftModel(null, 'claude-opus-4-6'), null);
    assert.equal(composerMod.resolvePromptModelOverride(undefined, 'claude-opus-4-6'), undefined);
    assert.equal(composerMod.resolvePromptModelOverride(null, 'claude-opus-4-6'), '');
    assert.equal(composerMod.resolvePromptModelOverride(null, null), undefined);
    assert.equal(composerMod.resolvePromptModelOverride('claude-sonnet-4-6', 'claude-opus-4-6'), 'claude-sonnet-4-6');

    // IDE Agent Dock `<ide_context>` round-trip: Edit/Resend strips XML wrapper and restores chips cleanly!
    const diffApplierMod = await import('../src/modules/ide/store/ideDiffApplier.ts');
    const wrappedPrompt = diffApplierMod.buildIdeAgentPrompt(
      'Fix nil pointer in Submit',
      [
        {
          id: 'chip-1',
          key: 'selection:/ws/app/pool.go:10-14',
          kind: 'selection',
          label: 'pool.go:10-14',
          path: '/ws/app/pool.go',
          startLine: 10,
          endLine: 14,
          language: 'go',
          snippet: 'func (p *Pool) Submit(f func()) error {\n\treturn p.q.Push(f)\n}',
        },
      ],
      { workspaceRoot: '/ws/app', activePath: '/ws/app/pool.go', omitWhenEmpty: true },
    );
    assert.ok(wrappedPrompt.includes('<ide_context>'));
    const restored = diffApplierMod.extractIdeContextChipsFromPrompt(wrappedPrompt);
    assert.equal(restored.cleanText, 'Fix nil pointer in Submit');
    assert.equal(restored.chips.length, 1);
    assert.equal(restored.chips[0].label, 'pool.go:10-14');
    assert.equal(restored.chips[0].path, '/ws/app/pool.go');
    assert.equal(restored.chips[0].startLine, 10);
    assert.equal(restored.chips[0].endLine, 14);
    assert.ok(restored.chips[0].snippet.includes('p.q.Push(f)'));
  });
});

describe('Round 5 — Go build tag group negation, `--` separator, RFC 3986 fileUri, and fail-closed prod bridge guard', () => {
  it('ignores negated parenthesized `//go:build` groups, preserves args after `--`, and isolates sibling modules', () => {
    const src = '//go:build integration && !(slow || flaky) && !(!e2e_positive)\n\npackage svc\n';
    assert.deepEqual(extractGoBuildTagsFromSource(src), ['integration', 'e2e_positive']);
    assert.deepEqual(sanitizeGoBuildTags(['-tags=component,integration', '--tags=smoke']), [
      'component',
      'integration',
      'smoke',
    ]);
    assert.deepEqual(
      mergeGoTagsCliArgs(['test', './...', '-tags', 'unit', '--', '-tags=app_flag'], ['component']),
      ['test', './...', '-tags=unit,component', '--', '-tags=app_flag'],
    );

    // Explicit empty override `[]` on nested module clears parent tags; sibling root never inherits primary tags.
    const modOverrides = {
      '/ws/primary/submod-no-tags': [],
      '/ws/sibling-go': ['sibling_tag'],
    };
    assert.deepEqual(
      resolveEffectiveRepoGoBuildTags('/ws/primary/pkg/a.go', '/ws/primary', ['component'], modOverrides),
      ['component'],
    );
    assert.deepEqual(
      resolveEffectiveRepoGoBuildTags('/ws/primary/submod-no-tags/b.go', '/ws/primary', ['component'], modOverrides),
      [],
    );
    assert.deepEqual(
      resolveEffectiveRepoGoBuildTags('/ws/unrelated-sibling/c.go', '/ws/primary', ['component'], modOverrides),
      [],
    );
  });

  it('round-trips spaces, #, ?, %, Hebrew, and emoji in pathToFileUri / fileUriToPath including no-TextEncoder VM contexts', () => {
    const trickyPath = '/Users/dev/My Project #1/איציק 🚀/file?v=2%25[test].go';
    const uri = pathToFileUri(trickyPath);
    assert.ok(!uri.includes(' ') && !uri.includes('#') && !uri.includes('?'));
    assert.equal(fileUriToPath(uri), trickyPath);
    assert.equal(percentDecodeUtf8('hello%20%D7%A9%D7%9C%D7%95%D7%9D'), 'hello שלום');

    // Also verify pure-JS UTF-8 fallback in a VM context without TextEncoder/TextDecoder!
    const vmFileUri = loadSource(
      new URL('../src/lib/fileUri.ts', import.meta.url),
      {},
      { TextEncoder: undefined, TextDecoder: undefined },
    );
    const vmUri = vmFileUri.pathToFileUri(trickyPath);
    assert.equal(vmUri, uri);
    assert.equal(vmFileUri.fileUriToPath(vmUri), trickyPath);
  });

  it('fails closed (isProd: true) when /k8s/clusters or /connections fails for an unverified bridge target', async () => {
    const guardMod = loadSource(
      new URL('../src/modules/ide/shell/networkBridgeGuard.svelte.ts', import.meta.url),
      {
        '../../../lib/api/client': {
          api: {
            get: async () => {
              throw new Error('503 daemon unavailable');
            },
          },
        },
        '../../../lib/confirm.svelte': {
          confirmer: { ask: async () => false },
        },
        '../../../lib/stores/workspace.svelte': {
          SCRATCH_WORKSPACE_ID: 'ws_scratch',
          ws: { currentId: 'ws_1', workspaces: [{ id: 'ws_1' }], scratch: { id: 'ws_scratch' } },
        },
      },
      {},
      { relative: true },
    );

    const cfg = {
      enabled: true,
      k8s_cluster_id: 'cls_unknown_mystery',
      k8s_namespace: 'default',
      ssh_tunnel_connection_id: null,
    };
    await guardMod.bridgeEnvCache.ensureLoaded('ws_1');
    const info = guardMod.bridgeEnvCache.inspect(cfg);
    assert.equal(info.isProd, true, 'must fail closed when cluster metadata failed to load');
    assert.match(info.summary, /unverified target/);
    assert.equal(await guardMod.confirmProdNetworkBridge(cfg, 'launch', 'ws_1'), false);
  });
});
