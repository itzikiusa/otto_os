<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import type { WorkItem } from '../../lib/api/types';
  import {
    KIND_ICON,
    KIND_LABEL,
    STATUS_LABEL,
    RISK_LABEL,
    statusColor,
    riskColor,
    fmtCost,
    relTime,
  } from './lib';

  interface Props {
    items: WorkItem[];
    needsApproval: Set<string>;
    selectedId: string | null;
    onOpen: (id: string) => void;
  }
  let { items, needsApproval, selectedId, onOpen }: Props = $props();

  function shortRepo(p: string | null): string {
    if (!p) return '';
    const parts = p.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || p;
  }
</script>

<div class="wi-list" aria-label="Work items">
  {#each items as it (it.id)}
    <button
      class="wi-row"
      class:active={it.id === selectedId}
      onclick={() => onOpen(it.id)}
    >
      <span class="wi-icon" title={KIND_LABEL[it.kind]}><Icon name={KIND_ICON[it.kind]} size={15} /></span>
      <span class="wi-main">
        <span class="wi-title">{it.title}</span>
        <span class="wi-sub">
          <span class="wi-kindlabel">{KIND_LABEL[it.kind]}</span>
          {#if it.repo_id}<span class="wi-dot">·</span><span class="mono">{shortRepo(it.repo_id)}</span>{/if}
          {#if it.owner}<span class="wi-dot">·</span><span>{it.owner}</span>{/if}
        </span>
      </span>
      <span class="wi-meta">
        {#if needsApproval.has(it.id)}
          <span class="badge-approve" title="Needs human approval">Needs approval</span>
        {/if}
        <span class="chip-status" style="--c:{statusColor(it.status)}">{STATUS_LABEL[it.status]}</span>
        <span class="chip-risk" style="--c:{riskColor(it.risk_level)}" title="Risk / policy">{RISK_LABEL[it.risk_level]}</span>
        <span class="wi-cost mono" title="Cost so far">{fmtCost(it.cost_so_far)}</span>
        <span class="wi-time dim" title={it.last_event_at ?? it.updated_at}>{relTime(it.last_event_at ?? it.updated_at)}</span>
      </span>
    </button>
  {/each}
</div>

<style>
  .wi-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .wi-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    text-align: left;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 9px 12px;
    color: var(--text);
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s;
  }
  .wi-row:hover {
    border-color: var(--accent);
  }
  .wi-row.active {
    background: #7ee787;
    color: #0a0a0a;
    border-color: #2ea043;
  }
  .wi-row.active .wi-sub,
  .wi-row.active .wi-time {
    color: #0a3d12;
  }
  .wi-icon {
    flex: 0 0 auto;
    display: inline-flex;
    color: var(--text-dim);
  }
  .wi-row.active .wi-icon {
    color: #0a0a0a;
  }
  .wi-main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .wi-title {
    font-weight: 600;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wi-sub {
    font-size: 11px;
    color: var(--text-dim);
    display: flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wi-dot {
    opacity: 0.5;
  }
  .wi-meta {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .chip-status,
  .chip-risk {
    font-size: 10.5px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 999px;
    white-space: nowrap;
    color: var(--c);
    border: 1px solid color-mix(in srgb, var(--c) 45%, transparent);
    background: color-mix(in srgb, var(--c) 14%, transparent);
  }
  .chip-risk {
    opacity: 0.92;
  }
  .badge-approve {
    font-size: 10.5px;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 999px;
    background: #ffd33d;
    color: #3a2c00;
    white-space: nowrap;
  }
  .wi-cost {
    font-size: 11.5px;
    color: var(--text-dim);
    min-width: 48px;
    text-align: right;
  }
  .wi-row.active .wi-cost {
    color: #0a3d12;
  }
  .wi-time {
    font-size: 11px;
    min-width: 28px;
    text-align: right;
  }
  @media (max-width: 640px) {
    .wi-cost,
    .wi-time {
      display: none;
    }
    .wi-meta {
      gap: 5px;
    }
  }
</style>
