<script lang="ts">
  // One evaluation run: header summary + per-iteration report. Each iteration
  // shows the implementation (+ diff), every validation's issues & suggested
  // fixes, the score, before/after regression vs the previous round, and the
  // improver's skill diff. Header actions: cancel, delete, promote the winning
  // skill. Polls while running.
  import { untrack } from 'svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import { skillEvalBus } from '../../lib/events.svelte';
  import type {
    EvalFinding,
    EvalIteration,
    EvalValidationState,
    ImplDiffResp,
    PromoteSkillReq,
    SkillEval,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  interface Props {
    evalId: string;
    /** Bubble the freshest run up so the parent list reflects live status. */
    onupdate?: (e: SkillEval) => void;
    /** Called after the run is deleted so the parent can drop it from the list. */
    ondeleted?: (evalId: string) => void;
  }
  let { evalId, onupdate, ondeleted }: Props = $props();

  let run: SkillEval | null = $state(null);
  let loading = $state(true);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollCount = $state(0);

  // Expansion / busy state (keyed by stable ids).
  let openTerminals = $state<Set<string>>(new Set());
  let openFindings = $state<Set<string>>(new Set());
  let openDiffs = $state<Set<string>>(new Set());
  let openImplDiffs = $state<Set<string>>(new Set());
  let implDiffs = $state<Record<string, ImplDiffResp>>({});
  let implDiffLoading = $state<Set<string>>(new Set());
  let retrying = $state<Set<string>>(new Set());
  let cancelling = $state(false);
  let confirmingDelete = $state(false);
  let deleting = $state(false);

  // Promote modal.
  let promoteOpen = $state(false);
  let promoteIterId = $state('');
  let promoteSource = $state<'tested' | 'improved'>('tested');
  let promoteName = $state('');
  let promoting = $state(false);

  $effect(() => {
    const id = evalId;
    void load(id);
    return () => {
      if (pollTimer !== null) clearTimeout(pollTimer);
    };
  });

  // Live refresh: a `skill_eval_updated` WS event for this run triggers an
  // immediate poll instead of waiting for the 2s timer. The last-tick guard +
  // untrack keep this from self-triggering (the timed poll remains as fallback).
  let lastEvalTick = 0;
  $effect(() => {
    const t = skillEvalBus.tick;
    if (t === lastEvalTick) return;
    lastEvalTick = t;
    untrack(() => {
      if (skillEvalBus.runId === evalId && run && isActive(run)) void poll();
    });
  });

  async function load(id: string): Promise<void> {
    loading = true;
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollCount = 0;
    try {
      const r = await skillsEvalApi.get(id);
      run = r;
      onupdate?.(r);
      if (isActive(r)) schedulePoll();
    } catch (e) {
      toasts.error('Could not load evaluation', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  function isActive(r: SkillEval): boolean {
    return (
      r.status === 'running' ||
      r.iterations.some((it) => it.status !== 'done' && it.status !== 'error')
    );
  }

  function schedulePoll(delay = 2000): void {
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void poll(), delay);
  }

  async function poll(): Promise<void> {
    pollCount++;
    try {
      const r = await skillsEvalApi.get(evalId);
      run = r;
      onupdate?.(r);
      if (isActive(r)) schedulePoll(pollCount > 600 ? 5000 : 2000);
    } catch {
      schedulePoll();
    }
  }

  function toggle(set: Set<string>, key: string): Set<string> {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  }

  function scoreClass(n: number): string {
    if (n >= 85) return 'good';
    if (n >= 60) return 'ok';
    return 'bad';
  }

  function findingCount(it: EvalIteration): number {
    return it.agents.reduce((sum, a) => sum + a.findings.length, 0);
  }

  function diffLineClass(line: string): string {
    if (line.startsWith('+ ')) return 'add';
    if (line.startsWith('- ')) return 'del';
    return 'ctx';
  }

  function valKey(itId: string, a: EvalValidationState, i: number): string {
    return `${itId}:${a.name}:${i}`;
  }

  // --- before/after regression vs the previous iteration --------------------
  function fkey(validation: string, f: EvalFinding): string {
    return `${validation}|${f.issue.toLowerCase().split(/\s+/).join(' ').slice(0, 80)}`;
  }
  function iterKeys(it: EvalIteration): Set<string> {
    const s = new Set<string>();
    for (const a of it.agents) for (const f of a.findings) s.add(fkey(a.validation, f));
    return s;
  }
  function regression(idx: number): { fixed: number; introduced: number } | null {
    if (!run || idx === 0) return null;
    const prev = run.iterations[idx - 1];
    const cur = run.iterations[idx];
    if (!prev || prev.status !== 'done' || cur.status !== 'done') return null;
    const pk = iterKeys(prev);
    const ck = iterKeys(cur);
    let fixed = 0;
    let introduced = 0;
    for (const k of pk) if (!ck.has(k)) fixed++;
    for (const k of ck) if (!pk.has(k)) introduced++;
    return { fixed, introduced };
  }

  // --- header actions -------------------------------------------------------
  async function cancelRun(): Promise<void> {
    if (!run || cancelling) return;
    cancelling = true;
    try {
      const r = await skillsEvalApi.cancel(run.id);
      run = r;
      onupdate?.(r);
      toasts.info('Evaluation cancelled');
    } catch (e) {
      toasts.error('Cancel failed', e instanceof Error ? e.message : String(e));
    } finally {
      cancelling = false;
    }
  }

  async function deleteRun(): Promise<void> {
    if (!run || deleting) return;
    deleting = true;
    try {
      await skillsEvalApi.remove(run.id);
      toasts.info('Evaluation deleted');
      ondeleted?.(run.id);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    } finally {
      deleting = false;
      confirmingDelete = false;
    }
  }

  // --- per-validation retry -------------------------------------------------
  async function retry(it: EvalIteration, index: number): Promise<void> {
    if (!run) return;
    const key = `${it.id}:${index}`;
    if (retrying.has(key)) return;
    retrying = new Set(retrying).add(key);
    try {
      const r = await skillsEvalApi.retryValidation(run.id, it.id, index);
      run = r;
      onupdate?.(r);
      if (isActive(r)) schedulePoll();
      toasts.info('Re-running validation…');
    } catch (e) {
      toasts.error('Retry failed', e instanceof Error ? e.message : String(e));
    } finally {
      retrying = new Set([...retrying].filter((k) => k !== key));
    }
  }

  // --- implementation diff --------------------------------------------------
  async function toggleImplDiff(it: EvalIteration): Promise<void> {
    const open = openImplDiffs.has(it.id);
    openImplDiffs = toggle(openImplDiffs, it.id);
    if (!open && !implDiffs[it.id] && run) {
      implDiffLoading = new Set(implDiffLoading).add(it.id);
      try {
        implDiffs[it.id] = await skillsEvalApi.implDiff(run.id, it.id);
      } catch (e) {
        toasts.error('Could not load diff', e instanceof Error ? e.message : String(e));
      } finally {
        implDiffLoading = new Set([...implDiffLoading].filter((k) => k !== it.id));
      }
    }
  }

  // --- export / promote -----------------------------------------------------
  function skillContent(it: EvalIteration, source: 'tested' | 'improved'): string {
    return source === 'improved' ? (it.skill_after ?? '') : it.skill_before;
  }
  async function copySkill(it: EvalIteration, source: 'tested' | 'improved'): Promise<void> {
    try {
      await navigator.clipboard.writeText(skillContent(it, source));
      toasts.success('Skill copied to clipboard');
    } catch {
      toasts.error('Copy failed');
    }
  }
  function downloadSkill(it: EvalIteration, source: 'tested' | 'improved'): void {
    const blob = new Blob([skillContent(it, source)], { type: 'text/markdown' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${it.skill_name}${source === 'improved' ? '-improved' : ''}.SKILL.md`;
    a.click();
    URL.revokeObjectURL(url);
  }
  function openPromote(it: EvalIteration, source: 'tested' | 'improved'): void {
    promoteIterId = it.id;
    promoteSource = source;
    promoteName = run?.source_skill ?? it.skill_name;
    promoteOpen = true;
  }
  function promoteWinner(): void {
    if (!run || run.best_iteration == null) return;
    const it = run.iterations.find((i) => i.iter === run!.best_iteration);
    if (it) openPromote(it, 'tested');
  }
  async function doPromote(): Promise<void> {
    if (!run || promoting) return;
    const name = promoteName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      toasts.error('Invalid name', "Use letters, digits, '-' or '_' only.");
      return;
    }
    promoting = true;
    try {
      const body: PromoteSkillReq = {
        iteration_id: promoteIterId,
        source: promoteSource,
        name,
      };
      await skillsEvalApi.promote(run.id, body);
      toasts.success('Skill saved to library', name);
      promoteOpen = false;
    } catch (e) {
      toasts.error('Promote failed', e instanceof Error ? e.message : String(e));
    } finally {
      promoting = false;
    }
  }

  const promoteIter = $derived.by(() => {
    if (!run) return null;
    return run.iterations.find((it) => it.id === promoteIterId) ?? null;
  });
</script>

{#if loading && !run}
  <div class="rd-loading"><span class="spinner-xs"></span> Loading…</div>
{:else if run}
  <div class="rd">
    <header class="rd-head">
      <div class="rd-title-row">
        <Icon name="zap" size={16} />
        <h2 class="rd-title">{run.source_skill}</h2>
        <span class="status-pill st-{run.status}">
          {#if run.status === 'running'}<span class="spinner-xs"></span>{/if}
          {run.status}
        </span>
        <span class="grow"></span>
        {#if run.best_score != null}
          <span class="score-badge {scoreClass(run.best_score)}">
            best {run.best_score.toFixed(0)}{run.best_iteration ? ` · iter ${run.best_iteration}` : ''}
          </span>
        {/if}
      </div>

      <div class="rd-actions">
        {#if isActive(run)}
          <button class="btn small" disabled={cancelling} onclick={cancelRun}>
            {cancelling ? 'Cancelling…' : 'Cancel run'}
          </button>
        {/if}
        {#if run.best_iteration != null}
          <button class="btn small primary" onclick={promoteWinner}>
            <Icon name="check" size={13} /> Promote winning skill
          </button>
        {/if}
        <span class="grow"></span>
        {#if confirmingDelete}
          <span class="confirm-text">Delete this run?</span>
          <button class="btn small ghost" onclick={() => (confirmingDelete = false)}>Keep</button>
          <button class="btn small danger" disabled={deleting} onclick={deleteRun}>
            {deleting ? 'Deleting…' : 'Delete'}
          </button>
        {:else}
          <button class="btn small ghost danger" onclick={() => (confirmingDelete = true)} title="Delete run + worktrees">
            <Icon name="trash" size={13} /> Delete
          </button>
        {/if}
      </div>

      <p class="rd-task">{run.task}</p>
      <div class="rd-meta">
        <span class="chip">impl: {run.impl_cli}</span>
        <span class="chip">{run.target_iterations} iteration{run.target_iterations === 1 ? '' : 's'}</span>
        {#each run.iterations as it (it.id)}
          <span class="chip score {scoreClass(it.score)}">iter {it.iter}: {it.score.toFixed(0)}</span>
        {/each}
      </div>
      {#if run.summary}<p class="rd-summary">{run.summary}</p>{/if}
      {#if run.error}<p class="rd-error">⚠ {run.error}</p>{/if}
    </header>

    {#each run.iterations as it, idx (it.id)}
      {@const reg = regression(idx)}
      <section class="iter card">
        <div class="iter-head">
          <span class="iter-num">Iteration {it.iter}</span>
          {#if it.base_iter}<span class="chip subtle">improved from iter {it.base_iter}</span>{/if}
          <span class="chip subtle mono">{it.skill_name}</span>
          {#if reg}
            <span class="reg" class:bad={reg.introduced > reg.fixed}>
              fixed {reg.fixed} · introduced {reg.introduced}
            </span>
          {/if}
          <span class="grow"></span>
          {#if it.status === 'done'}
            <span class="score-badge {scoreClass(it.score)}">{it.score.toFixed(0)}</span>
          {/if}
          <span class="status-pill ist-{it.status}">
            {#if it.status !== 'done' && it.status !== 'error'}<span class="spinner-xs"></span>{/if}
            {it.status}
          </span>
        </div>

        <!-- Implementation -->
        <div class="impl">
          <div class="impl-top">
            <span class="lbl">Implementation</span>
            <span class="chip mono">{it.impl_provider}</span>
            <span class="grow"></span>
            {#if it.worktree_path}
              <button class="btn small ghost" onclick={() => toggleImplDiff(it)}>
                {openImplDiffs.has(it.id) ? 'Hide code diff' : 'View code diff'}
              </button>
            {/if}
            {#if it.impl_session_id}
              <button class="btn small ghost" onclick={() => (openTerminals = toggle(openTerminals, it.impl_session_id ?? ''))}>
                {openTerminals.has(it.impl_session_id ?? '') ? 'Hide session' : 'Open session'}
              </button>
            {/if}
          </div>
          {#if it.impl_summary}<p class="impl-summary">{it.impl_summary}</p>{/if}
          {#if it.worktree_path}<p class="worktree mono">{it.worktree_path}</p>{/if}
          {#if it.impl_session_id && openTerminals.has(it.impl_session_id)}
            <div class="term">{#key it.impl_session_id}<Terminal sessionId={it.impl_session_id} />{/key}</div>
          {/if}
          {#if openImplDiffs.has(it.id)}
            {#if implDiffLoading.has(it.id)}
              <p class="muted"><span class="spinner-xs"></span> Loading diff…</p>
            {:else if implDiffs[it.id]}
              {#if implDiffs[it.id].diff.trim()}
                <pre class="diff">{#each implDiffs[it.id].diff.split('\n') as line, li (li)}<span class="dl {line.startsWith('+') ? 'add' : line.startsWith('-') ? 'del' : 'ctx'}">{line || ' '}</span>{'\n'}{/each}</pre>
              {:else}
                <p class="muted">No code changes captured.</p>
              {/if}
            {/if}
          {/if}
        </div>

        <!-- Validations -->
        <div class="vals">
          <div class="vals-head">Validations · {findingCount(it)} issue{findingCount(it) === 1 ? '' : 's'} found</div>
          {#each it.agents as a, ai (valKey(it.id, a, ai))}
            {@const key = valKey(it.id, a, ai)}
            {@const rk = `${it.id}:${ai}`}
            <div class="val card">
              <div class="val-top">
                <span class="val-name">{a.name}</span>
                <span class="chip mono">{a.provider}{a.model ? ' · ' + a.model : ''}</span>
                {#if a.status === 'done'}
                  <span class="pf {a.passed ? 'pass' : 'fail'}">{a.passed ? 'passed' : 'failed'}</span>
                  <span class="score-badge sm {scoreClass(a.score)}">{a.score.toFixed(0)}</span>
                {/if}
                <span class="grow"></span>
                {#if a.session_id}
                  <button class="btn small ghost" onclick={() => (openTerminals = toggle(openTerminals, a.session_id ?? ''))}>
                    {openTerminals.has(a.session_id ?? '') ? 'Hide' : 'Open'}
                  </button>
                {/if}
                <button class="btn small ghost" disabled={retrying.has(rk)} onclick={() => retry(it, ai)} title="Re-run this validation">
                  {retrying.has(rk) ? 'Retrying…' : 'Retry'}
                </button>
                {#if a.findings.length > 0}
                  <button class="btn small ghost" onclick={() => (openFindings = toggle(openFindings, key))}>
                    {openFindings.has(key) ? 'Hide' : `${a.findings.length} issue${a.findings.length === 1 ? '' : 's'}`}
                  </button>
                {/if}
                <span class="status-pill ist-{a.status}">
                  {#if a.status === 'running' || a.status === 'waiting'}<span class="spinner-xs"></span>{/if}
                  {a.status}
                </span>
              </div>
              {#if a.note && a.status !== 'done'}<p class="val-note">{a.note}</p>{/if}
              {#if a.status === 'waiting'}
                <p class="val-waiting">⚠ Looks blocked on input — <strong>Open</strong> the session to respond.</p>
              {/if}
              {#if a.session_id && openTerminals.has(a.session_id)}
                <div class="term">{#key a.session_id}<Terminal sessionId={a.session_id} />{/key}</div>
              {/if}
              {#if openFindings.has(key)}
                <ul class="findings">
                  {#each a.findings as f, fi (key + ':' + fi)}
                    <li class="finding">
                      <div class="finding-head">
                        <span class="sev sev-{f.severity}">{f.severity}</span>
                        {#if f.location}<span class="mono loc">{f.location}</span>{/if}
                      </div>
                      <p class="issue"><span class="tag">Issue</span> {f.issue}</p>
                      {#if f.suggestion}<p class="fix"><span class="tag fix-tag">Fix</span> {f.suggestion}</p>{/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/each}
        </div>

        <!-- Export / promote this iteration's tested skill -->
        <div class="skill-actions">
          <span class="lbl">Skill ({it.skill_name})</span>
          <span class="grow"></span>
          <button class="btn small ghost" onclick={() => copySkill(it, 'tested')}>Copy</button>
          <button class="btn small ghost" onclick={() => downloadSkill(it, 'tested')}>Download</button>
          <button class="btn small ghost" onclick={() => openPromote(it, 'tested')}>Promote</button>
        </div>

        <!-- Improvement -->
        {#if it.improvement_summary}
          <div class="improve">
            <div class="improve-top">
              <span class="lbl"><Icon name="edit" size={12} /> Skill improvement</span>
              <span class="grow"></span>
              {#if it.skill_after}
                <button class="btn small ghost" onclick={() => copySkill(it, 'improved')}>Copy improved</button>
                <button class="btn small ghost" onclick={() => openPromote(it, 'improved')}>Promote improved</button>
              {/if}
              {#if it.skill_diff}
                <button class="btn small ghost" onclick={() => (openDiffs = toggle(openDiffs, it.id))}>
                  {openDiffs.has(it.id) ? 'Hide skill diff' : 'View skill diff'}
                </button>
              {/if}
            </div>
            <p class="improve-summary">{it.improvement_summary}</p>
            {#if it.skill_diff && openDiffs.has(it.id)}
              <pre class="diff">{#each it.skill_diff.split('\n') as line, li (li)}<span class="dl {diffLineClass(line)}">{line || ' '}</span>{'\n'}{/each}</pre>
            {/if}
          </div>
        {/if}
      </section>
    {/each}

    {#if run.iterations.length === 0}
      <div class="rd-loading"><span class="spinner-xs"></span> Preparing first iteration…</div>
    {/if}
  </div>
{/if}

{#if promoteOpen}
  <Modal title="Promote skill to library" width={460} onclose={() => (promoteOpen = false)}>
    {#snippet children()}
      <p class="modal-lede">
        Saves this {promoteSource === 'improved' ? 'improved' : 'tested'} skill into the Otto library
        under the name below (overwrites an existing skill of that name).
      </p>
      <label class="field-label" for="promote-name">Library skill name</label>
      <input id="promote-name" class="input" bind:value={promoteName} placeholder="my-skill" />
      {#if promoteIter?.skill_after}
        <div class="src-toggle">
          <label class="chip-toggle" class:on={promoteSource === 'tested'}>
            <input type="radio" name="promote-src" checked={promoteSource === 'tested'} onchange={() => (promoteSource = 'tested')} />
            Tested version
          </label>
          <label class="chip-toggle" class:on={promoteSource === 'improved'}>
            <input type="radio" name="promote-src" checked={promoteSource === 'improved'} onchange={() => (promoteSource = 'improved')} />
            Improved version
          </label>
        </div>
      {/if}
    {/snippet}
    {#snippet footer()}
      <button class="btn" onclick={() => (promoteOpen = false)}>Cancel</button>
      <button class="btn primary" disabled={promoting} onclick={doPromote}>
        {promoting ? 'Saving…' : 'Promote'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .rd {
    padding: 16px 18px 60px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
    height: 100%;
  }
  .rd-loading,
  .muted {
    padding: 8px 0;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .rd-loading {
    padding: 30px;
    text-align: center;
  }
  .rd-head {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rd-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .rd-title {
    margin: 0;
    font-size: 15px;
  }
  .rd-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .confirm-text {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .rd-task {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.5;
  }
  .rd-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
  .rd-summary {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .rd-error {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--status-exited);
  }

  .iter {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .iter-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .iter-num {
    font-size: 13px;
    font-weight: 700;
  }
  .reg {
    font-size: 10.5px;
    font-weight: 600;
    color: var(--status-idle, #3a8c3a);
  }
  .reg.bad {
    color: #b07d00;
  }

  .impl,
  .improve {
    border-left: 2px solid var(--border);
    padding-left: 10px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .impl-top,
  .improve-top,
  .vals-head,
  .skill-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .skill-actions {
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }
  .lbl,
  .vals-head {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .impl-summary,
  .improve-summary {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
  }
  .worktree {
    margin: 0;
    font-size: 10.5px;
    color: var(--text-dim);
    word-break: break-all;
  }

  .vals {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .val {
    padding: 8px 10px;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .val-top {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
  }
  .val-name {
    font-size: 12px;
    font-weight: 600;
  }
  .val-note {
    margin: 0;
    font-size: 11px;
    color: var(--text-dim);
  }
  .val-waiting {
    margin: 0;
    font-size: 11px;
    color: #b07d00;
  }

  .findings {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .finding {
    border-top: 1px solid var(--border);
    padding-top: 6px;
  }
  .finding-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 3px;
  }
  .issue,
  .fix {
    margin: 2px 0;
    font-size: 11.5px;
    line-height: 1.45;
  }
  .tag {
    display: inline-block;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 3px;
    margin-right: 5px;
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
    vertical-align: middle;
  }
  .fix-tag {
    background: color-mix(in srgb, var(--status-idle, #3a8c3a) 18%, transparent);
    color: var(--status-idle, #3a8c3a);
  }
  .loc {
    font-size: 10.5px;
    color: var(--text-dim);
  }

  .term {
    height: 320px;
    margin: 6px 0 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: #1b1b1b;
  }

  .diff {
    margin: 6px 0 0;
    max-height: 360px;
    overflow: auto;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    line-height: 1.45;
    background: var(--bg-1, #1b1b1b);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .dl {
    display: inline;
  }
  .dl.add {
    color: #4caf6a;
  }
  .dl.del {
    color: #d66;
  }
  .dl.ctx {
    color: var(--text-dim);
  }

  .status-pill {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: var(--radius-s, 4px);
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .st-running,
  .ist-implementing,
  .ist-validating,
  .ist-improving,
  .ist-running,
  .ist-pending {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .st-done,
  .ist-done {
    background: color-mix(in srgb, var(--status-idle, #6bbf6b) 15%, transparent);
    color: var(--status-idle, #3a8c3a);
  }
  .st-error,
  .ist-error,
  .st-cancelled {
    background: color-mix(in srgb, var(--status-exited) 15%, transparent);
    color: var(--status-exited);
  }
  .ist-waiting {
    background: color-mix(in srgb, #e0a000 20%, transparent);
    color: #b07d00;
  }

  .pf {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: var(--radius-s, 4px);
  }
  .pf.pass {
    background: color-mix(in srgb, var(--status-idle, #6bbf6b) 18%, transparent);
    color: var(--status-idle, #3a8c3a);
  }
  .pf.fail {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }

  .score-badge {
    font-size: 11px;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .score-badge.sm {
    font-size: 10px;
    padding: 1px 6px;
  }
  .score-badge.good,
  .chip.score.good {
    background: color-mix(in srgb, var(--status-idle, #6bbf6b) 20%, transparent);
    color: var(--status-idle, #3a8c3a);
  }
  .score-badge.ok,
  .chip.score.ok {
    background: color-mix(in srgb, #e0a000 22%, transparent);
    color: #b07d00;
  }
  .score-badge.bad,
  .chip.score.bad {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }

  .sev {
    display: inline-block;
    padding: 2px 7px;
    border-radius: var(--radius-s, 4px);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .sev-info {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .sev-warn {
    background: color-mix(in srgb, #e0a000 20%, transparent);
    color: #b07d00;
  }
  .sev-fail {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }

  .chip.subtle {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text-dim);
  }

  .modal-lede {
    margin: 0 0 10px;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .field-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
    display: block;
    margin-bottom: 4px;
  }
  .src-toggle {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }
  .chip-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 11.5px;
    cursor: pointer;
  }
  .chip-toggle.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--accent);
  }

  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    vertical-align: middle;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .grow {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
</style>
