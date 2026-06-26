<script lang="ts">
  // Proof section: a two-pane viewer of proof packs. Left = status filter chips
  // + the pack list; right = the open pack's detail (badges, artifacts grouped
  // by kind, and assemble / add-artifact / waive / delete actions).
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
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
    getRepoProofConfig,
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
  import type { ProofArtifactView } from '../../lib/api/types';

  const MEDIA_KINDS = new Set(['screenshot', 'video']);

  const STATUS_FILTERS = ['all', 'passed', 'failed', 'partial', 'missing', 'waived'] as const;
  type StatusFilter = (typeof STATUS_FILTERS)[number];
  let filter = $state<StatusFilter>('all');

  const ARTIFACT_KINDS = [
    'command', 'log', 'screenshot', 'diff', 'ci', 'api', 'db', 'review', 'approval', 'self_review',
  ];
  const ARTIFACT_STATUSES = ['info', 'passed', 'failed', 'pending'];

  // Load the list (for the active filter) + the summary roll-up for this ws.
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    const f = filter;
    void proof.loadPacks(id, f === 'all' ? undefined : { status: f });
    void proof.loadSummary(id);
  });

  const detail = $derived(proof.detail);

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

  async function loadFull(id: string): Promise<void> {
    try {
      const c = await artifactContent(id);
      fullContent[id] = c.content ?? '(no content)';
      expanded[id] = true;
    } catch (e) {
      toasts.error('Load failed', e instanceof Error ? e.message : String(e));
    }
  }

  function open(id: string): void {
    void proof.open(id);
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
      toasts.error('Add artifact failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function removeArtifact(id: string): Promise<void> {
    if (!detail) return;
    if (!(await confirmer.ask('Delete this artifact?', { title: 'Delete artifact?' }))) return;
    try {
      await deleteArtifact(id);
      await proof.refreshDetail();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---- pack-level actions --------------------------------------------------
  async function assemble(): Promise<void> {
    if (!detail) return;
    const cwd = await confirmer.promptText('Working directory to assemble proof from:', {
      title: 'Assemble proof',
      confirmLabel: 'Assemble',
      placeholder: '/path/to/repo',
    });
    if (cwd === null) return;
    try {
      await assembleProof(detail.pack.id, { cwd: cwd.trim() || undefined });
      await proof.refreshDetail();
      toasts.success('Assembled', 'Proof re-assembled from the working directory.');
    } catch (e) {
      toasts.error('Assemble failed', e instanceof Error ? e.message : String(e));
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
    } catch (e) {
      toasts.error('Waive failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Add media failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Add evidence failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---- CI refresh (R2) -----------------------------------------------------
  async function refreshCi(): Promise<void> {
    if (!detail) return;
    try {
      await ciRefresh(detail.pack.id, {});
      await proof.refreshDetail();
      toasts.success('CI refreshed', 'Live CI status pulled into a ci artifact.');
    } catch (e) {
      toasts.error('CI refresh failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('PR check failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Export failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Load requirements failed', e instanceof Error ? e.message : String(e));
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
      toasts.success('Saved', 'Proof requirements updated for this repo.');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function removePack(): Promise<void> {
    if (!detail) return;
    const t = detail.pack.title || 'this pack';
    if (!(await confirmer.ask(`Delete proof pack "${t}" and all its artifacts?`, { title: 'Delete proof pack?' }))) {
      return;
    }
    try {
      await deleteProofPack(detail.pack.id);
      proof.closeDetail();
      if (ws.currentId) await proof.loadPacks(ws.currentId, filter === 'all' ? undefined : { status: filter });
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
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
      await proof.loadPacks(ws.currentId, filter === 'all' ? undefined : { status: filter });
      await proof.open(created.id);
    } catch (e) {
      toasts.error('Create failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="proof-page" class:phone={viewport.isPhone}>
  <!-- Left: filters + pack list. Hidden on a phone while a pack is open. -->
  <aside class="rail" class:hide-phone={viewport.isPhone && detail}>
    <div class="rail-head">
      <span class="section-title">Proof Packs</span>
      <button class="icon-btn" onclick={newPack} aria-label="New proof pack" title="New manual proof pack">
        <Icon name="plus" size={15} />
      </button>
    </div>
    <div class="filters">
      {#each STATUS_FILTERS as f (f)}
        <button class="chip-btn" class:active={filter === f} onclick={() => (filter = f)}>{f}</button>
      {/each}
    </div>
    <div class="rail-list">
      {#each proof.packs as p (p.id)}
        <button class="pack-item" class:active={detail?.pack.id === p.id} onclick={() => open(p.id)}>
          <div class="pack-top">
            <span class="grow ellipsis pack-title">{p.title || p.work_item_id}</span>
            <span class="done-pill" title={`Done score ${p.done_score}/100`}>{p.done_score}</span>
            <ProofStatusChip status={p.status} risk={p.risk_score} />
          </div>
          <div class="pack-meta">
            <span class="kind-tag">{p.work_item_kind}</span>
            <ProofBadges badges={p.badges} />
          </div>
        </button>
      {/each}
      {#if proof.packs.length === 0}
        <p class="dim empty">{proof.loading ? 'Loading…' : 'No proof packs match.'}</p>
      {/if}
    </div>
  </aside>

  <!-- Right: detail. -->
  <section class="main">
    {#if !detail}
      <EmptyState
        icon="check"
        title="Proof packs"
        body="Verified evidence — tests, diffs, CI, reviews, approvals — assembled for each piece of work. Select a pack to inspect its artifacts and badges."
        actionLabel="New proof pack"
        onaction={newPack}
      />
    {:else}
      <header class="page-header detail-head">
        <div class="title-wrap">
          {#if viewport.isPhone}
            <button class="back-btn" onclick={() => proof.closeDetail()} aria-label="Back to list">
              <Icon name="chevronLeft" size={16} />
            </button>
          {/if}
          <h2 class="ellipsis">{detail.pack.title || detail.pack.work_item_id}</h2>
          <ProofStatusChip status={detail.pack.status} risk={detail.pack.risk_score} />
          <span class="kind-tag">{detail.pack.work_item_kind}</span>
        </div>
        <div class="head-actions">
          <button class="btn small" onclick={assemble}><Icon name="refresh" size={12} /> Assemble</button>
          <button class="btn small" onclick={() => { resetAdd(); addOpen = true; }}><Icon name="plus" size={12} /> Add artifact</button>
          <button class="btn small" onclick={() => { resetMedia(); mediaOpen = true; }}><Icon name="file" size={12} /> Add media</button>
          <button class="btn small" onclick={() => { resetEvidence(); evidenceOpen = true; }}><Icon name="db" size={12} /> Add evidence</button>
          <button class="btn small" onclick={() => { resetPr(); prOpen = true; }}><Icon name="pr" size={12} /> PR check</button>
          {#if detail.pack.repo_id}
            <button class="btn small" onclick={refreshCi}><Icon name="fetch" size={12} /> Refresh CI</button>
          {/if}
          <button class="btn small" onclick={() => { waiveReason = ''; waiveOpen = true; }}><Icon name="check" size={12} /> Waive</button>
          <button class="icon-btn" onclick={removePack} aria-label="Delete pack"><Icon name="trash" size={14} /></button>
        </div>
      </header>

      <div class="detail-body">
        <!-- Done contract (R8): explainable readiness score + checklist. -->
        <DoneContractMeter contract={detail.done_contract} />

        <!-- Pack-level tools: report export (R9) + repo requirements (R3). -->
        <div class="tools-row">
          <button class="btn small" onclick={() => exportReport('md')}><Icon name="file" size={12} /> Export .md</button>
          <button class="btn small" onclick={() => exportReport('html')}><Icon name="external" size={12} /> Export .html</button>
          {#if detail.pack.repo_id}
            <button class="btn small" onclick={openConfig}><Icon name="gear" size={12} /> Requirements</button>
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
          <p class="dim empty">No artifacts yet — Assemble or Add artifact to attach evidence.</p>
        {:else}
          {#each artifactGroups as [kind, items] (kind)}
            <section class="art-group">
              <h3 class="group-title">{kind} <span class="dim">· {items.length}</span></h3>
              {#each items as a (a.id)}
                <div class="art-row">
                  <div class="art-top">
                    <span class="art-status {a.status}" title={a.status}></span>
                    <span class="grow ellipsis art-title">{a.title}</span>
                    {#if a.content_sha256}
                      <span class="sha-chip" title={`content sha256: ${a.content_sha256}`}>sha:{a.content_sha256.slice(0, 8)}…</span>
                    {/if}
                    <span class="art-status-label {a.status}">{a.status}</span>
                    {#if a.preview != null}
                      <button class="link-btn" onclick={() => toggleExpand(a.id)}>
                        {expanded[a.id] ? 'Hide' : 'Show'}
                      </button>
                    {/if}
                    <button class="link-btn" onclick={() => loadFull(a.id)}>Load full</button>
                    <button class="icon-btn small" onclick={() => removeArtifact(a.id)} aria-label="Delete artifact"><Icon name="trash" size={12} /></button>
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
                  {#if expanded[a.id]}
                    <pre class="art-content">{fullContent[a.id] ?? a.preview ?? ''}</pre>
                    {#if a.truncated && fullContent[a.id] == null}
                      <button class="link-btn trunc" onclick={() => loadFull(a.id)}>…truncated — Load full</button>
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
                <span class="grow ellipsis">{c.title || c.work_item_id}</span>
                <span class="kind-tag">{c.work_item_kind}</span>
                <ProofStatusChip status={c.status} risk={c.risk_score} />
              </button>
            {/each}
          </section>
        {/if}
      </div>
    {/if}
  </section>
</div>

{#if addOpen && detail}
  <Modal title="Add artifact" width={520} onclose={() => (addOpen = false)}>
    <div class="field">
      <label for="a-kind">Kind</label>
      <select id="a-kind" class="input" bind:value={aKind}>
        {#each ARTIFACT_KINDS as k (k)}<option value={k}>{k}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="a-title">Title</label>
      <input id="a-title" class="input" bind:value={aTitle} placeholder="e.g. cargo test output" />
    </div>
    <div class="field">
      <label for="a-status">Status</label>
      <select id="a-status" class="input" bind:value={aStatus}>
        {#each ARTIFACT_STATUSES as s (s)}<option value={s}>{s}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="a-content">Content (optional)</label>
      <textarea id="a-content" class="input" rows={6} bind:value={aContent} placeholder="Paste log / command output / note"></textarea>
    </div>
    {#snippet footer()}
      <button class="btn ghost" onclick={() => (addOpen = false)}>Cancel</button>
      <button class="btn primary" onclick={submitAdd} disabled={!aTitle.trim()}>Add</button>
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
        <option value="screenshot">screenshot</option>
        <option value="video">video</option>
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
      <button class="btn primary" onclick={submitMedia} disabled={!mFile || !mTitle.trim()}>Attach</button>
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
        <input id="pr-cwd" class="input" bind:value={prCwd} placeholder="/path/to/repo" />
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
      <p class="dim">Loading…</p>
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
      <button class="btn primary" onclick={saveConfig} disabled={cfgLoading}>Save</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .proof-page {
    display: flex;
    height: 100%;
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
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    border-radius: 999px;
    padding: 2px 9px;
    font-size: 11px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .chip-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .chip-btn.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
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
    border-radius: var(--radius-s);
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .pack-item:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .pack-item.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .pack-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pack-title {
    font-size: 12.5px;
    font-weight: 500;
  }
  .pack-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .kind-tag {
    font-size: 10px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border-radius: var(--radius-s);
    padding: 1px 6px;
    white-space: nowrap;
    text-transform: capitalize;
  }
  .empty {
    padding: 12px;
    font-size: 12px;
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .detail-head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .title-wrap {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }
  .title-wrap h2 {
    margin: 0;
    font-size: 16px;
    min-width: 0;
  }
  .back-btn {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    display: grid;
    place-items: center;
    flex: none;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .detail-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 16px 32px;
  }
  .badges-row {
    margin-bottom: 10px;
  }
  .summary {
    font-size: 12.5px;
    line-height: 1.5;
    margin: 0 0 12px;
  }
  .waived-note {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 10%, transparent);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    margin: 0 0 12px;
  }
  .art-group {
    margin-bottom: 16px;
  }
  .group-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin: 0 0 6px;
  }
  .art-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 7px 10px;
    margin-bottom: 5px;
    background: var(--surface);
  }
  .art-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .art-title {
    font-size: 12.5px;
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
    background: var(--accent);
  }
  .art-status-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .art-status-label.passed {
    color: var(--status-working);
  }
  .art-status-label.failed {
    color: var(--status-exited);
  }
  .art-status-label.pending {
    color: var(--status-warn);
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    font-size: 11px;
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
    font-size: 11px;
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
    font-size: 12.5px;
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
    font-size: 10px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
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
    font-size: 10px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
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
    font-size: 11px;
  }
  .modal-hint {
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.45;
    margin: 0 0 12px;
  }
  .char-hint {
    display: block;
    margin-top: 4px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .char-hint.short {
    color: var(--status-warn);
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
    font-size: 12.5px;
    margin-bottom: 10px;
    cursor: pointer;
  }
  .check-row input {
    accent-color: var(--accent);
  }

  @media (max-width: 640px) {
    .proof-page.phone {
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
