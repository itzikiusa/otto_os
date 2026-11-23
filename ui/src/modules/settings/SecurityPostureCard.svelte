<script lang="ts">
  // Detailed Security Posture card (`E-12`): displays the overall verdict,
  // categorized diagnostic checks (sorted fail -> warn -> pass), inline/expandable
  // remediation guidance, relative `checked_at` timestamp, and a Refresh control.
  import type { SecurityPostureCheck, SecurityPostureResp } from '../../lib/api/types';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import {
    checkStatusInfo,
    groupChecksByCategory,
    hasRemediation,
    overallPostureInfo,
    postureSummaryLine,
    summarizeChecks,
  } from './securityPosture';

  interface Props {
    posture: SecurityPostureResp | null;
    loading?: boolean;
    error?: string;
    onrefresh: () => void;
  }

  let { posture, loading = false, error = '', onrefresh }: Props = $props();

  const checks = $derived(posture?.checks ?? []);
  const hasChecksField = $derived(Array.isArray(posture?.checks));
  const counts = $derived(summarizeChecks(checks));
  const groups = $derived(groupChecksByCategory(checks));
  const overall = $derived(overallPostureInfo(posture?.overall, checks));
  const summaryText = $derived(postureSummaryLine(counts));

  // Track user toggles per check id so failing/warning checks open their
  // remediation by default while still allowing the user to collapse or expand.
  let toggledRemediations: Record<string, boolean> = $state({});

  function isRemediationOpen(check: SecurityPostureCheck): boolean {
    if (Object.hasOwn(toggledRemediations, check.id)) {
      return toggledRemediations[check.id];
    }
    return check.status !== 'pass';
  }

  function toggleRemediation(id: string, currentlyOpen: boolean): void {
    toggledRemediations = { ...toggledRemediations, [id]: !currentlyOpen };
  }

  function fullTimestamp(ts: string | undefined): string {
    if (!ts) return '';
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
  }
</script>

<section class="sp-card" aria-label="Security posture checks">
  <header class="sp-header">
    <div class="sp-title-wrap">
      <div class="sp-heading-row">
        <span class="sp-icon" aria-hidden="true">
          <Icon name="shield" size={16} />
        </span>
        <h2 class="sp-title">Security posture</h2>
        {#if posture && hasChecksField}
          <StatusBadge status={overall} />
        {/if}
      </div>
      <div class="sp-meta">
        {#if posture && hasChecksField}
          <span>{summaryText}</span>
          {#if posture.checked_at}
            <span class="sp-sep" aria-hidden="true">·</span>
            <time datetime={posture.checked_at} title={fullTimestamp(posture.checked_at)}>
              Checked {rel(posture.checked_at)}
            </time>
          {/if}
        {:else if !loading && !error}
          <span>Automated security checks for this daemon</span>
        {/if}
      </div>
    </div>

    <div class="sp-header-actions">
      {#if posture && hasChecksField && counts.total > 0}
        <div class="sp-counts" aria-label="Check counts">
          {#if counts.fail > 0}
            <Badge tone="bad" label={`${counts.fail} fail`} />
          {/if}
          {#if counts.warn > 0}
            <Badge tone="warn" label={`${counts.warn} warn`} />
          {/if}
          {#if counts.pass > 0}
            <Badge tone="ok" label={`${counts.pass} pass`} />
          {/if}
        </div>
      {/if}
      <button
        type="button"
        class="btn small"
        disabled={loading}
        onclick={onrefresh}
      >
        <Icon name="refresh" size={12} />
        {loading ? 'Refreshing…' : 'Refresh'}
      </button>
    </div>
  </header>

  <div class="sp-body">
    <LoadState
      what="security posture checks"
      {loading}
      error={error || null}
      empty={!posture || checks.length === 0}
      variant="panel"
      rows={3}
      onretry={onrefresh}
    >
      {#snippet emptyView()}
        <EmptyState
          icon="shield"
          title={hasChecksField ? 'No security checks reported' : 'Detailed checks unavailable'}
          body={hasChecksField
            ? 'The daemon returned an empty check list. Refresh to run the posture checks again.'
            : 'Update the daemon to view detailed secrets, filesystem, network, access, plugin, and database checks.'}
        />
      {/snippet}

      <div class="sp-groups">
        {#each groups as group (group.category)}
          <div class="sp-group">
            <div class="sp-group-head">
              <h3 class="sp-group-title">{group.label}</h3>
              <Badge
                tone={checkStatusInfo(group.worstStatus).badgeTone}
                variant="outline"
                label={`${group.counts.pass}/${group.counts.total} passed`}
              />
            </div>

            <ul class="sp-list">
              {#each group.checks as check (check.id)}
                {@const info = checkStatusInfo(check.status)}
                {@const remediable = hasRemediation(check)}
                {@const open = remediable && isRemediationOpen(check)}
                <li class="sp-item sp-status-{check.status}">
                  <div class="sp-item-main">
                    <div class="sp-item-head">
                      <StatusBadge tone={info.tone} label={info.label} variant="pill" />
                      <span class="sp-item-title">{check.title}</span>
                    </div>
                    <p class="sp-item-detail">{check.detail}</p>
                  </div>

                  {#if remediable}
                    <div class="sp-remediation-wrap">
                      <button
                        type="button"
                        class="sp-remediation-toggle"
                        aria-expanded={open}
                        onclick={() => toggleRemediation(check.id, open)}
                      >
                        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
                        <span>{open ? 'Hide remediation' : 'How to fix'}</span>
                      </button>
                      {#if open}
                        <div class="sp-remediation-box">
                          <span class="sp-remediation-label">Remediation</span>
                          <p class="sp-remediation-text">{check.remediation}</p>
                        </div>
                      {/if}
                    </div>
                  {/if}
                </li>
              {/each}
            </ul>
          </div>
        {/each}
      </div>
    </LoadState>
  </div>
</section>

<style>
  .sp-card {
    margin-block-start: 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }
  .sp-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-block: 12px;
    padding-inline: 16px;
    border-block-end: 1px solid var(--border);
  }
  .sp-title-wrap {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .sp-heading-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .sp-icon {
    display: inline-flex;
    align-items: center;
    color: var(--text-dim);
  }
  .sp-title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .sp-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sp-sep {
    color: var(--text-dim);
  }
  .sp-header-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .sp-counts {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .sp-body {
    padding-block: 12px;
    padding-inline: 16px;
  }
  .sp-groups {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .sp-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sp-group-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .sp-group-title {
    margin: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .sp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sp-item {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-block: 10px;
    padding-inline: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .sp-status-fail {
    border-inline-start: 2px solid var(--danger);
  }
  .sp-status-warn {
    border-inline-start: 2px solid var(--warning);
  }
  .sp-status-pass {
    border-inline-start: 2px solid var(--success);
  }
  .sp-item-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .sp-item-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .sp-item-title {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
  }
  .sp-item-detail {
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .sp-remediation-wrap {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }
  .sp-remediation-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .sp-remediation-toggle:hover {
    text-decoration: underline;
  }
  .sp-remediation-box {
    inline-size: 100%;
    box-sizing: border-box;
    padding-block: 8px;
    padding-inline: 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sp-remediation-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .sp-remediation-text {
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text);
    overflow-wrap: anywhere;
  }
</style>
