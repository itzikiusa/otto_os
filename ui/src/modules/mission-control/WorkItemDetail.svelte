<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { missionControlApi } from '../../lib/api/missionControl';
  import { ApiError } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import type { WorkItemDetail, RiskLevel } from '../../lib/api/types';
  import {
    KIND_ICON,
    KIND_LABEL,
    STATUS_LABEL,
    RISK_LABEL,
    RISK_LEVELS,
    ACTOR_LABEL,
    ARTIFACT_LABEL,
    statusColor,
    riskColor,
    fmtCost,
    relTime,
  } from './lib';

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
    loading = true;
    err = '';
    try {
      detail = await missionControlApi.item(wsId, id);
      editGoal = detail.goal ?? '';
      editResult = detail.result_summary ?? '';
      editRisk = detail.risk_level;
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Failed to load item';
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
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Save failed';
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
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Request failed';
    } finally {
      busy = false;
    }
  }

  async function decide(aid: string, decision: 'approved' | 'rejected'): Promise<void> {
    busy = true;
    try {
      await missionControlApi.decideApproval(wsId, aid, { decision });
      await load();
      onChange?.();
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Decision failed';
    } finally {
      busy = false;
    }
  }

  function openSession(): void {
    if (!detail) return;
    ws.navigateToSession(detail.source_id);
    router.go('agents');
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

<aside class="detail">
  <header class="d-head">
    <div class="d-title">
      {#if detail}<span class="d-kicon"><Icon name={KIND_ICON[detail.kind]} size={16} /></span>{/if}
      <div class="d-titletext">
        <h2>{detail?.title ?? 'Work item'}</h2>
        {#if detail}<span class="dim small">{KIND_LABEL[detail.kind]} · {detail.source_id}</span>{/if}
      </div>
    </div>
    <button class="icon-btn" title="Close" aria-label="Close" onclick={onClose}><Icon name="x" size={15} /></button>
  </header>

  {#if loading && !detail}
    <div class="d-body dim">Loading…</div>
  {:else if err && !detail}
    <div class="d-body err">{err}</div>
  {:else if detail}
    <div class="d-body">
      {#if err}<div class="err small">{err}</div>{/if}

      <!-- status / risk / approval banner -->
      <div class="d-banner">
        <span class="chip-status" style="--c:{statusColor(detail.status)}">{STATUS_LABEL[detail.status]}</span>
        <span class="chip-risk" style="--c:{riskColor(detail.risk_level)}">Risk: {RISK_LABEL[detail.risk_level]}</span>
        {#if detail.needs_approval}<span class="badge-approve">Needs approval ({detail.pending_approvals})</span>{/if}
        <span class="grow"></span>
        {#if detail.kind === 'session' || detail.kind === 'external_trigger'}
          <button class="btn small" onclick={openSession}>Open session</button>
        {/if}
      </div>

      <!-- fact grid: who/what/where/cost -->
      <div class="facts">
        <div><span class="flabel">Owner</span><span>{detail.owner ?? '—'} <span class="dim">({ACTOR_LABEL[detail.owner_kind]})</span></span></div>
        <div><span class="flabel">Cost so far</span><span class="mono">{fmtCost(detail.cost_so_far)}</span></div>
        <div><span class="flabel">Repo</span><span class="mono ell">{detail.repo_id ?? '—'}</span></div>
        <div><span class="flabel">Branch</span><span class="mono">{detail.branch ?? '—'}</span></div>
        <div><span class="flabel">Created</span><span>{relTime(detail.created_at)} ago</span></div>
        <div><span class="flabel">Updated</span><span>{relTime(detail.updated_at)} ago</span></div>
      </div>

      <!-- goal / context / result + inline editor -->
      <section class="d-sec">
        <div class="sec-head">
          <h3>Goal &amp; context</h3>
          <button class="btn ghost small" onclick={() => (editing = !editing)}>{editing ? 'Cancel' : 'Edit'}</button>
        </div>
        {#if editing}
          <label class="fld"><span class="flabel">Goal</span><textarea rows="2" bind:value={editGoal}></textarea></label>
          <label class="fld"><span class="flabel">Result summary</span><textarea rows="2" bind:value={editResult}></textarea></label>
          <label class="fld">
            <span class="flabel">Risk (policy)</span>
            <select bind:value={editRisk}>
              {#each RISK_LEVELS as r (r)}<option value={r}>{RISK_LABEL[r]}</option>{/each}
            </select>
          </label>
          <button class="btn primary small" disabled={busy} onclick={saveEdits}>Save</button>
        {:else}
          <p class="goal">{detail.goal ?? '—'}</p>
          {#if detail.context_summary}<p class="ctx dim">{detail.context_summary}</p>{/if}
          {#if detail.result_summary}<div class="result"><span class="flabel">Result</span><p>{detail.result_summary}</p></div>{/if}
        {/if}
      </section>

      <!-- approvals -->
      <section class="d-sec">
        <h3>Approvals</h3>
        {#if detail.approvals.length === 0}
          <p class="dim small">No approval gates yet.</p>
        {:else}
          <ul class="approvals">
            {#each detail.approvals as a (a.id)}
              <li class="ap" class:pending={a.status === 'pending'}>
                <div class="ap-main">
                  <span class="ap-status ap-{a.status}">{a.status}</span>
                  <span class="small">{a.reason ?? 'approval requested'}</span>
                  <span class="dim small">· {a.requested_by}</span>
                </div>
                {#if a.status === 'pending'}
                  <div class="ap-actions">
                    <button class="btn small ok" disabled={busy} onclick={() => decide(a.id, 'approved')}>Approve</button>
                    <button class="btn small danger" disabled={busy} onclick={() => decide(a.id, 'rejected')}>Reject</button>
                  </div>
                {:else if a.decided_by}
                  <span class="dim small">{a.status} by {a.decided_by}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <div class="ap-req">
          <input placeholder="Reason (optional)" bind:value={approveReason} />
          <button class="btn small" disabled={busy} onclick={requestApproval}>Request approval</button>
        </div>
      </section>

      <!-- relations -->
      <section class="d-sec">
        <h3>Relations</h3>
        {#if detail.edges.length === 0}
          <p class="dim small">No linked work items.</p>
        {:else}
          <ul class="edges">
            {#each detail.edges as e (e.direction + e.peer_id + e.relation)}
              <li>
                <button class="edge-link" onclick={() => onOpen(e.peer_id)}>
                  <span class="rel">{e.direction === 'out' ? '→' : '←'} {e.relation.replace('_', ' ')}</span>
                  <span class="peer-icon"><Icon name={KIND_ICON[e.peer_kind]} size={13} /></span>
                  <span class="peer-title">{e.peer_title}</span>
                  <span class="chip-status sm" style="--c:{statusColor(e.peer_status)}">{STATUS_LABEL[e.peer_status]}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <!-- evidence / artifacts -->
      <section class="d-sec">
        <h3>Evidence</h3>
        {#if detail.artifacts.length === 0}
          <p class="dim small">No artifacts.</p>
        {:else}
          <ul class="artifacts">
            {#each detail.artifacts as a (a.id)}
              <li>
                <span class="art-kind">{ARTIFACT_LABEL[a.kind]}</span>
                <span class="art-title">{a.title}</span>
                {#if isUrl(a.ref)}<a class="art-ref" href={a.ref} target="_blank" rel="noopener">open</a>
                {:else if a.ref}<span class="art-ref mono dim">{a.ref}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <!-- timeline / audit -->
      <section class="d-sec">
        <h3>Timeline <span class="dim small">({detail.events.length})</span></h3>
        {#if detail.events.length === 0}
          <p class="dim small">No events recorded.</p>
        {:else}
          <ul class="timeline">
            {#each detail.events as ev (ev.id)}
              <li>
                <span class="tl-dot actor-{ev.actor}"></span>
                <span class="tl-type">{ev.event_type}</span>
                <span class="tl-actor dim small">{ACTOR_LABEL[ev.actor]}</span>
                <span class="tl-time dim small">{relTime(ev.ts)}</span>
                {#if payloadPreview(ev.payload)}<span class="tl-payload mono small dim">{payloadPreview(ev.payload)}</span>{/if}
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
    border-left: 1px solid var(--border);
    min-width: 0;
  }
  .d-head {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
  }
  .d-title {
    display: flex;
    gap: 9px;
    flex: 1 1 auto;
    min-width: 0;
  }
  .d-kicon {
    color: var(--accent);
    margin-top: 2px;
  }
  .d-titletext {
    min-width: 0;
  }
  .d-titletext h2 {
    margin: 0;
    font-size: 15px;
    line-height: 1.25;
    word-break: break-word;
  }
  .icon-btn {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
  }
  .icon-btn:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .d-body {
    flex: 1 1 auto;
    overflow: auto;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
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
  }
  .facts > div {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .flabel {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .ell,
  .facts > div span:last-child {
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
  .d-sec h3 {
    margin: 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .goal {
    margin: 0;
    font-size: 13px;
  }
  .ctx {
    margin: 0;
    font-size: 12px;
  }
  .result {
    background: var(--bg);
    border-radius: 6px;
    padding: 7px 9px;
  }
  .result p {
    margin: 2px 0 0;
    font-size: 12.5px;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  textarea,
  select,
  input {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    padding: 6px 8px;
    width: 100%;
  }
  .chip-status,
  .chip-risk {
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
    color: var(--c);
    border: 1px solid color-mix(in srgb, var(--c) 45%, transparent);
    background: color-mix(in srgb, var(--c) 14%, transparent);
    white-space: nowrap;
  }
  .chip-status.sm {
    font-size: 9.5px;
    padding: 1px 6px;
  }
  .badge-approve {
    font-size: 10.5px;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 999px;
    background: #ffd33d;
    color: #3a2c00;
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
    gap: 5px;
  }
  .ap {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--bg);
    border-radius: 6px;
    padding: 6px 9px;
  }
  .ap.pending {
    border: 1px solid #ffd33d66;
  }
  .ap-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .ap-status {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
  }
  .ap-pending {
    color: #d6a800;
  }
  .ap-approved {
    color: #2ea043;
  }
  .ap-rejected {
    color: #ff5f57;
  }
  .ap-actions {
    display: flex;
    gap: 5px;
    flex: 0 0 auto;
  }
  .ap-req {
    display: flex;
    gap: 6px;
  }
  .ap-req input {
    flex: 1 1 auto;
  }
  .edge-link {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    text-align: left;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 9px;
    color: var(--text);
    cursor: pointer;
  }
  .edge-link:hover {
    border-color: var(--accent);
  }
  .rel {
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .peer-icon {
    color: var(--text-dim);
    display: inline-flex;
  }
  .peer-title {
    flex: 1 1 auto;
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .artifacts li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .art-kind {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--accent);
    flex: 0 0 auto;
  }
  .art-title {
    flex: 1 1 auto;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .art-ref {
    flex: 0 0 auto;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .timeline li {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
  }
  .tl-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--text-dim);
  }
  .tl-dot.actor-user {
    background: #7ee787;
  }
  .tl-dot.actor-agent {
    background: var(--accent);
  }
  .tl-dot.actor-integration {
    background: #ffd33d;
  }
  .tl-type {
    font-weight: 600;
  }
  .tl-payload {
    flex: 1 1 auto;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .small {
    font-size: 11.5px;
  }
  .err {
    color: #ff5f57;
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
