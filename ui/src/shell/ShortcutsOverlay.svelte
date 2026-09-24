<script lang="ts">
  // Keyboard-shortcut cheat sheet, triggered by `?` (handled in App.svelte).
  // A shared Modal: Esc / backdrop / ✕ close it, focus is trapped, and it
  // registers as an overlay (the live browser's native webview hides under
  // it). The binding list is derived from the KEYMAP in lib/keys.ts so it
  // stays in lockstep with the actual chords.
  import Modal from '../lib/components/Modal.svelte';
  import { KEYMAP } from '../lib/keys';

  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();

  /** Split a chord like "⌘⇧B" into individual <kbd> tokens; leave words like
   *  "Tab" / ranges like "⌃1…⌃9" intact. */
  function tokens(keys: string): string[] {
    // Keep modifier glyphs as separate keys, but don't split multi-char tokens.
    return keys.match(/⌘|⌃|⌥|⇧|[^⌘⌃⌥⇧]+/g) ?? [keys];
  }
</script>

{#if open}
  <Modal title="Keyboard shortcuts" width={720} {onclose}>
    <div class="sc-grid">
      {#each KEYMAP as group (group.category)}
        <section class="sc-group">
          <h3 class="sc-cat">{group.category}</h3>
          {#each group.bindings as b (b.keys + b.label)}
            <div class="sc-row">
              <span class="sc-label">{b.label}</span>
              <span class="sc-keys">
                {#each tokens(b.keys) as t, i (i)}<kbd>{t}</kbd>{/each}
              </span>
            </div>
          {/each}
        </section>
      {/each}
    </div>
  </Modal>
{/if}

<style>
  .sc-grid {
    padding-top: 6px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 4px 28px;
    align-content: start;
  }
  @media (max-width: 640px) {
    .sc-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .sc-group {
    break-inside: avoid;
    margin-bottom: 10px;
  }
  .sc-cat {
    margin: 0 0 6px;
    font-size: var(--fs-xs);
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
    font-size: var(--fs-m);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    white-space: nowrap;
  }
</style>
