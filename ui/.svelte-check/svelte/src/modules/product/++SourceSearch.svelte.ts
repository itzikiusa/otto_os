///<reference types="svelte" />
;// SourceSearch — unified tracker issue (Jira / Linear) / Confluence page picker.
// Mirrors the structure and CSS of JiraIssuePicker.svelte.

import { announceShown } from '../../lib/announce.svelte';
import { listConfluenceSpaces, listIssueProjects, searchConfluence, searchIssues } from '../../lib/api/issues';
import type { IssueProject, IssueProviderKind, IssueSummary } from '../../lib/api/types';
import { truncatedNote } from '../../lib/listingPage';
import type { ConfluenceSpace, ConfluencePageSummary } from './types';
import { loadErrorText } from '../../lib/loadError';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { untrack } from 'svelte';
import { openExternal } from '../../lib/external';
import {
    CQL_CHEATSHEET, CQL_DOCS, CQL_TEMPLATES, JQL_CHEATSHEET, JQL_DOCS, JQL_TEMPLATES, fillTemplate,
    LINEAR_CHEATSHEET, LINEAR_DOCS, LINEAR_TEMPLATES, fillLinearTemplate,
  } from './queryTemplates';
;

  interface PickedItem {
    key: string;
    label: string;
  };;

  interface Props {
    accountId: string;
    sourceKind: 'jira' | 'confluence';
    /** The account's tracker: Linear's raw query is an IssueFilter JSON
     *  object and its "projects" are teams. */
    provider?: IssueProviderKind;
    onpick: (sel: PickedItem) => void;
  };;
  // How the query is read: free text (Jira) / title (Confluence), title +
  // body (Confluence), or a raw JQL / CQL expression run verbatim. Raw modes
  // search on Enter — a half-typed expression is a guaranteed 400.
  type SearchMode = 'text' | 'body' | 'raw';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let { accountId, sourceKind, provider = 'jira', onpick }: Props = $props();
  const linear = $derived(provider === 'linear' && sourceKind === 'jira');
  const projectNoun = $derived(linear ? 'team' : 'project');

  // --- Jira state ---
  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  /** The project / space list stopped at its page cap (S5-22). */
  let listTruncated = $state(false);
  let selectedProjectKey = $state('');

  // --- Confluence state ---
  let spaces: ConfluenceSpace[] = $state([]);
  let spacesLoading = $state(false);
  let selectedSpaceKey = $state('');

  // --- shared search state ---
  let query = $state('');

  let mode = $state<SearchMode>('text');
  const rawMode = $derived(mode === 'raw');
  let jiraResults: IssueSummary[] = $state([]);
  let confluenceResults: ConfluencePageSummary[] = $state([]);
  let searching = $state(false);
  let loadingMore = $state(false);
  // Tracks how many Jira results have been fetched so far (cursor for "load more").
  let jiraOffset = $state(0);
  // Whether more Jira results are likely available (last page returned a full 25).
  let jiraHasMore = $state(false);
  // The query + mode whose results are listed (the cursor for Load more).
  let lastRun: { q: string; mode: SearchMode; project: string } | null = null;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  // Only the newest search may land: a slow earlier query (or a multi-second
  // "load more") must never overwrite / append to a newer one. The superseded
  // request is aborted too, so it stops holding a webview connection.
  let searchError = $state('');
  let filterError = $state('');
  let searched = $state(false);
  let retrySearch: (() => void) | null = null;
  let filterSeq = 0;
  let searchSeq = 0;
  let searchCtl: AbortController | null = null;
  $effect(() => () => searchCtl?.abort());

  // --- load projects / spaces when accountId or sourceKind changes ---
  $effect(() => {
    const aid = accountId;
    const kind = sourceKind;
    if (!aid) return;
    resetSearch();
    if (kind === 'jira') {
      // The empty-query "recent issues" search is fired by the filter effect
      // below (it re-runs on account/kind too) — firing it here as well made
      // two identical Jira searches on every open (backlog B6 / SE-11).
      void loadProjects(aid);
    } else {
      void loadSpaces(aid);
    }
  });

  function resetSearch(): void {
    ++searchSeq; searchCtl?.abort();
    searching = false; loadingMore = false; searchError = ''; searched = false; retrySearch = null;
    query = '';
    mode = 'text';
    jiraResults = [];
    confluenceResults = [];
    jiraOffset = 0;
    jiraHasMore = false;
    selectedProjectKey = '';
    selectedSpaceKey = '';
    if (debounceTimer) { clearTimeout(debounceTimer); debounceTimer = null; }
  }

  async function loadProjects(aid: string): Promise<void> {
    const seq = ++filterSeq;
    const current = () => seq === filterSeq && accountId === aid && sourceKind === 'jira';
    filterError = '';
    projectsLoading = true;
    projects = [];
    listTruncated = false;
    try {
      const page = await listIssueProjects(aid, { meta: true });
      if (current()) {
        projects = page.items;
        listTruncated = page.truncated;
      }
    } catch (e) {
      if (current()) filterError = loadErrorText(e);
    } finally {
      if (current()) projectsLoading = false;
    }
  }

  async function loadSpaces(aid: string): Promise<void> {
    const seq = ++filterSeq;
    const current = () => seq === filterSeq && accountId === aid && sourceKind === 'confluence';
    filterError = '';
    spacesLoading = true;
    spaces = [];
    listTruncated = false;
    try {
      const page = await listConfluenceSpaces(aid);
      if (current()) {
        spaces = page.items;
        listTruncated = page.truncated;
      }
    } catch (e) {
      if (current()) filterError = loadErrorText(e);
    } finally {
      if (current()) spacesLoading = false;
    }
  }

  // --- reset results when project/space selector changes ---
  $effect(() => {
    const proj = selectedProjectKey;
    const space = selectedSpaceKey;
    ++searchSeq; searchCtl?.abort();
    searching = false; loadingMore = false; searchError = ''; searched = false; retrySearch = null;
    jiraResults = [];
    confluenceResults = [];
    jiraOffset = 0;
    jiraHasMore = false;
    query = '';
    // Reload with the new project/space filter.
    if (accountId && sourceKind === 'jira') {
      // untrack: search() reads the Match mode — switching it must not re-run
      // this reset (it wiped the typed query and fired an empty search).
      untrack(() => void search('', 0, false, proj || undefined));
    } else if (accountId && sourceKind === 'confluence' && space) {
      // Confluence: wait for user to type (spaces are wide).
    }
  });

  // --- search ---
  function onQueryInput(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    if (rawMode) return;
    const q = query.trim();
    if (!accountId) return;
    // Empty query re-triggers the recency default; no early return.
    debounceTimer = setTimeout(() => void search(q, 0, false), 350);
  }

  // Raw-mode help: examples to start from + a one-screen syntax sheet.
  let syntaxOpen = $state(false);
  let queryInput: HTMLInputElement | undefined = $state();
  const rawLang = $derived(linear ? 'Linear filter' : sourceKind === 'jira' ? 'JQL' : 'CQL');
  const rawDocs = $derived(linear ? LINEAR_DOCS : sourceKind === 'jira' ? JQL_DOCS : CQL_DOCS);
  const rawSheet = $derived(linear ? LINEAR_CHEATSHEET : sourceKind === 'jira' ? JQL_CHEATSHEET : CQL_CHEATSHEET);
  function showExamples(e: MouseEvent): void {
    const key = sourceKind === 'jira' ? selectedProjectKey : selectedSpaceKey;
    const list = linear ? LINEAR_TEMPLATES : sourceKind === 'jira' ? JQL_TEMPLATES : CQL_TEMPLATES;
    ctxMenu.show(e, list.map((t) => {
      const q = linear ? fillLinearTemplate(t.query, key) : fillTemplate(t.query, key);
      return {
        label: t.label,
        title: q,
        action: () => {
          query = q;
          // Placed, not run: most need a value edited (version, epic, label).
          queueMicrotask(() => queryInput?.focus());
        },
      };
    }));
  }

  function onQueryKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter' || !rawMode || !accountId) return;
    e.preventDefault();
    const q = query.trim();
    if (q) void search(q, 0, false);
  }

  function onModeChange(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const q = query.trim();
    // Switching INTO a raw mode leaves the results until Enter; switching out
    // re-runs the text search (empty = the Jira recency default).
    if (!rawMode && accountId && (q || sourceKind === 'jira')) void search(q, 0, false);
  }

  /**
   * Perform a search.
   *
   * @param q - The search string (empty = recency default for Jira).
   * @param startAt - Cursor offset for pagination.
   * @param append - When true, new results are appended (load-more); otherwise replaces.
   * @param projectOverride - Optional project key override (used on filter change before
   *   selectedProjectKey reactive state has settled).
   */
  async function search(
    q: string,
    startAt: number,
    append: boolean,
    projectOverride?: string,
    runMode: SearchMode = mode,
  ): Promise<void> {
    if (!accountId) return;
    const raw = runMode === 'raw';
    // An empty raw query is not "recent issues" — it waits for Enter.
    if (raw && !q.trim()) return;
    searchCtl?.abort();
    const ctl = new AbortController();
    searchCtl = ctl;
    const seq = ++searchSeq;
    const aid = accountId, kind = sourceKind, project = projectOverride ?? selectedProjectKey, space = selectedSpaceKey;
    const current = () => seq === searchSeq && accountId === aid && sourceKind === kind;
    searchError = ''; retrySearch = null;
    if (append) {
      loadingMore = true;
    } else {
      searching = true;
    }
    try {
      if (sourceKind === 'jira') {
        const proj = projectOverride ?? selectedProjectKey;
        const page = await searchIssues(
          raw
            ? (linear ? { accountId, filter: q, startAt } : { accountId, jql: q, startAt })
            : { accountId, q, project: proj || undefined, startAt },
          ctl.signal,
        );
        if (!current()) return;
        if (append) {
          // Offset paging over a live list: an issue updated between pages
          // can come back again — a repeated key breaks the keyed list.
          const seen = new Set(jiraResults.map((i) => i.key));
          jiraResults = [...jiraResults, ...page.filter((i) => !seen.has(i.key))];
        } else {
          jiraResults = page;
        }
        jiraOffset = startAt + page.length;
        // If a full page came back, assume there may be more.
        jiraHasMore = page.length >= 25;
        // Load more continues THIS query: raw modes don't search while
        // typing, so the box can already hold an unrun expression.
        lastRun = { q, mode: runMode, project };
      } else {
        const pages = await searchConfluence(
          raw
            ? { accountId, cql: q }
            : { accountId, q, space: selectedSpaceKey || undefined, mode: runMode === 'body' ? 'text' : undefined },
          ctl.signal,
        );
        if (!current()) return;
        confluenceResults = pages;
      }
      if (current()) searched = true;
    } catch (e) {
      if (!current() || ctl.signal.aborted) return;
      searchError = loadErrorText(e);
      retrySearch = () => {
        if (accountId === aid && sourceKind === kind && selectedSpaceKey === space && selectedProjectKey === project) void search(q, startAt, append, project, runMode);
      };
      if (!append) {
        jiraResults = [];
        confluenceResults = [];
        // No list on screen → no Load more for the previous query's pages.
        jiraHasMore = false;
        jiraOffset = 0;
      }
    } finally {
      if (current()) {
        searching = false;
        loadingMore = false;
        searchCtl = null;
      }
    }
  }

  function loadMoreJira(): void {
    if (lastRun) void search(lastRun.q, jiraOffset, true, lastRun.project, lastRun.mode);
  }

  function pickIssue(issue: IssueSummary): void {
    onpick({ key: issue.key, label: `${issue.key} — ${issue.summary}` });
  }

  function pickPage(page: ConfluencePageSummary): void {
    onpick({ key: page.id, label: page.title });
  }

  const searchPlaceholder = $derived(
    rawMode
      ? (linear
        ? '{"state":{"type":{"eq":"started"}}} — Enter to run'
        : sourceKind === 'jira'
        ? 'fixVersion = "6.23.0" ORDER BY key — Enter to run'
        : 'space = DEV AND label = "release" — Enter to run')
      : sourceKind === 'jira'
        ? (selectedProjectKey ? 'Number (5218) or name…' : linear ? 'ENG-123 or text…' : 'PROJ-123, number, or text…')
        : mode === 'body' ? 'Words in the title or page body…' : 'Page title or id…',
  );

  const hasResults = $derived(
    sourceKind === 'jira' ? jiraResults.length > 0 : confluenceResults.length > 0,
  );

  const listLoading = $derived(
    sourceKind === 'jira' ? projectsLoading : spacesLoading,
  );

  // cleanup on destroy
  $effect(() => {
    return () => {
      if (debounceTimer) clearTimeout(debounceTimer);
    };
  });
