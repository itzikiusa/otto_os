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

const st = (p: Partial<Mod.UpdateState>): Mod.UpdateState => ({ phase: 'idle', current: '0.5.3', channel: 'release', ...p });
const flush = () => new Promise((r) => setTimeout(r, 0));
const json = (v: unknown) => JSON.stringify(v);

test('the sidebar row walks available → downloading → staged, and is silent otherwise', () => {
  const { mod } = load();
  const row = (p: Partial<Mod.UpdateState>) => mod.rowLabel(st(p), null);
  assert.equal(row({ phase: 'available', version: '0.5.4' }), 'Update to 0.5.4');
  assert.equal(row({ phase: 'downloading', version: '0.5.4', size: 1000, received: 427 }), 'Downloading… 42 %');
  assert.equal(row({ phase: 'downloading', version: '0.5.4', received: 427 }), 'Downloading…', 'no size, no percentage');
  assert.equal(row({ phase: 'staged', version: '0.5.4' }), 'Restart to update');
  assert.equal(row({ phase: 'restarting', version: '0.5.4' }), 'Restart to update');
  // A failed download keeps offering the version.
  assert.equal(row({ phase: 'error', version: '0.5.4', error_kind: 'disk' }), 'Update to 0.5.4');
  // Silent: nothing known, up to date, checking, a failed automatic check, a source build, a skipped version.
  assert.equal(mod.rowLabel(null, null), null);
  assert.equal(row({ phase: 'up_to_date', last_check: 1 }), null);
  assert.equal(row({ phase: 'checking' }), null);
  assert.equal(row({ phase: 'error', error: 'offline', error_kind: 'offline' }), null);
  assert.equal(row({ phase: 'available', version: '0.5.4', channel: 'source' }), null);
  assert.equal(row({ phase: 'available', version: '0.5.4', skipped: '0.5.4' }), null);
  assert.equal(row({ phase: 'available', version: '0.5.5', skipped: '0.5.4' }), 'Update to 0.5.5', 'a newer version shows again');
  // A rolled-back update wins over everything.
  assert.equal(mod.rowLabel(null, { ok: false, from: '0.5.3', to: '0.5.4' }), 'Update failed and was undone');
});

test('the dialog has one primary that follows the state', () => {
  const { mod } = load();
  const v = (s: Mod.UpdateState | null, failed: Mod.UpdateResult | null = null, err: string | null = null) => {
    const r = mod.dialogView(s, failed, err);
    return `${r.kind}:${r.primary.label}:${r.primary.action}`;
  };
  assert.equal(v(null), 'checking:Checking…:null');
  assert.equal(v(st({ phase: 'checking' })), 'checking:Checking…:null');
  assert.equal(v(st({ phase: 'up_to_date', last_check: 1 })), 'uptodate:Close:close');
  assert.equal(v(st({ phase: 'idle' })), 'uptodate:Close:close');
  assert.equal(v(st({ phase: 'available', version: '0.5.4' })), 'available:Download and install:download');
  assert.equal(v(st({ phase: 'available', version: '0.5.4', blocked: 'Move Otto to the Applications folder…' })), 'available:Close:close');
  assert.equal(v(st({ phase: 'downloading', version: '0.5.4', size: 100, received: 9 })), 'downloading:Downloading… 9 %:null');
  assert.equal(v(st({ phase: 'staged', version: '0.5.4' })), 'staged:Restart to finish:restart');
  assert.equal(v(st({ phase: 'restarting', version: '0.5.4' })), 'restarting:Restarting…:null');
  assert.equal(v(st({ phase: 'error', version: '0.5.4', error: 'disk' })), 'error:Try again:download');
  assert.equal(v(st({ phase: 'error', version: '0.5.4', error: 'x', blocked: 'not writable' })), 'error:Close:close');
  assert.equal(v(st({ phase: 'error', error: 'offline' })), 'error:Close:close');
  assert.equal(v(st({ phase: 'idle' }), null, 'offline'), 'error:Close:close');
  assert.equal(v(null, null, 'offline'), 'error:Close:close', 'never "checking" forever');
  assert.equal(v(st({ channel: 'source' })), 'source:Close:close');
  assert.equal(v(st({ phase: 'available', version: '0.5.4', channel: 'source' })), 'source:Close:close');
  assert.equal(v(st({}), { ok: false, from: '0.5.3', to: '0.5.4' }), 'failed:Close:close');
});

