<script lang="ts">
  // "Diagnose with an agent": the consent dialog and the embedded thread,
  // mounted ONCE beside the Connections page's main area so it works for DB
  // tabs, SSH tabs and the empty hub alike. On a phone it is a full-height
  // sheet. The user never leaves the page; Detach moves the thread to Threads.
  // State + rules: diagnose.svelte.ts, diagnose-prompt.ts.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { api } from '../../lib/api/client';
  import { loadErrorText } from '../../lib/loadError';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadRuntimeThreadView, openInThreads } from '../../lib/runtimeThreadPort';
  import type { Connection, TestConnectionResp } from '../../lib/api/types';
  import { connDiagnose } from './diagnose.svelte';
  import { diagnoseChecks, diagnoseConsent } from './diagnose-prompt';

  interface Props {
    /** The page's edit permission rule: a handler when the viewer may edit. */
    editFor?: (c: Connection | null) => ((c: Connection) => void) | undefined;
  }
  let { editFor }: Props = $props();

  const ThreadView = lazyComponent(loadRuntimeThreadView);
  const a = $derived(connDiagnose.active);
  const consent = $derived(connDiagnose.consent);
  const onedit = $derived(a ? editFor?.(a.conn) : undefined);

  // "Test again" after the agent finished: the connection's own Test probe.
  let retest = $state<{ ok: boolean; text: string } | null>(null);
  let retesting = $state(false);
  $effect(() => {
    void a?.sessionId;
    retest = null;
  });
  async function testAgain(): Promise<void> {
    if (!a || retesting) return;
    retesting = true;
    try {
      const r = await api.post<TestConnectionResp>(`/connections/${a.conn.id}/test`, {});
      retest = { ok: r.ok, text: r.ok ? `Connected${r.latency_ms != null ? ` (${r.latency_ms} ms)` : ''}` : r.message };
    } catch (e) {
      retest = { ok: false, text: loadErrorText(e) };
    } finally {
      retesting = false;
    }
  }

  function detach(id: string): void {
    connDiagnose.close();
    openInThreads(id);
  }

  const phaseText = (p: string): string =>
    p === 'starting' ? 'Starting…' : p === 'running' ? 'Running' : p === 'finished' ? 'Finished' : 'Failed to start';
</script>

{#if consent}
  <Modal title="Diagnose {consent.conn.name} with an agent" width={560} onclose={() => connDiagnose.cancel()}>
    <div class="consent" data-testid="diagnose-consent">
      <p>{diagnoseConsent(consent.conn)}</p>
      <details>
        <summary>Show the checks</summary>
        <ul class="checks">
          {#each diagnoseChecks(consent.conn) as cmd (cmd)}<li><code class="mono">{cmd}</code></li>{/each}
        </ul>
      </details>
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => connDiagnose.cancel()}>Cancel</button>
      <button class="btn primary" onclick={() => void connDiagnose.confirm()} data-testid="diagnose-start">Start</button>
    {/snippet}
  </Modal>
{/if}

{#if a && connDiagnose.shown}
  <aside class="diag" class:max={connDiagnose.maximized} aria-label="Diagnosing {a.conn.name}" data-testid="diagnose-aside">
    <header class="diag-head">
      <Icon name="assistant" size={14} />
      <span class="diag-title ellipsis">Diagnosing {a.conn.name}</span>
      <span class="diag-phase {a.phase}">{phaseText(a.phase)}</span>
      <button class="icon-btn" onclick={() => connDiagnose.close()} aria-label="Close — the thread is kept" title="Close — the thread is kept; Diagnose opens it again">
        <Icon name="x" size={14} />
      </button>
    </header>
    <div class="diag-body">
      {#if a.phase === 'failed'}
        <LoadState what="the diagnosis" errorTitle="Couldn’t start the diagnosis" error={a.error ?? 'Unknown error'} empty onretry={() => void connDiagnose.retry()} variant="panel" />
      {:else if a.phase === 'starting' || !a.sessionId}
        <LoadState what="the diagnosis thread" loading empty variant="panel" />
      {:else if ThreadView.error}
        <LoadState what="the thread" error={ThreadView.error} empty onretry={() => ThreadView.retry()} variant="panel" />
      {:else if ThreadView.component}
        <ThreadView.component
          sessionId={a.sessionId}
          workspaceId={a.workspaceId}
          compact
          onmaximize={() => (connDiagnose.maximized = !connDiagnose.maximized)}
          ondetach={detach}
        />
      {:else}
        <LoadState what="the thread" loading empty variant="panel" />
      {/if}
    </div>
    {#if a.phase === 'finished'}
      <footer class="diag-foot" data-testid="diagnose-after">
        <button class="btn small primary" onclick={() => void testAgain()} disabled={retesting}>
          <Icon name="refresh" size={12} />{retesting ? 'Testing…' : 'Test again'}
        </button>
        {#if onedit}
          {@const c = a.conn}
          <button class="btn small" onclick={() => onedit(c)}><Icon name="edit" size={12} />Edit connection</button>
        {/if}
        <!-- Always mounted: the result line changes in place, so it is announced. -->
        <span class="retest" class:ok={retest?.ok} role="status">{retest?.text ?? ''}</span>
      </footer>
    {/if}
  </aside>
{/if}

<style>
  .consent p {
    margin: 0 0 10px;
    line-height: 1.5;
  }
  .consent summary {
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .checks {
    margin: 6px 0 0;
    padding-inline-start: 18px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .checks code {
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .diag {
    flex: none;
    width: min(460px, 45%);
    min-width: 300px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
  }
  .diag.max {
    width: 70%;
  }
  .diag-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding-block: 6px;
    padding-inline: 12px 8px;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .diag-title {
    flex: 1;
    min-width: 0;
    font-weight: 600;
  }
  .diag-phase {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .diag-phase.failed {
    color: var(--danger);
  }
  .diag-phase.finished {
    color: var(--success);
  }
  .diag-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .diag-body > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .diag-foot {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-block-start: 1px solid var(--border);
  }
  .retest {
    font-size: var(--fs-s);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .retest.ok {
    color: var(--success);
  }
  /* Phones (portrait AND landscape, e.g. 932×430) and tablets: a full-height
     sheet over the page. Below ~1025 px a side split leaves neither the
     workbench nor a compact thread (≥ 280 px) usable width next to the list. */
  @media (max-width: 1024px) {
    .diag,
    .diag.max {
      position: fixed;
      inset: 0;
      z-index: var(--z-drawer);
      width: auto;
      min-width: 0;
      border-inline-start: none;
      padding-block-end: env(safe-area-inset-bottom);
    }
  }
</style>
