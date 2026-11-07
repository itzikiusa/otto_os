import test from 'node:test';
import assert from 'node:assert/strict';
import { questionAnswerError, pendingQuestions } from '../src/modules/threads/runtime/questions.ts';
const question = { id: 'native-key', header: 'Strategy', prompt: 'Which strategy?', kind: 'single' as const, required: true, allow_free_text: false, options: [{ value: 'safe_v1', label: 'Safe' }] };
test('answers preserve native values and reject labels, wrong kinds and missing required fields', () => {
  assert.equal(questionAnswerError([question], [{ question_id: 'native-key', type: 'single', value: 'safe_v1' }]), null);
  assert.match(questionAnswerError([question], [{ question_id: 'native-key', type: 'single', value: 'Safe' }])!, /offered/);
  assert.match(questionAnswerError([question], [{ question_id: 'native-key', type: 'multi', values: ['safe_v1'] }])!, /one/);
  assert.match(questionAnswerError([question], [])!, /answer/i);
});
test('free text is explicit and duplicates are rejected', () => {
  const q = { ...question, allow_free_text: true };
  assert.equal(questionAnswerError([q], [{ question_id: q.id, type: 'text', text: 'Something else' }]), null);
  assert.match(questionAnswerError([q], [{ question_id: q.id, type: 'text', text: ' ' }])!, /answer/i);
  assert.match(questionAnswerError([q], [{ question_id: q.id, type: 'text', text: 'a' }, { question_id: q.id, type: 'text', text: 'b' }])!, /duplicate/i);
});
test('question reachability uses full requests, excludes approvals and expired requests', () => {
  const r = { id: 'r', kind: 'question', status: 'pending', response_capability: { type: 'live' }, questions: [question] };
  assert.deepEqual(pendingQuestions([r, { ...r, id: 'expired', status: 'expired' }, { ...r, id: 'approval', kind: 'command' }] as never), [r]);
});

test('multi answers retain values and optional text can be omitted', () => {
  const q = { ...question, kind: 'multi' as const, options: [...question.options, { value: 'fast_v2', label: 'Fast' }] };
  assert.equal(questionAnswerError([q], [{ question_id: q.id, type: 'multi', values: ['fast_v2', 'safe_v1'] }]), null);
  assert.match(questionAnswerError([q], [{ question_id: q.id, type: 'multi', values: ['safe_v1', 'safe_v1'] }])!, /duplicate/i);
  assert.equal(questionAnswerError([{ ...question, kind: 'text', required: false, options: [] }], []), null);
});

test('opaque prototype-like question IDs remain exact outbound answers', async () => {
  const { componentFunctions } = await import('./componentFunctions.ts');
  const ids = ['__proto__', 'constructor', 'toString'];
  const questions = ids.map((id) => ({ ...question, id, kind: 'multi' as const }));
  let sent: unknown;
  const state = componentFunctions(new URL('../src/modules/threads/runtime/RuntimeQuestionCard.svelte', import.meta.url), ['toggle', 'send'], {
    answers: new Map(), other: new Map(), questions, busy: false, live: true, error: null, sessionId: 's', request: { id: 'r' },
    questionAnswerError,
    runtimeThreads: { respondQuestion: async (_id: string, _request: string, answers: unknown) => { sent = answers; } },
    announce: () => {}, onanswered: () => {}, loadErrorText: String,
  });
  for (const id of ids) state.toggle(id, 'safe_v1', true);
  await state.send({ preventDefault() {} });
  assert.deepEqual(JSON.parse(JSON.stringify(sent ?? null)), ids.map((id) => ({ question_id: id, type: 'multi', values: ['safe_v1'] })));
});

test('ACP required fields allow empty values but still require property presence', () => {
  const qs = [{ ...question, id: 'tags', kind: 'multi' as const, allow_empty: true }, { ...question, id: 'name', kind: 'text' as const, allow_empty: true }];
  assert.equal(questionAnswerError(qs, [{ question_id: 'tags', type: 'multi', values: [] }, { question_id: 'name', type: 'text', text: '' }]), null);
  assert.match(questionAnswerError(qs, [])!, /required/);
});

test('an untouched ACP form sends explicit empty native fields', async () => {
  const { componentFunctions } = await import('./componentFunctions.ts');
  let sent: unknown;
  const state = componentFunctions(new URL('../src/modules/threads/runtime/RuntimeQuestionCard.svelte', import.meta.url), ['initializeEmptyAnswers', 'send'], {
    answers: new Map(), questions: [{ ...question, id: 'tags', kind: 'multi', allow_empty: true }, { ...question, id: 'name', kind: 'text', allow_empty: true }],
    busy: false, live: true, error: null, sessionId: 's', request: { id: 'r' }, questionAnswerError,
    runtimeThreads: { respondQuestion: async (_id: string, _request: string, answers: unknown) => { sent = answers; } },
    announce: () => {}, onanswered: () => {}, loadErrorText: String,
  });
  state.initializeEmptyAnswers();
  await state.send({ preventDefault() {} });
  assert.deepEqual(JSON.parse(JSON.stringify(sent)), [{ question_id: 'tags', type: 'multi', values: [] }, { question_id: 'name', type: 'text', text: '' }]);
});