;
async () => {


if(sourceKind === 'jira'){
   { svelteHTML.createElement("div", { "class":`picker-field`,});
     { svelteHTML.createElement("label", {   "class":`picker-label`,"for":`ss-project`,});linear ? 'Team' : 'Project'; }
    if(listLoading){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`${projectNoun}s`,}});}
    }else{
       { svelteHTML.createElement("select", {     "id":`ss-project`,"class":`picker-select`,"bind:value":selectedProjectKey,});/*Ωignore_startΩ*/() => selectedProjectKey = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",}); projectNoun;  }
           for(let p of __sveltets_2_ensureArray(projects)){p.key;
           { svelteHTML.createElement("option", { "value":p.key,});p.name; p.key;  }
        }
       }
      if(listTruncated){
         { svelteHTML.createElement("p", {   "class":`trunc-note`,"role":`note`,});truncatedNote(`${projectNoun}s`, projects.length); }
      }
    }
   }
}else{
   { svelteHTML.createElement("div", { "class":`picker-field`,});
     { svelteHTML.createElement("label", {   "class":`picker-label`,"for":`ss-space`,});  }
    if(listLoading){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`spaces`,}});}
    }else{
       { svelteHTML.createElement("select", {     "id":`ss-space`,"class":`picker-select`,"bind:value":selectedSpaceKey,});/*Ωignore_startΩ*/() => selectedSpaceKey = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let s of __sveltets_2_ensureArray(spaces)){s.key;
           { svelteHTML.createElement("option", { "value":s.key,});s.name; s.key;  }
        }
       }
      if(listTruncated){
         { svelteHTML.createElement("p", {   "class":`trunc-note`,"role":`note`,});truncatedNote('spaces', spaces.length); }
      }
    }
   }
}

