// The story Chat's cards: a proposed story body is NEVER a card any more (the
// merged Chat saves it as a suggested version and links to Rewrite; the card
// filter is covered in productChatMerged.test.ts), so the card has no
// "Replace draft" path; the cards it renders apply through the Chat
// conversation (`…/refinement-threads/{tid}/apply`) and send only the items
// the person kept checked.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

const url = new URL('../src/modules/product/ActionCard.svelte', import.meta.url);

function card(action: Record<string, unknown>, unchecked: number[] = []) {
  const calls: string[] = [];
  const applied: unknown[] = [];
  const view = componentFunctions(url, ['isChecked', 'applyQuestions', 'applyNotes'], {
    tid: 'thread-1',
    action,
    applying: false, applied: false, appliedLabel: '',
    createdQuestionIds: [], createdNoteIds: [],
    unchecked: new Set(unchecked),
    plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`,
    applyChatAction: async (tid: string, a: unknown) => {
      calls.push(`apply:${tid}`);
      applied.push(JSON.parse(JSON.stringify(a)));
      return { created_question_ids: ['q1'], created_note_ids: ['n1'] };
    },
    product: {
      loadQuestions: async () => { calls.push('reloadQuestions'); },
      loadNotes: async () => { calls.push('reloadNotes'); },
    },
    toasts: { success: (t: string) => calls.push(`success:${t}`) },
    toastError: () => calls.push('toastError'),
  });
  return { view, calls, applied };
}

test('questions apply through the conversation and only the checked ones are sent', async () => {
  const { view, calls, applied } = card(
    { type: 'add_questions', questions: [{ text: 'SLA?' }, { text: 'Who can unlock?' }] },
    [0],
  );
  await view.applyQuestions();
  assert.deepEqual(calls, ['apply:thread-1', 'reloadQuestions', 'success:Added 1 question']);
  assert.deepEqual(applied, [{ type: 'add_questions', questions: [{ text: 'Who can unlock?' }] }]);
  assert.equal(view.applied, true);
  assert.deepEqual(JSON.parse(JSON.stringify(view.createdQuestionIds)), ['q1']);
});

test('nothing checked → nothing applied', async () => {
  const { view, calls } = card({ type: 'add_notes', notes: [{ body: 'x' }] }, [0]);
  await view.applyNotes();
  assert.deepEqual(calls, []);
  assert.equal(view.applied, false);
});

test('a created board opens in Design Hall (its mirror), not the retired Canvas page', async () => {
  const calls: string[] = [];
  const view = componentFunctions(url, ['applyCanvas'], {
    tid: 'thread-1',
    action: { type: 'create_canvas', title: 'Flow', mermaid: 'flowchart TD' },
    applying: false, applied: false, appliedLabel: '',
    applyChatAction: async () => ({ canvas_id: 'sc-1' }),
    openSceneInHall: async (id: string) => { calls.push(`hall:${id}`); },
    toasts: { success: (t: string) => calls.push(`success:${t}`) },
    toastError: () => calls.push('toastError'),
  });
  await view.applyCanvas();
  assert.deepEqual(calls, ['hall:sc-1', 'success:Opened in Design Hall']);
  assert.equal(view.applied, true);
});
