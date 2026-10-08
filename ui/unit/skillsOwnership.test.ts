import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';
const quiet = { success() {}, info() {} };

test('file deletion confirmation keeps the skill shown in the question', async () => {
  const answer = deferred<boolean>(); const calls: string[] = [];
  const state = componentFunctions(new URL('../src/modules/skills-lab/SkillEditor.svelte', import.meta.url), ['deleteFile'], {
    name: 'original', source: 'library', currentFile: 'SKILL.md', loadGeneration: 1, disposed: false,
    confirmer: { ask: () => answer.promise }, skillLabApi: { deleteFile: async (n: string) => calls.push(n), listFiles: async () => [] },
    onsaved() {}, open() {}, toastError() {},
  });
  const deleting = state.deleteFile('notes.md'); state.name = 'replacement'; state.loadGeneration++;
  answer.resolve(true); await deleting;
  assert.equal(calls.includes('replacement'), false);
});

test('save completing after selecting another skill cannot replace its body or files', async () => {
  const response = deferred<unknown>(); let published = 0;
  const state = componentFunctions(new URL('../src/modules/skills-lab/SkillEditor.svelte', import.meta.url), ['save'], {
    editable: true, saving: false, loading: false, loadError: null, binary: false, dirty: true,
    name: 'original', source: 'library', currentFile: 'SKILL.md', content: 'old submitted', original: 'old', loadGeneration: 1, disposed: false,
    skillLabApi: { putFile: () => response.promise }, onsaved() { published++; }, toasts: quiet, toastError() {},
  });
  const saving = state.save(); state.name = 'replacement'; state.loadGeneration++; state.original = 'new';
  response.resolve([]); await saving;
  assert.equal(published, 0); assert.equal(state.original, 'new');
});

test('review stop confirmation never targets a newly selected review', async () => {
  const answer = deferred<boolean>(); const calls: string[] = [];
  const state = componentFunctions(new URL('../src/modules/skills-lab/SkillReviewPanel.svelte', import.meta.url), ['cancelReview'], {
    selected: { id: 'original', skill_name: 'A' }, wsId: 'workspace', disposed: false, detailGeneration: 1,
    confirmer: { ask: () => answer.promise }, skillReviewApi: { cancel: async (id: string) => { calls.push(id); return { id }; } },
    loadList: async () => {}, toastError() {},
  });
  const stopping = state.cancelReview(); state.selected = { id: 'replacement' }; state.detailGeneration++;
  answer.resolve(true); await stopping; assert.equal(calls.includes('replacement'), false);
});

test('review fixer completion cannot replace a newly selected review', async () => {
  const response = deferred<unknown>();
  const state = componentFunctions(new URL('../src/modules/skills-lab/SkillReviewPanel.svelte', import.meta.url), ['applyFixes'], {
    selected: { id: 'original' }, wsId: 'workspace', disposed: false, detailGeneration: 1,
    applying: false, fixProvider: 'fixture', fixInstructions: '', fixTermOpen: false,
    skillReviewApi: { apply: () => response.promise }, toasts: quiet, toastError() {},
  });
  const fixing = state.applyFixes(); state.selected = { id: 'replacement' }; state.detailGeneration++;
  response.resolve({ id: 'original' }); await fixing;
  assert.equal(state.selected.id, 'replacement'); assert.equal(state.fixTermOpen, false);
});

for (const file of ['skills-eval/SkillsEvalPage', 'skills-lab/SkillReviewPanel']) {
  test(`${file}: a late workspace list cannot publish into the new workspace`, async () => {
    const response = deferred<any>();
    const state = componentFunctions(new URL(`../src/modules/${file}.svelte`, import.meta.url), ['loadList'], {
      ws: { currentId: 'A' }, wsId: 'A', loadedWs: 'A', disposed: false, listGeneration: 0,
      loading: false, loadError: null, listLoading: false, listError: null, runs: [], reviews: [], nextCursor: null,
      selectedId: null, formSkill: null, PAGE: 50, mode: 'form',
      skillsEvalApi: { listSummaries: () => response.promise }, skillReviewApi: { list: () => response.promise }, loadErrorText: String,
    });
    const loading = state.loadList('A'); state.ws.currentId = 'B'; state.wsId = 'B';
    response.resolve(file.includes('skills-eval') ? { items: [{id:'old'}],next_cursor:null } : [{id:'old'}]); await loading;
    assert.equal(state.runs.length + state.reviews.length, 0);
  });
}

