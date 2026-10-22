<script lang="ts">
  // Otto School — the selected kid's card (ClassroomsBox, room / screen
  // view): its state lines and every action as a button — open, look at the
  // screen, send the headmaster over, detention, kick out, release.
  import Icon from '../../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../../lib/components/ProviderIcon.svelte';
  import { now } from '../../../../lib/stores/now.svelte';
  import { relTime } from '../../../mission-control/lib';
  import { kidLines, type Kid } from '../../school/model';
  import type { SchoolHandle, SchoolView } from '../../school/scene';

  interface Props {
    selKid: Kid;
    view: SchoolView;
    handle: SchoolHandle | null;
    /** The card element (the box keeps it in view). */
    el?: HTMLElement | null;
    onclose: () => void;
    open: (k: Kid) => Promise<void>;
    free: (k: Kid) => Promise<void>;
    checkOn: (k: Kid) => void;
    detention: (k: Kid) => Promise<void>;
    kick: (k: Kid) => Promise<void>;
  }
  let { selKid, view, handle, el = $bindable(null), onclose, open, free, checkOn, detention, kick }: Props = $props();
</script>

<div class="kid-card" role="group" aria-label="{selKid.title} — details" bind:this={el}>
  <div class="card-head">
    <ProviderIcon provider={selKid.provider} size={16} />
    <b class="ellipsis">{selKid.title}</b>
    <button class="icon-btn" onclick={onclose} aria-label="Close details" title="Close details"><Icon name="x" size={12} /></button>
  </div>
  {#each kidLines(selKid, now() ? relTime(selKid.lastActiveAt) : '') as line, i (i)}
    <div class="card-line" class:needs={i === 0 && selKid.pose === 'needs-you'}>{line}</div>
  {/each}
  <div class="card-actions">
    {#if selKid.detention}
      <button class="btn small primary" disabled={!selKid.canManage} onclick={() => selKid && void free(selKid)}>Release from detention</button>
    {:else}
      <button class="btn small primary" onclick={() => void open(selKid)}>Open session</button>
      {#if view.kind === 'screen'}
        <button class="btn small" onclick={() => void handle?.backToRoom()}>Back to the room</button>
      {:else}
        <button class="btn small" onclick={() => selKid && void handle?.lookAtScreen(selKid.id)}>Look at screen</button>
      {/if}
      <button class="btn small" onclick={() => selKid && checkOn(selKid)} title="Send the headmaster over">Check on</button>
      {#if selKid.canManage}
        <button class="btn small" onclick={() => selKid && void detention(selKid)} title="Archive — resumable, with Undo">Detention</button>
        <button class="btn small danger" onclick={() => selKid && void kick(selKid)} title="Delete this session and its history">Kick out…</button>
      {/if}
    {/if}
  </div>
</div>

<style>
  .kid-card {
    position: absolute;
    inset-inline-end: 8px;
    top: 8px;
    width: min(320px, calc(100% - 16px));
    max-height: calc(100% - 16px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .card-head b {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-m);
  }
  .card-line {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .card-line.needs {
    color: var(--warning);
    font-weight: 600;
  }
  .card-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 4px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
