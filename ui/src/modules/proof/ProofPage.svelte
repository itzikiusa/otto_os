<script lang="ts">
  // Proof section: a two-pane viewer of proof packs. Left = status filter chips
  // + the pack list; right = the open pack's detail (badges, artifacts grouped
  // by kind, and assemble / add-artifact / waive / delete actions).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ProofBadges from '../../lib/components/ProofBadges.svelte';
  import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
  import { proof } from '../../lib/stores/proof.svelte';
  import {
    addArtifact,
    artifactContent,
    assembleProof,
    createProofPack,
    deleteArtifact,
    deleteProofPack,
    waiveProof,
  } from '../../lib/api/proof';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProofArtifactView } from '../../lib/api/types';

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

  async function waive(): Promise<void> {
    if (!detail) return;
    const reason = await confirmer.promptText('Reason for waiving this proof gate:', {
      title: 'Waive proof',
      confirmLabel: 'Waive',
      placeholder: 'e.g. covered by manual QA',
    });
    if (!reason || !reason.trim()) return;
    try {
      await waiveProof(detail.pack.id, reason.trim());
      await proof.refreshDetail();
    } catch (e) {
      toasts.error('Waive failed', e instanceof Error ? e.message : String(e));
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
          <button class="btn small" onclick={waive}><Icon name="check" size={12} /> Waive</button>
          <button class="icon-btn" onclick={removePack} aria-label="Delete pack"><Icon name="trash" size={14} /></button>
        </div>
      </header>

      <div class="detail-body">
        {#if detail.badges.length > 0}
          <div class="badges-row"><ProofBadges badges={detail.badges} /></div>
        {/if}

        {#if detail.pack.summary}
          <p class="summary">{detail.pack.summary}</p>
        {/if}

        {#if detail.pack.waived_reason}
          <p class="waived-note">
            <Icon name="info" size={12} /> Waived{detail.pack.waived_by ? ` by ${detail.pack.waived_by}` : ''}: {detail.pack.waived_reason}
          </p>
        {/if}

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
                    <span class="art-status-label {a.status}">{a.status}</span>
                    {#if a.preview != null}
                      <button class="link-btn" onclick={() => toggleExpand(a.id)}>
                        {expanded[a.id] ? 'Hide' : 'Show'}
                      </button>
                    {/if}
                    <button class="link-btn" onclick={() => loadFull(a.id)}>Load full</button>
                    <button class="icon-btn small" onclick={() => removeArtifact(a.id)} aria-label="Delete artifact"><Icon name="trash" size={12} /></button>
                  </div>
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
