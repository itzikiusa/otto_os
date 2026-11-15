import { test } from 'node:test';
import assert from 'node:assert/strict';
import { snapshotDraft, acknowledgeComposer } from '../src/lib/stores/runtimeDraftPersistence.ts';
import { componentFunctions } from './componentFunctions.ts';
import { loadSource } from './sourceHarness.ts';

// No agents picked as chips: submit sends to the thread's own agent.
const sendHelpers = { snapshotDraft, acknowledgeComposer, assembleComposerMessage: (text: string) => text, askTargets: { for: () => [] }, askFromComposer: async () => {} };
const component = new URL('../src/modules/threads/runtime/RuntimeThread.svelte', import.meta.url);

test('with agents picked as chips, submit asks them and never prompts this thread', async () => {
  const asked: unknown[][] = [];
  const prompts: string[] = [];
  const draft = { flushSave: () => {}, freshContext: false, context: [], text: 'why is CI red?', images: [], model: null, effort: null, pickedMode: null, sending: false, reopening: false, recovered: true, awaitSeq: null, startError: null };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    ...sendHelpers,
    askTargets: { for: () => [{ provider: 'codex' }] },
    askFromComposer: async (...args: unknown[]) => { asked.push(args); },
    sessionId: 'A', ownerId: 'A', draft, needsRevive: false, running: false,
    parseInteractiveSlashInput: () => null,
    runtimeThreads: { prompt: async (id: string) => { prompts.push(id); return { sequence: 1 }; } },
    runtimeDrafts: { owns: () => true },
    toastError: () => {}, ApiError: class extends Error {},
  });
  state.submit();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(prompts, []);
  assert.equal(asked.length, 1);
  assert.equal(asked[0][0], 'A');
  assert.equal(asked[0][2], 'why is CI red?');
});

test('reopening A then switching to B sends the waiting prompt only to A', async () => {
  let finish!: () => void;
  const calls: string[] = [];
  const draft = { flushSave: () => {}, freshContext: false, context: [], text: 'for A', images: [], model: null, effort: null, pickedMode: null, sending: false, reopening: false, awaitSeq: null, startError: null };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    ...sendHelpers,
    sessionId: 'A', ownerId: 'A', text: 'for A', images: [], model: null, effort: null, pickedMode: null,
    draft, needsRevive: true, reopening: false, sending: false, running: false, awaitSeq: null, startError: null,
    parseInteractiveSlashInput: () => null,
    runtimeThreads: {
      reopen: () => new Promise<void>((resolve) => { finish = resolve; }),
      prompt: async (id: string) => { calls.push(id); return { sequence: 1 }; },
    },
    runtimeDrafts: { owns: () => true },
    toastError: () => {}, ApiError: class extends Error {},
  });
  state.submit();
  state.sessionId = 'B';
  finish();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, ['A']);
});

test('runtime drafts retain attachments and choices only for their owning thread', async () => {
  const { importRunes, svelteClient } = await import('./runesHarness.ts');
  const { RuntimeDrafts } = await importRunes<{ RuntimeDrafts: new () => { for(id: string): any } }>(new URL('../src/lib/stores/runtimeDrafts.svelte.ts', import.meta.url));
  const store = new RuntimeDrafts();
  const a = store.for('A');
  const b = store.for('B');
  a.text = 'draft for A';
  a.pickedMode = 'full-access';
  a.model = 'model-a';
  a.images.push({ name: 'a.png', mime: 'image/png', data_base64: 'AA==' });
  assert.equal(b.text, '');
  assert.equal(b.pickedMode, null);
  assert.equal(b.model, undefined);
  assert.equal(b.images.length, 0);
  assert.equal(store.for('A'), a);
  assert.equal(store.for('A').text, 'draft for A');
  // A file read started before switching still updates the remounted owner's draft.
  const runtime = await svelteClient();
  let count = 0;
  const stop = runtime.effectRoot(() => runtime.effect(() => { count = store.for('A').images.length; }));
  runtime.flushSync();
  a.images.push({ name: 'second.png', mime: 'image/png', data_base64: 'BB==' });
  runtime.flushSync();
  assert.equal(count, 2);
  assert.equal(b.images.length, 0);
  stop();
});

