///<reference types="svelte" />
;// Eval-lab matrix: provider × skill × prompt comparison. A left list of
// matrices (+ a "New matrix" form), and a grid of scored cells for the
// selected matrix — one row per prompt, one column per provider·skill pair,
// the highest-scoring cell in each row highlighted as the winner.

import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { toastError } from '../../lib/toastError';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, sentenceCase } from '../../lib/status';
import { rel } from '../../lib/stores/now.svelte';
import { pollWhileVisible } from '../../lib/poll';
import { viewport } from '../../lib/stores/viewport.svelte';
import type { EvalMatrix, MatrixCell, MatrixPrompt, StartMatrixReq } from '../../lib/api/types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';

;type $$ComponentProps =  {
    listCollapsed?: boolean;
    onopenrun?: (evalId: string) => void;
    onphonedetail?: (open: boolean) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    listCollapsed = false,
    onopenrun,
    onphonedetail,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let matrices: EvalMatrix[] = $state([]);
  let loading = $state(true);
  // Inline list-load failure + Retry (instead of a toast over "No matrices yet").
  let loadError: string | null = $state(null);
  let selected: EvalMatrix | null = $state(null);
  let selectedId: string | null = $state(null);
  let showForm = $state(false);
  let creating = $state(false);
  let phoneListOpen = $state(false);
  // The selected matrix's own load: shown inline in the detail pane (a toast
  // left the pane on the generic "Eval matrix" intro, as if nothing existed).
  let detailError: string | null = $state(null);
  let detailLoading = $state(false);

  const showPhoneList = $derived(
    viewport.isPhone && (phoneListOpen || ((loading || !!loadError) && matrices.length === 0)),
  );
  $effect(() => {
    onphonedetail?.(viewport.isPhone && !showPhoneList && matrices.length > 0);
  });
   function back(): void {
    phoneListOpen = true;
  }

  // --- New-matrix form fields --------------------------------------------
  let fName = $state('');
  // Providers are picked from the live registry (a typo'd free-text name used
  // to fan out cells that could only fail).
  const providerOpts = $derived(agentProviders());
  let fProviders = $state<string[]>([defaultAgentProvider()]);
  function toggleProvider(p: string): void {
    fProviders = fProviders.includes(p) ? fProviders.filter((x) => x !== p) : [...fProviders, p];
  }
  let fSkills = $state(''); // comma-separated skill names
  let fTestCmd = $state('');
  let fPrompts: MatrixPrompt[] = $state([{ label: '', task: '' }]);

  // Load the workspace's matrices whenever the active workspace changes.
  $effect(() => {
    const wsId = ws.currentId;
    // A workspace switch drops the previous workspace's open matrix.
    selected = null;
    selectedId = null;
    phoneListOpen = false;
    if (wsId) {
      void loadList(wsId);
    } else {
      loading = false;
      matrices = [];
    }
  });

  let listGeneration = 0;
  async function loadList(wsId: string): Promise<void> {
    const generation = ++listGeneration;
    loading = true;
    loadError = null;
    try {
      const next = await skillsEvalApi.listMatrices(wsId);
      if (ws.currentId !== wsId || generation !== listGeneration) return;
      matrices = next;
      // Open on the newest matrix rather than an empty "pick one" pane.
      if (!showForm && selectedId === null && matrices.length > 0) void selectMatrix(matrices[0].id);
    } catch (e) {
      if (ws.currentId === wsId && generation === listGeneration) loadError = loadErrorText(e);
    } finally {
      if (ws.currentId === wsId && generation === listGeneration) loading = false;
    }
  }

  async function selectMatrix(id: string): Promise<void> {
    selectedId = id;
    showForm = false;
    phoneListOpen = false;
    detailError = null;
    if (selected?.id !== id) selected = null;
    detailLoading = true;
    try {
      const m = await skillsEvalApi.getMatrix(id);
      if (selectedId === id) selected = m;
      syncListEntry(m);
    } catch (e) {
      if (selectedId === id) detailError = loadErrorText(e);
    } finally {
      if (selectedId === id) detailLoading = false;
    }
  }

  /** Leave the form: back to the matrix that was open, else the newest one. */
  function closeForm(): void {
    showForm = false;
    if (matrices.length > 0) void selectMatrix(matrices[0].id);
  }

  function syncListEntry(m: EvalMatrix): void {
    matrices = matrices.map((x) => (x.id === m.id ? m : x));
  }

  // Poll the selected matrix while it's running. Keyed on a derived primitive so
  // a poll that returns the same id+status does NOT churn the interval.
  const pollKey = $derived.by(() => {
    const s = selected;
    return s && s.status === 'running' ? s.id : null;
  });
  $effect(() => {
    const id = pollKey;
    if (!id) return;
    // No event covers matrix cells: a disciplined poll (in-flight guard,
    // paused while hidden, backoff) instead of a bare interval.
    const p = pollWhileVisible(() => refreshSelected(id), { ms: 2500, immediate: false });
    return () => p.stop();
  });

  async function refreshSelected(id: string): Promise<void> {
    try {
      const m = await skillsEvalApi.getMatrix(id);
      if (selectedId === id) selected = m;
      syncListEntry(m);
    } catch {
      /* transient — next tick retries */
    }
  }

  async function cancel(): Promise<void> {
    if (!selected) return;
    const id = selected.id;
    const workspace = ws.currentId;
    if (
      !(await confirmer.ask(`Stop the matrix “${selected.name}”? Cells still running are abandoned; scored cells are kept.`, {
        title: 'Stop matrix',
        confirmLabel: 'Stop matrix',
        danger: true,
      }))
    )
      return;
    try {
      if (workspace !== ws.currentId || selected?.id !== id) return;
      const m = await skillsEvalApi.cancelMatrix(id);
      if (workspace !== ws.currentId) return;
      if (selected?.id === id) selected = m;
      syncListEntry(m);
    } catch (e) {
      toastError('Couldn’t stop the matrix', e);
    }
  }

  // --- Form actions ------------------------------------------------------
  function addPrompt(): void {
    fPrompts = [...fPrompts, { label: '', task: '' }];
  }
  function removePrompt(i: number): void {
    fPrompts = fPrompts.filter((_, idx) => idx !== i);
  }
  function openForm(): void {
    showForm = true;
    selectedId = null;
    selected = null;
    phoneListOpen = false;
  }

  function parseList(s: string): string[] {
    return s
      .split(',')
      .map((x) => x.trim())
      .filter((x) => x.length > 0);
  }

  const canCreate = $derived(
    !creating &&
      fProviders.length > 0 &&
      parseList(fSkills).length > 0 &&
      fPrompts.some((p) => p.task.trim().length > 0),
  );

  // Why "Create matrix" is disabled, for its tooltip.
  const createBlock = $derived.by(() => {
    if (creating) return 'Creating…';
    if (fProviders.length === 0) return 'Pick at least one agent';
    if (parseList(fSkills).length === 0) return 'Add at least one skill';
    if (!fPrompts.some((p) => p.task.trim())) return 'Add at least one prompt with a task';
    return '';
  });

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || creating) return;
    const providers = providerOpts.filter((p) => fProviders.includes(p));
    const skillNames = parseList(fSkills);
    const prompts: MatrixPrompt[] = fPrompts
      .map((p) => ({
        label: p.label.trim() || p.task.trim().slice(0, 48),
        task: p.task.trim(),
      }))
      .filter((p) => p.task.length > 0);

    // Create stays disabled until these hold (see canCreate / createBlock).
    if (providers.length === 0 || skillNames.length === 0 || prompts.length === 0) return;

    const body: StartMatrixReq = {
      name: fName.trim() || 'Untitled matrix',
      mode: 'score_only',
      providers,
      skills: skillNames.map((reference) => ({ kind: 'library', reference })),
      prompts,
      target: { kind: 'working' },
      test_cmd: fTestCmd.trim() || null,
      iterations: 1,
    };

    creating = true;
    try {
      const m = await skillsEvalApi.createMatrix(wsId, body);
      if (ws.currentId !== wsId) return;
      matrices = [m, ...matrices];
      await selectMatrix(m.id);
      toasts.success('Matrix created', 'Cells score the working tree in the background.');
    } catch (e) {
      toastError('Couldn’t create the matrix', e);
    } finally {
      creating = false;
    }
  }

  // --- Grid derivations --------------------------------------------------
  const promptRows = $derived.by<string[]>(() => {
    if (!selected) return [];
    if (selected.prompts.length > 0) return selected.prompts.map((p) => p.label);
    const set = new Set<string>();
    for (const c of selected.cells) set.add(c.prompt);
    return [...set];
  });

  const cols = $derived.by<{ provider: string; skill: string }[]>(() => {
    if (!selected) return [];
    const seen = new Map<string, { provider: string; skill: string }>();
    for (const c of selected.cells) {
      const key = `${c.provider}\u0000${c.skill}`;
      if (!seen.has(key)) seen.set(key, { provider: c.provider, skill: c.skill });
    }
    return [...seen.values()];
  });

  // One pass over the cells builds both lookups; a `cells.find` per <td> and a
  // full scan per row was O(rows × cols × cells) per render (57 ms at 200×5).
  const cellIndex = $derived.by(() => {
    const byKey = new Map<string, MatrixCell>();
    const winners = new Map<string, MatrixCell>();
    for (const c of selected?.cells ?? []) {
      const key = `${c.prompt}\u0000${c.provider}\u0000${c.skill}`;
      if (!byKey.has(key)) byKey.set(key, c); // first match, as `find` did
      if (c.composite_score == null) continue;
      const best = winners.get(c.prompt);
      if (!best || (best.composite_score ?? -Infinity) < c.composite_score) winners.set(c.prompt, c);
    }
    return { byKey, winners };
  });

  function cellAt(prompt: string, provider: string, skill: string): MatrixCell | undefined {
    return cellIndex.byKey.get(`${prompt}\u0000${provider}\u0000${skill}`);
  }

  // eval_id of the winning (max composite) cell within a prompt row.
  function rowWinnerId(prompt: string): string | null {
    return cellIndex.winners.get(prompt)?.eval_id ?? null;
  }

  function proofClass(status: string | undefined): string {
    switch (status) {
      case 'passed':
        return 'pf-pass';
      case 'failed':
        return 'pf-fail';
      case 'partial':
        return 'pf-partial';
      default:
        return 'pf-none';
    }
  }

