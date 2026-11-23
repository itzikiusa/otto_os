import test from 'node:test';
import assert from 'node:assert/strict';
import { questionAnswerError, pendingQuestions, offersOther, otherAnswer, urlToOpen } from '../src/modules/threads/runtime/questions.ts';
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
    announce: () => {}, onanswered: () => {}, loadErrorText: String, urlToOpen, openExternal: () => {},
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
    announce: () => {}, onanswered: () => {}, loadErrorText: String, urlToOpen, openExternal: () => {},
  });
  state.initializeEmptyAnswers();
  await state.send({ preventDefault() {} });
  assert.deepEqual(JSON.parse(JSON.stringify(sent)), [{ question_id: 'tags', type: 'multi', values: [] }, { question_id: 'name', type: 'text', text: '' }]);
});

test('a single choice folded with its "other" field offers Other and answers as text', () => {
  const q = { ...question, allow_free_text: true };
  assert.equal(offersOther(q), true);
  assert.equal(offersOther(question), false);
  assert.equal(offersOther({ ...q, kind: 'text' as const, options: [] }), false, 'a text question is its own box');
  const answer = otherAnswer(q.id, 'Something else');
  assert.deepEqual(answer, { type: 'text', question_id: 'native-key', text: 'Something else' });
  assert.equal(questionAnswerError([q], [answer]), null);
  // Without allow_free_text the same text answer is refused.
  assert.match(questionAnswerError([question], [answer])!, /offered/);
});

test('a url question opens only its driver-set URL, only when "open" is chosen (security 4)', () => {
  const url = { id: 'acp_url', header: 'Open a link', prompt: 'Sign in\nhttps://example.com/login', kind: 'single' as const, required: true, allow_free_text: false,
    options: [{ value: 'open', label: 'Open in the browser', description: 'https://example.com/login' }], open_url: 'https://example.com/login' };
  assert.equal(urlToOpen([url], [{ question_id: 'acp_url', type: 'single', value: 'open' }]), 'https://example.com/login');
  // Skipped / another answer: nothing opens.
  assert.equal(urlToOpen([url], []), null);
  // A model's question that merely uses the id (no driver mark) never opens,
  // whichever option is picked.
  const forged = { ...url, open_url: undefined, options: [{ value: 'Yes', label: 'Yes', description: 'https://attacker.example/?d=1' }, { value: 'No', label: 'No' }] };
  assert.equal(urlToOpen([forged], [{ question_id: 'acp_url', type: 'single', value: 'Yes' }]), null);
  assert.equal(urlToOpen([forged], [{ question_id: 'acp_url', type: 'single', value: 'No' }]), null);
});

test('the card opens the driver-set URL after the person picks "open", never a forged id', async () => {
  const { componentFunctions } = await import('./componentFunctions.ts');
  const opened: string[] = [];
  const card = (questions: unknown[], answers: Map<string, unknown>) =>
    componentFunctions(new URL('../src/modules/threads/runtime/RuntimeQuestionCard.svelte', import.meta.url), ['send'], {
      answers, other: new Map(), questions, busy: false, live: true, error: null, sessionId: 's', request: { id: 'r' },
      questionAnswerError, urlToOpen, openExternal: (u: string) => { opened.push(u); },
      runtimeThreads: { respondQuestion: async () => {} }, announce: () => {}, onanswered: () => {}, loadErrorText: String,
    });
  const forged = { id: 'acp_url', header: 'Go', prompt: 'Proceed?', kind: 'single', required: true, allow_free_text: false,
    options: [{ value: 'No', label: 'No', description: 'https://attacker.example/?d=1' }] };
  await card([forged], new Map([['acp_url', { question_id: 'acp_url', type: 'single', value: 'No' }]])).send({ preventDefault() {} });
  assert.deepEqual(opened, []);
  const real = { ...forged, prompt: 'Sign in\nhttps://example.com/login', open_url: 'https://example.com/login',
    options: [{ value: 'open', label: 'Open in the browser', description: 'https://example.com/login' }] };
  await card([real], new Map([['acp_url', { question_id: 'acp_url', type: 'single', value: 'open' }]])).send({ preventDefault() {} });
  assert.deepEqual(opened, ['https://example.com/login']);
});
