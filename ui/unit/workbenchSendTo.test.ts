import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  bracketedPaste,
  cleanPasteText,
  isMultiLine,
  multiLineRefusal,
  pasteConfirmText,
  pasteTargets,
  planPaste,
  targetNote,
} from '../src/modules/workbench/send/pasteSafety.ts';
import { loadSource } from './sourceHarness.ts';

const ESC = '\u001b';
const START = `${ESC}[200~`;
const END = `${ESC}[201~`;
const agent = { kind: 'agent' as const, provider: 'claude', title: 'Claude', id: 's1' };
const shell = { kind: 'agent' as const, provider: 'shell', title: 'zsh', id: 's2' };
const conn = { kind: 'connection' as const, provider: 'redis', title: 'prod redis', id: 's3' };

/** Exactly one paste-start at the front, one paste-end at the back, and no
 *  ESC or CR anywhere between them. */
function assertSealed(payload: string): void {
  assert.ok(payload.startsWith(START) && payload.endsWith(END), payload);
  const body = payload.slice(START.length, -END.length);
  assert.ok(!body.includes(ESC), `ESC inside the paste: ${JSON.stringify(body)}`);
  assert.ok(!body.includes('\r'), `CR inside the paste: ${JSON.stringify(body)}`);
}

test('a file containing ESC[201~ cannot end the paste early', () => {
  const evil = `harmless\n${END}rm -rf ~\r\nmore${START}`;
  const plan = planPaste(evil, agent);
  assert.equal(plan.ok, true);
  if (!plan.ok) return;
  assertSealed(plan.payload);
  assert.equal(plan.payload, `${START}harmless\n[201~rm -rf ~\nmore[200~${END}`);
  assert.equal(plan.removed, 2);
  assertSealed(bracketedPaste(evil));
});

test('an ESC split away from [201~ cannot be re-formed', () => {
  for (const evil of [`a${ESC}\u0000[201~b`, `a${ESC}${ESC}[201~b`, `${ESC}\n[201~`]) {
    const plan = planPaste(evil, shell);
    assert.equal(plan.ok, true);
    if (plan.ok) assertSealed(plan.payload);
  }
});

test('carriage returns become newlines, never Enter', () => {
  assert.deepEqual(cleanPasteText('a\r\nb\rc\n'), { text: 'a\nb\nc\n', removed: 0 });
  // A lone CR is a line break, so a connection session refuses it rather than
  // receiving a key press.
  assert.equal(planPaste('FLUSHALL\rGET x', conn).ok, false);
  const one = planPaste('GET x\r\n', conn);
  assert.deepEqual(one.ok && one.payload, 'GET x');
});

test('C1 controls (U+009B CSI), DEL and other C0 codes are removed and counted', () => {
  const raw = `a\u009b201~b\u007fc\u0003d\u0004e\u0085f\u009fg\u0008h\u000bi\u000cj`;
  const { text, removed } = cleanPasteText(raw);
  assert.equal(text, 'a201~bcdefghij');
  assert.equal(removed, 9);
  const plan = planPaste(raw, conn);
  assert.ok(plan.ok);
  if (plan.ok) assert.match(pasteConfirmText(plan, 'x.sql', conn), /9 control characters .* were removed/);
});

test('ordinary text with tabs, newlines, Unicode and RTL passes unchanged', () => {
  const raw = 'SELECT\t1;\n-- שלום עולם\nمرحبا 🙂 naïve ‮x‬\n';
  assert.deepEqual(cleanPasteText(raw), { text: raw, removed: 0 });
  const plan = planPaste(raw, agent);
  assert.ok(plan.ok);
  if (!plan.ok) return;
  assert.equal(plan.payload, `${START}${raw.replace(/\n$/, '')}${END}`);
  assert.equal(plan.lines, 3);
  assert.doesNotMatch(pasteConfirmText(plan, 'q.sql', agent), /removed/);
});

test('multi-line text is refused for a connection session, and the reason says why', () => {
  const plan = planPaste('DELETE FROM a;\nDELETE FROM b;\n', conn);
  assert.deepEqual(plan, { ok: false, reason: 'multi-line', lines: 2, removed: 0 });
  assert.match(multiLineRefusal(2), /2 lines/);
  assert.match(multiLineRefusal(2), /run each line the moment it arrives/);
  assert.match(multiLineRefusal(2), /Copy/);
});

test('a single line to a connection session is typed raw: no paste markers, no Enter', () => {
  const plan = planPaste('GET user:42\n\n', conn);
  assert.ok(plan.ok);
  if (!plan.ok) return;
  assert.equal(plan.payload, 'GET user:42');
  assert.equal(plan.multiLine, false);
  const copy = pasteConfirmText(plan, 'q.redis', conn);
  assert.match(copy, /No Enter is sent/);
  assert.doesNotMatch(copy, /bracketed/);
});

