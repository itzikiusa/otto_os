// An unknown route (`#/nope`) used to show Agents silently and be saved as
// the window's last route and a module resume point; a role-locked deep link
// rendered a page that only failed with 403s. Now: the route gate page, and
// unknown routes are never persisted (lib/routeKeys.ts). Resume points are
// per workspace and survive a relaunch.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { PAGE_ROUTES, ephemeralHash, knownHash, knownRoute, parseResumePoints } from '../src/lib/routeKeys.ts';
import { routeAllowed } from '../src/lib/sidebar.ts';

const read = (p: string): string => readFileSync(join(import.meta.dirname, '..', 'src', p), 'utf8');

test('PAGE_ROUTES matches the shell page map (plus the gate page itself)', () => {
  const src = read('shell/pages.svelte.ts');
  const body = src.slice(src.indexOf('const LOADERS'), src.indexOf('};', src.indexOf('const LOADERS')));
  const keys = [...body.matchAll(/^\s*'?([a-z][a-z-]*)'?:\s*\(\)/gm)].map((m) => m[1]).filter((k) => k !== 'route-gate');
  assert.deepEqual([...keys].sort(), [...PAGE_ROUTES].sort());
});

test('pageKeyOf sends an unknown module to the gate, not to Agents, and checks the role gate', () => {
  const src = read('shell/pages.svelte.ts');
  const fn = src.slice(src.indexOf('export function pageKeyOf'), src.indexOf('/** Resolved pages'));
  assert.match(fn, /knownRoute\(m\) \? 'agents' : ROUTE_GATE/);
  assert.match(fn, /routeLocked\(parts\) \? ROUTE_GATE : key/);
  assert.match(fn, /routeAllowed\(parts\.join\('\/'\), \(f\) => auth\.can\(f, 'view'\)\)/);
  // Locked only once access is known: no "no access" flash while it loads.
  assert.match(fn, /auth\.phase !== 'ready' \|\| !auth\.me \|\| auth\.capabilitiesError/);
});

test('knownRoute / knownHash: page routes, the root App views and aliases; nothing else', () => {
  for (const m of ['', 'agents', 'git', 'settings', 'connections', 'plugin', 's', 'pair', 'room', 'projects']) assert.ok(knownRoute(m), m);
  for (const m of ['nope', 'history', 'mission-control', 'route-gate']) assert.ok(!knownRoute(m), m);
  assert.ok(knownHash('#/git/r1/pr/7'));
  assert.ok(knownHash('#/'));
  assert.ok(knownHash(''));
  assert.ok(!knownHash('#/nope/1'));
  assert.ok(!knownHash('#/%E0%A4%A'), 'a malformed escape is simply unknown');
});

test('a role gate locks a deep link the sidebar would hide', () => {
  const none = () => false;
  assert.equal(routeAllowed('kubernetes/c1/pods', none), false);
  assert.equal(routeAllowed('kubernetes/c1/pods', (f) => f === 'kubernetes'), true);
  // Ungated modules and routes without a sidebar entry stay open.
  assert.equal(routeAllowed('vault', none), true);
  assert.equal(routeAllowed('settings/appearance', none), true);
});

test('resume points: per workspace, only known non-ephemeral routes, corrupt storage = none', () => {
  const raw = JSON.stringify({
    w1: { git: '#/git/r1', kubernetes: '#/kubernetes/c1', bogus: '#/nope/x', share: '#/s/abc', bad: 3 },
    w2: { git: '#/git/r9' },
  });
  assert.deepEqual([...parseResumePoints(raw, 'w1')], [['git', '#/git/r1'], ['kubernetes', '#/kubernetes/c1']]);
  assert.deepEqual([...parseResumePoints(raw, 'w2')], [['git', '#/git/r9']]);
  assert.deepEqual([...parseResumePoints(raw, 'w3')], []);
  assert.deepEqual([...parseResumePoints('{oops', 'w1')], []);
  assert.deepEqual([...parseResumePoints(null, 'w1')], []);
  assert.ok(ephemeralHash('#/room-host/x') && ephemeralHash('#/pair') && !ephemeralHash('#/rooms'));
});

test('the router never persists an unknown route and keeps resume points in localStorage', () => {
  const src = read('lib/router.svelte.ts');
  assert.match(src, /if \(ephemeralRoute\(hash\) \|\| !knownHash\(hash\)\) return;/);
  assert.match(src, /if \(ephemeralRoute\(h\) \|\| !knownHash\(h\)\) return;/);
  assert.doesNotMatch(src, /sessionStorage/);
  assert.match(src, /setResumeScope\(workspaceId: string \| null\)/);
  assert.match(read('shell/installShell.svelte.ts'), /router\.setResumeScope\(id\)/);
});

// The "needs you" storage listener (App.svelte) compares against this export.
test('the attention key is exported for the cross-window storage listener', () => {
  assert.match(read('lib/stores/workspace.svelte.ts'), /export const LS_ATTENTION = 'otto_attention_v2';/);
});
