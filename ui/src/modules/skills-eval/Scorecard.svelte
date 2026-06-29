<script lang="ts">
  // Renders one iteration's multi-signal eval score (composite + the five
  // signals that feed it) and, on demand, the assembled proof pack. Read-only —
  // it never mutates the eval; rating happens elsewhere.
  import type { EvalScore } from '../../lib/api/types';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    score?: EvalScore | null;
    evalId?: string;
    iterId?: string;
    compact?: boolean;
  }
  let { score, evalId, iterId, compact = false }: Props = $props();

  type Artifact = {
    kind: string;
    title: string;
    status: string;
    preview: string;
    truncated: boolean;
    metadata: unknown;
  };

  let expanded = $state(false);
  let loading = $state(false);
  let loaded = $state(false);
  let artifacts = $state<Artifact[]>([]);

  // Proof-status pill palette: passed=light-green, failed=red, partial=amber,
  // waived=accent-blue, anything else (missing/empty)=dim grey.
  const grey = { bg: 'var(--text-dim)', fg: '#000' };
  const PROOF: Record<string, { bg: string; fg: string }> = {
    passed: { bg: '#7ee787', fg: '#000' },
    failed: { bg: '#d66', fg: '#fff' },
    partial: { bg: '#d8a657', fg: '#000' },
    waived: { bg: 'var(--accent)', fg: '#fff' },
  };
  const proofTone = $derived(score ? (PROOF[score.proof_status] ?? grey) : grey);

  function barColor(s: number): string {
    if (s >= 80) return '#7ee787';
    if (s >= 50) return '#d8a657';
    return '#d66';
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

  function dotTone(status: string): string {
    if (status === 'passed') return '#7ee787';
    if (status === 'failed') return '#d66';
    return 'var(--text-dim)';
  }

  function clamp(n: number): number {
    return Math.max(0, Math.min(100, n));
  }

  function clip(s: string): string {
    return s.length > 400 ? s.slice(0, 400) : s;
  }

  async function toggleProof(): Promise<void> {
    if (expanded) {
      expanded = false;
      return;
    }
    expanded = true;
    if (loaded || !evalId || !iterId) return;
    loading = true;
    try {
      const pack = await skillsEvalApi.iterProofPack(evalId, iterId);
      artifacts = pack.artifacts ?? [];
      loaded = true;
    } catch (e) {
      toasts.error('Proof pack', e instanceof Error ? e.message : String(e));
      expanded = false;
    } finally {
      loading = false;
    }
  }
</script>

{#if score}
  <div class="scorecard" data-testid="scorecard">
    <div class="head">
      <div class="composite">
        <span class="num" data-testid="scorecard-composite">{score.composite.toFixed(0)}</span>
        <span class="lbl">Composite</span>
      </div>
      <div class="meta">
        <span
          class="proof"
          data-testid="scorecard-proof"
          style="background:{proofTone.bg};color:{proofTone.fg}"
        >
          {(score.proof_status || 'missing').toUpperCase()}
        </span>
        <span class="done">done {score.done_score}</span>
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
            <span class="track"><span class="notrun">not run</span></span>
            <span class="rscore">—</span>
          {/if}
          {#if r.detail}<span class="rdetail" title={r.detail}>{r.detail}</span>{/if}
        </div>
      {/each}
    </div>

    {#if !compact}
      <button class="proofpack-btn" data-testid="scorecard-proofpack-btn" onclick={toggleProof}>
        <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={13} />
        View proof pack
      </button>

      {#if expanded}
        <div class="pack">
          {#if loading}
            <div class="pmsg">Loading…</div>
          {:else if artifacts.length === 0}
            <div class="pmsg">No proof artifacts.</div>
          {:else}
            {#each artifacts as a, i (i)}
              <div class="artifact">
                <div class="ahead">
                  <span class="adot" style="background:{dotTone(a.status)}"></span>
                  <span class="akind">{a.kind}</span>
                  <span class="sep">·</span>
                  <span class="atitle">{a.title}</span>
                  <span class="sep">·</span>
                  <span class="astatus">{a.status}</span>
                </div>
                {#if a.preview}
                  <pre class="apreview">{clip(a.preview)}{a.truncated ? '\n…' : ''}</pre>
                {/if}
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  /* Dense single-line rules to keep the component compact; values mirror the
     repo's spacing/type scale and CSS custom properties. */
  .scorecard { display: flex; flex-direction: column; gap: 10px; padding: 10px 12px; border: 1px solid var(--border); border-radius: var(--radius-m); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .composite { display: flex; align-items: baseline; gap: 6px; }
  .composite .num { font-size: 28px; font-weight: 700; line-height: 1; color: var(--text); font-variant-numeric: tabular-nums; }
  .composite .lbl, .done, .rlabel, .pmsg { font-size: 11px; color: var(--text-dim); }
  .composite .lbl { text-transform: uppercase; letter-spacing: 0.04em; }
  .meta { display: flex; align-items: center; gap: 8px; }
  .proof { padding: 2px 8px; border-radius: 999px; font-size: 10px; font-weight: 700; letter-spacing: 0.03em; white-space: nowrap; }
  .done { font-variant-numeric: tabular-nums; }
  .signals { display: flex; flex-direction: column; gap: 5px; }
  .row { display: grid; grid-template-columns: 54px 1fr 28px; align-items: center; gap: 8px; }
  .row.dim { opacity: 0.55; }
  .track { position: relative; height: 7px; border-radius: 999px; overflow: hidden; background: var(--surface-2, color-mix(in srgb, var(--text-dim) 18%, transparent)); }
  .fill { display: block; height: 100%; border-radius: 999px; }
  .notrun { position: absolute; left: 6px; top: -4px; font-size: 9px; color: var(--text-dim); }
  .rscore { font-size: 11px; font-weight: 600; text-align: right; color: var(--text); font-variant-numeric: tabular-nums; }
  .row.dim .rscore { color: var(--text-dim); }
  .rdetail { grid-column: 2 / -1; font-size: 10px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .proofpack-btn { align-self: flex-start; display: inline-flex; align-items: center; gap: 4px; padding: 3px 8px; font-size: 11px; color: var(--text-dim); background: transparent; border: 1px solid var(--border); border-radius: var(--radius-m); cursor: pointer; }
  .proofpack-btn:hover { color: var(--text); border-color: var(--accent); }
  .pack { display: flex; flex-direction: column; gap: 8px; }
  .artifact { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 6px 8px; }
  .ahead { display: flex; align-items: center; gap: 5px; font-size: 11px; color: var(--text); }
  .adot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .akind { font-weight: 600; }
  .sep, .atitle, .astatus { color: var(--text-dim); }
  .astatus { text-transform: uppercase; letter-spacing: 0.02em; }
  .apreview { margin: 6px 0 0; padding: 6px 8px; max-height: 160px; overflow: auto; font-size: 10px; line-height: 1.4; color: var(--text-dim); background: var(--surface-2, color-mix(in srgb, var(--text-dim) 10%, transparent)); border-radius: var(--radius-m); white-space: pre-wrap; word-break: break-word; }
</style>
