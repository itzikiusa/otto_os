///<reference types="svelte" />
;
import { originChanged } from './credentialOrigin';
import { announceShown } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import { plural } from '../../lib/plural';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Issue-tracker accounts (Jira, Linear) settings page.
import { createIssueAccount, deleteIssueAccount, listIssueProjects, updateIssueAccount } from '../../lib/api/issues';
import { issueAccounts } from '../../lib/stores/issueAccounts.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { IssueAccount, IssueProviderKind } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let accounts: IssueAccount[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let addOpen = $state(false);
  let busy = $state(false);

  // Edit state — null means "Add" mode, non-null means "Edit" mode.
  let editing: IssueAccount | null = $state(null);

  let provider = $state<IssueProviderKind>('jira');
  let label = $state('');
  let baseUrl = $state('');
  let email = $state('');
  let token = $state('');
  // User-entered token expiry as a yyyy-mm-dd value for the date input; '' = unset/clear.
  let tokenExpiresAt = $state('');

  const isEdit = $derived(editing !== null);
  const isLinear = $derived(provider === 'linear');
  /** Display name of an account's tracker. */
  function providerName(p: IssueProviderKind): string {
    return p === 'linear' ? 'Linear' : 'Jira';
  }
  /** Jira signs in with site + email + token; Linear with the API key alone
   *  (the base URL is just the workspace link). */
  const formReady = $derived(
    label.trim() !== '' && (isLinear || (baseUrl.trim() !== '' && email.trim() !== '')),
  );
  /** Repointing an account at another origin (scheme, host or port) must not
   *  carry the stored token there (it would be sent as Basic auth to the new
   *  origin): a changed origin needs a fresh token — the daemon's own rule. */
  const hostChanged = $derived.by(() => {
    // (cast: TS narrows the `$state(null)` initializer to `null` at this point)
    const e = editing as IssueAccount | null;
    return !!e && originChanged(e.base_url, baseUrl);
  });
  const needsToken = $derived(hostChanged && token === '');

  // ── Token-expiry helpers ───────────────────────────────────────────────────
  /** ISO timestamp → yyyy-mm-dd for an <input type="date"> (UTC date part). */
  function toDateInput(iso: string | null): string {
    return iso ? iso.slice(0, 10) : '';
  }
  /** yyyy-mm-dd from the date input → ISO timestamp (or null when cleared). */
  function fromDateInput(d: string): string | null {
    return d.trim() === '' ? null : new Date(`${d}T00:00:00Z`).toISOString();
  }
  /** Human-friendly expiry label, e.g. "expired", "expires today", "expires in 5 days". */
  function expiryLabel(iso: string): string {
    const day = 86_400_000;
    const diff = new Date(iso).getTime() - Date.now();
    const days = Math.ceil(diff / day);
    if (days < 0) return 'expired';
    if (days === 0) return 'expires today';
    if (days === 1) return 'expires in 1 day';
    if (days <= 30) return `expires in ${days} days`;
    return `expires ${new Date(iso).toLocaleDateString()}`;
  }

  /** Returns 'expired', 'soon' (within 14 days), or '' for no warning. */
  function expiryWarning(iso: string | null): '' | 'soon' | 'expired' {
    if (!iso) return '';
    const day = 86_400_000;
    const diff = new Date(iso).getTime() - Date.now();
    const days = Math.ceil(diff / day);
    if (days < 0) return 'expired';
    if (days <= 14) return 'soon';
    return '';
  }

  $effect(() => {
    void load();
  });

  // ── Connection test ────────────────────────────────────────────────────────
  // There's no dedicated test route; listing the projects (Jira) or teams
  // (Linear) the token can see exercises the same credential and is read-only.
  type TestResult = { ok: true; projects: number } | { ok: false; error: string } | 'busy';
  let testResults: Record<string, TestResult> = $state({});

  async function testAccount(a: IssueAccount): Promise<void> {
    testResults = { ...testResults, [a.id]: 'busy' };
    try {
      const projects = await listIssueProjects(a.id, { meta: false });
      testResults = { ...testResults, [a.id]: { ok: true, projects: projects.items.length } };
    } catch (e) {
      testResults = { ...testResults, [a.id]: { ok: false, error: loadErrorText(e) } };
    }
  }

  function testLabel(a: IssueAccount, r: Exclude<TestResult, 'busy'>): string {
    return r.ok
      ? `Connected · ${plural(r.projects, a.provider === 'linear' ? 'team' : 'project')} visible`
      : `Couldn’t connect: ${r.error}`;
  }

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      accounts = await issueAccounts.load(true);
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    provider = 'jira';
    label = '';
    baseUrl = '';
    email = '';
    token = '';
    tokenExpiresAt = '';
    addOpen = true;
  }

  function openEdit(a: IssueAccount): void {
    editing = a;
    provider = a.provider;
    label = a.label;
    baseUrl = a.base_url;
    email = a.email;
    token = '';
    tokenExpiresAt = toDateInput(a.token_expires_at);
    addOpen = true;
  }

  function closeModal(): void {
    addOpen = false;
    editing = null;
  }

  async function create(): Promise<void> {
    busy = true;
    try {
      const a = await createIssueAccount({
        provider,
        label: label.trim(),
        base_url: baseUrl.trim(),
        email: isLinear ? '' : email.trim(),
        token,
        token_expires_at: fromDateInput(tokenExpiresAt),
      });
      accounts = [...accounts, a];
      issueAccounts.upsert(a);
      closeModal();
      toasts.success(`${providerName(a.provider)} account added`, a.label);
    } catch (e) {
      toasts.error(`Couldn’t add the ${providerName(provider)} account`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function save(): Promise<void> {
    if (!editing || needsToken) return;
    busy = true;
    try {
      const body: Record<string, string | null | undefined> = {
        label: label.trim(),
        email: email.trim(),
        base_url: baseUrl.trim(),
        token_expires_at: fromDateInput(tokenExpiresAt),
      };
      if (token !== '') body.token = token;
      const updated = await updateIssueAccount(editing.id, body);
      accounts = accounts.map((x) => (x.id === updated.id ? updated : x));
      issueAccounts.upsert(updated);
      // A changed token/URL makes an earlier verdict stale.
      const { [updated.id]: _stale, ...rest } = testResults;
      testResults = rest;
      closeModal();
      toasts.success(`${providerName(updated.provider)} account updated`, updated.label);
    } catch (e) {
      toasts.error(`Couldn’t save the ${providerName(provider)} account`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  /** Accounts whose delete is in flight, by id — that row's button disables. */
  let deleting = $state<Record<string, boolean>>({});

  async function remove(a: IssueAccount): Promise<void> {
    if (deleting[a.id]) return;
    if (!(await confirmer.ask(`Delete account “${a.label}”? Its token is removed from the Keychain.`, { title: 'Delete account?', danger: true, confirmLabel: 'Delete account' }))) return;
    deleting = { ...deleting, [a.id]: true };
    try {
      await deleteIssueAccount(a.id);
      accounts = accounts.filter((x) => x.id !== a.id);
      issueAccounts.remove(a.id);
      toasts.success(`${providerName(a.provider)} account deleted`, a.label);
    } catch (e) {
      toasts.error(`Couldn’t delete the ${providerName(a.provider)} account`, loadErrorText(e));
    } finally {
      const next = { ...deleting };
      delete next[a.id];
      deleting = next;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('jira'),"subtitle":`Search and attach Jira and Linear issues`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      
      if(accounts.length > 0){
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":openAdd,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}   }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`issue-tracker accounts`,"variant":`page`,loading,"error":loadError,"empty":accounts.length === 0,"onretry":() => void load(),"rows":2,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`ticket`,"title":`No Jira or Linear accounts yet`,"body":`Add a Jira Cloud or Linear account to search issues, attach them to sessions and publish from Product. The token is stored in the macOS Keychain.`,"actionLabel":`Add account`,"actionIcon":`plus`,"onaction":openAdd,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`acct-list`,});
         for(let a of __sveltets_2_ensureArray(accounts)){a.id;
        const warn = expiryWarning(a.token_expires_at);
        const r = testResults[a.id];
        
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "role":`presentation`,"class":`acct card`,"oncontextmenu":(e) => ctxMenu.show(e, [
            { label: 'Test connection', icon: 'refresh', action: () => testAccount(a) },
            { label: 'Edit…', icon: 'edit', action: () => openEdit(a) },
            { separator: true },
            { label: 'Delete…', icon: 'trash', danger: true, action: () => remove(a) },
          ]),});
           { svelteHTML.createElement("span", { "class":`acct-icon`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":14,}});} }
           { svelteHTML.createElement("div", { "class":`grow`,});
             { svelteHTML.createElement("div", { "class":`acct-label`,});
               { svelteHTML.createElement("span", {   "class":`acct-name`,"title":a.label,});a.label; }
               { svelteHTML.createElement("span", { "class":`chip`,});providerName(a.provider); }
              if(warn === 'expired'){
                 { svelteHTML.createElement("span", {   "class":`chip bad`,"title":`The token has expired — edit the account and paste a new one`,});  }
              } else if (warn === 'soon'){
                 { svelteHTML.createElement("span", {   "class":`chip chip-warn`,"title":`Update the token before it lapses`,});expiryLabel(a.token_expires_at!).replace(/^./, (c) => c.toUpperCase()); }
              }
             }
             { svelteHTML.createElement("div", { "class":`acct-sub`,});
              if(a.email){a.email; } { svelteHTML.createElement("span", { "class":`mono`,});a.base_url; }
              if(a.token_expires_at && !warn){  { svelteHTML.createElement("span", { "class":`expiry`,});expiryLabel(a.token_expires_at); }
              }
             }
            if(r){
               {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(r === 'busy' ? 'Testing…' : { text: testLabel(a, r), assertive: !r.ok })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`test-result`,});r !== 'busy' && r.ok;r !== 'busy' && !r.ok;
                if(r !== 'busy'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":r.ok ? 'check' : 'warning',"size":12,}});}}
                r === 'busy' ? 'Testing…' : testLabel(a, r);
               }}
            }
           }
           { svelteHTML.createElement("button", {         "class":`btn small acct-test`,"title":`Check the stored token against ${a.base_url}`,"disabled":r === 'busy',"onclick":() => testAccount(a),});
            r === 'busy' ? 'Testing…' : 'Test';
           }
           { svelteHTML.createElement("button", {       "class":`icon-btn acct-tool`,"title":`Edit ${a.label}`,"aria-label":`Edit ${a.label}`,"onclick":() => openEdit(a),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":14,}});}
           }
           { svelteHTML.createElement("button", {             "class":`icon-btn acct-tool`,"title":deleting[a.id] ? `Deleting ${a.label}…` : `Delete ${a.label}`,"aria-label":deleting[a.id] ? `Deleting ${a.label}…` : `Delete ${a.label}`,"aria-busy":deleting[a.id] ? 'true' : undefined,"disabled":deleting[a.id],"onclick":() => remove(a),});
            if(deleting[a.id]){ { svelteHTML.createElement("span", {   "class":`spinner`,"aria-hidden":`true`,}); }}else{ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}}
           }
         }}
      }
     }
   LoadState}
   PageBody}
 }

if(addOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":isEdit ? `Edit ${providerName(provider)} account` : 'Add issue-tracker account',"onclose":closeModal,footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":closeModal,});  }
      if(isEdit){
         { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !formReady || needsToken,"onclick":save,});
          busy ? 'Saving…' : 'Save changes';
         }
      }else{
         { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !formReady || token === '',"onclick":create,});
          busy ? 'Adding…' : 'Add account';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    if(!isEdit){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", {   "class":`field-label`,"id":`ia-provider-label`,});  }
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-labelledby":`ia-provider-label`,});
             for(let p of __sveltets_2_ensureArray((['jira', 'linear'] as const))){p;
             { svelteHTML.createElement("button", {     "aria-pressed":provider === p,"onclick":() => (provider = p),});provider === p;providerName(p); }
          }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});            }
       }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ia-label`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ia-label`,"class":`input`,"bind:value":label,"placeholder":isLinear ? 'Work Linear' : 'Work Jira',});/*Ωignore_startΩ*/() => label = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ia-base`,});isLinear ? 'Workspace URL' : 'Base URL';if(isLinear){  { svelteHTML.createElement("span", { "class":`dim`,});  }} }
       { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`ia-base`,"class":`input mono`,"bind:value":baseUrl,"placeholder":isLinear ? 'https://linear.app/yourteam' : 'https://yourcompany.atlassian.net',"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => baseUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    if(!isLinear){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ia-email`,});  }
       { svelteHTML.createElement("input", {                "dir":`ltr`,"id":`ia-email`,"class":`input`,"type":`email`,"bind:value":email,"placeholder":`you@company.com`,"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => email = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ia-token`,});isLinear ? 'API key' : 'API token'; }
       { svelteHTML.createElement("input", {             "id":`ia-token`,"class":`input`,"type":`password`,"bind:value":token,"autocomplete":`off`,"placeholder":isEdit ? '•••••• (leave blank to keep)' : '',});/*Ωignore_startΩ*/() => token = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(isEdit && hostChanged){
         { svelteHTML.createElement("span", {   "class":`hint warn-hint`,"role":`alert`,});                     }
      } else if (isEdit){
         { svelteHTML.createElement("span", { "class":`hint`,});       }
      } else if (isLinear){
         { svelteHTML.createElement("span", { "class":`hint`,});
                      
           { svelteHTML.createElement("strong", {});         }
         }
      }else{
         { svelteHTML.createElement("span", { "class":`hint`,});
                    
           { svelteHTML.createElement("strong", {});      }
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ia-expiry`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
       { svelteHTML.createElement("input", {        "id":`ia-expiry`,"class":`input`,"type":`date`,"bind:value":tokenExpiresAt,});/*Ωignore_startΩ*/() => tokenExpiresAt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`hint`,});            }
     }

    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const IssueAccounts__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type IssueAccounts__SvelteComponent_ = ReturnType<typeof IssueAccounts__SvelteComponent_>;
/*Ωignore_endΩ*/export default IssueAccounts__SvelteComponent_;