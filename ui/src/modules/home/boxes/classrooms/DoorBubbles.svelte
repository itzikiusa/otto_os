<script lang="ts">
  // Otto School — the ✋ / ❗ badges over corridor doors (ClassroomsBox's
  // overlay). Decorative (the overlay is aria-hidden; the "needs you" queue
  // and the list carry the same facts as real buttons). `bubble` registers
  // each badge with the box, which places it at its door's projection.
  import type { Action } from 'svelte/action';
  import type { Room } from '../../school/model';

  interface Props {
    /** Corridor doors with someone waiting (or in error). */
    rooms: Room[];
    bubble: Action<HTMLElement, string>;
  }
  let { rooms, bubble }: Props = $props();
</script>

{#each rooms as r (r.id)}
  <div class="bubble" class:err={r.counts.needsYou === 0} use:bubble={`door:${r.id}`}>
    {r.counts.needsYou > 0 ? `✋ ${r.counts.needsYou}` : `❗ ${r.counts.error}`}
  </div>
{/each}

<style>
  .bubble {
    position: absolute;
    top: 0;
    left: 0; /* ui-guards: allow — origin of projected screen coordinates, not layout */
    visibility: hidden;
    white-space: nowrap;
    line-height: 1;
    padding: 4px 6px;
    border-radius: var(--radius-m);
    border: 1px solid var(--warning);
    animation: otto-pulse 1.4s ease-in-out infinite;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    background: var(--surface);
    box-shadow: var(--shadow-xs);
  }
  .bubble.err {
    border-color: var(--danger);
  }
  @media (prefers-reduced-motion: reduce) {
    .bubble {
      animation: none;
    }
  }
</style>
