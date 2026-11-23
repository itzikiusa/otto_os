<script lang="ts">
  // Container Overview tab: summary metadata grid, clickable localhost port
  // links, environment variables with masked secrets + click-to-reveal,
  // mounts, networks, and collapsible labels.
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { openExternal } from '../../lib/external';
  import { rel } from '../../lib/stores/now.svelte';
  import type { ContainerDetail, ContainerSummary } from '../../lib/api/types/containers';
  import {
    containerStateInfo,
    dedupePorts,
    displayEnvEntry,
    formatPortMapping,
    portUrl,
  } from './containers-util';

  interface Props {
    container: ContainerSummary;
    detail: ContainerDetail | null;
    loading: boolean;
    error: string | null;
    revealingSecret: boolean;
    revealedKeys: ReadonlySet<string>;
    onretry: () => void;
    ontogglesecret: (key: string) => void;
    onhideallsecrets: () => void;
  }

  let {
    container,
    detail,
    loading,
    error,
    revealingSecret,
    revealedKeys,
    onretry,
    ontogglesecret,
    onhideallsecrets,
  }: Props = $props();

  let showLabels = $state(false);

  $effect(() => {
    void container.id;
    showLabels = false;
  });

  const stateBadge = $derived(containerStateInfo(detail?.state ?? container.state));
  const ports = $derived(dedupePorts(detail?.ports ?? container.ports));
  const labelEntries = $derived(Object.entries(detail?.labels ?? container.labels ?? {}));
  const cmdLine = $derived(
    detail ? [...detail.entrypoint, ...detail.cmd].join(' ') : container.command,
  );
</script>