test('a failed A send restores A draft while B stays untouched', async () => {
  let reject!: (e: Error) => void;
  const a = { flushSave: () => {}, freshContext: false, context: [], text: 'draft A', images: [{ name: 'a.png', mime: 'image/png', data_base64: 'AA==' }], model: null, effort: null, pickedMode: null, sending: false, awaitSeq: null, startError: null };
  const b = { text: 'draft B', images: [] };
  const calls: string[] = [];
  const state = componentFunctions(component, ['send'], {
    ...sendHelpers,
    sessionId: 'A', ownerId: 'A', draft: a,
    runtimeThreads: { prompt: (id: string) => { calls.push(id); return new Promise((_resolve, fail) => { reject = fail; }); } },
    runtimeDrafts: { owns: () => true },
    ApiError: class extends Error {}, toastError: () => {},
  });
  const pending = state.send('draft A');
  state.sessionId = 'B';
  reject(new Error('offline'));
  await pending;
  assert.deepEqual(calls, ['A']);
  assert.equal(a.text, 'draft A');
  assert.equal(a.images[0]?.name, 'a.png');
  assert.equal(a.sending, false);
  assert.equal(b.text, 'draft B');
  assert.equal(b.images.length, 0);
});

test('deleted threads and changed identities forget drafts without dropping a same-identity draft', async () => {
  const { importRunes } = await import('./runesHarness.ts');
  const { RuntimeDrafts } = await importRunes<{ RuntimeDrafts: new () => { for(id: string): any; forget(id: string): void; setIdentity(id: string): void } }>(new URL('../src/lib/stores/runtimeDrafts.svelte.ts', import.meta.url));
  const drafts = new RuntimeDrafts();
  drafts.setIdentity('server-a/user-a');
  drafts.for('A').text = 'private';
  drafts.for('A').images.push({ name: 'secret.png', mime: 'image/png', data_base64: 'AA==' });
  drafts.setIdentity('server-a/user-a');
  assert.equal(drafts.for('A').text, 'private');
  drafts.forget('A');
  assert.equal(drafts.for('A').text, '');
  assert.equal(drafts.for('A').images.length, 0);
  drafts.for('A').text = 'another private draft';
  drafts.setIdentity('server-a/user-b');
  assert.equal(drafts.for('A').text, '');
  drafts.for('A').text = 'user B draft';
  drafts.setIdentity('server-b/user-b');
  assert.equal(drafts.for('A').text, '');
});


test('an identity change while reopening cancels the old pending prompt', async () => {
  let finish!: () => void;
  let current = true;
  const calls: string[] = [];
  const draft = { flushSave: () => {}, freshContext: false, context: [], text: 'private A', images: [], model: null, effort: null, pickedMode: null, reopening: false };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    ...sendHelpers,
    sessionId: 'A', ownerId: 'A', draft, needsRevive: true, running: false,
    parseInteractiveSlashInput: () => null,
    runtimeDrafts: { owns: () => current },
    runtimeThreads: {
      reopen: () => new Promise<void>((resolve) => { finish = resolve; }),
      prompt: async (id: string) => { calls.push(id); return { sequence: 1 }; },
    },
    toastError: () => {}, ApiError: class extends Error {},
  });
  state.submit();
  current = false;
  finish();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, []);
});

test('successful send removes only its snapshot while newer typing and chips remain', async () => {
  let finish!: (value: {sequence:number}) => void;
  const draft = { flushSave: () => {}, freshContext: true, context: [{ id:'old', kind:'diff', label:'old', text:'old' }], text:'submitted', images:[], model:null, effort:null, pickedMode:null, sending:false, awaitSeq:null, startError:null };
  const state = componentFunctions(component, ['send'], {
    ...sendHelpers, sessionId:'A', ownerId:'A', draft,
    runtimeThreads:{prompt:()=>new Promise(resolve=>finish=resolve)},runtimeDrafts:{owns:()=>true}, ApiError:class extends Error{},toastError:()=>{},
  });
  const sending=state.send('submitted'); draft.text='next message'; draft.context.push({id:'new',kind:'diff',label:'new',text:'new'});
  finish({sequence:1}); await sending;
  assert.equal(draft.text,'next message'); assert.deepEqual(draft.context.map(c=>c.id),['new']); assert.equal(draft.freshContext,false);
});

test('reopen uses the submitted snapshot and never consumes edits made while reopening', async () => {
  let finish!:()=>void; const calls:any[]=[];
  const draft={flushSave:()=>{},freshContext:false,context:[],text:'first',images:[],model:null,effort:null,pickedMode:null,reopening:false,sending:false};
  const state=componentFunctions(component,['submit','send','reopen'],{
    ...sendHelpers,sessionId:'A',ownerId:'A',draft,needsRevive:true,running:false,parseInteractiveSlashInput:()=>null,
    runtimeDrafts:{owns:()=>true},runtimeThreads:{reopen:()=>new Promise<void>(r=>finish=r),prompt:async(_id:string,body:unknown)=>{calls.push(body);return {sequence:1};}},toastError:()=>{},ApiError:class extends Error{},
  });
  state.submit(); draft.text='second'; finish(); await new Promise(r=>setTimeout(r,0));
  assert.equal(calls[0].text,'first'); assert.equal(draft.text,'second');
});

