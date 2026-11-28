// What the shared side system costs every page (scripts/bundle-budget.mjs
// measures it after a build; these pin the structure without one):
//   - phone-only shell pieces (the BottomNav, the Navigator drawer's contents)
//     are dynamic imports, so a desktop or tablet never loads them;
//   - PageHeader, on every page, imports from side/ only what the shell
//     already loads, so it adds no side chunk of its own to each page.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), 'utf8');
const imports = (src: string) => [...src.matchAll(/^\s*import\s[^'"]*?from\s+'([^']+)'/gm)].filter((m) => !/^\s*import\s+type\s/.test(m[0])).map((m) => m[1]);

test('phone-only shell pieces load on demand: no static import from the shell', () => {
  const app = read('shell/App.svelte');
  for (const name of ['BottomNav', 'PhoneNavDrawer']) {
    assert.doesNotMatch(app, new RegExp(`^\\s*import ${name} from`, 'm'), `${name} is a lazyComponent (shell/overlays.svelte.ts)`);
    assert.match(read('shell/overlays.svelte.ts'), new RegExp(`import\\('\\./${name}\\.svelte'\\)`));
  }
});

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
