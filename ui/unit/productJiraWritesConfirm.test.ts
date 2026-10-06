// S18-302: every live Jira write in the story Overview (status, assignee,
// title, description, custom fields, comments) notifies watchers and can fire
// Jira automations, so each one must go through `confirmOutward` first. This
// pins it per function: a new `/issue/…` PUT/POST without a confirm fails.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('../src/modules/product/OverviewTab.svelte', import.meta.url), 'utf8');

/** Body of every top-level `async function name(` in the script block. */
function functions(text: string): Array<{ name: string; body: string }> {
  const out: Array<{ name: string; body: string }> = [];
  const re = /\n {2}async function (\w+)\s*\(/g;
  const starts = [...text.matchAll(re)];
  for (let i = 0; i < starts.length; i++) {
    const from = starts[i].index ?? 0;
    const to = i + 1 < starts.length ? (starts[i + 1].index ?? text.length) : text.indexOf('</script>', from);
    out.push({ name: starts[i][1], body: text.slice(from, to) });
  }
  return out;
}

test('every Overview Jira write is confirmed before it is sent', () => {
  const writers = functions(src).filter((f) => /api\.(put|post)[^(]*\(\s*`\/issue\//.test(f.body) || /api\.(put|post)<[^>]+>\(\s*`\/issue\//.test(f.body));
  assert.ok(writers.length >= 5, `expected the Jira writers, found ${writers.map((f) => f.name).join(', ')}`);
  for (const f of writers) {
    const confirmAt = f.body.indexOf('confirmOutward(');
    const writeAt = f.body.search(/api\.(put|post)/);
    assert.ok(confirmAt >= 0 && confirmAt < writeAt, `${f.name} writes to Jira without confirmOutward first`);
  }
  assert.ok(writers.some((f) => f.name === 'saveField'), 'saveField is a Jira writer');
});
