<!--
  Under the composer's error line when "New context" was refused because
  background work still runs: one click stops that work and starts fresh
  (the send carries `stop_background: true`). A daemon that ignores the flag
  refuses again, and then only the reason shows — never a button that loops.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { backgroundLine } from './taskStop';

  interface Props {
    show: boolean;
    /** Background tasks running now. */
    count: number;
    onstop: () => void;
  }
  let { show, count, onstop }: Props = $props();
</script>

{#if show}
  <div class="rfs" data-testid="runtime-fresh-stop-bg">
    <span class="rfs-text">{backgroundLine(count)} — a fresh start needs {count === 1 ? 'it' : 'them'} stopped.</span>
    <button type="button" class="btn small rfs-btn" title="Stop the background work, then start the fresh conversation" onclick={onstop}>
      <Icon name="stop" size={12} />Stop background work and start fresh
    </button>
  </div>
{/if}

<style>
  .rfs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rfs-text {
    min-inline-size: 0;
    overflow-wrap: anywhere;
  }
  .rfs-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
