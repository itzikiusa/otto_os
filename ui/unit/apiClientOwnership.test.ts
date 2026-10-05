import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {randomUUID} from 'node:crypto';
import ts from 'typescript';
import { HistoryRefresh, HistoryDetail } from '../src/lib/stores/apiHistory.ts';
import * as scriptRuntime from '../src/lib/api/scripts.ts';
import * as secretShapes from '../src/lib/api/apiSecretShapes.ts';

function setup(overrides: Record<string, unknown> = {}, runScript?: (...args: any[]) => Promise<any>) {
  const ws = {currentId: 'A'};
  const api: Record<string, any> = {get: async () => [], patch: async () => ({}), post: async () => ({}), ...overrides};
  // The API client's "Send" rides the long lane (`api.long.post`).
  api.long = {post: (...args: unknown[]) => api.post(...args), get: (...args: unknown[]) => api.get(...args)};
  const context = {exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, {snapshot: (v: unknown) => v, raw: (v: unknown) => v}), $derived: (v: unknown) => v,
    crypto: {randomUUID}, URL, URLSearchParams, AbortController, DOMException, setTimeout, clearTimeout, performance, encodeURIComponent,
    localStorage: {getItem() {return null;},setItem() {}},
    require: (p: string) => p.endsWith('/client') ? {api, isAbortError: () => false}
      : p.includes('workspace.svelte') ? {ws}
      : p.includes('toast') ? {toasts: {error() {},success() {},info() {}}}
      : p.endsWith('/apiHistory') ? {HistoryRefresh, HistoryDetail}
      : p.endsWith('/apiSecretShapes') ? secretShapes
      : p.endsWith('/scriptRunner') ? {runScript}
      : p.endsWith('/lazyModule') ? {announceModule() {}}
      : p.endsWith('/plural') ? {plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`}
      : p.endsWith('/scripts') ? scriptRuntime
      : p.endsWith('/importers') ? {isImportedEnvironment: (d: any) => d.format === 'postman-env'}
      : p.endsWith('/types') ? {isSecretRef: (v: any) => !!v?.$secret} : {},
  };
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/lib/stores/apiClient.svelte.ts',import.meta.url),'utf8'),
    {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context);
  return {v: context.exports.apiClient, ws, extras: context.exports.draftToExtras};
}

test('save completion belongs to the initiating tab, never the newly active tab', async () => {
  let resolve!: (v: unknown) => void;
  const pending = new Promise(r => {resolve = r;});
  const {v} = setup({post: () => pending});
  v.draft = {...v.draft, url: 'https://a.test',name:'A'};
  const saving = v.saveDraft('A',null);
  v.newDraft(); v.draft = {...v.draft,url:'https://b.test',name:'B'};
  resolve({id:'saved-a',name:'A',auth:{type:'bearer',token:{$secret:'a-secret'}}});
  await saving;
  assert.equal(v.draft.url,'https://b.test'); assert.equal(v.draft.requestId,null);
  assert.equal(v.tabs[0].requestId,'saved-a');
});

test('runtime variables are scoped to the restored workspace', () => {
  const {v,ws} = setup(); v.restoreTabs('A');
  v.setRuntimeVar('token','token-A');
  ws.currentId='B';v.restoreTabs('B');
  assert.equal(v.runtimeVars.token,undefined);
  v.setRuntimeVar('token','token-B');
  ws.currentId='A';v.restoreTabs('A');
  assert.equal(v.runtimeVars.token,'token-A');
});

test('cleared extras serialize explicitly, and gRPC schema/method round trip', () => {
  const {v,extras} = setup();
  assert.equal(JSON.stringify(extras(v.draft)), '{"v":1}');
  const proto = 'syntax = "proto3"; service Test { rpc Read(Input) returns (Output); }';
  v.draft = {...v.draft,kind:'grpc',proto,grpc_method:'Test/Read'};
  const savedExtras = extras(v.draft);
  v.loadRequestIntoDraft({id:'grpc',name:'gRPC',method:'POST',url:'localhost:50051',headers:[],query:[],body_mode:'json',body:'{}',auth:{type:'none'},extras:savedExtras});
  assert.equal(v.draft.proto,proto); assert.equal(v.draft.grpc_method,'Test/Read');
});

test('OAuth completion updates originating inactive tab and keeps later auth edits', () => {
  const {v}=setup();v.restoreTabs('A');
  const original={type:'oauth2',access_token:''};
  v.draft={...v.draft,requestId:'oauth',auth:original};
  v.newDraft();const current=v.draft.tabId;
  v.applySavedAuth('A',{id:'oauth',auth:{type:'oauth2',access_token:{$secret:'stored'}}},original);
  assert.equal(v.draft.tabId,current);assert.equal(v.tabs[0].auth.access_token.$secret,'stored');
  v.tabs[0].auth={type:'bearer',token:'user-edit'};
  v.applySavedAuth('A',{id:'oauth',auth:{type:'oauth2',access_token:{$secret:'new'}}},original);
  assert.equal(v.tabs[0].auth.token,'user-edit');
});


test('cancel during asynchronous pre-script prevents HTTP and clears sending', async () => {
  let sent=0, release!: (value: unknown) => void;
  const script=new Promise(r=>{release=r;});
  const {v}=setup({post:async()=>{sent++;return {headers:[],status:200};}},()=>script);
  v.draft={...v.draft,url:'https://example.test',pre_request_script:'console.log(1)'};
  const pending=v.execute();
  assert.equal(v.sending,true);assert.equal(sent,0);
  v.cancelExecute();assert.equal(v.sending,false);
  release({run:{logs:[],tests:[]},request:{method:'GET',url:'https://example.test',headers:[],body:''},vars:{late:'no'}});
  await pending;assert.equal(sent,0);assert.equal(v.runtimeVars.late,undefined);
});

test('closing the initiating tab cancels a pending pre-script', async () => {
  let signal: AbortSignal | undefined;
  const {v}=setup({},(_input,s)=>{signal=s;return new Promise((_resolve,reject)=>s.addEventListener('abort',()=>reject(new DOMException('canceled','AbortError'))));});
  v.draft={...v.draft,url:'https://example.test',pre_request_script:'while(true){}'};
  const pending=v.execute();v.closeTab(0);
  assert.equal(signal?.aborted,true);await pending;assert.equal(v.sending,false);
});

test('replaced execution ignores late pre-script even with the same tab identity', async () => {
  let release!: (value: unknown) => void;let sent=0;
  const slow=new Promise(r=>{release=r;});let scripts=0;
  const {v}=setup({post:async()=>{sent++;return {headers:[],status:200};}},async input=>++scripts===1?slow:{run:{logs:[],tests:[],writes:{current:'yes'}},request:input.request,vars:{current:'yes'}});
  v.draft={...v.draft,url:'https://example.test',pre_request_script:'console.log(1)'};
  const first=v.execute();await v.execute();
  release({run:{logs:[],tests:[]},request:{method:'GET',url:'https://old.test',headers:[],body:''},vars:{old:'no'}});
  await first;assert.equal(sent,1);assert.equal(v.runtimeVars.old,undefined);assert.equal(v.runtimeVars.current,'yes');
});


test('environment stays with the execution snapshot while pre-script is pending', async () => {
  let release!: (value: unknown) => void;
  const script = new Promise(r => { release = r; });
  let dispatched: any;
  const {v} = setup({post: async (_url: string, body: any) => { dispatched = body; return {headers:[],status:200}; }}, () => script);
  v.activeEnv = {id:'env-a'};
  v.draft = {...v.draft,url:'https://example.test',pre_request_script:'console.log(1)'};
  const pending = v.execute();
  v.activeEnv = {id:'env-b'};
  release({run:{logs:[],tests:[]},request:{method:'GET',url:'https://example.test',headers:[],body:''},vars:{}});
  await pending;
  assert.equal(dispatched.environment_id, 'env-a');
});

const savedReq = (id: string, url: string, auth: unknown = {type:'none'}) =>
  ({id,name:id,method:'GET',url,headers:[],query:[],body_mode:'none',body:'',auth,extras:null});

test('opening a saved request never replaces unsaved edits in the active tab', () => {
  const {v} = setup();
  v.draft = {...v.draft, url: 'https://edit.test/x', body: 'typed'};
  v.loadRequestIntoDraft(savedReq('r1','https://r1.test'));
  assert.equal(v.tabs.length, 2, 'opened in a new tab');
  assert.equal(v.tabs[0].body, 'typed', 'unsaved edits kept');
  assert.equal(v.draft.requestId, 'r1');
  // Opening it again focuses the existing tab instead of duplicating it.
  v.switchTab(0);
  v.loadRequestIntoDraft(savedReq('r1','https://r1.test'));
  assert.equal(v.tabs.length, 2);
  assert.equal(v.activeTab, 1);
});

test('a pristine blank tab is reused when opening a saved request', () => {
  const {v} = setup();
  v.loadRequestIntoDraft(savedReq('r1','https://r1.test'));
  assert.equal(v.tabs.length, 1);
  assert.equal(v.draft.requestId, 'r1');
});

test('history replay rehydrates masked credentials from the saved request, never sends ***', async () => {
  const marker = {$secret:'otto.api.request.r1'};
  const full = savedReq('r1','https://r1.test/x',{type:'bearer',token:marker});
  const gets: string[] = [];
  // The tree holds summaries; the full row (with the auth marker) is fetched.
  const {v} = setup({get: async (url: string) => { gets.push(url); return full; }});
  v.requests = [{id:'r1',name:'r1',method:'GET',url:'https://r1.test/x',collection_id:null,workspace_id:'A',position:0}];
  await v.loadHistoryIntoDraft({id:'h1',method:'GET',url:'https://r1.test/x',
    request:{request_id:'r1',method:'GET',url:'https://r1.test/x',headers:[],query:[],auth:{type:'bearer',token:'***'}}});
  assert.deepEqual(v.draft.auth, {type:'bearer',token:marker});
  assert.deepEqual(gets, ['/workspaces/A/api-client/requests/r1']);
  // Unknown origin: blanked instead of replaying the mask.
  v.newDraft();
  await v.loadHistoryIntoDraft({id:'h2',method:'GET',url:'https://other.test/',
    request:{method:'GET',url:'https://other.test/',headers:[{key:'Authorization',value:'***',enabled:true}],query:[],auth:{type:'bearer',token:'***'}}});
  assert.deepEqual(v.draft.auth, {type:'bearer',token:''});
  assert.equal(v.draft.headers[0].value, '');
  assert.equal(v.draft.headers[0].enabled, false);
});

test('automation runs poll deltas and append only the new steps', async () => {
  const gets: string[] = [];
  const step = (name: string) => ({request_id:'r',name,status:200,duration_ms:1,ok:true,assertions:[],error:null});
  const run = {id:'run1',workspace_id:'A',automation_id:'auto',environment_id:null,created_by:'u',created_at:'',finished_at:null,stop_on_failure:false,dataset_rows:1,error:null};
  const {v} = setup({
    post: async () => ({...run,status:'running',snapshot:[{request_id:'r'}],report:{automation_id:'auto',steps:[step('one')],passed:false},result_rows:[0],result_ids:['s1']}),
    get: async (url: string) => {
      if (!url.includes('/automation-runs/run1')) return [];
      gets.push(url);
      return {...run,status:'passed',snapshot:null,report:{automation_id:'auto',steps:[step('two')],passed:true},result_rows:[0],result_ids:['s2']};
    },
  });
  const report = await v.runAutomation('auto');
  assert.equal(gets.length, 1);
  assert.match(gets[0], /\/automation-runs\/run1\?after=1$/);
  // Values come from the store's vm realm: compare as JSON.
  assert.equal(JSON.stringify(report.steps.map((s: any) => s.name)), '["one","two"]');
  assert.equal(report.passed, true);
  assert.equal(JSON.stringify(v.currentRun.result_ids), '["s1","s2"]');
  assert.equal(JSON.stringify(v.currentRun.snapshot), '[{"request_id":"r"}]');
});

test('import creates every item in order and reloads the lists once', async () => {
  const posts: string[] = [];
  const gets: string[] = [];
  let n = 0;
  const {v} = setup({
    post: async (url: string, body: any) => { posts.push(`${url.split('/api-client')[1]}:${body.name}`); return {id:`id-${n++}`,name:body.name,position:0}; },
    get: async (url: string) => { gets.push(url.split('/api-client')[1] ?? url); return []; },
  });
  const req = (name: string, folderPath: string[]) => ({name,method:'GET',url:'https://x.test',folderPath,headers:[],query:[],body_mode:'none',body:'',auth:{type:'none'}});
  await v.importParsed({name:'Imported',format:'postman',requests:[req('a',['F']),req('b',['F']),req('c',[])]}, true);
  assert.deepEqual(posts, ['/collections:Imported','/collections:F','/requests:a','/requests:b','/requests:c']);
  assert.equal(gets.filter((g) => g === '/requests/summaries').length, 1);
  assert.equal(gets.filter((g) => g === '/requests').length, 0, 'never the full rows');
  assert.equal(gets.filter((g) => g === '/collections').length, 1);
});

// ── perf2 N1: summaries in the tree, the full row on open ──────────────────

test('opening a saved request fetches its full row once and reuses it', async () => {
  const gets: string[] = [];
  const full = {...savedReq('r1','https://r1.test/full'), body: 'big body', headers: [{key:'X',value:'1',enabled:true}]};
  const {v} = setup({get: async (url: string) => { gets.push(url); return full; }});
  v.requests = [{id:'r1',name:'r1',method:'GET',url:'https://r1.test/full',collection_id:null,workspace_id:'A',position:0}];
  const [a, b] = await Promise.all([v.ensureRequest('r1'), v.ensureRequest('r1')]);
  assert.equal(a, b);
  assert.equal(gets.length, 1, 'concurrent opens share one GET');
  assert.equal(await v.openRequest('r1'), true);
  assert.equal(v.draft.body, 'big body');
  assert.equal(v.isDirty(v.draft), false);
  await v.openRequest('r1'); // already open: focused, no refetch
  assert.equal(gets.length, 1);
});

test('a restored tab whose saved row is not loaded yet is never repurposed', () => {
  const {v} = setup();
  v.requests = [{id:'r9',name:'r9',method:'GET',url:'https://r9.test',collection_id:null,workspace_id:'A',position:0}];
  v.draft = {...v.draft, requestId: 'r9', url: 'https://r9.test/edited'};
  assert.equal(v.isDirty(v.draft), false, 'no dirty dot while unknown');
  const before = v.tabs.length;
  v.loadRequestIntoDraft(savedReq('r1','https://r1.test'));
  assert.equal(v.tabs.length, before + 1, 'opened beside, not over, the unknown tab');
});

// ── perf2 N3: coalesced, latched run-progress wakes ──────────────────────────

test('a final run_progress during an in-flight delta GET is latched, not dropped', async () => {
  const step = (name: string) => ({request_id:'r',name,status:200,duration_ms:1,ok:true,assertions:[],error:null});
  const run = {id:'run1',workspace_id:'A',automation_id:'auto',environment_id:null,created_by:'u',created_at:'',finished_at:null,stop_on_failure:false,dataset_rows:1,error:null,snapshot:null,result_rows:[],result_ids:[]};
  let calls = 0;
  let v: any;
  ({v} = setup({
    post: async () => ({...run,status:'running',report:{automation_id:'auto',steps:[],passed:false}}),
    get: async (url: string) => {
      if (!url.includes('/automation-runs/run1')) return [];
      calls++;
      if (calls === 1) {
        // The run finishes while this GET is in flight: no waiter is armed.
        v.noteRunProgress('run1', 'passed');
        return {...run,status:'running',report:{automation_id:'auto',steps:[step('one')],passed:false}};
      }
      return {...run,status:'passed',report:{automation_id:'auto',steps:[step('two')],passed:true}};
    },
  }));
  const t0 = Date.now();
  setTimeout(() => v.noteRunProgress('run1', 'running'), 5);
  const report = await v.runAutomation('auto');
  const took = Date.now() - t0;
  assert.equal(calls, 2);
  assert.equal(JSON.stringify(report.steps.map((s: any) => s.name)), '["one","two"]');
  assert.ok(took < 1500, `finished in ${took} ms (the 2 s fallback would be the dropped-event path)`);
});

test('run_progress wakes are coalesced to one delta GET per 250 ms', async () => {
  const run = {id:'run1',workspace_id:'A',automation_id:'auto',environment_id:null,created_by:'u',created_at:'',finished_at:null,stop_on_failure:false,dataset_rows:1,error:null,snapshot:null,result_rows:[],result_ids:[]};
  let calls = 0;
  let finished = false;
  const {v} = setup({
    post: async () => ({...run,status:'running',report:{automation_id:'auto',steps:[],passed:false}}),
    get: async (url: string) => {
      if (!url.includes('/automation-runs/run1')) return [];
      calls++;
      return {...run,status:finished ? 'passed' : 'running',report:{automation_id:'auto',steps:[],passed:finished}};
    },
  });
  const t0 = Date.now();
  const timer = setInterval(() => v.noteRunProgress('run1', 'running'), 2);
  setTimeout(() => { finished = true; clearInterval(timer); v.noteRunProgress('run1', 'passed'); }, 800);
  await v.runAutomation('auto');
  const took = Date.now() - t0;
  assert.ok(calls <= Math.ceil(took / 250) + 2, `${calls} GETs in ${took} ms`);
  assert.ok(took < 1500, `finished in ${took} ms`);
});

// ── perf2 N4: api_client_changed keeps the reuse window honest ──────────────

test('api_client_changed patches the loaded workspace and expires others', async () => {
  const gets: string[] = [];
  const full = {...savedReq('r2','https://agent.test'), name: 'Agent made', workspace_id: 'A', updated_at: 't2', position: 3};
  const {v, ws} = setup({get: async (url: string) => {
    gets.push(url);
    if (url.endsWith('/requests/r2')) return full;
    return [];
  }});
  await v.loadAll();
  assert.equal(v.requests.length, 0);
  v.noteClientChanged({workspace_id:'A',kind:'request',id:'r2',deleted:false});
  v.noteClientChanged({workspace_id:'A',kind:'request',id:'r2',deleted:false});
  await new Promise((r) => setTimeout(r, 400));
  assert.equal(gets.filter((g) => g.endsWith('/requests/r2')).length, 1, 'coalesced');
  assert.equal(JSON.stringify(v.requests.map((r: any) => [r.id, r.name, r.position])), '[["r2","Agent made",3]]');
  v.noteClientChanged({workspace_id:'A',kind:'request',id:'r2',deleted:true});
  await new Promise((r) => setTimeout(r, 400));
  assert.equal(v.requests.length, 0);
  // Another workspace's change: nothing fetched now; its next entry reloads.
  ws.currentId = 'B';
  await v.loadAll();
  const n = gets.length;
  v.noteClientChanged({workspace_id:'A',kind:'request',id:'r3',deleted:false});
  await new Promise((r) => setTimeout(r, 400));
  assert.equal(gets.length, n);
});

// ── Per-tab response slots ─────────────────────────────────────────────────

const okResp = (status: number, body = '') => ({status,status_text:'',headers:[],body,body_base64:'',truncated:false,too_large:false,duration_ms:1,size_bytes:body.length,content_type:null,trace:[]});

test('switching tabs keeps each tab its own response', async () => {
  let n = 0;
  const {v} = setup({post: async () => okResp(200 + n++)});
  v.draft = {...v.draft, url: 'https://a.test'};
  await v.execute();
  const first = v.draft.tabId;
  assert.equal(v.lastResponse.status, 200);
  v.openTab({...v.draft, url: 'https://b.test'});
  assert.equal(v.lastResponse, null, 'a new tab starts empty');
  await v.execute();
  assert.equal(v.lastResponse.status, 201);
  v.switchTab(0);
  assert.equal(v.draft.tabId, first);
  assert.equal(v.lastResponse.status, 200, 'switching back shows tab A’s response again');
  assert.equal(v.tabStatus(first), 'ok');
});

test('two tabs send concurrently; a background tab’s response is kept, never cancelled', async () => {
  const pending: Record<string, (r: unknown) => void> = {};
  const {v} = setup({post: (_u: string, body: any) => new Promise(r => { pending[body.url] = r; })});
  v.draft = {...v.draft, url: 'https://a.test'};
  const a = v.execute();
  const tabA = v.draft.tabId;
  v.openTab({...v.draft, url: 'https://b.test'});
  const b = v.execute();
  assert.equal(v.tabStatus(tabA), 'sending', 'sending in B did not cancel A');
  assert.equal(v.sending, true);
  pending['https://a.test'](okResp(404));   // A lands while B is in front
  await a;
  assert.equal(v.responses.get(tabA).resp.status, 404, 'background response kept');
  assert.equal(v.tabStatus(tabA), 'fail');
  pending['https://b.test'](okResp(200));
  await b;
  assert.equal(v.lastResponse.status, 200);
  v.switchTab(0);
  assert.equal(v.lastResponse.status, 404);
});

test('cancel only stops the active tab’s send', async () => {
  const signals: AbortSignal[] = [];
  const {v} = setup({post: (_u: string, _b: unknown, s: AbortSignal) => { signals.push(s); return new Promise(() => {}); }});
  v.draft = {...v.draft, url: 'https://a.test'};
  void v.execute();
  const tabA = v.draft.tabId;
  v.openTab({...v.draft, url: 'https://b.test'});
  void v.execute();
  await Promise.resolve();
  v.cancelExecute();
  assert.equal(v.sending, false);
  assert.equal(v.tabStatus(tabA), 'sending');
  assert.equal(signals[0].aborted, false);
});

test('opening a history entry shows its stored response, flagged as from history', () => {
  const {v} = setup();
  v.loadHistoryIntoDraft({id:'h1',method:'POST',url:'https://r.test/x',status:201,duration_ms:12,executed_at:'2026-10-01T10:00:00Z',
    request:{method:'POST',url:'https://r.test/x',headers:[],query:[],auth:{type:'none'}},
    response:{status:201,status_text:'Created',headers:[{key:'Content-Type',value:'application/json'}],body:'{"id":1}',truncated:true,body_id:'stale'}});
  assert.equal(v.lastResponse.status, 201);
  assert.equal(v.lastResponse.body, '{"id":1}');
  assert.equal(v.lastResponse.body_id, null, 'the expired raw-body id is not reused');
  assert.equal(v.lastResponse.content_type, 'application/json');
  assert.equal(v.responseFromHistory.at, '2026-10-01T10:00:00Z');
  assert.equal(v.responseFromHistory.truncated, true);
});

test('api_history_appended refetches only a history list someone asked for (perf H1)', async () => {
  const gets: string[] = [];
  const {v} = setup({get: async (url: string) => { gets.push(url); return []; }});
  const summaries = () => gets.filter((g) => g.includes('/history/summaries')).length;
  // Loaded by another surface (⌘K, an agent UI command): no list on screen.
  for (const id of ['h1', 'h2', 'h3']) v.noteHistoryAppended('A', id);
  await new Promise((r) => setTimeout(r, 250));
  assert.equal(summaries(), 0, 'an automation run must not refetch history per step');
  await v.loadHistory();
  assert.equal(summaries(), 1);
  v.noteHistoryAppended('A', 'h4');
  await new Promise((r) => setTimeout(r, 250));
  assert.equal(summaries(), 2, 'the History list stays live once loaded');
});

for (const code of ["console.log('done')", "pm.variables.set('a','one')", "pm.variables.unset('remove')"]) {
  test(`concurrent script writes preserve newer unrelated variables: ${code}`, async () => {
    let release!: () => void;
    const pending = new Promise<void>(r => {release=r;});
    const {v} = setup({post:async()=>({headers:[],status:200,status_text:'OK',body:'',duration_ms:1})}, async input => {
      if (input.code === code) await pending;
      const run = scriptRuntime.runPostResponse(input.code,input.response,input.vars);
      return {run,vars:input.vars};
    });
    v.setRuntimeVar('token','old');v.setRuntimeVar('remove','yes');
    v.draft={...v.draft,url:'https://slow.test',post_response_script:code};
    const slow=v.execute();
    v.newDraft();v.draft={...v.draft,url:'https://fast.test',post_response_script:"pm.variables.set('token','new'); pm.variables.set('b','two')"};
    await v.execute();v.setRuntimeVar('manual','kept');release();await slow;
    assert.equal(v.runtimeVars.token,'new');assert.equal(v.runtimeVars.b,'two');assert.equal(v.runtimeVars.manual,'kept');
    if(code.includes('unset')) assert.equal(v.runtimeVars.remove,undefined);
    if(code.includes("set('a'")) assert.equal(v.runtimeVars.a,'one');
  });
}
