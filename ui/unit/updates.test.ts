// The desktop updater's UI state machine (lib/stores/updates.svelte.ts):
// the sidebar row, the Update dialog's one primary button, the restart step
// and the store's calls into the shell — with `invoke` and the daemon mocked.
import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import type * as Mod from '../src/lib/stores/updates.svelte.ts';

type Call = { cmd: string; args?: Record<string, unknown> };
function load(opts: { invoke?: (cmd: string, args?: Record<string, unknown>) => unknown; impact?: () => unknown; confirm?: boolean } = {}) {
  const calls: Call[] = [];
  const toasts: string[] = [];
  const asks: { msg: string; opts: Record<string, unknown> }[] = [];
  let listener: ((e: { payload: unknown }) => void) | null = null;
  const mod = loadSource(new URL('../src/lib/stores/updates.svelte.ts', import.meta.url), {
    '../desktop': { isTauri: true },
    '../api/client': { api: { get: async (p: string) => { calls.push({ cmd: `GET ${p}` }); return opts.impact ? opts.impact() : { keeps: [], ends: [] }; } } },
    '../confirm.svelte': { confirmer: { ask: async (msg: string, o: Record<string, unknown>) => { asks.push({ msg, opts: o }); return opts.confirm ?? true; } } },
    '../toast.svelte': { toasts: { success: (t: string) => toasts.push(`ok:${t}`), error: (t: string) => toasts.push(`err:${t}`) } },
    '../loadError': { loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)) },
    '@tauri-apps/api/core': { invoke: async (cmd: string, args?: Record<string, unknown>) => { calls.push({ cmd, args }); return opts.invoke?.(cmd, args); } },
    '@tauri-apps/api/event': { listen: async (_: string, cb: (e: { payload: unknown }) => void) => { listener = cb; return () => {}; } },
  }) as typeof Mod;
  return { mod, calls, toasts, asks, emit: (p: unknown) => listener?.({ payload: p }) };
}

const base = { current: '0.5.3', channel: 'release' as const };
const st = (p: Partial<Mod.UpdateState>): Mod.UpdateState => ({ phase: 'idle', ...base, ...p });
const prefs: Mod.UpdatePrefs = { auto_install: false, skipped: null };
const flush = () => new Promise((r) => setTimeout(r, 0));

test('the sidebar row walks available → downloading → staged, and is silent otherwise', () => {
  const { mod } = load();
  assert.equal(mod.rowLabel(st({ phase: 'available', version: '0.5.4' }), prefs, null), 'Update to 0.5.4');
  assert.equal(mod.rowLabel(st({ phase: 'downloading', version: '0.5.4', pct: 42.7 }), prefs, null), 'Downloading… 42 %');
  assert.equal(mod.rowLabel(st({ phase: 'downloading', version: '0.5.4' }), prefs, null), 'Downloading…');
  assert.equal(mod.rowLabel(st({ phase: 'staged', version: '0.5.4' }), prefs, null), 'Restart to update');
  // Silent: nothing known, up to date, checking, a failed automatic check, a source build, a skipped version.
  assert.equal(mod.rowLabel(null, prefs, null), null);
  assert.equal(mod.rowLabel(st({ phase: 'idle', last_check: '2026-11-25T10:00:00Z' }), prefs, null), null);
  assert.equal(mod.rowLabel(st({ phase: 'checking' }), prefs, null), null);
  assert.equal(mod.rowLabel(st({ phase: 'error', error: 'offline' }), prefs, null), null);
  assert.equal(mod.rowLabel(st({ phase: 'available', version: '0.5.4', channel: 'source' }), prefs, null), null);
  assert.equal(mod.rowLabel(st({ phase: 'available', version: '0.5.4' }), { ...prefs, skipped: '0.5.4' }, null), null);
  // A newer version than the skipped one shows again.
  assert.equal(mod.rowLabel(st({ phase: 'available', version: '0.5.5' }), { ...prefs, skipped: '0.5.4' }, null), 'Update to 0.5.5');
  // A rolled-back update wins over everything.
  assert.equal(mod.rowLabel(null, prefs, { ok: false, from: '0.5.3', to: '0.5.4' }), 'Update failed and was undone');
});

