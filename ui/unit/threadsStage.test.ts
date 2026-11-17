// The new-thread stage and the Threads page frame (round 5, T-STAGE): which
// folder a new thread starts in (newThreadDefaults.ts), folder names, the
// same-checkout warning, the access caution, closing a thread with children
// (threadClosePlan.ts), the toolbar ⋯ per viewport (threadsPageMenu.ts) and
// the right panel's width at 1280 (threadsLayout.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  accessCaution,
  canonicalPath,
  defaultStageFolder,
  folderCandidates,
  folderLabel,
  parentHint,
  stageAccess,
  threadsInCheckout,
} from '../src/modules/threads/newThreadDefaults.ts';
import { closeWithChildrenDialog, openDescendants } from '../src/modules/threads/threadClosePlan.ts';
import { threadsPageMenu, RELATED_LABEL, TERMINAL_LABEL } from '../src/modules/threads/threadsPageMenu.ts';
import { fitRightPanel } from '../src/modules/threads/threadsLayout.ts';

const HOME = '/Users/me';
const billing = { name: 'billing-service', path: '/Users/me/code/billing' };
const web = { name: 'web', path: '/Users/me/code/web' };
const wsBilling = { name: 'Billing WS', root_path: '/Users/me/code/billing' };

test('stage folder: home until the workspace loads, then the workspace — no pick needed', () => {
  const base = { repos: [], recentCwds: [], lastProject: null, homePath: HOME };
  // Workspace not loaded yet (or none): the home folder, labelled as such.
  assert.deepEqual(defaultStageFolder({ ...base, workspace: null }), { path: HOME, kind: 'home' });
  // It arrives: the same inputs re-derive to its repo (the old bug seeded
  // `~` once and never looked again).
  assert.deepEqual(defaultStageFolder({ ...base, workspace: wsBilling, repos: [billing] }), { path: billing.path, kind: 'repo' });
  // A workspace with no repo: its root.
  assert.deepEqual(defaultStageFolder({ ...base, workspace: { name: 'Docs', root_path: '/Users/me/docs' } }), { path: '/Users/me/docs', kind: 'workspace' });
});

test('stage folder: several repos — the most recently used, else the root repo, else the first', () => {
  const ws = { name: 'Mono', root_path: '/Users/me/code' };
  const base = { workspace: ws, repos: [billing, web], lastProject: null, homePath: HOME };
  assert.equal(defaultStageFolder({ ...base, recentCwds: ['/tmp/x', '/Users/me/code/web/src'] }).path, web.path);
  assert.equal(defaultStageFolder({ ...base, recentCwds: [] }).path, billing.path);
  assert.equal(defaultStageFolder({ ...base, repos: [billing, { name: 'root', path: '/Users/me/code' }], recentCwds: [] }).path, '/Users/me/code');
});

test('stage folder: the last project wins only inside this workspace', () => {
  const base = { workspace: wsBilling, repos: [billing], recentCwds: [], homePath: HOME };
  assert.deepEqual(defaultStageFolder({ ...base, lastProject: '/Users/me/code/billing/api' }), { path: '/Users/me/code/billing/api', kind: 'last' });
  // Another workspace's project never leaks in.
  assert.equal(defaultStageFolder({ ...base, lastProject: '/Users/me/elsewhere' }).kind, 'repo');
  // No workspace: the last Scratch project, else home.
  assert.deepEqual(defaultStageFolder({ workspace: null, repos: [], recentCwds: [], lastProject: '/tmp/play', homePath: HOME }), { path: '/tmp/play', kind: 'last' });
});