test('library replacement and removal cannot switch targets while confirmation is open', async () => {
  for (const action of ['install','remove']) {
    const answer=deferred<boolean>();const targets:string[]=[];
    const state=componentFunctions(new URL('../src/modules/skills-lab/SkillDetail.svelte',import.meta.url),[action],{
      group:{name:'original',variants:[{source:'bundled',bundledVersion:1}]},source:'library',hasLibrary:true,
      selectionKey:'library:original',disposed:false,loadGeneration:1,busy:false,
      confirmer:{ask:()=>answer.promise},skillLabApi:{install:async(n:string)=>targets.push(n),remove:async(n:string)=>targets.push(n)},
      toasts:quiet,toastError(){},onchanged(){},ondeleted(){},sourceLabel:String,
    });
    const pending=state[action]();state.group={name:'replacement',variants:[]};state.selectionKey='library:replacement';state.loadGeneration++;
    answer.resolve(true);await pending;
    assert.equal(targets.includes('replacement'),false,action);
  }
});

test('provider comparison cannot cache old text under a different selected skill', async()=>{
  const response=deferred<any>();let published=0;
  const state=componentFunctions(new URL('../src/modules/skills-lab/SkillDetail.svelte',import.meta.url),['compare'],{
    group:{name:'original',reference:'library'},selectionKey:'library:original',disposed:false,loadGeneration:1,
    comparing:null,tab:'overview',compareError:null,bodyOf:()=>undefined,skillLabApi:{getProvider:()=>response.promise},
    onbody(){published++;},sourceLabel:String,loadErrorText:String,
  });
  const pending=state.compare('codex');state.group={name:'replacement',reference:'library'};state.selectionKey='library:replacement';state.loadGeneration++;
  response.resolve({body:'original body'});await pending;assert.equal(published,0);
});

test('unchanged save acknowledges only submitted text while newer typing stays dirty',async()=>{
  const response=deferred<any>();let body='';
  const state=componentFunctions(new URL('../src/modules/skills-lab/SkillEditor.svelte',import.meta.url),['save'],{
    editable:true,saving:false,loading:false,loadError:null,binary:false,dirty:true,name:'same',source:'library',currentFile:'SKILL.md',
    content:'submitted',original:'old',loadGeneration:1,disposed:false,skillLabApi:{putFile:()=>response.promise},
    onsaved(_files:unknown,md:string){body=md;},toasts:quiet,toastError(){},
  });
  const pending=state.save();state.content='newer typing';response.resolve([]);await pending;
  assert.equal(state.original,'submitted');assert.equal(state.content,'newer typing');assert.equal(body,'submitted');
});

for (const [component,handler,method,field] of [
  ['GoldenTasksView','load','listGolden','tasks'], ['MatrixView','loadList','listMatrices','matrices'],
]) test(`${component} list completion keeps workspace ownership`,async()=>{
  const response=deferred<any>();const state=componentFunctions(new URL(`../src/modules/skills-eval/${component}.svelte`,import.meta.url),[handler],{
    ws:{currentId:'A'},listGeneration:0,loading:false,loadError:null,tasks:[],matrices:[],showForm:false,selectedId:null,
    skillsEvalApi:{[method]:()=>response.promise},loadErrorText:String,selectMatrix(){},
  });
  const pending=state[handler]('A');state.ws.currentId='B';response.resolve([{id:'foreign'}]);await pending;
  assert.equal(state[field].length,0);
});

test('matrix stop confirmation cannot cancel the next matrix',async()=>{
  const answer=deferred<boolean>();const targets:string[]=[];
  const state=componentFunctions(new URL('../src/modules/skills-eval/MatrixView.svelte',import.meta.url),['cancel'],{
    ws:{currentId:'A'},selected:{id:'old',name:'Old'},selectedId:'old',confirmer:{ask:()=>answer.promise},
    skillsEvalApi:{cancelMatrix:async(id:string)=>{targets.push(id);return{id};}},syncListEntry(){},toastError(){},
  });
  const pending=state.cancel();state.selected={id:'new',name:'New'};state.selectedId='new';answer.resolve(true);await pending;
  assert.equal(targets.includes('new'),false);
});

test('creating golden task after workspace switch cannot close the new form or publish old task',async()=>{
  const response=deferred<any>();let closed=0;
  const state=componentFunctions(new URL('../src/modules/skills-eval/GoldenTasksView.svelte',import.meta.url),['submit'],{
    ws:{currentId:'A'},saving:false,editingId:null,formGeneration:0,fName:'Old',fPrompt:'Old prompt',fSkill:'',fTest:'',fLint:'',fRubric:'',tasks:[],
    skillsEvalApi:{createGolden:()=>response.promise},closeForm(){closed++;},toasts:quiet,toastError(){},
  });
  const pending=state.submit();state.ws.currentId='B';response.resolve({id:'foreign',name:'Old'});await pending;
  assert.equal(state.tasks.length,0);assert.equal(closed,0);
});
