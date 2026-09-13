import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {randomUUID} from 'node:crypto';
import ts from 'typescript';

function setup(overrides: Record<string, unknown> = {}) {
  const ws = {currentId: 'A'};
  const api = {get: async () => [], patch: async () => ({}), post: async () => ({}), ...overrides};
  const context = {exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, {snapshot: (v: unknown) => v}), $derived: (v: unknown) => v,
    crypto: {randomUUID}, URL, AbortController, setTimeout, clearTimeout,
    localStorage: {getItem() {return null;},setItem() {}},
    require: (p: string) => p.endsWith('/client') ? {api, isAbortError: () => false}
      : p.includes('workspace.svelte') ? {ws}
      : p.includes('toast') ? {toasts: {error() {},success() {}}}
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
