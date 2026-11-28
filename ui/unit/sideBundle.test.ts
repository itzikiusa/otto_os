// What the shared side system costs every page (scripts/bundle-budget.mjs
// measures it after a build; this pins the structure without one):
//   - PageHeader, on every page, imports from side/ only what the shell
//     already loads, so it adds no side chunk of its own to each page.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), 'utf8');
const imports = (src: string) => [...src.matchAll(/^\s*import\s[^'"]*?from\s+'([^']+)'/gm)].filter((m) => !/^\s*import\s+type\s/.test(m[0])).map((m) => m[1]);

test('PageHeader takes from side/ only what the shell already loads', () => {
  const side = (src: string, base: string) =>
    imports(src)
      .filter((s) => s.includes('side/'))
      .map((s) => new URL(s, `file:///src/${base}`).pathname.replace(/\.svelte$|\.ts$/, ''));
  const shell = new Set(
    ['shell/App.svelte', 'shell/TabBar.svelte', 'shell/Navigator.svelte', 'shell/Rail.svelte'].flatMap((f) => side(read(f), f)),
  );
  const header = side(read('lib/components/PageHeader.svelte'), 'lib/components/PageHeader.svelte');
  assert.ok(header.length > 0);
  assert.deepEqual(header.filter((p) => !shell.has(p)), []);
});
