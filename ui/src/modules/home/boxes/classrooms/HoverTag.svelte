<script lang="ts">
  // Otto School — the hover tooltip over the 3D stage (ClassroomsBox): a
  // kid's title, provider and state with a crisp peek at its live terminal,
  // a door's room summary, or the headmaster. The box positions it (it owns
  // the projection), so the element is bound back.
  import { now } from '../../../../lib/stores/now.svelte';
  import { relTime } from '../../../mission-control/lib';
  import { isAsleep, kidActivity, providerLabel, roomSummary, type Kid, type Room } from '../../school/model';
  import type { SchoolPick } from '../../school/scene';

  interface Props {
    hover: SchoolPick | null;
    hoverKid: Kid | null;
    hoverRoom: Room | null;
    /** The hovered kid's last terminal rows (null = no peek). */
    peek: string[] | null;
    el?: HTMLElement | null;
  }
  let { hover, hoverKid, hoverRoom, peek, el = $bindable(null) }: Props = $props();
</script>

<div class="hover-tag" class:wide={!!peek} bind:this={el}>
  {#if hoverKid}
    <b>{hoverKid.title}</b>
    <small>{providerLabel(hoverKid.provider)} · {kidActivity(hoverKid, relTime(hoverKid.lastActiveAt), isAsleep(hoverKid, now()))}</small>
    {#if peek}
      <pre class="peek">{peek.length ? peek.join('\n') : 'No output yet'}</pre>
    {/if}
  {:else if hoverRoom}
    <b>{hoverRoom.name}</b><small>{roomSummary(hoverRoom)} · click to walk in</small>
  {:else if hover?.kind === 'head'}
    <b>Otto</b><small>the headmaster</small>
  {:else if hover?.kind === 'door'}
    <b>Back to the corridor</b>
  {/if}
</div>

<style>
  .hover-tag {
    position: absolute;
    top: 0;
    left: 0; /* ui-guards: allow — origin of projected screen coordinates, not layout */
    visibility: hidden;
    white-space: nowrap;
  }
  .hover-tag {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-xs);
    max-width: 260px;
  }
  .hover-tag:empty {
    display: none;
  }
  .hover-tag b {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hover-tag small {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hover-tag.wide {
    max-width: min(460px, 90%);
    align-items: stretch;
  }
  .hover-tag.wide b {
    max-width: none;
  }
  /* The zoom lens on a kid's monitor: its last terminal rows as real text. */
  .peek {
    margin: 4px 0 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--term-bg);
    border: 1px solid var(--border);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.35;
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
