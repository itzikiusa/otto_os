import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';

function functions(path: string, names: string[], state: Record<string, any>) {
  const text = readFileSync(new URL(path, import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('component.ts', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const source = file.statements.filter(n => ts.isFunctionDeclaration(n) && n.name && names.includes(n.name.text)).map(n => n.getText(file)).join('\n');
  const context = { ...state, setTimeout, clearTimeout, Error };
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return context as Record<string, any>;
}
const snipFile = '../src/modules/snip/SnipEditor.svelte';

test('a draft without a source revision becomes dirty after typing', () => {
  const text = readFileSync(new URL('../src/modules/product/OverviewTab.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('overview.ts', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const declaration = file.statements.filter(ts.isVariableStatement).flatMap(n => [...n.declarationList.declarations])
    .find(n => n.name.getText(file) === 'draftDirty');
  assert.ok(declaration?.initializer);
  const expression = declaration.initializer.getText(file);
  const state = {isDraft:true,story:{title:'New draft'},source:null,draftTitle:'New draft',draftBody:'', $derived:(value:unknown)=>value};
  assert.equal(runInNewContext(expression,state),false);
  state.draftBody='Unsaved notes';
  assert.equal(runInNewContext(expression,state),true);
});

function snip(overrides: Record<string, unknown>) {
  return functions(snipFile, ['copyNow', 'drainPersistence', 'approveLeave'], {
    img: {}, annos: [], savedHash: '[]', copyTimer: null, copyInFlight: null,
    copyAgain: false, copyState: 'idle', snipId: 'snip', leavePending: null, allowClose: false,
    annosHash: JSON.stringify, uploadNeeded: (a: unknown, saved: string) => JSON.stringify(a) === saved ? null : JSON.stringify(a),
    flatten: async () => new Blob(), toasts: { error() {} }, toastError() {}, commitText() {},
    confirmer: { choose: async () => ({value: null}) },
    ...overrides,
  });
}

test('explicit Copy retries the clipboard for an unchanged persisted image', async () => {
  let copies = 0;
  const s = snip({ snipApi: { copy: async () => { ++copies; return { copied: true }; } } });
  await s.copyNow(true);
  assert.equal(copies, 1); assert.equal(s.copyState, 'copied');
});

test('keyboard Copy explicitly recopies unchanged content', () => {
  let explicit = false;
  const s = functions(snipFile, ['onKeydown'], { confirmer: {open:false}, textDraft:null, canvasEl:{}, selected:null,
    copyNow: (value: boolean) => { explicit = value; } });
  s.onKeydown({key:'c',metaKey:true,ctrlKey:false,target:{},preventDefault(){}});
  assert.equal(explicit, true);
});

test('a dirty snip without a decoded image fails persistence without spinning', async () => {
  const s = snip({img:null,annos:[{id:1}]});
  assert.equal(await s.drainPersistence(), false);
});

test('close retains a draft after failed persistence and allows explicit discard', async () => {
  let choice: string | null = null;
  const s = snip({ annos: [{id:1}], snipApi: { saveAnnotatedPng: async () => { throw new Error('offline'); } },
    confirmer: { choose: async () => ({value: choice}) } });
  assert.equal(await s.approveLeave(), false);
  assert.equal(s.annos.length, 1);
  choice = 'discard'; assert.equal(await s.approveLeave(), true);
});

test('close waits for the newest annotation while a previous upload is pending', async () => {
  const gate = deferred<unknown>(); const writes: string[] = [];
  const s = snip({ annos: [{id:1}], flatten: async (_img: unknown, a: unknown) => JSON.stringify(a),
    snipApi: { saveAnnotatedPng: async (_id: string, a: string) => { writes.push(a); if (writes.length === 1) await gate.promise; return { copied: false }; } } });
  const first = s.copyNow(); await Promise.resolve();
  s.annos = [{id:1},{id:2}]; const leaving = s.approveLeave();
  gate.resolve(null); await first;
  assert.equal(await leaving, true);
  assert.equal(writes.length, 2); assert.equal(s.savedHash, JSON.stringify(s.annos));
});

test('failed publishing preview remains unavailable until a successful retry', async () => {
  let fail = true;
  const s = functions('../src/modules/product/PublishDialog.svelte', ['loadPreview','submit'], {
    previewSequence: 0, previewStoryId: null, previewError: '', previewBody: null, submitting: false,
    product: { selectedId: 'A' }, api: { get: async () => { if (fail) throw new Error('offline'); return []; } },
  });
  await s.loadPreview('A'); assert.match(s.previewError, /offline/); assert.equal(s.previewBody, null);
  await s.submit(); // Must return before touching any publishing dependency.
  fail = false; await s.loadPreview('A'); assert.equal(s.previewBody, ''); assert.equal(s.previewStoryId, 'A');
});

test('artifact keep-mine never reloads over a draft when history lookup fails', async () => {
  let reloads = 0;
  const s = functions('../src/modules/design-hall/ArtifactView.svelte', ['resolveConflict'], {
    id: 'A', source: 'my draft', api: { listVersions: async () => { throw new Error('offline'); } },
    confirmer: { choose: async () => ({value:'mine'}) }, toasts: { error() {} }, load: async () => { ++reloads; },
  });
  await s.resolveConflict(); assert.equal(reloads, 0); assert.equal(s.source, 'my draft');
});

test('Brand keep-mine retries within one save and preserves edits on retry failure', async () => {
  class ApiError extends Error {status = 409;}
  let failRetry = false, calls: Array<{content:string;base_version:string}> = [], saved: string | null = null;
  const s = functions('../src/modules/design-hall/brand/BrandEditor.svelte', ['save','resolveConflict'], {
    id:'kit',saving:false,baseVersionId:'v1',draftText:'my kit',localChanges:[],commitMessage:()=> 'save',
    ApiError,errText:String,artifact:{title:'Kit'},usage:null,showImpact:false,
    api:{commitVersion:async (_id:string,body:{content:string;base_version:string})=>{
      calls.push(body);if(body.base_version==='v1')throw new ApiError('changed');
      if(failRetry)throw new Error('offline');return {created:true,version:{id:'v3',seq:3}};
    },listVersions:async()=>[{id:'v2',seq:2}]},
    confirmer:{choose:async()=>({value:'mine'})},applySaved:(_res:unknown,text:string)=>{saved=text;},
    toasts:{error(){},success(){},info(){}},
  });
  await s.save(); assert.equal(calls.length,2);assert.equal(calls[1].base_version,'v2');assert.equal(saved,'my kit');
  failRetry=true;calls=[];saved=null;await s.save();assert.equal(saved,null);assert.equal(s.draftText,'my kit');assert.equal(s.saving,false);
});

test('SourceSearch preserves existing results and retries the failed page context', async () => {
  let fail = true;
  const urls: string[] = [];
  const s = functions('../src/modules/product/SourceSearch.svelte', ['search'], {
    accountId:'account',sourceKind:'jira',searchCtl:null,searchSeq:0,searchError:'',retrySearch:null,
    selectedProjectKey:'P',selectedSpaceKey:'',searching:false,loadingMore:false,searched:true,
    jiraResults:[{key:'P-1'}],confluenceResults:[],jiraOffset:25,jiraHasMore:true,
    AbortController,loadErrorText:String,
    api:{get:async(url:string)=>{urls.push(url);if(fail)throw new Error('offline');return [{key:'P-2'}];}},
  });
  await s.search('hello',25,true);assert.match(s.searchError,/offline/);assert.equal(s.jiraResults[0].key,'P-1');
  fail=false;s.retrySearch();await new Promise(resolve=>setTimeout(resolve,0));
  assert.equal(urls[1],urls[0]);assert.equal(s.jiraResults[1].key,'P-2');assert.equal(s.searchError,'');
});