if(filterError){
   {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Couldn’t load the filters. ${filterError}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  });    filterError; }}
   { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => sourceKind === 'jira' ? void loadProjects(accountId) : void loadSpaces(accountId),});  }
}


 { svelteHTML.createElement("div", { "class":`picker-field`,});
   { svelteHTML.createElement("div", { "class":`query-head`,});
     { svelteHTML.createElement("label", {   "class":`picker-label`,"for":`ss-query`,});
      sourceKind === 'jira' ? 'Search issues' : 'Search pages';
     }
     { svelteHTML.createElement("select", {           "class":`mode-select`,"aria-label":`Match`,"title":`How the search box is read`,"bind:value":mode,"onchange":onModeChange,});/*Ωignore_startΩ*/() => mode = __sveltets_2_any(null);/*Ωignore_endΩ*/
      if(sourceKind === 'jira'){
         { svelteHTML.createElement("option", { "value":`text`,});   }
         { svelteHTML.createElement("option", { "value":`raw`,});linear ? 'Linear filter' : 'JQL'; }
      }else{
         { svelteHTML.createElement("option", { "value":`text`,});  }
         { svelteHTML.createElement("option", { "value":`body`,});   }
         { svelteHTML.createElement("option", { "value":`raw`,});  }
      }
     }
   }
   { const $$_input1 = svelteHTML.createElement("input", {                    "dir":`auto`,"id":`ss-query`,"class":`picker-input`,"bind:value":query,"oninput":onQueryInput,"onkeydown":onQueryKey,"placeholder":searchPlaceholder,"spellcheck":`false`,"autocomplete":`off`,});rawMode;queryInput = $$_input1;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  if(rawMode){
     { svelteHTML.createElement("div", { "class":`raw-help`,});
       { svelteHTML.createElement("button", {       "type":`button`,"class":`help-btn`,"aria-haspopup":`menu`,"onclick":showExamples,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {       "type":`button`,"class":`help-btn`,"aria-expanded":syntaxOpen,"onclick":() => (syntaxOpen = !syntaxOpen),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":syntaxOpen ? 'chevronDown' : 'chevronRight',"size":12,}});} rawLang; 
       }
       { svelteHTML.createElement("button", {       "type":`button`,"class":`help-btn`,"title":`Open the ${rawLang} reference in the browser`,"onclick":() => void openExternal(rawDocs),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} 
       }
     }
    if(syntaxOpen){
       { svelteHTML.createElement("dl", {   "class":`cheatsheet`,"aria-label":`${rawLang} cheat sheet`,});
           for(let [term, text] of __sveltets_2_ensureArray(rawSheet)){term;
           { svelteHTML.createElement("dt", {});term; }
           { svelteHTML.createElement("dd", { "class":`mono`,});text; }
        }
       }
    }
     { svelteHTML.createElement("p", { "class":`raw-note`,});
             sourceKind === 'jira' ? projectNoun : 'space';      
     }
  }
 }

if(searchError){
   {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Couldn’t search. ${searchError}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  });  searchError; }}
   { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => retrySearch?.(),});  }
}


 { svelteHTML.createElement("div", { "class":`picker-results`,});
  if(searching){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":3,"height":44,}});}
  } else if (!hasResults && searched && !searchError && query.trim() !== ''){
     { svelteHTML.createElement("div", { "class":`no-results dim`,});   }
  } else if (sourceKind === 'jira'){
       for(let issue of __sveltets_2_ensureArray(jiraResults)){issue.key;
       { svelteHTML.createElement("button", {   "class":`issue-row`,"onclick":() => pickIssue(issue),});
         { svelteHTML.createElement("div", { "class":`issue-left`,});
           { svelteHTML.createElement("span", { "class":`issue-key`,});issue.key; }
           { svelteHTML.createElement("span", { "class":`issue-summary`,});issue.summary; }
         }
         { svelteHTML.createElement("span", { "class":`chip status-chip`,});issue.status; }
       }
    }
    if(jiraHasMore){
       { svelteHTML.createElement("button", {       "class":`load-more-btn`,"onclick":loadMoreJira,"disabled":loadingMore,});
        loadingMore ? 'Loading more results…' : 'Load more';
       }
    }
  }else{
       for(let page of __sveltets_2_ensureArray(confluenceResults)){page.id;
       { svelteHTML.createElement("button", {   "class":`issue-row`,"onclick":() => pickPage(page),});
         { svelteHTML.createElement("div", { "class":`issue-left`,});
           { svelteHTML.createElement("span", { "class":`issue-summary`,});page.title; }
         }
         { svelteHTML.createElement("span", { "class":`chip status-chip`,});page.space_key; }
       }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SourceSearch__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SourceSearch__SvelteComponent_ = ReturnType<typeof SourceSearch__SvelteComponent_>;
/*Ωignore_endΩ*/export default SourceSearch__SvelteComponent_;