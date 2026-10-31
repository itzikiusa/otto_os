///<reference types="svelte" />
;// Environments, edited in the main area (they used to be squeezed into the
// 280 px sidebar). A list of environments on the left — the active one is
// what {{variables}} resolve against when you send — and the selected
// environment's variables as a full-width table on the right.

import Icon from '../../lib/components/Icon.svelte';
import { focusAfterRowRemoval, rowListOf } from '../../lib/rowFocus';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import { plural } from '../../lib/plural';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { router } from '../../lib/router.svelte';
import type { ApiEnvironment, Id } from '../../lib/api/types';
;

  interface Props {
    /** Preselect this environment (e.g. from the header switcher). */
    initialId?: Id | null;
  };;

  // Local key/value rows for the selected env. A `secret` row's value lives in
  // the macOS Keychain: it renders masked; `touched` marks a newly typed
  // replacement (the only case where a value is sent on save). `storedKey` is
  // the Keychain name (null for new rows) — renaming sends `secret_renames` so
  // the stored value follows the new name instead of being deleted.
  interface VarRow { key: string; value: string; secret: boolean; touched: boolean; storedKey: string | null; };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { initialId = null }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  let listW = $state(loadPaneWidth('api.envListW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  // svelte-ignore state_referenced_locally
  let selectedId = $state<Id | null>(initialId);
  const selected = $derived(
    apiClient.environments.find((e) => e.id === selectedId) ??
      apiClient.environments.find((e) => e.is_active) ??
      apiClient.environments[0] ??
      null,
  );

  let rows: VarRow[] = $state([]);
  let dirty = $state(false);
  let loadedFor = $state<string | null>(null);

  let leavePending: Promise<boolean> | null = null;
   function approveLeave(): Promise<boolean> {
    if (!dirty) return Promise.resolve(true);
    if (leavePending) return leavePending;
    leavePending = (async () => {
      const { value } = await confirmer.choose(`“${selected?.name ?? 'This environment'}” has unsaved changes.`, {
        title: 'Save environment changes?',
        options: [{ label: 'Save', value: 'save', kind: 'primary' }, { label: 'Discard', value: 'discard', kind: 'danger' }],
      });
      if (value === 'save') return save();
      if (value !== 'discard') return false;
      dirty = false;
      return true;
    })().finally(() => { leavePending = null; });
    return leavePending;
  }
  // The router asks these same guards before module and workspace changes.
  // Secret drafts stay only in this mounted editor, never browser storage.
  $effect(() => router.guard(() => approveLeave()));

  // Selection/reseed ownership is distinct from edits within the same draft.
  let editorGeneration = 0;
  function seed(env: ApiEnvironment | null): void {
    editorGeneration++;
    loadedFor = env?.id ?? null;
    dirty = false;
    rows = env
      ? [
          ...Object.entries(env.variables).map(([key, value]) => ({ key, value, secret: false, touched: false, storedKey: null })),
          ...env.secret_keys.map((key) => ({ key, value: '', secret: true, touched: false, storedKey: key })),
        ]
      : [];
    if (env && canEdit) rows.push(blankRow());
  }
  function blankRow(): VarRow {
    return { key: '', value: '', secret: false, touched: false, storedKey: null };
  }
  $effect(() => {
    const env = selected;
    if ((env?.id ?? null) !== loadedFor) seed(env);
  });

  async function pick(env: ApiEnvironment): Promise<void> {
    if (env.id === selected?.id) return;
    if (!(await approveLeave())) return;
    selectedId = env.id;
    seed(env);
  }

  function updateRow(i: number, patch: Partial<VarRow>): void {
    rows = rows.map((r, idx) => (idx === i ? { ...r, ...patch, ...(patch.value !== undefined ? { touched: true } : {}) } : r));
    dirty = true;
    // Keep one empty row at the end so adding a variable is just typing.
    const last = rows[rows.length - 1];
    if (canEdit && last && (last.key !== '' || last.value !== '')) rows = [...rows, blankRow()];
  }
  function removeRow(i: number, e?: Event): void {
    const list = e ? rowListOf(e, '.var-table') : null;
    rows = rows.filter((_, idx) => idx !== i);
    dirty = true;
    if (canEdit && !rows.some((r) => r.key === '' && r.value === '')) rows = [...rows, blankRow()];
    void focusAfterRowRemoval(list, i, '.var-key');
  }
  /** Marking secret moves the value to the Keychain on save; making a STORED
   *  secret plain deletes its Keychain value unless it is typed again — ask. */
  async function toggleSecret(i: number): Promise<void> {
    const row = rows[i];
    if (!row) return;
    if (row.secret && row.storedKey !== null && !(row.touched && row.value !== '')) {
      const ok = await confirmer.ask(
        `“${row.key || row.storedKey}” is stored in the Keychain and can’t be shown. Making it a plain variable deletes the stored value when you save, so type the value again to keep it.`,
        { title: 'Make variable plain?', confirmLabel: 'Make plain' },
      );
      if (!ok) return;
    }
    rows = rows.map((r, idx) => (idx === i ? { ...r, secret: !r.secret } : r));
    dirty = true;
  }

  let saving = $state(false);
  let savePending: Promise<boolean> | null = null;
  function save(): Promise<boolean> {
    // A leave decision made during a normal Save waits for that same write.
    if (savePending) return savePending;
    savePending = saveDraft().finally(() => { savePending = null; saving = false; });
    return savePending;
  }
  async function saveDraft(): Promise<boolean> {
    const env = selected;
    if (!env) return false;
    const submittedRows = rows;
    const submittedGeneration = editorGeneration;
    const workspaceId = ws.currentId;
    const variables: Record<string, string> = {};
    const secret_keys: string[] = [];
    const secret_values: Record<string, string> = {};
    const secret_renames: Record<string, string> = {};
    for (const r of rows) {
      const key = r.key.trim();
      if (key === '') continue;
      if (r.secret) {
        secret_keys.push(key);
        // Only new/changed values travel; untouched secrets keep their stored value.
        if (r.touched && r.value !== '') secret_values[key] = r.value;
        // A renamed stored secret: its Keychain value moves to the new name.
        else if (r.storedKey !== null && r.storedKey !== key) secret_renames[r.storedKey] = key;
      } else {
        variables[key] = r.value;
      }
    }
    saving = true;
    const saved = await apiClient.saveEnvironment({ name: env.name, variables, secret_keys, secret_values, secret_renames }, env.id);
    if (!saved || selected?.id !== env.id || loadedFor !== env.id || ws.currentId !== workspaceId) return false;
    if (rows === submittedRows || (editorGeneration !== submittedGeneration && !dirty)) {
      seed(saved);
    } else {
      // An unchanged secret row can drop its submitted plaintext. A secret
      // renamed again while saving must now refer to the first persisted name.
      // Keep all newer keys/values and the dirty flag for the next Save.
      rows = rows.map((row) => {
        if (!row.secret) return row;
        const submitted = submittedRows.find((r) => r === row || (r.storedKey !== null && r.storedKey === row.storedKey));
        const storedKey = submitted?.key.trim();
        if (!submitted || !storedKey || !saved.secret_keys.includes(storedKey)) return row;
        return row === submitted
          ? { ...row, storedKey, touched: false, value: '' }
          : { ...row, storedKey };
      });
    }
    // A successful write only approves leaving when it saved the whole
    // current draft; edits made while awaiting it still belong to the user.
    return !dirty;
  }

   async function create(): Promise<void> {
    if (!(await approveLeave())) return;
    const generation = editorGeneration;
    const workspaceId = ws.currentId;
    const name = await confirmer.promptText('Name', { title: 'New environment', confirmLabel: 'Create', initial: '' });
    if (!name || ws.currentId !== workspaceId) return;
    const saved = await apiClient.saveEnvironment({ name }, undefined);
    // Creating another environment must not overwrite edits or a selection
    // made while its request was in flight.
    if (saved && !dirty && editorGeneration === generation && ws.currentId === workspaceId) {
      selectedId = saved.id;
      seed(saved);
    }
  }
  async function rename(env: ApiEnvironment): Promise<void> {
    const name = await confirmer.promptText('Name', { title: 'Rename environment', confirmLabel: 'Rename', initial: env.name });
    if (!name || name === env.name) return;
    // Carry secret_keys through — omitting them would un-mark every secret.
    await apiClient.saveEnvironment({ name, variables: env.variables, secret_keys: env.secret_keys }, env.id);
  }
  async function remove(env: ApiEnvironment): Promise<void> {
    const secrets = env.secret_keys.length ? ` Its ${env.secret_keys.length} secret value${env.secret_keys.length === 1 ? ' is' : 's are'} removed from the Keychain.` : '';
    if (!(await confirmer.ask(`Delete the environment “${env.name}”?${secrets} Requests that use its {{variables}} will send them unresolved.`, { title: 'Delete environment' }))) return;
    await apiClient.deleteEnvironment(env.id);
    selectedId = null;
  }
  async function secureAll(): Promise<void> {
    if (!(await confirmer.ask(
      'Move every plaintext credential in this workspace into the macOS Keychain: auth tokens and passwords in saved requests, and environment variables whose names look secret (token, secret, password, api key…). They are masked from then on.',
      { title: 'Secure plaintext secrets', confirmLabel: 'Secure secrets', danger: false },
    ))) return;
    await apiClient.secureAll();
  }
  function menu(e: MouseEvent, env: ApiEnvironment): void {
    ctxMenu.show(e, [
      { label: 'Rename…', icon: 'edit', action: () => void rename(env), disabled: !canEdit },
      { label: 'Secure plaintext secrets…', icon: 'shield', action: () => void secureAll(), disabled: !canEdit },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(env), disabled: !canEdit },
    ]);
  }
  const varCount = (env: ApiEnvironment) => Object.keys(env.variables).length + env.secret_keys.length;
;
async () => {

 { svelteHTML.createElement("div", { "class":`envs`,});
   { svelteHTML.createElement("header", { "class":`view-head`,});
     { svelteHTML.createElement("div", { "class":`vh-text`,});
       { svelteHTML.createElement("h2", {});  }
       { svelteHTML.createElement("p", {});
                    { svelteHTML.createElement("code", {});'{{name}}'; }       
                  { svelteHTML.createElement("strong", {});  }      
                         
       }
     }
    if(canEdit){
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":create,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
    }
   }

  if(apiClient.environments.length === 0 && (apiClient.envLoadError || apiClient.listsPending)){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`environments`,"loading":apiClient.listsPending,"error":apiClient.envLoadError,"empty":true,"onretry":() => void apiClient.loadAll(),}});}
  } else if (apiClient.environments.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`globe`,"title":`No environments yet`,"body":`Create one per place you call, for example Staging and Production, each with its own base_url and api_token.`,"actionLabel":canEdit ? 'New environment' : undefined,"actionIcon":`plus`,"onaction":canEdit ? create : undefined,}});}
  }else{
     { svelteHTML.createElement("div", { "class":`split`,});
       { svelteHTML.createElement("ul", {    "class":`env-list`,"aria-label":`Environments`,});__sveltets_2_ensureType(String, Number, `${listW} `);
           for(let env of __sveltets_2_ensureArray(apiClient.environments)){env.id;
           { svelteHTML.createElement("li", {  "class":`env-item`,});env.id === selected?.id;
             { svelteHTML.createElement("button", {     "class":`env-pick`,"onclick":() => void pick(env),"aria-current":env.id === selected?.id ? 'true' : undefined,});
               { svelteHTML.createElement("span", { "class":`env-name`,});env.name; }
               { svelteHTML.createElement("span", { "class":`env-meta`,});
                plural(varCount(env), 'variable');env.secret_keys.length ? ` · ${plural(env.secret_keys.length, 'secret')}` : '';
               }
             }
            if(env.is_active){ { svelteHTML.createElement("span", {   "class":`chip ok active-chip`,"title":`Used when you send`,});  }}
           }
        }
       }
      
       { svelteHTML.createElement("div", { "class":`divider-slot`,}); { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`api.envListW`,"label":`Resize the environments list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';} }

      if(selected){
         { svelteHTML.createElement("section", {   "class":`editor`,"aria-label":`Variables in ${selected.name}`,});
           { svelteHTML.createElement("div", { "class":`ed-head`,});
             { svelteHTML.createElement("h3", {});selected.name; }
            if(selected.is_active){
               { svelteHTML.createElement("span", { "class":`dim`,});     }
            }else{
               { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void apiClient.activateEnvironment(selected.id),"disabled":!canEdit,});  }
            }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":(e) => menu(e, selected),"aria-label":`Environment options`,"title":`Environment options`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
             { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":save,"disabled":!canEdit || !dirty || saving,});saving ? 'Saving…' : 'Save changes'; }
           }

           { svelteHTML.createElement("div", { "class":`var-table`,});
             { svelteHTML.createElement("div", { "class":`vt-row vt-head`,});
               { svelteHTML.createElement("span", {});  }
               { svelteHTML.createElement("span", {});  }
               { svelteHTML.createElement("span", {   "class":`vt-secret`,"aria-hidden":`true`,});  }
               { svelteHTML.createElement("span", {}); }
             }
                for(let row of __sveltets_2_ensureArray(rows)){let i = 1;i;
               { svelteHTML.createElement("div", { "class":`vt-row var-row`,});
                 { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input mono var-key`,"placeholder":`base_url`,"value":row.key,"disabled":!canEdit,"aria-label":`Variable name`,"oninput":(e) => updateRow(i, { key: (e.currentTarget as HTMLInputElement).value }),});}
                 { svelteHTML.createElement("input", {               "class":`input mono var-val`,"type":row.secret ? 'password' : 'text',"placeholder":row.secret && !row.touched ? '•••••• stored in Keychain — type to replace' : 'https://api.example.com',"value":row.value,"disabled":!canEdit,"aria-label":`Value of ${row.key || 'new variable'}`,"oninput":(e) => updateRow(i, { value: (e.currentTarget as HTMLInputElement).value }),});}
                 { svelteHTML.createElement("span", { "class":`vt-secret`,});
                   { svelteHTML.createElement("button", {            "class":`icon-btn`,"disabled":!canEdit,"title":`Keep the value in the Keychain`,"aria-label":`Keep the value in the Keychain`,"aria-pressed":row.secret,"onclick":() => void toggleSecret(i),});row.secret;
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":row.secret ? 'lock' : 'unlock',"size":14,}});}
                   }
                 }
                 { svelteHTML.createElement("span", {});
                  if(canEdit && !(i === rows.length - 1 && row.key === '' && row.value === '')){
                     { svelteHTML.createElement("button", {       "class":`icon-btn`,...__sveltets_2_empty({"data-remove-row":true}),"title":`Remove variable ${row.key || i + 1}`,"aria-label":`Remove variable ${row.key || i + 1}`,"onclick":(e) => removeRow(i, e),}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                  }
                 }
               }
            }
           }
           { svelteHTML.createElement("p", { "class":`foot`,});     { svelteHTML.createElement("code", {});'{{base_url}}'; }                   }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {} as any as { approveLeave: typeof approveLeave,create: typeof create }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EnvironmentsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EnvironmentsView__SvelteComponent_ = ReturnType<typeof EnvironmentsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default EnvironmentsView__SvelteComponent_;