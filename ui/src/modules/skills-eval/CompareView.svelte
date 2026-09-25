<script lang="ts">
  // Side-by-side comparison of several runs (A/B two skills, or any N runs):
  // best score, iterations, and per-validation scores from each run's best
  // iteration. Pure client-side over already-loaded runs.
  import type { SkillEval } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    runs: SkillEval[];
  }
  let { runs }: Props = $props();

  function bestIteration(r: SkillEval) {
    if (r.best_iteration != null) {
      const m = r.iterations.find((i) => i.iter === r.best_iteration);
      if (m) return m;
    }
    const done = r.iterations.filter((i) => i.status === 'done');
    return done.length > 0 ? done[done.length - 1] : r.iterations[r.iterations.length - 1] ?? null;
  }

  // validation name -> mean score, for a run's best iteration.
  function valScores(r: SkillEval): Record<string, number> {
    const it = bestIteration(r);
    const out: Record<string, number> = {};
    if (!it) return out;
    const groups: Record<string, number[]> = {};
    for (const a of it.agents) {
      if (a.status !== 'done') continue;
      (groups[a.validation] ??= []).push(a.score);
    }
    for (const [k, arr] of Object.entries(groups)) {
      out[k] = arr.reduce((s, x) => s + x, 0) / arr.length;
    }
    return out;
  }

  const cols = $derived(runs.map((r) => ({ run: r, scores: valScores(r) })));
  const dimensions = $derived.by(() => {
    const set = new Set<string>();
    for (const c of cols) for (const k of Object.keys(c.scores)) set.add(k);
    return [...set].sort();
  });

  function scoreClass(n: number | undefined): string {
    if (n == null) return '';
    if (n >= 85) return 'good';
    if (n >= 60) return 'ok';
    return 'bad';
  }
  function best(values: (number | undefined)[]): number | null {
    const nums = values.filter((v): v is number => v != null);
    return nums.length ? Math.max(...nums) : null;
  }
</script>

<div class="cmp">
  <h2>Compare {runs.length} runs</h2>
  <p class="lede">Scores from each run's best iteration. <Icon name="check" size={12} /> marks the top score in each row.</p>
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th class="rowlabel" scope="col"><span class="sr-only">Signal</span></th>
          {#each cols as c (c.run.id)}
            <th scope="col" title={c.run.task}>
              <div class="col-skill">{c.run.source_skill}</div>
              <div class="col-cli mono">{c.run.impl_cli || 'score only'}</div>
              {#if c.run.task}<div class="col-task">{c.run.task}</div>{/if}
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        <tr class="major">
          <th class="rowlabel" scope="row">Best score</th>
          {#each cols as c (c.run.id)}
            {@const bs = c.run.best_score ?? undefined}
            {@const top = best(cols.map((x) => x.run.best_score ?? undefined))}
            <td>
              {#if bs != null}
                <span class="score {scoreClass(bs)}" class:winner={top != null && bs === top} title={top != null && bs === top ? 'Top score' : undefined}>{#if top != null && bs === top}<Icon name="check" size={12} />{/if}{bs.toFixed(0)}</span>
              {:else}
                <span class="dash">—</span>
              {/if}
            </td>
          {/each}
        </tr>
        <tr>
          <th class="rowlabel" scope="row">Iterations</th>
          {#each cols as c (c.run.id)}
            <td>{c.run.iterations.length}</td>
          {/each}
        </tr>
        {#each dimensions as dim (dim)}
          {@const top = best(cols.map((c) => c.scores[dim]))}
          <tr>
            <th class="rowlabel" scope="row">{dim}</th>
            {#each cols as c (c.run.id)}
              {@const v = c.scores[dim]}
              <td>
                {#if v != null}
                  <span class="score {scoreClass(v)}" class:winner={top != null && v === top} title={top != null && v === top ? 'Top score' : undefined}>{#if top != null && v === top}<Icon name="check" size={12} />{/if}{v.toFixed(0)}</span>
                {:else}
                  <span class="dash">—</span>
                {/if}
              </td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .cmp {
    padding: 16px 18px 60px;
    overflow: auto;
    height: 100%;
  }
  h2 {
    margin: 0 0 12px;
    font-size: var(--fs-l);
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: var(--fs-s);
  }
  th,
  td {
    border: 1px solid var(--border);
    padding: 8px 10px;
    text-align: center;
    vertical-align: middle;
  }
  thead th {
    background: var(--surface-2);
  }
  .lede {
    margin: -6px 0 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .lede :global(svg) {
    vertical-align: -1px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .rowlabel {
    text-align: start;
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .major td {
    font-weight: 600;
  }
  .col-skill {
    font-weight: 600;
  }
  .col-cli {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .score {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    min-width: 28px;
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 600;
  }
  .score.good {
    background: var(--success-soft);
    color: var(--success);
  }
  .score.ok {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .score.bad {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .score.winner {
    outline: 2px solid color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .dash {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  /* Runs of the same skill differ by task: show it (clipped, full text in the header's title). */
  .col-task {
    max-width: 180px;
    margin: 2px auto 0;
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
