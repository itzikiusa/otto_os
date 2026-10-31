///<reference types="svelte" />
;// Environment picker: dropdown of environments with an active indicator,
// create/activate/delete, and an inline variable editor (key/value rows).

import Icon from '../../lib/components/Icon.svelte';
import { focusAfterRowRemoval, rowListOf } from '../../lib/rowFocus';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { ApiEnvironment } from '../../lib/api/types';
;

  interface Props {
    /** Compact mode hides the variable editor (panel use). */
    compact?: boolean;
  };;

  // Local key/value rows for the env being edited. A `secret` row's value
  // lives in the macOS Keychain: it renders masked, `touched` marks a newly
  // typed replacement value (the only case where a value is sent on save).
  // `storedKey` is the name the secret is stored under in the Keychain (null
  // for new rows) — renaming the row sends `secret_renames` so the stored
  // value follows the new name instead of being deleted.
  interface VarRow { key: string; value: string; secret: boolean; touched: boolean; storedKey: string | null; };function $$render() {

  
  
  
  
  
  
  
  

  let { compact = false }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  let editing: string | null = $state(null);

  let rows: VarRow[] = $state([]);

  function startEdit(env: ApiEnvironment): void {
    editing = editing === env.id ? null : env.id;
    if (editing) {
      rows = [
        ...Object.entries(env.variables).map(([key, value]) => ({ key, value, secret: false, touched: false, storedKey: null })),
        // Secret keys carry no value in the row — masked placeholder instead.
        ...env.secret_keys.map((key) => ({ key, value: '', secret: true, touched: false, storedKey: key })),
      ];
    }
  }

  function addRow(): void {
    rows = [...rows, { key: '', value: '', secret: false, touched: false, storedKey: null }];
  }
  function updateRow(i: number, patch: Partial<VarRow>): void {
    rows = rows.map((r, idx) => (idx === i ? { ...r, ...patch, ...(patch.value !== undefined ? { touched: true } : {}) } : r));
  }
  function removeRow(i: number, e?: Event): void {
    const list = e ? rowListOf(e, '.var-editor') : null;
    rows = rows.filter((_, idx) => idx !== i);
    void focusAfterRowRemoval(list, i, '.var-actions .btn');
  }
  /** Toggle the lock: marking secret hides the value from the row on save;
   * unmarking turns it back into a plain variable (value must be re-typed
   * unless it was just entered). Unmarking a STORED secret nobody retyped
   * deletes its Keychain value on save — ask first. */
  async function toggleSecret(i: number): Promise<void> {
    const row = rows[i];
    if (!row) return;
    if (row.secret && row.storedKey !== null && !(row.touched && row.value !== '')) {
      const ok = await confirmer.ask(
        `“${row.key || row.storedKey}” is stored in the Keychain and can’t be shown. Making it a plain variable deletes the stored value when you save — type the value again to keep it.`,
        { title: 'Make variable plain?', confirmLabel: 'Make plain' },
      );
      if (!ok) return;
    }
    rows = rows.map((r, idx) => (idx === i ? { ...r, secret: !r.secret } : r));
  }

  async function saveVars(env: ApiEnvironment): Promise<void> {
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
    const saved = await apiClient.saveEnvironment(
      { name: env.name, variables, secret_keys, secret_values, secret_renames }, env.id,
    );
    if (saved) editing = null;
  }

  async function create(): Promise<void> {
    if (!canEdit) return;
    const name = await confirmer.promptText('Environment name', {
      title: 'New environment',
      confirmLabel: 'Create',
    });
    if (!name) return;
    await apiClient.saveEnvironment({ name }, undefined);
  }

  async function rename(env: ApiEnvironment): Promise<void> {
    if (!canEdit) return;
    const name = await confirmer.promptText('Rename environment', {
      title: 'Rename environment',
      confirmLabel: 'Rename',
      initial: env.name,
    });
    if (!name || name === env.name) return;
    // Carry secret_keys through — omitting them would un-mark every secret.
    await apiClient.saveEnvironment(
      { name, variables: env.variables, secret_keys: env.secret_keys }, env.id,
    );
  }

  async function remove(env: ApiEnvironment): Promise<void> {
    if (!canEdit) return;
    if (!(await confirmer.ask(`Delete environment “${env.name}”?`, { title: 'Delete environment' }))) return;
    if (editing === env.id) editing = null;
    await apiClient.deleteEnvironment(env.id);
  }

  function activate(env: ApiEnvironment): void {
    void apiClient.activateEnvironment(env.id);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`env-wrap`,});
   { svelteHTML.createElement("div", { "class":`env-head`,});
     { svelteHTML.createElement("span", { "class":`env-title`,});  }
    if(canEdit){
       { svelteHTML.createElement("span", { "class":`env-actions`,});
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Secure secrets: move plaintext tokens/passwords (requests + environments) to the macOS Keychain`,"aria-label":`Secure secrets`,"onclick":() => apiClient.secureAll(),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`shield`,"size":13,}});} }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`New environment`,"aria-label":`New environment`,"onclick":create,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} }
       }
    }
   }

  if(apiClient.environments.length === 0){
     { svelteHTML.createElement("div", { "class":`empty-mini`,});      '{{base_url}}';  }
  }else{
     { svelteHTML.createElement("div", { "class":`env-list`,});
         for(let env of __sveltets_2_ensureArray(apiClient.environments)){env.id;
         { svelteHTML.createElement("div", { "class":`env-item`,});
           { svelteHTML.createElement("div", { "class":`env-row`,});
             { svelteHTML.createElement("button", {        "class":`env-pick`,"onclick":() => activate(env),"title":env.is_active ? 'Active' : 'Set active',});env.is_active;
               { svelteHTML.createElement("span", {  "class":`radio`,});env.is_active;
                if(env.is_active){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}
               }
               { svelteHTML.createElement("span", { "class":`env-name ellipsis grow`,});env.name; }
               { svelteHTML.createElement("span", { "class":`env-count`,});Object.keys(env.variables).length + env.secret_keys.length; if(env.secret_keys.length){' '; env.secret_keys.length; } }
             }
            if(!compact && canEdit){
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Edit variables`,"aria-label":`Edit variables`,"onclick":() => startEdit(env),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Rename`,"aria-label":`Rename`,"onclick":() => rename(env),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});} }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Delete`,"aria-label":`Delete`,"onclick":() => remove(env),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
            }
           }

          if(!compact && editing === env.id){
             { svelteHTML.createElement("div", { "class":`var-editor`,});
                  for(let row of __sveltets_2_ensureArray(rows)){let i = 1;i;
                 { svelteHTML.createElement("div", { "class":`var-row`,});
                   { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input var-key mono`,"placeholder":`base_url`,"aria-label":`Variable name`,"value":row.key,"oninput":(e) => updateRow(i, { key: (e.currentTarget as HTMLInputElement).value }),});}
                   { svelteHTML.createElement("input", {              "class":`input var-val mono`,"type":row.secret ? 'password' : 'text',"aria-label":`Variable value`,"placeholder":row.secret && !row.touched ? '•••••• stored in Keychain — type to replace' : 'https://api.example.com',"value":row.value,"oninput":(e) => updateRow(i, { value: (e.currentTarget as HTMLInputElement).value }),});row.secret;}
                   { svelteHTML.createElement("button", {          "class":`icon-btn`,"title":`Keep the value in the Keychain`,"aria-label":`Keep the value in the Keychain`,"aria-pressed":row.secret,"onclick":() => void toggleSecret(i),});row.secret; { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":row.secret ? 'lock' : 'unlock',"size":12,}});} }
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,...__sveltets_2_empty({"data-remove-row":true}),"title":`Remove variable ${row.key || i + 1}`,"aria-label":`Remove variable ${row.key || i + 1}`,"onclick":(e) => removeRow(i, e),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                 }
              }
               { svelteHTML.createElement("div", { "class":`var-actions`,});
                 { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":addRow,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => saveVars(env),});  }
               }
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EnvSelector__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EnvSelector__SvelteComponent_ = ReturnType<typeof EnvSelector__SvelteComponent_>;
/*Ωignore_endΩ*/export default EnvSelector__SvelteComponent_;