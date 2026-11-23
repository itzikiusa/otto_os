// "MCP servers changed" re-reads the workspace's servers on mount and when
// the agent reports its servers again — never per stream batch. Its
// `servers` prop must be a stable derived value: an inline
// `p?.mcpServers ?? []` makes the child's effect depend on the whole
// projection (a new object per batch) and builds a new [] each time.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('../src/modules/threads/runtime/RuntimeThread.svelte', import.meta.url), 'utf8');

test('RuntimeThread passes RuntimeMcpChanged a derived, stable servers list', () => {
  const tag = /<RuntimeMcpChanged[^>]*>/.exec(src)?.[0] ?? '';
  const prop = /servers=\{([^}]+)\}/.exec(tag)?.[1]?.trim() ?? '';
  assert.match(prop, /^[A-Za-z_$][\w$]*$/, `servers={${prop}} must be a plain derived name`);
  assert.match(src, new RegExp(`const ${prop} = \\$derived\\(`));
});
