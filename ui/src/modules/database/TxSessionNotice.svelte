<script lang="ts">
  // Inline notice above the results after a run that used transaction control
  // the way a desktop SQL client would — a lone BEGIN / COMMIT / ROLLBACK, or a
  // batch that leaves a transaction open. Each run has its own session here
  // (see `tx-shape.ts`), so that transaction never spanned runs; say so where
  // the result is, not in a toast that vanishes. MySQL / PostgreSQL only.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import type { DbEngine } from '../../lib/api/types';
  import { countStatements } from '../../lib/database/sql-util';
  import { splitModeFor } from '../../lib/database/sql-dialect';
  import { txNotice, txShape } from './tx-shape';

  interface Props {
    /** The statement that produced what is on screen (ran / errored). */
    statement: string | null | undefined;
    engine: DbEngine | null | undefined;
    /** Statements in the editor buffer now — "Run all" shows when it holds
     *  more than ran. */
    bufferCount: number;
    running: boolean;
    onrunall: () => void;
  }
  let { statement, engine, bufferCount, running, onrunall }: Props = $props();

  const sqlEngine = $derived(engine === 'mysql' || engine === 'postgres' ? engine : null);
  const shape = $derived(sqlEngine ? txShape(statement, sqlEngine) : null);
  const show = $derived(!!shape && (shape.controlOnly || shape.unclosed));
  const ranCount = $derived(show && statement ? countStatements(statement, splitModeFor(sqlEngine)) : 0);
  /** Dismissed for exactly this statement text; a different run shows it again. */
  let dismissedFor = $state<string | null>(null);
</script>

{#if show && shape && sqlEngine && !running && dismissedFor !== statement}
  <div class="tx-notice">
    <Icon name="info" size={14} />
    <p use:announceShown={txNotice(shape, sqlEngine)}>{txNotice(shape, sqlEngine)}</p>
    <div class="tx-actions">
      {#if bufferCount > ranCount}
        <button class="btn small" onclick={onrunall} title="Run every statement in the editor as one batch on one session (⇧⌘↵)">Run all</button>
      {/if}
      <button
        class="btn small ghost"
        onclick={() => (dismissedFor = statement ?? null)}
        aria-label="Dismiss"
        title="Dismiss"
      ><Icon name="x" size={12} /></button>
    </div>
  </div>
{/if}

<style>
  .tx-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-bottom: 8px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    color: var(--text);
    border: 1px solid color-mix(in srgb, var(--warning) 45%, transparent);
    background: var(--warning-soft);
    border-radius: var(--radius-s);
  }
  .tx-notice > :global(svg) {
    flex: none;
    margin-block-start: 2px;
    color: var(--warning);
  }
  .tx-notice p {
    flex: 1;
    min-width: 0;
    margin: 0;
    line-height: 1.45;
  }
  .tx-actions {
    display: flex;
    gap: 6px;
    flex: none;
  }
</style>
