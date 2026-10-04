<script lang="ts">
  // Drawer "Terminal" tab: opens a `kubectl exec -it` terminal session for the pod
  // (`POST …/exec`, Edit) and renders it inline with `<Terminal preferDom>`
  // (agent-TUI renderer; shells in a pod redraw prompts constantly). The
  // session is killed when the view unmounts — it lives only in this drawer.
  import { untrack } from 'svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import { clusterLabel } from './k8s-util';
  import type { K8sContainer, SessionStatus } from '../../lib/api/types';

  interface Props {
    clusterId: string;
    ns: string;
    pod: string;
    containers: K8sContainer[];
    /** Open the shell immediately (the `s` shortcut). */
    autoOpen?: boolean;
  }
  let { clusterId, ns, pod, containers, autoOpen = false }: Props = $props();

  $effect(() => { void resourceAccess.load('k8s_cluster', clusterId, `namespace:${ns}`); });
  const canExec = $derived(resourceAccess.can('k8s_cluster', clusterId, 'exec', 'kubernetes', 'edit', `namespace:${ns}`));
  let container = $state('');
  let sessionId = $state<string | null>(null);
  let status = $state<SessionStatus | null>(null);
  let opening = $state(false);
  let error = $state('');

  const running = $derived(containers.filter((c) => !c.init));
  const cluster = $derived(k8s.clusters.find((c) => c.id === clusterId) ?? null);
  const isProd = $derived(cluster?.environment === 'prod');

  async function open(): Promise<void> {
    if (!canExec || opening) return;
    const wsId = ws.currentId;
    if (!wsId) {
      error = 'Select a workspace first — the exec session is attached to it.';
      return;
    }
    // A shell in a production pod can change anything in it — ask first.
    if (isProd) {
      const ok = await confirmer.ask(
        `Open a shell in pod “${pod}” in ${ns} on ${cluster ? clusterLabel(cluster) : 'this cluster'} (PRODUCTION)? Anything you type runs inside the live container.`,
        { title: 'Open a production shell', confirmLabel: 'Open shell', danger: true },
      );
      if (!ok) return;
    }
    opening = true;
    error = '';
    try {
      const s = await k8sApi.exec(clusterId, {
        workspace_id: wsId,
        ns,
        pod,
        container: container || null,
      });
      sessionId = s.id;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      opening = false;
    }
  }

  async function close(): Promise<void> {
    const id = sessionId;
    sessionId = null;
    status = null;
    if (!id) return;
    try {
      await api.del(`/sessions/${id}`);
    } catch {
      /* best-effort — the terminal session is torn down when the daemon notices anyway */
    }
  }

  // Auto-open once when asked; kill the session on unmount.
  $effect(() => {
    if (autoOpen) untrack(() => void open());
    return () => {
      untrack(() => void close());
    };
  });
</script>

<div class="exec">
  {#if !canExec}
    <div class="note"><Icon name="lock" size={13} /> Opening a shell needs the Kubernetes <b>edit</b> grant.</div>
  {:else if sessionId}
    <div class="exec-bar">
      <span class="mono">{pod}{container ? ` · ${container}` : ''}</span>
      {#if cluster}<EnvBadge env={cluster.environment} />{/if}
      <span class="dim">{status ?? ''}</span>
      <span class="spacer"></span>
      <button class="btn small" onclick={() => void close()}>Close shell</button>
    </div>
    <div class="term">
      {#key sessionId}
        <Terminal {sessionId} preferDom autoFocus restartable onrestart={() => { void close().then(open); }} onstatus={(s) => (status = s)} />
      {/key}
    </div>
  {:else}
    <div class="launch">
      <p class="dim">Runs <span class="mono">kubectl exec -it {pod} -- sh</span> (bash when the image has it) as a terminal session inside Otto.{#if cluster} Cluster: <EnvBadge env={cluster.environment} />{/if}</p>
      {#if running.length > 1}
        <label class="field">
          <span class="lbl">Container</span>
          <select class="input" bind:value={container}>
            <option value="">default ({running[0]?.name})</option>
            {#each running as c (c.name)}<option value={c.name}>{c.name}</option>{/each}
          </select>
        </label>
      {/if}
      <button class="btn primary" onclick={() => void open()} disabled={opening}>
        <Icon name="terminal" size={13} /> {opening ? 'Opening…' : 'Open shell'}
      </button>
      {#if error}<div class="err">{error}</div>{/if}
    </div>
  {/if}
</div>

<style>
  .exec {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .exec-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .spacer {
    flex: 1;
  }
  .term {
    flex: 1;
    min-height: 260px;
    background: #000;
  }
  .launch {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: flex-start;
  }
  .launch p {
    margin: 0;
    font-size: var(--fs-m);
    line-height: 1.5;
  }
  .lbl {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 16px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
