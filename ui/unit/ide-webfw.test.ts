import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';
import { buildLspInitializationOptions, normalizeIdeLspLanguage, shouldAttachIdeLsp } from '../src/lib/components/cm-ide-lsp-completion.ts';
import { ideLanguageLabel, ideLanguageSettingsSection, ideLspLangFor, ideLspServerName } from '../src/modules/ide/editor/ideStatusModel.ts';
import { nestParentName } from '../src/modules/ide/history/ideTreeNesting.ts';
import { buildDedupedBulkRepos } from '../src/modules/ide/shell/bulkRepoModel.ts';
import { inferTestDebugLanguage } from '../src/modules/ide/debug/ideFailedTestDebug.ts';
import { langGutters, langKeyFor } from '../src/lib/components/langForPath.ts';

test('Web framework LSP gating, status strip labels, and settings links', async () => {
  assert.equal(normalizeIdeLspLanguage('svelte'), 'svelte');
  assert.equal(normalizeIdeLspLanguage('vue'), 'vue');
  assert.equal(normalizeIdeLspLanguage('angular'), 'angular');

  assert.equal(shouldAttachIdeLsp('/app/src/App.svelte', 'svelte', '/app'), true);
  assert.equal(shouldAttachIdeLsp('/app/src/App.vue', 'vue', '/app'), true);
  assert.equal(shouldAttachIdeLsp('/app/src/app/hero.component.html', 'angular', '/app'), true);

  assert.equal(ideLanguageLabel('/app/src/App.svelte', 'svelte'), 'Svelte');
  assert.equal(ideLanguageLabel('/app/src/App.vue', 'vue'), 'Vue');
  assert.equal(ideLanguageSettingsSection('/app/src/App.svelte', 'svelte'), 'lang.node');
  assert.equal(ideLanguageSettingsSection('/app/src/App.vue', 'vue'), 'lang.node');

  assert.equal(ideLspLangFor('/app/src/App.svelte', 'svelte'), 'svelte');
  assert.equal(ideLspLangFor('/app/src/App.vue', 'vue'), 'vue');
  assert.equal(ideLspLangFor('/app/src/app/hero.component.html', 'html'), 'angular');
  assert.equal(ideLspServerName('svelte'), 'svelteserver');
  assert.equal(ideLspServerName('vue'), 'vue-language-server');
  assert.equal(ideLspServerName('angular'), 'ngserver');

  // SEC3-2: a Vue LSP trust refusal asks for trust in the status strip.
  const { lspHandshakeFailureReason } = await import('../src/lib/components/cm-ide-lsp-reconnect.ts');
  const { ideLspItem } = await import('../src/modules/ide/editor/ideStatusModel.ts');
  assert.equal(lspHandshakeFailureReason('vue'), 'trust_or_unavailable');
  const vueItem = ideLspItem('vue', {
    lang: 'vue',
    rootPath: '/app',
    phase: 'unavailable',
    reason: 'trust_or_unavailable',
  }, false);
  assert.equal(vueItem?.label, 'vue-language-server needs trust');
  assert.equal(vueItem?.action, 'trust');
});

