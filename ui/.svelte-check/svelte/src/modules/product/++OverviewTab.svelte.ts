///<reference types="svelte" />
;
import { rel } from '../../lib/stores/now.svelte';
import { formatBytes } from '../../lib/metric-format';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import Badge from '../../lib/components/Badge.svelte';
// Overview tab — shows the selected story's detail: title, source link, stage
// badge, issue_type, a version dropdown (with body_md rendering), Refresh
// button, watch toggle, and (for Jira stories) a rich section with status,
// assignee, details, linked issues, comments, history, and attachments (the
// Details sidebar is JiraIssueSection).
import { untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { kindLabel } from '../../lib/labels';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { focusOnMount } from '../../lib/focusOnMount';
import { refocusEditTrigger } from '../../lib/inlineEdit';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { storyStage, STORY_STAGES } from '../../lib/status';
import { product } from '../../lib/stores/product.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { renderMarkdown } from '../../lib/md';
import { toasts } from '../../lib/toast.svelte';
import { AttachmentUrls } from './attachmentUrls.svelte';
import * as issues from '../../lib/api/issues';
import type { ProductStoryVersion } from './types';
import type { ProductAttachment } from './types';
import { router } from '../../lib/router.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmOutward } from '../../lib/confirmOutward';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import PublishDialog from './PublishDialog.svelte';
import { reseedDraft, type DraftSeed } from './draftSeed';
import SwarmLinkCard from './SwarmLinkCard.svelte';
import AttachmentsPanel from './AttachmentsPanel.svelte';
import DraftEditor from './DraftEditor.svelte';
import LinkedCanvases from './LinkedCanvases.svelte';
import ChildrenBoard from './ChildrenBoard.svelte';
import { escapeHtml, unescapeHtml } from '../../lib/html';
import JiraIssueSection from './JiraIssueSection.svelte';
import { JiraIssue } from './jiraIssue.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Version picker state — null means "show current (source) version".
  let viewingVersion = $state<ProductStoryVersion | null>(null);
  let versionsLoaded = $state(false);
  let versionLoading = $state(false);
  let refreshing = $state(false);
  let watchWorking = $state(false);

  // ── Discovery launch state ────────────────────────────────────────────────
  let discoverySwarmId = $state('');
  let runningDiscovery = $state(false);

  // The detail object for the currently selected story.
  const detail = $derived(product.detail);

  // ProductPage keys this tab by story ({#key product.selectedId}), so every
  // story gets a fresh instance. A request started by an instance can still
  // settle after it was torn down (or after A → B → A reopened A in a NEW
  // instance), so each loader/writer captures its story and re-checks this
  // after every await before assigning state or sending a Jira write.
  let destroyed = false;
  $effect(() => () => { destroyed = true; });
  function onStory(s: { id: string }): boolean {
    return !destroyed && product.selectedId === s.id;
  }
  const story = $derived(detail?.story ?? null);
  const source = $derived(detail?.source ?? null);

  // Rendered markdown: version override wins over the live source body.
  const bodyMd = $derived((viewingVersion ?? source)?.body_md ?? '');
  const renderedBody = $derived(bodyMd ? renderMarkdown(bodyMd) : '');

  // ── Jira issue: the full issue, dev status and the field editor ──────────
  // (jiraIssue.svelte.ts). JiraIssueSection renders its Details sidebar and
  // starts the load; the body column reads the comments/history/attachments.
  const ji = new JiraIssue({
    story: () => story,
    isCurrent: (s) => onStory(s),
    afterTransition: () => product.refresh(),
  });
  const issueFull = $derived(ji.full);
  const jira = $derived(ji.editor);

  // ── Manual title / description edit (Jira stories) ────────────────────────
  // Jira exposes summary + description as first-class editable content — the
  // generic editmeta field editor deliberately filters both out, so they get
  // dedicated inline editors here (title next to the header, description in the
  // body column, mirroring Jira's own inline-edit affordances).
  let editingTitle = $state(false);
  let titleDraft = $state('');
  /** Inline validation for the title editor — Save stays disabled while empty. */
  const titleEmpty = $derived(!titleDraft.trim());
  let titleSaving = $state(false);
  let editingDesc = $state(false);
  let descDraft = $state('');
  let descSaving = $state(false);

  // Collapsible sections
  let collapsed = $state<Record<string, boolean>>({
    details: false,
    links: false,
    development: true,
    comments: false,
    history: true,
    attachments: false,
  });

  // Attachment object URL cache: id → url
  let attachmentUrls = $state<Record<string, string>>({});
  let attachmentLoading = $state<Record<string, boolean>>({});
  // Track all created object URLs for revocation on unmount.
  let createdObjectUrls: string[] = [];

  const isJira = $derived(story?.source_kind === 'jira');
  const isDraft = $derived(story?.source_kind === 'draft');
  const isConfluence = $derived(story?.source_kind === 'confluence');



  // AttachmentsPanel instance (bound for the Jira / Confluence layouts).
  let panelRef = $state<{ uploadBlob: (blob: Blob, opts?: { filename?: string; kind?: string }) => Promise<ProductAttachment> } | null>(null);

  // ── Body preview: attachment token resolution ─────────────────────────────
  // Resolved authed blob URLs for `attachment:<id>` refs found in renderedBody
  // (attachmentUrls.svelte.ts, shared with AttachmentsPanel: de-duplicated
  // in-flight fetches, revoked on reset). Separate from the Jira attachmentUrls.
  const bodyAtts = new AttachmentUrls();
  /** Bumped per resolve pass and on story change — a pass that settles after a
   *  newer one (or after a switch) drops its result and frees its blobs. */
  let resolveSeq = 0;

  // HTML of renderedBody with markdown image tokens rewritten to <img> tags with blob URLs.
  let resolvedBody = $state('');

  // Track the last rendered body to avoid redundant re-resolution.
  let lastResolvedInput = $state('');


  // ── Add comment state ─────────────────────────────────────────────────────
  let newCommentBody = $state('');
  let postingComment = $state(false);

  // ── Publish dialog state ──────────────────────────────────────────────────
  let publishDialogMode = $state<'story' | 'rfc' | null>(null);

  // ── Tags state ────────────────────────────────────────────────────────────
  let tagInput = $state('');
  let tagSaving = $state(false);

  /** Parse comma-separated tags string → deduplicated, trimmed, non-empty array. */
  function parseTags(csv: string): string[] {
    return [...new Set(csv.split(',').map((t) => t.trim()).filter(Boolean))];
  }

  /** Join tag array back to canonical csv. */
  function joinTags(tags: string[]): string {
    return tags.join(',');
  }

  /** Current tags as array (derived from live story). */
  const currentTags = $derived(parseTags(story?.tags ?? ''));

  /** Related stories: share ≥1 tag with current story, from the stories list. */
  const relatedStories = $derived(
    currentTags.length === 0
      ? []
      : product.stories.filter(
          (s) =>
            s.id !== story?.id &&
            parseTags(s.tags).some((t) => currentTags.includes(t)),
        ),
  );

  async function addTag(): Promise<void> {
    const tag = tagInput.trim();
    if (!tag || !story) return;
    const updated = joinTags([...new Set([...currentTags, tag])]);
    tagSaving = true;
    tagInput = '';
    try {
      await product.updateStory({ tags: updated });
    } catch (e) {
      toastError('Couldn’t save tag', e);
    } finally {
      tagSaving = false;
    }
  }

  async function removeTag(tag: string): Promise<void> {
    if (!story) return;
    const updated = joinTags(currentTags.filter((t) => t !== tag));
    try {
      await product.updateStory({ tags: updated });
    } catch (e) {
      toastError('Couldn’t remove tag', e);
    }
  }

  // ── Reset per-story state on a story switch ──────────────────────────────

  $effect(() => {
    // Track selectedId so this re-runs on story change.
    product.selectedId;
    viewingVersion = null;
    versionsLoaded = false;
    // Reset Jira state (issue, dev status, editor; loading flags too, S18-02).
    // Inline editors close so A's text can never be saved "to B" (S18-01).
    ji.reset();
    editingTitle = false;
    titleDraft = '';
    editingDesc = false;
    descDraft = '';
    ++resolveSeq;
    collapsed = { details: false, links: false, development: true, comments: false, history: true, attachments: false };
    newCommentBody = '';
    postingComment = false;
    // Revoke old Jira attachment object URLs.
    for (const url of createdObjectUrls) {
      URL.revokeObjectURL(url);
    }
    createdObjectUrls = [];
    attachmentUrls = {};
    attachmentLoading = {};
    // Revoke body-preview attachment token object URLs.
    bodyAtts.reset();
    resolvedBody = '';
    lastResolvedInput = '';
  });

  // Cleanup object URLs on unmount.
  $effect(() => {
    return () => {
      for (const url of createdObjectUrls) {
        URL.revokeObjectURL(url);
      }
      bodyAtts.dispose();
    };
  });

  // Load swarms lazily so the discovery team picker is populated.
  $effect(() => {
    const wsId = ws.currentId;
    if (wsId) void swarm.ensureSwarms(wsId);
  });

  /** Launch a discovery swarm run and switch to the Discovery tab. */
  async function runDiscovery(): Promise<void> {
    if (runningDiscovery) return;
    const targetSwarm = swarm.swarms.find((s) => s.id === discoverySwarmId);
    const teamName = targetSwarm ? `“${targetSwarm.name}”` : 'a swarm';
    const attCount = 0; // attachments listed in the panel; count not tracked here
    const ok = await confirmer.ask(
      `Run Discovery in ${teamName}? This will START the swarm and send the story info${attCount > 0 ? ` + ${attCount} attachments` : ''} as discovery context.`,
      { title: 'Run discovery', confirmLabel: 'Run discovery', danger: false },
    );
    if (!ok) return;
    runningDiscovery = true;
    try {
      await product.discover(discoverySwarmId ? { swarm_id: discoverySwarmId } : {});
      toasts.success('Discovery started', 'The swarm is now analyzing the story.');
      await product.changeTab('discovery');
    } catch (e) {
      toastError('Couldn’t start discovery', e);
    } finally {
      runningDiscovery = false;
    }
  }

  // ── Body preview: resolve markdown image tokens in rendered HTML ────────
  // renderMarkdown HTML-escapes `![alt](attachment:<id>)` literally — it never
  // emits src="attachment:<id>". We scan the rendered HTML for those literal
  // markdown image tokens, fetch authed blob URLs, and replace each token with
  // an <img> tag so pasted screenshots appear inline. `draftBody` / source
  // body_md keep the portable `attachment:<id>` form; blob URLs only exist here.
  $effect(() => {
    const html = renderedBody;
    if (!html || html === lastResolvedInput) return;
    lastResolvedInput = html;
    // Untracked: the resolver's synchronous cache reads/writes are not inputs.
    const seq = ++resolveSeq;
    untrack(() => void resolveAttachmentTokens(html, seq));
  });

  async function resolveAttachmentTokens(html: string, seq: number): Promise<void> {
    // Match literal markdown image syntax `![alt](attachment:<id>)` in the HTML
    // (renderMarkdown outputs these as-is since it doesn't handle image syntax).
    const tokenRe = /!\[([^\]]*)\]\(attachment:([A-Za-z0-9]+)\)/g;
    const matches: Array<{ full: string; alt: string; id: string }> = [];
    let m: RegExpExecArray | null;
    while ((m = tokenRe.exec(html)) !== null) {
      matches.push({ full: m[0], alt: m[1], id: m[2] });
    }
    if (matches.length === 0) {
      resolvedBody = html;
      return;
    }
    // Fetch any uncached URLs (the cache shares an in-flight fetch between
    // overlapping passes and frees a blob that lands after a story switch).
    const urls: Record<string, string> = {};
    await Promise.all(
      matches.map(async ({ id }) => {
        const url = await bodyAtts.load(id);
        if (url) urls[id] = url;
        else if (bodyAtts.errors[id]) console.warn('[OverviewTab] attachment token resolution failed', id, bodyAtts.errors[id]);
      }),
    );
    // A newer pass (or a story switch) owns resolvedBody now: never let A's
    // body replace B's description.
    if (destroyed || seq !== resolveSeq) return;
    // Replace each markdown image token with an <img> tag using the blob URL.
    let out = html;
    for (const { full, alt, id } of matches) {
      const blobUrl = urls[id];
      if (blobUrl) {
        // `alt` comes from already-rendered HTML (entities escaped once):
        // decode, then escape for the attribute — escaping again showed
        // "&amp;amp;". A replacer function keeps `$&`/`$1` in alt literal.
        const img = `<img class="body-attachment" alt="${escapeHtml(unescapeHtml(alt))}" src="${blobUrl}">`;
        out = out.replace(full, () => img);
      }
    }
    resolvedBody = out;
  }

  // Status, assignee and the editmeta field editor live in jira/ (the
  // `JiraIssueEditor` + its IssueEditor field component, hosted by
  // JiraIssueSection); title, description
  // and comments stay here because they edit the story the tab renders.

  // ── Title (summary) edit ──────────────────────────────────────────────────
  function beginEditTitle(): void {
    if (!story) return;
    titleDraft = story.title;
    editingTitle = true;
  }
  function cancelEditTitle(): void {
    editingTitle = false;
    titleDraft = '';
    void refocusEditTrigger('title');
  }
  /** Save the story summary via the generic fields endpoint (native string). */
  async function saveTitle(): Promise<void> {
    if (!story) return;
    const s = story;
    const next = titleDraft.trim();
    if (!next) return; // the empty field says so inline (titleEmpty)
    if (next === s.title) {
      cancelEditTitle();
      return;
    }
    // The summary is a live Jira write like every other in this file: name the
    // issue and show old → new before sending (watchers are notified).
    const ok = await confirmOutward({
      verb: 'Rename issue',
      title: `Rename ${s.source_key}?`,
      where: jira.where(),
      what: `Title: “${s.title}” → “${next}”`,
      who: `Everyone with access to ${s.source_key} sees the new title; watchers are notified.`,
    });
    if (!ok || !onStory(s)) return;
    titleSaving = true;
    try {
      const full = await issues.updateFields(s.account_id, s.source_key, { summary: next });
      // Reflect the new title in the header + story list without a full refresh.
      product.patchLocalTitle(s.id, next);
      toasts.info('Title updated', s.source_key);
      if (!onStory(s)) return;
      ji.full = full;
      cancelEditTitle();
    } catch (e) {
      toastError('Couldn’t update title', e);
    } finally {
      titleSaving = false;
    }
  }

  // ── Description edit ──────────────────────────────────────────────────────
  function beginEditDesc(): void {
    descDraft = bodyMd;
    editingDesc = true;
  }
  function cancelEditDesc(): void {
    editingDesc = false;
    descDraft = '';
    void refocusEditTrigger('description');
  }
  /** Save the description via the dedicated (ADF-aware) description endpoint. */
  async function saveDesc(): Promise<void> {
    if (!story) return;
    const s = story;
    const draft = descDraft;
    // The whole description is replaced upstream (not merged) — the one inline
    // Jira write that can silently lose someone else's text, so it asks first.
    const ok = await confirmOutward({
      verb: `Replace description`,
      title: `Replace the description of ${s.source_key}?`,
      where: jira.where(),
      what: draft.trim() || '(empty description)',
      who: `Everyone with access to ${s.source_key} sees the new text; the previous description is only in Jira’s history.`,
    });
    if (!ok || !onStory(s)) return;
    descSaving = true;
    try {
      const full = await issues.updateDescription(s.account_id, s.source_key, draft);
      // The body column reads from the cached source version — patch it locally
      // with exactly what we just sent so the change shows without a refresh.
      product.patchLocalBody(s.id, draft);
      toasts.info('Description updated', s.source_key);
      if (!onStory(s)) return;
      ji.full = full;
      editingDesc = false;
      descDraft = '';
    } catch (e) {
      toastError('Couldn’t update description', e);
    } finally {
      descSaving = false;
    }
  }

  async function loadAttachmentUrl(attId: string): Promise<void> {
    if (!story || attachmentUrls[attId] || attachmentLoading[attId]) return;
    const s = story;
    attachmentLoading = { ...attachmentLoading, [attId]: true };
    try {
      const url = await issues.attachmentBlobUrl(s.account_id, s.source_key, attId);
      if (!onStory(s)) {
        URL.revokeObjectURL(url);
        return;
      }
      createdObjectUrls.push(url);
      attachmentUrls = { ...attachmentUrls, [attId]: url };
    } catch (e) {
      console.warn('[OverviewTab] attachment load failed', attId, e);
    } finally {
      if (onStory(s)) attachmentLoading = { ...attachmentLoading, [attId]: false };
    }
  }

  function toggleSection(key: string): void {
    collapsed = { ...collapsed, [key]: !collapsed[key] };
    // Lazy-load attachment previews when section opens.
    if (key === 'attachments' && !collapsed[key] && issueFull) {
      for (const att of issueFull.attachments) {
        if (att.mime.startsWith('image/') || att.mime === 'application/pdf') {
          void loadAttachmentUrl(att.id);
        }
      }
    }
  }

  async function loadVersions(): Promise<void> {
    if (versionsLoaded) return;
    try {
      await product.loadVersions();
      versionsLoaded = true;
    } catch (e) {
      toastError('Couldn’t load versions', e);
    }
  }

  async function onVersionChange(e: Event): Promise<void> {
    const vid = (e.target as HTMLSelectElement).value;
    if (!vid) {
      viewingVersion = null;
      return;
    }
    versionLoading = true;
    try {
      viewingVersion = await product.getVersion(vid);
    } catch (err) {
      toastError('Couldn’t load version', err);
    } finally {
      versionLoading = false;
    }
  }

  async function refresh(): Promise<void> {
    refreshing = true;
    try {
      await product.refresh();
      viewingVersion = null;
      toasts.info('Story refreshed');
      // Re-fetch IssueFull after refresh.
      if (isJira) {
        await ji.load();
        await ji.reloadDev();
      }
    } catch (e) {
      toastError('Couldn’t refresh the story', e);
    } finally {
      refreshing = false;
    }
  }

  async function toggleWatch(): Promise<void> {
    if (!story) return;
    watchWorking = true;
    try {
      await product.updateStory({ watch_enabled: !story.watch_enabled });
    } catch (e) {
      toastError('Couldn’t update watch', e);
    } finally {
      watchWorking = false;
    }
  }

  async function addComment(): Promise<void> {
    if (!story || !newCommentBody.trim()) return;
    const s = story;
    const key = s.source_key;
    const body = newCommentBody.trim();
    const ok = await confirmOutward({
      verb: 'Post comment',
      title: `Post comment to ${key}?`,
      where: jira.where(),
      what: body,
      who: `Everyone with access to ${key} in Jira; watchers are notified.`,
    });
    if (!ok || !onStory(s)) return;
    postingComment = true;
    try {
      await issues.addComment(s.account_id, s.source_key, body);
      newCommentBody = '';
      toasts.success('Comment posted');
      await ji.load();
    } catch (e) {
      toastError('Couldn’t post comment', e);
    } finally {
      postingComment = false;
    }
  }

  // Lifecycle gate: the operator advances the story through the stages. Approval
  // is the gate before "Send to Swarm" (PlanTab warns when not approved). The
  // stage reads through the shared `storyStage` mapping (lib/status.ts); the
  // picker is a ctxMenu of checkable rows. An agent-written stage (analyzed,
  // planned…) stays listed while it is current.
  function openStageMenu(e: MouseEvent): void {
    if (!story) return;
    const cur = story.stage;
    const keys: string[] = [...STORY_STAGES];
    if (!keys.includes(cur)) keys.unshift(cur);
    ctxMenu.showAt(
      e.currentTarget as HTMLElement,
      keys.map((k) => ({ label: storyStage(k).label, checked: k === cur, action: () => void setStage(k) })),
    );
  }
  async function setStage(stage: string): Promise<void> {
    if (!story || stage === story.stage) return;
    try {
      await product.updateStory({ stage });
    } catch (e) {
      toastError('Couldn’t update stage', e);
    }
  }