test('the restart step lists what keeps running and what ends; confirm only when something ends', () => {
  const { mod } = load();
  const quiet = mod.restartPlan({ keeps: [{ label: 'sessions', count: 4 }], ends: [{ label: 'connection terminals', count: 0 }] });
  assert.equal(json(quiet), json({ keeps: ['4 sessions'], ends: [], endCount: 0, label: 'Restart now', confirm: false }));
  const loud = mod.restartPlan({
    keeps: [{ label: 'sessions', count: 4 }],
    ends: [{ label: 'connection terminals', count: 2, detail: ['prod-db', 'k8s exec'] }, { label: 'debug session', count: 1 }],
  });
  assert.deepEqual([...loud.ends], ['2 connection terminals (prod-db, k8s exec)', '1 debug session']);
  assert.equal(loud.label, 'Restart and end 3');
  assert.equal(loud.confirm, true);
  assert.equal(mod.restartPlan(null).confirm, true, 'an unknown impact never restarts without asking');
});

test('init reads the shell once, follows its event, toasts a good update once and marks it seen', async () => {
  const { mod, calls, toasts, emit } = load({ invoke: (cmd) => (cmd === 'update_state' ? st({ last_result: { ok: true, from: '0.5.3', to: '0.5.4' } }) : null) });
  mod.updates.init(true);
  mod.updates.init(true);
  await flush();
  assert.deepEqual(calls.map((c) => c.cmd), ['update_state', 'update_result_seen']);
  assert.deepEqual(toasts, ['ok:Updated to 0.5.4']);
  emit(st({ phase: 'downloading', version: '0.5.5', size: 200, received: 100 }));
  assert.equal(mod.updates.row, 'Downloading… 50 %');
});

test('a rolled-back update shows inline (not a toast) until dismissed, only in the main window; the log is revealed', async () => {
  const failed = st({ last_result: { ok: false, skip: true, from: '0.5.3', to: '0.5.4', reason: 'no answer', log: '/x/finish-1.log' } });
  const main = load({ invoke: (cmd) => (cmd === 'update_state' ? failed : null) });
  main.mod.updates.init(true);
  await flush();
  assert.equal(main.mod.updates.row, 'Update failed and was undone');
  assert.deepEqual(main.toasts, []);
  assert.ok(!main.calls.some((c) => c.cmd === 'update_result_seen'), 'kept until the person saw it');
  await main.mod.updates.showLog();
  assert.equal(json(main.calls.at(-1)), json({ cmd: 'plugin:opener|reveal_item_in_dir', args: { paths: ['/x/finish-1.log'] } }));
  main.mod.updates.dismissFailed();
  await flush();
  assert.equal(main.mod.updates.row, null);
  assert.equal(main.calls.at(-1)?.cmd, 'update_result_seen');
  const popout = load({ invoke: (cmd) => (cmd === 'update_state' ? failed : null) });
  popout.mod.updates.init(false);
  await flush();
  assert.equal(popout.mod.updates.failed, null);
});

test('a manual check that fails (offline) shows the cause; a later success clears it', async () => {
  let offline = true;
  const { mod } = load({
    invoke: (cmd) => (cmd === 'update_check' ? (offline ? st({ phase: 'error', error: 'You’re offline', error_kind: 'offline' }) : st({ phase: 'up_to_date', last_check: 2 })) : null),
  });
  mod.updates.openDialog(true);
  await flush();
  assert.equal(mod.updates.open, true);
  assert.equal(mod.updates.checkError, 'You’re offline');
  assert.equal(mod.dialogView(mod.updates.state, null, mod.updates.checkError).kind, 'error');
  offline = false;
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

test('skip, unskip and automatic download go through the shell and follow its answer', async () => {
  let fail = false;
  let shell = st({ phase: 'available', version: '0.5.4' });
  const { mod, calls, toasts } = load({
    invoke: (cmd, args) => {
      if (cmd === 'update_skip') shell = { ...shell, skipped: (args?.version as string | null) ?? null };
      if (cmd === 'update_set_auto') { if (fail) throw new Error('disk full'); shell = { ...shell, auto_install: !!args?.on }; }
      return shell;
    },
  });
  mod.updates.apply(shell);
  mod.updates.open = true;
  await mod.updates.skip('0.5.4');
  assert.equal(mod.updates.open, false);
  assert.equal(json(calls.at(-1)), json({ cmd: 'update_skip', args: { version: '0.5.4' } }));
  assert.equal(mod.updates.row, null);
  await mod.updates.skip(null);
  assert.equal(mod.updates.row, 'Update to 0.5.4');
  await mod.updates.setAuto(true);
  assert.equal(mod.updates.state?.auto_install, true);
  fail = true;
  await mod.updates.setAuto(false);
  assert.equal(mod.updates.state?.auto_install, true, 'a failed save changes nothing');
  assert.deepEqual(toasts, ['err:Couldn’t save the update setting']);
});
