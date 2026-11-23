<script lang="ts">
  // Workbench toolbar extracted from DatabasePage.svelte: segmented view
  // switcher (Query / Builder / Structure / Diagram / Dashboards), guardrail
  // badge, connection health status, and the ⋯ actions menu (Test, Copy as CLI,
  // Database changes).
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import { database, type DbMainTab } from '../../lib/stores/database.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { onTabKey } from '../../lib/tabKeys';
  import { buildDbCliSnippet, cliLabelForKind } from './cliSnippet';

  interface Props {
    onopenchanges: () => void;
  }
  let { onopenchanges }: Props = $props();

  const mainTabs: { id: DbMainTab; label: string; icon: IconName; show: () => boolean }[] = [
    { id: 'query', label: 'Query', icon: 'terminal', show: () => true },
    { id: 'builder', label: 'Builder', icon: 'layers', show: () => database.supportsBuilder },
    { id: 'structure', label: 'Structure', icon: 'columns', show: () => true },
    { id: 'diagram', label: 'Diagram', icon: 'shapes', show: () => database.capabilities?.engine !== 'redis' },
    { id: 'dashboards', label: 'Dashboards', icon: 'chart', show: () => true },
  ];
  const visibleTabs = $derived(mainTabs.filter((t) => t.show()));
  const activeConn = $derived(database.connections.find((c) => c.id === database.selectedConnId) ?? null);
  const hasSchemaChanges = $derived(['mysql', 'postgres'].includes(activeConn?.kind ?? ''));

  function healthTitle(st: { serverVersion?: string; latencyMs?: number }): string {
    const parts = ['Connected'];
    if (st.serverVersion) parts.push(st.serverVersion);
    if (st.latencyMs != null) parts.push(`${st.latencyMs} ms`);
    return parts.join(' · ');
  }

  function connActionsMenu(e: MouseEvent): void {
    const cliCmd = activeConn ? buildDbCliSnippet(activeConn, { activeDb: database.activeDb }) : null;
    const cliName = activeConn ? cliLabelForKind(activeConn.kind) : null;
    ctxMenu.show(e, [
      {
        label: database.testing ? 'Testing connection…' : 'Test connection',
        icon: 'plug',
        disabled: database.testing,
        action: () => void database.testConnection(),
      },
      ...(cliCmd && cliName
        ? [{
            label: `Copy as ${cliName}`,
            icon: 'copy',
            title: 'Copy a secret-free CLI command for this connection',
            action: () => void copyWithToast(cliCmd, `${cliName} command`),
          }]
        : []),
      ...(hasSchemaChanges
        ? [{
            label: 'Database changes',
            icon: 'branch',
            title: 'Reviewed database changes for this connection',
            action: onopenchanges,
          }]
        : []),
    ]);
  }
</script>

