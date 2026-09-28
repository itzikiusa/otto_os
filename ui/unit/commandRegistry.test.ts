// ⌘K command registry under the REAL Svelte runtime (unit/runesHarness.ts).
// SF-06 root cause: an unregister that rebuilt the map from `$state` inside an
// effect teardown read Svelte's pre-flush `old_values`, so when two
// registration effects re-ran in the same flush the second teardown dropped
// the first one's fresh set ("go to vault" lost every Go-to entry).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { importRunes, svelteClient } from './runesHarness.ts';

type Cmd = { id: string; title: string; run: () => unknown };
type Registry = { register: (owner: string, cmds: Cmd[]) => () => void; readonly all: Cmd[] };

const cmd = (id: string): Cmd => ({ id, title: id, run: () => {} });

async function fresh(): Promise<Registry> {
  const mod = await importRunes<{ registry: Registry }>(new URL('../src/lib/commands.svelte.ts', import.meta.url));
  return mod.registry;
}

test('two registration effects re-running in one flush keep both sets', async () => {
  const registry = await fresh();
  const $ = await svelteClient();
  const trig = $.state(0);
  const stop = $.effectRoot(() => {
    $.effect(() => registry.register('core', [cmd('core.new')]));
    $.effect(() => {
      const n = $.get(trig);
      return registry.register('nav', n === 0 ? [] : [cmd('core.go-vault'), cmd('core.go-git')]);
    });
    // App's 'side-pane-commands' shape: same trigger, registers after 'nav'.
    $.effect(() => {
      void $.get(trig);
      return registry.register('side', []);
    });
  });
  $.flushSync();
  $.set(trig, 1);
  $.flushSync();
  assert.deepEqual(
    registry.all.map((c) => c.id).sort(),
    ['core.go-git', 'core.go-vault', 'core.new'],
    'the Go-to set registered in this flush survives the later teardown',
  );
  $.set(trig, 2);
  $.flushSync();
  assert.equal(registry.all.length, 3, 'stable across another joint re-run');
  stop();
  $.flushSync();
  assert.deepEqual(registry.all, [], 'teardown of every owner empties the registry');
});

test('a stale unregister never removes the set that replaced it', async () => {
  const registry = await fresh();
  const first = registry.register('x', [cmd('a')]);
  registry.register('x', [cmd('b')]);
  first();
  assert.deepEqual(registry.all.map((c) => c.id), ['b']);
});

test('registry.all is memoized: unchanged sources hand back the same array', async () => {
  const registry = await fresh();
  const $ = await svelteClient();
  registry.register('a', [cmd('a1')]);
  let seen: Cmd[][] = [];
  const stop = $.effectRoot(() => {
    $.effect(() => {
      seen.push(registry.all);
    });
  });
  $.flushSync();
  const before = seen.length;
  const again = registry.all;
  assert.equal(again, seen[before - 1], 'reads without a write reuse the flat list');
  registry.register('b', [cmd('b1')]);
  $.flushSync();
  assert.equal(seen.length, before + 1, 'one re-run per registration');
  stop();
  seen = [];
});
