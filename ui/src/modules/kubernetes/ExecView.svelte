<script lang="ts">
  // Drawer "Terminal" tab: opens a `kubectl exec -it` PTY session for the pod
  // (`POST …/exec`, Edit) and renders it inline with `<Terminal preferDom>`
  // (agent-TUI renderer; shells in a pod redraw prompts constantly). The
  // session is killed when the view unmounts — it lives only in this drawer.
  import { untrack } from 'svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
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

  const canExec = $derived(auth.can('kubernetes', 'edit'));
  let container = $state('');
  let sessionId = $state<string | null>(null);
  let status = $state<SessionStatus | null>(null);
  let opening = $state(false);
  let error = $state('');

  const running = $derived(containers.filter((c) => !c.init));

  async function open(): Promise<void> {
    if (!canExec || opening) return;
    const wsId = ws.currentId;
    if (!wsId) {
      error = 'Select a workspace first — the exec session is attached to it.';
      return;
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
      /* best-effort — the PTY is torn down when the daemon notices anyway */
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
      <p class="dim">Runs <span class="mono">kubectl exec -it {pod} -- sh</span> (bash when the image has it) as a PTY session inside Otto.</p>
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
    font-size: 12px;
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
    font-size: 12.5px;
    line-height: 1.5;
  }
  .lbl {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .note {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 16px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .err {
    color: var(--status-exited);
    font-size: 12px;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
