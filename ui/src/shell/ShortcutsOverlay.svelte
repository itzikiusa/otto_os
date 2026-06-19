<script lang="ts">
  // Keyboard-shortcut cheat sheet, triggered by `?` (handled in App.svelte) and
  // dismissed with Esc or a backdrop click. The binding list is derived from the
  // KEYMAP in lib/keys.ts so it stays in lockstep with the actual chords.
  import { KEYMAP } from '../lib/keys';

  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
    }
  }

  /** Split a chord like "⌘⇧B" into individual <kbd> tokens; leave words like
   *  "Tab" / ranges like "⌃1…⌃9" intact. */
  function tokens(keys: string): string[] {
    // Keep modifier glyphs as separate keys, but don't split multi-char tokens.
    return keys.match(/⌘|⌃|⌥|⇧|[^⌘⌃⌥⇧]+/g) ?? [keys];
  }
</script>

<svelte:window onkeydown={open ? onKeydown : undefined} />

{#if open}
  <div
    class="sc-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onclose();
    }}
  >
    <div class="sc-sheet" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
      <div class="sc-head">
        <span class="sc-title">Keyboard Shortcuts</span>
        <span class="grow"></span>
        <kbd>Esc</kbd>
        <span class="sc-hint">to close</span>
      </div>
      <div class="sc-grid">
        {#each KEYMAP as group (group.category)}
          <section class="sc-group">
            <h3 class="sc-cat">{group.category}</h3>
            {#each group.bindings as b (b.keys + b.label)}
              <div class="sc-row">
                <span class="sc-label">{b.label}</span>
                <span class="sc-keys">
                  {#each tokens(b.keys) as t (t)}<kbd>{t}</kbd>{/each}
                </span>
              </div>
            {/each}
          </section>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .sc-backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: rgba(0, 0, 0, 0.25);
    display: flex;
    justify-content: center;
    padding-top: 10vh;
    animation: fade-in 120ms ease-out;
  }
  .sc-sheet {
    width: 720px;
    max-width: calc(100vw - 48px);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    overflow: hidden;
    align-self: flex-start;
    animation: pal-in 150ms ease-out;
  }
  .sc-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .sc-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .sc-hint {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .sc-grid {
    overflow-y: auto;
    padding: 14px 16px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 4px 28px;
    align-content: start;
  }
  .sc-group {
    break-inside: avoid;
    margin-bottom: 10px;
  }
  .sc-cat {
    margin: 0 0 6px;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .sc-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 3px 0;
    font-size: 12.5px;
    color: var(--text);
  }
  .sc-label {
    flex: 1;
    min-width: 0;
  }
  .sc-keys {
    flex-shrink: 0;
    display: inline-flex;
    gap: 3px;
  }
  kbd {
    font-family: var(--font-ui);
    font-size: 10.5px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
  @keyframes pal-in {
    from {
      opacity: 0;
      transform: translateY(-6px) scale(0.99);
    }
  }
</style>
