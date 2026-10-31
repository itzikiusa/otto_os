///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { onDestroy, untrack } from 'svelte';
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
// PublishDialog — shared modal for both "Publish as Jira Story" and
// "Publish as Confluence RFC" actions. Also used for "Convert RFC → Story".
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { router } from '../../lib/router.svelte';
import { api, ApiError } from '../../lib/api/client';
import { listConfluenceSpaces, listIssueProjects, listIssueTypes } from '../../lib/api/issues';
import { issueAccounts } from '../../lib/stores/issueAccounts.svelte';
import { product } from '../../lib/stores/product.svelte';
import { toasts } from '../../lib/toast.svelte';
import type { IssueAccount, IssueProject } from '../../lib/api/types';
import { truncatedNote } from '../../lib/listingPage';
import { providerErrorText } from '../../lib/loadError';
import ConfluencePagePicker from '../../lib/components/ConfluencePagePicker.svelte';
import type { ConfluenceSpace, ProductStoryVersion, ProductStoryDetail, ReviewedContent } from './types';
;

  interface Props {
    mode: 'story' | 'rfc';
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { mode, onclose }: Props = $props();
  /** Why Publish is disabled once another story is selected underneath the dialog. */
  const STORY_CHANGED =
    'A different story is open now. Close this dialog and open Publish again to review what will be sent.';
  let alive = true;
  onDestroy(() => { alive = false; });

  // ── Accounts (shared) ─────────────────────────────────────────────────────
  let accounts: IssueAccount[] = $state([]);
  let accountsLoading = $state(true);
  let accountId = $state('');

  // ── Story-mode: projects + issue types ───────────────────────────────────
  let projects: IssueProject[] = $state([]);
  let projectsLoading = $state(false);
  /** The project list stopped at its page cap (S5-22). */
  let projectsTruncated = $state(false);
  let projectKey = $state('');
  let projectsError = $state('');
  let projectsSequence = 0;

  let issueTypes: string[] = $state([]);
  let issueTypesLoading = $state(false);
  let issueType = $state('Story');
  let issueTypesSequence = 0;

  // ── RFC-mode: spaces + optional parent + optional title ──────────────────
  let spaces: ConfluenceSpace[] = $state([]);
  let spacesLoading = $state(false);
  /** The space list stopped at its page cap (S5-22). */
  let spacesTruncated = $state(false);
  let spaceKey = $state('');
  let spacesError = $state('');
  let spacesSequence = 0;
  // Parent page: picked by TITLE (PC-9), stored by id — the confirm copy names it.
  let parentId = $state<string | null>(null);
  let parentTitle = $state<string | null>(null);
  let rfcTitle = $state('');

  // ── Submit ────────────────────────────────────────────────────────────────
  let submitting = $state(false);
  // Errors are a human title plus the raw cause as a dim detail line — never
  // the bare exception text on its own.
  let formError = $state('');
  let formErrorDetail = $state('');

  function setError(what: string, e: unknown): void {
    const raw = e instanceof Error ? e.message : String(e);
    let why = '';
    if (e instanceof ApiError) {
      if (e.status === 401 || e.status === 403) why = ' The account was refused — check its token in Settings → Integrations → Jira & Linear accounts.';
      else if (e.status === 404) why = ' The project, space or parent page was not found.';
      else if (e.status === 409) why = ' It conflicts with the current state on the server.';
    } else if (e instanceof TypeError) why = " Otto can’t reach the daemon.";
    formError = `${what}.${why}`;
    formErrorDetail = raw;
  }

  /** The selected account's tracker (a story publishes to Jira or Linear;
   *  Confluence needs an Atlassian account). */
  const tracker = $derived(accounts.find((a) => a.id === accountId)?.provider === 'linear' ? 'Linear' : 'Jira');
  const container = $derived(tracker === 'Linear' ? 'team' : 'project');
  const title = $derived(mode === 'story' ? `Publish as ${tracker} story` : 'Publish as Confluence RFC');

  // ── Preview: WHAT is sent (the same version the daemon publishes — newest
  // suggested, else draft, else source; mirrors `best_content_version`). ─────
  let reviewedContent = $state<ReviewedContent | null>(null);
  const storyTitle = $derived(reviewedContent?.title ?? '');
  const isDraft = $derived(product.detail?.story.source_kind === 'draft');
  let previewError = $state('');
  let previewStoryId = $state<string | null>(null);
  let previewSequence = 0;
  let previewBody = $state<string | null>(null); // null = loading
  // Converting a Confluence RFC → story: the daemon prepends a "> RFC: <url>"
  // reference line to the Jira description (`publish_as_story`), so the
  // preview shows it too — the confirm must match what is actually sent.
  const rfcRef = $derived.by(() => {
    const s = reviewedContent;
    return mode === 'story' && s?.source_kind === 'confluence' && s.url ? `> RFC: ${s.url}` : '';
  });
  const PREVIEW_LINES = 6;
  const previewLines = $derived.by(() => {
    const body = rfcRef ? `${rfcRef}\n\n${previewBody ?? ''}` : (previewBody ?? '');
    const lines = body ? body.split('\n') : [];
    return { body, head: lines.slice(0, PREVIEW_LINES), more: Math.max(0, lines.length - PREVIEW_LINES) };
  });

  async function loadPreview(sid: string | null): Promise<void> {
    const seq = ++previewSequence;
    previewBody = null; previewError = ''; previewStoryId = null; reviewedContent = null;
    if (!sid) { previewError = 'Select a story before publishing.'; return; }
    const current = () => seq === previewSequence && product.selectedId === sid;
    try {
      const { story } = await api.get<ProductStoryDetail>(`/product/stories/${sid}`);
      const vs = await api.get<ProductStoryVersion[]>(`/product/stories/${sid}/versions`);
      const pick = vs.find((v) => v.kind === 'suggested') ?? vs.find((v) => v.kind === 'draft') ?? vs.find((v) => v.kind === 'source');
      const body = pick ? (pick.body_md || (await product.getVersion(pick.id)).body_md) : '';
      const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(body));
      const body_sha256 = Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
      if (current()) {
        reviewedContent = { version_id: pick?.id ?? null, body_sha256, title: story.title, source_kind: story.source_kind, url: story.url };
        previewBody = body; previewStoryId = sid;
      }
    } catch (e) {
      if (current()) previewError = `Couldn’t load the content preview. ${providerErrorText(e)}`;
    }
  }

  // ── WHO sees it ───────────────────────────────────────────────────────────
  const visibility = $derived.by(() => {
    if (mode === 'story') {
      const p = projects.find((x) => x.key === projectKey);
      return p
        ? `Creates a ${issueType || 'Story'} in ${p.name} (${p.key}), visible to everyone with access to that ${tracker} ${container}.`
        : '';
    }
    const sp = spaces.find((x) => x.key === spaceKey);
    return sp
      ? `Creates a page in the ${sp.name} (${sp.key}) space${parentId ? `, under “${parentTitle ?? `page ${parentId}`}”` : ''}, visible to everyone who can view that space.`
      : '';
  });

  // Load accounts + the preview on mount (and on a story switch). The loaders
  // run untracked: `issueAccounts.load()` reads its cache gate synchronously,
  // and tracking it re-ran both loads (preview flashed back to loading,
  // projects fetched twice) once a cold cache filled.
  $effect(() => {
    const sid = product.selectedId;
    untrack(() => {
      void loadAccounts();
      void loadPreview(sid);
    });
  });

  async function loadAccounts(): Promise<void> {
    accountsLoading = true;
    try {
      const all = await issueAccounts.load();
      accounts = mode === 'story' ? all : all.filter((a) => a.provider === 'jira');
      if (accounts.length > 0) {
        accountId = accounts[0].id;
        await onAccountChange();
      }
    } catch (e) {
      setError("Couldn’t load your issue-tracker accounts", e);
    } finally {
      accountsLoading = false;
    }
  }

  async function onAccountChange(): Promise<void> {
    if (!accountId) return;
    if (mode === 'story') {
      await loadProjects();
    } else {
      await loadSpaces();
    }
  }

  async function loadProjects(): Promise<void> {
    const account = accountId, sequence = ++projectsSequence;
    const current = () => sequence === projectsSequence && accountId === account && mode === 'story';
    projectsLoading = true;
    projectsError = ''; projects = []; projectsTruncated = false;
    issueTypesSequence++; issueTypesLoading = false;
    projectKey = '';
    issueTypes = [];
    issueType = 'Story';
    try {
      const next = await listIssueProjects(account, { meta: true });
      if (!current()) return;
      projects = next.items;
      projectsTruncated = next.truncated;
      if (projects.length > 0) {
        projectKey = projects[0].key;
        await loadIssueTypes();
      }
    } catch (e) {
      if (!current()) return;
      projectsError = `Couldn’t load Jira projects. ${providerErrorText(e)}`;
    } finally {
      if (current()) projectsLoading = false;
    }
  }

  async function loadIssueTypes(): Promise<void> {
    if (!accountId || !projectKey) return;
    const account = accountId, project = projectKey, sequence = ++issueTypesSequence;
    const current = () => sequence === issueTypesSequence && accountId === account && projectKey === project && mode === 'story';
    issueTypesLoading = true;
    try {
      const next = await listIssueTypes(account, project);
      if (!current()) return;
      issueTypes = next;
      issueType = issueTypes.includes('Story')
        ? 'Story'
        : issueTypes[0] ?? 'Story';
    } catch {
      if (!current()) return;
      // Non-fatal — default to 'Story'.
      issueTypes = [];
      issueType = 'Story';
    } finally {
      if (current()) issueTypesLoading = false;
    }
  }

  async function loadSpaces(): Promise<void> {
    const account = accountId, sequence = ++spacesSequence;
    const current = () => sequence === spacesSequence && accountId === account && mode === 'rfc';
    spacesLoading = true;
    spacesError = ''; spaces = []; spacesTruncated = false;
    spaceKey = '';
    try {
      const next = await listConfluenceSpaces(account);
      if (!current()) return;
      spaces = next.items;
      spacesTruncated = next.truncated;
      if (spaces.length > 0) spaceKey = spaces[0].key;
    } catch (e) {
      if (!current()) return;
      spacesError = `Couldn’t load Confluence spaces. ${providerErrorText(e)}`;
    } finally {
      if (current()) spacesLoading = false;
    }
  }

  async function submit(): Promise<void> {
    if (submitting || previewBody === null || !reviewedContent || previewError || previewStoryId !== product.selectedId) return;
    formError = '';
    formErrorDetail = '';
    if (!accountId) { formError = 'Select an account.'; return; }

    submitting = true;
    const ownsSelection = product.captureSelection();
    const current = () => alive && ownsSelection();
    try {
      if (mode === 'story') {
        if (!projectKey) { formError = 'Select a project.'; submitting = false; return; }
        const detail = await product.publishAsStory({
          reviewed_content: reviewedContent,
          account_id: accountId,
          project_key: projectKey,
          issue_type: issueType || 'Story',
        });
        toasts.success(`Published as ${tracker} story`, detail.story.title);
        if (!current()) return;
        // Select the resulting story.
        if (detail.story.id !== product.selectedId) {
          await product.select(detail.story.id);
        }
      } else {
        if (!spaceKey) { formError = 'Select a Confluence space.'; submitting = false; return; }
        const detail = await product.publishAsRfc({
          reviewed_content: reviewedContent,
          account_id: accountId,
          space_key: spaceKey,
          parent_id: parentId,
          title: rfcTitle.trim() || null,
        });
        toasts.success('Published as Confluence RFC', detail.story.title);
      }
      if (current()) onclose();
    } catch (e) {
      if (!current()) return;
      setError(mode === 'story' ? `Couldn’t publish to ${tracker}` : "Couldn’t publish to Confluence", e);
      if (e instanceof ApiError && e.status === 409) {
        reviewedContent = null;
        previewError = 'The content changed. Select Retry preview and review it again before publishing.';
      }
    } finally {
      submitting = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   title,"width":440,onclose,children:() => { async ()/*Ωignore_positionΩ*/ => {
    if(accountsLoading){
        { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {    "what":`your publishing accounts`,"variant":`compact`,"loading":true,"empty":true,}});}
    } else if (accounts.length === 0 && formError){
      
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: formError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`field-error`,});
        formError;
        if(formErrorDetail){ { svelteHTML.createElement("div", { "class":`pd-error-detail`,});formErrorDetail; }}
       }}
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { formError = ''; formErrorDetail = ''; void loadAccounts(); },});  }
    } else if (accounts.length === 0){
       { svelteHTML.createElement("div", { "class":`no-accounts`,});mode === 'story' ? 'No Jira or Linear account is connected yet — add one to publish.' : 'No Jira (Atlassian) account is connected yet — Confluence needs one to publish.'; }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { onclose(); router.go('settings/jira'); },});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    
       }
    }else{
      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", {   "class":`label`,"for":`pd-account`,});  }
         { svelteHTML.createElement("select", {           "id":`pd-account`,"class":`input`,"bind:value":accountId,"onchange":onAccountChange,"disabled":submitting,});/*Ωignore_startΩ*/() => accountId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let a of __sveltets_2_ensureArray(accounts)){a.id;
             { svelteHTML.createElement("option", { "value":a.id,});a.label; a.base_url;  }
          }
         }
       }

      if(mode === 'story'){
        
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", {   "class":`label`,"for":`pd-project`,});tracker === 'Linear' ? 'Team' : 'Project'; }
          if(projectsLoading){
             { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`projects`,}});}
          } else if (projectsError){
             {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: projectsError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`field-error`,});projectsError; }}
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":loadProjects,});  }
          }else{
             { svelteHTML.createElement("select", {           "id":`pd-project`,"class":`input`,"bind:value":projectKey,"onchange":loadIssueTypes,"disabled":submitting || projects.length === 0,});/*Ωignore_startΩ*/() => projectKey = __sveltets_2_any(null);/*Ωignore_endΩ*/
              if(projects.length === 0){
                 { svelteHTML.createElement("option", {"value":"",});   }
              }else{
                   for(let p of __sveltets_2_ensureArray(projects)){p.key;
                   { svelteHTML.createElement("option", { "value":p.key,});p.name; p.key;  }
                }
              }
             }
            if(projectsTruncated){
               { svelteHTML.createElement("p", {   "class":`trunc-note`,"role":`note`,});truncatedNote('projects', projects.length); }
            }
          }
         }

        
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", {   "class":`label`,"for":`pd-issuetype`,});  }
          if(issueTypesLoading){
             { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`issue types`,}});}
          }else{
             { svelteHTML.createElement("select", {         "id":`pd-issuetype`,"class":`input`,"bind:value":issueType,"disabled":submitting,});/*Ωignore_startΩ*/() => issueType = __sveltets_2_any(null);/*Ωignore_endΩ*/
              if(issueTypes.length === 0){
                 { svelteHTML.createElement("option", { "value":`Story`,});  }
              }else{
                   for(let t of __sveltets_2_ensureArray(issueTypes)){t;
                   { svelteHTML.createElement("option", { "value":t,});t; }
                }
              }
             }
          }
         }

      }else{
        
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", {   "class":`label`,"for":`pd-space`,});  }
          if(spacesLoading){
             { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`spaces`,}});}
          } else if (spacesError){
             {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: spacesError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`field-error`,});spacesError; }}
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":loadSpaces,});  }
          }else{
             { svelteHTML.createElement("select", {         "id":`pd-space`,"class":`input`,"bind:value":spaceKey,"disabled":submitting || spaces.length === 0,});/*Ωignore_startΩ*/() => spaceKey = __sveltets_2_any(null);/*Ωignore_endΩ*/
              if(spaces.length === 0){
                 { svelteHTML.createElement("option", {"value":"",});   }
              }else{
                   for(let sp of __sveltets_2_ensureArray(spaces)){sp.key;
                   { svelteHTML.createElement("option", { "value":sp.key,});sp.name; sp.key;  }
                }
              }
             }
            if(spacesTruncated){
               { svelteHTML.createElement("p", {   "class":`trunc-note`,"role":`note`,});truncatedNote('spaces', spaces.length); }
            }
          }
         }

        
         { svelteHTML.createElement("div", { "class":`field`,});
           { const $$_rekciPegaPecneulfnoC1C = __sveltets_2_ensureComponent(ConfluencePagePicker); new $$_rekciPegaPecneulfnoC1C({ target: __sveltets_2_any(), props: {             "id":`pd-parent`,accountId,spaceKey,"value":parentId,"title":parentTitle,"disabled":submitting,"onchange":(id, t) => { parentId = id; parentTitle = t; },}});}
         }

        
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", {   "class":`label`,"for":`pd-title`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
           { svelteHTML.createElement("input", {              "dir":`auto`,"id":`pd-title`,"class":`input`,"bind:value":rfcTitle,"placeholder":`Defaults to story title`,"spellcheck":`false`,"disabled":submitting,});/*Ωignore_startΩ*/() => rfcTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }

      
       { svelteHTML.createElement("div", {   "class":`pd-preview`,...__sveltets_2_empty({"data-testid":`publish-preview`}),});
         { svelteHTML.createElement("div", { "class":`label`,});   }
         { svelteHTML.createElement("div", { "class":`pd-preview-title`,});(mode === 'rfc' && rfcTitle.trim()) || storyTitle || 'Untitled'; }
        if(previewError){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: previewError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`field-error`,});previewError; }}
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void loadPreview(product.selectedId),});  }
        } else if (previewBody === null){
           { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":4,"height":14,"label":`the content`,}});}
        } else if (previewLines.head.length === 0){
           { svelteHTML.createElement("div", { "class":`loading-inline`,});        }
        }else{
           { svelteHTML.createElement("pre", { "class":`pd-preview-body`,});previewLines.head.join('\n'); }
          if(previewLines.more > 0){
             { svelteHTML.createElement("details", {});
               { svelteHTML.createElement("summary", { "class":`pd-preview-more`,});  plural(previewLines.head.length + previewLines.more, 'line'); }
               { svelteHTML.createElement("pre", { "class":`pd-preview-body`,});previewLines.body; }
             }
          }
        }
        if(visibility){
           { svelteHTML.createElement("div", {   "class":`pd-visibility`,...__sveltets_2_empty({"data-testid":`publish-visibility`}),});visibility; }
        }
        if(isDraft){
           { svelteHTML.createElement("div", { "class":`pd-visibility`,});     mode === 'story' ? 'issue' : 'page';  }
        }
       }

      if(previewStoryId && previewStoryId !== product.selectedId){
        
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(STORY_CHANGED)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`field-error`,...__sveltets_2_empty({"data-testid":`publish-story-changed`}),});
          STORY_CHANGED;
         }}
      }
      if(formError){
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: formError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`field-error`,});
          formError;
          if(formErrorDetail){ { svelteHTML.createElement("div", { "class":`pd-error-detail`,});formErrorDetail; }}
         }}
      }
    }
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn ghost`,"onclick":onclose,"disabled":submitting,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":submit,"disabled":submitting || accountsLoading || accounts.length === 0 || previewBody === null || !reviewedContent || !!previewError || previewStoryId !== product.selectedId,});
      submitting ? 'Publishing…' : (mode === 'story' ? 'Publish story' : 'Publish RFC');
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PublishDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PublishDialog__SvelteComponent_ = ReturnType<typeof PublishDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default PublishDialog__SvelteComponent_;