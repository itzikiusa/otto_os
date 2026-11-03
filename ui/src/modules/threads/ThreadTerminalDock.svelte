<script lang="ts">
  // The bottom terminal drawer under the stage (T3): plain project shells in
  // the thread's folder, as tabs, with New, Split (two side by side) and Kill.
  // Its shells are ordinary shell sessions tagged `meta.threads_terminal`, so
  // they stay out of the thread lists. Lazy: loads when first opened.
  import Icon from '../../lib/components/Icon.svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { threadsChrome } from './threadsChrome.svelte';
  import { projectNameFromCwd } from './threadsHelpers';

  interface Props {
    cwd: string | null;
  }
  let { cwd }: Props = $props();

  let busy = $state(false);
  let tall = $state(false);

  const terms = $derived(
    threadsChrome.terminalIds
      .map((id) => ws.getSession(id))
      .filter((s): s is NonNullable<typeof s> => !!s && !s.archived),
  );
  const active = $derived(terms.find((t) => t.id === threadsChrome.terminalActive) ?? terms[0] ?? null);
  const second = $derived(
    threadsChrome.terminalSplit ? (terms.find((t) => t.id !== active?.id) ?? null) : null,
  );

  // Drop ids whose sessions are gone (killed elsewhere, another machine).
  $effect(() => {
    if (!ws.listSettled) return;
    threadsChrome.pruneTerminals(new Set(ws.sessions.filter((s) => !s.archived).map((s) => s.id)));
  });

  async function newTerminal(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const s = await ws.createSessionQuiet(
        {
          kind: 'agent',
          provider: 'shell',
          title: `Terminal · ${projectNameFromCwd(cwd)}`,
          cwd: cwd || null,
          meta: { surface: 'threads', threads_terminal: true },
        },
        { scratch: ws.current === null && ws.currentId !== SCRATCH_WORKSPACE_ID },
      );
      threadsChrome.addTerminal(s.id);
    } catch (e) {
      toastError('Couldn’t open a terminal', e);
    } finally {
      busy = false;
    }
  }

  async function kill(id: string): Promise<void> {
    try {
      await ws.killSession(id);
      threadsChrome.dropTerminal(id);
    } catch (e) {
      toastError('Couldn’t kill the terminal', e);
    }
  }

  // Opening the drawer with no shell starts one — once (a failure shows its
  // toast and leaves New for a manual retry, never a loop).
  let autoTried = false;
  $effect(() => {
    if (!autoTried && ws.listSettled && terms.length === 0 && !busy) {
      autoTried = true;
      void newTerminal();
    }
  });
</script>

<section class="ttd otto-force-dark" class:tall aria-label="Terminal" data-testid="threads-terminal-dock">
  <header class="ttd-bar">
    <div class="ttd-tabs" role="group" aria-label="Terminals">
      {#each terms as t, i (t.id)}
        <button
          type="button"
          class="ttd-tab"
          class:on={t.id === active?.id || t.id === second?.id}
          aria-pressed={t.id === active?.id}
          title={t.cwd ?? t.title}
          onclick={() => (threadsChrome.terminalActive = t.id)}
        >
          <Icon name="terminal" size={12} />
          <span>Terminal {i + 1}</span>
        </button>
      {/each}
    </div>
    <span class="ttd-grow"></span>
    <button type="button" class="icon-btn" aria-label="New terminal" title="New terminal" disabled={busy} onclick={() => void newTerminal()}>
      <Icon name="plus" size={12} />
    </button>
    <button
      type="button"
      class="icon-btn"
      class:ttd-on={threadsChrome.terminalSplit}
      aria-pressed={threadsChrome.terminalSplit}
      aria-label="Split terminals side by side"
      title={terms.length > 1 ? 'Split' : 'Open a second terminal to split'}
      disabled={terms.length < 2}
      onclick={() => threadsChrome.toggleSplit()}
    >
      <Icon name="split" size={12} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Kill the active terminal"
      title="Kill terminal"
      disabled={!active}
      onclick={() => active && void kill(active.id)}
    >
      <Icon name="trash" size={12} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label={tall ? 'Restore terminal height' : 'Expand terminal height'}
      title={tall ? 'Restore height' : 'Expand height'}
      onclick={() => (tall = !tall)}
    >
      <Icon name={tall ? 'minimize' : 'maximize'} size={12} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Hide terminal drawer"
      title="Hide terminal"
      onclick={() => threadsChrome.setTerminal(false)}
    >
      <Icon name="x" size={12} />
    </button>
  </header>
  <div class="ttd-body" class:split={!!second}>
    {#if active}
      {#key active.id}
        <div class="ttd-pane"><LazyTerminal sessionId={active.id} forceDark autoFocus resumeOnOpen /></div>
      {/key}
    {/if}
    {#if second}
      {#key second.id}
        <div class="ttd-pane"><LazyTerminal sessionId={second.id} forceDark resumeOnOpen /></div>
      {/key}
    {/if}
  </div>
</section>

<style>
  .ttd {
    display: flex;
    flex-direction: column;
    block-size: 240px;
    flex-shrink: 0;
    border-block-start: 1px solid var(--border);
    background: var(--term-bg);
  }
  .ttd.tall {
    block-size: 420px;
  }
  .ttd-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    block-size: 32px;
    padding-inline: 8px;
    background: var(--surface);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ttd-tabs {
    display: flex;
    gap: 2px;
    min-width: 0;
    overflow-x: auto;
  }
  .ttd-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    block-size: 24px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    white-space: nowrap;
    cursor: pointer;
  }
  .ttd-tab.on {
    background: var(--hover);
    color: var(--text);
  }
  .ttd-grow {
    flex: 1;
  }
  .ttd-on {
    color: var(--accent-text);
  }
  .ttd-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .ttd-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .ttd-body.split .ttd-pane + .ttd-pane {
    border-inline-start: 1px solid var(--border);
  }
  @media (prefers-reduced-motion: no-preference) {
    .ttd {
      animation: otto-fade-in var(--dur-enter) var(--ease-out);
    }
  }
</style>
