// "Continue with another provider" (providerSwitch.ts): the picker's list,
// the armed line, what a send adds, the divider's words and the row change.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  armedSwitchLine,
  handoffSummary,
  handoffTitle,
  switchPromptFields,
  switchTargets,
} from '../src/modules/threads/runtime/providerSwitch.ts';
import { acknowledgeDraft, emptyDraft } from '../src/lib/stores/runtimeDraftPersistence.ts';

const h = { from_provider: 'claude', to_provider: 'codex', turns: 3, omitted_turns: 0, images_omitted: 0, text: 'x' };

test('the picker lists the thread’s provider first, then every other switchable one', () => {
  const all = ['codex', 'claude', 'gemini', 'shell', 'pi'];
  assert.deepEqual(switchTargets('claude', all, (p) => p !== 'shell' && p !== 'pi'), ['claude', 'codex', 'gemini']);
  // A thread on a provider no longer listed keeps it at the top.
  assert.deepEqual(switchTargets('claudework', ['claude'], () => true), ['claudework', 'claude']);
});

test('arming says where the next message goes; sending carries provider, model and effort', () => {
  assert.equal(armedSwitchLine('codex'), 'Next message continues on Codex');
  assert.deepEqual(switchPromptFields({ provider: 'codex', model: 'gpt-5.5', effort: 'high' }, 'claude'), { provider: 'codex', model: 'gpt-5.5', effort: 'high' });
  // "Default model" on the new provider sends no model.
  assert.deepEqual(switchPromptFields({ provider: 'codex', model: null, effort: null }, 'claude'), { provider: 'codex' });
  // Not armed, or armed on the thread's own provider: an ordinary send.
  assert.deepEqual(switchPromptFields({ provider: null, model: 'opus', effort: null }, 'claude'), {});
  assert.deepEqual(switchPromptFields({ provider: 'claude', model: 'opus', effort: null }, 'claude'), {});
});

test('an accepted send disarms only the switch it carried', () => {
  const sent = { ...emptyDraft(), text: 'hi', provider: 'codex' };
  const current = { ...emptyDraft(), text: 'hi', provider: 'codex' };
  acknowledgeDraft(current, sent);
  assert.equal(current.provider, null);
  // Re-armed onto another provider while the send ran: that one stays.
  const later = { ...emptyDraft(), provider: 'gemini' };
  acknowledgeDraft(later, sent);
  assert.equal(later.provider, 'gemini');
});

test('the divider names both providers and what was handed over, in words', () => {
  assert.equal(handoffTitle(h), 'Context handoff · Claude Code → Codex');
  assert.equal(handoffSummary(h), '3 turns handed over.');
  assert.equal(handoffSummary({ ...h, turns: 1, omitted_turns: 4, images_omitted: 2 }), '1 turn handed over · 4 earlier turns left out (still readable above) · 2 images not handed over.');
  assert.equal(handoffSummary({ ...h, turns: 0 }), 'Nothing to hand over yet: the thread had no messages.');
});
