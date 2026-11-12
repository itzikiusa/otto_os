import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  NOT_GOVERNED_NOTE,
  SERVERS_ROUTE,
  nonEmptyLines,
  parseEnvLines,
  secretEnvBody,
  serverRuns,
  switchBlocked,
  switchConfirm,
} from '../src/modules/mcp/runMode.ts';
import { availableSections, findSection, movedSectionRoute } from '../src/modules/settings/sections.ts';

test('the managed flag is the run mode, one per server', () => {
  assert.equal(serverRuns({ managed: true }), 'governed');
  assert.equal(serverRuns({ managed: false }), 'native');
  assert.match(NOT_GOVERNED_NOTE, /no policy, approval or audit/);
});

test('a switch says exactly what changes, including the tool names', () => {
  const g = switchConfirm('linear', 'governed');
  assert.match(g.message, /stops being written into each session’s \.mcp\.json/);
  assert.match(g.message, /mcp__linear__<tool>/);
  assert.match(g.message, /skills and prompts that name the native tools must be updated/);
  assert.match(g.message, /secret values stay in the Keychain/);
  const n = switchConfirm('linear', 'native');
  assert.match(n.message, /No policy, approval or audit applies/);
  assert.match(n.message, /Skills and prompts that name its governed tools/);
  assert.notEqual(g.confirmLabel, n.confirmLabel);
});

test('only a stdio server can run natively, and a no-op switch is refused', () => {
  assert.equal(switchBlocked({ transport: 'stdio', managed: true }, 'native'), null);
  assert.equal(switchBlocked({ transport: 'stdio', managed: false }, 'governed'), null);
  assert.match(switchBlocked({ transport: 'http', managed: true }, 'native') ?? '', /stdio/);
  assert.match(switchBlocked({ transport: 'stdio', managed: false }, 'native') ?? '', /already/);
});

test('native env parsing keeps the Settings page’s rules', () => {
  assert.deepEqual(nonEmptyLines(' -y \n\n@linear/mcp\n'), ['-y', '@linear/mcp']);
  assert.deepEqual(parseEnvLines('A=1\nB=\n=x\nC=a=b'), { A: '1', C: 'a=b' });
  assert.deepEqual(parseEnvLines('B=', true), { B: '' });
  // Edit: a typed value replaces, a bare stored key keeps (''), an unknown
  // bare key and an omitted stored key are dropped.
  assert.deepEqual(secretEnvBody('TOKEN=\nNEW=v\nGHOST=', ['TOKEN', 'OLD'], true), { TOKEN: '', NEW: 'v' });
  // Create: nothing is stored, so a bare key is never sent.
  assert.deepEqual(secretEnvBody('TOKEN=\nNEW=v', ['TOKEN'], false), { NEW: 'v' });
});

test('Settings ▸ MCP servers redirects to the Control Plane’s Servers page', () => {
  assert.equal(SERVERS_ROUTE, 'mcp/servers');
  assert.equal(movedSectionRoute('mcp-servers'), SERVERS_ROUTE);
  // Out of the Settings list; the route itself still resolves (old links).
  assert.ok(findSection('mcp-servers'));
  assert.ok(!availableSections({ can: () => true, isRoot: true }).some((s) => s.id === 'mcp-servers'));
});
