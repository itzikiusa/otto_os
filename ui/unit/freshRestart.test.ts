// I-01/I-02: a restart Otto makes as a side effect of a settings change asks
// first when it would start a NEW conversation (Muse, opencode, a session
// whose provider id was never captured); unknown (older daemon) keeps today's
// behaviour.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  confirmFreshRestart,
  FRESH_RESTART_TITLE,
  respawnChoice,
  restartResumes,
  saveAndRestartChoice,
  type RestartDialog,
} from '../src/lib/freshRestart.ts';

const meta = { provider_resumable: { claude: true, codex: true, muse: false, opencode: false } };
const agent = (provider: string, psid: string | null = 'p1') => ({
  kind: 'agent' as const,
  provider,
  provider_session_id: psid,
  archived: false,
  status: 'idle' as const,
});

function fakeDialog(answer: { ask?: boolean; choose?: string | null }) {
  const calls: { kind: string; message: string; opts: any }[] = [];
  const dialog: RestartDialog = {
    ask: async (message, opts) => {
      calls.push({ kind: 'ask', message, opts });
      return answer.ask ?? false;
    },
    choose: async (message, opts) => {
      calls.push({ kind: 'choose', message, opts });
      return { value: answer.choose ?? null, remember: false };
    },
  };
  return { dialog, calls };
}

test('resumability comes from /meta and the captured provider id', () => {
  assert.equal(restartResumes(agent('claude'), meta), true);
  assert.equal(restartResumes(agent('muse'), meta), false);
  assert.equal(restartResumes(agent('opencode'), meta), false);
  assert.equal(restartResumes(agent('codex', null), meta), false, 'codex id never captured');
  assert.equal(restartResumes(agent('claude'), null), null, 'older daemon: unknown');
  assert.equal(restartResumes(agent('claude'), {}), null, 'older daemon: unknown');
  assert.equal(restartResumes(agent('custom'), meta), null, 'unlisted provider: unknown');
  assert.equal(restartResumes({ ...agent('shell'), provider: 'shell' }, meta), null);
});

test('a resumable or unknown session restarts without asking', async () => {
  for (const [s, m] of [[agent('claude'), meta], [agent('muse'), null]] as const) {
    const { dialog, calls } = fakeDialog({});
    assert.equal(await confirmFreshRestart(s, dialog, { canSaveOnly: true, meta: m }), 'restart');
    assert.equal(calls.length, 0);
  }
});

test('a non-resumable session asks: restart, save without restarting, or cancel', async () => {
  for (const pick of ['restart', 'save', null] as const) {
    const { dialog, calls } = fakeDialog({ choose: pick });
    const got = await confirmFreshRestart(agent('muse'), dialog, { canSaveOnly: true, meta });
    assert.equal(got, pick ?? 'cancel');
    assert.equal(calls[0].kind, 'choose');
    assert.equal(calls[0].opts.title, FRESH_RESTART_TITLE);
    assert.match(calls[0].message, /can’t resume this session’s current conversation/);
    assert.match(calls[0].message, /applies to the next session/);
    assert.deepEqual(
      calls[0].opts.options.map((o: { label: string }) => o.label),
      ['Restart', 'Save without restarting'],
    );
  }
  const { dialog, calls } = fakeDialog({ ask: true });
  assert.equal(await confirmFreshRestart(agent('opencode'), dialog, { meta }), 'restart');
  assert.equal(calls[0].kind, 'ask');
  assert.equal(calls[0].opts.confirmLabel, 'Restart');
});

test('the settings-change gate asks only when a respawn would happen', async () => {
  const { dialog, calls } = fakeDialog({ choose: 'save' });
  // Reads /meta from the registered source; none registered here → unknown.
  assert.equal(await respawnChoice(agent('muse'), 'idle', true, dialog), 'restart');
  assert.equal(await respawnChoice(agent('muse'), 'working', true, dialog), 'restart');
  assert.equal(await respawnChoice(agent('muse'), 'idle', false, dialog), 'restart');
  assert.equal(await respawnChoice(null, 'idle', true, dialog), 'restart');
  assert.equal(calls.length, 0);
});

test('"Save & restart": working-guard first, then the can’t-resume ask', async () => {
  const no = fakeDialog({});
  assert.equal(await saveAndRestartChoice(agent('muse'), async () => false, no.dialog), null);
  assert.equal(no.calls.length, 0, 'a refused working-guard asks nothing else');
  const { registerMetaSource } = await import('../src/lib/freshRestart.ts');
  registerMetaSource(() => meta);
  try {
    const save = fakeDialog({ choose: 'save' });
    assert.equal(await saveAndRestartChoice(agent('muse'), async () => true, save.dialog), false);
    const cancel = fakeDialog({ choose: null });
    assert.equal(await saveAndRestartChoice(agent('muse'), async () => true, cancel.dialog), null);
    const keep = fakeDialog({});
    assert.equal(await saveAndRestartChoice(agent('claude'), async () => true, keep.dialog), true);
    assert.equal(keep.calls.length, 0);
    const gate = fakeDialog({ choose: 'save' });
    assert.equal(await respawnChoice(agent('muse'), 'running', true, gate.dialog), 'save');
  } finally {
    registerMetaSource(() => null);
  }
});
