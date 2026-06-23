<script lang="ts">
  // Overview tab — shows the selected story's detail: title, source link, stage
  // badge, issue_type, a version dropdown (with body_md rendering), Refresh
  // button, watch toggle, and (for Jira stories) a rich section with status,
  // assignee, details, linked issues, comments, history, and attachments.
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { toasts } from '../../lib/toast.svelte';
  import { api, authedBlobUrl } from '../../lib/api/client';
  import type { ProductStoryVersion, IssueFull, JiraTransition, JiraUser, EditableField, FieldOption, DevStatus } from './types';
  import type { ProductAttachment } from './types';
  import { confirmer } from '../../lib/confirm.svelte';
  import PublishDialog from './PublishDialog.svelte';
  import SwarmLinkCard from './SwarmLinkCard.svelte';
  import type { ProductTranscript } from './types';
  import AttachmentsPanel from './AttachmentsPanel.svelte';

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
  const story = $derived(detail?.story ?? null);
  const source = $derived(detail?.source ?? null);

  // Rendered markdown: version override wins over the live source body.
  const bodyMd = $derived((viewingVersion ?? source)?.body_md ?? '');
  const renderedBody = $derived(bodyMd ? renderMarkdown(bodyMd) : '');

  // ── Jira section state ────────────────────────────────────────────────────
  let issueFull = $state<IssueFull | null>(null);
  let issueLoading = $state(false);
  let issueError = $state<string | null>(null);

  // Status transition
  let transitions = $state<JiraTransition[]>([]);
  let transitionsLoading = $state(false);
  let transitionsLoaded = $state(false);
  let transitionWorking = $state(false);
  let statusOpen = $state(false);

  // Assignee
  let assignables = $state<JiraUser[]>([]);
  let assignablesLoading = $state(false);
  let assignablesLoaded = $state(false);
  let assigneeWorking = $state(false);
  let assigneeOpen = $state(false);

  // ── Development info (linked branches / commits / PRs via Jira dev-status) ──
  // Lazily fetched once the issue opens or the section is expanded.
  let devStatus = $state<DevStatus | null>(null);
  let devLoading = $state(false);
  let devLoaded = $state(false); // guards the lazy-load (distinct from "has data")
  let devError = $state<string | null>(null);

  // ── Editable fields (generic editmeta-driven editor) ──────────────────────
  // editmeta is lazily fetched once per story; null = not loaded yet.
  let editmeta = $state<EditableField[] | null>(null);
  let editmetaLoading = $state(false);
  let editingField = $state<string | null>(null); // field key currently in edit mode
  let fieldDraft = $state<unknown>(null); // working value for the field being edited
  let fieldSaving = $state(false);

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

  // ── Draft edit state ─────────────────────────────────────────────────────
  let draftTitle = $state('');
  let draftBody = $state('');
  let draftSaving = $state(false);

  // ── AttachmentsPanel ref + screenshot paste counter ───────────────────────
  // panelRef exposes uploadBlob(blob, opts) for the textarea paste handler.
  let panelRef = $state<{ uploadBlob: (blob: Blob, opts?: { filename?: string; kind?: string }) => Promise<ProductAttachment> } | null>(null);
  // Session-local counter for unique screenshot filenames.
  let bodyScreenshotIdx = $state(0);

  // ── Body preview: attachment token resolution ─────────────────────────────
  // Resolved authed blob URLs for `attachment:<id>` refs found in renderedBody.
  // These are separate from Jira attachmentUrls to avoid naming collisions.
  let bodyAttUrls = $state<Record<string, string>>({});
  // Object URLs created for bodyAttUrls — revoked alongside createdObjectUrls.
  let bodyAttObjectUrls: string[] = [];
  // IDs currently being fetched — prevents duplicate in-flight requests across
  // overlapping resolver runs (mirrors AttachmentsPanel.loadAttUrl guard).
  const bodyAttFetching = new Set<string>();

  // HTML of renderedBody with markdown image tokens rewritten to <img> tags with blob URLs.
  let resolvedBody = $state('');

  // Track the last rendered body to avoid redundant re-resolution.
  let lastResolvedInput = $state('');

  // ── Transcript state ──────────────────────────────────────────────────────
  let transcriptsLoaded = $state(false);
  let newTranscriptTitle = $state('');
  let newTranscriptBody = $state('');
  let addingTranscript = $state(false);
  let expandedTranscripts = $state<Record<string, boolean>>({});

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
      toasts.error('Could not save tag', product.errMsg(e));
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
      toasts.error('Could not remove tag', product.errMsg(e));
    }
  }

  // ── Load IssueFull on tab mount when Jira ────────────────────────────────

  $effect(() => {
    // Track selectedId so this re-runs on story change.
    product.selectedId;
    viewingVersion = null;
    versionsLoaded = false;
    // Reset Jira state.
    issueFull = null;
    issueError = null;
    transitions = [];
    transitionsLoaded = false;
    assignables = [];
    assignablesLoaded = false;
    statusOpen = false;
    assigneeOpen = false;
    // Reset development info.
    devStatus = null;
    devLoaded = false;
    devLoading = false;
    devError = null;
    // Reset editable-field state.
    editmeta = null;
    editmetaLoading = false;
    editingField = null;
    fieldDraft = null;
    fieldSaving = false;
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
    for (const url of bodyAttObjectUrls) {
      URL.revokeObjectURL(url);
    }
    bodyAttObjectUrls = [];
    bodyAttUrls = {};
    resolvedBody = '';
    lastResolvedInput = '';
  });

  $effect(() => {
    if (isJira && story && !issueFull && !issueLoading) {
      void loadIssueFull();
    }
  });

  // Once the issue is loaded, lazily fetch editmeta so edit affordances (and the
  // always-on Story Points / Estimate add-row) can render. Reads issueFull only;
  // the mutation of `editmeta` happens inside ensureEditmeta(), never in a $derived.
  $effect(() => {
    if (isJira && issueFull && editmeta === null && !editmetaLoading) {
      void ensureEditmeta();
    }
  });

  // Cleanup object URLs on unmount.
  $effect(() => {
    return () => {
      for (const url of createdObjectUrls) {
        URL.revokeObjectURL(url);
      }
      for (const url of bodyAttObjectUrls) {
        URL.revokeObjectURL(url);
      }
    };
  });

  $effect(() => {
    // Seed draft edit fields from the live source version.
    if (isDraft && story && source) {
      draftTitle = story.title;
      draftBody = source.body_md ?? '';
    }
    // Reset transcript state.
    transcriptsLoaded = false;
    newTranscriptTitle = '';
    newTranscriptBody = '';
    expandedTranscripts = {};
  });

  $effect(() => {
    if (isDraft && story && !transcriptsLoaded) {
      transcriptsLoaded = true;
      void product.loadTranscripts();
    }
  });

  // Load swarms lazily so the discovery team picker is populated.
  $effect(() => {
    const wsId = ws.currentId;
    if (wsId && swarm.swarms.length === 0) {
      void swarm.loadSwarms(wsId);
    }
  });

  /** Launch a discovery swarm run and switch to the Discovery tab. */
  async function runDiscovery(): Promise<void> {
    if (runningDiscovery) return;
    const targetSwarm = swarm.swarms.find((s) => s.id === discoverySwarmId);
    const teamName = targetSwarm ? `"${targetSwarm.name}"` : 'a swarm';
    const attCount = 0; // attachments listed in the panel; count not tracked here
    const ok = await confirmer.ask(
      `Run Discovery in ${teamName}? This will START the swarm and send the story info${attCount > 0 ? ` + ${attCount} attachments` : ''} as discovery context.`,
      { title: 'Run Discovery', confirmLabel: 'Run Discovery' },
    );
    if (!ok) return;
    runningDiscovery = true;
    try {
      await product.discover(discoverySwarmId ? { swarm_id: discoverySwarmId } : {});
      toasts.success('Discovery started', 'The swarm is now analysing the story.');
      product.tab = 'discovery';
    } catch (e) {
      toasts.error('Discovery failed', e instanceof Error ? e.message : String(e));
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
    void resolveAttachmentTokens(html);
  });

  function escapeHtmlAttr(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  async function resolveAttachmentTokens(html: string): Promise<void> {
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
    // Fetch any uncached URLs, guarding against concurrent duplicate fetches.
    // Collect all results locally so one merged assignment updates the cache
    // (avoids stale-snapshot overwrites from concurrent reactive spreads).
    const uncached = matches.map(({ id }) => id).filter(
      (id) => !bodyAttUrls[id] && !bodyAttFetching.has(id),
    );
    for (const id of uncached) bodyAttFetching.add(id);
    const collected: Record<string, string> = {};
    await Promise.all(
      uncached.map(async (id) => {
        try {
          const url = await authedBlobUrl(`/product/attachments/${id}`);
          bodyAttObjectUrls.push(url);
          collected[id] = url;
        } catch (e) {
          console.warn('[OverviewTab] attachment token resolution failed', id, e);
        } finally {
          bodyAttFetching.delete(id);
        }
      }),
    );
    if (Object.keys(collected).length > 0) {
      bodyAttUrls = { ...bodyAttUrls, ...collected };
    }
    // Replace each markdown image token with an <img> tag using the blob URL.
    let out = html;
    for (const { full, alt, id } of matches) {
      const blobUrl = bodyAttUrls[id];
      if (blobUrl) {
        out = out.replace(full, `<img class="body-attachment" alt="${escapeHtmlAttr(alt)}" src="${blobUrl}">`);
      }
    }
    resolvedBody = out;
  }

  async function loadIssueFull(): Promise<void> {
    if (!story) return;
    issueLoading = true;
    issueError = null;
    try {
      issueFull = await api.get<IssueFull>(
        `/issue/${story.account_id}/${story.source_key}/full`,
      );
    } catch (e) {
      issueError = e instanceof Error ? e.message : String(e);
    } finally {
      issueLoading = false;
    }
  }

  // Best-effort dev-status fetch. Early-returns once loaded so the expand-trigger
  // and the on-open effect can both call it safely.
  async function loadDevStatus(): Promise<void> {
    if (devLoaded || !story) return;
    devLoading = true;
    devError = null;
    try {
      const idParam = issueFull?.id ? `?issueId=${encodeURIComponent(issueFull.id)}` : '';
      devStatus = await api.get<DevStatus>(
        `/issue/${story.account_id}/${story.source_key}/devstatus${idParam}`,
      );
    } catch (e) {
      devError = e instanceof Error ? e.message : String(e);
    } finally {
      // Mark loaded even on error: the on-open $effect gates on `devLoaded`, so
      // leaving it false after a failed fetch would re-trigger this every time
      // the request settles → an infinite retry loop. (Same guard rationale as
      // ensureEditmeta's `editmeta = []` on error.) The explicit refresh path
      // resets `devLoaded = false` to force a fresh fetch.
      devLoaded = true;
      devLoading = false;
    }
  }

  async function loadTransitions(): Promise<void> {
    if (transitionsLoaded || !story) return;
    transitionsLoading = true;
    try {
      transitions = await api.get<JiraTransition[]>(
        `/issue/${story.account_id}/${story.source_key}/transitions`,
      );
      transitionsLoaded = true;
    } catch (e) {
      toasts.error('Could not load transitions', e instanceof Error ? e.message : String(e));
    } finally {
      transitionsLoading = false;
    }
  }

  async function applyTransition(tid: string): Promise<void> {
    if (!story) return;
    transitionWorking = true;
    statusOpen = false;
    try {
      await api.post(`/issue/${story.account_id}/${story.source_key}/transitions`, {
        transition_id: tid,
      });
      toasts.info('Status updated');
      await loadIssueFull();
      await product.refresh();
    } catch (e) {
      toasts.error('Transition failed', e instanceof Error ? e.message : String(e));
    } finally {
      transitionWorking = false;
    }
  }

  async function loadAssignables(): Promise<void> {
    if (assignablesLoaded || !story) return;
    assignablesLoading = true;
    try {
      assignables = await api.get<JiraUser[]>(
        `/issue/${story.account_id}/${story.source_key}/assignable`,
      );
      assignablesLoaded = true;
    } catch (e) {
      toasts.error('Could not load assignable users', e instanceof Error ? e.message : String(e));
    } finally {
      assignablesLoading = false;
    }
  }

  async function assignUser(accountId: string): Promise<void> {
    if (!story) return;
    assigneeWorking = true;
    assigneeOpen = false;
    try {
      await api.put(`/issue/${story.account_id}/${story.source_key}/assignee`, {
        account_id: accountId,
      });
      toasts.info('Assignee updated');
      await loadIssueFull();
    } catch (e) {
      toasts.error('Assign failed', e instanceof Error ? e.message : String(e));
    } finally {
      assigneeWorking = false;
    }
  }

  // ── Editable-field helpers ──────────────────────────────────────────────

  /** Lazily fetch editmeta once and cache it. On error mark it loaded-but-empty
   *  so everything stays read-only and we don't retry-loop. */
  async function ensureEditmeta(): Promise<void> {
    if (editmeta !== null || editmetaLoading || !story) return;
    editmetaLoading = true;
    try {
      editmeta = await api.get<EditableField[]>(
        `/issue/${story.account_id}/${story.source_key}/editmeta`,
      );
    } catch (e) {
      toasts.error('Could not load editable fields', e instanceof Error ? e.message : String(e));
      editmeta = []; // loaded-but-empty: every field stays read-only
    } finally {
      editmetaLoading = false;
    }
  }

  /** The EditableField metadata for a given key, or undefined if not editable. */
  function editableFor(key: string): EditableField | undefined {
    return editmeta?.find((f) => f.key === key);
  }

  /** The numeric estimate field (Story Points / Original Estimate), if editable.
   *  Pure read of editmeta — safe inside $derived. Returns null when none. */
  const estimateField = $derived(
    editmeta?.find(
      (f) => f.schema_type === 'number' && /story point|estimate/i.test(f.name),
    ) ?? null,
  );

  /** Raw current numeric value for the estimate field, sourced from issueFull.fields
   *  (parse_issue_full strips ".0", so a plain number string is fine to seed). */
  function rawFieldValue(key: string): string {
    const f = issueFull?.fields.find((ff) => ff.key === key);
    return f?.value ?? '';
  }

  /** Enter edit mode for a field, seeding fieldDraft from its current value. */
  async function beginEdit(ef: EditableField, currentRaw: string): Promise<void> {
    await ensureEditmeta();
    if (!editableFor(ef.key)) return; // not actually editable — stay read-only
    if (ef.schema_type === 'array') {
      if (ef.items === 'string') {
        // labels / free-text array: comma text
        fieldDraft = currentRaw;
      } else {
        // array of options/users: list of selected ids (best-effort from labels)
        const labels = currentRaw.split(',').map((s) => s.trim()).filter(Boolean);
        fieldDraft = ef.allowed_values
          .filter((o) => labels.includes(o.label))
          .map((o) => o.id);
      }
    } else if (
      ef.schema_type === 'option' ||
      ef.schema_type === 'priority' ||
      ef.schema_type === 'version' ||
      ef.schema_type === 'component'
    ) {
      // select: pre-select by matching label → id
      fieldDraft = ef.allowed_values.find((o) => o.label === currentRaw)?.id ?? '';
    } else if (ef.schema_type === 'user') {
      await loadAssignables();
      fieldDraft = '';
    } else if (ef.schema_type === 'datetime') {
      // Seed datetime-local input from an existing ISO value. Parse to local
      // "YYYY-MM-DDTHH:mm" which is what <input type="datetime-local"> expects.
      if (currentRaw) {
        try {
          const d = new Date(currentRaw);
          // Use local time by offsetting so the user sees their TZ.
          const offset = d.getTimezoneOffset() * 60000;
          const local = new Date(d.getTime() - offset);
          fieldDraft = local.toISOString().slice(0, 16); // "YYYY-MM-DDTHH:mm"
        } catch {
          fieldDraft = '';
        }
      } else {
        fieldDraft = '';
      }
    } else {
      // string / number / date / unknown → raw text
      fieldDraft = currentRaw;
    }
    editingField = ef.key;
  }

  /** Cancel the in-progress field edit. */
  function cancelEdit(): void {
    editingField = null;
    fieldDraft = null;
  }

  /** Toggle membership of an option id in the multi-select array draft. */
  function toggleArrayOption(id: string): void {
    const cur = Array.isArray(fieldDraft) ? (fieldDraft as string[]) : [];
    fieldDraft = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
  }

  /** Build the Jira-shaped value for a field from the working draft. */
  function buildFieldValue(ef: EditableField, draft: unknown): unknown {
    switch (ef.schema_type) {
      case 'number': {
        const s = String(draft ?? '').trim();
        return s === '' ? null : Number(s);
      }
      case 'option':
      case 'priority':
      case 'version':
      case 'component': {
        const id = String(draft ?? '').trim();
        return id === '' ? null : { id };
      }
      case 'user': {
        const id = String(draft ?? '').trim();
        return id === '' ? null : { accountId: id };
      }
      case 'array': {
        if (ef.items === 'string') {
          // labels / free-text: split csv
          return String(draft ?? '')
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean);
        }
        const ids = Array.isArray(draft) ? (draft as string[]) : [];
        if (ef.items === 'user') return ids.map((id) => ({ accountId: id }));
        // option / version / component arrays
        return ids.map((id) => ({ id }));
      }
      case 'date':
        // <input type="date"> yields YYYY-MM-DD; Jira date fields accept this.
        return String(draft ?? '').trim() || null;
      case 'datetime': {
        // <input type="datetime-local"> yields "YYYY-MM-DDTHH:mm"; Jira datetime
        // fields require a full ISO-8601 string with offset — emit UTC.
        const raw = String(draft ?? '').trim();
        if (!raw) return null;
        return new Date(raw).toISOString(); // e.g. "2024-01-15T10:00:00.000Z"
      }
      default:
        // unknown → raw string
        return String(draft ?? '');
    }
  }

  /** PUT a single field then swap in the refreshed issue returned by the server. */
  async function saveField(ef: EditableField): Promise<void> {
    if (!story) return;
    fieldSaving = true;
    try {
      const value = buildFieldValue(ef, fieldDraft);
      issueFull = await api.put<IssueFull>(
        `/issue/${story.account_id}/${story.source_key}/fields`,
        { fields: { [ef.key]: value } },
      );
      editingField = null;
      fieldDraft = null;
      toasts.info('Field updated');
    } catch (e) {
      toasts.error('Could not update field', e instanceof Error ? e.message : String(e));
    } finally {
      fieldSaving = false;
    }
  }

  async function loadAttachmentUrl(attId: string): Promise<void> {
    if (!story || attachmentUrls[attId] || attachmentLoading[attId]) return;
    attachmentLoading = { ...attachmentLoading, [attId]: true };
    try {
      const url = await authedBlobUrl(
        `/issue/${story.account_id}/${story.source_key}/attachment/${attId}`,
      );
      createdObjectUrls.push(url);
      attachmentUrls = { ...attachmentUrls, [attId]: url };
    } catch (e) {
      console.warn('[OverviewTab] attachment load failed', attId, e);
    } finally {
      attachmentLoading = { ...attachmentLoading, [attId]: false };
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
    // Lazy-load development info when the section opens (no-op if already loaded).
    if (key === 'development' && !collapsed[key]) {
      void loadDevStatus();
    }
  }

  async function loadVersions(): Promise<void> {
    if (versionsLoaded) return;
    try {
      await product.loadVersions();
      versionsLoaded = true;
    } catch (e) {
      toasts.error('Could not load versions', product.errMsg(e));
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
      toasts.error('Could not load version', product.errMsg(err));
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
        await loadIssueFull();
        // Force the dev-status section to repopulate with fresh data.
        devLoaded = false;
        devStatus = null;
        await loadDevStatus();
      }
    } catch (e) {
      toasts.error('Refresh failed', product.errMsg(e));
    } finally {
      refreshing = false;
    }
  }

  async function saveDraft(): Promise<void> {
    draftSaving = true;
    try {
      await product.updateDraft({ title: draftTitle, body_md: draftBody });
      toasts.success('Draft saved');
    } catch (e) {
      toasts.error('Save failed', product.errMsg(e));
    } finally {
      draftSaving = false;
    }
  }

  /**
   * Paste handler for the draft body `<textarea>`. When the clipboard contains
   * an image, upload it via the AttachmentsPanel's uploadBlob action, then
   * splice `![filename](attachment:<id>)` at the caret so the markdown body
   * references the attachment portably (not a blob URL).
   */
  async function handleBodyPaste(e: ClipboardEvent): Promise<void> {
    if (!e.clipboardData) return;
    for (const item of Array.from(e.clipboardData.items)) {
      if (item.kind === 'file' && item.type.startsWith('image/')) {
        e.preventDefault();
        const blob = item.getAsFile();
        if (!blob || !panelRef) return;
        const idx = ++bodyScreenshotIdx;
        const filename = `screenshot-${idx}.png`;
        try {
          const textarea = e.currentTarget as HTMLTextAreaElement;
          const caretPos = textarea.selectionStart ?? draftBody.length;
          const att = await panelRef.uploadBlob(blob, { filename, kind: 'image' });
          // Splice the markdown reference at the caret position.
          const token = `![${att.filename}](attachment:${att.id})`;
          draftBody =
            draftBody.slice(0, caretPos) + token + draftBody.slice(caretPos);
        } catch (ex) {
          toasts.error(
            'Screenshot upload failed',
            ex instanceof Error ? ex.message : String(ex),
          );
        }
        return; // only handle the first image item
      }
    }
  }

  async function doAddTranscript(): Promise<void> {
    if (!newTranscriptBody.trim()) return;
    addingTranscript = true;
    try {
      await product.addTranscript({
        title: newTranscriptTitle.trim() || null,
        body: newTranscriptBody.trim(),
      });
      newTranscriptTitle = '';
      newTranscriptBody = '';
      toasts.success('Transcript added');
    } catch (e) {
      toasts.error('Add transcript failed', product.errMsg(e));
    } finally {
      addingTranscript = false;
    }
  }

  async function doDeleteTranscript(t: ProductTranscript): Promise<void> {
    const ok = await confirmer.ask(
      `Remove transcript "${t.title || 'untitled'}"?`,
      { title: 'Remove transcript', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      await product.deleteTranscript(t.id);
      toasts.info('Transcript removed');
    } catch (e) {
      toasts.error('Remove failed', product.errMsg(e));
    }
  }

  function toggleTranscript(id: string): void {
    expandedTranscripts = { ...expandedTranscripts, [id]: !expandedTranscripts[id] };
  }

  async function toggleWatch(): Promise<void> {
    if (!story) return;
    watchWorking = true;
    try {
      await product.updateStory({ watch_enabled: !story.watch_enabled });
    } catch (e) {
      toasts.error('Could not update watch', product.errMsg(e));
    } finally {
      watchWorking = false;
    }
  }

  async function addComment(): Promise<void> {
    if (!story || !newCommentBody.trim()) return;
    postingComment = true;
    try {
      await api.post(`/issue/${story.account_id}/${story.source_key}/comment`, {
        body: newCommentBody.trim(),
      });
      newCommentBody = '';
      toasts.success('Comment posted');
      await loadIssueFull();
    } catch (e) {
      toasts.error('Could not post comment', e instanceof Error ? e.message : String(e));
    } finally {
      postingComment = false;
    }
  }

  function stageColor(stage: string): string {
    switch (stage) {
      case 'draft': return 'stage-draft';
      case 'review': return 'stage-review';
      case 'approved': return 'stage-approved';
      case 'done': return 'stage-done';
      default: return 'stage-other';
    }
  }

  // Lifecycle gate: the operator advances the story through the stages. Approval
  // is the gate before "Send to Swarm" (PlanTab warns when not approved).
  const STAGES = ['draft', 'review', 'approved', 'done'];
  async function setStage(stage: string): Promise<void> {
    if (!story || stage === story.stage) return;
    try {
      await product.updateStory({ stage });
    } catch (e) {
      toasts.error('Could not update stage', product.errMsg(e));
    }
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  function relDate(iso: string): string {
    try {
      const diff = Date.now() - new Date(iso).getTime();
      const s = Math.floor(diff / 1000);
      if (s < 60) return 'just now';
      const m = Math.floor(s / 60);
      if (m < 60) return `${m}m ago`;
      const h = Math.floor(m / 60);
      if (h < 24) return `${h}h ago`;
      const d = Math.floor(h / 24);
      if (d < 30) return `${d}d ago`;
      return new Date(iso).toLocaleDateString();
    } catch {
      return iso;
    }
  }
</script>

<!-- Inline "Edit" pencil shown next to an editable detail value. Renders only
     when editmeta marks the field editable. `current` seeds the draft. -->
{#snippet editBtn(key: string, current: string)}
  {#if editableFor(key)}
    <button
      class="field-edit-btn"
      title="Edit"
      aria-label="Edit field"
      onclick={() => beginEdit(editableFor(key)!, current)}
    >
      ✎
    </button>
  {/if}
{/snippet}

<!-- Inline editor for a single editable field, dispatched by schema_type. -->
{#snippet fieldEditor(ef: EditableField)}
  <div class="field-editor">
    {#if ef.schema_type === 'number'}
      <input class="field-input" type="number" step="any" bind:value={fieldDraft} />
    {:else if ef.schema_type === 'date'}
      <input class="field-input" type="date" bind:value={fieldDraft} />
    {:else if ef.schema_type === 'datetime'}
      <input class="field-input" type="datetime-local" bind:value={fieldDraft} />
    {:else if ef.schema_type === 'user'}
      <select class="field-input" bind:value={fieldDraft}>
        <option value="">Unassigned</option>
        {#if assignablesLoading}
          <option disabled>Loading…</option>
        {/if}
        {#each assignables as u (u.account_id)}
          <option value={u.account_id}>{u.display_name}</option>
        {/each}
      </select>
    {:else if (ef.schema_type === 'option' || ef.schema_type === 'priority' || ef.schema_type === 'version' || ef.schema_type === 'component') && ef.allowed_values.length > 0}
      <select class="field-input" bind:value={fieldDraft}>
        {#if !ef.required}
          <option value="">— None —</option>
        {/if}
        {#each ef.allowed_values as opt (opt.id)}
          <option value={opt.id}>{opt.label}</option>
        {/each}
      </select>
    {:else if ef.schema_type === 'array' && ef.items !== 'string' && ef.allowed_values.length > 0}
      <div class="field-multiselect">
        {#each ef.allowed_values as opt (opt.id)}
          <label class="field-check">
            <input
              type="checkbox"
              checked={Array.isArray(fieldDraft) && (fieldDraft as string[]).includes(opt.id)}
              onchange={() => toggleArrayOption(opt.id)}
            />
            {opt.label}
          </label>
        {/each}
      </div>
    {:else if ef.schema_type === 'array'}
      <!-- labels / free-text array (no allowed values) → comma-separated text -->
      <input class="field-input" type="text" placeholder="comma,separated" bind:value={fieldDraft} />
    {:else}
      <!-- string / unknown → raw text -->
      <input class="field-input" type="text" bind:value={fieldDraft} />
      {#if ef.schema_type !== 'string'}
        <span class="field-raw-note">raw ({ef.schema_type})</span>
      {/if}
    {/if}
    <div class="field-editor-actions">
      <button class="field-save-btn" onclick={() => saveField(ef)} disabled={fieldSaving}>
        {fieldSaving ? 'Saving…' : 'Save'}
      </button>
      <button class="field-cancel-btn" onclick={cancelEdit} disabled={fieldSaving}>Cancel</button>
    </div>
  </div>
{/snippet}

{#if product.loadingDetail}
  <div class="loading">Loading…</div>
{:else if !detail || !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="overview">
    <!-- ── Story header (full width) ────────────────────────────── -->
    <div class="story-header">
      <div class="story-meta-row">
        <select
          class="stage-badge stage-select {stageColor(story.stage)}"
          value={story.stage}
          onchange={(e) => setStage((e.currentTarget as HTMLSelectElement).value)}
          title="Lifecycle stage — advance to Approved before sending to a swarm"
        >
          {#each STAGES as st (st)}<option value={st}>{st}</option>{/each}
        </select>
        {#if story.issue_type}
          <span class="chip">{story.issue_type}</span>
        {/if}
        {#if story.url}
          <a class="source-link mono" href={story.url} target="_blank" rel="noopener noreferrer" title="Open in source">
            {story.source_key}
            <Icon name="external" size={11} />
          </a>
        {:else}
          <span class="source-key mono">{story.source_key}</span>
        {/if}
      </div>
      <h1 class="story-title">{story.title}</h1>

      <!-- counts row -->
      <div class="counts-row">
        <span class="count-chip" title="Versions"><Icon name="archive" size={11} />{detail.counts.versions} version{detail.counts.versions !== 1 ? 's' : ''}</span>
        <span class="count-chip" title="Analyses"><Icon name="gauge" size={11} />{detail.counts.analyses} anal.</span>
        <span class="count-chip" title="Open questions"><Icon name="comment" size={11} />{detail.counts.open_questions} Q</span>
        <span class="count-chip" title="Notes"><Icon name="note" size={11} />{detail.counts.notes} notes</span>
        <span class="count-chip" title="Test cases"><Icon name="check" size={11} />{detail.counts.testcases} tests</span>
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
            >×</button>
          </span>
        {/each}
        <form
          class="tag-add-form"
          onsubmit={(e) => { e.preventDefault(); void addTag(); }}
        >
          <input
            class="tag-input"
            bind:value={tagInput}
            placeholder="+ tag"
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
        <!-- svelte-ignore a11y_label_has_associated_control -->
        <label class="ver-label">Version</label>
        <select
          class="ver-select"
          onchange={onVersionChange}
          onfocus={loadVersions}
          disabled={versionLoading}
        >
          <option value="">Current ({source ? `v${source.version_no}` : 'none'})</option>
          {#each product.versions as v (v.id)}
            {#if v.id !== source?.id}
              <option value={v.id}>v{v.version_no} — {v.kind} ({new Date(v.created_at).toLocaleDateString()})</option>
            {/if}
          {/each}
        </select>
        {#if versionLoading}
          <span class="ver-loading">…</span>
        {/if}
      </div>

      <span class="grow"></span>

      <!-- Watch toggle -->
      <button
        class="toolbar-btn"
        class:active={story.watch_enabled}
        onclick={toggleWatch}
        disabled={watchWorking}
        title={story.watch_enabled ? 'Watching — click to disable' : 'Click to watch this story'}
        aria-label="Toggle watch"
      >
        <Icon name="eye" size={13} />
        {story.watch_enabled ? 'Watching' : 'Watch'}
      </button>

      <!-- Refresh -->
      <button
        class="toolbar-btn"
        onclick={refresh}
        disabled={refreshing}
        title="Pull latest content from source"
        aria-label="Refresh story"
      >
        <Icon name="refresh" size={13} />
        {refreshing ? 'Refreshing…' : 'Refresh'}
      </button>

      <!-- Discovery: team picker + launch button -->
      {#if swarm.swarms.length > 1}
        <select
          class="disc-swarm-pick"
          bind:value={discoverySwarmId}
          title="Which swarm runs the discovery"
        >
          <option value="">First swarm</option>
          {#each swarm.swarms as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
        </select>
      {/if}
      <button
        class="toolbar-btn"
        onclick={runDiscovery}
        disabled={runningDiscovery}
        title="Launch a discovery swarm run — agents analyse the story and report findings"
        aria-label="Run Discovery"
      >
        {runningDiscovery ? 'Starting…' : '⚡ Run Discovery'}
      </button>
    </div>

    <!-- ── Body area ──────────────────────────────────────────────── -->
    {#if isDraft}
      <!-- ── DRAFT: two-column layout (editor left, transcripts right) ── -->
      <div class="two-col draft-layout">
        <!-- Left: title + body editor + save + publish bar -->
        <div class="col-left">
          <div class="draft-section">
            <div class="draft-hint">
              Refine this draft with the Analysis / Questions / Rewrite tabs, then publish when ready.
            </div>

            <div class="field">
              <label class="label" for="draft-title">Title</label>
              <input
                id="draft-title"
                class="input"
                bind:value={draftTitle}
                placeholder="Story title…"
                spellcheck="false"
              />
            </div>

            <div class="field">
              <label class="label" for="draft-body">Body (Markdown)</label>
              <textarea
                id="draft-body"
                class="textarea"
                bind:value={draftBody}
                rows={14}
                placeholder="Write your story or paste notes here…"
                spellcheck="false"
                onpaste={handleBodyPaste}
              ></textarea>
            </div>

            <div class="draft-save-row">
              <button
                class="toolbar-btn save-btn"
                onclick={saveDraft}
                disabled={draftSaving}
              >
                {draftSaving ? 'Saving…' : 'Save draft'}
              </button>
            </div>

            <!-- ── Publish bar ─────────────────────────────────────── -->
            <div class="publish-bar">
              <button
                class="publish-btn"
                onclick={() => (publishDialogMode = 'story')}
              >
                Publish as Jira Story
              </button>
              <button
                class="publish-btn secondary"
                onclick={() => (publishDialogMode = 'rfc')}
              >
                Publish as Confluence RFC
              </button>
            </div>
          </div>
        </div>

        <!-- Right: Transcripts + Attachments -->
        <div class="col-right">
          <div class="transcripts-section">
            <div class="transcripts-header">
              <span class="section-title">Transcripts</span>
            </div>

            {#if product.loadingTranscripts}
              <div class="muted">Loading transcripts…</div>
            {:else if product.transcripts.length === 0}
              <div class="muted">No transcripts yet. Paste a conversation below.</div>
            {:else}
              <div class="transcript-list">
                {#each product.transcripts as t (t.id)}
                  <div class="transcript-item">
                    <div class="transcript-header">
                      <button
                        class="transcript-toggle"
                        onclick={() => toggleTranscript(t.id)}
                        aria-expanded={expandedTranscripts[t.id] ?? false}
                      >
                        <span class="coll-arrow">{expandedTranscripts[t.id] ? '▼' : '▶'}</span>
                        <span class="transcript-title">{t.title || 'Untitled transcript'}</span>
                        <span class="transcript-date">{relDate(t.created_at)}</span>
                      </button>
                      <button
                        class="del-transcript-btn"
                        onclick={() => doDeleteTranscript(t)}
                        title="Remove transcript"
                        aria-label="Remove transcript"
                      >✕</button>
                    </div>
                    {#if expandedTranscripts[t.id]}
                      <div class="transcript-body">{t.body}</div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}

            <!-- Add transcript form -->
            <div class="add-transcript-form">
              <input
                class="input"
                bind:value={newTranscriptTitle}
                placeholder="Title (optional)"
                spellcheck="false"
              />
              <textarea
                class="textarea"
                bind:value={newTranscriptBody}
                rows={5}
                placeholder="Paste conversation or notes here…"
                spellcheck="false"
              ></textarea>
              <button
                class="toolbar-btn"
                onclick={doAddTranscript}
                disabled={addingTranscript || !newTranscriptBody.trim()}
              >
                {addingTranscript ? 'Adding…' : 'Add transcript'}
              </button>
            </div>
          </div>

          <!-- Attachments panel: paste/drag/file-picker with previews. -->
          <!-- bind:this captures the exported uploadBlob action. -->
          <AttachmentsPanel bind:this={panelRef} />
        </div>
      </div>

    {:else if isJira}
      <!-- ── JIRA: two-column layout (body left, metadata right) ──── -->
      <div class="two-col jira-layout">
        <!-- Left: story body -->
        <div class="col-left">
          <div class="body-wrap">
            {#if viewingVersion}
              <div class="version-banner">
                Viewing v{viewingVersion.version_no} ({viewingVersion.kind})
                — {new Date(viewingVersion.created_at).toLocaleString()}
                {#if viewingVersion.change_notes}
                  <span class="change-notes">· {viewingVersion.change_notes}</span>
                {/if}
              </div>
            {/if}

            {#if renderedBody}
              <!-- resolvedBody has attachment:<id> tokens rewritten to authed blob URLs;
                   falls back to renderedBody while async resolution is in progress. -->
              <div class="md-body">{@html resolvedBody || renderedBody}</div>
            {:else}
              <div class="muted">No content yet. Use Refresh to pull from source.</div>
            {/if}
          </div>
        </div>

        <!-- Right: Jira metadata panel -->
        <div class="col-right">
          <div class="jira-section">
            {#if issueLoading}
              <Skeleton rows={6} height={36} />
              <div class="jira-loading">Loading Jira details…</div>
            {:else if issueError}
              <div class="jira-error">Could not load Jira details: {issueError}</div>
            {:else if issueFull}

              <!-- ── Status + Transition ──────────────────────────── -->
              <div class="jira-card">
                <div class="jira-card-header">
                  <span class="jira-section-label">Status</span>
                  <div class="status-control">
                    <span class="status-badge">{issueFull.status}</span>
                    <div class="transition-wrap">
                      <button
                        class="change-btn"
                        onclick={async () => {
                          if (!statusOpen) {
                            await loadTransitions();
                            statusOpen = true;
                          } else {
                            statusOpen = false;
                          }
                        }}
                        disabled={transitionWorking}
                        title="Change status"
                      >
                        {transitionWorking ? 'Working…' : 'Transition ▾'}
                      </button>
                      {#if statusOpen}
                        <div class="dropdown-menu">
                          {#if transitionsLoading}
                            <div class="dropdown-loading">Loading…</div>
                          {:else if transitions.length === 0}
                            <div class="dropdown-empty">No transitions available</div>
                          {:else}
                            {#each transitions as t (t.id)}
                              <button
                                class="dropdown-item"
                                onclick={() => applyTransition(t.id)}
                              >
                                {t.name}
                                <span class="dropdown-item-sub">→ {t.to_status}</span>
                              </button>
                            {/each}
                          {/if}
                        </div>
                      {/if}
                    </div>
                  </div>
                </div>
              </div>

              <!-- ── Assignee ─────────────────────────────────────── -->
              <div class="jira-card">
                <div class="jira-card-header">
                  <span class="jira-section-label">Assignee</span>
                  <div class="assignee-control">
                    {#if issueFull.assignee}
                      <div class="user-row">
                        {#if issueFull.assignee.avatar_url}
                          <img class="avatar" src={issueFull.assignee.avatar_url} alt={issueFull.assignee.display_name} />
                        {:else}
                          <span class="avatar-placeholder">{issueFull.assignee.display_name.slice(0, 1).toUpperCase()}</span>
                        {/if}
                        <span class="user-name">{issueFull.assignee.display_name}</span>
                      </div>
                    {:else}
                      <span class="unassigned">Unassigned</span>
                    {/if}
                    <div class="transition-wrap">
                      <button
                        class="change-btn"
                        onclick={async () => {
                          if (!assigneeOpen) {
                            await loadAssignables();
                            assigneeOpen = true;
                          } else {
                            assigneeOpen = false;
                          }
                        }}
                        disabled={assigneeWorking}
                        title="Change assignee"
                      >
                        {assigneeWorking ? 'Working…' : 'Change ▾'}
                      </button>
                      {#if assigneeOpen}
                        <div class="dropdown-menu">
                          {#if assignablesLoading}
                            <div class="dropdown-loading">Loading…</div>
                          {:else if assignables.length === 0}
                            <div class="dropdown-empty">No users found</div>
                          {:else}
                            <button class="dropdown-item" onclick={() => assignUser('')}>
                              <span class="unassigned-opt">Unassign</span>
                            </button>
                            {#each assignables as u (u.account_id)}
                              <button class="dropdown-item" onclick={() => assignUser(u.account_id)}>
                                {#if u.avatar_url}
                                  <img class="avatar-sm" src={u.avatar_url} alt={u.display_name} />
                                {/if}
                                {u.display_name}
                              </button>
                            {/each}
                          {/if}
                        </div>
                      {/if}
                    </div>
                  </div>
                </div>
              </div>

              <!-- ── Details ─────────────────────────────────────── -->
              <div class="jira-card collapsible-card">
                <button
                  class="jira-coll-trigger"
                  onclick={() => toggleSection('details')}
                  aria-expanded={!collapsed.details}
                >
                  <span class="coll-arrow">{collapsed.details ? '▶' : '▼'}</span>
                  <span class="jira-section-label">Details</span>
                </button>
                {#if !collapsed.details}
                  <div class="details-grid">
                    {#if issueFull.reporter}
                      <span class="detail-key">Reporter</span>
                      <span class="detail-val">
                        {#if editingField === 'reporter' && editableFor('reporter')}
                          {@render fieldEditor(editableFor('reporter')!)}
                        {:else}
                          <div class="detail-val-row">
                            <div class="user-row-sm">
                              {#if issueFull.reporter.avatar_url}
                                <img class="avatar-sm" src={issueFull.reporter.avatar_url} alt={issueFull.reporter.display_name} />
                              {/if}
                              {issueFull.reporter.display_name}
                            </div>
                            {@render editBtn('reporter', issueFull.reporter.display_name)}
                          </div>
                        {/if}
                      </span>
                    {/if}

                    <span class="detail-key">Priority</span>
                    <span class="detail-val">
                      {#if editingField === 'priority' && editableFor('priority')}
                        {@render fieldEditor(editableFor('priority')!)}
                      {:else}
                        <div class="detail-val-row">
                          <span>{issueFull.priority ?? '—'}</span>
                          {@render editBtn('priority', issueFull.priority ?? '')}
                        </div>
                      {/if}
                    </span>

                    <!-- Story Points / Estimate — always editable when editmeta exposes a
                         numeric estimate field, even if currently empty (the "add" case). -->
                    {#if estimateField}
                      <span class="detail-key">{estimateField.name}</span>
                      <span class="detail-val">
                        {#if editingField === estimateField.key}
                          {@render fieldEditor(estimateField)}
                        {:else}
                          <div class="detail-val-row">
                            {#if issueFull.estimate}
                              <span class="estimate-chip">{issueFull.estimate}</span>
                            {:else}
                              <span class="detail-empty">No estimate</span>
                            {/if}
                            {@render editBtn(estimateField.key, rawFieldValue(estimateField.key))}
                          </div>
                        {/if}
                      </span>
                    {:else if issueFull.estimate}
                      <span class="detail-key">Estimate</span>
                      <span class="detail-val"><span class="estimate-chip">{issueFull.estimate}</span></span>
                    {/if}

                    <span class="detail-key">Labels</span>
                    <span class="detail-val">
                      {#if editingField === 'labels' && editableFor('labels')}
                        {@render fieldEditor(editableFor('labels')!)}
                      {:else}
                        <div class="detail-val-row">
                          {#if issueFull.labels && issueFull.labels.length > 0}
                            <div class="label-chips">
                              {#each issueFull.labels as lbl (lbl)}
                                <span class="label-chip">{lbl}</span>
                              {/each}
                            </div>
                          {:else}
                            <span class="detail-empty">No labels</span>
                          {/if}
                          {@render editBtn('labels', (issueFull.labels ?? []).join(', '))}
                        </div>
                      {/if}
                    </span>

                    {#each issueFull.fields.filter((f) => f.value && f.value.trim() !== '') as field (field.key)}
                      {#if field.key === estimateField?.key || (estimateField === null && /story\s*point/i.test(field.name))}
                        <!-- already rendered as the dedicated estimate row above, or matches story-points heuristic -->
                      {:else}
                        <span class="detail-key">{field.name}</span>
                        <span class="detail-val">
                          {#if editingField === field.key && editableFor(field.key)}
                            {@render fieldEditor(editableFor(field.key)!)}
                          {:else}
                            <div class="detail-val-row">
                              <span>{field.value}</span>
                              {@render editBtn(field.key, field.value)}
                            </div>
                          {/if}
                        </span>
                      {/if}
                    {/each}

                    <!-- Empty-field add rows: editmeta fields with no current value,
                         not already rendered by a dedicated row above. -->
                    {#if editmeta}
                      {#each editmeta.filter((ef) => {
                        // Skip fields rendered by dedicated controls above.
                        if (ef.key === 'priority' || ef.key === 'labels') return false;
                        if (estimateField && ef.key === estimateField.key) return false;
                        if (!estimateField && /story\s*point/i.test(ef.name)) return false;
                        // Skip if there's already a populated value row for this field.
                        const existing = issueFull?.fields.find((f) => f.key === ef.key);
                        if (existing && existing.value && existing.value.trim() !== '') return false;
                        return true;
                      }) as ef (ef.key)}
                        <span class="detail-key detail-key-empty">{ef.name}</span>
                        <span class="detail-val">
                          {#if editingField === ef.key}
                            {@render fieldEditor(ef)}
                          {:else}
                            <div class="detail-val-row">
                              <span class="detail-empty">—</span>
                              <button
                                class="field-edit-btn"
                                title="Set {ef.name}"
                                aria-label="Set {ef.name}"
                                onclick={() => beginEdit(ef, '')}
                              >+</button>
                            </div>
                          {/if}
                        </span>
                      {/each}
                    {/if}
                  </div>
                {/if}
              </div>

              <!-- ── Linked Issues ───────────────────────────────── -->
              {#if issueFull.links && issueFull.links.length > 0}
                <div class="jira-card collapsible-card">
                  <button
                    class="jira-coll-trigger"
                    onclick={() => toggleSection('links')}
                    aria-expanded={!collapsed.links}
                  >
                    <span class="coll-arrow">{collapsed.links ? '▶' : '▼'}</span>
                    <span class="jira-section-label">Linked Issues</span>
                    <span class="section-count">({issueFull.links.length})</span>
                  </button>
                  {#if !collapsed.links}
                    <div class="links-list">
                      {#each issueFull.links as lnk (lnk.key)}
                        <div class="link-row">
                          <span class="link-rel">{lnk.rel}</span>
                          <span class="link-key mono-sm">{lnk.key}</span>
                          <span class="link-type chip-sm">{lnk.issue_type}</span>
                          <span class="link-summary">{lnk.summary}</span>
                          <span class="link-status status-sm">{lnk.status}</span>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/if}

              <!-- ── Development (linked branches / commits / PRs) ─── -->
              <div class="jira-card collapsible-card">
                <button
                  class="jira-coll-trigger"
                  onclick={() => toggleSection('development')}
                  aria-expanded={!collapsed.development}
                >
                  <span class="coll-arrow">{collapsed.development ? '▶' : '▼'}</span>
                  <span class="jira-section-label">Development</span>
                  {#if devStatus}
                    <span class="section-count">
                      ({devStatus.branches.length + devStatus.commits.length + devStatus.pull_requests.length})
                    </span>
                  {/if}
                </button>
                {#if !collapsed.development}
                  <div class="dev-body">
                    {#if devLoading}
                      <div class="dropdown-loading">Loading development info…</div>
                    {:else if devError}
                      <div class="jira-error">Could not load development info: {devError}</div>
                    {:else if devStatus && (devStatus.branches.length || devStatus.commits.length || devStatus.pull_requests.length)}
                      {#if devStatus.pull_requests.length}
                        <div class="dev-group">
                          <span class="dev-group-label">Pull requests</span>
                          {#each devStatus.pull_requests as pr (pr.repo + ':' + pr.id)}
                            <a
                              class="dev-row"
                              href={pr.url}
                              target="_blank"
                              rel="noopener noreferrer"
                            >
                              <span class="dev-pr-status status-sm">{pr.status}</span>
                              <span class="dev-pr-name">{pr.name}</span>
                              <span class="dev-repo chip-sm">{pr.repo}</span>
                            </a>
                          {/each}
                        </div>
                      {/if}
                      {#if devStatus.branches.length}
                        <div class="dev-group">
                          <span class="dev-group-label">Branches</span>
                          {#each devStatus.branches as b (b.repo + ':' + b.name)}
                            <a
                              class="dev-row"
                              href={b.url}
                              target="_blank"
                              rel="noopener noreferrer"
                            >
                              <span class="dev-branch-name mono-sm">{b.name}</span>
                              <span class="dev-repo chip-sm">{b.repo}</span>
                            </a>
                          {/each}
                        </div>
                      {/if}
                      {#if devStatus.commits.length}
                        <div class="dev-group">
                          <span class="dev-group-label">Commits</span>
                          {#each devStatus.commits as c (c.repo + ':' + c.id)}
                            <a
                              class="dev-row"
                              href={c.url}
                              target="_blank"
                              rel="noopener noreferrer"
                            >
                              <span class="dev-commit-id mono-sm">{c.id.slice(0, 8)}</span>
                              <span class="dev-commit-msg">{c.message}</span>
                              <span class="dev-repo chip-sm">{c.repo}</span>
                            </a>
                          {/each}
                        </div>
                      {/if}
                    {:else}
                      <div class="comments-empty">
                        No linked development info — connect GitHub/Bitbucket in Jira to see
                        branches, commits and PRs here.
                      </div>
                    {/if}
                  </div>
                {/if}
              </div>

              <!-- ── Comments ─────────────────────────────────────── -->
              <div class="jira-card collapsible-card">
                <button
                  class="jira-coll-trigger"
                  onclick={() => toggleSection('comments')}
                  aria-expanded={!collapsed.comments}
                >
                  <span class="coll-arrow">{collapsed.comments ? '▶' : '▼'}</span>
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
                            <span class="comment-date">{relDate(comment.created)}</span>
                          </div>
                          <div class="comment-body md-body">{@html renderMarkdown(comment.body_md)}</div>
                        </div>
                      {/each}
                    {/if}
                  </div>
                  <!-- Add comment form -->
                  <div class="add-comment-form">
                    <textarea
                      class="textarea comment-textarea"
                      bind:value={newCommentBody}
                      rows={3}
                      placeholder="Add a comment…"
                      spellcheck="true"
                      disabled={postingComment}
                    ></textarea>
                    <div class="add-comment-row">
                      <button
                        class="toolbar-btn comment-submit-btn"
                        onclick={addComment}
                        disabled={postingComment || !newCommentBody.trim()}
                      >
                        {postingComment ? 'Posting…' : 'Comment'}
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
                    <span class="coll-arrow">{collapsed.history ? '▶' : '▼'}</span>
                    <span class="jira-section-label">History</span>
                    <span class="section-count">({issueFull.history.length} entries)</span>
                  </button>
                  {#if !collapsed.history}
                    <div class="history-list">
                      {#each issueFull.history as entry, i (i)}
                        <div class="history-entry">
                          <div class="history-meta">
                            <span class="history-author">{entry.author}</span>
                            <span class="history-date">{relDate(entry.created)}</span>
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

              <!-- ── Attachments ─────────────────────────────────── -->
              {#if issueFull.attachments && issueFull.attachments.length > 0}
                <div class="jira-card collapsible-card">
                  <button
                    class="jira-coll-trigger"
                    onclick={() => toggleSection('attachments')}
                    aria-expanded={!collapsed.attachments}
                  >
                    <span class="coll-arrow">{collapsed.attachments ? '▶' : '▼'}</span>
                    <span class="jira-section-label">Attachments</span>
                    <span class="section-count">({issueFull.attachments.length})</span>
                  </button>
                  {#if !collapsed.attachments}
                    <div class="attachments-grid">
                      {#each issueFull.attachments as att (att.id)}
                        <div class="attachment-card">
                          <div class="att-header">
                            <span class="att-filename" title={att.filename}>{att.filename}</span>
                            <span class="att-meta">{fmtBytes(att.size)} · {att.mime}</span>
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
                                  {attachmentLoading[att.id] ? 'Loading…' : 'Load preview'}
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
                                  {attachmentLoading[att.id] ? 'Loading…' : 'Preview PDF'}
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

            {/if}
          </div>

          <!-- Attachments panel in Jira right column -->
          <AttachmentsPanel bind:this={panelRef} />
        </div>
      </div>

    {:else}
      <!-- ── CONFLUENCE / other: single-column read-only body ──────── -->
      <div class="body-wrap">
        {#if viewingVersion}
          <div class="version-banner">
            Viewing v{viewingVersion.version_no} ({viewingVersion.kind})
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
          <button
            class="publish-btn"
            onclick={() => (publishDialogMode = 'story')}
          >
            Convert to Jira Story
          </button>
        </div>
      {/if}

      <!-- Attachments panel for Confluence / other layouts -->
      <AttachmentsPanel bind:this={panelRef} />
    {/if}

    <!-- ── Swarm link (cross-link back to the project this story spawned) ── -->
    <SwarmLinkCard storyId={story.id} />

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
    font-size: 13px;
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

  /* Responsive: collapse to single column below 900px */
  @media (max-width: 900px) {
    .two-col {
      grid-template-columns: 1fr;
    }
  }

  /* ── Tags ──────────────────────────────────────────────────── */
  .tags-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 5px;
    margin-top: 8px;
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 2px 8px 2px 9px;
    border-radius: 999px;
    font-size: 10.5px;
    font-weight: 500;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .tag-remove {
    background: none;
    border: none;
    padding: 0 1px;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    color: var(--accent);
    opacity: 0.6;
    transition: opacity 100ms;
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
    font-size: 10.5px;
    padding: 2px 9px;
    width: 72px;
    outline: none;
    transition: border-color 120ms, width 120ms;
  }
  .tag-input:focus {
    border-color: var(--accent);
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
    font-size: 10.5px;
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
    border-radius: var(--radius-s, 4px);
    padding: 5px 10px;
    cursor: pointer;
    text-align: start;
    font-size: 12.5px;
    color: var(--text);
    transition: background 100ms;
  }
  .related-item:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
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
    font-size: 9.5px;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
  }

  /* Story header */
  .story-header {
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 12px;
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
    font-size: 20px;
    font-weight: 700;
    line-height: 1.25;
    color: var(--text);
  }
  .stage-badge {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 2px 8px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .stage-select {
    border: 1px solid color-mix(in srgb, currentColor 35%, transparent);
    cursor: pointer;
    appearance: none;
    padding-inline: 8px 8px;
  }
  .stage-draft {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .stage-review {
    background: color-mix(in srgb, #f59e0b 18%, transparent);
    color: #b45309;
  }
  .stage-approved {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .stage-done {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .stage-other {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .chip {
    font-size: 10.5px;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .source-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    color: var(--accent);
    text-decoration: none;
  }
  .source-link:hover {
    text-decoration: underline;
  }
  .source-key {
    font-size: 11.5px;
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
    font-size: 11px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
    padding: 2px 8px;
    border-radius: 999px;
  }

  /* Toolbar */
  .toolbar {
    display: flex;
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
    font-size: 11px;
    color: var(--text-dim);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }
  .ver-select {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
    max-width: 280px;
  }
  .ver-loading {
    font-size: 11px;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .toolbar-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    transition: background 110ms, border-color 110ms, color 110ms;
    white-space: nowrap;
  }
  .toolbar-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .toolbar-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .toolbar-btn.active {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  /* Discovery swarm picker (toolbar) — matches PlanTab .swarm-pick style */
  .disc-swarm-pick {
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
  }

  /* Body */
  .body-wrap {
    flex: 1;
  }
  .version-banner {
    font-size: 11.5px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
    border-radius: var(--radius-s);
    padding: 6px 12px;
    margin-bottom: 12px;
  }
  .change-notes {
    font-style: italic;
  }
  .md-body {
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text);
  }
  /* Markdown element styling */
  .md-body :global(h1),
  .md-body :global(h2),
  .md-body :global(h3),
  .md-body :global(h4) {
    margin: 1.2em 0 0.4em;
    font-weight: 700;
    line-height: 1.25;
    color: var(--text);
  }
  .md-body :global(h1) { font-size: 1.35em; }
  .md-body :global(h2) { font-size: 1.2em; }
  .md-body :global(h3) { font-size: 1.05em; }
  .md-body :global(p) {
    margin: 0 0 0.75em;
  }
  .md-body :global(ul),
  .md-body :global(ol) {
    padding-inline-start: 1.5em;
    margin: 0 0 0.75em;
  }
  .md-body :global(li) {
    margin-bottom: 0.25em;
  }
  .md-body :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.88em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    padding: 1px 5px;
    border-radius: 3px;
  }
  .md-body :global(pre) {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    overflow-x: auto;
    margin: 0 0 0.75em;
  }
  .md-body :global(pre code) {
    background: none;
    padding: 0;
    font-size: 0.86em;
  }
  .md-body :global(blockquote) {
    border-inline-start: 3px solid var(--border);
    padding-inline-start: 12px;
    color: var(--text-dim);
    margin: 0 0 0.75em;
    font-style: italic;
  }
  .md-body :global(a) {
    color: var(--accent);
    text-decoration: none;
  }
  .md-body :global(a:hover) {
    text-decoration: underline;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }

  /* ── Jira section (right column) ──────────────────────────── */
  .jira-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .jira-loading,
  .jira-error {
    font-size: 12.5px;
    color: var(--text-dim);
    font-style: italic;
    padding: 8px 0;
  }
  .jira-error {
    color: #b91c1c;
    font-style: normal;
  }

  /* ── Jira card ─────────────────────────────────────────────── */
  .jira-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 14px;
    background: var(--surface-raised, var(--surface));
  }
  .collapsible-card {
    padding: 0;
  }
  .jira-card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .jira-section-label {
    font-size: 11px;
    font-weight: 700;
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
    font-size: 12px;
    border-radius: var(--radius-s);
    transition: background 100ms;
  }
  .jira-coll-trigger:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .coll-arrow {
    font-size: 9px;
    flex-shrink: 0;
  }
  .section-count {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-dim);
    margin-inline-start: 2px;
  }

  /* ── Status control ────────────────────────────────────────── */
  .status-control {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status-badge {
    font-size: 12px;
    font-weight: 600;
    padding: 2px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }

  /* ── Assignee control ──────────────────────────────────────── */
  .assignee-control {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .user-row {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .user-row-sm {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12.5px;
  }
  .avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }
  .avatar-sm {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }
  .avatar-placeholder {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
    font-size: 11px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .user-name {
    font-size: 12.5px;
    color: var(--text);
  }
  .unassigned {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
  }
  .unassigned-opt {
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Dropdown ──────────────────────────────────────────────── */
  .transition-wrap {
    position: relative;
  }
  .change-btn {
    height: 24px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
    white-space: nowrap;
    transition: background 100ms, color 100ms;
  }
  .change-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .change-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .dropdown-menu {
    position: absolute;
    top: calc(100% + 4px);
    inset-inline-end: 0;
    z-index: 50;
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    min-width: 180px;
    max-height: 240px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .dropdown-loading,
  .dropdown-empty {
    font-size: 12px;
    color: var(--text-dim);
    padding: 10px 12px;
    font-style: italic;
  }
  .dropdown-item {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 7px 12px;
    background: none;
    border: none;
    text-align: start;
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    transition: background 80ms;
    white-space: nowrap;
  }
  .dropdown-item:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
  }
  .dropdown-item-sub {
    font-size: 11px;
    color: var(--text-dim);
    margin-inline-start: 4px;
  }

  /* ── Details grid ──────────────────────────────────────────── */
  .details-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    padding: 0 14px 12px;
    align-items: start;
  }
  .detail-key {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
    padding-top: 1px;
  }
  .detail-key-empty {
    opacity: 0.65;
  }
  .detail-val {
    font-size: 12.5px;
    color: var(--text);
    line-height: 1.4;
  }
  .label-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .label-chip {
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }

  /* ── Editable detail fields ────────────────────────────────── */
  .detail-val-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .detail-empty {
    color: var(--text-dim);
    font-style: italic;
  }
  .field-edit-btn {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    line-height: 1;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 100ms, background 100ms, color 100ms;
  }
  .detail-val-row:hover .field-edit-btn,
  .field-edit-btn:focus-visible {
    opacity: 1;
  }
  .field-edit-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .field-editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }
  .field-input {
    width: 100%;
    max-width: 240px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 12.5px;
  }
  .field-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .field-multiselect {
    display: flex;
    flex-direction: column;
    gap: 3px;
    max-height: 160px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .field-check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text);
    cursor: pointer;
  }
  .field-raw-note {
    font-size: 10.5px;
    color: var(--text-dim);
    font-style: italic;
  }
  .field-editor-actions {
    display: flex;
    gap: 6px;
  }
  .field-save-btn,
  .field-cancel-btn {
    height: 24px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    font-size: 11.5px;
    cursor: pointer;
    transition: background 100ms, color 100ms;
  }
  .field-save-btn {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--bg, #fff);
  }
  .field-save-btn:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .field-cancel-btn {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
  }
  .field-cancel-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .field-save-btn:disabled,
  .field-cancel-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ── Links ─────────────────────────────────────────────────── */
  .links-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 14px 10px;
  }
  .link-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .link-rel {
    font-size: 10.5px;
    color: var(--text-dim);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    min-width: 70px;
  }
  .link-key {
    font-size: 11.5px;
    color: var(--accent);
    font-family: var(--font-mono, monospace);
  }
  .chip-sm {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text-dim);
  }
  .link-summary {
    font-size: 12.5px;
    color: var(--text);
    flex: 1;
    min-width: 120px;
  }
  .status-sm {
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .mono-sm {
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
  }

  /* ── Development (branches / commits / PRs) ────────────────── */
  .dev-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 0 14px 10px;
  }
  .dev-group {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .dev-group-label {
    font-size: 10.5px;
    color: var(--text-dim);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 6px 0 2px;
  }
  .dev-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
    text-decoration: none;
    color: inherit;
  }
  .dev-row:hover {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .dev-pr-name,
  .dev-commit-msg {
    font-size: 12.5px;
    color: var(--text);
    flex: 1;
    min-width: 120px;
  }
  .dev-branch-name,
  .dev-commit-id {
    color: var(--accent);
  }
  .dev-pr-status {
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .dev-repo {
    margin-inline-start: auto;
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
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .comment-date {
    font-size: 11px;
    color: var(--text-dim);
  }
  .comment-body {
    font-size: 12.5px;
    line-height: 1.55;
  }
  .comments-empty {
    padding: 10px 14px 6px;
    font-size: 12px;
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
    font-size: 12.5px;
    line-height: 1.5;
  }
  .add-comment-row {
    display: flex;
    justify-content: flex-end;
  }
  .comment-submit-btn {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .comment-submit-btn:hover:not(:disabled) {
    opacity: 0.88;
    background: var(--accent);
    color: #fff;
  }

  /* ── Estimate chip ─────────────────────────────────────────── */
  .estimate-chip {
    display: inline-block;
    font-size: 11.5px;
    font-weight: 600;
    padding: 2px 9px;
    border-radius: 999px;
    background: color-mix(in srgb, #f59e0b 15%, transparent);
    color: #b45309;
    border: 1px solid color-mix(in srgb, #f59e0b 30%, transparent);
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
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .history-date {
    font-size: 11px;
    color: var(--text-dim);
  }
  .history-change {
    font-size: 12px;
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
    border-radius: 3px;
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
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .att-meta {
    font-size: 11px;
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
    font-size: 11.5px;
    cursor: pointer;
    transition: background 100ms, color 100ms;
  }
  .att-load-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
    border-color: var(--accent);
  }
  .att-load-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .att-dl-link {
    font-size: 12px;
    color: var(--accent);
    text-decoration: none;
  }
  .att-dl-link:hover {
    text-decoration: underline;
  }

  /* ── Draft editing ─────────────────────────────────────────── */
  .draft-section {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding-top: 4px;
  }
  .draft-hint {
    font-size: 12px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 22%, transparent);
    border-radius: var(--radius-s);
    padding: 8px 12px;
    line-height: 1.5;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .input {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 13px;
    padding: 6px 10px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .input:focus {
    border-color: var(--accent);
  }
  .textarea {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12.5px;
    font-family: var(--font-mono, monospace);
    padding: 8px 10px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    line-height: 1.55;
  }
  .textarea:focus {
    border-color: var(--accent);
  }
  .draft-save-row {
    display: flex;
    justify-content: flex-end;
  }
  .save-btn {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .save-btn:hover:not(:disabled) {
    opacity: 0.88;
  }

  /* ── Transcripts (right column in draft mode) ──────────────── */
  .transcripts-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 4px;
  }
  .transcripts-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .section-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .transcript-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .transcript-item {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .transcript-header {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .transcript-toggle {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: 8px 10px;
    cursor: pointer;
    text-align: start;
    color: var(--text);
    font-size: 12.5px;
    transition: background 80ms;
  }
  .transcript-toggle:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .transcript-title {
    flex: 1;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .transcript-date {
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .del-transcript-btn {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    background: none;
    border: none;
    cursor: pointer;
    font-size: 11px;
    color: var(--text-dim);
    transition: color 80ms, background 80ms;
    margin-inline-end: 4px;
    border-radius: var(--radius-s);
  }
  .del-transcript-btn:hover {
    color: #ef4444;
    background: color-mix(in srgb, #ef4444 12%, transparent);
  }
  .transcript-body {
    padding: 8px 12px 10px;
    font-size: 12px;
    white-space: pre-wrap;
    color: var(--text-dim);
    line-height: 1.55;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--text-dim) 3%, transparent);
    font-family: var(--font-mono, monospace);
    max-height: 320px;
    overflow-y: auto;
  }
  .add-transcript-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
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
  .publish-btn {
    height: 32px;
    padding: 0 16px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: #fff;
    transition: opacity 110ms;
    white-space: nowrap;
  }
  .publish-btn:hover {
    opacity: 0.88;
  }
  .publish-btn.secondary {
    background: transparent;
    color: var(--accent);
  }
  .publish-btn.secondary:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    opacity: 1;
  }
</style>
