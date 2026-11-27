import assert from 'node:assert/strict';
import { test } from 'node:test';
import { normalizeIdeLspLanguage, shouldAttachIdeLsp } from '../src/lib/components/cm-ide-lsp-completion.ts';
import { ideLanguageLabel, ideLanguageSettingsSection, ideLspLangFor, ideLspServerName } from '../src/modules/ide/editor/ideStatusModel.ts';
import { nestParentName } from '../src/modules/ide/history/ideTreeNesting.ts';
import { buildDedupedBulkRepos } from '../src/modules/ide/shell/bulkRepoModel.ts';
import { inferTestDebugLanguage } from '../src/modules/ide/debug/ideFailedTestDebug.ts';
import { langGutters, langKeyFor } from '../src/lib/components/langForPath.ts';

test('Web framework LSP gating, status strip labels, and settings links', () => {
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
