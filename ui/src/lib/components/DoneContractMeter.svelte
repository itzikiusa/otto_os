<script lang="ts">
  // The "done contract" (R8): an explainable readiness score rendered as a ring
  // plus an itemized checklist. Each item shows satisfied ✓ / missing ✗, its
  // label, a required/optional tag, and the human-readable detail text.
  import Icon from './Icon.svelte';
  import type { DoneContract } from '../api/types';

  interface Props {
    contract: DoneContract;
  }
  let { contract }: Props = $props();

  const score = $derived(Math.max(0, Math.min(100, Math.round(contract.score))));

  type Tone = 'ok' | 'warn' | 'bad';
  const tone = $derived<Tone>(score >= 80 ? 'ok' : score >= 50 ? 'warn' : 'bad');

  // Ring geometry: r=26 → circumference ≈ 163.36; the dash offset draws the arc.
  const R = 26;
  const CIRC = 2 * Math.PI * R;
  const dash = $derived((score / 100) * CIRC);
</script>

<section class="meter" aria-label="Done contract">
  <div class="ring-wrap {tone}">
    <svg viewBox="0 0 64 64" class="ring" role="img" aria-label={`Done score ${score} of 100`}>
      <circle class="track" cx="32" cy="32" r={R} fill="none" stroke-width="7" />
      <circle
        class="value"
        cx="32"
        cy="32"
        r={R}
        fill="none"
        stroke-width="7"
        stroke-linecap="round"
        stroke-dasharray={`${dash} ${CIRC}`}
        transform="rotate(-90 32 32)"
      />
    </svg>
    <div class="ring-label">
      <span class="num">{score}</span>
      <span class="den">/100</span>
    </div>
  </div>

  <div class="contract">
    <div class="contract-head">
      <span class="title">Done contract</span>
      <span class="dim">{contract.satisfied}/{contract.required} required met</span>
    </div>
    {#if contract.items.length === 0}
      <p class="dim empty">No contract items.</p>
    {:else}
      <ul class="items">
        {#each contract.items as it (it.key)}
          <li class="item" class:ok={it.satisfied} class:miss={!it.satisfied}>
            <span class="mark" aria-hidden="true">
              <Icon name={it.satisfied ? 'check' : 'x'} size={11} />
            </span>
            <div class="item-body">
              <div class="item-top">
                <span class="item-label">{it.label}</span>
                <span class="tag {it.required ? 'req' : 'opt'}">{it.required ? 'required' : 'optional'}</span>
              </div>
              {#if it.detail}
                <span class="item-detail">{it.detail}</span>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</section>

<style>
  .meter {
    display: flex;
    gap: 16px;
    align-items: flex-start;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 14px 16px;
    margin-bottom: 14px;
  }
  .ring-wrap {
    position: relative;
    width: 64px;
    height: 64px;
    flex: none;
    color: var(--text-dim);
  }
  .ring {
    width: 64px;
    height: 64px;
  }
  .track {
    stroke: color-mix(in srgb, var(--text-dim) 16%, transparent);
  }
  .value {
    stroke: currentColor;
    transition: stroke-dasharray 240ms ease-out;
  }
  .ring-wrap.ok {
    color: var(--status-working);
  }
  .ring-wrap.warn {
    color: var(--status-warn);
  }
  .ring-wrap.bad {
    color: var(--status-exited);
  }
  .ring-label {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--text);
  }
  .ring-label .num {
    font-size: 17px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .ring-label .den {
    font-size: 9px;
    color: var(--text-dim);
  }
  .contract {
    flex: 1;
    min-width: 0;
  }
  .contract-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 8px;
  }
  .contract-head .title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .dim {
    color: var(--text-dim);
    font-size: 11px;
  }
  .empty {
    margin: 0;
  }
  .items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .item {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .mark {
    flex: none;
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    margin-top: 1px;
  }
  .item.ok .mark {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .item.miss .mark {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
  .item-body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .item-top {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
  }
  .item-label {
    font-size: 12.5px;
    font-weight: 500;
  }
  .item.miss .item-label {
    color: var(--text-dim);
  }
  .tag {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-radius: 999px;
    padding: 0 6px;
    line-height: 14px;
    border: 1px solid transparent;
  }
  .tag.req {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .tag.opt {
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .item-detail {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
    word-break: break-word;
  }
</style>
