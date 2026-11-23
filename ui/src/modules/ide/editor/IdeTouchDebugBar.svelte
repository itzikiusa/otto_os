<script lang="ts">
  // Paused in the debugger on a phone: step without leaving the code. A pill over the
  // bottom of the editor pane — Resume · Step over · Step into · Step out · Stop, 44 px
  // each, and where the program stopped. The same store actions as the debugger panel;
  // the paused line is scrolled clear of the pill after every stop. Stacked layout only.
  import { onDestroy } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ide } from '../ideStore.svelte';
  import { ideLayout } from '../shell/ideLayout.svelte';
  import { idePhoneChrome } from './idePhoneChrome.svelte';
  import { pausedLineClearance, touchDebugBar } from './ideTouchDebugBar';

  let stepping = $state(false);
  let barEl = $state<HTMLDivElement | null>(null);
  let stepTimer: ReturnType<typeof setTimeout> | undefined;

  const bar = $derived(
    touchDebugBar({
      dbg: ide.activeDebug,
      editorPane: ideLayout.stack && ideLayout.pane === 'editor',
      stepping,
      typingMode: idePhoneChrome.current.typingMode,
    }),
  );

  // A stop ends the step (the bar then shows from the paused state itself).
  $effect(() => {
    if (ide.activeDebug?.state !== 'running') stepping = false;
  });

  function step(run: () => Promise<void>): void {
    stepping = true;
    clearTimeout(stepTimer);
    // A step that runs to the end of the program never pauses again: let the bar go.
    stepTimer = setTimeout(() => (stepping = false), 2500);
    void run();
  }

  onDestroy(() => {
    clearTimeout(stepTimer);
  });

  // After every stop: the paused line must not sit under the pill.
  $effect(() => {
    void ide.activeDebug?.paused_line;
    void ide.activeDebug?.paused_file;
    const el = barEl;
    if (!el || !bar.paused) return;
    let tries = 0;
    let raf = 0;
    const clear = (): void => {
      const pane = el.closest('.ide-pane');
      const line = pane?.querySelector<HTMLElement>('.cm-ide-paused-line');
      const scroller = pane?.querySelector<HTMLElement>('.cm-scroller');
      // The editor may still be opening the paused file / scrolling to it.
      if (!line || !scroller) { if (++tries < 20) raf = requestAnimationFrame(clear); return; }
      const d = pausedLineClearance(line.getBoundingClientRect().bottom, el.getBoundingClientRect().top);
      if (d > 0) scroller.scrollTop += d;
    };
    raf = requestAnimationFrame(clear);
    return () => cancelAnimationFrame(raf);
  });
</script>

{#if bar.show}
  <div class="ide-touch-dbg" role="toolbar" aria-label="Debugger" bind:this={barEl} data-testid="ide-touch-debug-bar">
    <button type="button" class="tdb-btn" aria-label="Resume program" title="Resume program" disabled={!bar.paused} onclick={() => step(ide.resumeDebug)} data-testid="ide-touch-debug-resume"><Icon name="play" size={16} /></button>
    <button type="button" class="tdb-btn" aria-label="Step over" title="Step over" disabled={!bar.paused} onclick={() => step(ide.stepOver)} data-testid="ide-touch-debug-step-over"><Icon name="stepOver" size={16} /></button>
    <button type="button" class="tdb-btn" aria-label="Step into" title="Step into" disabled={!bar.paused} onclick={() => step(ide.stepInto)} data-testid="ide-touch-debug-step-into"><Icon name="stepInto" size={16} /></button>
    <button type="button" class="tdb-btn" aria-label="Step out" title="Step out" disabled={!bar.paused} onclick={() => step(ide.stepOut)} data-testid="ide-touch-debug-step-out"><Icon name="stepOut" size={16} /></button>
    <button type="button" class="tdb-btn tdb-stop" aria-label="Stop debug session" title="Stop debug session" onclick={() => void ide.stopDebug()} data-testid="ide-touch-debug-stop"><Icon name="stop" size={16} /></button>
    <span class="tdb-where mono" dir="ltr" use:announceShown={bar.where || 'Running…'} data-testid="ide-touch-debug-where">{bar.where || 'Running…'}</span>
  </div>
{/if}

<style>
  /* The pane is the pill's containing block; its code can scroll above the pill. */
  :global(.ide-pane:has(> .ide-touch-dbg)) { position: relative; }
  :global(.ide-pane:has(> .ide-touch-dbg) .cm-content) { padding-block-end: 72px; }
  .ide-touch-dbg {
    position: absolute; inset-block-end: var(--sp-3); inset-inline: var(--sp-3); z-index: var(--z-sticky);
    display: flex; align-items: center; gap: 2px; padding: 2px; max-inline-size: calc(100% - 2 * var(--sp-3));
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-l);
    box-shadow: var(--glass-shadow);
  }
  .tdb-btn {
    display: grid; place-items: center; inline-size: 44px; block-size: 44px; flex-shrink: 0;
    background: transparent; border: none; border-radius: var(--radius-s); color: var(--text); cursor: pointer;
  }
  .tdb-btn:hover:not(:disabled) { background: var(--surface-2); }
  .tdb-btn:disabled { opacity: var(--disabled-opacity); cursor: default; }
  .tdb-stop { color: var(--danger); }
  .tdb-where {
    flex: 1 1 auto; min-inline-size: 0; padding-inline: var(--sp-2); overflow: hidden; text-overflow: ellipsis;
    white-space: nowrap; font-size: var(--fs-xs); color: var(--text-dim); text-align: end;
  }
</style>
