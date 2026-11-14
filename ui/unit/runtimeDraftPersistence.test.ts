import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DraftPersistence, emptyDraft, acknowledgeDraft } from '../src/lib/stores/runtimeDraftPersistence.ts';

const deferred = <T>() => { let resolve!: (value:T)=>void; const promise = new Promise<T>(r=>resolve=r); return {promise,resolve}; };
test('late recovery keeps typing and restores untouched attachment fields', async () => {
  const pending = deferred<any>(); const draft = emptyDraft();
  const saved = {...emptyDraft(), text:'older', images:[{name:'a.png',mime:'image/png',data_base64:'AA=='}]};
  const p = new DraftPersistence(draft, {load:()=>pending.promise, save:async d=>({revision:2,draft:d})}, ()=>true);
  const loading = p.load(); draft.text='new typing';
  pending.resolve({revision:1,draft:saved}); await loading;
  assert.equal(draft.text,'new typing'); assert.equal(draft.images[0]?.name,'a.png');
});
test('saves serialize and drain the newest edit after an in-flight save', async () => {
  const pending = deferred<any>(); const draft = emptyDraft(); const calls:any[]=[];
  const p = new DraftPersistence(draft, {load:async()=>({revision:0,draft:null}), save:async(d,r)=>{calls.push([d,r]);return calls.length===1?pending.promise:{revision:2,draft:d};}}, ()=>true);
  await p.load(); draft.text='one'; const saving=p.save(); draft.text='two'; void p.save();
  pending.resolve({revision:1,draft:calls[0][0]}); await saving;
  assert.equal(calls.length,2); assert.equal(calls[1][0].text,'two'); assert.equal(calls[1][1],1);
});
test('identity change discards late recovery and acknowledgement preserves newer content', async () => {
  const pending=deferred<any>(); const draft=emptyDraft(); let owns=true;
  const p=new DraftPersistence(draft,{load:()=>pending.promise,save:async d=>({revision:1,draft:d})},()=>owns);
  const loading=p.load(); owns=false; pending.resolve({revision:1,draft:{...emptyDraft(),text:'private'}}); await loading;
  assert.equal(draft.text,'');
  draft.text='sent'; draft.context=[{id:'1',kind:'diff',label:'diff',text:'first'}];
  const snapshot=structuredClone(draft); draft.text='next'; draft.context.push({id:'2',kind:'diff',label:'diff2',text:'second'});
  acknowledgeDraft(draft,snapshot); assert.equal(draft.text,'next'); assert.deepEqual(draft.context.map(x=>x.id),['2']);
});

test('failed initial load retry cannot overwrite typing and merges newly added context', async () => {
  const draft=emptyDraft(); let attempt=0;
  const p=new DraftPersistence(draft,{load:async()=>{if (++attempt===1) throw Error('offline'); return {revision:3,draft:{...emptyDraft(),text:'saved',context:[{id:'old',kind:'diff',label:'old',text:'old'}]}};},save:async d=>({revision:4,draft:d})},()=>true);
  await p.load(); draft.text='typed while offline'; draft.context=[{id:'new',kind:'diff',label:'new',text:'new'}]; await p.retry();
  assert.equal(draft.text,'typed while offline'); assert.deepEqual(draft.context.map(c=>c.id),['old','new']);
});
test('conflicting window is never overwritten automatically; explicit Retry updates its revision', async () => {
  const draft=emptyDraft(); const calls:number[]=[]; let loadCount=0;
  const p=new DraftPersistence(draft,{load:async()=>({revision:loadCount++?5:0,draft:null}),save:async(d,r)=>{calls.push(r); if(r===0) throw {status:409}; return {revision:r+1,draft:d};}},()=>true);
  await p.load(); draft.text='local'; await p.save(); draft.text='local edited'; await p.save(); assert.deepEqual(calls,[0]);
  await p.retry(); assert.deepEqual(calls,[0,5]); assert.equal(draft.text,'local edited');
});
test('send acknowledgement persists empty content behind an older in-flight save', async () => {
  const draft=emptyDraft(); const pending=deferred<any>(); const calls:any[]=[];
  const p=new DraftPersistence(draft,{load:async()=>({revision:0,draft:null}),save:async(d,r)=>{calls.push(d);return calls.length===1?pending.promise:{revision:r+1,draft:d};}},()=>true);
  await p.load(); draft.text='sent'; const writing=p.save(); const submitted=structuredClone(draft); acknowledgeDraft(draft,submitted); void p.save();
  pending.resolve({revision:1,draft:calls[0]}); await writing;
  assert.deepEqual(calls.map(c=>c.text),['sent','']);
});