test('Angular and SvelteKit tree nesting, repo badges, and runner/debug metadata', () => {
  assert.equal(nestParentName('hero.component.html'), 'hero.component.ts');
  assert.equal(nestParentName('hero.component.scss'), 'hero.component.ts');
  assert.equal(nestParentName('hero.component.spec.ts'), 'hero.component.ts');
  assert.equal(nestParentName('+page.ts'), '+page.svelte');
  assert.equal(nestParentName('+page.server.ts'), '+page.svelte');
  assert.equal(nestParentName('+layout.server.ts'), '+layout.svelte');

  const { techByPath } = buildDedupedBulkRepos([
    { name: 'ng-app', path: '/ws/ng-app', kind: 'angular', has_go_mod: false, has_pom_or_gradle: false, has_git: true },
    { name: 'next-app', path: '/ws/next-app', kind: 'nextjs', has_go_mod: false, has_pom_or_gradle: false, has_git: true },
    { name: 'svelte-app', path: '/ws/svelte-app', kind: 'svelte', has_go_mod: false, has_pom_or_gradle: false, has_git: true },
    { name: 'nuxt-app', path: '/ws/nuxt-app', kind: 'nuxt', has_go_mod: false, has_pom_or_gradle: false, has_git: true },
  ]);
  assert.deepEqual(techByPath['/ws/ng-app'], ['angular']);
  assert.deepEqual(techByPath['/ws/next-app'], ['nextjs']);
  assert.deepEqual(techByPath['/ws/svelte-app'], ['svelte']);
  assert.deepEqual(techByPath['/ws/nuxt-app'], ['nuxt']);

  for (const r of ['vitest', 'jest', 'playwright', 'karma']) {
    assert.equal(inferTestDebugLanguage(null, r), 'javascript');
  }
  assert.equal(inferTestDebugLanguage('/app/src/App.svelte'), 'typescript');
  assert.equal(inferTestDebugLanguage('/app/src/App.vue'), 'typescript');
  assert.equal(langKeyFor('/app/src/App.svelte'), 'svelte');
  assert.equal(langKeyFor('/app/src/App.vue'), 'vue');
  assert.equal(langGutters('/app/src/App.svelte').breakpoints, true);
  assert.equal(langGutters('/app/src/App.vue').breakpoints, true);
});

test('BUG-26: Angular .component.html attaches to angular LSP in both editor selector and status strip', () => {
  const keymapMod = loadSource(
    new URL('../src/lib/components/cm-ide-keymap.ts', import.meta.url),
    {
      '@codemirror/commands': { toggleComment: () => false },
      '@codemirror/search': {},
      '@codemirror/state': {},
      '@codemirror/view': {},
      './cm-ide-quickfix.ts': {},
      './cm-ide-semantic.ts': {},
    },
  );
  assert.equal(keymapMod.lspLanguageIdFor('/app/src/app/hero.component.html', true, 'html'), 'angular');
  assert.equal(keymapMod.lspLanguageIdFor('/app/src/app/hero.component.html', true), 'angular');
  assert.equal(keymapMod.lspLanguageIdFor('/app/src/App.svelte', true), 'svelte');
  assert.equal(keymapMod.lspLanguageIdFor('/app/src/App.vue', true), 'vue');
  assert.equal(shouldAttachIdeLsp('/app/src/app/hero.component.html', 'html', '/app'), true);
  assert.equal(ideLspLangFor('/app/src/app/hero.component.html', 'html'), 'angular');
  assert.equal(buildLspInitializationOptions('svelte')?.isTrusted, false);
  assert.deepEqual(buildLspInitializationOptions('vue'), { vue: { hybridMode: false } });
});

test('BUG-27: resolveFrameworkDevDebugLaunch forwards framework CLI instead of package.json', async () => {
  const { resolveFrameworkDevDebugLaunch } = await import('../src/modules/ide/store/ideDebugLaunchHelpers.ts');
  const viteDev = resolveFrameworkDevDebugLaunch({
    id: 'auto:fw-dev',
    name: 'Vite Dev',
    runner: 'npm',
    command: 'npx',
    args: ['--no-install', 'vite'],
    target_path: 'package.json',
    env_overrides: {},
    inherit_shell_env: true,
    build_flags: [],
    compound_configs: [],
    auto_detected: true,
  });
  assert.deepEqual(viteDev, { program: 'vite', args: [] });

  const ngDev = resolveFrameworkDevDebugLaunch({
    id: 'auto:fw-dev',
    name: 'Angular Dev',
    runner: 'npm',
    command: 'npx',
    args: ['--no-install', 'ng', 'serve'],
    target_path: 'ng',
    env_overrides: {},
    inherit_shell_env: true,
    build_flags: [],
    compound_configs: [],
    auto_detected: true,
  });
  assert.deepEqual(ngDev, { program: 'ng', args: ['serve'] });

  // BUG-30: auto-discovered `npm run test` config preserves OTTO_NPM_SCRIPT=test and is_test=true
  const { resolveConfigDebugHints } = await import('../src/modules/ide/store/ideRepoTestAndBuildTags.ts');
  const npmTestHints = resolveConfigDebugHints({ runner: 'npm', kind: 'test', args: ['run', 'test'] });
  assert.equal(npmTestHints.isTest, true);
  assert.equal(npmTestHints.envHints.OTTO_NPM_SCRIPT, 'test');
});

