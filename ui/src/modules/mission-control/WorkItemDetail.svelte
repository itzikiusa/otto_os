<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { missionControlBus } from '../../lib/events.svelte';
  import { missionControlApi } from '../../lib/api/missionControl';
  import { ApiError } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { WorkItemDetail, RiskLevel } from '../../lib/api/types';
  import {
    KIND_ICON,
    KIND_LABEL,
    RISK_LABEL,
    RISK_LEVELS,
    ACTOR_LABEL,
    ARTIFACT_LABEL,
    riskColor,
    fmtCost,
    relTime,
    workStatus,
    ownerLabel,
    shortId,
    isUlid,
  } from './lib';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { now } from '../../lib/stores/now.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { openExternal } from '../../lib/external';
  import { copyText } from '../../lib/clipboard';
  import { sentenceCase, type Tone } from '../../lib/status';

  interface Props {
    wsId: string;
    id: string;
    onClose: () => void;
    onOpen: (id: string) => void;
    /** Fired after a mutation so the parent can refresh its summary/list. */
    onChange?: () => void;
  }
  let { wsId, id, onClose, onOpen, onChange }: Props = $props();

  let detail = $state<WorkItemDetail | null>(null);
  let loading = $state(false);
  let err = $state('');
  let busy = $state(false);

  // editing
  let editing = $state(false);
  let editGoal = $state('');
  let editResult = $state('');
  let editRisk = $state<RiskLevel>('low');
  let approveReason = $state('');

  async function load(): Promise<void> {
    // Another item: never show the previous item's facts (or its half-typed
    // edit) under the new title while it loads.
    if (detail && detail.id !== id) {
      detail = null;
      editing = false;
    }
    const want = id;
    loading = true;
    err = '';
    try {
      const d = await missionControlApi.item(wsId, want);
      if (want !== id) return;
      detail = d;
      editGoal = detail.goal ?? '';
      editResult = detail.result_summary ?? '';
      editRisk = detail.risk_level;
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Otto couldn’t reach the daemon.';
    } finally {
      loading = false;
    }
  }

  // Reload whenever the selected id changes.
  $effect(() => {
    void id;
    void wsId;
    void load();
  });

  // Live: reload when THIS item changes (or on a reconnect resync) — the
  // pane used to keep showing a stale status while the list updated. Never
  // while the user is editing (a reload resets the edit fields).
  let seenTick = untrack(() => missionControlBus.tick);
  $effect(() => {
    const tick = missionControlBus.tick;
    const evItem = missionControlBus.itemId;
    const evWs = missionControlBus.workspaceId;
    if (tick === seenTick) return;
    seenTick = tick;
    const resync = evWs === '' && evItem === '';
    const mine = untrack(() => evItem === id && evWs === wsId);
    if ((resync || mine) && !untrack(() => editing)) void load();
  });

  async function saveEdits(): Promise<void> {
    if (!detail) return;
    busy = true;
    try {
      await missionControlApi.patch(wsId, id, {
        risk_level: editRisk,
        goal: editGoal,
        result_summary: editResult,
      });
      editing = false;
      await load();
      onChange?.();
      toasts.success('Work item saved');
    } catch (e) {
      toasts.error("Couldn't save the work item", e instanceof ApiError ? e.message : 'Otto couldn’t reach the daemon.');
    } finally {
      busy = false;
    }
  }

  async function requestApproval(): Promise<void> {
    busy = true;
    try {
      await missionControlApi.requestApproval(wsId, id, { reason: approveReason || undefined });
      approveReason = '';
      await load();
      onChange?.();
      toasts.success('Approval requested');
    } catch (e) {
      toasts.error("Couldn't request approval", e instanceof ApiError ? e.message : 'Otto couldn’t reach the daemon.');
    } finally {
      busy = false;
    }
  }

  async function decide(aid: string, decision: 'approved' | 'rejected'): Promise<void> {
    // Reject asks why (optional) — the note is recorded with the decision.
    let note: string | undefined;
    if (decision === 'rejected') {
      const r = await confirmer.promptText('Why are you rejecting this? (optional — recorded with the decision)', {
        title: 'Reject approval',
        confirmLabel: 'Reject',
        placeholder: 'Reason',
        danger: true,
      });
      if (r === null) return;
      note = r.trim() || undefined;
    }
    busy = true;
    try {
      await missionControlApi.decideApproval(wsId, aid, { decision, note });
      await load();
      onChange?.();
      toasts.success(decision === 'approved' ? 'Approved' : 'Rejected');
    } catch (e) {
      toasts.error("Couldn't record the decision", e instanceof ApiError ? e.message : 'Otto couldn’t reach the daemon.');
    } finally {
      busy = false;
    }
  }

  // navigateToSession routes to `agents/<id>`; a following router.go('agents')
  // used to overwrite that and land on Agents without the session.
  function openSession(): void {
    if (!detail) return;
    ws.navigateToSession(detail.source_id);
  }

  /** A person's label: "You", a name, or a shortened id (full id in title). */
  function who(v: string | null | undefined): string {
    if (!v) return '—';
    return ownerLabel(v, auth.me?.id) ?? shortId(v);
  }

  const APPROVAL_TONE: Record<string, Tone> = { pending: 'warning', approved: 'success', rejected: 'danger' };

  function relLabel(rel: string): string {
    return sentenceCase(rel);
  }

  /** "3m ago" that keeps counting (shared clock); "—" when unknown. */
  function ago(iso: string | null | undefined): string {
    void now();
    const r = relTime(iso);
    return r === '—' ? r : `${r} ago`;
  }
  function localTime(iso: string | null | undefined): string | undefined {
    const t = iso ? Date.parse(iso) : NaN;
    return Number.isFinite(t) ? new Date(t).toLocaleString() : undefined;
  }

  async function copyId(v: string): Promise<void> {
    if (await copyText(v)) toasts.success('Copied ID');
  }

  function isUrl(s: string | null): boolean {
    return !!s && /^https?:\/\//.test(s);
  }
  function payloadPreview(p: unknown): string {
    if (p == null) return '';
    try {
      const s = typeof p === 'string' ? p : JSON.stringify(p);
      return s.length > 140 ? s.slice(0, 139) + '…' : s;
    } catch {
      return '';
    }
  }
