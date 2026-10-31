///<reference types="svelte" />
;// Golden tasks: the per-repo evaluation corpus + saved regression cases. Each
// task is a reusable prompt paired with the commands that decide whether the
// result is good (test/lint/build) and an optional rubric. From here you can
// add/edit/delete a task and kick off a single score-only run against the
// workspace's working tree — the result opens in the run detail via `onopenrun`.

import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import type { GoldenTask, GoldenTaskReq, SkillEval } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { toastError } from '../../lib/toastError';

;type $$ComponentProps =  { onopenrun?: (e: SkillEval) => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopenrun }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let tasks: GoldenTask[] = $state([]);
  let loading = $state(true);
  // Inline load failure + Retry (a failed load used to fall through to the
  // "No golden tasks" empty state, which reads as "you have none").
  let loadError: string | null = $state(null);
  let runningId: string | null = $state(null);

  // Inline create/edit form. `editingId === null` while creating.
  let showForm = $state(false);
  let editingId: string | null = $state(null);
  let saving = $state(false);
  let formGeneration = 0;
  let fName = $state('');
  let fPrompt = $state('');
  let fSkill = $state('');
  let fTest = $state('');
  let fLint = $state('');
  let fBuild = $state('');
  let fRubric = $state('');
  let fTags = $state('');
  let fEnabled = $state(true);
  let initialSnapshot = $state('');

  function currentSnapshot(): string {
    return JSON.stringify([
      (fName ?? '').trim(),
      (fPrompt ?? '').trim(),
      (fSkill ?? '').trim(),
      (fTest ?? '').trim(),
      (fLint ?? '').trim(),
      (fBuild ?? '').trim(),
      (fRubric ?? '').trim(),
      (fTags ?? '').trim(),
      fEnabled ?? true,
    ]);
  }

  function isFormDirty(): boolean {
    if (!showForm) return false;
    return currentSnapshot() !== initialSnapshot;
  }

  // Reload whenever the active workspace changes.
  $effect(() => {
    const wsId = ws.currentId;
    // An open edit form belongs to the previous workspace's task.
    showForm = false;
    editingId = null;
    if (wsId) {
      void load(wsId);
    } else {
      tasks = [];
      loading = false;
    }
  });

  let listGeneration = 0;
  async function load(wsId: string): Promise<void> {
    const generation = ++listGeneration;
    loading = true;
    loadError = null;
    try {
      const next = await skillsEvalApi.listGolden(wsId);
      if (ws.currentId !== wsId || generation !== listGeneration) return;
      tasks = next;
    } catch (e) {
      if (ws.currentId === wsId && generation === listGeneration) loadError = loadErrorText(e);
    } finally {
      if (ws.currentId === wsId && generation === listGeneration) loading = false;
    }
  }

  // Group by repo so a workspace with several repos reads cleanly.
  const groups = $derived.by(() => {
    const map = new Map<string, GoldenTask[]>();
    for (const t of tasks) {
      const key = t.repo_key || '';
      const arr = map.get(key) ?? [];
      arr.push(t);
      map.set(key, arr);
    }
    return [...map.entries()].map(([repoKey, items]) => ({ repoKey, items }));
  });

  function repoLabel(key: string): string {
    if (!key) return 'Unknown repository';
    return key === ws.currentId ? 'This workspace' : key;
  }

  function truncate(s: string, n = 140): string {
    return s.length > n ? `${s.slice(0, n).trimEnd()}…` : s;
  }

  function resetFields(): void {
    formGeneration++;
    fName = '';
    fPrompt = '';
    fSkill = '';
    fTest = '';
    fLint = '';
    fBuild = '';
    fRubric = '';
    fTags = '';
    fEnabled = true;
    initialSnapshot = currentSnapshot();
  }

  async function confirmDiscardForm(): Promise<boolean> {
    if (!isFormDirty()) return true;
    return confirmer.ask('You have unsaved changes to this golden task. Discard them?', {
      title: 'Discard unsaved changes?',
      confirmLabel: 'Discard',
      cancelLabel: 'Keep editing',
      danger: true,
    });
  }

  async function toggleCreate(): Promise<void> {
    if (showForm && editingId === null) {
      await cancelForm();
      return;
    }
    if (!(await confirmDiscardForm())) return;
    editingId = null;
    resetFields();
    showForm = true;
  }

  async function openEdit(t: GoldenTask): Promise<void> {
    if (editingId !== t.id && !(await confirmDiscardForm())) return;
    formGeneration++;
    editingId = t.id;
    fName = t.name;
    fPrompt = t.prompt;
    fSkill = t.skill;
    fTest = t.test_cmd;
    fLint = t.lint_cmd;
    fBuild = t.build_cmd ?? '';
    fRubric = t.rubric;
    fTags = (t.tags ?? []).join(', ');
    fEnabled = t.enabled ?? true;
    initialSnapshot = currentSnapshot();
    showForm = true;
  }

  function closeForm(): void {
    showForm = false;
    editingId = null;
    resetFields();
  }

  async function cancelForm(): Promise<void> {
    if (!(await confirmDiscardForm())) return;
    closeForm();
  }

  async function submit(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || saving) return;
    // Save stays disabled until both are filled (see canSave).
    if (!fName.trim() || !fPrompt.trim()) return;
    const id = editingId;
    const generation = formGeneration;
    saving = true;
    const rawBuild = typeof fBuild === 'string' ? fBuild.trim() : '';
    // Deduped: the list keys chips by position, but a repeated tag is still noise.
    const rawTags = typeof fTags === 'string' ? [...new Set(fTags.split(',').map((s) => s.trim()).filter(Boolean))] : undefined;
    const body: GoldenTaskReq = {
      name: fName.trim(),
      prompt: fPrompt.trim(),
      skill: fSkill.trim() || undefined,
      test_cmd: fTest.trim() || undefined,
      lint_cmd: fLint.trim() || undefined,
      ...(rawBuild ? { build_cmd: rawBuild } : {}),
      rubric: fRubric.trim() || undefined,
      ...(rawTags && rawTags.length > 0 ? { tags: rawTags } : {}),
      ...(id && typeof fEnabled === 'boolean' ? { enabled: fEnabled } : {}),
    };
    try {
      if (id) {
        const updated = await skillsEvalApi.updateGolden(id, body);
        if (ws.currentId !== wsId) return;
        tasks = tasks.map((t) => (t.id === updated.id ? updated : t));
        toasts.success('Golden task updated', updated.name);
      } else {
        const created = await skillsEvalApi.createGolden(wsId, body);
        if (ws.currentId !== wsId) return;
        tasks = [created, ...tasks];
        toasts.success('Golden task added', created.name);
      }
      if (generation === formGeneration) closeForm();
    } catch (e) {
      toastError('Couldn’t save the golden task', e);
    } finally {
      saving = false;
    }
  }

  async function run(t: GoldenTask): Promise<void> {
    if (runningId) return;
    const workspace = ws.currentId;
    runningId = t.id;
    try {
      const result = await skillsEvalApi.runGolden(t.id, {
        mode: 'score_only',
        target: { kind: 'working' },
      });
      if (ws.currentId !== workspace) return;
      onopenrun?.(result);
      toasts.success('Evaluation started', `Scoring “${t.name}” against the working tree.`);
    } catch (e) {
      toastError('Couldn’t run the golden task', e);
    } finally {
      runningId = null;
    }
  }

  async function remove(t: GoldenTask): Promise<void> {
    if (!(await confirmer.ask(`Delete golden task “${t.name}”? This cannot be undone.`, { title: 'Delete golden task' }))) return;
    try {
      await skillsEvalApi.deleteGolden(t.id);
      tasks = tasks.filter((x) => x.id !== t.id);
      if (editingId === t.id) closeForm();
      toasts.success('Golden task deleted', t.name);
    } catch (e) {
      toastError('Couldn’t delete the golden task', e);
    }
  }

  const canSave = $derived(!saving && fName.trim().length > 0 && fPrompt.trim().length > 0);