;
async () => {

 { svelteHTML.createElement("div", {    "class":`mx`,...__sveltets_2_empty({"data-testid":`matrix-view`}),});listCollapsed && !viewport.isPhone;
   { svelteHTML.createElement("aside", {    "class":`mx-side`,"id":`matrices-list`,});viewport.isPhone && !showPhoneList;
     { svelteHTML.createElement("div", { "class":`mx-side-head`,});
       { svelteHTML.createElement("span", { "class":`mx-side-title`,});  }
      
       { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-testid":`matrix-new-btn`}),"onclick":openForm,"aria-pressed":showForm,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
     }
     { svelteHTML.createElement("div", { "class":`mx-list`,});
      if(!ws.currentId){
         { svelteHTML.createElement("div", { "class":`mx-muted`,});   }
      } else if ((loading || loadError) && matrices.length === 0){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {           "what":`matrices`,loading,"error":loadError,"empty":true,"variant":`compact`,"onretry":() => ws.currentId && loadList(ws.currentId),}});}
      } else if (matrices.length === 0){
         { svelteHTML.createElement("div", { "class":`mx-muted`,});   }
      }else{
           for(let m of __sveltets_2_ensureArray(matrices)){m.id;
           { svelteHTML.createElement("button", {        "class":`mx-item`,"aria-current":!showForm && selectedId === m.id ? 'true' : undefined,"onclick":() => selectMatrix(m.id),});!showForm && selectedId === m.id;
             { svelteHTML.createElement("div", { "class":`mx-item-top`,});
               { svelteHTML.createElement("span", {   "class":`mx-item-name`,"title":m.name,});m.name; }
               { svelteHTML.createElement("span", {       "class":`mx-dot st-${m.status}`,"role":`img`,"aria-label":runStatus(m.status).label,"title":runStatus(m.status).label,}); }
             }
             { svelteHTML.createElement("div", { "class":`mx-item-meta`,});
               { svelteHTML.createElement("span", { "title":`${m.providers.length} providers × ${m.skills.length} skills × ${m.prompts.length} prompts`,});m.providers.length;  m.skills.length;  m.prompts.length; }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("span", { "title":new Date(m.created_at).toLocaleString(),});rel(m.created_at); }
             }
           }
        }
      }
     }
   }

   { svelteHTML.createElement("main", {  "class":`mx-main`,});viewport.isPhone && showPhoneList;
    if(!ws.currentId){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`grid`,"title":`No workspace selected`,"body":`Pick a workspace to build a matrix.`,}});}
    } else if (showForm){
       { svelteHTML.createElement("div", { "class":`mx-form`,});
         { svelteHTML.createElement("h2", {});  }
         { svelteHTML.createElement("p", { "class":`lede`,});
                           
                 
         }

         { svelteHTML.createElement("section", { "class":`card block`,});
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`mx-name`,});  }
             { svelteHTML.createElement("input", {            "dir":`auto`,"id":`mx-name`,"class":`input`,...__sveltets_2_empty({"data-testid":`matrix-name`}),"placeholder":`e.g. Logging skill bake-off`,"bind:value":fName,});/*Ωignore_startΩ*/() => fName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("span", {   "class":`field-label`,"id":`mx-prov-lbl`,});  }
             { svelteHTML.createElement("div", {       "class":`provider-chips`,"role":`group`,"aria-labelledby":`mx-prov-lbl`,...__sveltets_2_empty({"data-testid":`matrix-providers`}),});
                 for(let p of __sveltets_2_ensureArray(providerOpts)){p;
                 { svelteHTML.createElement("label", {  "class":`chip-toggle`,});fProviders.includes(p);
                   { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":fProviders.includes(p),"onchange":() => toggleProvider(p),});}
                   { svelteHTML.createElement("span", { "class":`mono`,});p; }
                 }
              }
             }
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`mx-skills`,});  { svelteHTML.createElement("span", { "class":`hint-inline`,});    } }
             { svelteHTML.createElement("input", {            "dir":`auto`,"id":`mx-skills`,"class":`input`,...__sveltets_2_empty({"data-testid":`matrix-skills`}),"placeholder":`golang-testing, golang-code-review`,"bind:value":fSkills,});/*Ωignore_startΩ*/() => fSkills = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`mx-test`,});   { svelteHTML.createElement("span", { "class":`hint-inline`,});  } }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`mx-test`,"class":`input`,...__sveltets_2_empty({"data-testid":`matrix-test-cmd`}),"placeholder":`go test ./...`,"bind:value":fTestCmd,});/*Ωignore_startΩ*/() => fTestCmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
         }

         { svelteHTML.createElement("section", { "class":`card block`,});
           { svelteHTML.createElement("div", { "class":`block-head`,});
             { svelteHTML.createElement("span", { "class":`field-label`,});  }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {     "class":`btn small`,"type":`button`,"onclick":addPrompt,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
             }
           }
              for(let p of __sveltets_2_ensureArray(fPrompts)){let i = 1;i;
             { svelteHTML.createElement("div", { "class":`prompt`,});
               { svelteHTML.createElement("div", { "class":`row`,});
                 { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input grow`,...__sveltets_2_empty({"data-testid":`matrix-prompt-label`}),"aria-label":`Prompt ${i + 1} label`,"placeholder":`Happy path`,"bind:value":p.label,});/*Ωignore_startΩ*/() => p.label = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                if(fPrompts.length > 1){
                   { svelteHTML.createElement("button", {         "class":`icon-btn`,"type":`button`,"title":`Remove prompt`,"aria-label":`Remove prompt ${i + 1}`,"onclick":() => removePrompt(i),});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                   }
                }
               }
               { svelteHTML.createElement("textarea", {              "dir":`auto`,"class":`input`,"rows":2,...__sveltets_2_empty({"data-testid":`matrix-prompt-task`}),"aria-label":`Prompt ${i + 1} task`,"placeholder":`What the agent should do…`,"bind:value":p.task,});/*Ωignore_startΩ*/() => p.task = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
             }
          }
         }

         { svelteHTML.createElement("div", { "class":`actions`,});
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {     "class":`btn`,"type":`button`,"onclick":closeForm,});  }
           { svelteHTML.createElement("button", {         "class":`btn primary`,...__sveltets_2_empty({"data-testid":`matrix-create`}),"disabled":!canCreate,"title":canCreate ? undefined : createBlock,"onclick":create,});
            creating ? 'Creating…' : 'Create matrix';
           }
         }
       }
    } else if (selectedId && (detailError || !selected)){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            "what":`this matrix`,"variant":`page`,"loading":detailLoading || !detailError,"error":detailError,"empty":true,"onretry":() => selectedId && selectMatrix(selectedId),}});}
    } else if (selected){
       { svelteHTML.createElement("div", { "class":`mx-detail`,});
         { svelteHTML.createElement("div", { "class":`mx-detail-head`,});
           { svelteHTML.createElement("div", { "class":`mx-detail-title`,});
             { svelteHTML.createElement("h2", {});selected.name; }
             { svelteHTML.createElement("span", { "class":`mx-sub`,});sentenceCase(selected.mode);  selected.repo_key && selected.repo_key !== ws.currentId ? selected.repo_key : 'This workspace'; }
           }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":runStatus(selected.status),}});}
          if(selected.status === 'running'){
            
             { svelteHTML.createElement("button", {     "class":`btn small danger`,"type":`button`,"onclick":cancel,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  
             }
          }
         }

        if(cols.length === 0){
           { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`grid`,"title":`No cells yet`,"body":`Cells appear as the matrix begins running.`,}});}
        }else{
           { svelteHTML.createElement("div", { "class":`table-wrap`,});
             { svelteHTML.createElement("table", { ...__sveltets_2_empty({"data-testid":`matrix-grid`}),});
               { svelteHTML.createElement("thead", {});
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("th", {   "class":`rowlabel`,"scope":`col`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
                     for(let col of __sveltets_2_ensureArray(cols)){col.provider + '\u0000' + col.skill;
                     { svelteHTML.createElement("th", { "scope":`col`,});
                       { svelteHTML.createElement("div", { "class":`col-prov`,});col.provider; }
                       { svelteHTML.createElement("div", { "class":`col-skill mono`,});col.skill; }
                     }
                  }
                 }
               }
               { svelteHTML.createElement("tbody", {});
                   for(let prompt of __sveltets_2_ensureArray(promptRows)){prompt;
                  const winnerId = rowWinnerId(prompt);
                   { svelteHTML.createElement("tr", {});
                     { svelteHTML.createElement("th", {   "class":`rowlabel`,"scope":`row`,});prompt; }
                       for(let col of __sveltets_2_ensureArray(cols)){col.provider + '\u0000' + col.skill;
                      const cell = cellAt(prompt, col.provider, col.skill);
                       { svelteHTML.createElement("td", {      "class":`cell`,...__sveltets_2_empty({"data-testid":`matrix-cell`}),});cell != null && winnerId != null && cell.eval_id === winnerId;
                         const cellBody/*Ωignore_positionΩ*/ = (c: MatrixCell)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
                          if(c.composite_score != null){
                             { svelteHTML.createElement("span", { "class":`score`,});if(winnerId != null && c.eval_id === winnerId){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} { svelteHTML.createElement("span", { "class":`sr-only`,});   }}c.composite_score.toFixed(0); }
                          }else{
                             { svelteHTML.createElement("span", { "class":`cell-status`,});runStatus(c.status).label; }
                          }
                           { svelteHTML.createElement("span", {   "class":`proof ${proofClass(c.proof_status)}`,"title":`Proof pack status`,});sentenceCase(c.proof_status || 'missing'); }
                        };return __sveltets_2_any(0)};
                        if(cell?.eval_id){
                          const evalId = cell.eval_id;
                           { svelteHTML.createElement("button", {         "type":`button`,"class":`cell-in clickable`,"title":winnerId != null && cell.eval_id === winnerId ? 'Top score in this row · open this run' : 'Open this run',"onclick":() => onopenrun?.(evalId),});;__sveltets_2_ensureSnippet(cellBody(cell)); }
                        } else if (cell){
                           { svelteHTML.createElement("div", { "class":`cell-in`,});;__sveltets_2_ensureSnippet(cellBody(cell)); }
                        }else{
                           { svelteHTML.createElement("span", { "class":`dash`,});  }
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
    }else{
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {             "icon":`grid`,"title":`No matrices yet`,"body":`A matrix scores several providers and skills against the same prompts in one grid, so you can see which combination does best.`,"actionLabel":`New matrix`,"actionIcon":`plus`,"onaction":openForm,}});}
    }
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {} as any as { back: typeof back }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MatrixView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MatrixView__SvelteComponent_ = ReturnType<typeof MatrixView__SvelteComponent_>;
/*Ωignore_endΩ*/export default MatrixView__SvelteComponent_;