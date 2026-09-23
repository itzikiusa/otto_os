import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {randomUUID} from 'node:crypto';
import ts from 'typescript';
import { HistoryRefresh, HistoryDetail } from '../src/lib/stores/apiHistory.ts';
import * as secretShapes from '../src/lib/api/apiSecretShapes.ts';

function setup(overrides: Record<string, unknown> = {}, runScript?: (...args: any[]) => Promise<any>) {
  const ws = {currentId: 'A'};
  const api = {get: async () => [], patch: async () => ({}), post: async () => ({}), ...overrides};
  const context = {exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, {snapshot: (v: unknown) => v}), $derived: (v: unknown) => v,
    crypto: {randomUUID}, URL, AbortController, DOMException, setTimeout, clearTimeout,
    localStorage: {getItem() {return null;},setItem() {}},
    require: (p: string) => p.endsWith('/client') ? {api, isAbortError: () => false}
      : p.includes('workspace.svelte') ? {ws}
      : p.includes('toast') ? {toasts: {error() {},success() {},info() {}}}
      : p.endsWith('/apiHistory') ? {HistoryRefresh, HistoryDetail}
      : p.endsWith('/apiSecretShapes') ? secretShapes
      : p.endsWith('/scriptRunner') ? {runScript}
      : p.endsWith('/scripts') ? {runPreRequest: () => ({logs:[],tests:[]})}
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
  const {v}=setup({post:async()=>{sent++;return {headers:[],status:200};}},async input=>++scripts===1?slow:{run:{logs:[],tests:[]},request:input.request,vars:{current:'yes'}});
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

test('history replay rehydrates masked credentials from the saved request, never sends ***', () => {
  const {v} = setup();
  const marker = {$secret:'otto.api.request.r1'};
  v.requests = [savedReq('r1','https://r1.test/x',{type:'bearer',token:marker})];
  v.loadHistoryIntoDraft({id:'h1',method:'GET',url:'https://r1.test/x',
    request:{request_id:'r1',method:'GET',url:'https://r1.test/x',headers:[],query:[],auth:{type:'bearer',token:'***'}}});
  assert.deepEqual(v.draft.auth, {type:'bearer',token:marker});
  // Unknown origin: blanked instead of replaying the mask.
  v.newDraft();
  v.loadHistoryIntoDraft({id:'h2',method:'GET',url:'https://other.test/',
    request:{method:'GET',url:'https://other.test/',headers:[{key:'Authorization',value:'***',enabled:true}],query:[],auth:{type:'bearer',token:'***'}}});
  assert.deepEqual(v.draft.auth, {type:'bearer',token:''});
  assert.equal(v.draft.headers[0].value, '');
  assert.equal(v.draft.headers[0].enabled, false);
});