test('failed send preserves an intentional clear while the request was pending', async () => {
  let fail!:(error:Error)=>void;
  const draft={flushSave:()=>{},freshContext:false,context:[],text:'submitted',images:[],model:null,effort:null,pickedMode:null,sending:false};
  const state=componentFunctions(component,['send'],{...sendHelpers,ownerId:'A',draft,runtimeDrafts:{owns:()=>true},runtimeThreads:{prompt:()=>new Promise((_r,reject)=>fail=reject)},ApiError:class extends Error{},toastError:()=>{}});
  const sending=state.send('submitted'); draft.text=''; fail(Error('offline')); await sending; assert.equal(draft.text,'');
});

test('submit clears composer in 0ms, skips /reopen on dormant threads, and restores draft if prompt fails', async () => {
  const { buildOptimisticUserMessage, restoreOptimisticDraft, withOptimisticItem } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url),
    {
      '../../../lib/toastError': { toastError: () => {} },
      '../../../lib/telemetry': { startMeasurement: () => null },
      './hostCommands': { runHostSlash: () => false },
      './timeline': { errorText: (s: string) => s },
      '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: {} },
    },
  ) as typeof import('../src/modules/threads/runtime/runtimeComposer.ts');
  let failPrompt!: (e: Error) => void;
  let reopenCalls = 0;
  const promptCalls: unknown[] = [];
  const draft = {
    flushSave: () => {},
    freshContext: false,
    context: [],
    text: 'fast mobile reply',
    images: [],
    model: null,
    effort: null,
    pickedMode: null,
    reopening: false,
    sending: false,
  };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    ...sendHelpers,
    buildOptimisticUserMessage,
    restoreOptimisticDraft,
    sessionId: 'A',
    ownerId: 'A',
    draft,
    closed: false,
    needsRevive: true, // dormant: `closed === false`, `needsRevive === true`
    running: false,
    optimisticMessage: null,
    p: { items: [] },
    parseInteractiveSlashInput: () => null,
    runtimeDrafts: { owns: () => true },
    runtimeThreads: {
      reopen: async () => {
        reopenCalls += 1;
      },
      prompt: (_id: string, body: unknown) => {
        promptCalls.push(body);
        return new Promise((_r, reject) => {
          failPrompt = reject;
        });
      },
    },
    toastError: () => {},
    ApiError: class extends Error {},
  });

  state.submit();
  // Cleared synchronously before any promise settles, and `/reopen` was skipped!
  assert.equal(draft.text, '');
  assert.equal(reopenCalls, 0);
  assert.equal(promptCalls.length, 1);
  assert.ok(state.optimisticMessage, 'optimistic user bubble created immediately');
  assert.equal(withOptimisticItem([], state.optimisticMessage).length, 1);

  // Deduplicates as soon as the server user_message item arrives.
  const serverMsg = {
    ...state.optimisticMessage.item,
    id: 'srv-u-1',
    ordinal: 1,
  };
  assert.equal(withOptimisticItem([serverMsg], state.optimisticMessage).length, 1);

  // On network error with empty composer, restores the user's text and clears optimistic bubble.
  failPrompt(new Error('network down'));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(draft.text, 'fast mobile reply');
  assert.equal(state.optimisticMessage, null);
});

