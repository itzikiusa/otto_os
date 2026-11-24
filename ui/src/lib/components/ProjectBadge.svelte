<script lang="ts">
  // A project's two-letter tile (projectBadge.ts): decoration beside the
  // project's name, so it is hidden from assistive tech.
  import { projectBadge } from '../projectBadge';

  let { name }: { name: string } = $props();
  const b = $derived(projectBadge(name));
</script>

<span class="pb t{b.tone}" aria-hidden="true" data-testid="project-badge">{b.initials}</span>

<style>
  .t1 { --pb-tone: var(--cat-1); }
  .t2 { --pb-tone: var(--cat-2); }
  .t3 { --pb-tone: var(--cat-3); }
  .t4 { --pb-tone: var(--cat-4); }
  .t5 { --pb-tone: var(--cat-5); }
  .t6 { --pb-tone: var(--cat-6); }
  .pb {
    display: inline-grid;
    place-items: center;
    flex: none;
    inline-size: 20px;
    block-size: 16px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--pb-tone) 18%, transparent);
    color: color-mix(in srgb, var(--pb-tone) 65%, var(--text));
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0;
  }
</style>
