// Threads on a phone: what folds away while typing with the on-screen
// keyboard up (threadsTyping.ts), phone code-block wrapping (codeBlocks.ts),
// and the shell's safe-area / keyboard / tablet-navigator wiring (source
// checks: these are CSS and markup decisions).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ALL_CHROME, TYPING_FOLD, typingChrome } from '../src/modules/threads/threadsTyping.ts';
import { decorateCodeBlocks, wrapsByDefault } from '../src/modules/agents/conversation/codeBlocks.ts';

const read = (p: string): string => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');

test('keyboard closed: every piece of chrome stays, at any height', () => {
  assert.deepEqual(typingChrome(false, true, 200), ALL_CHROME);
  assert.deepEqual(typingChrome(true, true, 0), ALL_CHROME, 'not measured yet');
});

test('iPhone portrait with the keyboard up (~406 px): plan/queue and the terminal fold, header and the full composer stay', () => {
  assert.deepEqual(typingChrome(true, true, 406), { header: true, banners: false, terminal: false, tight: false });
  // Android portrait (~461 px): only the terminal drawer gives way.
  assert.deepEqual(typingChrome(true, true, 461), { header: true, banners: true, terminal: false, tight: false });
});

test('a phone on its side / an iPhone SE with the keyboard up: one-row composer, no header', () => {
  assert.deepEqual(typingChrome(true, true, 200), { header: false, banners: false, terminal: false, tight: true });
  assert.deepEqual(typingChrome(true, true, 312), { header: false, banners: false, terminal: false, tight: true });
  assert.ok(TYPING_FOLD.tight < TYPING_FOLD.banners && TYPING_FOLD.banners < TYPING_FOLD.terminal, 'least needed folds first');
});

test('typing into the terminal drawer (not the composer) keeps the drawer and the notices', () => {
  assert.deepEqual(typingChrome(true, false, 200), { header: false, banners: true, terminal: true, tight: false });
});

test('phone code blocks wrap by default, diffs keep one line per line', () => {
  assert.equal(wrapsByDefault({ dataset: { lang: 'ts' } }, true), true);
  assert.equal(wrapsByDefault({ dataset: {} }, true), true);
  assert.equal(wrapsByDefault({ dataset: { lang: 'diff' } }, true), false);
  assert.equal(wrapsByDefault({ dataset: { lang: 'Patch' } }, true), false);
  assert.equal(wrapsByDefault({ dataset: { lang: 'ts' } }, false), false, 'desktop: Wrap is opt-in');
  // The decorated block carries the language the CSS keys on.
  const html = decorateCodeBlocks('<pre><code class="hljs language-diff"><span class="cl">+a</span></code></pre>');
  assert.match(html, /data-lang="diff"/);
  const css = read('src/app.css');
  assert.match(css, /@media \(max-width: 640px\), \(max-height: 500px\) and \(pointer: coarse\) \{[\s\S]*\.md \.code-block:not\(\.nowrap\):not\(\[data-lang='diff'\], \[data-lang='patch'\]\) \.cl \{\s*white-space: pre-wrap;/);
  assert.match(css, /\.hljs-addition \{\s*color: var\(--success\);\s*background: var\(--success-soft\);/);
  assert.match(css, /\.hljs-deletion \{\s*color: var\(--danger\);\s*background: var\(--danger-soft\);/);
});

test('safe areas: one set of variables, padded once by the compact shell', () => {
  const css = read('src/app.css');
  for (const v of ['top', 'bottom', 'left', 'right']) assert.match(css, new RegExp(`--safe-${v}: env\\(safe-area-inset-${v}, 0px\\);`));
  assert.match(css, /:root\[dir='rtl'\] \{\s*--safe-start: var\(--safe-right\);\s*--safe-end: var\(--safe-left\);/);
  assert.match(read('index.html'), /viewport-fit=cover/);
  assert.match(css, /\.shell\.mobile \{\s*padding-block-start: var\(--safe-top\);\s*padding-inline: var\(--safe-start\) var\(--safe-end\);/);
  // The home indicator: the BottomNav pads it; without one the shell does,
  // and nothing is added above an open keyboard.
  assert.match(css, /\.shell\.mobile:not\(\.bnav\) \{\s*padding-block-end: var\(--safe-bottom\);/);
  assert.match(css, /html\[data-kb='open'\] \.shell\.mobile \{\s*padding-block-end: 0;/);
  // env() is read in exactly one place per edge.
  assert.equal(css.match(/env\(safe-area-inset-/g)?.length, 4);
});