</script>

<aside class="detail" aria-label="Work item details">
  <header class="d-head">
    <div class="d-title">
      {#if detail}<span class="d-kicon"><Icon name={KIND_ICON[detail.kind]} size={16} /></span>{/if}
      <div class="d-titletext">
        <h2 title={detail?.title}>{detail?.title ?? 'Work item'}</h2>
        {#if detail}
          <span class="d-sub">
            {KIND_LABEL[detail.kind]} ·
            <button class="id-btn mono" title="{detail.source_id} — click to copy" onclick={() => detail && void copyId(detail.source_id)}>{isUlid(detail.source_id) ? shortId(detail.source_id) : detail.source_id}</button>
          </span>
        {/if}
      </div>
    </div>
    <button class="icon-btn" title="Close details" aria-label="Close details" onclick={onClose}><Icon name="x" size={14} /></button>
  </header>

  {#if loading && !detail}
    <div class="d-body" aria-busy="true"><Skeleton rows={6} /></div>
  {:else if err && !detail}
    <div class="d-body">
      <div class="d-err" role="alert">
        <Icon name="warning" size={14} />
        <span>Couldn't load this work item. {err}</span>
      </div>
      <div><button class="btn small" onclick={() => void load()}><Icon name="refresh" size={12} />Retry</button></div>
    </div>
  {:else if detail}
    <div class="d-body">
      {#if err}<div class="d-err" role="alert"><Icon name="warning" size={14} /><span>{err}</span></div>{/if}

      <!-- status / risk / approval banner -->
      <div class="d-banner">
        <StatusBadge status={workStatus(detail.status)} />
        <span class="chip-risk" style="--c:{riskColor(detail.risk_level)}">{RISK_LABEL[detail.risk_level]} risk</span>
        {#if detail.needs_approval}<span class="badge-approve">Needs approval ({detail.pending_approvals})</span>{/if}
        <span class="grow"></span>
        {#if detail.kind === 'session' || detail.kind === 'external_trigger'}
          <button class="btn small" onclick={openSession}><Icon name="terminal" size={12} />Open session</button>
        {/if}
      </div>

      <!-- fact grid: who/what/where/cost -->
      <dl class="facts">
        <div><dt>Owner</dt><dd title={detail.owner ?? undefined}>{who(detail.owner)} <span class="dim">({ACTOR_LABEL[detail.owner_kind]})</span></dd></div>
        <div><dt>Cost so far</dt><dd class="mono">{fmtCost(detail.cost_so_far)}</dd></div>
        <div><dt>Repo</dt><dd class="mono" title={detail.repo_id ?? undefined}>{detail.repo_id ?? '—'}</dd></div>
        <div><dt>Branch</dt><dd class="mono" title={detail.branch ?? undefined}>{detail.branch ?? '—'}</dd></div>
        <div><dt>Created</dt><dd title={localTime(detail.created_at)}>{ago(detail.created_at)}</dd></div>
        <div><dt>Updated</dt><dd title={localTime(detail.updated_at)}>{ago(detail.updated_at)}</dd></div>
      </dl>

      <!-- goal / context / result + inline editor -->
      <section class="d-sec">
        <div class="sec-head">
          <h3 class="section-title">Goal &amp; context</h3>
          {#if !editing}<button class="btn ghost small" onclick={() => (editing = true)}><Icon name="edit" size={12} />Edit</button>{/if}
        </div>
        {#if editing}
          <label class="fld"><span class="flabel">Goal</span><textarea class="input fld-in" rows="2" bind:value={editGoal}></textarea></label>
          <label class="fld"><span class="flabel">Result summary</span><textarea class="input fld-in" rows="2" bind:value={editResult}></textarea></label>
          <label class="fld">
            <span class="flabel">Risk (policy)</span>
            <select class="input fld-in" bind:value={editRisk}>
              {#each RISK_LEVELS as r (r)}<option value={r}>{RISK_LABEL[r]}</option>{/each}
            </select>
          </label>
          <div class="edit-actions">
            <button class="btn small" disabled={busy} onclick={() => { editing = false; editGoal = detail?.goal ?? ''; editResult = detail?.result_summary ?? ''; editRisk = detail?.risk_level ?? 'low'; }}>Cancel</button>
            <button class="btn primary small" disabled={busy} onclick={saveEdits}>{busy ? 'Saving…' : 'Save'}</button>
          </div>
        {:else}
          <p class="goal" class:dim={!detail.goal}>{detail.goal ?? 'No goal recorded.'}</p>
          {#if detail.context_summary}<p class="ctx dim">{detail.context_summary}</p>{/if}
          {#if detail.result_summary}<div class="result"><span class="flabel">Result</span><p>{detail.result_summary}</p></div>{/if}
        {/if}
      </section>

      <!-- approvals -->
      <section class="d-sec">
        <h3 class="section-title">Approvals</h3>
        {#if detail.approvals.length === 0}
          <p class="dim small">No approval gates yet.</p>
        {:else}
          <ul class="approvals">
            {#each detail.approvals as a (a.id)}
              <li class="ap" class:pending={a.status === 'pending'}>
                <div class="ap-main">
                  <StatusBadge tone={APPROVAL_TONE[a.status] ?? 'neutral'} label={sentenceCase(a.status)} />
                  <span class="small ap-reason" title={a.reason ?? undefined}>{a.reason ?? 'Approval requested'}</span>
                  <span class="dim small" title={a.requested_by}>· {who(a.requested_by)}</span>
                </div>
                {#if a.status === 'pending'}
                  <div class="ap-actions">
                    <button class="btn small danger" disabled={busy} onclick={() => decide(a.id, 'rejected')}>Reject…</button>
                    <button class="btn small primary" disabled={busy} onclick={() => decide(a.id, 'approved')}>Approve</button>
                  </div>
                {:else if a.decided_by}
                  <span class="dim small" title={a.decided_by}>{sentenceCase(a.status)} by {who(a.decided_by)}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <div class="ap-req">
          <input class="input req-in" placeholder="Reason (optional)" aria-label="Approval reason (optional)" bind:value={approveReason} onkeydown={(e) => e.key === 'Enter' && !busy && void requestApproval()} />
          <button class="btn small" disabled={busy} onclick={requestApproval}>Request approval</button>
        </div>
      </section>

      <!-- relations -->
      <section class="d-sec">
        <h3 class="section-title">Relations</h3>
        {#if detail.edges.length === 0}
          <p class="dim small">No linked work items.</p>
        {:else}
          <ul class="edges">
            {#each detail.edges as e (e.direction + e.peer_id + e.relation)}
              <li>
                <button class="edge-link" onclick={() => onOpen(e.peer_id)} title="Open {e.peer_title}">
                  <span class="rel" title={e.direction === 'out' ? 'Outgoing' : 'Incoming'}>
                    <span class="rel-dir"><Icon name={e.direction === 'out' ? 'chevronRight' : 'chevronLeft'} size={12} /></span>{relLabel(e.relation)}
                  </span>
                  <span class="peer-icon"><Icon name={KIND_ICON[e.peer_kind]} size={12} /></span>
                  <span class="peer-title">{e.peer_title}</span>
                  <StatusBadge status={workStatus(e.peer_status)} variant="text" />
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <!-- evidence / artifacts -->
      <section class="d-sec">
        <h3 class="section-title">Evidence</h3>
        {#if detail.artifacts.length === 0}
          <p class="dim small">No artifacts.</p>
        {:else}
          <ul class="artifacts">
            {#each detail.artifacts as a (a.id)}
              <li>
                <span class="chip art-kind">{ARTIFACT_LABEL[a.kind]}</span>
                <span class="art-title" title={a.title}>{a.title}</span>
                {#if isUrl(a.ref)}
                  <button class="btn ghost small" onclick={() => void openExternal(a.ref)} title={a.ref ?? undefined}><Icon name="external" size={12} />Open</button>
                {:else if a.ref}<span class="art-ref mono dim" title={a.ref}>{a.ref}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <!-- timeline / audit -->
      <section class="d-sec">
        <h3 class="section-title">Timeline <span class="count">({detail.events.length})</span></h3>
        {#if detail.events.length === 0}
          <p class="dim small">No events recorded.</p>
        {:else}
          <ul class="timeline">
            {#each detail.events as ev (ev.id)}
              <li>
                <span class="tl-dot actor-{ev.actor}" aria-hidden="true"></span>
                <span class="tl-type">{sentenceCase(ev.event_type)}</span>
                <span class="tl-actor dim small">{ACTOR_LABEL[ev.actor]}</span>
                <span class="tl-time dim small" title={localTime(ev.ts)}>{ago(ev.ts)}</span>
                {#if payloadPreview(ev.payload)}<span class="tl-payload mono small dim" title={payloadPreview(ev.payload)}>{payloadPreview(ev.payload)}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </div>
  {/if}
</aside>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--surface);
    min-width: 0;
  }
  .d-head {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 12px 10px 12px 14px;
    border-bottom: 1px solid var(--border);
  }
  .d-title {
    display: flex;
    gap: 8px;
    flex: 1 1 auto;
    min-width: 0;
  }
  .d-kicon {
    display: inline-flex;
    color: var(--text-dim);
    margin-top: 2px;
  }
  .d-titletext {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .d-titletext h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }
  .d-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .id-btn {
    padding: 0;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: copy;
  }
  .id-btn:hover {
    color: var(--text);
    text-decoration: underline;
  }
  .d-body {
    flex: 1 1 auto;
    overflow: auto;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .d-err {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .d-err span {
    color: var(--text);
  }
  .d-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .facts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 14px;
    margin: 0;
  }
  .facts > div {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .facts dt,
  .flabel {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    font-size: var(--fs-m);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .d-sec {
    border-top: 1px solid var(--border);
    padding-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .d-sec .section-title {
    margin: 0;
  }
  .count {
    font-weight: 400;
    letter-spacing: 0;
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 24px;
  }
  .goal {
    margin: 0;
    font-size: var(--fs-m);
  }
  .ctx {
    margin: 0;
    font-size: var(--fs-s);
  }
  .result {
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 8px 10px;
  }
  .result p {
    margin: 2px 0 0;
    font-size: var(--fs-s);
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .fld-in {
    width: 100%;
  }
  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .chip-risk {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 8px;
    border-radius: 999px;
    color: var(--c);
    border: 1px solid color-mix(in srgb, var(--c) 45%, transparent);
    background: color-mix(in srgb, var(--c) 14%, transparent);
    white-space: nowrap;
  }
  .badge-approve {
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--warning-soft);
    color: var(--warning);
    border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
  }
  .approvals,
  .edges,
  .artifacts,
  .timeline {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ap {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    background: var(--surface-2);
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    padding: 6px 8px;
  }
  .ap.pending {
    border-color: color-mix(in srgb, var(--warning) 40%, transparent);
  }
  .ap-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .ap-reason {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ap-actions {
    display: flex;
    gap: 6px;
    flex: 0 0 auto;
  }
  .ap-req {
    display: flex;
    gap: 6px;
  }
  .req-in {
    flex: 1 1 auto;
    min-width: 0;
  }
  .edge-link {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: start;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .edge-link:hover {
    border-color: var(--border-strong);
    background: var(--hover);
  }
  .rel {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    flex: none;
  }
  .rel-dir {
    display: inline-flex;
  }
  :global([dir='rtl']) .rel-dir {
    transform: scaleX(-1);
  }
  .peer-icon {
    color: var(--text-dim);
    display: inline-flex;
    flex: none;
  }
  .peer-title {
    flex: 1 1 auto;
    min-width: 0;
    font-size: var(--fs-s);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .artifacts li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    min-height: 24px;
  }
  .art-kind {
    flex: none;
  }
  .art-title {
    flex: 1 1 auto;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .art-ref {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .timeline li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .tl-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--text-dim);
  }
  .tl-dot.actor-user {
    background: var(--status-working);
  }
  .tl-dot.actor-agent {
    background: var(--accent);
  }
  .tl-dot.actor-integration {
    background: var(--status-warn);
  }
  .tl-type {
    font-weight: 500;
    flex: none;
  }
  .tl-actor,
  .tl-time {
    flex: none;
  }
  .tl-payload {
    flex: 1 1 auto;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .small {
    font-size: var(--fs-xs);
  }
  .grow {
    flex: 1 1 auto;
  }
  @media (max-width: 640px) {
    .facts {
      grid-template-columns: 1fr;
    }
  }
</style>
