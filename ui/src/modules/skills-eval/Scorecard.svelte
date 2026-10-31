<script lang="ts">
  // Renders one iteration's multi-signal eval score (composite + the five
  // signals that feed it). Read-only — it never mutates the eval; rating happens elsewhere.
  import type { EvalScore } from '../../lib/api/types';
  import { sentenceCase } from '../../lib/status';

  interface Props {
    score?: EvalScore | null;
    evalId?: string;
    iterId?: string;
    compact?: boolean;
  }
  let { score }: Props = $props();

  // Proof-status pill palette: soft tone tint + text-safe tone, per the
  // design tokens (passed/failed/partial/waived; anything else = neutral).
  const grey = { bg: 'var(--surface-2)', fg: 'var(--text-dim)' };
  const PROOF: Record<string, { bg: string; fg: string }> = {
    passed: { bg: 'var(--success-soft)', fg: 'var(--success)' },
    failed: { bg: 'var(--danger-soft)', fg: 'var(--danger)' },
    partial: { bg: 'var(--warning-soft)', fg: 'var(--warning)' },
    waived: { bg: 'var(--accent-soft)', fg: 'var(--accent-text)' },
  };
  const proofTone = $derived(score ? (PROOF[score.proof_status] ?? grey) : grey);

  // Same bands as the score chips in RunDetail / CompareView (85 / 60), so a
  // bar never reads green next to an amber chip for the same number.
  function barColor(s: number): string {
    if (s >= 85) return 'var(--status-working)';
    if (s >= 60) return 'var(--status-warn)';
    return 'var(--status-exited)';
  }

  type Row = { label: string; score: number; detail: string; ran: boolean };
  const rows = $derived<Row[]>(
    score
      ? [
          { label: 'Tests', score: score.tests.score, detail: score.tests.detail, ran: score.tests.ran },
          { label: 'Lint', score: score.lint.score, detail: score.lint.detail, ran: score.lint.ran },
          { label: 'Diff', score: score.diff.score, detail: score.diff.detail, ran: score.diff.ran },
          { label: 'Review', score: score.review.score, detail: score.review.detail, ran: score.review.ran },
          { label: 'Human', score: score.human.score, detail: score.human.note, ran: score.human.rating != null },
        ]
      : [],
  );

  function clamp(n: number): number {
    return Math.max(0, Math.min(100, n));
  }
</script>

{#if score}
  <div class="scorecard" data-testid="scorecard">
    <div class="head">
      <div class="composite">
        <span class="num" data-testid="scorecard-composite">{score.composite.toFixed(0)}</span>
        <span class="lbl">{score.proof_status === 'pending' ? 'Provisional composite' : 'Composite'}</span>
      </div>
      <div class="meta">
        <span
          class="proof"
          data-testid="scorecard-proof"
          style="background:{proofTone.bg};color:{proofTone.fg}"
          title="Verification status"
        >
          Proof: {sentenceCase(score.proof_status || 'missing')}
        </span>
        <span class="done" title="How much of the done contract (tests, lint) this iteration met">Done contract {score.done_score}/100</span>
      </div>
    </div>

    <div class="signals">
      {#each rows as r (r.label)}
        <div class="row" class:dim={!r.ran}>
          <span class="rlabel">{r.label}</span>
          {#if r.ran}
            <span class="track">
              <span class="fill" style="width:{clamp(r.score)}%;background:{barColor(r.score)}"></span>
            </span>
            <span class="rscore">{r.score.toFixed(0)}</span>
          {:else}
            <span class="notrun">{r.label === 'Human' ? 'Not rated' : 'Not run'}</span>
            <span class="rscore">—</span>
          {/if}
          {#if r.detail}<span class="rdetail" title={r.detail}>{r.detail}</span>{/if}
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  /* Dense single-line rules to keep the component compact; values mirror the
     repo's spacing/type scale and CSS custom properties. */
  .scorecard { display: flex; flex-direction: column; gap: 10px; padding: 10px 12px; border: 1px solid var(--border); border-radius: var(--radius-m); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .composite { display: flex; align-items: baseline; gap: 6px; }
  .composite .num { font-size: var(--fs-2xl); font-weight: 600; line-height: 1; color: var(--text); font-variant-numeric: tabular-nums; }
  .composite .lbl, .done, .rlabel { font-size: var(--fs-xs); color: var(--text-dim); }
  .composite .lbl { text-transform: uppercase; letter-spacing: .06em; }
  .meta { display: flex; align-items: center; gap: 8px; }
  .proof { padding: 2px 8px; border-radius: 999px; font-size: var(--fs-xs); font-weight: 500; white-space: nowrap; }
  .done { font-variant-numeric: tabular-nums; }
  .signals { display: flex; flex-direction: column; gap: 4px; }
  .row { display: grid; grid-template-columns: 54px 1fr 28px; align-items: center; gap: 8px; }
  .row.dim { opacity: 0.55; }
  .track { position: relative; height: 7px; border-radius: 999px; overflow: hidden; background: var(--surface-2); }
  .fill { display: block; height: 100%; border-radius: 999px; }
  /* Replaces the bar (a 7px track can't hold readable text). */
  .notrun { font-size: var(--fs-xs); line-height: 1; color: var(--text-dim); }
  .rscore { font-size: var(--fs-xs); font-weight: 600; text-align: end; color: var(--text); font-variant-numeric: tabular-nums; }
  .row.dim .rscore { color: var(--text-dim); }
  .rdetail { grid-column: 2 / -1; font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