<div class="main-tabs" class:many={visibleTabs.length > 3}>
  <div class="segmented view-switch" role="tablist" aria-label="Workbench view" tabindex="-1" onkeydown={onTabKey}>
    {#each visibleTabs as t (t.id)}
      <button
        class="mt"
        class:active={database.mainTab === t.id}
        role="tab"
        aria-selected={database.mainTab === t.id}
        tabindex={database.mainTab === t.id ? 0 : -1}
        title={t.label}
        onclick={() => database.setMainTab(t.id)}
      >
        <Icon name={t.icon} size={12} />{t.label}
      </button>
    {/each}
  </div>
  <span class="grow"></span>
  <div class="conn-status">
    {#if database.isGuarded}
      {@const guardText = database.isProd
        ? 'Production connection — schema changes require an approved change. Other writes require permission and confirmation.'
        : 'Read-only connection — native privileges and access rules limit execution.'}
      <span class="guard-chip" class:prod={database.isProd} title={guardText} data-testid="db-guard-chip">
        <Icon name={database.isProd ? 'zap' : 'key'} size={12} />
        <span aria-hidden="true">{database.isProd ? 'Production' : 'Read-only'}</span>
        <span class="sr-only">{guardText}</span>
      </span>
    {/if}
    {#if database.activeConnStatus?.phase === 'connecting'}
      <span class="conn-state" title="Connecting…">
        <span class="conn-tab-spin spinner" style="--spinner-size: 12px" aria-hidden="true"></span>
        <span class="lbl">Connecting…</span>
      </span>
    {:else if database.activeConnStatus?.phase === 'error'}
      <span class="conn-state err" title={database.activeConnStatus.error}>Disconnected</span>
    {/if}
    {#if database.capabilities}
      <span
        class="conn-state"
        class:ok={database.activeConnStatus?.phase === 'ready'}
        title={database.activeConnStatus?.phase === 'ready' ? healthTitle(database.activeConnStatus) : 'Engine'}
      >
        {#if database.activeConnStatus?.phase === 'ready'}<span class="health-dot" aria-hidden="true"></span>{/if}
        <span class="cap-chip mono">{database.capabilities.engine}</span>
      </span>
    {/if}
    {#if database.testResult}
      <span class="test-dot" class:ok={database.testResult.ok} title={database.testResult.message}></span>
    {/if}
    <button
      class="icon-btn"
      onclick={connActionsMenu}
      aria-haspopup="menu"
      aria-label="Connection actions"
      title="Connection actions — test, copy as CLI{hasSchemaChanges ? ', schema changes' : ''}"
      data-testid="db-conn-actions"
    >
      <Icon name="more" size={14} />
    </button>
  </div>
</div>

<style>
  .main-tabs {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    box-sizing: border-box;
    padding: 0 16px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .grow {
    flex: 1;
  }
  .view-switch {
    flex-shrink: 0;
  }
  .view-switch .mt {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 10px;
  }
  .view-switch .mt :global(svg) {
    opacity: 0.75;
  }
  .view-switch .mt.active :global(svg) {
    opacity: 1;
  }
  .view-switch .mt:focus-visible {
    outline: 1.5px solid var(--accent-text);
    outline-offset: 1px;
  }
  .conn-status {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .guard-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--success);
    background: var(--success-soft);
    white-space: nowrap;
  }
  .guard-chip.prod {
    color: var(--danger);
    background: var(--danger-soft);
    font-weight: 600;
  }
  @container dbmain (max-width: 900px) {
    .cap-chip {
      display: none;
    }
    .main-tabs.many .conn-status .lbl {
      display: none;
    }
  }
  @container dbmain (max-width: 700px) {
    .conn-status .lbl {
      display: none;
    }
  }
  @container dbmain (max-width: 640px) {
    .view-switch .mt {
      font-size: 0;
      gap: 0;
      padding: 0 8px;
    }
  }
  .cap-chip {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .test-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-exited);
  }
  .test-dot.ok {
    background: var(--status-working);
  }
  .conn-tab-spin {
    color: var(--text-dim);
    margin-inline-start: 4px;
    flex-shrink: 0;
  }
  .conn-state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .conn-state.err {
    color: var(--danger);
    font-weight: 500;
  }
  .conn-state.ok {
    color: var(--text-dim);
    max-width: 220px;
  }
  .health-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-working);
    flex-shrink: 0;
  }
  @media (max-width: 640px) {
    .main-tabs {
      height: auto;
      min-height: 44px;
      padding: 6px 12px;
      flex-wrap: wrap;
      row-gap: 4px;
    }
    .view-switch {
      max-width: 100%;
      overflow-x: auto;
    }
    .main-tabs .grow {
      flex-basis: 100%;
      height: 0;
    }
    .view-switch .mt {
      height: 30px;
      font-size: var(--fs-m);
      padding: 0 10px;
    }
    .conn-status {
      flex-wrap: wrap;
    }
  }
  @media (min-width: 641px) and (max-width: 1024px) {
    .main-tabs {
      height: auto;
      min-height: 44px;
      padding: 6px 12px;
      flex-wrap: wrap;
      row-gap: 4px;
    }
    .main-tabs .grow {
      flex-basis: 100%;
      height: 0;
    }
    .conn-status {
      flex-wrap: wrap;
    }
  }
</style>