test('API2-2: Angular .component.html connects to angular server but sends languageId html in didOpen', async () => {
  const { lspDocumentLanguageIdFor, normalizeIdeLspLanguage } = await import('../src/lib/components/cm-ide-lsp-completion.ts');
  assert.equal(normalizeIdeLspLanguage('angular'), 'angular');
  assert.equal(lspDocumentLanguageIdFor('angular', '/app/src/app/hero.component.html'), 'html');
  assert.equal(lspDocumentLanguageIdFor('html', '/app/src/app/hero.component.html'), 'html');
  assert.equal(lspDocumentLanguageIdFor('svelte', '/app/src/App.svelte'), 'svelte');

  // Exercise the actual direct LSP plugin WebSocket handshake to verify `textDocument/didOpen`.
  const sent: Record<string, unknown>[] = [];
  const listeners: Record<string, ((ev?: unknown) => void)[]> = {};
  class FakeWS {
    static OPEN = 1;
    readyState = 1;
    url: string;
    constructor(url: string) { this.url = url; }
    addEventListener(ev: string, cb: (e?: unknown) => void) {
      (listeners[ev] ??= []).push(cb);
    }
    send(raw: string) {
      const msg = JSON.parse(raw) as Record<string, unknown>;
      sent.push(msg);
      if (msg.method === 'initialize') {
        for (const cb of listeners.message ?? []) {
          cb({ data: JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: { capabilities: {} } }) });
        }
      }
    }
    close() {}
  }
  const prevWS = (globalThis as { WebSocket?: unknown }).WebSocket;
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeWS;
  try {
    let pluginCtor: ((v: unknown) => unknown) | null = null;
    const directMod = loadSource(
      new URL('../src/lib/components/cm-ide-lsp-direct.ts', import.meta.url),
      {
        '@codemirror/view': {
          EditorView: { theme: () => ({}) },
          ViewPlugin: {
            fromClass: (cls: new (v: unknown) => unknown) => {
              pluginCtor = (v: unknown) => new cls(v);
              return {};
            },
          },
        },
        '@codemirror/lint': { setDiagnostics: () => ({}) },
        '../fileUri.ts': { fileUriToPath: (u: string) => u.replace(/^file:\/\//, '') },
        './cm-ide-drafts.ts': { markLiveLspDiagnostics: () => {} },
        './cm-ide-lsp-completion.ts': {
          buildLspInitializationOptions: () => undefined,
          createIdeLspCompletionExtension: () => ({}),
          createLspSemanticTokensExtension: () => ({}),
          dedupeLspDiagnostics: (d: unknown) => d,
          DEFAULT_LSP_TOKEN_MODIFIERS: [],
          DEFAULT_LSP_TOKEN_TYPES: [],
          lspDocumentLanguageIdFor,
          lspToPos: () => 0,
          rewriteGoBuildTagDiagnostic: (d: unknown) => d,
          shouldAttachIdeLsp: () => true,
        },
      },
      { WebSocket: FakeWS },
    );
    directMod.createDirectLspExtension({
      wsUri: 'ws://127.0.0.1:7700/ws/lsp?lang=angular&root=/app',
      rootUri: 'file:///app',
      documentUri: 'file:///app/src/app/hero.component.html',
      filePath: '/app/src/app/hero.component.html',
      languageId: 'angular',
    });
    assert.ok(pluginCtor, 'ViewPlugin class should be constructed');
    (pluginCtor as (v: unknown) => unknown)({ state: { doc: { toString: () => '<h1>{{ name }}</h1>' } } });
    for (const cb of listeners.open ?? []) cb();
    const didOpen = sent.find((m) => m.method === 'textDocument/didOpen') as
      | { params?: { textDocument?: { languageId?: string; uri?: string } } }
      | undefined;
    assert.ok(didOpen, 'textDocument/didOpen must be sent after initialize');
    assert.equal(didOpen?.params?.textDocument?.languageId, 'html');
    assert.equal(didOpen?.params?.textDocument?.uri, 'file:///app/src/app/hero.component.html');
  } finally {
    (globalThis as { WebSocket?: unknown }).WebSocket = prevWS;
  }
});
