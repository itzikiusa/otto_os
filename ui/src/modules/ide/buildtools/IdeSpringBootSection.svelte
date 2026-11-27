<script lang="ts">
  // Collapsible Spring Boot Actuator & DevTools inspector inside the Build Tool Window.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import type { IdeSpringActuatorResp, IdeSpringBootInfo, IdeTargetSpec } from '../../../lib/api/types';
  import { loadErrorText } from '../../../lib/loadError';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide, ideBasename } from '../ideStore.svelte';
  import { inspectSpringActuator, triggerSpringDevtoolsRestart } from './ideBuildToolModel';

  type SpringTab = 'endpoints' | 'beans' | 'config';

  interface Props {
    info: IdeSpringBootInfo;
    target: IdeTargetSpec;
    open: boolean;
    disabled?: boolean;
    ontoggle: () => void;
  }

  let { info, target, open, disabled = false, ontoggle }: Props = $props();

  let actuatorUrl = $state('');
  let tab = $state<SpringTab>('endpoints');
  let query = $state('');
  let loading = $state(false);
  let restarting = $state(false);
  let error = $state<string | null>(null);
  let snapshot = $state<IdeSpringActuatorResp | null>(null);
  let pollSeq = 0;

  $effect(() => {
    if (!actuatorUrl && info.actuator_url) {
      actuatorUrl = info.actuator_url;
    }
  });

  async function refreshActuator(): Promise<void> {
    if (disabled) return;
    const url = (actuatorUrl || info.actuator_url).trim();
    if (!url) return;
    const seq = ++pollSeq;
    const boundTarget = target;
    loading = true;
    error = null;
    try {
      const res = await inspectSpringActuator(url, boundTarget);
      if (seq !== pollSeq) return;
      snapshot = res;
    } catch (e) {
      if (seq !== pollSeq) return;
      error = loadErrorText(e);
    } finally {
      if (seq === pollSeq) loading = false;
    }
  }

  async function handleDevtoolsRestart(): Promise<void> {
    if (disabled || !info.has_devtools) return;
    const boundTarget = target;
    const boundTrigger = info.trigger_file;
    const boundModuleRoot = info.module_root ?? null;
    restarting = true;
    try {
      const res = await triggerSpringDevtoolsRestart(boundTrigger, boundTarget, boundModuleRoot);
      toasts.success(res.message);
    } catch (e) {
      toasts.error(loadErrorText(e));
    } finally {
      restarting = false;
    }
  }

  const statusTone = $derived<'ok' | 'bad' | 'neutral'>(
    snapshot?.status === 'UP' ? 'ok' : snapshot?.reachable ? 'bad' : 'neutral',
  );
  const statusLabel = $derived(
    snapshot?.status ?? (snapshot ? (snapshot.reachable ? 'LIVE' : 'Offline') : 'Idle'),
  );

  const tabItems = $derived<readonly TabItem<SpringTab>[]>([
    { id: 'endpoints', label: 'Endpoints', count: snapshot?.endpoints.length ?? 0 },
    { id: 'beans', label: 'Beans', count: snapshot?.beans.length ?? 0 },
    { id: 'config', label: 'Config', count: info.config_files.length },
  ]);

  const filteredEndpoints = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = snapshot?.endpoints ?? [];
    if (!q) return list;
    return list.filter(
      (ep) =>
        ep.path.toLowerCase().includes(q)
        || ep.method.toLowerCase().includes(q)
        || ep.handler.toLowerCase().includes(q),
    );
  });

  const filteredBeans = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = snapshot?.beans ?? [];
    if (!q) return list;
    return list.filter(
      (b) => b.name.toLowerCase().includes(q) || b.bean_type.toLowerCase().includes(q),
    );
  });
</script>