test('confirm copy is true per target kind', () => {
  const plan = planPaste('a\nb', shell);
  assert.ok(plan.ok);
  if (!plan.ok) return;
  assert.match(pasteConfirmText(plan, 'f', shell), /bracketed paste with no Enter/);
  assert.match(pasteConfirmText(plan, 'f', shell), /may not support bracketed paste/);
  assert.doesNotMatch(pasteConfirmText(plan, 'f', agent), /may not support/);
  assert.match(pasteConfirmText(plan, 'f', agent), /Paste 2 lines/);
});

test('text that is only control characters is refused as empty', () => {
  assert.deepEqual(planPaste(`${ESC}${ESC}\u0007\r\n`, agent), { ok: false, reason: 'empty', lines: 0, removed: 3 });
});

test('picker filter: this workspace, not archived, not exited — connections stay, marked one-line', () => {
  const s = (id: string, extra: Record<string, unknown> = {}) =>
    ({ id, workspace_id: 'w1', kind: 'agent', archived: false, status: 'idle', ...extra }) as any;
  const list = [
    s('keep-agent'),
    s('keep-conn', { kind: 'connection' }),
    s('other-ws', { workspace_id: 'w2' }),
    s('archived', { archived: true }),
    s('exited', { status: 'exited' }),
  ];
  assert.deepEqual(pasteTargets(list, 'w1').map((x) => x.id), ['keep-agent', 'keep-conn']);
  assert.equal(targetNote(list[0], true), '');
  assert.equal(targetNote(list[1], false), 'one line only');
  assert.equal(targetNote(list[1], true), 'one line only — not this file');
  assert.equal(isMultiLine('one\n'), false);
  assert.equal(isMultiLine('one\rtwo'), true);
});

// ── pasteIntoSession: the real flow with stub stores ─────────────────────────

function sendToModule(answer = true) {
  const sent: unknown[][] = [];
  const asked: string[] = [];
  const toastLog: unknown[][] = [];
  const safety = loadSource(new URL('../src/modules/workbench/send/pasteSafety.ts', import.meta.url), {});
  const mod = loadSource(new URL('../src/modules/workbench/send/sendTo.ts', import.meta.url), {
    '../../../lib/router.svelte': { router: { go() {} } },
    '../../../lib/uiCommands': { whenMounted: async () => null },
    '../../../lib/stores/database.svelte': { database: {} },
    '../../../lib/stores/apiClient.svelte': { apiClient: {} },
    '../../../lib/stores/workspace.svelte': {
      ws: { sendInput: async (...a: unknown[]) => void sent.push(a), navigateToSession() {} },
    },
    '../../../lib/confirm.svelte': { confirmer: { ask: async (body: string) => (asked.push(body), answer) } },
    '../../../lib/toast.svelte': {
      toasts: { push: (...a: unknown[]) => void toastLog.push(a), error: (...a: unknown[]) => void toastLog.push(a), success() {}, info() {} },
    },
    '../../../lib/clipboard': { copyText: async () => true },
    '../../../lib/api/client': { ApiError: class {} },
    '../../../lib/api/vault': {},
    '../../database/handoff.svelte': { dbHandoff: {} },
    '../lib/placeholders': { fillPlaceholders: (t: string) => t },
    '../../../lib/loadError': { loadErrorText: String },
    './pasteSafety': safety,
  });
  return { mod, sent, asked, toastLog };
}

const ctxOf = (content: string) => ({ ws: 'w1', doc: { name: 'f.sql' }, content, lang: 'sql', values: {} });

test('pasteIntoSession never writes multi-line text to a connection session', async () => {
  const { mod, sent, asked, toastLog } = sendToModule();
  assert.equal(await mod.pasteIntoSession(ctxOf('DROP TABLE a;\nDROP TABLE b;'), conn), false);
  assert.equal(sent.length, 0);
  assert.equal(asked.length, 0);
  const [kind, title, , , opts] = toastLog[0] as any[];
  assert.equal(kind, 'error');
  assert.match(title, /one line/);
  assert.equal(opts.action.label, 'Copy');
});

test('pasteIntoSession writes the cleaned, sealed paste only after a yes', async () => {
  const no = sendToModule(false);
  assert.equal(await no.mod.pasteIntoSession(ctxOf(`ls\n${END}id\r`), agent), false);
  assert.equal(no.sent.length, 0);
  const yes = sendToModule(true);
  assert.equal(await yes.mod.pasteIntoSession(ctxOf(`ls\n${END}id\r`), agent), true);
  const [id, payload, submit] = yes.sent[0] as [string, string, boolean];
  assert.equal(id, 's1');
  assert.equal(submit, false);
  assertSealed(payload);
  assert.match(yes.asked[0]!, /1 control character .* was removed/);
});