test('conflict retry compares local content against the refreshed remote draft', async () => {
  const draft=emptyDraft(); let reads=0; let remote={...emptyDraft(),text:'original'}; let attempts=0;
  const p=new DraftPersistence(draft,{load:async()=>({revision:++reads,draft:structuredClone(remote)}),save:async(d,r)=>{attempts++; if(attempts===1) throw {status:409};remote=structuredClone(d);return {revision:r+1,draft:d};}},()=>true);
  await p.load(); remote.text='other window'; draft.text='edited'; await p.save(); draft.text='original'; await p.retry();
  assert.equal(remote.text,'original'); assert.equal(attempts,2);
});

test('recovery of an unchanged composer does not write a new revision', async () => {
  const { composerContent } = await import('../src/lib/stores/runtimeDraftPersistence.ts');
  const composer={text:'',images:[],context:[],model:null,effort:null,pickedMode:null,freshContext:false};
  let writes=0;
  const p=new DraftPersistence(composerContent(composer),{load:async()=>({revision:4,draft:emptyDraft()}),save:async d=>{writes++;return {revision:5,draft:d};}},()=>true);
  await p.save(); assert.equal(writes,0);
});

test('acknowledgement consumes only the submitted count of identical attachments', () => {
  const draft=emptyDraft();
  const image={name:'same.png',mime:'image/png',data_base64:'AA=='};
  draft.images=[{...image}];
  const submitted=structuredClone(draft);
  draft.images.push({...image});
  acknowledgeDraft(draft,submitted);
  assert.deepEqual(draft.images,[image]);
});

test('syncIfClean updates revision after another surface sends/clears the draft so subsequent edits never 409', async () => {
  const draft = emptyDraft();
  let serverRevision = 1;
  let serverDraft: ReturnType<typeof emptyDraft> | null = {
    ...emptyDraft(),
    text: 'draft before mobile send',
  };
  const savedRevisions: number[] = [];
  const p = new DraftPersistence(
    draft,
    {
      load: async () => ({
        revision: serverRevision,
        draft: serverDraft ? structuredClone(serverDraft) : null,
      }),
      save: async (d, r) => {
        savedRevisions.push(r);
        if (r !== serverRevision) throw { status: 409 };
        serverRevision = r + 1;
        serverDraft = structuredClone(d);
        return { revision: serverRevision, draft: structuredClone(d) };
      },
    },
    () => true,
  );

  await p.load();
  assert.equal(draft.text, 'draft before mobile send');

  // Another window/mobile sends the prompt: clears server draft and bumps revision 1 -> 4.
  serverRevision = 4;
  serverDraft = { ...emptyDraft(), model: 'claude-sonnet-4-6' };

  await p.syncIfClean();
  assert.equal(draft.text, '');
  assert.equal(draft.model, 'claude-sonnet-4-6');

  // User returns to this window, pastes an image + types, and saves cleanly at revision 4 with zero 409.
  draft.text = 'follow-up after mobile turn';
  draft.images = [{ name: 'clip.png', mime: 'image/png', data_base64: 'AA==' }];
  await p.save();
  assert.deepEqual(savedRevisions, [4]);
  assert.equal(serverRevision, 5);
});

test('syncIfClean never overwrites typing or image paste started while GET is in flight', async () => {
  const inflightSync = deferred<any>();
  const draft = emptyDraft();
  let loadCalls = 0;
  const savedCalls: Array<{ text: string; imageCount: number; rev: number }> = [];

  const p = new DraftPersistence(
    draft,
    {
      load: () => {
        loadCalls++;
        if (loadCalls === 1) return Promise.resolve({ revision: 2, draft: null });
        return inflightSync.promise;
      },
      save: async (d, r) => {
        savedCalls.push({ text: d.text, imageCount: d.images.length, rev: r });
        if (r !== 7) throw { status: 409 };
        return { revision: 8, draft: structuredClone(d) };
      },
    },
    () => true,
  );

  await p.load();

  // Window regains focus -> syncIfClean() starts GET; user immediately pastes & types before GET finishes!
  const syncing = p.syncIfClean();
  draft.text = 'typed during focus sync';
  draft.images = [{ name: 'paste.png', mime: 'image/png', data_base64: 'BB==' }];
  const saving = p.save();

  // Server had cleared the draft at revision 7 after another surface sent a turn.
  inflightSync.resolve({ revision: 7, draft: null });
  await Promise.all([syncing, saving]);

  assert.equal(draft.text, 'typed during focus sync');
  assert.equal(draft.images.length, 1);
  assert.deepEqual(savedCalls, [{ text: 'typed during focus sync', imageCount: 1, rev: 7 }]);

  // And when local composer already has unsaved edits, syncIfClean() is a complete no-op.
  draft.text = 'dirty local edit';
  const loadsBefore = loadCalls;
  await p.syncIfClean();
  assert.equal(loadCalls, loadsBefore);
  assert.equal(draft.text, 'dirty local edit');
});