<div class="sb-sec" data-testid="ide-bt-spring-boot">
  <div class="sb-hdr-row">
    <button type="button" class="sb-hdr-btn" aria-expanded={open} onclick={ontoggle}>
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} noflip />
      <span class="sb-mark">SB</span>
      <span class="sb-title">Spring Boot</span>
      <Badge tone={statusTone} variant="soft" label={statusLabel} />
    </button>
    <button
      type="button"
      class="sb-mini-btn"
      aria-label="Refresh Actuator"
      title="Poll local Spring Boot Actuator"
      disabled={disabled || loading}
      onclick={() => void refreshActuator()}
    >
      <Icon name="refresh" size={12} />
    </button>
    {#if info.has_devtools}
      <button
        type="button"
        class="sb-mini-btn"
        aria-label="DevTools Restart"
        title="Request Spring Boot DevTools restart check (touch classpath trigger file)"
        disabled={disabled || restarting}
        onclick={() => void handleDevtoolsRestart()}
      >
        <Icon name="play" size={12} />
      </button>
    {/if}
  </div>

  {#if open}
    <div class="sb-body">
      <div class="sb-url-row">
        <input
          class="sb-url-in"
          type="text"
          dir="ltr"
          aria-label="Spring Boot Actuator loopback URL"
          placeholder="http://127.0.0.1:8080/actuator"
          disabled={disabled}
          bind:value={actuatorUrl}
          onkeydown={(e) => { if (e.key === 'Enter') void refreshActuator(); }}
        />
        <button type="button" class="btn small ghost" disabled={disabled || loading} onclick={() => void refreshActuator()}>
          {loading ? '…' : 'Poll'}
        </button>
      </div>

      <LoadState
        what="Spring Boot Actuator"
        loading={loading && !snapshot}
        {error}
        variant="compact"
        onretry={() => void refreshActuator()}
      >
        <Tabs
          label="Spring Boot inspector"
          tabs={tabItems}
          value={tab}
          onchange={(id) => (tab = id)}
          idBase="ide-spring"
          size="s"
        />

        <div role="tabpanel" id={`ide-spring-panel-${tab}`} aria-labelledby={`ide-spring-tab-${tab}`}>
          {#if tab !== 'config'}
            <input
              class="sb-filter-in"
              type="search"
              dir="ltr"
              aria-label="Filter Spring Boot Actuator items"
              placeholder={tab === 'endpoints' ? 'Filter routes or handlers…' : 'Filter bean names or types…'}
              bind:value={query}
            />
          {/if}

          {#if tab === 'endpoints'}
            {#if !snapshot}
              <div class="sb-empty">Click <strong>Poll</strong> while the app runs locally to inspect Actuator endpoints.</div>
            {:else if !snapshot.reachable}
              <div class="sb-empty">Actuator not reachable at <code dir="ltr">{snapshot.base_url}</code>.</div>
            {:else if filteredEndpoints.length === 0}
              <div class="sb-empty">No HTTP mappings exposed (enable <code>management.endpoints.web.exposure.include=mappings</code>).</div>
            {:else}
              <div class="sb-list">
                {#each filteredEndpoints as ep, idx (`${ep.method}:${ep.path}:${idx}`)}
                  <div class="sb-item" title={ep.handler}>
                    <Badge tone="info" variant="outline" label={ep.method} />
                    <span class="sb-mono" dir="ltr">{ep.path}</span>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if tab === 'beans'}
            {#if !snapshot || !snapshot.reachable}
              <div class="sb-empty">Poll a running local Actuator instance to inspect Spring beans.</div>
            {:else if filteredBeans.length === 0}
              <div class="sb-empty">No beans returned (expose <code>beans</code> endpoint in Actuator).</div>
            {:else}
              <div class="sb-list">
                {#each filteredBeans as b (`${b.context}:${b.name}`)}
                  <div class="sb-item" title={`${b.bean_type} (${b.scope})`}>
                    <span class="sb-mono" dir="ltr">{b.name}</span>
                    <span class="sb-dim" dir="ltr">{b.scope}</span>
                  </div>
                {/each}
              </div>
            {/if}
          {:else}
            <div class="sb-list">
              {#if snapshot?.health_components?.length}
                <div class="sb-subhdr">Health Components</div>
                {#each snapshot.health_components as hc (hc.name)}
                  <div class="sb-item">
                    <span class="sb-mono" dir="ltr">{hc.name}</span>
                    <Badge tone={hc.status === 'UP' ? 'ok' : 'bad'} variant="soft" label={hc.status} />
                  </div>
                {/each}
              {/if}
              {#if info.config_files.length > 0}
                <div class="sb-subhdr">Configuration Files</div>
                {#each info.config_files as cf (cf)}
                  <button type="button" class="sb-file-btn" title={cf} onclick={() => void ide.openFile(cf, { preview: false })}>
                    <Icon name="file" size={12} />
                    <span class="sb-mono" dir="ltr">{ideBasename(cf)}</span>
                  </button>
                {/each}
              {/if}
              {#if info.apps.length > 0}
                <div class="sb-subhdr">Applications</div>
                {#each info.apps as app (app.main_class)}
                  <div class="sb-item" title={app.main_class}>
                    <span class="sb-mono" dir="ltr">{app.name}</span>
                    <span class="sb-dim" dir="ltr">:{info.server_port}</span>
                  </div>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      </LoadState>
    </div>
  {/if}
</div>

<style>
  .sb-sec { border-block-end: 1px solid var(--border); padding-block-end: 4px; margin-block-end: 4px; }
  .sb-hdr-row { display: flex; align-items: center; gap: 2px; padding-inline: 2px; }
  .sb-hdr-btn { display: flex; align-items: center; gap: 6px; flex: 1; min-inline-size: 0; border: 0; border-radius: var(--radius-s); background: transparent; color: var(--text); font-size: var(--fs-xs); font-weight: 500; padding-block: 4px; padding-inline: 4px; text-align: start; cursor: pointer; }
  .sb-hdr-btn:hover { background: var(--hover); }
  .sb-mark { display: inline-flex; align-items: center; justify-content: center; padding-inline: 4px; block-size: 16px; border-radius: var(--radius-s); background: var(--success-soft); color: var(--success); font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .sb-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sb-mini-btn { display: inline-flex; align-items: center; justify-content: center; inline-size: 20px; block-size: 20px; border: 0; border-radius: var(--radius-s); background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0; }
  .sb-mini-btn:hover { background: var(--surface-3); color: var(--text); }
  .sb-body { display: flex; flex-direction: column; gap: 6px; padding-inline: 8px 4px; padding-block-start: 4px; }
  .sb-url-row { display: flex; align-items: center; gap: 4px; }
  .sb-url-in, .sb-filter-in { inline-size: 100%; min-inline-size: 0; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); color: var(--text); font-size: var(--fs-xs); padding-block: 4px; padding-inline: 6px; }
  .sb-filter-in { margin-block: 4px; }
  .sb-list { display: flex; flex-direction: column; gap: 2px; max-block-size: 180px; overflow-y: auto; }
  .sb-item { display: flex; align-items: center; justify-content: space-between; gap: 6px; padding-block: 2px; padding-inline: 4px; font-size: var(--fs-xs); color: var(--text); }
  .sb-file-btn { display: flex; align-items: center; gap: 6px; inline-size: 100%; border: 0; border-radius: var(--radius-s); background: transparent; color: var(--text); font-size: var(--fs-xs); padding-block: 4px; padding-inline: 4px; text-align: start; cursor: pointer; }
  .sb-file-btn:hover { background: var(--hover); }
  .sb-mono { font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .sb-dim { color: var(--text-dim); flex-shrink: 0; }
  .sb-subhdr { font-size: var(--fs-xs); color: var(--text-dim); font-weight: 600; padding-block-start: 4px; }
  .sb-empty { font-size: var(--fs-xs); color: var(--text-dim); padding-block: 4px; }
</style>
