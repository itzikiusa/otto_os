<script lang="ts">
  // Extracted from Studio3D.svelte: "Optimize for web (meshopt)" modal.
  import { announceShown } from '../../../lib/announce.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { budgetStatus, formatBytes, glbFileName } from '../../../lib/design/scene3d';
  import type { OptimizeResult } from '../../../lib/design/scene3d/optimize';

  export interface OptimizeState {
    phase: 'running' | 'done' | 'error';
    result?: OptimizeResult;
    error?: string;
  }

  interface Props {
    opt: OptimizeState;
    title: string;
    readonly: boolean;
    saving: boolean;
    onclose: () => void;
    ondownload: (bytes: Uint8Array, fileName: string) => void;
    onsave: () => void;
  }
  let { opt, title, readonly, saving, onclose, ondownload, onsave }: Props = $props();
</script>

<Modal title="Optimize for web" width={500} {onclose}>
  <div class="opt" data-testid="s3d-optimize">
    {#if opt.phase === 'running'}
      <p class="dim">Exporting the scene and compressing it with gltf-transform + meshopt…</p>
    {:else if opt.phase === 'error'}
      <p class="err" use:announceShown={{ text: `Couldn’t optimize: ${opt.error ?? ''}`.trim(), assertive: true }}>
        Couldn’t optimize: {opt.error}
      </p>
    {:else if opt.result}
      {@const r = opt.result}
      {@const b = budgetStatus(r.after)}
      <div class="sizes">
        <div><span class="k">Before</span><strong>{formatBytes(r.before)}</strong></div>
        <Icon name="chevronRight" size={14} />
        <div><span class="k">After</span><strong>{formatBytes(r.after)}</strong></div>
        <div><span class="k">Saved</span><strong>{r.before > 0 ? Math.max(0, Math.round((1 - r.after / r.before) * 100)) : 0}%</strong></div>
      </div>
      <div class="budget {b.tone}" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.min(100, b.pct)} aria-label="Web budget used">
        <span class="bar" style:inline-size="{Math.min(100, b.pct)}%"></span>
      </div>
      <p class="dim">Web budget: {b.label}{b.tone === 'over' ? ' — simplify the scene or split it before embedding.' : '.'}</p>
      <ul class="steps">
        {#each r.steps as s (s)}<li><Icon name="check" size={12} /> {s}</li>{/each}
      </ul>
      <p class="dim">Draco isn’t offered: its encoder isn’t bundled and Otto stays offline. Every Otto viewer decodes meshopt.</p>
    {/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose}>Close</button>
    {#if opt.phase === 'done' && opt.result}
      <button type="button" class="btn" onclick={() => opt.result && ondownload(opt.result.bytes, glbFileName(`${title}-web`))}>Download</button>
      <button type="button" class="btn primary" disabled={readonly || saving} onclick={onsave}>
        {saving ? 'Saving…' : 'Save as GLB design'}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .opt {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .dim {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .sizes {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .sizes div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sizes strong {
    font-size: var(--fs-l);
    font-variant-numeric: tabular-nums;
  }
  .budget {
    block-size: 8px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .budget .bar {
    display: block;
    block-size: 100%;
    background: var(--success);
  }
  .budget.warn .bar {
    background: var(--warning);
  }
  .budget.over .bar {
    background: var(--danger);
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
