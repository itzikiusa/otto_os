<script lang="ts">
  // Brand Kit → Voice / Imagery: a one-line summary and paired do / don't
  // examples (≤ 20 each). Agents read these in every design turn's brief, so
  // concrete examples beat adjectives.
  import type { BrandDoc, BrandProse } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';

  interface Props {
    doc: BrandDoc;
    group: 'voice' | 'imagery';
    readonly?: boolean;
  }
  let { doc = $bindable(), group, readonly = false }: Props = $props();

  const COPY = {
    voice: {
      title: 'Voice',
      meta: 'How the brand sounds — agents write copy from these examples',
      summary: 'Warm, confident, never salesy.',
      doPh: 'Earn on every purchase, spend anywhere.',
      dontPh: 'Unlock INCREDIBLE savings NOW!!!',
    },
    imagery: {
      title: 'Imagery',
      meta: 'What pictures to use, and what to avoid',
      summary: 'Product renders and bold type over stock photography.',
      doPh: 'Product renders with soft light',
      dontPh: 'Stock photos of people shopping',
    },
  } as const;
  const c = $derived(COPY[group]);
  const prose = $derived<BrandProse>(doc[group] ?? {});
  const rows = $derived(Math.max(prose.do?.length ?? 0, prose.dont?.length ?? 0));

  function ensure(): BrandProse {
    if (!doc[group]) doc[group] = { summary: '', do: [], dont: [] };
    return doc[group]!;
  }

  function setSummary(v: string): void {
    ensure().summary = v;
  }

  function setLine(kind: 'do' | 'dont', i: number, v: string): void {
    const p = ensure();
    const list = [...(p[kind] ?? [])];
    while (list.length <= i) list.push('');
    list[i] = v;
    p[kind] = list;
  }

  function addPair(): void {
    const p = ensure();
    const n = rows;
    const d = [...(p.do ?? [])];
    const x = [...(p.dont ?? [])];
    while (d.length <= n) d.push('');
    while (x.length <= n) x.push('');
    p.do = d.slice(0, 20);
    p.dont = x.slice(0, 20);
  }

  function removePair(i: number): void {
    const p = ensure();
    p.do = (p.do ?? []).filter((_, j) => j !== i);
    p.dont = (p.dont ?? []).filter((_, j) => j !== i);
  }
</script>

<section id="brand-{group}" class="bsec" aria-labelledby="brand-{group}-h">
  <div class="sec-h">
    <h2 id="brand-{group}-h">{c.title}</h2>
    <span class="meta">{c.meta}</span>
  </div>

  <div class="card" data-testid="brand-{group}">
    {#if readonly}
      <p class="vq" class:big={group === 'voice'}>{prose.summary || '—'}</p>
    {:else}
      <textarea
        class="vq input"
        class:big={group === 'voice'}
        rows={group === 'voice' ? 1 : 2}
        value={prose.summary ?? ''}
        oninput={(e) => setSummary(e.currentTarget.value)}
        placeholder={c.summary}
        aria-label="{c.title} summary"
        maxlength="2000"
      ></textarea>
    {/if}

    <div class="dd" role="table" aria-label="{c.title} do and don’t examples">
      <div class="ddrow head" role="row">
        <span role="columnheader"><Icon name="check" size={12} /> Do</span>
        <span role="columnheader"><Icon name="x" size={12} /> Don’t</span>
      </div>
      {#each Array(rows) as _, i (i)}
        <div class="ddrow" role="row">
          <span class="cell do" role="cell">
            <Icon name="check" size={12} />
            <input class="line" value={prose.do?.[i] ?? ''} oninput={(e) => setLine('do', i, e.currentTarget.value)} disabled={readonly} placeholder={c.doPh} aria-label="Do example {i + 1}" maxlength="2000" />
          </span>
          <span class="cell dont" role="cell">
            <Icon name="x" size={12} />
            <input class="line" value={prose.dont?.[i] ?? ''} oninput={(e) => setLine('dont', i, e.currentTarget.value)} disabled={readonly} placeholder={c.dontPh} aria-label="Don’t example {i + 1}" maxlength="2000" />
            {#if !readonly}
              <button class="icon-btn" aria-label="Remove example {i + 1}" title="Remove" onclick={() => removePair(i)}><Icon name="trash" size={12} /></button>
            {/if}
          </span>
        </div>
      {/each}
    </div>
    {#if !readonly && rows < 20}
      <button class="btn small add" onclick={addPair}><Icon name="plus" size={12} /> Add example</button>
    {/if}
  </div>
</section>

<style>
  .bsec {
    display: flex;
    flex-direction: column;
    gap: 12px;
    scroll-margin-top: 12px;
  }
  .sec-h {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .meta {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .vq {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text);
    resize: vertical;
    min-height: 0;
    height: auto;
    width: 100%;
    font-family: inherit;
    line-height: 1.45;
    padding: 6px 8px;
  }
  .vq.big {
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .dd {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ddrow {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .ddrow.head span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    min-width: 0;
  }
  .cell.do {
    background: var(--success-soft);
    color: var(--success);
  }
  .cell.dont {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .line {
    flex: 1;
    min-width: 0;
    height: 24px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    outline: none;
  }
  .line:focus-visible {
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 70%, transparent);
    border-radius: var(--radius-s);
  }
  .add {
    align-self: flex-start;
  }
  @media (max-width: 720px) {
    .ddrow {
      grid-template-columns: 1fr;
    }
  }
</style>
