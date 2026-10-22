// architecture-2-05: the shell's palette sources left App.svelte. The repo
// commands are a pure builder; App keeps no feature palette source of its own.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { repoCommands } from '../src/shell/paletteSources/repos.ts';

test('one "Open repository" command per repo, routed to its git page', () => {
  const went: string[] = [];
  const cmds = repoCommands([{ id: 'r1', name: 'otto', path: '/src/otto' } as never], (r) => went.push(r));
  assert.equal(cmds.length, 1);
  assert.equal(cmds[0].id, 'repo.r1');
  assert.equal(cmds[0].title, 'Open repository otto');
  assert.match(cmds[0].keywords ?? '', /\/src\/otto/);
  cmds[0].run();
  assert.deepEqual(went, ['git/r1']);
});

test('App.svelte installs the sources once from onMount instead of registering them itself', () => {
  const app = readFileSync(new URL('../src/shell/App.svelte', import.meta.url), 'utf8');
  for (const owner of ['core', 'nav', 'favorites', 'settings', 'guides', 'workspaces', 'sessions', 'repos', 'connections']) {
    assert.doesNotMatch(app, new RegExp(`registry\\.register\\(\\s*'${owner}'`), `${owner} belongs in shell/paletteSources.svelte.ts`);
  }
  assert.match(app, /onMount\(\(\) =>\s*installShell\(/);
  const sources = readFileSync(new URL('../src/shell/paletteSources.svelte.ts', import.meta.url), 'utf8');
  assert.match(sources, /loadConnectionCommands\(wsId\)/, 'connections come from modules/connections/paletteCommands.ts');
});
