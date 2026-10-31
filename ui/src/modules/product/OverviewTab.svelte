<script lang="ts">
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
  import { renderMarkdown } from '../../lib/md';
  import { toasts } from '../../lib/toast.svelte';
  import { AttachmentUrls } from './attachmentUrls.svelte';
  import * as issues from '../../lib/api/issues';
  import type { ProductStoryVersion } from './types';
  import type { ProductAttachment } from './types';
  import { confirmOutward } from '../../lib/confirmOutward';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import PublishDialog from './PublishDialog.svelte';
  import AttachmentsPanel from './AttachmentsPanel.svelte';
  import DraftEditor from './DraftEditor.svelte';
  import LinkedCanvases from './LinkedCanvases.svelte';
  import ChildrenBoard from './ChildrenBoard.svelte';
  import { escapeHtml, unescapeHtml } from '../../lib/html';
  import JiraIssueSection from './JiraIssueSection.svelte';
  import { JiraIssue } from './jiraIssue.svelte';

  // Version picker state — null means "show current (source) version".
  let viewingVersion = $state<ProductStoryVersion | null>(null);
  let versionsLoaded = $state(false);
  let versionLoading = $state(false);
  let refreshing = $state(false);
  let watchWorking = $state(false);

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

</script>

{#if product.loadingDetail && (!detail || detail.story.id !== product.selectedId)}
  <!-- First open (or a switch to another story): a skeleton. A refresh of the
       story already on screen keeps it visible instead of blanking the tab. -->
  <div class="loading"><Skeleton rows={5} height={36} label="story" /></div>
{:else if !detail || !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="overview">
    <!-- ── Story header (full width) ────────────────────────────── -->
    <div class="story-header">
      <div class="story-meta-row">
        <button
          class="ov-stage-btn"
          onclick={openStageMenu}
          aria-haspopup="menu"
          aria-label="Lifecycle stage: {storyStage(story.stage).label}. Change stage"
          title="Lifecycle stage"
        >
          <StatusBadge status={storyStage(story.stage)} title="" />
          <Icon name="chevronDown" size={12} />
        </button>
        {#if story.issue_type}
          <Badge label={story.issue_type} />
        {/if}
        {#if story.url}
          <a class="source-link mono" href={story.url} target="_blank" rel="noopener noreferrer" title="Open in source">
            {story.source_key}
            <Icon name="external" size={12} />
          </a>
        {:else}
          <span class="source-key mono">{story.source_key}</span>
        {/if}
      </div>
      {#if editingTitle}
        <div class="title-edit">
          <input dir="auto"
            class="title-input"
            use:focusOnMount
            bind:value={titleDraft}
            spellcheck="false"
            aria-label="Story title"
            aria-invalid={titleEmpty}
            aria-describedby={titleEmpty ? 'ov-title-err' : undefined}
            onkeydown={(e) => {
              if (e.key === 'Enter') { e.preventDefault(); void saveTitle(); }
              else if (e.key === 'Escape') { e.preventDefault(); cancelEditTitle(); }
            }}
          />
          <button class="btn small primary" onclick={saveTitle} disabled={titleSaving || titleEmpty} title={titleEmpty ? 'Give the story a title first' : 'Writes to the live Jira issue — everyone with access sees it'}>
            {titleSaving ? 'Saving…' : `Save to ${story.source_key}`}
          </button>
          <button class="btn small" onclick={cancelEditTitle} disabled={titleSaving}>Cancel</button>
          {#if titleEmpty}<p class="title-err" id="ov-title-err">The title can’t be empty.</p>{/if}
        </div>
      {:else if !isDraft}
        <!-- The page header already names the story; this row is the Jira
             title's edit affordance (a draft edits its title in the form). -->
        <div class="title-row">
          <h2 class="story-title">{story.title}</h2>
          {#if isJira}
            <button
              class="title-edit-btn"
              data-edit-for="title"
              title="Edit title"
              aria-label="Edit title"
              onclick={beginEditTitle}
            ><Icon name="edit" size={13} /></button>
          {/if}
        </div>
      {/if}

      <!-- counts row -->
      <div class="counts-row">
        <span class="count-chip" title="Versions"><Icon name="archive" size={12} />{plural(detail.counts.versions, 'version')}</span>
        <span class="count-chip" title="Analyses"><Icon name="gauge" size={12} />{plural(detail.counts.analyses, 'analysis', 'analyses')}</span>
        <span class="count-chip" title="Open questions"><Icon name="comment" size={12} />{plural(detail.counts.open_questions, 'open question')}</span>
        <span class="count-chip" title="Notes"><Icon name="note" size={12} />{plural(detail.counts.notes, 'note')}</span>
        <span class="count-chip" title="Test cases"><Icon name="check" size={12} />{plural(detail.counts.testcases, 'test')}</span>
      </div>

      <!-- tags row -->
      <div class="tags-row">
        {#each currentTags as tag (tag)}
          <span class="tag-chip">
            {tag}
            <button
              class="tag-remove"
              onclick={() => removeTag(tag)}
              aria-label="Remove tag {tag}"
              title="Remove tag"
            ><Icon name="x" size={12} /></button>
          </span>
        {/each}
        <form
          class="tag-add-form"
          onsubmit={(e) => { e.preventDefault(); void addTag(); }}
        >
          <input dir="auto"
            class="tag-input"
            bind:value={tagInput}
            placeholder="Add tag…"
            disabled={tagSaving}
            aria-label="Add tag"
            spellcheck="false"
          />
        </form>
      </div>
    </div>

    <!-- ── Toolbar (full width) ───────────────────────────────────── -->
    <div class="toolbar">
      <!-- Version picker -->
      <div class="version-sel">
        <label class="ver-label" for="ov-version-select">Version</label>
        <select
          id="ov-version-select"
          class="ver-select"
          onchange={onVersionChange}
          onfocus={loadVersions}
          disabled={versionLoading}
        >
          <option value="">Current ({source ? `v${source.version_no}` : 'none'})</option>
          {#each product.versions as v (v.id)}
            {#if v.id !== source?.id}
              <option value={v.id}>v{v.version_no} — {kindLabel(v.kind)} ({new Date(v.created_at).toLocaleDateString()})</option>
            {/if}
          {/each}
        </select>
        {#if versionLoading}
          <span class="spinner" role="img" aria-label="Loading the version" title="Loading the version"></span>
        {/if}
      </div>

      <span class="grow"></span>

      {#if !isDraft}
      <!-- Watch / Refresh follow the SOURCE issue/page — a draft has none. -->
      <!-- Watch toggle -->
      <button
        class="btn small"
        aria-pressed={story.watch_enabled}
        onclick={toggleWatch}
        disabled={watchWorking}
        title={story.watch_enabled ? 'Watching the source for changes — click to stop' : 'Watch the source for changes'}
      >
        <Icon name="eye" size={13} />
        {story.watch_enabled ? 'Watching' : 'Watch'}
      </button>

      <!-- Refresh -->
      <button
        class="btn small"
        onclick={refresh}
        disabled={refreshing}
        title="Pull latest content from source"
        aria-label="Refresh story"
      >
        <Icon name="refresh" size={13} />
        {refreshing ? 'Refreshing…' : 'Refresh'}
      </button>
      {/if}
    </div>

    <!-- ── Body area ──────────────────────────────────────────────── -->
    {#if isDraft}
      <!-- ── DRAFT: two-column layout (editor left, transcripts right) ── -->
      <DraftEditor {story} {source} onpublish={(mode) => (publishDialogMode = mode)} />

    {:else if isJira}
      <!-- ── JIRA: two-column layout (body left, metadata right) ──── -->
      <div class="two-col jira-layout">
        <!-- Left: story body -->
        <div class="col-left">
          <div class="body-wrap">
            {#if viewingVersion}
              <div class="version-banner">
                Viewing v{viewingVersion.version_no} ({kindLabel(viewingVersion.kind)})
                — {new Date(viewingVersion.created_at).toLocaleString()}
                {#if viewingVersion.change_notes}
                  <span class="change-notes">· {viewingVersion.change_notes}</span>
                {/if}
              </div>
            {/if}

            <!-- Description header + inline edit affordance (Jira-style). The Edit
                 button is hidden while viewing a historical version — edits always
                 apply to the live issue's current description. -->
            <div class="desc-header">
              <span class="desc-label">Description</span>
              {#if !editingDesc && !viewingVersion}
                <button class="desc-edit-btn" data-edit-for="description" aria-label="Edit description" onclick={beginEditDesc}>
                  <Icon name="edit" size={12} /> Edit
                </button>
              {/if}
            </div>

            {#if editingDesc}
              <div class="desc-editor">
                <textarea dir="auto"
                  class="desc-textarea"
                  use:focusOnMount
                  bind:value={descDraft}
                  rows={16}
                  placeholder="e.g. ## Goal — let players cap their weekly deposits"
                  spellcheck="false"
                  aria-label="Description (Markdown)"
                  onkeydown={(e) => { if (e.key === 'Escape') { e.preventDefault(); cancelEditDesc(); } }}
                ></textarea>
                <div class="desc-editor-actions">
                  <button class="btn small primary" onclick={saveDesc} disabled={descSaving} title="Overwrites the live Jira description — everyone with access sees it">
                    {descSaving ? 'Saving…' : `Save to ${story.source_key}`}
                  </button>
                  <button class="btn small" onclick={cancelEditDesc} disabled={descSaving}>Cancel</button>
                </div>
              </div>
            {:else if renderedBody}
              <!-- resolvedBody has attachment:<id> tokens rewritten to authed blob URLs;
                   falls back to renderedBody while async resolution is in progress. -->
              <div class="md-body">{@html resolvedBody || renderedBody}</div>
            {:else}
              <div class="muted">No description yet. Use Edit to add one, or Refresh to pull from source.</div>
            {/if}
          </div>

          <!-- ── Activity: comments / history / attachments ──────────────
               These live below the description in the main column (like Jira);
               the right column keeps just Status / Assignee / Details / Links /
               Development. `{#if issueFull}` narrows the type for the cards. -->
          {#if issueFull}
            <div class="jira-section jira-activity">
              <!-- ── Comments ─────────────────────────────────────── -->
              <div class="jira-card collapsible-card">
                <button
                  class="jira-coll-trigger"
                  onclick={() => toggleSection('comments')}
                  aria-expanded={!collapsed.comments}
                >
                  <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed.comments ? 'chevronRight' : 'chevronDown'} size={12} /></span>
                  <span class="jira-section-label">Comments</span>
                  <span class="section-count">({issueFull.comments.length})</span>
                </button>
                {#if !collapsed.comments}
                  <div class="comments-list">
                    {#if issueFull.comments.length === 0}
                      <div class="comments-empty">No comments yet.</div>
                    {:else}
                      {#each issueFull.comments as comment (comment.id)}
                        <div class="comment-item">
                          <div class="comment-meta">
                            <span class="comment-author">{comment.author}</span>
                            <span class="comment-date" title={new Date(comment.created).toLocaleString()}>{rel(comment.created)}</span>
                          </div>
                          <div class="comment-body md-body">{@html renderMarkdown(comment.body_md)}</div>
                        </div>
                      {/each}
                    {/if}
                  </div>
                  <!-- Add comment form -->
                  <div class="add-comment-form">
                    <textarea dir="auto"
                      class="textarea comment-textarea"
                      bind:value={newCommentBody}
                      rows={3}
                      aria-label="Comment"
                      placeholder="e.g. Should the limit also cover bonus funds?"
                      spellcheck="true"
                      disabled={postingComment}
                    ></textarea>
                    <div class="add-comment-row">
                      <button
                        class="btn small primary"
                        onclick={addComment}
                        disabled={postingComment || !newCommentBody.trim()}
                      >
                        {postingComment ? 'Posting…' : `Post comment to ${story.source_key}…`}
                      </button>
                    </div>
                  </div>
                {/if}
              </div>

              <!-- ── History ─────────────────────────────────────── -->
              {#if issueFull.history && issueFull.history.length > 0}
                <div class="jira-card collapsible-card">
                  <button
                    class="jira-coll-trigger"
                    onclick={() => toggleSection('history')}
                    aria-expanded={!collapsed.history}
                  >
                    <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed.history ? 'chevronRight' : 'chevronDown'} size={12} /></span>
                    <span class="jira-section-label">History</span>
                    <span class="section-count">({issueFull.history.length} entries)</span>
                  </button>
                  {#if !collapsed.history}
                    <div class="history-list">
                      {#each issueFull.history as entry, i (i)}
                        <div class="history-entry">
                          <div class="history-meta">
                            <span class="history-author">{entry.author}</span>
                            <span class="history-date" title={new Date(entry.created).toLocaleString()}>{rel(entry.created)}</span>
                          </div>
                          {#each entry.items as item, j (j)}
                            <div class="history-change">
                              changed <span class="history-field">{item.field}</span>
                              {#if item.from}
                                from <span class="history-val">{item.from}</span>
                              {/if}
                              to <span class="history-val">{item.to ?? '(empty)'}</span>
                            </div>
                          {/each}
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/if}

              <!-- ── Attachments (from Jira) ─────────────────────── -->
              {#if issueFull.attachments && issueFull.attachments.length > 0}
                <div class="jira-card collapsible-card">
                  <button
                    class="jira-coll-trigger"
                    onclick={() => toggleSection('attachments')}
                    aria-expanded={!collapsed.attachments}
                  >
                    <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed.attachments ? 'chevronRight' : 'chevronDown'} size={12} /></span>
                    <span class="jira-section-label">Jira attachments</span>
                    <span class="section-count">({issueFull.attachments.length})</span>
                  </button>
                  {#if !collapsed.attachments}
                    <div class="attachments-grid">
                      {#each issueFull.attachments as att (att.id)}
                        <div class="attachment-card">
                          <div class="att-header">
                            <span class="att-filename" title={att.filename}>{att.filename}</span>
                            <span class="att-meta">{formatBytes(att.size)} · {att.mime}</span>
                          </div>

                          {#if att.mime.startsWith('image/')}
                            <div class="att-preview">
                              {#if attachmentUrls[att.id]}
                                <img
                                  class="att-img"
                                  src={attachmentUrls[att.id]}
                                  alt={att.filename}
                                />
                              {:else}
                                <button
                                  class="att-load-btn"
                                  onclick={() => loadAttachmentUrl(att.id)}
                                  disabled={attachmentLoading[att.id]}
                                >
                                  {attachmentLoading[att.id] ? 'Loading preview…' : 'Load preview'}
                                </button>
                              {/if}
                            </div>
                          {:else if att.mime === 'application/pdf'}
                            <div class="att-preview">
                              {#if attachmentUrls[att.id]}
                                <iframe
                                  class="att-pdf"
                                  src={attachmentUrls[att.id]}
                                  title={att.filename}
                                ></iframe>
                              {:else}
                                <button
                                  class="att-load-btn"
                                  onclick={() => loadAttachmentUrl(att.id)}
                                  disabled={attachmentLoading[att.id]}
                                >
                                  {attachmentLoading[att.id] ? 'Loading preview…' : 'Preview PDF'}
                                </button>
                              {/if}
                            </div>
                          {:else}
                            <div class="att-download">
                              {#if attachmentUrls[att.id]}
                                <a
                                  class="att-dl-link"
                                  href={attachmentUrls[att.id]}
                                  download={att.filename}
                                >
                                  Download
                                </a>
                              {:else}
                                <button
                                  class="att-load-btn"
                                  onclick={() => loadAttachmentUrl(att.id)}
                                  disabled={attachmentLoading[att.id]}
                                >
                                  {attachmentLoading[att.id] ? 'Preparing…' : 'Download'}
                                </button>
                              {/if}
                            </div>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}

          <!-- Otto-side attachments (drop / paste / file-picker). Shown for
               Jira stories below the activity, mirroring Jira's attachment area. -->
          <AttachmentsPanel bind:this={panelRef} />
        </div>

        <!-- Right: Jira metadata panel (Details sidebar) -->
        <div class="col-right">
          <JiraIssueSection issue={ji} {story} {collapsed} ontoggle={toggleSection} />
        </div>
      </div>

    {:else}
      <!-- ── CONFLUENCE / other: single-column read-only body ──────── -->
      <div class="body-wrap">
        {#if viewingVersion}
          <div class="version-banner">
            Viewing v{viewingVersion.version_no} ({kindLabel(viewingVersion.kind)})
            — {new Date(viewingVersion.created_at).toLocaleString()}
            {#if viewingVersion.change_notes}
              <span class="change-notes">· {viewingVersion.change_notes}</span>
            {/if}
          </div>
        {/if}

        {#if renderedBody}
          <!-- resolvedBody has attachment:<id> tokens rewritten to authed blob URLs. -->
          <div class="md-body">{@html resolvedBody || renderedBody}</div>
        {:else}
          <div class="muted">No content yet. Use Refresh to pull from source.</div>
        {/if}
      </div>

      {#if isConfluence}
        <div class="publish-bar">
          <button class="btn primary" onclick={() => (publishDialogMode = 'story')}>
            Convert to Jira Story…
          </button>
        </div>
      {/if}

      <!-- Attachments panel for Confluence / other layouts -->
      <AttachmentsPanel bind:this={panelRef} />
    {/if}

    <!-- ── Epic: the Children board (folder → cards) + roll-up (design §3.2) ── -->
    {#if story.tree_kind === 'epic' || product.childrenOf(story.id).length > 0}
      <ChildrenBoard epicId={story.id} />
    {/if}

    <!-- ── Canvases linked to this story ─────────────────────────── -->
    <LinkedCanvases storyId={story.id} />

    <!-- ── Related by tag ───────────────────────────────────────── -->
    {#if relatedStories.length > 0}
      <div class="related-section">
        <span class="section-label-sm">Related stories</span>
        <div class="related-list">
          {#each relatedStories as rel (rel.id)}
            <button
              class="related-item"
              onclick={() => void product.select(rel.id)}
              title={rel.source_key}
            >
              <span class="related-title">{rel.title}</span>
              <span class="related-tags">
                {#each parseTags(rel.tags).filter((t) => currentTags.includes(t)) as sharedTag (sharedTag)}
                  <span class="tag-chip-sm">{sharedTag}</span>
                {/each}
              </span>
            </button>
          {/each}
        </div>
      </div>
    {/if}

  </div>
{/if}

{#if publishDialogMode}
  <PublishDialog
    mode={publishDialogMode}
    onclose={() => (publishDialogMode = null)}
  />
{/if}

<style>
  .loading,
  .muted {
    padding: 24px 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
    font-style: italic;
  }
  .overview {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
    max-width: 100%;
  }

  /* ── Two-column grid ───────────────────────────────────────── */
  .two-col {
    display: grid;
    grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr);
    gap: 20px;
    align-items: start;
  }
  .col-left {
    min-width: 0;
  }
  .col-right {
    min-width: 0;
  }

  /* Responsive: collapse to a single column at tablet width and below */
  @media (max-width: 1024px) {
    .two-col {
      grid-template-columns: 1fr;
    }
  }

  /* ── Tags ──────────────────────────────────────────────────── */
  .tags-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 8px;
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-block: 2px; padding-inline: 8px 8px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 500;
    background: var(--accent-soft);
    color: var(--accent-text);
    border: 1px solid var(--accent-line);
  }
  .tag-remove {
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    padding: 0 1px;
    cursor: pointer;
    font-size: var(--fs-s);
    line-height: 1;
    color: var(--accent-text);
    opacity: 0.6;
    transition: opacity var(--dur-fast);
  }
  .tag-remove:hover {
    opacity: 1;
  }
  .tag-add-form {
    display: inline-flex;
  }
  .tag-input {
    border: 1px dashed var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 8px;
    width: 72px;
    outline: none;
    transition: border-color var(--dur-fast);
  }
  .tag-input:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
    color: var(--text);
    width: 100px;
  }
  .tag-input::placeholder {
    color: var(--text-dim);
    opacity: 0.7;
  }

  /* ── Related section ────────────────────────────────────────── */
  .related-section {
    margin-top: 16px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .section-label-sm {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    display: block;
    margin-bottom: 6px;
  }
  .related-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .related-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 4px 10px;
    cursor: pointer;
    text-align: start;
    font-size: var(--fs-s);
    color: var(--text);
    transition: background var(--dur-fast);
  }
  .related-item:hover {
    background: var(--hover);
  }
  .related-title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .related-tags {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }
  .tag-chip-sm {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
    border: 1px solid var(--accent-soft-strong);
  }

  /* Story header */
  .story-header {
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 12px;
  }
  /* Stage picker: the shared StatusBadge + a caret, opening a ctxMenu. */
  .ov-stage-btn {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-block: 0; padding-inline: 0 4px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .ov-stage-btn:hover :global(.sbadge) {
    border-color: var(--border-strong);
  }
  .story-meta-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .story-title {
    margin: 0 0 10px;
    font-size: var(--fs-l);
    font-weight: 600;
    line-height: 1.3;
    color: var(--text);
  }
  /* Title row: heading + an on-hover pencil (mirrors the field-edit pattern). */
  .title-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .title-row .story-title {
    margin-bottom: 0;
    flex: 1 1 auto;
    min-width: 0;
  }
  .title-edit-btn {
    flex-shrink: 0;
    display: inline-grid;
    place-items: center;
    margin-top: 2px;
    width: 24px;
    height: 24px;
    line-height: 1;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-m);
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--dur-fast), background var(--dur-fast), color var(--dur-fast);
  }
  .title-row:hover .title-edit-btn,
  .title-row:focus-within .title-edit-btn,
  .title-edit-btn:focus-visible {
    opacity: 1;
  }
  /* Touch: no hover to reveal it, so it stays visible. */
  @media (hover: none) {
    .title-edit-btn {
      opacity: 1;
    }
  }
  .title-edit-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .title-edit {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 10px;
    flex-wrap: wrap;
  }
  .title-input {
    flex: 1 1 260px;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  .title-err {
    flex-basis: 100%;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .title-input:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .source-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    text-decoration: none;
  }
  .source-link:hover {
    text-decoration: underline;
  }
  .source-key {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .counts-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .count-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
    padding: 2px 8px;
    border-radius: 999px;
  }

  /* Toolbar */
  .toolbar {
    display: flex;
    /* Wraps on a phone instead of scrolling the whole Overview sideways. */
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-bottom: 12px;
    margin-bottom: 12px;
    border-bottom: 1px solid var(--border);
  }
  .version-sel {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ver-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: .06em;
    white-space: nowrap;
  }
  .ver-select {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 4px 8px;
    max-width: 280px;
  }
  .grow {
    flex: 1;
  }

  /* Body */
  .body-wrap {
    flex: 1;
  }
  .version-banner {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--accent-faint);
    border: 1px solid var(--accent-soft-strong);
    border-radius: var(--radius-s);
    padding: 6px 12px;
    margin-bottom: 12px;
  }
  .change-notes {
    font-style: italic;
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── Jira section (right column) ──────────────────────────── */
  .jira-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  /* Activity (comments / history / attachments) sits below the description in the
     main column, separated from the body with a little breathing room. */
  .jira-activity {
    margin-top: 18px;
  }

  /* ── Jira card ─────────────────────────────────────────────── */
  .jira-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 14px;
    background: var(--surface);
  }
  .collapsible-card {
    padding: 0;
  }
  .jira-section-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }

  /* ── Collapsible trigger ───────────────────────────────────── */
  .jira-coll-trigger {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: none;
    border: none;
    padding: 10px 14px;
    cursor: pointer;
    text-align: start;
    color: var(--text-dim);
    font-size: var(--fs-s);
    border-radius: var(--radius-s);
    transition: background var(--dur-fast);
  }
  .jira-coll-trigger:hover {
    background: var(--hover);
  }
  .coll-arrow {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }
  .section-count {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--text-dim);
    margin-inline-start: 2px;
  }
  /* ── Description edit (Jira-style inline editor) ───────────────── */
  .desc-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 8px;
  }
  .desc-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .desc-edit-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast), border-color var(--dur-fast);
  }
  .desc-edit-btn:hover {
    background: var(--hover);
    color: var(--text);
    border-color: var(--text-dim);
  }
  .desc-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .desc-textarea {
    width: 100%;
    box-sizing: border-box;
    min-height: 220px;
    resize: vertical;
    padding: 10px 12px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.55;
  }
  .desc-textarea:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .desc-editor-actions {
    display: flex;
    gap: 6px;
  }

  /* ── Comments ──────────────────────────────────────────────── */
  .comments-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 14px 10px;
  }
  .comment-item {
    padding: 10px 0;
    border-top: 1px solid var(--border);
  }
  .comment-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .comment-author {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .comment-date {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .comment-body {
    font-size: var(--fs-s);
    line-height: 1.55;
  }
  .comments-empty {
    padding: 10px 14px 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-style: italic;
  }
  .add-comment-form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 14px 12px;
    border-top: 1px solid var(--border);
  }
  .comment-textarea {
    font-family: inherit;
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .add-comment-row {
    display: flex;
    justify-content: flex-end;
  }

  /* ── History ────────────────────────────────────────────────── */
  .history-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 14px 10px;
  }
  .history-entry {
    padding: 8px 0;
    border-top: 1px solid var(--border);
  }
  .history-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .history-author {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .history-date {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .history-change {
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .history-field {
    font-weight: 600;
    color: var(--text);
  }
  .history-val {
    font-weight: 500;
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    padding: 0 4px;
    border-radius: var(--radius-s);
  }

  /* ── Attachments ───────────────────────────────────────────── */
  .attachments-grid {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0 14px 12px;
  }
  .attachment-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    background: color-mix(in srgb, var(--text-dim) 4%, transparent);
  }
  .att-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 6px;
  }
  .att-filename {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .att-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .att-preview {
    margin-top: 4px;
  }
  .att-img {
    max-width: 100%;
    max-height: 320px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    display: block;
    object-fit: contain;
  }
  .att-pdf {
    width: 100%;
    height: 400px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .att-download {
    margin-top: 4px;
  }
  .att-load-btn {
    height: 24px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .att-load-btn:hover:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent);
  }
  .att-load-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }
  .att-dl-link {
    font-size: var(--fs-s);
    color: var(--accent-text);
    text-decoration: none;
  }
  .att-dl-link:hover {
    text-decoration: underline;
  }

  /* Shared textarea look (the comment box; the draft editor has its own). */
  .textarea {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    font-family: var(--font-mono);
    padding: 8px 10px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    line-height: 1.55;
  }
  .textarea:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }

  /* ── Publish bar ───────────────────────────────────────────── */
  .publish-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }
</style>
