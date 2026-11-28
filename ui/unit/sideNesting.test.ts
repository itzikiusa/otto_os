// Nested side surfaces (lib/components/side/): a ListPane belongs to the
// nearest ListDetail that declares it as its `list`. One placed in a page's
// CONTENT (a second-level list: the IDE's Git log, usages) is on its own — its
// own header, no "Hide <page list>", it does not take the page list's phone
// header — and a second ListDetail nested in the content has its own list.
// Rendered with the real components under Svelte's server renderer (no DOM;
// effects do not run, so the header markup is what is checked); the page-header
// pick (pageNav) is checked on its own below.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { compile } from 'svelte/compiler';
import { loadSource } from './sourceHarness.ts';

const SIDE = new URL('../src/lib/components/side/', import.meta.url);
const dir = mkdtempSync(join(tmpdir(), 'otto-side-'));
const at = (name: string, code: string): string => {
  writeFileSync(join(dir, name), code);
  return pathToFileURL(join(dir, name)).href;
};
/** Server code for a component, its imports pointed at the files below. */
const server = (source: string, file: string): string =>
  compile(source, { generate: 'server', filename: file }).js.code.replace(/from\s+'([^']+)'/g, (_m, spec: string) => {
    const to = map[spec];
    if (!to) throw new Error(`sideNesting: no mapping for '${spec}' in ${file}`);
    return `from '${to}'`;
  });
const stub = (name: string) => at(name, server('<i></i>', name));
const map: Record<string, string> = {
  svelte: import.meta.resolve('svelte'),
  'svelte/internal/server': import.meta.resolve('svelte/internal/server'),
};
const viewportUrl = at('viewport.mjs', 'export const viewport = { navDrawer: false };');
Object.assign(map, {
  '../PaneDivider.svelte': stub('PaneDivider.mjs'),
  '../Icon.svelte': stub('Icon.mjs'),
  '../LoadState.svelte': stub('LoadState.mjs'),
  '../../stores/viewport.svelte': viewportUrl,
  './pageNav.svelte': at('pageNav.mjs', 'export const pageNav = { claim: () => () => {} };'),
  '../../contextmenu.svelte': at('ctx.mjs', 'export const ctxMenu = { showAt() {} };'),
  './listNav': at('listNav.mjs', 'export function listNav() {}'),
  './paneKeys': new URL('paneKeys.ts', SIDE).href,
  './context': new URL('context.ts', SIDE).href,
});
function component(file: string, source = readFileSync(new URL(file, SIDE), 'utf8')): string {
  return (map[`./${file}`] = at(file.replace('.svelte', '.mjs'), server(source, file)));
}
for (const f of ['ListHost.svelte', 'ListPane.svelte', 'ListDetail.svelte']) component(f);
map.LD = map['./ListDetail.svelte'];
map.LP = map['./ListPane.svelte'];

const page = component(
  'Page.svelte',
  `<script>
    import ListDetail from 'LD';
    import ListPane from 'LP';
    let { open = false, nested = false } = $props();
  </script>
  <ListDetail module="t" name="Outer" {open} onback={() => {}}>
    {#snippet list()}<ListPane count={2}>outer rows</ListPane>{/snippet}
    {#if nested}
      <ListDetail module="t" pane="inner" name="Commits" open={false} onback={() => {}}>
        {#snippet list()}<ListPane count={4}>commit rows</ListPane>{/snippet}
        commit
      </ListDetail>
    {:else}
      <ListPane name="Usages" count={3}>usage rows</ListPane>
    {/if}
  </ListDetail>`,
);
const { render } = (await import(import.meta.resolve('svelte/server'))) as {
  render: (c: unknown, o: { props: Record<string, unknown> }) => { body: string };
};
const Page = ((await import(page)) as { default: unknown }).default;
const { viewport } = (await import(viewportUrl)) as { viewport: { navDrawer: boolean } };
const html = (props: Record<string, unknown>, phone = false): string => {
  viewport.navDrawer = phone;
  return render(Page, { props }).body;
};
const title = (name: string) => new RegExp(`<h2 class="sph-title[^"]*">${name}</h2>`);

test('a ListPane in the content is on its own: its own header, no Hide of the page list', () => {
  const out = html({});
  assert.match(out, /aria-label="Outer actions"/, 'the page list keeps ⋯ (Hide Outer ⌘B)');
  assert.match(out, title('Usages'), 'the inner list draws its own header');
  assert.doesNotMatch(out, /aria-label="Usages actions"/, 'no ⋯: it has no menu and nothing of the page list to hide');
});

test('phone, the page list is the page: the inner list keeps its own header row', () => {
  const out = html({}, true);
  assert.doesNotMatch(out, title('Outer'), 'the page list hands its header to PageHeader');
  assert.match(out, title('Usages'), 'the inner list does not think it is the page list');
});

test('a ListDetail nested in the content has its own list (named by it, hidden by it)', () => {
  const out = html({ nested: true });
  assert.match(out, title('Commits'));
  assert.match(out, /aria-label="Commits actions"/, 'its ⋯ hides Commits');
});

test('the page header takes the NEARER list: depth wins over the order the panes registered in', () => {
  const { pageNav } = loadSource(new URL('pageNav.svelte.ts', SIDE), { svelte: { untrack: (f: () => unknown) => f() }, '../../keys': { keyContext: {} } });
  const entry = (depth: number, label: string) => ({
    depth,
    active: () => true,
    back: () => ({ label, onclick() {} }),
    show: () => null,
    toggle: () => null,
    listPage: () => null,
  });
  // Svelte runs a child's $effect before its parent's: the nested list claims first.
  const inner = pageNav.claim(entry(1, 'Commits'));
  const outer = pageNav.claim(entry(0, 'Files'));
  assert.equal(pageNav.back?.label, 'Commits', '‹ names the nearer list');
  inner();
  assert.equal(pageNav.back?.label, 'Files');
  outer();
  assert.equal(pageNav.back, null);
});

test('no module copies the list-pane chrome: a second-level list is a standalone ListPane', () => {
  const modules = new URL('../src/modules/', import.meta.url);
  const copies = readdirSync(modules, { recursive: true, encoding: 'utf8' })
    .filter((f) => f.endsWith('.svelte'))
    .filter((f) => /class="side-list[\s"]|class="side-pane-head[\s"]/.test(readFileSync(new URL(f, modules), 'utf8')));
  assert.deepEqual(copies, []);
});
