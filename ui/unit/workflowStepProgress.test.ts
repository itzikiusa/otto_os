import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
const file=new URL('../src/modules/workflows/stepProgress.ts',import.meta.url);
function mod():Record<string,any> {
  const context={exports:{} as Record<string,any>,require:()=>({})};
  runInNewContext(ts.transpileModule(readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context);
  return context.exports;
}
const agent=(status:string,session:string|null='s')=>({name:'a',provider:'claude',model:'',status,note:'',comment_count:0,session_id:session});
const review=(id:string,status:string,agents:any[])=>({id,repo_id:`r-${id}`,pr_number:0,status,error:null,comments:[],agents,created_at:''});

test('the user report: repo 1 done, repo 2 reviewers queued behind the shared slots',()=>{
  const {stepProgress}=mod();
  const one=review('1','done',[...Array(12)].map(()=>agent('done')).concat([agent('done',null)]));
  const two=review('2','running',[...Array(12)].map(()=>agent('pending',null)).concat([agent('pending',null)]));
  const p=stepProgress([one,two]);
  assert.equal(p.phase,'Queued');
  assert.equal(p.detail,'1/2 repos · 13/26 agents done');
  assert.match(p.waitingOn,/reviewer slot/);
  assert.equal(p.settled,false);
});
test('reviewers done → waiting for the summarizer; all settled while running → Finishing',()=>{
  const {stepProgress}=mod();
  let p=stepProgress([review('1','running',[agent('done'),agent('error'),agent('pending',null)])]);
  assert.equal(p.phase,'Summarizing'); assert.equal(p.waitingOn,'the summarizer'); assert.equal(p.settled,false);
  p=stepProgress([review('1','running',[agent('done'),agent('done'),agent('done',null)])]);
  assert.equal(p.phase,'Finishing'); assert.equal(p.settled,true);
  p=stepProgress([review('1','running',[agent('running'),agent('pending',null),agent('pending',null)])]);
  assert.equal(p.phase,'Reviewing'); assert.match(p.waitingOn,/1 reviewing · 1 queued/);
  assert.equal(stepProgress([]),null);
});
test('stuck only after the grace with every child settled',()=>{
  const {settledSince,isStuck,STUCK_AFTER_MS}=mod();
  let since=settledSince(null,true,1000); assert.equal(since,1000);
  since=settledSince(since,true,5000); assert.equal(since,1000);
  assert.equal(isStuck(since,1000+STUCK_AFTER_MS-1),false);
  assert.equal(isStuck(since,1000+STUCK_AFTER_MS),true);
  assert.equal(settledSince(since,false,9e9),null);
});