;
async () => {

 { svelteHTML.createElement("div", {   "class":`gt`,...__sveltets_2_empty({"data-testid":`golden-tasks`}),});
  
   { svelteHTML.createElement("header", { "class":`gt-head`,});
     { svelteHTML.createElement("p", { "class":`gt-hint`,});
                    
          
     }
    
    if(tasks.length > 0 || showForm){
       { svelteHTML.createElement("button", {            "class":`btn small`,...__sveltets_2_empty({"data-testid":`golden-new-btn`}),"onclick":() => void toggleCreate(),"disabled":!ws.currentId,"aria-expanded":showForm && editingId === null,});!showForm;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   
       }
    }
   }

  if(showForm){
     { svelteHTML.createElement("form", {   "class":`gt-form card`,"onsubmit":(e: SubmitEvent) => { e.preventDefault(); void submit(); },});
       { svelteHTML.createElement("div", { "class":`gt-form-head`,});
         { svelteHTML.createElement("h3", { "class":`gt-form-title`,});editingId ? 'Edit golden task' : 'New golden task'; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {         "type":`button`,"class":`icon-btn`,"onclick":() => void cancelForm(),"title":`Close form`,"aria-label":`Close form`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
         }
       }

       { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-name`,});  }
       { svelteHTML.createElement("input", {            "dir":`auto`,"id":`gt-name`,"class":`input`,...__sveltets_2_empty({"data-testid":`golden-name`}),"placeholder":`e.g. Add a player bonus-balance endpoint`,"bind:value":fName,});/*Ωignore_startΩ*/() => fName = __sveltets_2_any(null);/*Ωignore_endΩ*/}

       { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-prompt`,});  }
       { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`gt-prompt`,"class":`input`,"rows":3,...__sveltets_2_empty({"data-testid":`golden-prompt`}),"placeholder":`What the agent should implement…`,"bind:value":fPrompt,});/*Ωignore_startΩ*/() => fPrompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }

       { svelteHTML.createElement("div", { "class":`gt-grid`,});
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-skill`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`gt-skill`,"class":`input`,"placeholder":`skill name (optional)`,"bind:value":fSkill,});/*Ωignore_startΩ*/() => fSkill = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-test`,});  }
           { svelteHTML.createElement("input", {            "dir":`auto`,"id":`gt-test`,"class":`input`,...__sveltets_2_empty({"data-testid":`golden-test-cmd`}),"placeholder":`cargo test · npm test`,"bind:value":fTest,});/*Ωignore_startΩ*/() => fTest = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-lint`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`gt-lint`,"class":`input`,"placeholder":`cargo clippy · npm run check`,"bind:value":fLint,});/*Ωignore_startΩ*/() => fLint = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-build`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`gt-build`,"class":`input`,"placeholder":`cargo build · npm run build (optional)`,"bind:value":fBuild,});/*Ωignore_startΩ*/() => fBuild = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-tags`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`gt-tags`,"class":`input`,"placeholder":`regression, auth (optional)`,"bind:value":fTags,});/*Ωignore_startΩ*/() => fTags = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }

       { svelteHTML.createElement("label", {   "class":`field-label`,"for":`gt-rubric`,});  }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`gt-rubric`,"class":`input`,"rows":2,"placeholder":`How to judge a passing result (optional)`,"bind:value":fRubric,});/*Ωignore_startΩ*/() => fRubric = __sveltets_2_any(null);/*Ωignore_endΩ*/ }

       { svelteHTML.createElement("div", { "class":`gt-form-actions`,});
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small ghost`,"onclick":() => void cancelForm(),});  }
         { svelteHTML.createElement("button", {     "class":`btn small primary`,...__sveltets_2_empty({"data-testid":`golden-save`}),"disabled":!canSave,});
          saving ? 'Saving…' : editingId ? 'Save changes' : 'Add golden task';
         }
       }
     }
  }

   { svelteHTML.createElement("div", { "class":`gt-body`,});
    if(!ws.currentId){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`zap`,"title":`No workspace selected`,"body":`Pick a workspace to manage its golden tasks.`,}});}
    } else if ((loading || loadError) && tasks.length === 0){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         "what":`golden tasks`,loading,"error":loadError,"empty":true,"onretry":() => ws.currentId && load(ws.currentId),}});}
    } else if (tasks.length === 0){
      if(!showForm){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {             "icon":`target`,"title":`No golden tasks yet`,"body":`A golden task is a repo-specific prompt plus a test command — a fixed case to score skills against, run after run.`,"actionLabel":`New golden task`,"actionIcon":`plus`,"onaction":toggleCreate,}});}
      }
    }else{
         for(let g of __sveltets_2_ensureArray(groups)){g.repoKey;
         { svelteHTML.createElement("section", { "class":`gt-group`,});
           { svelteHTML.createElement("h3", { "class":`gt-group-head`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
            repoLabel(g.repoKey);
             { svelteHTML.createElement("span", { "class":`gt-count`,});g.items.length; }
           }
             for(let t of __sveltets_2_ensureArray(g.items)){t.id;
             { svelteHTML.createElement("article", {    "class":`gt-card`,...__sveltets_2_empty({"data-testid":`golden-card`}),});!t.enabled;
               { svelteHTML.createElement("div", { "class":`gt-card-top`,});
                 { svelteHTML.createElement("span", {   "class":`gt-name`,"title":t.name,});t.name; }
                if(t.origin === 'regression'){
                   { svelteHTML.createElement("span", {   "class":`gt-badge regression`,...__sveltets_2_empty({"data-testid":`golden-regression-badge`}),});  }
                }
                if(!t.enabled){ { svelteHTML.createElement("span", { "class":`gt-badge muted`,});  }}
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("div", { "class":`gt-actions`,});
                   { svelteHTML.createElement("button", {           "class":`btn small gt-run`,...__sveltets_2_empty({"data-testid":`golden-run`}),"onclick":() => run(t),"disabled":runningId !== null,"title":runningId !== null && runningId !== t.id ? 'Another golden task is starting…' : 'Score this task against the working tree',});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} runningId === t.id ? 'Running…' : 'Run';
                   }
                   { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => openEdit(t),});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} 
                   }
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => remove(t),"title":`Delete golden task`,"aria-label":`Delete golden task ${t.name}`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                   }
                 }
               }

               { svelteHTML.createElement("div", { "class":`gt-meta`,});
                if(t.skill){ { svelteHTML.createElement("span", { "class":`gt-pill`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});} t.skill; }}
                if(t.test_cmd){ { svelteHTML.createElement("code", {   "class":`gt-code`,"title":t.test_cmd,});t.test_cmd; }}
               }

              if(t.prompt){ { svelteHTML.createElement("p", { "class":`gt-prompt`,});truncate(t.prompt); }}

              if(t.tags.length > 0){
                 { svelteHTML.createElement("div", { "class":`gt-tags`,});
                      for(let tag of __sveltets_2_ensureArray(t.tags)){let i = 1;i;
                     { svelteHTML.createElement("span", { "class":`gt-tag`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});} tag; }
                  }
                 }
              }
             }
          }
         }
      }
    }
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GoldenTasksView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GoldenTasksView__SvelteComponent_ = ReturnType<typeof GoldenTasksView__SvelteComponent_>;
/*Ωignore_endΩ*/export default GoldenTasksView__SvelteComponent_;