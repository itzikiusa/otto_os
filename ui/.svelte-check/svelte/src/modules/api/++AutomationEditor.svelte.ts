///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// One automation, edited in the main area: an ordered list of saved requests
// ("steps"). Each step can CHECK its response (assertions) and SAVE values
// from it into variables that later steps use as {{name}}. Edits are a
// working copy until Save; Run saves first, then runs in the daemon and
// polls the saved report.
import { toastError } from '../../lib/toastError';
import { router } from '../../lib/router.svelte';
import { editableAssertion, editableExtract, editableSteps, movedArrow, rowKey, savedSteps, type EditableStep } from './automationInput';
import { KIND_LABEL, NO_VALUE_OPS, OP_LABEL, RUN_STATUS } from './automationLabels';
import { focusAfterRowRemoval, rowListOf } from '../../lib/rowFocus';
import { announce, announceShown } from '../../lib/announce.svelte';
import { tick } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import MethodTag from './MethodTag.svelte';
import StatusChip from './StatusChip.svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { formatSeconds } from '../../lib/metric-format';
import type { ApiAutomationRun, ApiAssertion, ApiAutomation, ApiExtract, Id } from '../../lib/api/types';
;

  interface Props {
    automationId: Id;
    /** The automation was deleted (the page goes back to the request view). */
    ondeleted?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { automationId, ondeleted }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  const automation = $derived(apiClient.automations.find((a) => a.id === automationId) ?? null);

  // Working copy (edits aren't live until Save).
  let steps: EditableStep[] = $state([]);
  let stepsEl = $state<HTMLOListElement | null>(null);
  let dirty = $state(false);
  let environmentId = $state('');
  let stopOnFailure = $state(false);
  let datasetText = $state('');
  let historyRun = $state<ApiAutomationRun | null>(null);
  let loadedFor = $state<string | null>(null);
  let leavePending: Promise<boolean> | null = null;
   function approveLeave(): Promise<boolean> {
    if (!dirty) return Promise.resolve(true);
    if (leavePending) return leavePending;
    leavePending = (async () => {
      const { value } = await confirmer.choose('This automation has unsaved changes.', {
        title: 'Save automation changes?',
        options: [{label:'Save', value:'save', kind:'primary'}, {label:'Discard', value:'discard', kind:'danger'}],
      });
      if (value === 'save') return save();
      if (value !== 'discard') return false;
      dirty = false;
      return true;
    })().finally(() => { leavePending = null; });
    return leavePending;
  }
  $effect(() => router.guard(() => approveLeave()));

  /** Run-list rows are summaries (no steps / snapshot): load the whole run. */
  async function openHistoryRun(id: string): Promise<void> {
    const run = await apiClient.getAutomationRun(id);
    if (run && run.automation_id === automationId) historyRun = run;
  }

  function loadInto(a: ApiAutomation): void {
    loadedFor = a.id;
    steps = editableSteps(a.steps);
    dirty = false;
    historyRun = null;
    datasetText = '';
    void apiClient.loadAutomationRuns(a.id);
  }
  $effect(() => {
    const a = automation;
    if (a && a.id !== loadedFor) loadInto(a);
  });

  // Id → request, built once per list change (a per-step `.find` re-scanned
  // every saved request for every step on every render).
  const reqById = $derived(new Map(apiClient.requests.map((r) => [r.id, r])));
  const req = (id: Id) => reqById.get(id);
  const reqLabel = (r: { name: string; method: string; url: string }) => `${r.name} · ${r.method} ${r.url}`;

  // One shared, filterable picker instead of a <select> per step holding every
  // saved request as an <option> (20 steps × 3k requests = 60k nodes, all
  // re-diffed on every requests change). The menu searches name/method/url and
  // renders at most 50 matches at a time.
  function pickRequest(e: MouseEvent, i: number): void {
    if (!canEdit) return;
    const current = steps[i]?.request_id;
    ctxMenu.show(
      e,
      apiClient.requests.map((q) => ({
        label: reqLabel(q),
        checked: q.id === current,
        action: () => { if (steps[i] && steps[i].request_id !== q.id) patchStep(i, { request_id: q.id }); },
      })),
      { filter: true, filterPlaceholder: 'Search saved requests', maxVisible: 50 },
    );
  }

  // Run report: render the first STEP_PAGE step results, more on demand (a
  // data-driven run can report 1,000 steps, each re-diffed on every poll).
  const STEP_PAGE = 100;
  let reportLimit = $state(STEP_PAGE);
  // The (large) snapshot JSON is only built while its disclosure is open.
  let snapshotOpen = $state(false);

  function addStep(): void {
    const rid = apiClient.requests[0]?.id;
    if (!rid) return;
    steps = [...steps, { key: rowKey(), request_id: rid, assertions: [editableAssertion({ kind: 'status', op: 'lt', value: '400' })], extract: [] }];
    dirty = true;
  }
  // a11y-6-03: removal keeps focus in the list (the next step's Remove, else
  // the previous one's, else "Add step"); a move keeps it on the moved step.
  function removeStep(i: number): void {
    steps = steps.filter((_, idx) => idx !== i);
    dirty = true;
    void focusAfterRowRemoval(stepsEl?.parentElement ?? null, i, '.add-step', '[data-remove-step]');
  }
  async function moveStep(i: number, dir: -1 | 1): Promise<void> {
    const j = i + dir;
    if (j < 0 || j >= steps.length) return;
    const moved = steps[i].key;
    const next = [...steps];
    [next[i], next[j]] = [next[j], next[i]];
    steps = next;
    dirty = true;
    const { arrow } = movedArrow(i, dir, steps.length);
    await tick();
    stepsEl?.querySelector<HTMLElement>(`[data-step-key="${moved}"] [data-move="${arrow}"]`)?.focus();
    announce(`Step moved to position ${j + 1} of ${steps.length}`);
  }
  function patchStep(i: number, patch: Partial<EditableStep>): void {
    steps = steps.map((s, idx) => (idx === i ? { ...s, ...patch } : s));
    dirty = true;
  }
  function addAssertion(i: number): void {
    patchStep(i, { assertions: [...steps[i].assertions, editableAssertion({ kind: 'status', op: 'eq', value: '200' })] });
  }
  function updateAssertion(i: number, a: number, patch: Partial<ApiAssertion>): void {
    patchStep(i, { assertions: steps[i].assertions.map((x, ai) => (ai === a ? { ...x, ...patch } : x)) });
  }
  function removeAssertion(e: Event, i: number, a: number): void {
    const list = rowListOf(e, '.sub');
    patchStep(i, { assertions: steps[i].assertions.filter((_, ai) => ai !== a) });
    void focusAfterRowRemoval(list, a, '.add');
  }
  function addExtract(i: number): void {
    patchStep(i, { extract: [...steps[i].extract, editableExtract({ path: '', var: '' })] });
  }
  function updateExtract(i: number, e: number, patch: Partial<ApiExtract>): void {
    patchStep(i, { extract: steps[i].extract.map((x, ei) => (ei === e ? { ...x, ...patch } : x)) });
  }
  function removeExtract(ev: Event, i: number, e: number): void {
    const list = rowListOf(ev, '.sub');
    patchStep(i, { extract: steps[i].extract.filter((_, ei) => ei !== e) });
    void focusAfterRowRemoval(list, e, '.add');
  }

  async function save(): Promise<boolean> {
    if (!automation || !canEdit) return false;
    const snapshot = JSON.stringify(steps);
    const clean = savedSteps(steps);
    const saved = await apiClient.saveAutomation({ name: automation.name, steps: clean }, automation.id);
    if (saved) {
      dirty = JSON.stringify(steps) !== snapshot;
      toasts.success('Automation saved', saved.name);
    }
    return !!saved && !dirty;
  }

  let datasetError = $state('');
  async function run(): Promise<void> {
    if (!automation) return;
    if (dirty && !(await save())) return; // never run a different persisted definition
    let dataset: Record<string, unknown>[] = [];
    datasetError = '';
    try {
      if (datasetText.trim()) dataset = JSON.parse(datasetText);
      if (!Array.isArray(dataset) || dataset.some((r) => !r || Array.isArray(r) || typeof r !== 'object')) throw new Error('Use a JSON array of objects.');
    } catch (e) {
      datasetError = `Dataset rows aren’t valid: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    historyRun = null;
    await apiClient.runAutomation(automation.id, { environment_id: environmentId || null, stop_on_failure: stopOnFailure, dataset });
    showReport();
  }

  // Stop the in-flight run: the requests already sent stay sent, the remaining
  // steps never fire. Halting HTTP steps ends no agent work and can simply be
  // re-run, so it is a neutral Stop without a confirm; a failure says so.
  let stopping = $state(false);
  async function stopRun(runId: Id): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await apiClient.cancelAutomationRun(runId);
      await apiClient.loadAutomationRuns(automationId);
    } catch (e) {
      toastError('Couldn’t stop the run', e);
    } finally {
      stopping = false;
    }
  }

  // The report sits below the steps: bring it into view when a run finishes
  // (or from the header's result chip) so a Run never ends off-screen.
  let reportEl = $state<HTMLElement | null>(null);
  function showReport(): void {
    requestAnimationFrame(() => {
      const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      reportEl?.scrollIntoView({ block: 'start', behavior: reduce ? 'auto' : 'smooth' });
    });
  }

  async function rename(): Promise<void> {
    if (!automation) return;
    const n = await confirmer.promptText('Name', { title: 'Rename automation', confirmLabel: 'Rename', initial: automation.name });
    if (!n || n === automation.name) return;
    await apiClient.saveAutomation({ name: n, steps: automation.steps }, automation.id);
  }
  async function remove(): Promise<void> {
    if (!automation) return;
    if (!(await confirmer.ask(`Delete the automation “${automation.name}” and its run history? The saved requests it uses are kept.`, { title: 'Delete automation' }))) return;
    await apiClient.deleteAutomation(automation.id);
    ondeleted?.();
  }
  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Rename…', icon: 'edit', action: () => void rename(), disabled: !canEdit },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(), disabled: !canEdit },
    ]);
  }

  const lastRun = $derived(historyRun?.report ?? apiClient.lastRun);
  const runDetails = $derived(historyRun ?? apiClient.currentRun);
  const showRun = $derived(lastRun && lastRun.automation_id === automationId ? lastRun : null);
  const runs = $derived(apiClient.automationRuns.filter((r) => r.automation_id === automationId));
  /** The live run's polls kept failing: it may still be running server-side,
   *  so its partial report is not a verdict ("Last run failed" was wrong). */
  // a11y-6-01: the run error line is plain text; the live run's error is
  // spoken once, assertively, where it first appears (an opened past run is
  // read on demand, not announced).
  let spokenRunError = '';
  $effect(() => {
    const err = !historyRun && runDetails?.automation_id === automationId ? runDetails.error ?? '' : '';
    if (err && err !== spokenRunError) announce(`Run failed: ${err}`, { assertive: true });
    spokenRunError = err;
  });
  const lost = $derived(!historyRun && !apiClient.running && !!showRun && apiClient.runLost != null && apiClient.currentRun?.id === apiClient.runLost);
;
async () => {

if(!automation){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`zap`,"title":`This automation no longer exists`,"body":`It may have been deleted. Pick another one from the list.`,}});}
}else{
   { svelteHTML.createElement("div", { "class":`auto`,});
     { svelteHTML.createElement("header", { "class":`view-head`,});
       { svelteHTML.createElement("div", { "class":`vh-text`,});
         { svelteHTML.createElement("h2", {});automation.name;if(dirty){ { svelteHTML.createElement("span", {   "class":`unsaved`,"title":`Unsaved changes`,});   }} }
         { svelteHTML.createElement("p", {});         { svelteHTML.createElement("strong", {});  }     { svelteHTML.createElement("strong", {});  }           { svelteHTML.createElement("code", {});'{{variables}}'; }                }
         { svelteHTML.createElement("p", {});  steps.length;  steps.length === 1 ? 'request' : 'requests';          { svelteHTML.createElement("strong", {});apiClient.environments.find(e => e.id === environmentId)?.name || apiClient.activeEnv?.name || 'no environment'; }              }
       }
       { svelteHTML.createElement("div", { "class":`vh-actions`,});
        if(showRun){
           { svelteHTML.createElement("button", {       "class":`chip result`,"onclick":showReport,"title":lost ? 'Lost track of this run — refresh run history to see how it ended' : 'Show the last run’s results',});!apiClient.running && !lost && showRun.passed;!apiClient.running && !lost && !showRun.passed;
            apiClient.running ? 'Running…' : lost ? 'Lost track of this run' : showRun.passed ? 'Last run passed' : 'Last run failed';  showRun.steps.filter((s) => s.ok).length; showRun.steps.length;
           }
        }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":menu,"aria-label":`Automation options`,"title":`Automation options`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void save(),"disabled":!canEdit || !dirty || apiClient.running,});  }
        if(runDetails?.status === 'running' && runDetails.automation_id === automationId){
           { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":() => void stopRun(runDetails!.id),"disabled":stopping,...__sveltets_2_empty({"data-testid":`automation-stop`}),"title":`Stop the run — steps already sent stay sent`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}stopping ? 'Stopping…' : 'Stop run'; }
        }
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":run,"disabled":!canEdit || steps.length === 0 || apiClient.running,"title":steps.length === 0 ? 'Add a step first' : 'Save and run every step',});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}apiClient.running ? 'Running…' : 'Run';
         }
       }
     }

     { svelteHTML.createElement("div", { "class":`run-opts`,});
       { svelteHTML.createElement("label", { "class":`opt`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", {     "class":`input`,"bind:value":environmentId,"disabled":apiClient.running,});/*Ωignore_startΩ*/() => environmentId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});      apiClient.activeEnv ? ` (${apiClient.activeEnv.name})` : ''; }
             for(let environment of __sveltets_2_ensureArray(apiClient.environments)){environment.id; { svelteHTML.createElement("option", { "value":environment.id,});environment.name; }}
         }
       }
       { svelteHTML.createElement("label", { "class":`checkbox-row`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":stopOnFailure,"disabled":apiClient.running,});/*Ωignore_startΩ*/() => stopOnFailure = __sveltets_2_any(null);/*Ωignore_endΩ*/}       }
     }

     { svelteHTML.createElement("details", { "class":`disclosure`,});
       { svelteHTML.createElement("summary", {});      }
       { svelteHTML.createElement("p", { "class":`hint`,});                 { svelteHTML.createElement("code", {});'{{variables}}'; }                   }
       { svelteHTML.createElement("textarea", {             "dir":`ltr`,"class":`input mono`,"aria-label":`Dataset rows`,"rows":4,"bind:value":datasetText,"placeholder":'[{"customer_id":"cus_1"},{"customer_id":"cus_2"}]',"disabled":apiClient.running,});/*Ωignore_startΩ*/() => datasetText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
      if(datasetError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: datasetError, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err`,});datasetError; }}}
     }

     { const $$_ol1 = svelteHTML.createElement("ol", {  "class":`steps`,});stepsEl = $$_ol1;
          for(let step of __sveltets_2_ensureArray(steps)){let i = 1;step.key;
        const r = req(step.request_id);
         { svelteHTML.createElement("li", {   "class":`step`,...__sveltets_2_empty({"data-step-key":step.key}),});
           { svelteHTML.createElement("div", { "class":`step-top`,});
             { svelteHTML.createElement("span", {   "class":`step-idx`,"aria-hidden":`true`,});i + 1; }
              { const $$_gaTdohteM4C = __sveltets_2_ensureComponent(MethodTag); new $$_gaTdohteM4C({ target: __sveltets_2_any(), props: {  "method":r?.method ?? '',"fixed":true,}});}
             { svelteHTML.createElement("button", {             "type":`button`,"class":`input grow req-pick`,"aria-label":`Request for step ${i + 1}`,"aria-haspopup":`menu`,"disabled":!canEdit,"title":r ? reqLabel(r) : 'Deleted request',"onclick":(e) => pickRequest(e, i),});
               { svelteHTML.createElement("span", { "class":`req-pick-label`,});r ? reqLabel(r) : 'Deleted request'; }
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
             }
            if(canEdit){
               { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`Move up`,"aria-label":`Move step ${i + 1} up`,...__sveltets_2_empty({"data-move":`up`}),"disabled":i === 0,"onclick":() => void moveStep(i, -1),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":14,}});} }
               { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`Move down`,"aria-label":`Move step ${i + 1} down`,...__sveltets_2_empty({"data-move":`down`}),"disabled":i === steps.length - 1,"onclick":() => void moveStep(i, 1),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":14,}});} }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove step`,"aria-label":`Remove step ${i + 1}`,...__sveltets_2_empty({"data-remove-step":true}),"onclick":() => removeStep(i),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});} }
            }
           }

           { svelteHTML.createElement("div", { "class":`sub`,});
             { svelteHTML.createElement("div", { "class":`sub-title`,});  }
            if(step.assertions.length === 0){
               { svelteHTML.createElement("p", { "class":`sub-empty`,});          }
            }
                for(let as of __sveltets_2_ensureArray(step.assertions)){let ai = 1;as.key;
               { svelteHTML.createElement("div", { "class":`rule`,});
                 { svelteHTML.createElement("select", {         "class":`input k-kind`,"aria-label":`What to check`,"value":as.kind,"disabled":!canEdit,"onchange":(e) => updateAssertion(i, ai, { kind: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['kind'] }),});
                     for(let [k, label] of __sveltets_2_ensureArray(Object.entries(KIND_LABEL))){k; { svelteHTML.createElement("option", { "value":k,});label; }}
                 }
                if(as.kind === 'json_path'){
                   { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input mono k-path`,"aria-label":`JSONPath`,"placeholder":`$.data.id`,"value":as.path ?? '',"disabled":!canEdit,"oninput":(e) => updateAssertion(i, ai, { path: (e.currentTarget as HTMLInputElement).value }),});}
                } else if (as.kind === 'header'){
                   { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input mono k-path`,"aria-label":`Header name`,"placeholder":`Content-Type`,"value":as.path ?? '',"disabled":!canEdit,"oninput":(e) => updateAssertion(i, ai, { path: (e.currentTarget as HTMLInputElement).value }),});}
                }
                 { svelteHTML.createElement("select", {         "class":`input k-op`,"aria-label":`Comparison`,"value":as.op,"disabled":!canEdit,"onchange":(e) => updateAssertion(i, ai, { op: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['op'] }),});
                     for(let [o, label] of __sveltets_2_ensureArray(Object.entries(OP_LABEL))){o; { svelteHTML.createElement("option", { "value":o,});label; }}
                 }
                if(!NO_VALUE_OPS.has(as.op)){
                   { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input mono k-val`,"aria-label":as.op === 'matches' ? 'Regular expression' : 'Expected value',"placeholder":as.op === 'matches' ? '^ORD-\\d+' : '200',"value":as.value,"disabled":!canEdit,"oninput":(e) => updateAssertion(i, ai, { value: (e.currentTarget as HTMLInputElement).value }),});}
                }
                if(canEdit){
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove check`,"aria-label":`Remove check on ${as.path || KIND_LABEL[as.kind]} in step ${i + 1}`,...__sveltets_2_empty({"data-remove-row":true}),"onclick":(e) => removeAssertion(e, i, ai),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                }
               }
            }
            if(canEdit){ { svelteHTML.createElement("button", {   "class":`btn ghost small add`,"onclick":() => addAssertion(i),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }}
           }

           { svelteHTML.createElement("div", { "class":`sub`,});
             { svelteHTML.createElement("div", { "class":`sub-title`,});    }
                for(let ex of __sveltets_2_ensureArray(step.extract)){let ei = 1;ex.key;
               { svelteHTML.createElement("div", { "class":`rule`,});
                 { svelteHTML.createElement("span", { "class":`word`,});  }
                 { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`input mono k-path`,"aria-label":`JSONPath, header:Name or status to take`,"title":`A JSONPath into the body ($.access_token), header:Name for a response header, or status`,"placeholder":`$.access_token`,"value":ex.path,"disabled":!canEdit,"oninput":(e) => updateExtract(i, ei, { path: (e.currentTarget as HTMLInputElement).value }),});}
                 { svelteHTML.createElement("span", { "class":`word`,});  }
                 { svelteHTML.createElement("span", { "class":`var-in`,});
                   { svelteHTML.createElement("span", {   "class":`brace`,"aria-hidden":`true`,});'{{'; }
                   { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input mono k-var`,"aria-label":`Variable name`,"placeholder":`api_token`,"value":ex.var,"disabled":!canEdit,"oninput":(e) => updateExtract(i, ei, { var: (e.currentTarget as HTMLInputElement).value }),});}
                   { svelteHTML.createElement("span", {   "class":`brace`,"aria-hidden":`true`,});'}}'; }
                 }
                if(canEdit){
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove`,"aria-label":`Remove saved value ${ex.var || ex.path || ei + 1} in step ${i + 1}`,...__sveltets_2_empty({"data-remove-row":true}),"onclick":(e) => removeExtract(e, i, ei),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                }
               }
            }
            if(canEdit){ { svelteHTML.createElement("button", {   "class":`btn ghost small add`,"onclick":() => addExtract(i),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }}
           }
         }
      }
     }

    if(apiClient.requests.length === 0){
       { svelteHTML.createElement("p", { "class":`sub-empty`,});              }
    } else if (canEdit){
       { svelteHTML.createElement("button", {   "class":`btn small add-step`,"onclick":addStep,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
    }

    if(showRun){
       { const $$_section1 = svelteHTML.createElement("section", {    "class":`report`,"aria-label":`Last run`,});reportEl = $$_section1;
         { svelteHTML.createElement("div", {   "class":`report-banner`,});!lost && showRun.passed;!lost && !showRun.passed;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":!lost && showRun.passed ? 'check' : 'warning',"size":14,}});}
          apiClient.running ? 'Running…' : lost ? 'Lost track of this run — refresh run history to see how it ended' : showRun.passed ? 'All steps passed' : 'Run failed';
           { svelteHTML.createElement("span", { "class":`rb-meta`,});showRun.steps.filter((s) => s.ok).length;  showRun.steps.length;   }
         }
        if(runDetails && runDetails.automation_id === automationId){
           { svelteHTML.createElement("p", { "class":`run-meta`,});
              { svelteHTML.createElement("span", {   "class":`mono`,"title":`Run id`,});runDetails.id; }  RUN_STATUS[runDetails.status];  rel(runDetails.created_at);if(runDetails.dataset_rows){' '; plural(runDetails.dataset_rows, 'data row');}
           }
          if(runDetails.error){ { svelteHTML.createElement("p", { "class":`err`,});runDetails.error; }}
        }
            for(let r of __sveltets_2_ensureArray((showRun.steps.length > reportLimit ? showRun.steps.slice(0, reportLimit) : showRun.steps))){let ri = 1;ri;
           { svelteHTML.createElement("div", {   "class":`r-step`,});r.ok;!r.ok;
             { svelteHTML.createElement("div", { "class":`r-top`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":r.ok ? 'check' : 'x',"size":12,}});}
               { svelteHTML.createElement("span", { "class":`r-idx`,});ri + 1; }
               { svelteHTML.createElement("span", { "class":`r-name`,});r.name; }
                { const $$_pihCsutatS4C = __sveltets_2_ensureComponent(StatusChip); new $$_pihCsutatS4C({ target: __sveltets_2_any(), props: {  "status":r.status,"small":true,}});}
               { svelteHTML.createElement("span", { "class":`r-dur`,});formatSeconds(r.duration_ms / 1000); }
             }
            if(r.error){ { svelteHTML.createElement("div", { "class":`r-error mono`,});r.error; }}
                for(let a of __sveltets_2_ensureArray(r.assertions)){let ax = 1;ax;
               { svelteHTML.createElement("div", {   "class":`r-assert`,});a.passed;!a.passed;
                 { svelteHTML.createElement("span", { "class":`r-word`,});a.passed ? 'Passed' : 'Failed'; } { svelteHTML.createElement("span", { "class":`r-desc`,});a.desc; }
               }
            }
           }
        }
        if(showRun.steps.length > reportLimit){
           { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => (reportLimit += STEP_PAGE),});
             Math.min(STEP_PAGE, showRun.steps.length - reportLimit);   showRun.steps.length - reportLimit;  
           }
        }
        if(runDetails && runDetails.automation_id === automationId){
           { svelteHTML.createElement("details", {   "class":`disclosure`,"bind:open":snapshotOpen,});/*Ωignore_startΩ*/() => snapshotOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("summary", {});      }if(snapshotOpen){ { svelteHTML.createElement("pre", { "class":`snap mono`,});JSON.stringify(runDetails.snapshot, null, 2); }} }
        }
       }
    }

     { svelteHTML.createElement("details", { "class":`disclosure`,});
       { svelteHTML.createElement("summary", {});  runs.length;  }
       { svelteHTML.createElement("div", { "class":`runs`,});
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`run history`,"variant":`compact`,"loading":apiClient.runsLoading,"error":apiClient.runsError,"empty":runs.length === 0,"onretry":() => void apiClient.loadAutomationRuns(automationId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`sub-empty`,});   }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
          
             for(let savedRun of __sveltets_2_ensureArray(runs)){savedRun.id;
             { svelteHTML.createElement("button", {      "class":`run-row`,"onclick":() => void openHistoryRun(savedRun.id),"title":new Date(savedRun.created_at).toLocaleString(),});historyRun?.id === savedRun.id;
              RUN_STATUS[savedRun.status];  savedRun.steps_total ?? savedRun.report.steps.length;   rel(savedRun.created_at);
             }
          }
         LoadState}
         { svelteHTML.createElement("div", { "class":`runs-actions`,});
           { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":async () => { await apiClient.loadAutomationRuns(automationId); if (historyRun) await openHistoryRun(historyRun.id); },});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
           }
          if(apiClient.automationRuns.length >= 50){
             { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => apiClient.loadAutomationRuns(automationId, apiClient.automationRuns.at(-1)!.id),});   }
          }
         }
       }
     }
   }
}


};
return { props: {} as any as Props, exports: {} as any as { approveLeave: typeof approveLeave }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutomationEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutomationEditor__SvelteComponent_ = ReturnType<typeof AutomationEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutomationEditor__SvelteComponent_;