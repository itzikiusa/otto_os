// What the shared side system costs every page (scripts/bundle-budget.mjs
// measures it after a build; these pin the structure without one):
//   - phone-only shell pieces (the BottomNav, the Navigator drawer's contents)
//     are dynamic imports, so a desktop or tablet never loads them;
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