test('the dialog has one primary that follows the state', () => {
  const { mod } = load();
  const v = (s: Mod.UpdateState | null, failed: Mod.UpdateResult | null = null, err: string | null = null) => {
    const r = mod.dialogView(s, failed, err);
    return `${r.kind}:${r.primary.label}:${r.primary.action}`;
  };
  assert.equal(v(null), 'checking:Checking…:null');
  assert.equal(v(st({ phase: 'checking' })), 'checking:Checking…:null');
  assert.equal(v(st({ phase: 'idle', last_check: 'x' })), 'uptodate:Close:close');
  assert.equal(v(st({ phase: 'available', version: '0.5.4' })), 'available:Download and install:download');
  assert.equal(v(st({ phase: 'downloading', version: '0.5.4', pct: 9 })), 'downloading:Downloading… 9 %:null');
  assert.equal(v(st({ phase: 'staged', version: '0.5.4' })), 'staged:Restart to finish:restart');
  assert.equal(v(st({ phase: 'error', version: '0.5.4', error: 'disk' })), 'error:Try again:download');
  assert.equal(v(st({ phase: 'error', error: 'offline' })), 'error:Close:close');
  assert.equal(v(st({ phase: 'idle' }), null, 'offline'), 'error:Close:close');
  assert.equal(v(st({ phase: 'source', channel: 'source' })), 'source:Close:close');
  assert.equal(v(st({ phase: 'available', version: '0.5.4', channel: 'source' })), 'source:Close:close');
  assert.equal(v(st({}), { ok: false, from: '0.5.3', to: '0.5.4' }), 'failed:Close:close');
  // A manual check that failed before the first state arrived is an error, not "checking" forever.
  assert.equal(v(null, null, 'offline'), 'error:Close:close');
});

test('the restart step lists what keeps running and what ends; confirm only when something ends', () => {
  const { mod } = load();
  const quiet = mod.restartPlan({ keeps: [{ label: 'sessions', count: 4 }], ends: [{ label: 'connection terminals', count: 0 }] });
  assert.equal(JSON.stringify(quiet), JSON.stringify({ keeps: ['4 sessions'], ends: [], endCount: 0, label: 'Restart now', confirm: false }));
  const loud = mod.restartPlan({
    keeps: [{ label: 'sessions', count: 4 }],
    ends: [{ label: 'connection terminals', count: 2, detail: ['prod-db', 'k8s exec'] }, { label: 'debug session', count: 1 }],
  });
  assert.deepEqual([...loud.ends], ['2 connection terminals (prod-db, k8s exec)', '1 debug session']);
  assert.equal(loud.label, 'Restart and end 3');
  assert.equal(loud.confirm, true);
  // An unknown impact never restarts without asking.
  assert.equal(mod.restartPlan(null).confirm, true);
});

test('init reads the shell once, follows its event, and toasts a successful update once', async () => {
  const { mod, calls, toasts, emit } = load({
    invoke: (cmd) => (cmd === 'update_state' ? st({ last_result: { ok: true, from: '0.5.3', to: '0.5.4' } }) : cmd === 'update_prefs_get' ? { auto_install: true, skipped: '0.5.2' } : null),
  });
  mod.updates.init(true);
  mod.updates.init(true);
  await flush();
  assert.deepEqual(calls.map((c) => c.cmd), ['update_state', 'update_prefs_get']);
  assert.deepEqual(toasts, ['ok:Updated to 0.5.4']);
  assert.deepEqual(mod.updates.prefs, { auto_install: true, skipped: '0.5.2' });
  emit(st({ phase: 'downloading', version: '0.5.5', pct: 50 }));
  assert.equal(mod.updates.row, 'Downloading… 50 %');
});

test('a rolled-back update shows the inline notice (not a toast) and only in the main window', async () => {
  const failed = st({ last_result: { ok: false, from: '0.5.3', to: '0.5.4', reason: 'the daemon did not answer' } });
  const main = load({ invoke: (cmd) => (cmd === 'update_state' ? failed : null) });
  main.mod.updates.init(true);
  await flush();
  assert.equal(main.mod.updates.row, 'Update failed and was undone');
  assert.deepEqual(main.toasts, []);
  main.mod.updates.dismissFailed();
  assert.equal(main.mod.updates.row, null);
  const popout = load({ invoke: (cmd) => (cmd === 'update_state' ? failed : null) });
  popout.mod.updates.init(false);
  await flush();
  assert.equal(popout.mod.updates.failed, null);
});

