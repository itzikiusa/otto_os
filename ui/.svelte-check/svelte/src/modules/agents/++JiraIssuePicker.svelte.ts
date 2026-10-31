///<reference types="svelte" />
;// Shared issue picker block (Jira or Linear, per account) — used by both ReviewPanel and AttachIssue.
// Handles account selection, project selection, and debounced search.

import { untrack } from 'svelte';
import { listIssueProjects, searchIssues } from '../../lib/api/issues';
import { issueAccounts } from '../../lib/stores/issueAccounts.svelte';
import { loadErrorText, providerErrorText } from '../../lib/loadError';
import LoadState from '../../lib/components/LoadState.svelte';
import type { IssueAccount, IssueProject, IssueProviderKind, IssueSummary } from '../../lib/api/types';
import { truncatedNote } from '../../lib/listingPage';
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { router } from '../../lib/router.svelte';
;

  /** What a pick hands back: enough to persist a session's `AttachedIssue` without a second
   *  lookup — the tracker link and status from the search hit, and the picked account's provider. */
  interface PickedIssue {
    account_id: string;
    provider: IssueProviderKind;
    key: string;
    summary: string;
    url: string;
    status: string;
  };;

  interface Props {
    onpick: (issue: PickedIssue) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  


  let { onpick }: Props = $props();

  // --- state ---
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  /** A failed account load shows inline with Retry — not as “no accounts”. */
  let accountsError = $state<string | null>(null);
  let selectedAccountId = $state('');

  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  /** The project list stopped at its page cap (S5-22). */
  let projectsTruncated = $state(false);
  let selectedProjectKey = $state(''); // '' = All projects

  let query = $state('');
  let results: IssueSummary[] = $state([]);
  let searching = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  // Why the last search failed — shown in the results area with Retry (a toast
  // plus "No issues found." read as a real empty result).
  let searchError = $state<string | null>(null);
  /** What the results status announces: the outcome of a finished search only. */
  const resultsStatus = $derived(
    searching || searchError || query.trim() === ''
      ? ''
      : results.length === 0
        ? 'No issues found'
        : plural(results.length, 'issue'),
  );
  let searchSeq = 0;

  // --- account loading ---
  // Once on mount: the loader's reads (the cache flags, the selection) must
  // not re-run it and reset the picked account (ui-correctness-5-02).
  $effect(() => {
    untrack(() => void loadAccounts());
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    accountsError = null;
    try {
      accounts = await issueAccounts.load();
      if (accounts.length > 0) selectedAccountId = accounts[0].id;
    } catch (e) {
      accountsError = loadErrorText(e);
    } finally {
      accountsLoading = false;
    }
  }

  // --- project loading when account changes ---
  $effect(() => {
    // Track selectedAccountId reactively.
    const aid = selectedAccountId;
    if (!aid) return;
    void loadProjects(aid);
  });

  let projectsSeq = 0;
  async function loadProjects(accountId: string): Promise<void> {
    // Switching accounts quickly: only the newest account's projects land.
    const seq = ++projectsSeq;
    const current = () => seq === projectsSeq && selectedAccountId === accountId;
    projectsLoading = true;
    projects = [];
    projectsTruncated = false;
    selectedProjectKey = '';
    resetSearch();
    try {
      const page = await listIssueProjects(accountId, { meta: true });
      if (!current()) return;
      projects = page.items;
      projectsTruncated = page.truncated;
    } catch {
      // Non-fatal: user can still search without a project filter.
      if (current()) projects = [];
    } finally {
      if (current()) projectsLoading = false;
    }
  }

  // --- reset results when project changes ---
  $effect(() => {
    void selectedProjectKey;
    resetSearch();
  });

  /** Drop the results AND anything still on its way for them: a search (or a
   *  pending debounce) started under the previous account/project would land
   *  after the switch — and picking from it attached that issue key under the
   *  newly selected account. */
  function resetSearch(): void {
    ++searchSeq;
    if (debounceTimer) { clearTimeout(debounceTimer); debounceTimer = null; }
    searching = false;
    searchError = null;
    results = [];
    query = '';
  }

  // --- search ---
  function onQueryInput(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const q = query.trim();
    if (!q || !selectedAccountId) {
      results = [];
      return;
    }
    debounceTimer = setTimeout(() => void search(q), 350);
  }


  async function search(q: string): Promise<void> {
    if (!selectedAccountId) return;
    const seq = ++searchSeq;
    searching = true;
    searchError = null;
    try {
      const found = await searchIssues({
        accountId: selectedAccountId,
        q,
        project: selectedProjectKey || undefined,
      });
      // A slower, older search must not overwrite the newer one's results.
      if (seq === searchSeq) results = found;
    } catch (e) {
      if (seq !== searchSeq) return;
      searchError = providerErrorText(e);
      results = [];
    } finally {
      if (seq === searchSeq) searching = false;
    }
  }

  function pick(issue: IssueSummary): void {
    const provider = accounts.find((a) => a.id === selectedAccountId)?.provider ?? 'jira';
    onpick({
      account_id: selectedAccountId,
      provider,
      key: issue.key,
      summary: issue.summary,
      url: issue.url ?? '',
      status: issue.status ?? '',
    });
  }

  function goToJiraSettings(): void {
    router.go('settings/jira');
  }

  const searchPlaceholder = $derived(
    selectedProjectKey
      ? 'Number (5218) or name…'
      : 'PROJ-123, number, or text…',
  );
;
async () => {

if(accountsLoading || accountsError){
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {            "what":`issue-tracker accounts`,"variant":`compact`,"loading":accountsLoading,"error":accountsError,"empty":true,"rows":2,"onretry":() => void loadAccounts(),}});}
} else if (accounts.length === 0){
   { svelteHTML.createElement("div", { "class":`picker-empty`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":14,}});}
     { svelteHTML.createElement("span", {});      }
     { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => goToJiraSettings(),});
          
     }
   }
}else{
  
  if(accounts.length > 1){
     { svelteHTML.createElement("div", { "class":`picker-field`,});
       { svelteHTML.createElement("label", {   "class":`picker-label`,"for":`jp-account`,});  }
       { svelteHTML.createElement("select", {     "id":`jp-account`,"class":`picker-select`,"bind:value":selectedAccountId,});/*Ωignore_startΩ*/() => selectedAccountId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let a of __sveltets_2_ensureArray(accounts)){a.id;
           { svelteHTML.createElement("option", { "value":a.id,});a.label; a.base_url;  }
        }
       }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`account-badge`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":13,}});}
       { svelteHTML.createElement("span", {});accounts[0].label; }
       { svelteHTML.createElement("span", {   "class":`dim mono`,"style":`font-size: var(--fs-xs)`,});accounts[0].base_url; }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`picker-field`,});
     { svelteHTML.createElement("label", {   "class":`picker-label`,"for":`jp-project`,});  }
    if(projectsLoading){
       { svelteHTML.createElement("div", { "class":`picker-loading`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {      "rows":3,"height":28,"label":`projects`,}});} }
    }else{
       { svelteHTML.createElement("select", {     "id":`jp-project`,"class":`picker-select`,"bind:value":selectedProjectKey,});/*Ωignore_startΩ*/() => selectedProjectKey = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let p of __sveltets_2_ensureArray(projects)){p.key;
           { svelteHTML.createElement("option", { "value":p.key,});p.name; p.key;  }
        }
       }
      if(projectsTruncated){
         { svelteHTML.createElement("p", {   "class":`trunc-note`,"role":`note`,});truncatedNote('projects', projects.length);         }
      }
    }
   }

  
   { svelteHTML.createElement("div", { "class":`picker-field`,});
     { svelteHTML.createElement("label", {   "class":`picker-label`,"for":`jp-query`,});  }
     { svelteHTML.createElement("input", {                "dir":`auto`,"id":`jp-query`,"class":`picker-input`,"bind:value":query,"oninput":onQueryInput,"placeholder":searchPlaceholder,"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }

  
   { svelteHTML.createElement("p", {     "class":`sr-only`,"role":`status`,"aria-live":`polite`,});resultsStatus; }
   { svelteHTML.createElement("div", { "class":`picker-results`,});
    if(searching){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":3,"height":44,}});}
    } else if (searchError && query.trim() !== ''){
       { svelteHTML.createElement("div", {   "class":`no-results`,"role":`alert`,});
         { svelteHTML.createElement("span", { "class":`err`,});  searchError; }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void search(query.trim()),});  }
       }
    } else if (results.length === 0 && query.trim() !== ''){
       { svelteHTML.createElement("div", { "class":`no-results dim`,});   }
    }else{
         for(let issue of __sveltets_2_ensureArray(results)){issue.key;
         { svelteHTML.createElement("button", {   "class":`issue-row`,"onclick":() => pick(issue),});
           { svelteHTML.createElement("div", { "class":`issue-left`,});
             { svelteHTML.createElement("span", { "class":`issue-key`,});issue.key; }
             { svelteHTML.createElement("span", { "class":`issue-summary`,});issue.summary; }
           }
           { svelteHTML.createElement("span", { "class":`chip status-chip`,});issue.status; }
         }
      }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JiraIssuePicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JiraIssuePicker__SvelteComponent_ = ReturnType<typeof JiraIssuePicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default JiraIssuePicker__SvelteComponent_;