test('folder names: Home, repo, workspace, then the folder; /private twins fold', () => {
  const n = { homePath: HOME, repos: [billing], workspaces: [{ name: 'Docs WS', root_path: '/Users/me/docs' }] };
  assert.equal(folderLabel(HOME, n), 'Home');
  assert.equal(folderLabel('~', n), 'Home');
  assert.equal(folderLabel('/Users/me/code/billing/', n), 'billing-service');
  assert.equal(folderLabel('/Users/me/docs', n), 'Docs WS');
  assert.equal(folderLabel('/Users/me/play/sandbox', n), 'sandbox');
  assert.equal(canonicalPath('/private/var/folders/T/x/'), '/var/folders/T/x');
  // A long parent keeps its tail.
  assert.equal(parentHint('/Users/me/a/very/long/parent/path/name', HOME, 14), '…g/parent/path');
  assert.equal(parentHint('/Users/me/a/app', HOME), '~/a');
});

test('folder menu rows: one per folder, never two identical names without the parent', () => {
  const n = { homePath: HOME, repos: [], workspaces: [] };
  const rows = folderCandidates(
    [
      { path: '/var/folders/T/repo', label: 'Billing WS' },
      { path: '/private/var/folders/T/repo' },
      { path: '/Users/me/a/app' },
      { path: '/Users/me/b/app' },
      { path: HOME },
    ],
    n,
  );
  assert.deepEqual(rows.map((r) => r.label), ['Billing WS', 'app', 'app', 'Home']);
  assert.deepEqual(rows.map((r) => r.hint), [undefined, '~/a', '~/b', undefined]);
});

test('same checkout: live writable threads in the folder, not read-only, dormant or closed ones', () => {
  const rows = [
    { id: 'a', cwd: '/r/app', status: 'working' },
    { id: 'b', cwd: '/r/app/src', status: 'idle' },
    { id: 'c', cwd: '/r/app', status: 'closed' },
    { id: 'd', cwd: '/r/app', status: 'dormant' },
    { id: 'e', cwd: '/r/app', status: 'working', meta: { read_only: true } },
    { id: 'f', cwd: '/r/app-wt', status: 'working' },
    { id: 'g', cwd: '/private/r/app', status: 'awaiting_permission' },
  ];
  assert.deepEqual(threadsInCheckout(rows, '/r/app').map((r) => r.id), ['a', 'b']);
  assert.deepEqual(threadsInCheckout(rows, '/r/app', 'a').map((r) => r.id), ['b']);
  assert.deepEqual(threadsInCheckout(rows, ''), []);
});

test('access: the saved default is shown, Full access in home / outside git gets one caution', () => {
  assert.equal(stageAccess(null, true), 'full-access');
  assert.equal(stageAccess(null, false), 'approval-required');
  assert.equal(stageAccess(null, null), 'approval-required');
  assert.equal(stageAccess('auto-accept-edits', true), 'auto-accept-edits');
  assert.match(accessCaution('full-access', { home: true, repo: false }) ?? '', /home folder/);
  assert.match(accessCaution('full-access', { home: false, repo: false }) ?? '', /outside a git repository/);
  assert.equal(accessCaution('full-access', { home: false, repo: true }), null);
  assert.equal(accessCaution('full-access', { home: false, repo: null }), null);
  assert.equal(accessCaution('approval-required', { home: true, repo: false }), null);
});

test('closing a parent: its open children (and theirs) are offered, closed ones are not', () => {
  const kid = (id: string, parent: string, status = 'working') => ({ id, status, meta: { child_of: { id: parent, by: 'agent' } } });
  const rows = [{ id: 'p', status: 'idle' }, kid('a', 'p'), kid('b', 'p', 'closed'), kid('c', 'a'), kid('x', 'other')];
  assert.deepEqual(openDescendants(rows, 'p').map((r) => r.id), ['a', 'c']);
  assert.deepEqual(openDescendants(rows, 'x'), []);
  const d = closeWithChildrenDialog('Fix the build', 2);
  assert.match(d.message, /2 threads opened from it keep running\./);
  assert.deepEqual(d.options.map((o) => [o.label, o.value, o.kind]), [['Close only this', 'this', 'normal'], ['Close all 3', 'all', 'danger']]);
  assert.match(closeWithChildrenDialog('X', 1).message, /1 thread opened from it keeps running\./);
});

