///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from '../settings/sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Git accounts settings page: provider, label, username, token (write-only),
// api_base_url for self-hosted GitLab.
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import type { GitAccount, GitAccountTestResp, GitProviderKind } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let accounts: GitAccount[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let addOpen = $state(false);
  let busy = $state(false);

  // Edit state — null means "Add" mode, non-null means "Edit" mode.
  let editing: GitAccount | null = $state(null);

  let provider: GitProviderKind = $state('github');
  let label = $state('');
  let username = $state('');
  let token = $state('');
  let apiBaseUrl = $state('');
  let namespace = $state('');
  // Token expiry: GitHub/GitLab auto-detect server-side; Bitbucket is manual.
  // Stored as a yyyy-mm-dd value for the date input; '' = unset/clear.
  let tokenExpiresAt = $state('');

  const isEdit = $derived(editing !== null);

  // ── Connection test ────────────────────────────────────────────────────────
  // Per-row results (keyed by account id) + one slot for the add/edit form.
  // 'busy' while in flight; the endpoint returns ok:false inline for auth
  // failures (HTTP 200), so a red row here is the provider's own error text.
  let testResults: Record<string, GitAccountTestResp | 'busy'> = $state({});
  let formTest: GitAccountTestResp | 'busy' | null = $state(null);

  function testLabel(r: GitAccountTestResp): string {
    if (!r.ok) return `Couldn’t connect: ${r.error ?? 'the provider refused the token'}`;
    const scopes = r.scopes?.length ? ` · scopes: ${r.scopes.join(', ')}` : '';
    return `Connected as ${r.login ?? 'an unknown user'}${scopes}`;
  }

  /** Provider ids → their proper names (GitHub, not "github"). */
  const PROVIDER_LABELS: Record<GitProviderKind, string> = {
    github: 'GitHub',
    bitbucket: 'Bitbucket',
    gitlab: 'GitLab',
  };
  const PROVIDERS: GitProviderKind[] = ['github', 'bitbucket', 'gitlab'];

  async function testAccount(a: GitAccount): Promise<void> {
    testResults = { ...testResults, [a.id]: 'busy' };
    try {
      const r = await api.post<GitAccountTestResp>(`/git/accounts/${a.id}/test`, {});
      testResults = { ...testResults, [a.id]: r };
    } catch (e) {
      testResults = {
        ...testResults,
        [a.id]: { ok: false, error: loadErrorText(e) },
      };
    }
  }

  /** Form variant: a typed token tests the DRAFT credentials; an edit form with
   *  the token left blank tests the stored one. */
  async function testForm(): Promise<void> {
    formTest = 'busy';
    try {
      formTest =
        token === '' && editing
          ? await api.post<GitAccountTestResp>(`/git/accounts/${editing.id}/test`, {})
          : await api.post<GitAccountTestResp>('/git/accounts/test', {
              provider,
              username: username.trim(),
              token,
              api_base_url:
                provider === 'gitlab' && apiBaseUrl.trim() !== '' ? apiBaseUrl.trim() : null,
            });
    } catch (e) {
      formTest = { ok: false, error: loadErrorText(e) };
    }
  }

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

  /** Same rule as Jira accounts: 'expired', 'soon' (within 14 days) or ''. */
  function expiryWarning(iso: string | null): '' | 'soon' | 'expired' {
    if (!iso) return '';
    const days = Math.ceil((new Date(iso).getTime() - Date.now()) / 86_400_000);
    if (days < 0) return 'expired';
    if (days <= 14) return 'soon';
    return '';
  }

  const providerHints: Record<GitProviderKind, string> = {
    github: 'Personal access token (classic or fine-grained) with repo scope.',
    bitbucket: 'App password with pullrequest read/write scopes.',
    gitlab: 'Personal access token with api scope. Set API base URL for self-hosted.',
  };

  // What the org/namespace is called on each provider (for browsing repos).
  const namespaceLabel: Record<GitProviderKind, string> = {
    github: 'Organization / user',
    bitbucket: 'Workspace',
    gitlab: 'Group / user',
  };
  const namespaceHint: Record<GitProviderKind, string> = {
    github: 'e.g. your org login. Repos under it can be searched and cloned.',
    bitbucket: 'e.g. your-org. Repos under it can be searched and cloned.',
    gitlab: 'e.g. your group path. Repos under it can be searched and cloned.',
  };

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      accounts = await api.get<GitAccount[]>('/git/accounts');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    provider = 'github';
    label = '';
    username = '';
    token = '';
    apiBaseUrl = '';
    namespace = '';
    tokenExpiresAt = '';
    addOpen = true;
  }

  function openEdit(a: GitAccount): void {
    editing = a;
    provider = a.provider;
    label = a.label;
    username = a.username;
    token = '';
    apiBaseUrl = a.api_base_url ?? '';
    namespace = a.namespace ?? '';
    tokenExpiresAt = toDateInput(a.token_expires_at);
    addOpen = true;
  }

  function closeModal(): void {
    addOpen = false;
    editing = null;
    formTest = null;
  }

  async function create(): Promise<void> {
    busy = true;
    try {
      const a = await api.post<GitAccount>('/git/accounts', {
        provider,
        label: label.trim(),
        username: username.trim(),
        token,
        api_base_url: provider === 'gitlab' && apiBaseUrl.trim() !== '' ? apiBaseUrl.trim() : null,
        namespace: namespace.trim() !== '' ? namespace.trim() : null,
        token_expires_at: fromDateInput(tokenExpiresAt),
      });
      accounts = [...accounts, a];
      closeModal();
      toasts.success('Git account added', a.label);
    } catch (e) {
      toasts.error("Couldn’t add the Git account", loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function save(): Promise<void> {
    if (!editing) return;
    busy = true;
    try {
      const body: Record<string, string | null | undefined> = {
        label: label.trim(),
        username: username.trim(),
        namespace: namespace.trim(),
        api_base_url: provider === 'gitlab' ? apiBaseUrl.trim() : '',
        token_expires_at: fromDateInput(tokenExpiresAt),
      };
      if (token !== '') body.token = token;
      const updated = await api.patch<GitAccount>(`/git/accounts/${editing.id}`, body);
      accounts = accounts.map((x) => (x.id === updated.id ? updated : x));
      // A changed token/username makes an earlier verdict stale.
      const { [updated.id]: _stale, ...rest } = testResults;
      testResults = rest;
      closeModal();
      toasts.success('Git account updated', updated.label);
    } catch (e) {
      toasts.error("Couldn’t save the Git account", loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function remove(a: GitAccount): Promise<void> {
    if (!(await confirmer.ask(`Delete account “${a.label}”? Its token is removed from the Keychain.`, { title: 'Delete account' }))) return;
    try {
      await api.del(`/git/accounts/${a.id}`);
      accounts = accounts.filter((x) => x.id !== a.id);
      toasts.success('Git account deleted', a.label);
    } catch (e) {
      toasts.error("Couldn’t delete the Git account", loadErrorText(e));
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('git-accounts'),"subtitle":`Tokens for pull requests and HTTPS pushes`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      
      if(accounts.length > 0){
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":openAdd,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}   }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`Git accounts`,"variant":`page`,loading,"error":loadError,"empty":accounts.length === 0,"onretry":() => void load(),"rows":2,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`branch`,"title":`No Git accounts yet`,"body":`Add a GitHub, Bitbucket or GitLab token to list and review pull requests and to push over HTTPS. The token is stored in the macOS Keychain.`,"actionLabel":`Add account`,"actionIcon":`plus`,"onaction":openAdd,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", {   "class":`acct-list`,"role":`list`,});
         for(let a of __sveltets_2_ensureArray(accounts)){a.id;
        const warn = expiryWarning(a.token_expires_at);
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`acct card`,"role":`listitem`,"oncontextmenu":(e) => ctxMenu.show(e, [
            { label: 'Test connection', icon: 'refresh', action: () => testAccount(a) },
            { label: 'Edit…', icon: 'edit', action: () => openEdit(a) },
            { separator: true },
            { label: 'Delete…', icon: 'trash', danger: true, action: () => remove(a) },
          ]),});
           { svelteHTML.createElement("span", { "class":`acct-icon`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":14,}});} }
           { svelteHTML.createElement("div", { "class":`grow`,});
             { svelteHTML.createElement("div", { "class":`acct-label`,});
               { svelteHTML.createElement("span", {   "class":`acct-name`,"title":a.label,});a.label; }
               { svelteHTML.createElement("span", { "class":`chip`,});PROVIDER_LABELS[a.provider] ?? a.provider; }
              if(warn === 'expired'){
                 { svelteHTML.createElement("span", {   "class":`chip bad`,"title":`The token has expired — edit the account and paste a new one`,});  }
              } else if (warn === 'soon'){
                 { svelteHTML.createElement("span", {   "class":`chip chip-warn`,"title":`Update the token before it lapses`,});expiryLabel(a.token_expires_at!).replace(/^./, (c) => c.toUpperCase()); }
              }
             }
             { svelteHTML.createElement("div", { "class":`acct-sub`,});
              a.username;
              if(a.namespace){  { svelteHTML.createElement("span", { "class":`mono`,});a.namespace; }}
              if(a.api_base_url){  { svelteHTML.createElement("span", { "class":`mono`,});a.api_base_url; }}
              if(a.token_expires_at && !warn){  { svelteHTML.createElement("span", { "class":`expiry`,});expiryLabel(a.token_expires_at); }
              }
             }
            if(testResults[a.id]){
              const r = testResults[a.id];
              
               {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: r === 'busy' ? 'Testing…' : testLabel(r), assertive: r !== 'busy' && !r.ok })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`test-result`,});r !== 'busy' && r.ok;r !== 'busy' && !r.ok;
                if(r !== 'busy'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":r.ok ? 'check' : 'warning',"size":12,}});}}
                r === 'busy' ? 'Testing…' : testLabel(r);
               }}
            }
           }
           { svelteHTML.createElement("button", {         "class":`btn small`,"title":`Check the stored token against ${PROVIDER_LABELS[a.provider] ?? a.provider}`,"disabled":testResults[a.id] === 'busy',"onclick":() => testAccount(a),});
            testResults[a.id] === 'busy' ? 'Testing…' : 'Test';
           }
           { svelteHTML.createElement("button", {       "class":`icon-btn acct-tool`,"title":`Edit ${a.label}`,"aria-label":`Edit ${a.label}`,"onclick":() => openEdit(a),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":14,}});}
           }
           { svelteHTML.createElement("button", {       "class":`icon-btn acct-tool`,"title":`Delete ${a.label}`,"aria-label":`Delete ${a.label}`,"onclick":() => remove(a),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
           }
         }}
      }
     }
   LoadState}
   PageBody}
 }

if(addOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":isEdit ? 'Edit Git account' : 'Add Git account',"onclose":closeModal,footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {           "class":`btn`,"style":`margin-inline-end: auto`,"title":token === '' && !isEdit ? 'Enter a token to test it' : 'Check the token against the provider without saving',"disabled":busy || formTest === 'busy' || (token === '' && !isEdit),"onclick":testForm,});
        formTest === 'busy' ? 'Testing…' : 'Test connection';
       }
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":closeModal,});  }
      if(isEdit){
         { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || label.trim() === '' || username.trim() === '',"onclick":save,});
          busy ? 'Saving…' : 'Save changes';
         }
      }else{
         { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || label.trim() === '' || username.trim() === '' || token === '',"onclick":create,});
          busy ? 'Adding…' : 'Add account';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ga-provider`,});  }
      if(isEdit){
         { svelteHTML.createElement("div", {   "class":`input provider-fixed`,"id":`ga-provider`,});PROVIDER_LABELS[provider]; }
         { svelteHTML.createElement("span", { "class":`hint`,});      }
      }else{
         { svelteHTML.createElement("div", {       "class":`segmented`,"id":`ga-provider`,"role":`group`,"aria-label":`Provider`,});
             for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
             { svelteHTML.createElement("button", {      "aria-pressed":provider === p,"onclick":() => { provider = p; formTest = null; },});provider === p;
              PROVIDER_LABELS[p];
             }
          }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});providerHints[provider]; }
      }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ga-label`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ga-label`,"class":`input`,"bind:value":label,"placeholder":`Work ${PROVIDER_LABELS[provider]}`,});/*Ωignore_startΩ*/() => label = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ga-user`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ga-user`,"class":`input`,"bind:value":username,"spellcheck":`false`,});/*Ωignore_startΩ*/() => username = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ga-token`,});  }
       { svelteHTML.createElement("input", {            "id":`ga-token`,"class":`input`,"type":`password`,"bind:value":token,"autocomplete":`off`,"placeholder":isEdit ? '•••••• (leave blank to keep)' : '',});/*Ωignore_startΩ*/() => token = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(isEdit){
         { svelteHTML.createElement("span", { "class":`hint`,});       }
      }else{
         { svelteHTML.createElement("span", { "class":`hint`,});         }
      }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ga-ns`,});namespaceLabel[provider];  { svelteHTML.createElement("span", { "class":`dim`,});  } }
       { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`ga-ns`,"class":`input mono`,"bind:value":namespace,"spellcheck":`false`,"placeholder":provider === 'bitbucket' ? 'your-org' : '',});/*Ωignore_startΩ*/() => namespace = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`hint`,});namespaceHint[provider]; }
     }
    if(provider === 'gitlab'){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ga-base`,});    { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`ga-base`,"class":`input mono`,"bind:value":apiBaseUrl,"placeholder":`https://gitlab.example.com/api/v4`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => apiBaseUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ga-expiry`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
       { svelteHTML.createElement("input", {        "id":`ga-expiry`,"class":`input`,"type":`date`,"bind:value":tokenExpiresAt,});/*Ωignore_startΩ*/() => tokenExpiresAt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`hint`,});
        if(provider === 'bitbucket'){             }else{         }
       }
     }

    if(formTest){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: formTest === 'busy' ? 'Testing…' : testLabel(formTest), assertive: formTest !== 'busy' && !formTest.ok })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`test-result form-test`,});formTest !== 'busy' && formTest.ok;formTest !== 'busy' && !formTest.ok;
        if(formTest !== 'busy'){ { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":formTest.ok ? 'check' : 'warning',"size":12,}});}}
        formTest === 'busy' ? 'Testing…' : testLabel(formTest);
       }}
    }

    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitAccounts__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitAccounts__SvelteComponent_ = ReturnType<typeof GitAccounts__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitAccounts__SvelteComponent_;