;
async () => {

if(product.loadingDetail && (!detail || detail.story.id !== product.selectedId)){
  
   { svelteHTML.createElement("div", { "class":`loading`,}); { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":5,"height":36,"label":`story`,}});} }
} else if (!detail || !story){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`overview`,});
    
     { svelteHTML.createElement("div", { "class":`story-header`,});
       { svelteHTML.createElement("div", { "class":`story-meta-row`,});
         { svelteHTML.createElement("button", {           "class":`ov-stage-btn`,"onclick":openStageMenu,"aria-haspopup":`menu`,"aria-label":`Lifecycle stage: ${storyStage(story.stage).label}. Change stage`,"title":`Lifecycle stage — advance to Approved before sending to a swarm`,});
            { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":storyStage(story.stage),"title":"",}});}
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
        if(story.issue_type){
           { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":story.issue_type,}});}
        }
        if(story.url){
           { svelteHTML.createElement("a", {         "class":`source-link mono`,"href":story.url,"target":`_blank`,"rel":`noopener noreferrer`,"title":`Open in source`,});
            story.source_key;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
           }
        }else{
           { svelteHTML.createElement("span", { "class":`source-key mono`,});story.source_key; }
        }
       }
      if(editingTitle){
         { svelteHTML.createElement("div", { "class":`title-edit`,});
           {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {                 "dir":`auto`,"class":`title-input`,"bind:value":titleDraft,"spellcheck":`false`,"aria-label":`Story title`,"aria-invalid":titleEmpty,"aria-describedby":titleEmpty ? 'ov-title-err' : undefined,"onkeydown":(e) => {
              if (e.key === 'Enter') { e.preventDefault(); void saveTitle(); }
              else if (e.key === 'Escape') { e.preventDefault(); cancelEditTitle(); }
            },});/*Ωignore_startΩ*/() => titleDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
           { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":saveTitle,"disabled":titleSaving || titleEmpty,"title":titleEmpty ? 'Give the story a title first' : 'Writes to the live Jira issue — everyone with access sees it',});
            titleSaving ? 'Saving…' : `Save to ${story.source_key}`;
           }
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":cancelEditTitle,"disabled":titleSaving,});  }
          if(titleEmpty){ { svelteHTML.createElement("p", {   "class":`title-err`,"id":`ov-title-err`,});     }}
         }
      } else if (!isDraft){
        
         { svelteHTML.createElement("div", { "class":`title-row`,});
           { svelteHTML.createElement("h2", { "class":`story-title`,});story.title; }
          if(isJira){
             { svelteHTML.createElement("button", {           "class":`title-edit-btn`,...__sveltets_2_empty({"data-edit-for":`title`}),"title":`Edit title`,"aria-label":`Edit title`,"onclick":beginEditTitle,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});} }
          }
         }
      }

      
       { svelteHTML.createElement("div", { "class":`counts-row`,});
         { svelteHTML.createElement("span", {   "class":`count-chip`,"title":`Versions`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":12,}});}plural(detail.counts.versions, 'version'); }
         { svelteHTML.createElement("span", {   "class":`count-chip`,"title":`Analyses`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`gauge`,"size":12,}});}plural(detail.counts.analyses, 'analysis', 'analyses'); }
         { svelteHTML.createElement("span", {   "class":`count-chip`,"title":`Open questions`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`comment`,"size":12,}});}plural(detail.counts.open_questions, 'open question'); }
         { svelteHTML.createElement("span", {   "class":`count-chip`,"title":`Notes`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});}plural(detail.counts.notes, 'note'); }
         { svelteHTML.createElement("span", {   "class":`count-chip`,"title":`Test cases`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}plural(detail.counts.testcases, 'test'); }
       }

      
       { svelteHTML.createElement("div", { "class":`tags-row`,});
           for(let tag of __sveltets_2_ensureArray(currentTags)){tag;
           { svelteHTML.createElement("span", { "class":`tag-chip`,});
            tag;
             { svelteHTML.createElement("button", {         "class":`tag-remove`,"onclick":() => removeTag(tag),"aria-label":`Remove tag ${tag}`,"title":`Remove tag`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
           }
        }
         { svelteHTML.createElement("form", {     "class":`tag-add-form`,"onsubmit":(e) => { e.preventDefault(); void addTag(); },});
           { svelteHTML.createElement("input", {              "dir":`auto`,"class":`tag-input`,"bind:value":tagInput,"placeholder":`Add tag…`,"disabled":tagSaving,"aria-label":`Add tag`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tagInput = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
     }

    
     { svelteHTML.createElement("div", { "class":`toolbar`,});
      
       { svelteHTML.createElement("div", { "class":`version-sel`,});
         { svelteHTML.createElement("label", {   "class":`ver-label`,"for":`ov-version-select`,});  }
         { svelteHTML.createElement("select", {           "id":`ov-version-select`,"class":`ver-select`,"onchange":onVersionChange,"onfocus":loadVersions,"disabled":versionLoading,});
           { svelteHTML.createElement("option", {"value":"",}); source ? `v${source.version_no}` : 'none';  }
             for(let v of __sveltets_2_ensureArray(product.versions)){v.id;
            if(v.id !== source?.id){
               { svelteHTML.createElement("option", { "value":v.id,}); v.version_no;  kindLabel(v.kind); new Date(v.created_at).toLocaleDateString();  }
            }
          }
         }
        if(versionLoading){
           { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading the version`,"title":`Loading the version`,}); }
        }
       }

       { svelteHTML.createElement("span", { "class":`grow`,}); }

      if(!isDraft){
      
      
       { svelteHTML.createElement("button", {           "class":`btn small`,"aria-pressed":story.watch_enabled,"onclick":toggleWatch,"disabled":watchWorking,"title":story.watch_enabled ? 'Watching the source for changes — click to stop' : 'Watch the source for changes',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":13,}});}
        story.watch_enabled ? 'Watching' : 'Watch';
       }

      
       { svelteHTML.createElement("button", {           "class":`btn small`,"onclick":refresh,"disabled":refreshing,"title":`Pull latest content from source`,"aria-label":`Refresh story`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
        refreshing ? 'Refreshing…' : 'Refresh';
       }
      }

      
      if(swarm.swarms.length > 1){
         { svelteHTML.createElement("select", {       "class":`disc-swarm-pick`,"bind:value":discoverySwarmId,"title":`Which swarm runs the discovery`,});/*Ωignore_startΩ*/() => discoverySwarmId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let s of __sveltets_2_ensureArray(swarm.swarms)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.name; }}
         }
      }
       { svelteHTML.createElement("button", {           "class":`btn small`,"onclick":runDiscovery,"disabled":runningDiscovery,"title":`Launch a discovery swarm run — agents analyze the story and report findings`,"aria-label":`Run discovery`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});} runningDiscovery ? 'Starting…' : 'Run discovery';
       }
     }

    
    if(isDraft){
      
       { const $$_rotidEtfarD1C = __sveltets_2_ensureComponent(DraftEditor); new $$_rotidEtfarD1C({ target: __sveltets_2_any(), props: {    story,source,"onpublish":(mode) => (publishDialogMode = mode),}});}

    } else if (isJira){
      
       { svelteHTML.createElement("div", { "class":`two-col jira-layout`,});
        
         { svelteHTML.createElement("div", { "class":`col-left`,});
           { svelteHTML.createElement("div", { "class":`body-wrap`,});
            if(viewingVersion){
               { svelteHTML.createElement("div", { "class":`version-banner`,});
                 viewingVersion.version_no; kindLabel(viewingVersion.kind);
                 new Date(viewingVersion.created_at).toLocaleString();
                if(viewingVersion.change_notes){
                   { svelteHTML.createElement("span", { "class":`change-notes`,}); viewingVersion.change_notes; }
                }
               }
            }

            
             { svelteHTML.createElement("div", { "class":`desc-header`,});
               { svelteHTML.createElement("span", { "class":`desc-label`,});  }
              if(!editingDesc && !viewingVersion){
                 { svelteHTML.createElement("button", {       "class":`desc-edit-btn`,...__sveltets_2_empty({"data-edit-for":`description`}),"aria-label":`Edit description`,"onclick":beginEditDesc,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} 
                 }
              }
             }

            if(editingDesc){
               { svelteHTML.createElement("div", { "class":`desc-editor`,});
                 {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('textarea')));{ svelteHTML.createElement("textarea", __sveltets_2_union($$action_0), {                 "dir":`auto`,"class":`desc-textarea`,"bind:value":descDraft,"rows":16,"placeholder":`e.g. ## Goal — let players cap their weekly deposits`,"spellcheck":`false`,"aria-label":`Description (Markdown)`,"onkeydown":(e) => { if (e.key === 'Escape') { e.preventDefault(); cancelEditDesc(); } },});/*Ωignore_startΩ*/() => descDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }}
                 { svelteHTML.createElement("div", { "class":`desc-editor-actions`,});
                   { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":saveDesc,"disabled":descSaving,"title":`Overwrites the live Jira description — everyone with access sees it`,});
                    descSaving ? 'Saving…' : `Save to ${story.source_key}`;
                   }
                   { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":cancelEditDesc,"disabled":descSaving,});  }
                 }
               }
            } else if (renderedBody){
              
               { svelteHTML.createElement("div", { "class":`md-body`,}); resolvedBody || renderedBody; }
            }else{
               { svelteHTML.createElement("div", { "class":`muted`,});              }
            }
           }

          
          if(issueFull){
             { svelteHTML.createElement("div", { "class":`jira-section jira-activity`,});
              
               { svelteHTML.createElement("div", { "class":`jira-card collapsible-card`,});
                 { svelteHTML.createElement("button", {       "class":`jira-coll-trigger`,"onclick":() => toggleSection('comments'),"aria-expanded":!collapsed.comments,});
                   { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":collapsed.comments ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                   { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
                   { svelteHTML.createElement("span", { "class":`section-count`,}); issueFull.comments.length;  }
                 }
                if(!collapsed.comments){
                   { svelteHTML.createElement("div", { "class":`comments-list`,});
                    if(issueFull.comments.length === 0){
                       { svelteHTML.createElement("div", { "class":`comments-empty`,});   }
                    }else{
                         for(let comment of __sveltets_2_ensureArray(issueFull.comments)){comment.id;
                         { svelteHTML.createElement("div", { "class":`comment-item`,});
                           { svelteHTML.createElement("div", { "class":`comment-meta`,});
                             { svelteHTML.createElement("span", { "class":`comment-author`,});comment.author; }
                             { svelteHTML.createElement("span", {   "class":`comment-date`,"title":new Date(comment.created).toLocaleString(),});rel(comment.created); }
                           }
                           { svelteHTML.createElement("div", { "class":`comment-body md-body`,}); renderMarkdown(comment.body_md); }
                         }
                      }
                    }
                   }
                  
                   { svelteHTML.createElement("div", { "class":`add-comment-form`,});
                     { svelteHTML.createElement("textarea", {                "dir":`auto`,"class":`textarea comment-textarea`,"bind:value":newCommentBody,"rows":3,"aria-label":`Comment`,"placeholder":`e.g. Should the limit also cover bonus funds?`,"spellcheck":`true`,"disabled":postingComment,});/*Ωignore_startΩ*/() => newCommentBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                     { svelteHTML.createElement("div", { "class":`add-comment-row`,});
                       { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":addComment,"disabled":postingComment || !newCommentBody.trim(),});
                        postingComment ? 'Posting…' : `Post comment to ${story.source_key}…`;
                       }
                     }
                   }
                }
               }

              
              if(issueFull.history && issueFull.history.length > 0){
                 { svelteHTML.createElement("div", { "class":`jira-card collapsible-card`,});
                   { svelteHTML.createElement("button", {       "class":`jira-coll-trigger`,"onclick":() => toggleSection('history'),"aria-expanded":!collapsed.history,});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":collapsed.history ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                     { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
                     { svelteHTML.createElement("span", { "class":`section-count`,}); issueFull.history.length;  }
                   }
                  if(!collapsed.history){
                     { svelteHTML.createElement("div", { "class":`history-list`,});
                          for(let entry of __sveltets_2_ensureArray(issueFull.history)){let i = 1;i;
                         { svelteHTML.createElement("div", { "class":`history-entry`,});
                           { svelteHTML.createElement("div", { "class":`history-meta`,});
                             { svelteHTML.createElement("span", { "class":`history-author`,});entry.author; }
                             { svelteHTML.createElement("span", {   "class":`history-date`,"title":new Date(entry.created).toLocaleString(),});rel(entry.created); }
                           }
                              for(let item of __sveltets_2_ensureArray(entry.items)){let j = 1;j;
                             { svelteHTML.createElement("div", { "class":`history-change`,});
                                { svelteHTML.createElement("span", { "class":`history-field`,});item.field; }
                              if(item.from){  { svelteHTML.createElement("span", { "class":`history-val`,});item.from; }
                              }
                                { svelteHTML.createElement("span", { "class":`history-val`,});item.to ?? '(empty)'; }
                             }
                          }
                         }
                      }
                     }
                  }
                 }
              }

              
              if(issueFull.attachments && issueFull.attachments.length > 0){
                 { svelteHTML.createElement("div", { "class":`jira-card collapsible-card`,});
                   { svelteHTML.createElement("button", {       "class":`jira-coll-trigger`,"onclick":() => toggleSection('attachments'),"aria-expanded":!collapsed.attachments,});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":collapsed.attachments ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                     { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
                     { svelteHTML.createElement("span", { "class":`section-count`,}); issueFull.attachments.length;  }
                   }
                  if(!collapsed.attachments){
                     { svelteHTML.createElement("div", { "class":`attachments-grid`,});
                         for(let att of __sveltets_2_ensureArray(issueFull.attachments)){att.id;
                         { svelteHTML.createElement("div", { "class":`attachment-card`,});
                           { svelteHTML.createElement("div", { "class":`att-header`,});
                             { svelteHTML.createElement("span", {   "class":`att-filename`,"title":att.filename,});att.filename; }
                             { svelteHTML.createElement("span", { "class":`att-meta`,});formatBytes(att.size);  att.mime; }
                           }

                          if(att.mime.startsWith('image/')){
                             { svelteHTML.createElement("div", { "class":`att-preview`,});
                              if(attachmentUrls[att.id]){
                                 { svelteHTML.createElement("img", {       "class":`att-img`,"src":attachmentUrls[att.id],"alt":att.filename,});}
                              }else{
                                 { svelteHTML.createElement("button", {       "class":`att-load-btn`,"onclick":() => loadAttachmentUrl(att.id),"disabled":attachmentLoading[att.id],});
                                  attachmentLoading[att.id] ? 'Loading preview…' : 'Load preview';
                                 }
                              }
                             }
                          } else if (att.mime === 'application/pdf'){
                             { svelteHTML.createElement("div", { "class":`att-preview`,});
                              if(attachmentUrls[att.id]){
                                 { svelteHTML.createElement("iframe", {       "class":`att-pdf`,"src":attachmentUrls[att.id],"title":att.filename,}); }
                              }else{
                                 { svelteHTML.createElement("button", {       "class":`att-load-btn`,"onclick":() => loadAttachmentUrl(att.id),"disabled":attachmentLoading[att.id],});
                                  attachmentLoading[att.id] ? 'Loading preview…' : 'Preview PDF';
                                 }
                              }
                             }
                          }else{
                             { svelteHTML.createElement("div", { "class":`att-download`,});
                              if(attachmentUrls[att.id]){
                                 { svelteHTML.createElement("a", {       "class":`att-dl-link`,"href":attachmentUrls[att.id],"download":att.filename,});
                                  
                                 }
                              }else{
                                 { svelteHTML.createElement("button", {       "class":`att-load-btn`,"onclick":() => loadAttachmentUrl(att.id),"disabled":attachmentLoading[att.id],});
                                  attachmentLoading[att.id] ? 'Preparing…' : 'Download';
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
             }
          }

          
           { const $$_lenaPstnemhcattA3C = __sveltets_2_ensureComponent(AttachmentsPanel); const $$_lenaPstnemhcattA3 = new $$_lenaPstnemhcattA3C({ target: __sveltets_2_any(), props: {  }});panelRef = $$_lenaPstnemhcattA3;}
         }

        
         { svelteHTML.createElement("div", { "class":`col-right`,});
           { const $$_noitceSeussIariJ3C = __sveltets_2_ensureComponent(JiraIssueSection); new $$_noitceSeussIariJ3C({ target: __sveltets_2_any(), props: {      "issue":ji,story,collapsed,"ontoggle":toggleSection,}});}
         }
       }

    }else{
      
       { svelteHTML.createElement("div", { "class":`body-wrap`,});
        if(viewingVersion){
           { svelteHTML.createElement("div", { "class":`version-banner`,});
             viewingVersion.version_no; kindLabel(viewingVersion.kind);
             new Date(viewingVersion.created_at).toLocaleString();
            if(viewingVersion.change_notes){
               { svelteHTML.createElement("span", { "class":`change-notes`,}); viewingVersion.change_notes; }
            }
           }
        }

        if(renderedBody){
          
           { svelteHTML.createElement("div", { "class":`md-body`,}); resolvedBody || renderedBody; }
        }else{
           { svelteHTML.createElement("div", { "class":`muted`,});         }
        }
       }

      if(isConfluence){
         { svelteHTML.createElement("div", { "class":`publish-bar`,});
           { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => (publishDialogMode = 'story'),});
               
           }
         }
      }

      
       { const $$_lenaPstnemhcattA1C = __sveltets_2_ensureComponent(AttachmentsPanel); const $$_lenaPstnemhcattA1 = new $$_lenaPstnemhcattA1C({ target: __sveltets_2_any(), props: {  }});panelRef = $$_lenaPstnemhcattA1;}
    }

    
    if(story.tree_kind === 'epic' || product.childrenOf(story.id).length > 0){
       { const $$_draoBnerdlihC1C = __sveltets_2_ensureComponent(ChildrenBoard); new $$_draoBnerdlihC1C({ target: __sveltets_2_any(), props: {  "epicId":story.id,}});}
    }

    
     { const $$_draCkniLmrawS1C = __sveltets_2_ensureComponent(SwarmLinkCard); new $$_draCkniLmrawS1C({ target: __sveltets_2_any(), props: {  "storyId":story.id,}});}

    
     { const $$_sesavnaCdekniL1C = __sveltets_2_ensureComponent(LinkedCanvases); new $$_sesavnaCdekniL1C({ target: __sveltets_2_any(), props: {  "storyId":story.id,}});}

    
    if(relatedStories.length > 0){
       { svelteHTML.createElement("div", { "class":`related-section`,});
         { svelteHTML.createElement("span", { "class":`section-label-sm`,});  }
         { svelteHTML.createElement("div", { "class":`related-list`,});
             for(let rel of __sveltets_2_ensureArray(relatedStories)){rel.id;
             { svelteHTML.createElement("button", {       "class":`related-item`,"onclick":() => void product.select(rel.id),"title":rel.source_key,});
               { svelteHTML.createElement("span", { "class":`related-title`,});rel.title; }
               { svelteHTML.createElement("span", { "class":`related-tags`,});
                   for(let sharedTag of __sveltets_2_ensureArray(parseTags(rel.tags).filter((t) => currentTags.includes(t)))){sharedTag;
                   { svelteHTML.createElement("span", { "class":`tag-chip-sm`,});sharedTag; }
                }
               }
             }
          }
         }
       }
    }

   }
}

if(publishDialogMode){
   { const $$_golaiDhsilbuP0C = __sveltets_2_ensureComponent(PublishDialog); new $$_golaiDhsilbuP0C({ target: __sveltets_2_any(), props: {     "mode":publishDialogMode,"onclose":() => (publishDialogMode = null),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OverviewTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OverviewTab__SvelteComponent_ = ReturnType<typeof OverviewTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default OverviewTab__SvelteComponent_;