const noop = () => {};
const acts = { newThread: noop, related: noop, toggleCard: noop, layout: noop, toggleTerminal: noop };
const labels = (items: { label?: string; separator?: boolean }[]) => items.map((i) => (i.separator ? '—' : i.label));
const runtimeItems = [
  { label: 'Pin thread' }, { label: 'Open beside' }, { label: 'Open in IDE' }, { label: 'Rename…' },
  { label: 'Terminal drawer', checked: false }, { label: 'Allow UI control', checked: false }, { label: 'Continue on phone…' },
  { separator: true }, { label: 'Move to History' }, { label: 'Close thread…' }, { label: 'Delete thread…' },
];

test('toolbar ⋯ on desktop: at most 9 entries, one terminal, layout in View', () => {
  const items = threadsPageMenu({ threadItems: runtimeItems, layout: 'single', terminalOpen: false, cardOn: false, threadId: 'x', sidebarVisible: true }, acts);
  const rows = items.filter((i) => !i.separator);
  assert.ok(rows.length <= 9, labels(items).join(', '));
  assert.equal(rows.filter((i) => /terminal/i.test(i.label ?? '')).length, 1);
  assert.deepEqual(labels(items), [
    'Open in IDE', 'Rename…', 'Terminal drawer', 'Allow UI control', 'Continue on phone…', '—', 'Close thread…', 'Delete thread…', '—', RELATED_LABEL, 'View',
  ]);
  const view = items.find((i) => i.label === 'View');
  assert.deepEqual(labels(view?.children ?? []), ['Single thread', 'Split', 'Grid', '—', 'Project card']);
  // The page's drawer open: that one (checked) replaces the thread's.
  const open = threadsPageMenu({ threadItems: runtimeItems, layout: 'single', terminalOpen: true, cardOn: false, threadId: 'x', sidebarVisible: true }, acts);
  const term = open.filter((i) => /terminal/i.test(i.label ?? ''));
  assert.equal(term.length, 1);
  assert.equal(term[0].label, TERMINAL_LABEL);
  assert.equal(term[0].checked, true);
  // Sidebar hidden: New thread comes back (its pencil is gone).
  const hidden = threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: true, threadId: null, sidebarVisible: false }, acts);
  assert.deepEqual(labels(hidden), ['New thread', TERMINAL_LABEL, 'View']);
});

test('toolbar ⋯ on a phone: no Split / Grid, no hand-off to the phone, New thread offered', () => {
  const items = threadsPageMenu({ threadItems: runtimeItems, layout: 'single', terminalOpen: false, cardOn: null, threadId: 'x', compact: true, phone: true }, acts);
  const all = labels(items);
  for (const gone of ['View', 'Split', 'Grid', 'Single thread', 'Continue on phone…', 'Open beside', 'Pin thread']) assert.ok(!all.includes(gone), gone);
  assert.ok(all.includes('New thread'));
  assert.equal(all.filter((l) => /terminal/i.test(l ?? '')).length, 1);
  // An iPad (compact, not a phone) can still hand a thread to the phone.
  const ipad = threadsPageMenu({ threadItems: runtimeItems, layout: 'single', terminalOpen: false, cardOn: null, threadId: 'x', compact: true }, acts);
  assert.ok(labels(ipad).includes('Continue on phone…'));
  assert.ok(!labels(ipad).includes('View'));
});

test('right panel at 1280 gives way to keep the conversation, and the saved width returns', () => {
  // 1236 px stage (1280 minus the rail), 272 sidebar: 380 would leave 576.
  assert.equal(fitRightPanel(380, 1236, 272), 316);
  assert.equal(fitRightPanel(380, 1236, 0), 380);
  assert.equal(fitRightPanel(380, 1000, 272), 300);
  assert.equal(fitRightPanel(420, 1680, 272), 420);
});