<div class="ct-overview-scroll">
  <LoadState
    what="container details"
    {loading}
    {error}
    empty={!detail}
    {onretry}
  >
    {#if detail}
      <div class="ct-section">
        <h3 class="section-title">Summary</h3>
        <dl class="ct-kv-grid">
          <div class="ct-kv">
            <dt>Status</dt>
            <dd>{detail.status || stateBadge.label}</dd>
          </div>
          <div class="ct-kv">
            <dt>Created</dt>
            <dd title={detail.created}>{rel(detail.created)}</dd>
          </div>
          {#if detail.started_at}
            <div class="ct-kv">
              <dt>Started</dt>
              <dd title={detail.started_at}>{rel(detail.started_at)}</dd>
            </div>
          {/if}
          <div class="ct-kv">
            <dt>Restart policy</dt>
            <dd>{detail.restart_policy || 'no'}</dd>
          </div>
          <div class="ct-kv">
            <dt>PID / Exit</dt>
            <dd class="ct-mono" dir="ltr">
              {detail.running ? `PID ${detail.pid}` : `Exit ${detail.exit_code}${detail.oom_killed ? ' (OOM)' : ''}`}
            </dd>
          </div>
          {#if detail.working_dir}
            <div class="ct-kv">
              <dt>Working dir</dt>
              <dd class="ct-mono" dir="ltr" title={detail.working_dir}>{detail.working_dir}</dd>
            </div>
          {/if}
          {#if cmdLine}
            <div class="ct-kv ct-kv-wide">
              <dt>Command</dt>
              <dd class="ct-mono" dir="ltr" title={cmdLine}>{cmdLine}</dd>
            </div>
          {/if}
        </dl>
      </div>

      {#if ports.length > 0}
        <div class="ct-section">
          <h3 class="section-title">Ports</h3>
          <div class="ct-ports-list">
            {#each ports as p (`${p.public_port ?? ''}:${p.private_port}/${p.protocol}`)}
              {@const href = portUrl(p)}
              {#if href}
                <button
                  type="button"
                  class="btn small ct-port-link"
                  dir="ltr"
                  title={`Open ${href} in browser`}
                  onclick={() => void openExternal(href)}
                >
                  <Icon name="external" size={12} />
                  <span>{formatPortMapping(p)}</span>
                </button>
              {:else}
                <Badge tone="neutral" variant="outline" label={formatPortMapping(p)} />
              {/if}
            {/each}
          </div>
        </div>
      {/if}

      <div class="ct-section">
        <div class="ct-section-head">
          <h3 class="section-title">Environment ({detail.env.length})</h3>
          {#if revealedKeys.size > 0}
            <button type="button" class="btn small ghost" onclick={onhideallsecrets}>
              <Icon name="eyeOff" size={12} /> Hide revealed
            </button>
          {/if}
        </div>
        {#if detail.env.length === 0}
          <p class="ct-empty-note">No environment variables configured.</p>
        {:else}
          <div class="ct-table-wrap">
            <table class="ct-table">
              <thead>
                <tr>
                  <th>Variable</th>
                  <th>Value</th>
                  <th class="ct-col-act"><span class="sr-only">Actions</span></th>
                </tr>
              </thead>
              <tbody>
                {#each detail.env as entry (entry.key)}
                  {@const view = displayEnvEntry(entry, revealedKeys)}
                  <tr>
                    <td class="ct-mono ct-env-key" dir="ltr" title={entry.key}>
                      {entry.key}
                      {#if view.isSecret}
                        <span class="ct-secret-tag">secret</span>
                      {/if}
                    </td>
                    <td
                      class="ct-mono ct-env-val"
                      class:ct-masked={view.isMasked}
                      dir="ltr"
                      title={view.isMasked ? 'Masked secret — click eye to reveal' : view.text}
                    >
                      {view.text || '—'}
                    </td>
                    <td class="ct-col-act">
                      <div class="ct-row-acts">
                        {#if view.isSecret}
                          <button
                            type="button"
                            class="icon-btn"
                            disabled={revealingSecret}
                            aria-label={view.isMasked ? `Reveal ${entry.key}` : `Hide ${entry.key}`}
                            title={view.isMasked ? `Reveal ${entry.key}` : `Hide ${entry.key}`}
                            onclick={() => ontogglesecret(entry.key)}
                          >
                            <Icon name={view.isMasked ? 'eye' : 'eyeOff'} size={13} />
                          </button>
                        {/if}
                        <button
                          type="button"
                          class="icon-btn"
                          disabled={view.isMasked}
                          aria-label={`Copy ${entry.key}`}
                          title={view.isMasked ? 'Reveal value before copying' : `Copy ${entry.key}`}
                          onclick={() => void copyWithToast(view.text, entry.key)}
                        >
                          <Icon name="copy" size={12} />
                        </button>
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>

      {#if detail.mounts.length > 0}
        <div class="ct-section">
          <h3 class="section-title">Mounts ({detail.mounts.length})</h3>
          <div class="ct-table-wrap">
            <table class="ct-table">
              <thead>
                <tr>
                  <th>Type</th>
                  <th>Source</th>
                  <th>Target</th>
                  <th>Access</th>
                </tr>
              </thead>
              <tbody>
                {#each detail.mounts as m (`${m.source}:${m.destination}`)}
                  <tr>
                    <td>{m.mount_type}</td>
                    <td class="ct-mono" dir="ltr" title={m.source}>{m.source || '—'}</td>
                    <td class="ct-mono" dir="ltr" title={m.destination}>{m.destination}</td>
                    <td>{m.rw ? 'Read/Write' : 'Read-only'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>
      {/if}

      {#if detail.networks.length > 0}
        <div class="ct-section">
          <h3 class="section-title">Networks ({detail.networks.length})</h3>
          <div class="ct-table-wrap">
            <table class="ct-table">
              <thead>
                <tr>
                  <th>Network</th>
                  <th>IP address</th>
                  <th>Gateway</th>
                </tr>
              </thead>
              <tbody>
                {#each detail.networks as n (n.network_id || n.name)}
                  <tr>
                    <td>{n.name}</td>
                    <td class="ct-mono" dir="ltr">{n.ip_address || '—'}</td>
                    <td class="ct-mono" dir="ltr">{n.gateway || '—'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>
      {/if}

      {#if labelEntries.length > 0}
        <div class="ct-section">
          <button
            type="button"
            class="btn small ghost"
            aria-expanded={showLabels}
            onclick={() => (showLabels = !showLabels)}
          >
            <Icon name={showLabels ? 'chevronDown' : 'chevronRight'} size={12} />
            Labels ({labelEntries.length})
          </button>
          {#if showLabels}
            <div class="ct-table-wrap ct-labels-wrap">
              <table class="ct-table">
                <thead>
                  <tr>
                    <th>Key</th>
                    <th>Value</th>
                  </tr>
                </thead>
                <tbody>
                  {#each labelEntries as [k, v] (k)}
                    <tr>
                      <td class="ct-mono" dir="ltr">{k}</td>
                      <td class="ct-mono" dir="ltr">{v || '—'}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </LoadState>
</div>

<style>
  .ct-overview-scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .ct-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ct-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .ct-kv-grid {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 10px 16px;
    padding: 12px 14px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .ct-kv {
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ct-kv-wide {
    grid-column: 1 / -1;
  }
  .ct-kv dt {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ct-kv dd {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-ports-list {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .ct-port-link {
    font-family: var(--font-mono);
    color: var(--accent-text);
  }
  .ct-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .ct-labels-wrap {
    margin-block-start: 6px;
  }
  .ct-table {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .ct-table th {
    text-align: start;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding: 8px 10px;
    border-block-end: 1px solid var(--border);
    white-space: nowrap;
  }
  .ct-table td {
    padding: 6px 10px;
    border-block-end: 1px solid var(--border);
    vertical-align: middle;
    max-inline-size: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-table tbody tr:last-child td {
    border-block-end: 0;
  }
  .ct-table tbody tr:hover {
    background: var(--hover);
  }
  .ct-col-act {
    inline-size: 64px;
    text-align: end;
  }
  .ct-row-acts {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .ct-env-key {
    font-weight: 500;
  }
  .ct-secret-tag {
    margin-inline-start: 6px;
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .ct-env-val.ct-masked {
    color: var(--text-dim);
  }
  .ct-mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .ct-empty-note {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .sr-only {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
  }
</style>
