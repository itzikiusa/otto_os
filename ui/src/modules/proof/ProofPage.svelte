<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  // Proof section: a two-pane viewer of proof packs. Left = status filter chips
  // + the pack list; right = the open pack's detail (badges, artifacts grouped
  // by kind, and assemble / add-artifact / waive / delete actions).
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import ProofBadges from '../../lib/components/ProofBadges.svelte';
  import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
  import DoneContractMeter from '../../lib/components/DoneContractMeter.svelte';
  import ProofSnapshotList from '../../lib/components/ProofSnapshotList.svelte';
  import { proof } from '../../lib/stores/proof.svelte';
  import {
    addArtifact,
    artifactBlobUrl,
    artifactContent,
    assembleProof,
    attachApiEvidence,
    attachDbEvidence,
    attachKafkaEvidence,
    attachMedia,
    ciRefresh,
    createProofPack,
    deleteArtifact,
    deleteProofPack,
    getProofPack,
    getRepoProofConfig,
    listProofPacks,
    proofReport,
    runPrCheck,
    setRepoProofConfig,
    waiveProof,
  } from '../../lib/api/proof';
  import { downloadText } from '../../lib/components/exporters';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { registry } from '../../lib/commands.svelte';
  import type { ProofArtifactView } from '../../lib/api/types';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { sentenceCase } from '../../lib/status';

  /** Enum → words for kinds and statuses (self_review → "Self review",
   *  pr_check → "PR check", ci → "CI", api → "API", db → "Database"). */
  const WORDS: Record<string, string> = { ci: 'CI', api: 'API', db: 'Database', pr_check: 'PR check', self_review: 'Self review', all: 'All' };
  function words(raw: string | null | undefined): string {
    const k = (raw ?? '').trim();
    return WORDS[k] ?? sentenceCase(k || 'unknown');
  }

  const MEDIA_KINDS = new Set(['screenshot', 'video']);

  const STATUS_FILTERS = ['all', 'passed', 'failed', 'partial', 'missing', 'waived'] as const;
  type StatusFilter = (typeof STATUS_FILTERS)[number];
  let filter = $state<StatusFilter>('all');

  const ARTIFACT_KINDS = [
    'command', 'log', 'screenshot', 'diff', 'ci', 'api', 'db', 'review', 'approval', 'self_review',
  ];
  const ARTIFACT_STATUSES = ['info', 'passed', 'failed', 'pending'];

  // Load the list (for the active filter) + the summary roll-up for this ws.
  // The store swallows a failed list load into an empty list; an empty result
  // is re-checked here so a failure shows inline (with Retry) instead of the
  // "No proof packs yet" empty state.
  let listError = $state<string | null>(null);
  let listLoaded = $state(false);
  async function loadList(id: string, f: StatusFilter): Promise<void> {
    const q = f === 'all' ? undefined : { status: f };
    await proof.loadPacks(id, q);
    if (proof.packs.length === 0) {
      try {
        await listProofPacks(id, q);
        listError = null;
      } catch (e) {
        listError = loadErrorText(e);
      }
    } else {
      listError = null;
    }
    listLoaded = true;
  }
  function retryList(): void {
    if (ws.currentId) void loadList(ws.currentId, filter);
  }
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    const f = filter;
    void loadList(id, f);
    void proof.loadSummary(id);
  });

  const detail = $derived(proof.detail);

  // Never open onto an empty "select a pack" pane when packs exist: restore the
  // last-opened pack (or the first) once per workspace load. Not on a phone,
  // where an open pack replaces the list.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || autoPickedFor === wsId || viewport.isPhone) return;
    if (proof.detail) {
      autoPickedFor = wsId;
      return;
    }
    if (proof.loading || proof.packs.length === 0) return;
    autoPickedFor = wsId;
    const id = initialSelection('proof', proof.packs, (p) => p.id);
    if (id) open(id);
  });
  $effect(() => {
    if (detail?.pack.id) rememberSelection('proof', detail.pack.id);
  });
  // An empty, unfiltered list has nothing to show: the page-level empty state
  // owns the page (a filtered-empty list keeps the rail so the filter can change).
  const showRail = $derived(proof.packs.length > 0 || filter !== 'all');
  const filterCounts = $derived.by(() => {
    // Counts only mean something on the unfiltered list.
    if (filter !== 'all') return null;
    const c: Record<string, number> = { all: proof.packs.length };
    for (const p of proof.packs) c[p.status] = (c[p.status] ?? 0) + 1;
    return c;
  });

  // Group the open pack's artifacts by kind for display.
  const artifactGroups = $derived.by((): [string, ProofArtifactView[]][] => {
    if (!detail) return [];
    const by: Record<string, ProofArtifactView[]> = {};
    for (const a of detail.artifacts) (by[a.kind] ??= []).push(a);
    return Object.entries(by);
  });

  // Per-artifact: previews are expandable, and "Load full" pulls uncapped content.
  let expanded = $state<Record<string, boolean>>({});
  let fullContent = $state<Record<string, string>>({});

  function toggleExpand(id: string): void {
    expanded[id] = !expanded[id];
  }

  /** One line of a PR-consistency report (mirrors PrConsistencyCheck). */
  interface PrCheckLine {
    label: string;
    passed: boolean;
    detail: string;
  }
  /** The PR-consistency report a `pr_check` artifact carries in `metadata.report`. */
  interface PrCheckReport {
    score: number;
    passed: boolean;
    hard_fail: boolean;
    checks: PrCheckLine[];
  }
  /** Extract the structured PR-consistency report from a pr_check artifact's
   *  metadata, or null when it isn't present/shaped as expected. */
  function prReport(meta: unknown): PrCheckReport | null {
    const r = (meta as { report?: unknown } | null | undefined)?.report as
      | PrCheckReport
      | undefined;
    return r && Array.isArray(r.checks) ? r : null;
  }

  async function loadFull(id: string): Promise<void> {
    try {
      const c = await artifactContent(id);
      fullContent[id] = c.content ?? '(no content)';
      expanded[id] = true;
    } catch (e) {
      toasts.error("Couldn't load the artifact", loadErrorText(e));
    }
  }

  // Opens a pack into the detail pane. The store's own open() drops a failure
  // silently (the pane just stays as it was), so load here and say so.
  let openingId = $state<string | null>(null);
  async function open(id: string): Promise<void> {
    openingId = id;
    try {
      proof.detail = await getProofPack(id);
    } catch (e) {
      toasts.error("Couldn't open the proof pack", loadErrorText(e));
    } finally {
      if (openingId === id) openingId = null;
    }
  }

  // ---- inline media (R4): object URLs for screenshot/video artifacts --------
  // Keyed by artifact id. The first effect (re)syncs the set against the open
  // pack; the second revokes every URL on unmount. `untrack` keeps the effect's
  // only dependency `detail` — reads/writes of `mediaUrls` must not re-trigger it.
  let mediaUrls = $state<Record<string, string>>({});

  $effect(() => {
    const wanted = (detail?.artifacts ?? [])
      .filter((a) => MEDIA_KINDS.has(a.kind))
      .map((a) => a.id);
    untrack(() => syncMedia(wanted));
  });

  $effect(() => () => untrack(() => {
    for (const url of Object.values(mediaUrls)) URL.revokeObjectURL(url);
  }));

  function syncMedia(wanted: string[]): void {
    const set = new Set(wanted);
    for (const id of wanted) if (!(id in mediaUrls)) void fetchMedia(id);
    const stale = Object.keys(mediaUrls).some((id) => !set.has(id));
    if (stale) {
      const next: Record<string, string> = {};
      for (const [id, url] of Object.entries(mediaUrls)) {
        if (set.has(id)) next[id] = url;
        else URL.revokeObjectURL(url);
      }
      mediaUrls = next;
    }
  }

  async function fetchMedia(id: string): Promise<void> {
    if (id in mediaUrls) return;
    try {
      mediaUrls[id] = await artifactBlobUrl(id);
    } catch {
      /* media blob unavailable — leave the loading placeholder */
    }
  }

  // ---- Add-artifact modal --------------------------------------------------
  let addOpen = $state(false);
  let aKind = $state('command');
  let aTitle = $state('');
  let aContent = $state('');
  let aStatus = $state('info');

  function resetAdd(): void {
    aKind = 'command';
    aTitle = '';
    aContent = '';
    aStatus = 'info';
  }

  async function submitAdd(): Promise<void> {
    if (!detail || !aTitle.trim()) return;
    try {
      await addArtifact(detail.pack.id, {
        kind: aKind,
        title: aTitle.trim(),
        content: aContent.trim() || undefined,
        status: aStatus,
      });
      await proof.refreshDetail();
      addOpen = false;
      resetAdd();
    } catch (e) {
      toasts.error("Couldn't add the artifact", e instanceof Error ? e.message : String(e));
    }
  }

  async function removeArtifact(id: string): Promise<void> {
    if (!detail) return;
    const name = detail.artifacts.find((x) => x.id === id)?.title;
    if (!(await confirmer.ask(name ? `Delete the artifact “${name}” from this proof pack? The pack's status is re-derived without it.` : 'Delete this artifact from the proof pack?', { title: 'Delete artifact' }))) return;
    try {
      await deleteArtifact(id);
      await proof.refreshDetail();
    } catch (e) {
      toasts.error("Couldn't delete the artifact", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- pack-level actions --------------------------------------------------
  // Anchored under the button (not at the cursor); ctxMenu clamps it. When
  // the button is collapsed into the header's ⋯ menu, showAt falls back to
  // that ⋯ button.
  function openAddMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Add artifact…', icon: 'plus', action: () => { resetAdd(); addOpen = true; } },
      { label: 'Add media…', icon: 'file', action: () => { resetMedia(); mediaOpen = true; } },
      { label: 'Add evidence…', icon: 'db', action: () => { resetEvidence(); evidenceOpen = true; } },
      { label: 'PR check…', icon: 'pr', action: () => { resetPr(); prOpen = true; } },
    ]);
  }

  async function assemble(): Promise<void> {
    if (!detail) return;
    const cwd = await confirmer.promptText('Working directory to assemble proof from:', {
      title: 'Assemble proof',
      browseFolder: true,
      confirmLabel: 'Assemble',
      placeholder: '/path/to/repo',
    });
    if (cwd === null) return;
    try {
      await assembleProof(detail.pack.id, { cwd: cwd.trim() || undefined });
      await proof.refreshDetail();
      toasts.success('Proof assembled', 'Re-assembled from the working directory.');
    } catch (e) {
      toasts.error("Couldn't assemble proof", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- waive (R10): require an approver reason (≥10 chars) -----------------
  let waiveOpen = $state(false);
  let waiveReason = $state('');

  async function submitWaive(): Promise<void> {
    if (!detail || waiveReason.trim().length < 10) return;
    try {
      await waiveProof(detail.pack.id, waiveReason.trim());
      await proof.refreshDetail();
      waiveOpen = false;
      waiveReason = '';
      toasts.success('Proof gate waived', 'Recorded with you as the approver.');
    } catch (e) {
      toasts.error("Couldn't waive the proof gate", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- media evidence (R4): screenshot / video upload ----------------------
  let mediaOpen = $state(false);
  let mKind = $state('screenshot');
  let mTitle = $state('');
  let mFile = $state<File | null>(null);

  function resetMedia(): void {
    mKind = 'screenshot';
    mTitle = '';
    mFile = null;
  }

  /** Read a File → base64 (no data-URL prefix). */
  function fileToB64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(reader.error);
      reader.onload = () => {
        const result = reader.result as string;
        const idx = result.indexOf(',');
        resolve(idx >= 0 ? result.slice(idx + 1) : result);
      };
      reader.readAsDataURL(blob);
    });
  }

  async function submitMedia(): Promise<void> {
    if (!detail || !mFile || !mTitle.trim()) return;
    if (mFile.size > 25 * 1024 * 1024) {
      toasts.warn('File too large', 'Media evidence must be 25 MiB or smaller.');
      return;
    }
    try {
      const data_base64 = await fileToB64(mFile);
      await attachMedia(detail.pack.id, {
        kind: mKind === 'video' ? 'video' : 'screenshot',
        title: mTitle.trim(),
        mime: mFile.type || (mKind === 'video' ? 'video/mp4' : 'image/png'),
        data_base64,
      });
      await proof.refreshDetail();
      mediaOpen = false;
      resetMedia();
    } catch (e) {
      toasts.error("Couldn't attach the media", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- API / DB / Kafka evidence (R5/R6) -----------------------------------
  let evidenceOpen = $state(false);
  let eType = $state('api');
  let eTitle = $state('');
  let eMethod = $state('GET');
  let eUrl = $state('');
  let eStatus = $state('200');
  let eEngine = $state('');
  let eQuery = $state('');
  let eRowCount = $state('');
  let eTopic = $state('');
  let eMsgCount = $state('');
  let eSample = $state('');
  let eResponse = $state('');

  function resetEvidence(): void {
    eType = 'api';
    eTitle = '';
    eMethod = 'GET';
    eUrl = '';
    eStatus = '200';
    eEngine = '';
    eQuery = '';
    eRowCount = '';
    eTopic = '';
    eMsgCount = '';
    eSample = '';
    eResponse = '';
  }

  function numOrUndef(s: string): number | undefined {
    const t = s.trim();
    if (!t) return undefined;
    const n = Number(t);
    return Number.isFinite(n) ? n : undefined;
  }

  const evidenceValid = $derived(
    eTitle.trim().length > 0 &&
      (eType === 'api'
        ? eUrl.trim().length > 0
        : eType === 'kafka'
          ? eTopic.trim().length > 0
          : true),
  );

  async function submitEvidence(): Promise<void> {
    if (!detail || !evidenceValid) return;
    try {
      const id = detail.pack.id;
      if (eType === 'api') {
        await attachApiEvidence(id, {
          title: eTitle.trim(),
          method: eMethod,
          url: eUrl.trim(),
          status: numOrUndef(eStatus) ?? 0,
          response: eResponse.trim() || undefined,
        });
      } else if (eType === 'db') {
        await attachDbEvidence(id, {
          title: eTitle.trim(),
          engine: eEngine.trim() || undefined,
          query: eQuery.trim() || undefined,
          row_count: numOrUndef(eRowCount),
          sample: eSample.trim() || undefined,
        });
      } else {
        await attachKafkaEvidence(id, {
          title: eTitle.trim(),
          topic: eTopic.trim(),
          message_count: numOrUndef(eMsgCount),
          sample: eSample.trim() || undefined,
        });
      }
      await proof.refreshDetail();
      evidenceOpen = false;
      resetEvidence();
    } catch (e) {
      toasts.error("Couldn't add the evidence", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- CI refresh (R2) -----------------------------------------------------
  async function refreshCi(): Promise<void> {
    if (!detail) return;
    try {
      await ciRefresh(detail.pack.id, {});
      await proof.refreshDetail();
      toasts.success('CI refreshed', 'Live CI status pulled into a CI artifact.');
    } catch (e) {
      toasts.error("Couldn't refresh CI", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- PR consistency check (R7) -------------------------------------------
  let prOpen = $state(false);
  let prTitle = $state('');
  let prDesc = $state('');
  let prBase = $state('');
  let prCwd = $state('');

  function resetPr(): void {
    prTitle = '';
    prDesc = '';
    prBase = '';
    prCwd = '';
  }

  async function submitPr(): Promise<void> {
    if (!detail || !prTitle.trim() || !prDesc.trim()) return;
    try {
      await runPrCheck(detail.pack.id, {
        title: prTitle.trim(),
        description: prDesc.trim(),
        base: prBase.trim() || undefined,
        cwd: prCwd.trim() || undefined,
      });
      await proof.refreshDetail();
      prOpen = false;
      resetPr();
    } catch (e) {
      toasts.error("Couldn't run the PR check", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- report export (R9) --------------------------------------------------
  async function exportReport(format: 'md' | 'html'): Promise<void> {
    if (!detail) return;
    try {
      const text = await proofReport(detail.pack.id, format);
      downloadText(
        text,
        `proof-${detail.pack.id}.${format}`,
        format === 'md' ? 'text/markdown' : 'text/html',
      );
    } catch (e) {
      toasts.error("Couldn't export the report", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- per-repo proof requirements (R3) ------------------------------------
  let cfgOpen = $state(false);
  let cfgLoading = $state(false);
  let cfg = $state({
    require_test: false,
    test_cmd: '',
    require_ci: false,
    require_pr_consistency: false,
    require_review: false,
  });

  async function openConfig(): Promise<void> {
    const repoId = detail?.pack.repo_id;
    if (!repoId) return;
    cfgOpen = true;
    cfgLoading = true;
    try {
      const c = await getRepoProofConfig(repoId);
      cfg = {
        require_test: !!c.require_test,
        test_cmd: c.test_cmd ?? '',
        require_ci: !!c.require_ci,
        require_pr_consistency: !!c.require_pr_consistency,
        require_review: !!c.require_review,
      };
    } catch (e) {
      toasts.error("Couldn't load the proof requirements", loadErrorText(e));
      cfgOpen = false;
    } finally {
      cfgLoading = false;
    }
  }

  async function saveConfig(): Promise<void> {
    const repoId = detail?.pack.repo_id;
    if (!repoId) return;
    try {
      await setRepoProofConfig(repoId, {
        require_test: cfg.require_test,
        test_cmd: cfg.test_cmd.trim() || null,
        require_ci: cfg.require_ci,
        require_pr_consistency: cfg.require_pr_consistency,
        require_review: cfg.require_review,
      });
      await proof.refreshDetail();
      cfgOpen = false;
      toasts.success('Requirements saved', 'Proof requirements updated for this repo.');
    } catch (e) {
      toasts.error("Couldn't save the requirements", e instanceof Error ? e.message : String(e));
    }
  }

  async function removePack(): Promise<void> {
    if (!detail) return;
    const t = detail.pack.title || 'this pack';
    const n = detail.artifacts.length;
    if (!(await confirmer.ask(`Delete proof pack "${t}"? Its ${n} artifact${n === 1 ? '' : 's'} and snapshots are deleted too.`, { title: 'Delete proof pack' }))) {
      return;
    }
    try {
      await deleteProofPack(detail.pack.id);
      proof.closeDetail();
      toasts.success('Proof pack deleted', t);
      if (ws.currentId) await loadList(ws.currentId, filter);
      // Land on the next pack instead of an empty "pick one" pane.
      if (!viewport.isPhone && proof.packs.length > 0) void open(proof.packs[0].id);
    } catch (e) {
      toasts.error("Couldn't delete the proof pack", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- create a manual pack ------------------------------------------------
  async function newPack(): Promise<void> {
    if (!ws.currentId) {
      toasts.warn('No workspace selected');
      return;
    }
    const title = await confirmer.promptText('Title for the new manual proof pack:', {
      title: 'New proof pack',
      confirmLabel: 'Create',
      placeholder: 'e.g. Release 1.4 verification',
    });
    if (!title || !title.trim()) return;
    try {
      const created = await createProofPack(ws.currentId, {
        work_item_kind: 'manual',
        work_item_id: crypto.randomUUID(),
        title: title.trim(),
      });
      await loadList(ws.currentId, filter);
      await open(created.id);
    } catch (e) {
      toasts.error("Couldn't create the proof pack", e instanceof Error ? e.message : String(e));
    }
  }

  // ⌘K: the page's verbs (pack verbs only while a pack is open).
  $effect(() => {
    const d = detail;
    return registry.register('proof', [
      { id: 'proof.new', title: 'New proof pack…', group: 'Proof', keywords: 'create manual evidence', run: () => void newPack() },
      ...(d
        ? [
            { id: 'proof.assemble', title: 'Assemble proof pack…', group: 'Proof', keywords: 'rebuild refresh evidence', run: () => void assemble() },
            { id: 'proof.add', title: 'Add artifact to proof pack…', group: 'Proof', keywords: 'evidence attach log', run: () => { resetAdd(); addOpen = true; } },
            { id: 'proof.waive', title: 'Waive proof pack…', group: 'Proof', keywords: 'approve override gate', run: () => { waiveReason = ''; waiveOpen = true; } },
            { id: 'proof.export', title: 'Export proof report (Markdown)', group: 'Proof', keywords: 'download md report', run: () => void exportReport('md') },
          ]
        : []),
    ]);
  });
</script>

<div class="proof-page" class:phone={viewport.isPhone}>
  <PageHeader
    class="detail-head"
    title={detail ? detail.pack.title || detail.pack.work_item_id : 'Proof Packs'}
  >
    {#snippet leading()}
      {#if viewport.isPhone && detail}
        <button class="icon-btn" onclick={() => proof.closeDetail()} aria-label="Back to list" title="Back to list">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet badge()}
      {#if detail}
        <ProofStatusChip status={detail.pack.status} risk={detail.pack.risk_score} />
        <span class="kind-tag">{words(detail.pack.work_item_kind)}</span>
        {#if detail.pack.pr_number != null}
          <span class="kind-tag pr" title="Linked pull request">PR #{detail.pack.pr_number}</span>
        {/if}
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if detail}
        <!-- Destructive first (collapses first, never beside the primary). -->
        <button class="icon-btn" data-overflow="-2" data-icon="trash" data-label="Delete pack" onclick={removePack} aria-label="Delete pack" title="Delete pack"><Icon name="trash" size={14} /></button>
        <button class="btn small" data-overflow="-1" data-icon="check" onclick={() => { waiveReason = ''; waiveOpen = true; }}><Icon name="check" size={12} /> Waive</button>
        {#if detail.pack.repo_id}
          <button class="btn small" data-icon="fetch" onclick={refreshCi}><Icon name="fetch" size={12} /> Refresh CI</button>
        {/if}
        <!-- The four ways to attach evidence share one menu: four sibling
             "Add …" buttons made this the busiest header in the app. -->
        <button class="btn small" data-icon="plus" data-label="Add evidence…" onclick={openAddMenu} aria-haspopup="menu"><Icon name="plus" size={12} /> Add <Icon name="chevronDown" size={11} /></button>
        <button class="btn small primary" onclick={assemble}><Icon name="refresh" size={12} /> Assemble</button>
      {/if}
    {/snippet}
  </PageHeader>

  <div class="proof-split">
  <!-- Left: filters + pack list. Hidden on a phone while a pack is open. -->
  {#if showRail}
  <aside class="rail" class:hide-phone={viewport.isPhone && detail}>
    <div class="rail-head">
      <span class="section-title">Proof Packs</span>
      <button class="icon-btn" onclick={newPack} aria-label="New proof pack" title="New proof pack">
        <Icon name="plus" size={14} />
      </button>
    </div>
    <div class="filters" role="group" aria-label="Filter by status">
      {#each STATUS_FILTERS as f (f)}
        <button class="chip-btn" class:active={filter === f} aria-pressed={filter === f} onclick={() => (filter = f)}>
          {words(f)}{#if filterCounts && (filterCounts[f] ?? 0) > 0}<span class="chip-n">{filterCounts[f]}</span>{/if}
        </button>
      {/each}
    </div>
    <div class="rail-list">
      {#if listError && proof.packs.length === 0}
        <LoadState what="proof packs" error={listError} empty variant="compact" loading={proof.loading} onretry={retryList} />
      {/if}
      {#each proof.packs as p (p.id)}
        <button class="pack-item" class:active={detail?.pack.id === p.id} aria-current={detail?.pack.id === p.id ? 'true' : undefined} aria-busy={openingId === p.id} onclick={() => open(p.id)}>
          <div class="pack-top">
            <span class="grow ellipsis pack-title" title={p.title || p.work_item_id}>{p.title || p.work_item_id}</span>
            <span class="done-pill" title={`Done score ${p.done_score}/100`}>{p.done_score}</span>
            <ProofStatusChip status={p.status} risk={p.risk_score} />
          </div>
          <div class="pack-meta">
            <span class="kind-tag">{words(p.work_item_kind)}</span>
            <ProofBadges badges={p.badges} />
          </div>
        </button>
      {/each}
      {#if proof.packs.length === 0 && !listError}
        {#if proof.loading && !listLoaded}
          <p class="dim empty" role="status">Loading proof packs…</p>
        {:else}
          <div class="empty">
            <p class="dim">No {words(filter).toLowerCase()} proof packs.</p>
            <button class="btn small ghost" onclick={() => (filter = 'all')}>Show all</button>
          </div>
        {/if}
      {/if}
    </div>
  </aside>
  {/if}

  <!-- Right: detail. -->
  <section class="main">
    {#if !detail}
      {#if listError && !showRail}
        <LoadState what="proof packs" error={listError} empty variant="page" loading={proof.loading} onretry={retryList} />
      {:else if showRail}
        <EmptyState
          variant="page"
          icon="check"
          title={openingId ? 'Opening the proof pack…' : 'No proof pack open'}
          body="Verified evidence — tests, diffs, CI, reviews, approvals — assembled for each piece of work. Open a pack from the list to inspect its artifacts and badges."
        />
      {:else if proof.loading || !listLoaded}
        <p class="dim empty" role="status">Loading proof packs…</p>
      {:else}
        <EmptyState
          variant="page"
          icon="check"
          title="No proof packs yet"
          body="Verified evidence — tests, diffs, CI, reviews, approvals — assembled for each piece of work."
          actionLabel="New proof pack"
          actionIcon="plus"
          onaction={newPack}
        />
      {/if}
    {:else}
      <div class="detail-body">
        <!-- Done contract (R8): explainable readiness score + checklist. -->
        <DoneContractMeter contract={detail.done_contract} />

        <!-- Pack-level tools: report export (R9) + repo requirements (R3). -->
        <div class="tools-row">
          <button class="btn small ghost" onclick={() => exportReport('md')}><Icon name="download" size={12} /> Export Markdown</button>
          <button class="btn small ghost" onclick={() => exportReport('html')}><Icon name="download" size={12} /> Export HTML</button>
          {#if detail.pack.repo_id}
            <button class="btn small ghost" onclick={openConfig}><Icon name="gear" size={12} /> Requirements</button>
          {/if}
        </div>

        {#if detail.badges.length > 0}
          <div class="badges-row"><ProofBadges badges={detail.badges} /></div>
        {/if}

        {#if detail.pack.summary}
          <p class="summary">{detail.pack.summary}</p>
        {/if}

        {#if detail.pack.waived_reason}
          <p class="waived-note">
            <Icon name="info" size={12} /> Waived{detail.pack.waived_by ? ` by ${detail.pack.waived_by}` : ''}{detail.pack.waived_at ? ` · ${new Date(detail.pack.waived_at).toLocaleString()}` : ''}: {detail.pack.waived_reason}
          </p>
        {/if}

        <!-- Snapshots (R1) + per-snapshot report download (R9). -->
        <ProofSnapshotList
          packId={detail.pack.id}
          snapshots={detail.snapshots}
          onchange={() => proof.refreshDetail()}
        />

        <!-- Artifacts grouped by kind. -->
        {#if detail.artifacts.length === 0}
          <p class="dim empty">No artifacts yet. Assemble, or use Add to attach evidence.</p>
        {:else}
          {#each artifactGroups as [kind, items] (kind)}
            <section class="art-group">
              <h3 class="group-title">{words(kind)} <span class="dim">· {items.length}</span></h3>
              {#each items as a (a.id)}
                <div class="art-row">
                  <div class="art-top">
                    <span class="art-status {a.status}" aria-hidden="true"></span>
                    <span class="grow ellipsis art-title" title={a.title}>{a.title}</span>
                    {#if a.content_sha256}
                      <span class="sha-chip" title={`content sha256: ${a.content_sha256}`}>sha:{a.content_sha256.slice(0, 8)}…</span>
                    {/if}
                    <span class="art-status-label {a.status}">{words(a.status)}</span>
                    {#if a.preview != null}
                      <button class="link-btn" aria-expanded={!!expanded[a.id]} onclick={() => toggleExpand(a.id)}>
                        {expanded[a.id] ? 'Hide' : 'Show'}
                      </button>
                    {/if}
                    {#if a.preview == null || a.truncated}
                      <button class="link-btn" onclick={() => loadFull(a.id)}>Load full</button>
                    {/if}
                    <button class="icon-btn small" onclick={() => removeArtifact(a.id)} aria-label="Delete artifact {a.title}" title="Delete artifact"><Icon name="trash" size={12} /></button>
                  </div>
                  {#if MEDIA_KINDS.has(a.kind)}
                    {#if mediaUrls[a.id]}
                      {#if a.kind === 'video'}
                        <video class="art-media" controls src={mediaUrls[a.id]}><track kind="captions" /></video>
                      {:else}
                        <img class="art-media" src={mediaUrls[a.id]} alt={a.title} />
                      {/if}
                    {:else}
                      <p class="dim media-loading">Loading media…</p>
                    {/if}
                  {/if}
                  {#if a.kind === 'pr_check'}
                    {@const rep = prReport(a.metadata)}
                    {#if rep}
                      <div class="pr-report">
                        <div class="pr-report-head">
                          Consistency <strong>{rep.score}/100</strong> ·
                          <span class={rep.hard_fail ? 'bad' : rep.passed ? 'ok' : 'warn'}>
                            {rep.hard_fail ? 'inconsistent with the change' : rep.passed ? 'consistent' : 'weak — review the description'}
                          </span>
                        </div>
                        <ul class="pr-checks">
                          {#each rep.checks as c (c.label)}
                            <li class={c.passed ? 'ok' : 'miss'}>
                              <span class="tick" aria-label={c.passed ? 'Passed' : 'Missing'}><Icon name={c.passed ? 'check' : 'x'} size={12} /></span>
                              <span class="lbl">{c.label}</span>
                              <span class="dim">— {c.detail}</span>
                            </li>
                          {/each}
                        </ul>
                      </div>
                    {/if}
                  {/if}
                  {#if expanded[a.id]}
                    <pre class="art-content">{fullContent[a.id] ?? a.preview ?? ''}</pre>
                    {#if a.truncated && fullContent[a.id] == null}
                      <button class="link-btn trunc" onclick={() => loadFull(a.id)}>Truncated — load full</button>
                    {/if}
                  {/if}
                </div>
              {/each}
            </section>
          {/each}
        {/if}

        <!-- Child packs (rollup). -->
        {#if detail.children.length > 0}
          <section class="art-group">
            <h3 class="group-title">Child packs <span class="dim">· {detail.children.length}</span></h3>
            {#each detail.children as c (c.id)}
              <button class="child-link" onclick={() => open(c.id)}>
                <span class="grow ellipsis" title={c.title || c.work_item_id}>{c.title || c.work_item_id}</span>
                <span class="kind-tag">{words(c.work_item_kind)}</span>
                <ProofStatusChip status={c.status} risk={c.risk_score} />
              </button>
            {/each}
          </section>
        {/if}
      </div>
    {/if}
  </section>
  </div>
</div>

{#if addOpen && detail}
  <Modal title="Add artifact" width={520} onclose={() => (addOpen = false)}>
    <div class="field">
      <label for="a-kind">Kind</label>
      <select id="a-kind" class="input" bind:value={aKind}>
        {#each ARTIFACT_KINDS as k (k)}<option value={k}>{words(k)}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="a-title">Title</label>
      <input id="a-title" class="input" bind:value={aTitle} placeholder="e.g. cargo test output" />
    </div>
    <div class="field">
      <label for="a-status">Status</label>
      <select id="a-status" class="input" bind:value={aStatus}>
        {#each ARTIFACT_STATUSES as s (s)}<option value={s}>{words(s)}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="a-content">Content (optional)</label>
      <textarea id="a-content" class="input" rows={6} bind:value={aContent} placeholder="Paste log / command output / note"></textarea>
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (addOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={submitAdd} disabled={!aTitle.trim()} title={aTitle.trim() ? undefined : 'Give the artifact a title'}>Add artifact</button>
    {/snippet}
  </Modal>
{/if}

{#if waiveOpen && detail}
  <Modal title="Waive proof gate" width={480} onclose={() => (waiveOpen = false)}>
    <p class="modal-hint">
      Waiving records you as the approver. A reason of at least 10 characters is required.
    </p>
    <div class="field">
      <label for="w-reason">Reason</label>
      <textarea
        id="w-reason"
        class="input"
        rows={4}
        bind:value={waiveReason}
        placeholder="e.g. verified manually in staging; CI flaky on unrelated job"
      ></textarea>
      <span class="char-hint" class:short={waiveReason.trim().length < 10}>
        {waiveReason.trim().length}/10 min
      </span>
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (waiveOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={submitWaive} disabled={waiveReason.trim().length < 10}>Waive</button>
    {/snippet}
  </Modal>
{/if}

{#if mediaOpen && detail}
  <Modal title="Add media evidence" width={480} onclose={() => (mediaOpen = false)}>
    <div class="field">
      <label for="m-kind">Kind</label>
      <select id="m-kind" class="input" bind:value={mKind}>
        <option value="screenshot">Screenshot</option>
        <option value="video">Video</option>
      </select>
    </div>
    <div class="field">
      <label for="m-title">Title</label>
      <input id="m-title" class="input" bind:value={mTitle} placeholder="e.g. Dashboard after fix" />
    </div>
    <div class="field">
      <label for="m-file">File <span class="dim">(≤ 25 MiB)</span></label>
      <input
        id="m-file"
        class="input"
        type="file"
        accept={mKind === 'video' ? 'video/*' : 'image/*'}
        onchange={(e) => (mFile = e.currentTarget.files?.[0] ?? null)}
      />
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (mediaOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={submitMedia} disabled={!mFile || !mTitle.trim()} title={!mFile ? 'Choose a file' : !mTitle.trim() ? 'Give it a title' : undefined}>Attach media</button>
    {/snippet}
  </Modal>
{/if}

{#if evidenceOpen && detail}
  <Modal title="Add evidence" width={520} onclose={() => (evidenceOpen = false)}>
    <div class="field">
      <label for="e-type">Type</label>
      <select id="e-type" class="input" bind:value={eType}>
        <option value="api">API request/response</option>
        <option value="db">Database read</option>
        <option value="kafka">Kafka read</option>
      </select>
    </div>
    <div class="field">
      <label for="e-title">Title</label>
      <input id="e-title" class="input" bind:value={eTitle} placeholder="e.g. GET /health → 200" />
    </div>
    {#if eType === 'api'}
      <div class="field-row">
        <div class="field">
          <label for="e-method">Method</label>
          <select id="e-method" class="input" bind:value={eMethod}>
            {#each ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'] as m (m)}<option value={m}>{m}</option>{/each}
          </select>
        </div>
        <div class="field">
          <label for="e-status">Status</label>
          <input id="e-status" class="input" inputmode="numeric" bind:value={eStatus} placeholder="200" />
        </div>
      </div>
      <div class="field">
        <label for="e-url">URL</label>
        <input id="e-url" class="input" bind:value={eUrl} placeholder="https://api.example.com/health" />
      </div>
      <div class="field">
        <label for="e-response">Response (optional)</label>
        <textarea id="e-response" class="input" rows={4} bind:value={eResponse} placeholder="Response body / snippet"></textarea>
      </div>
    {:else if eType === 'db'}
      <div class="field-row">
        <div class="field">
          <label for="e-engine">Engine (optional)</label>
          <input id="e-engine" class="input" bind:value={eEngine} placeholder="mysql / postgres / clickhouse" />
        </div>
        <div class="field">
          <label for="e-rows">Row count (optional)</label>
          <input id="e-rows" class="input" inputmode="numeric" bind:value={eRowCount} placeholder="42" />
        </div>
      </div>
      <div class="field">
        <label for="e-query">Query (optional)</label>
        <textarea id="e-query" class="input" rows={3} bind:value={eQuery} placeholder="SELECT count(*) FROM orders"></textarea>
      </div>
      <div class="field">
        <label for="e-sample">Sample (optional)</label>
        <textarea id="e-sample" class="input" rows={4} bind:value={eSample} placeholder="Rows / result sample"></textarea>
      </div>
    {:else}
      <div class="field-row">
        <div class="field">
          <label for="e-topic">Topic</label>
          <input id="e-topic" class="input" bind:value={eTopic} placeholder="orders.events" />
        </div>
        <div class="field">
          <label for="e-msgs">Message count (optional)</label>
          <input id="e-msgs" class="input" inputmode="numeric" bind:value={eMsgCount} placeholder="100" />
        </div>
      </div>
      <div class="field">
        <label for="e-ksample">Sample (optional)</label>
        <textarea id="e-ksample" class="input" rows={4} bind:value={eSample} placeholder="Message payload sample"></textarea>
      </div>
    {/if}
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (evidenceOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={submitEvidence} disabled={!evidenceValid}>Add evidence</button>
    {/snippet}
  </Modal>
{/if}

{#if prOpen && detail}
  <Modal title="PR consistency check" width={520} onclose={() => (prOpen = false)}>
    <p class="modal-hint">
      Checks the PR description's claims against the actual change. An inconsistent claim
      (e.g. "tests pass" with a failing test artifact) is flagged.
    </p>
    <div class="field">
      <label for="pr-title">Title</label>
      <input id="pr-title" class="input" bind:value={prTitle} placeholder="e.g. PR #123 description check" />
    </div>
    <div class="field">
      <label for="pr-desc">PR description</label>
      <textarea id="pr-desc" class="input" rows={6} bind:value={prDesc} placeholder="Paste the PR description / claims"></textarea>
    </div>
    <div class="field-row">
      <div class="field">
        <label for="pr-base">Base (optional)</label>
        <input id="pr-base" class="input" bind:value={prBase} placeholder="main" />
      </div>
      <div class="field">
        <label for="pr-cwd">Working dir (optional)</label>
        <PathField bind:value={prCwd}><input id="pr-cwd" class="input" bind:value={prCwd} placeholder="/path/to/repo" /></PathField>
      </div>
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (prOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={submitPr} disabled={!prTitle.trim() || !prDesc.trim()}>Run check</button>
    {/snippet}
  </Modal>
{/if}

{#if cfgOpen && detail}
  <Modal title="Proof requirements" width={480} onclose={() => (cfgOpen = false)}>
    {#if cfgLoading}
      <p class="dim" role="status">Loading requirements…</p>
    {:else}
      <p class="modal-hint">Per-repo gates. These can only strengthen the default proof contract.</p>
      <label class="check-row">
        <input type="checkbox" bind:checked={cfg.require_test} /> Require passing tests
      </label>
      <div class="field">
        <label for="cfg-cmd">Test command (optional)</label>
        <input id="cfg-cmd" class="input" bind:value={cfg.test_cmd} placeholder="e.g. cargo test --workspace" />
      </div>
      <label class="check-row">
        <input type="checkbox" bind:checked={cfg.require_ci} /> Require passing CI
      </label>
      <label class="check-row">
        <input type="checkbox" bind:checked={cfg.require_pr_consistency} /> Require PR-description consistency
      </label>
      <label class="check-row">
        <input type="checkbox" bind:checked={cfg.require_review} /> Require resolved review
      </label>
    {/if}
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (cfgOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={saveConfig} disabled={cfgLoading}>Save requirements</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .proof-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .proof-split {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .rail {
    width: 280px;
    flex: none;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .rail-head .section-title {
    margin: 0;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .chip-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    border-radius: 999px;
    padding: 0 8px;
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .chip-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .chip-btn.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  .chip-n {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .rail-list {
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .pack-item {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 8px 10px;
    border: 1px solid transparent;
    background: transparent;
    border-radius: var(--radius-m);
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .pack-item:hover {
    background: var(--hover);
  }
  .pack-item.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .pack-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pack-title {
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .pack-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .kind-tag {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 8px;
    white-space: nowrap;
  }
  .kind-tag.pr {
    color: var(--text);
  }
  /* PR-consistency report (R7): the structured per-check breakdown a pr_check
     artifact carries in metadata.report. */
  .pr-report {
    margin: 6px 0 2px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .pr-report-head {
    font-size: var(--fs-s);
    margin-bottom: 4px;
  }
  .pr-report-head .ok {
    color: var(--success);
  }
  .pr-report-head .warn {
    color: var(--warning);
  }
  .pr-report-head .bad {
    color: var(--danger);
  }
  .pr-checks {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: var(--fs-s);
  }
  .pr-checks li {
    padding: 1px 0;
    display: flex;
    gap: 6px;
    align-items: baseline;
  }
  .pr-checks .tick {
    display: inline-flex;
    align-self: center;
    width: 12px;
    flex: none;
  }
  .pr-checks li.ok .tick {
    color: var(--success);
  }
  .pr-checks li.miss .tick {
    color: var(--danger);
  }
  .pr-checks .lbl {
    flex: none;
  }
  .empty {
    padding: 12px;
    font-size: var(--fs-s);
  }
  .empty p {
    margin: 0 0 6px;
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .detail-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 20px 32px;
  }
  .badges-row {
    margin-bottom: 10px;
  }
  .summary {
    font-size: var(--fs-s);
    line-height: 1.5;
    margin: 0 0 12px;
  }
  .waived-note {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text);
    background: var(--warning-soft);
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    border-radius: var(--radius-m);
    padding: 6px 10px;
    margin: 0 0 12px;
  }
  .art-group {
    margin-bottom: 16px;
  }
  .group-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    margin: 0 0 6px;
  }
  .art-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    margin-bottom: 6px;
    background: var(--surface);
  }
  .art-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .art-title {
    font-size: var(--fs-s);
  }
  .art-status {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .art-status.passed {
    background: var(--status-working);
  }
  .art-status.failed {
    background: var(--status-exited);
  }
  .art-status.pending {
    background: var(--status-warn);
  }
  .art-status.info {
    background: var(--info);
  }
  .art-status-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .art-status-label.passed {
    color: var(--success);
  }
  .art-status-label.failed {
    color: var(--danger);
  }
  .art-status-label.pending {
    color: var(--warning);
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    font-size: var(--fs-xs);
    padding: 0 2px;
    white-space: nowrap;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
  .art-content {
    margin: 8px 0 0;
    padding: 8px 10px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 320px;
    overflow: auto;
  }
  .child-link {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    text-align: start;
    margin-bottom: 5px;
    font-size: var(--fs-s);
  }
  .child-link:hover {
    background: var(--surface-2);
  }
  .icon-btn.small {
    width: 22px;
    height: 22px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .done-pill {
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 0 6px;
    line-height: 16px;
    flex: none;
  }
  .tools-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 12px;
  }
  .sha-chip {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 1px 6px;
    white-space: nowrap;
  }
  .art-media {
    display: block;
    margin: 8px 0 0;
    max-width: 100%;
    max-height: 360px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .media-loading {
    margin: 8px 0 0;
    font-size: var(--fs-xs);
  }
  .modal-hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
    margin: 0 0 12px;
  }
  .char-hint {
    display: block;
    margin-top: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .char-hint.short {
    color: var(--warning);
  }
  .field-row {
    display: flex;
    gap: 10px;
  }
  .field-row > .field {
    flex: 1;
    min-width: 0;
  }
  .check-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    margin-bottom: 10px;
    cursor: pointer;
  }
  .check-row input {
    accent-color: var(--accent);
  }

  @media (max-width: 640px) {
    .proof-page.phone .proof-split {
      flex-direction: column;
    }
    .proof-page.phone .rail {
      width: 100%;
      flex: 1;
      border-inline-end: none;
    }
    .proof-page.phone .rail.hide-phone {
      display: none;
    }
  }
</style>
