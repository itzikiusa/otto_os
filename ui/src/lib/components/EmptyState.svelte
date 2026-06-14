<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  interface Props {
    icon?: string;
    title: string;
    body?: string;
    actionLabel?: string;
    onaction?: () => void;
    children?: Snippet;
  }
  let { icon = 'box', title, body, actionLabel, onaction, children }: Props = $props();
</script>

<div class="empty">
  <div class="empty-icon"><Icon name={icon} size={28} /></div>
  <h3>{title}</h3>
  {#if body}<p>{body}</p>{/if}
  {#if actionLabel && onaction}
    <button class="btn primary" onclick={onaction}>{actionLabel}</button>
  {/if}
  {#if children}{@render children()}{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
    color: var(--text-dim);
  }
  .empty-icon {
    width: 56px;
    height: 56px;
    border-radius: var(--radius-l);
    background: var(--surface-2);
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    margin-bottom: 4px;
    color: var(--text-dim);
  }
  h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  p {
    margin: 0;
    font-size: 12.5px;
    max-width: 360px;
    line-height: 1.5;
  }
  button {
    margin-top: 8px;
  }
</style>
