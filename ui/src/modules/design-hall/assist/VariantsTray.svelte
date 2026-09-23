<script lang="ts">
  // One variants run as equal cards (patterns.md §3): a live sandboxed preview
  // of each variant version, its direction name and one line on how it
  // differs, then the same three actions on every card — Apply (fast-forward
  // main; the person's explicit pick), Compare (against the current version)
  // and Reject, which asks why (the reason feeds the learning loop). Nothing
  // is pre-applied: the head stays where it is until a person applies one.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { DesignArtifact, DesignVariantRun } from '../../../lib/api/types';
  import ArtifactThumb from '../ArtifactThumb.svelte';
  import { directionBlurb, directionName, variantCards, type VariantCard } from './model';

  interface Props {
    artifact: DesignArtifact;
    run: DesignVariantRun;
    /** version id → the reason chip a person rejected it with. */
    rejected: Record<string, string>;
    /** The variant version being applied right now. */
    applying: string | null;
    readonly: boolean;
    onapply: (card: VariantCard) => void;
    oncompare: (card: VariantCard) => void;
    onreject: (e: MouseEvent, card: VariantCard) => void;
  }
  let { artifact, run, rejected, applying, readonly, onapply, oncompare, onreject }: Props = $props();

  const cards = $derived(variantCards(run));
  const letter = (k: number) => String.fromCharCode(64 + Math.min(26, Math.max(1, k)));
  const decided = $derived(!!run.accepted_version_id);
</script>

<div class="tray" role="group" aria-label="Variants" data-testid="design-variants-tray">
  {#each cards as c (c.k)}
    {@const rej = c.version ? rejected[c.version.id] : undefined}
    {@const name = directionName(c.direction)}
    <article
      class="card"
      class:accepted={c.state === 'accepted'}
      class:passed={c.state === 'passed' || !!rej}
      aria-label={`Variant ${letter(c.k)}: ${name}`}
      data-testid="design-variant-card"
      data-state={c.state}
    >
      <div class="pic">
        <span class="k" aria-hidden="true">{letter(c.k)}</span>
        {#if c.version}
          <ArtifactThumb {artifact} versionId={c.version.id} live compact />
        {:else if c.state === 'running'}
          <div class="drawing" role="status"><span class="pulse" aria-hidden="true"></span> Drawing…</div>
        {:else}
          <div class="drawing failed"><Icon name="warning" size={12} /> No result</div>
        {/if}
      </div>
      <div class="body">
        <span class="name">{name}</span>
        <span class="blurb" title={c.summary ?? directionBlurb(c.direction)}>{c.summary ?? directionBlurb(c.direction)}</span>
        {#if c.state === 'accepted'}
          <span class="state ok"><Icon name="check" size={11} /> Applied</span>
        {:else if rej}
          <span class="state">Rejected · {rej}</span>
        {:else if c.state === 'passed'}
          <span class="state">Not picked</span>
        {:else if c.state === 'failed'}
          <span class="state bad">{c.turn?.error ? 'Failed' : 'Couldn’t draw this one'}</span>
        {/if}
      </div>
      <div class="acts">
        <button
          class="btn small"
          disabled={readonly || decided || !c.version || applying !== null}
          onclick={() => onapply(c)}
          title={decided ? 'A variant of this set was already applied' : `Apply ${name} as a new version`}
          data-testid="design-variant-apply"
        >
          {applying && applying === c.version?.id ? 'Applying…' : 'Apply'}
        </button>
        <button class="icon-btn" disabled={!c.version} onclick={() => oncompare(c)}
          aria-label={`Compare ${name} with the current version`} title="Compare with the current version" data-testid="design-variant-compare">
          <Icon name="columns" size={12} />
        </button>
        <button class="icon-btn" disabled={readonly || decided || !c.version || !!rej} onclick={(e) => onreject(e, c)}
          aria-label={`Reject ${name} — tell Otto why`} title="Not this one — tell Otto why" aria-haspopup="menu" data-testid="design-variant-reject">
          <Icon name="x" size={12} />
        </button>
      </div>
    </article>
  {/each}
</div>

<style>
  .tray {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(118px, 1fr));
    gap: 8px;
  }
  .card {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    overflow: hidden;
  }
  .card.accepted {
    border-color: var(--success);
    box-shadow: 0 0 0 1px var(--success);
  }
  .card.passed .pic,
  .card.passed .body {
    opacity: 0.6;
  }
  .pic {
    position: relative;
    aspect-ratio: 4 / 3;
    border-block-end: 1px solid var(--border);
    background: var(--surface-2);
  }
  .k {
    position: absolute;
    inset-block-start: 4px;
    inset-inline-start: 4px;
    z-index: 1;
    min-width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 700;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
  }
  .drawing {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .drawing.failed {
    color: var(--danger);
  }
  .pulse {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-working);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px 4px;
    min-width: 0;
  }
  .name {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .blurb {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .state.ok {
    color: var(--success);
  }
  .state.bad {
    color: var(--danger);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px 6px;
    margin-block-start: auto;
  }
  .acts .btn {
    flex: 1;
    justify-content: center;
  }
</style>