test('a manual check that fails shows the cause; a later success clears it', async () => {
  let fail = true;
  const { mod } = load({ invoke: (cmd) => { if (cmd === 'update_check') { if (fail) throw new Error('You are offline'); return st({ last_check: 'now' }); } return null; } });
  mod.updates.openDialog(true);
  await flush();
  assert.equal(mod.updates.open, true);
  assert.equal(mod.updates.checkError, 'You are offline');
  assert.equal(mod.dialogView(mod.updates.state, null, mod.updates.checkError).kind, 'error');
  fail = false;
  await mod.updates.check();
  assert.equal(mod.updates.checkError, null);
  assert.equal(mod.dialogView(mod.updates.state, null, null).kind, 'uptodate');
});

test('restart: nothing ends → one click; something ends → one attributed confirm; declining restarts nothing', async () => {
  const quiet = load({ impact: () => ({ keeps: [{ label: 'sessions', count: 2 }], ends: [] }) });
  quiet.mod.updates.apply(st({ phase: 'staged', version: '0.5.4' }));
  await quiet.mod.updates.loadImpact();
  await quiet.mod.updates.restart();
  assert.equal(quiet.asks.length, 0);
  assert.ok(quiet.calls.some((c) => c.cmd === 'update_restart'));
  assert.equal(quiet.mod.updates.restarting, true);

  const loud = load({ confirm: false, impact: () => ({ keeps: [], ends: [{ label: 'connection terminals', count: 2 }] }) });
  loud.mod.updates.apply(st({ phase: 'staged', version: '0.5.4' }));
  await loud.mod.updates.loadImpact();
  await loud.mod.updates.restart();
  assert.equal(loud.asks.length, 1);
  assert.match(loud.asks[0].msg, /ends: 2 connection terminals/);
  assert.equal(loud.asks[0].opts.confirmLabel, 'Restart and end 2');
  assert.match(String(loud.asks[0].opts.title), /0\.5\.4/);
  assert.ok(!loud.calls.some((c) => c.cmd === 'update_restart'), 'declined: no restart');

  const unknown = load({ impact: () => { throw new Error('daemon down'); } });
  unknown.mod.updates.apply(st({ phase: 'staged', version: '0.5.4' }));
  await unknown.mod.updates.loadImpact();
  assert.equal(unknown.mod.updates.impactError, 'daemon down');
  await unknown.mod.updates.restart();
  assert.equal(unknown.asks.length, 1, 'an unknown impact still asks');
});

test('a failed restart says why and re-enables the button', async () => {
  const { mod } = load({ invoke: (cmd) => { if (cmd === 'update_restart') throw new Error('/Applications is not writable'); return null; } });
  mod.updates.apply(st({ phase: 'staged', version: '0.5.4' }));
  await mod.updates.loadImpact();
  await mod.updates.restart();
  assert.equal(mod.updates.restarting, false);
  assert.equal(mod.updates.actionError, '/Applications is not writable');
});

test('skip, unskip and automatic download save through the shell; a failed save reverts', async () => {
  let failSave = false;
  const { mod, calls, toasts } = load({ invoke: (cmd) => { if (cmd === 'update_prefs_set' && failSave) throw new Error('disk full'); return null; } });
  mod.updates.apply(st({ phase: 'available', version: '0.5.4' }));
  mod.updates.open = true;
  mod.updates.skip();
  await flush();
  assert.equal(mod.updates.open, false);
  assert.equal(mod.updates.prefs.skipped, '0.5.4');
  assert.equal(mod.updates.row, null);
  assert.equal(JSON.stringify(calls.at(-1)), JSON.stringify({ cmd: 'update_prefs_set', args: { prefs: { skipped: '0.5.4' } } }));
  await mod.updates.setPrefs({ skipped: null });
  assert.equal(mod.updates.row, 'Update to 0.5.4');
  failSave = true;
  await mod.updates.setPrefs({ auto_install: true });
  assert.equal(mod.updates.prefs.auto_install, false);
  assert.deepEqual(toasts, ['err:Couldn’t save the update setting']);
});
