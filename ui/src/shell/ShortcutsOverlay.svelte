<script lang="ts">
  // Keyboard-shortcut cheat sheet, triggered by `?` (handled in App.svelte).
  // A shared Modal: Esc / backdrop / ✕ close it, focus is trapped, and it
  // registers as an overlay (the live browser's native webview hides under
  // it). The binding list is derived from the KEYMAP in lib/keys.ts so it
  // stays in lockstep with the actual chords.
  import Modal from '../lib/components/Modal.svelte';
  import { KEYMAP } from '../lib/keys';
  import { shortcutChips } from '../lib/shortcutChips';

  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();
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
                {#each shortcutChips(b.keys) as c, i (i)}{#if c.kind === 'sep'}<span
                      class="sc-sep">{c.text}</span
                    >{:else}<kbd>{c.text}</kbd>{/if}{/each}
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
    direction: ltr;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .sc-sep {
    font-size: var(--fs-xs);
    color: var(--text-dim);
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