test('RuntimeThread and runtimeComposer distinguish inherited undefined vs explicit null Default model', async () => {
  const { resolveEffectiveDraftModel, resolvePromptModelOverride } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url),
    {
      '../../../lib/toastError': { toastError: () => {} },
      '../../../lib/telemetry': { startMeasurement: () => null },
      './hostCommands': { runHostSlash: () => false },
      './timeline': { errorText: (s: string) => s },
      '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: {} },
    },
  ) as typeof import('../src/modules/threads/runtime/runtimeComposer.ts');

  // `undefined` inherits `session.model` and omits `model` on prompt payload.
  assert.equal(resolveEffectiveDraftModel(undefined, 'claude-opus-4-6'), 'claude-opus-4-6');
  assert.equal(resolvePromptModelOverride(undefined, 'claude-opus-4-6'), undefined);

  // Explicit `null` ("Default model") displays Default (`null`) and sends `""` to clear a pinned `session.model`.
  assert.equal(resolveEffectiveDraftModel(null, 'claude-opus-4-6'), null);
  assert.equal(resolvePromptModelOverride(null, 'claude-opus-4-6'), '');
  assert.equal(resolvePromptModelOverride(null, null), undefined);

  // Explicit model string pins that model.
  assert.equal(resolveEffectiveDraftModel('sonnet', 'claude-opus-4-6'), 'sonnet');
  assert.equal(resolvePromptModelOverride('sonnet', 'claude-opus-4-6'), 'sonnet');

  const promptPayloads: any[] = [];
  const draft = {
    flushSave: () => {},
    freshContext: false,
    context: [],
    text: 'clear pinned model',
    images: [],
    model: null,
    effort: null,
    pickedMode: null,
    sending: false,
    awaitSeq: null,
    startError: null,
  };
  const state = componentFunctions(component, ['send'], {
    ...sendHelpers,
    resolvePromptModelOverride,
    session: { model: 'claude-opus-4-6' },
    sessionId: 'A',
    ownerId: 'A',
    draft,
    runtimeThreads: {
      prompt: async (_id: string, body: unknown) => {
        promptPayloads.push(body);
        return { sequence: 2 };
      },
    },
    runtimeDrafts: { owns: () => true },
    ApiError: class extends Error {},
    toastError: () => {},
  });
  await state.send('clear pinned model');
  assert.equal(promptPayloads.length, 1);
  assert.equal(promptPayloads[0].model, '');
});

test('image-only optimistic send populates immediate preview_url attachments instead of an empty bubble', async () => {
  const { buildOptimisticUserMessage, restoreOptimisticDraft } = loadSource(
    new URL('../src/modules/threads/runtime/runtimeComposer.ts', import.meta.url),
    {
      '../../../lib/toastError': { toastError: () => {} },
      '../../../lib/telemetry': { startMeasurement: () => null },
      './hostCommands': { runHostSlash: () => false },
      './timeline': { errorText: (s: string) => s },
      '../../../lib/stores/runtimeDrafts.svelte': { runtimeDrafts: {} },
    },
  ) as typeof import('../src/modules/threads/runtime/runtimeComposer.ts');

  const direct = buildOptimisticUserMessage('', 4, [
    { name: 'shot.png', mime: 'image/png', data_base64: 'QUJD' },
    { name: 'spec.pdf', mime: 'application/pdf', data_base64: 'REVG' },
  ]);
  assert.equal(direct.item.type, 'user_message');
  if (direct.item.type === 'user_message') {
    assert.equal(direct.item.text, '');
    assert.equal(direct.item.attachments?.length, 2);
    assert.equal(direct.item.attachments?.[0]?.kind, 'image');
    assert.equal(direct.item.attachments?.[0]?.name, 'shot.png');
    assert.equal(direct.item.attachments?.[0]?.media_type, 'image/png');
    assert.equal(direct.item.attachments?.[0]?.preview_url, 'data:image/png;base64,QUJD');
    assert.equal(direct.item.attachments?.[1]?.kind, 'file');
    assert.equal(direct.item.attachments?.[1]?.preview_url, undefined);
  }

  let resolvePrompt!: (v: { sequence: number }) => void;
  const draft = {
    flushSave: () => {},
    freshContext: false,
    context: [],
    text: '',
    images: [{ name: 'shot.png', mime: 'image/png', data_base64: 'QUJD' }],
    model: undefined,
    effort: null,
    pickedMode: null,
    reopening: false,
    sending: false,
  };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    ...sendHelpers,
    buildOptimisticUserMessage,
    restoreOptimisticDraft,
    sessionId: 'A',
    ownerId: 'A',
    draft,
    closed: false,
    needsRevive: false,
    running: false,
    optimisticMessage: null,
    p: { items: [] },
    parseInteractiveSlashInput: () => null,
    runtimeDrafts: { owns: () => true },
    runtimeThreads: {
      prompt: () =>
        new Promise((r) => {
          resolvePrompt = r;
        }),
    },
    toastError: () => {},
    ApiError: class extends Error {},
  });

  state.submit();
  assert.equal(draft.images.length, 0);
  assert.equal(state.optimisticMessage?.item?.attachments?.[0]?.preview_url, 'data:image/png;base64,QUJD');
  resolvePrompt({ sequence: 3 });
  await new Promise((r) => setTimeout(r, 0));
});
