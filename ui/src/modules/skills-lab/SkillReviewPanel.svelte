<script lang="ts">
  // Skills Lab → Review. A list of skill reviews + a detail pane that mirrors the
  // code-review UX: a deterministic static-analysis card, N visible embedded
  // agent terminals (SkillReviewAgents), and the summarizer's aggregated report.
  // Live-refreshes on the skill_review_updated bus, with a fallback poll while a
  // review is running.
  import { onDestroy } from 'svelte';
  import type { LibrarySkill, BundledSkillView, ProviderSkillInfo, SkillReview } from '../../lib/api/types';
  import { skillReviewApi } from '../../lib/api/skillReview';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { skillReviewBus } from '../../lib/events.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import SkillReviewAgents from './SkillReviewAgents.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import { rel } from '../../lib/stores/now.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { sourceLabel } from './skillGroups';

  interface Props {
    wsId: string;
    /** Optional skill to pre-select in the New Review form (from the Skills tab). */
    initialTarget?: { name: string; source: string } | null;
    onconsumed?: () => void;
    /** Open this existing review (a skill's Usage → Reviews card). */
    initialReview?: string | null;
    onreviewconsumed?: () => void;
  }
  let { wsId, initialTarget = null, onconsumed, initialReview = null, onreviewconsumed }: Props = $props();

  // `source` is "library" | "bundled" | a provider name (claude/codex/agy) — all
  // valid `skill_source` values the review engine resolves.
  type SkillOpt = { name: string; source: string; label: string };

  let reviews = $state<SkillReview[]>([]);
  // Inline list-load failure + Retry (instead of a toast over "No skill reviews yet").
  let listError = $state<string | null>(null);
  // First list load: don't flash "No skill reviews yet" before it lands.
  let listLoading = $state(true);
  let selected = $state<SkillReview | null>(null);
  let skillOpts = $state<SkillOpt[]>([]);

  // New-review form.
  let fSkill = $state('');
  let fMode = $state<'static' | 'agents'>('agents');
  // Reviewer providers from the live registry (built-ins + custom, e.g. grok).
  // Selection is a set of provider names; default to the first agent provider.
  let fProviders = $state<Set<string>>(new Set([defaultAgentProvider()]));
  function toggleReviewer(p: string): void {
    const next = new Set(fProviders);
    if (next.has(p)) next.delete(p);
    else next.add(p);
    fProviders = next;
  }
  let fInstructions = $state('');
  let starting = $state(false);
  let listHidden = $state(false);

  // Apply-fixes form.
  let fixProvider = $state(defaultAgentProvider());
  let fixInstructions = $state('');
  let fixTermOpen = $state(false);
  let applying = $state(false);

  const fixRunning = $derived(
    !!selected?.fix_agent && ['pending', 'running', 'waiting'].includes(selected.fix_agent.status),
  );
  // Keep the fallback poll alive while the fixer runs, not just the review.
  const activeReview = $derived(selected && (selected.status === 'running' || fixRunning));
  const canApply = $derived(
    !!selected &&
      selected.status === 'done' &&
      !fixRunning &&
      ((selected.summary?.findings.length ?? 0) > 0 ||
        (selected.summary?.patch_plan.length ?? 0) > 0 ||
        (selected.static_report?.findings.length ?? 0) > 0),
  );

  async function loadSkills(): Promise<void> {
    try {
      const [lib, bundled, provider] = await Promise.all([
        skillLabApi.listLibrary().catch(() => [] as LibrarySkill[]),
        skillLabApi.listBundled().catch(() => [] as BundledSkillView[]),
        skillLabApi.listProvider().catch(() => [] as ProviderSkillInfo[]),
      ]);
      const opts: SkillOpt[] = [];
      for (const s of lib) opts.push({ name: s.name, source: 'library', label: `${s.name} · library` });
      const libNames = new Set(lib.map((s) => s.name));
      for (const b of bundled)
        if (!libNames.has(b.name)) opts.push({ name: b.name, source: 'bundled', label: `${b.name} · bundled` });
      for (const p of provider)
        opts.push({ name: p.name, source: p.provider, label: `${p.name} · ${p.provider}` });
      opts.sort((a, b) => a.name.localeCompare(b.name));
      skillOpts = opts;
    } catch {
      skillOpts = [];
    }
  }

  async function loadList(): Promise<void> {
    if (!wsId) {
      listLoading = false;
      return;
    }
    try {
      reviews = await skillReviewApi.list(wsId);
      listError = null;
    } catch (e) {
      listError = e instanceof Error ? e.message : String(e);
    } finally {
      listLoading = false;
    }
  }

  /** `quiet` for background refreshes (poll / bus): a transient failure there
   *  must not raise a toast every 2.5 s — the next tick retries. */
  let detailGeneration = 0;
  let selectionPending: number | null = null;
  let refreshPending: number | null = null;
  async function openReview(id: string, quiet = false): Promise<void> {
    // A poll for the still-visible old review must not supersede a deliberate
    // selection. Coalesce background reads so a slow response can finish.
    if (quiet && (selected?.id !== id || selectionPending === detailGeneration || refreshPending === detailGeneration)) return;
    const generation = quiet ? detailGeneration : ++detailGeneration;
    if (quiet) refreshPending = generation;
    else selectionPending = generation;
    const workspace = wsId;
    try {
      const review = await skillReviewApi.get(id);
      if (generation === detailGeneration && workspace === wsId) selected = review;
    } catch (e) {
      if (generation === detailGeneration && workspace === wsId && !quiet) toasts.error("Couldn't open the review", e instanceof Error ? e.message : String(e));
    } finally {
      if (quiet && refreshPending === generation) refreshPending = null;
      if (!quiet && selectionPending === generation) selectionPending = null;
    }
  }

  async function start(): Promise<void> {
    if (!fSkill || starting) return;
    const opt = skillOpts.find((o) => `${o.source}:${o.name}` === fSkill);
    if (!opt) return;
    const providers: string[] = [];
    if (fMode === 'agents') {
      providers.push(...fProviders);
      if (providers.length === 0) providers.push(defaultAgentProvider());
    }
    starting = true;
    try {
      const rev = await skillReviewApi.start(wsId, {
        skill_name: opt.name,
        skill_source: opt.source,
        providers,
        agent_mode: fMode,
        instructions: fInstructions.trim(),
      });
      selected = rev;
      await loadList();
    } catch (e) {
      toasts.error("Couldn't start the review", e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

  async function applyFixes(): Promise<void> {
    if (!selected || applying) return;
    applying = true;
    try {
      selected = await skillReviewApi.apply(selected.id, {
        provider: fixProvider,
        instructions: fixInstructions.trim(),
      });
      fixTermOpen = true;
      toasts.info('Fixer agent starting…');
    } catch (e) {
      toasts.error("Couldn't start the fixer agent", e instanceof Error ? e.message : String(e));
    } finally {
      applying = false;
    }
  }

  async function cancelReview(): Promise<void> {
    if (!selected) return;
    if (
      !(await confirmer.ask(`Stop the review of "${selected.skill_name}"? Agents still running are stopped and their partial findings are not summarized.`, {
        title: 'Stop review',
        confirmLabel: 'Stop review',
      }))
    )
      return;
    try {
      selected = await skillReviewApi.cancel(selected.id);
      await loadList();
    } catch (e) {
      toasts.error("Couldn't stop the review", e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteReview(rev: SkillReview): Promise<void> {
    // Every other delete in Skills Lab / Evaluator confirms; this one used to
    // fire on a single click of a bare ✕.
    if (
      !(await confirmer.ask(
        `Delete the review of "${rev.skill_name}"? Its findings, agent transcripts and summary are removed. The skill itself is not touched.`,
        { title: 'Delete review' },
      ))
    )
      return;
    try {
      await skillReviewApi.remove(rev.id);
      if (selected?.id === rev.id) selected = null;
      await loadList();
    } catch (e) {
      toasts.error("Couldn't delete the review", e instanceof Error ? e.message : String(e));
    }
  }

  function newReview(): void {
    detailGeneration++;
    selected = null;
    fSkill = '';
  }

  // --- live refresh -----------------------------------------------------------
  let lastTick = 0;
  $effect(() => {
    const t = skillReviewBus.tick;
    if (t === lastTick) return;
    lastTick = t;
    if (skillReviewBus.workspaceId && skillReviewBus.workspaceId !== wsId) return;
    // A review advanced — refresh the list and, if it's the open one, the detail.
    void loadList();
    if (selected && skillReviewBus.reviewId === selected.id) void openReview(selected.id, true);
  });

  // Fallback poll while the open review is running (covers dropped sockets).
  let poll: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (poll) { clearInterval(poll); poll = null; }
    if (activeReview && selected) {
      const id = selected.id;
      poll = setInterval(() => { void openReview(id, true); }, 2500);
    }
  });
  onDestroy(() => { detailGeneration++; if (poll) clearInterval(poll); });

  // Load on workspace change.
  let loadedWs = '';
  $effect(() => {
    if (wsId && wsId !== loadedWs) {
      // The open review belongs to the previous workspace.
      detailGeneration++;
      if (loadedWs) selected = null;
      loadedWs = wsId;
      void loadSkills();
      // Open on the newest review rather than a blank form — unless the form
      // is already in use (a "Review this skill" hand-off pre-fills fSkill).
      void loadList().then(() => {
        if (!selected && !fSkill && reviews.length > 0) void openReview(reviews[0].id);
      });
    }
  });

  // Consume a cross-tab "review this skill" intent by pre-filling the form.
  let consumedTarget = '';
  $effect(() => {
    if (initialTarget && `${initialTarget.source}:${initialTarget.name}` !== consumedTarget) {
      detailGeneration++;
      consumedTarget = `${initialTarget.source}:${initialTarget.name}`;
      selected = null;
      // Ensure the option exists even before the skill list loads.
      const key = `${initialTarget.source}:${initialTarget.name}`;
      if (!skillOpts.some((o) => `${o.source}:${o.name}` === key)) {
        skillOpts = [
          { name: initialTarget.name, source: initialTarget.source, label: `${initialTarget.name} · ${initialTarget.source}` },
          ...skillOpts,
        ];
      }
      fSkill = key;
      onconsumed?.();
    }
  });

  $effect(() => {
    if (!initialReview) return;
    const id = initialReview;
    onreviewconsumed?.();
    void openReview(id);
  });

  /** "spec_compliance" → "Spec compliance" (scorecard areas are snake_case ids). */
  function areaLabel(a: string): string {
    const t = a.replace(/_/g, ' ').trim();
    return t.charAt(0).toUpperCase() + t.slice(1);
  }

  function verdictClass(v: string): string {
    if (v === 'Ready') return 'verdict-ready';
    if (v === 'Ready with fixes') return 'verdict-fixes';
    return 'verdict-block';
  }
  function sevClass(sev: string): string {
    return `sev-${sev.toLowerCase()}`;
  }
</script>

<div class="review-wrap">
  <div class="review-toolbar"><button class="btn small ghost" aria-label={listHidden ? 'Show reviews list' : 'Hide reviews list'} title={listHidden ? 'Show reviews list' : 'Hide reviews list'} aria-expanded={!listHidden} aria-controls="reviews-list" onclick={() => (listHidden = !listHidden)}><Icon name="sidebar" size={14} /></button></div>
<div class="lab-review" class:list-hidden={listHidden} data-testid="skill-review">
  <aside class="lr-side" id="reviews-list">
    <!-- Same list head as the Evaluator's runs. Not .primary: the form's
         "Start review" is the view's primary. -->
    <div class="lr-side-head">
      <span class="lr-side-title">Reviews</span>
      <button class="btn small" onclick={newReview} aria-pressed={!selected} title="New review" data-testid="new-skill-review"><Icon name="plus" size={12} /> New</button>
    </div>
    {#if listError && reviews.length === 0}
      <div class="lr-empty lr-list-err" role="alert">
        <span><Icon name="warning" size={12} /> <strong>Couldn't load reviews.</strong> <span class="lr-err-detail">{listError}</span></span>
        <button class="btn small" onclick={loadList}>Retry</button>
      </div>
    {:else if listLoading && reviews.length === 0}
      <p class="lr-empty" role="status">Loading reviews…</p>
    {:else if reviews.length === 0}
      <p class="lr-empty">No skill reviews yet. Start one with the form.</p>
    {:else}
      <ul class="lr-list">
        {#each reviews as r (r.id)}
          <li>
            <button class="lr-item" class:active={selected?.id === r.id} aria-current={selected?.id === r.id ? 'true' : undefined} onclick={() => openReview(r.id)}>
              <span class="lr-item-top">
                <span class="lr-item-name" title={r.skill_name}>{r.skill_name}</span>
                <span class="rp-status-pill" data-status={r.status}><StatusBadge status={runStatus(r.status)} variant="text" /></span>
              </span>
              <span class="lr-item-meta">
                <span>{sourceLabel(r.skill_source)}{r.static_report ? ` · ${r.static_report.verdict}` : ''}</span>
                <span class="grow"></span>
                <span title={new Date(r.created_at).toLocaleString()}>{rel(r.created_at)}</span>
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </aside>

  <main class="lr-main">
    {#if !selected}
      <!-- New review form -->
      <div class="lr-form card">
        <h3>New skill review</h3>
        <p class="lr-hint">
          A deterministic static pass runs instantly. In <strong>agents</strong> mode, N provider
          agents run the bundled <code>skills-reviewer</code> method — their shells embed live below —
          and a summarizer folds everything into one ranked report.
        </p>
        <label class="lr-field">
          <span>Skill under review</span>
          <select class="input" bind:value={fSkill} data-testid="skill-review-select">
            <option value="" disabled>Pick a skill…</option>
            {#each skillOpts as o (o.source + ':' + o.name)}
              <option value={o.source + ':' + o.name}>{o.label}</option>
            {/each}
          </select>
        </label>
        <fieldset class="lr-field">
          <span>Mode</span>
          <label class="lr-radio"><input type="radio" bind:group={fMode} value="static" /> Static analysis only (fast, no agents)</label>
          <label class="lr-radio"><input type="radio" bind:group={fMode} value="agents" /> Static + review agents + summarizer</label>
        </fieldset>
        {#if fMode === 'agents'}
          <fieldset class="lr-field">
            <span>Reviewer agents</span>
            {#each agentProviders() as p (p)}
              <label class="lr-check">
                <input type="checkbox" checked={fProviders.has(p)} onchange={() => toggleReviewer(p)} /> {p}
              </label>
            {/each}
          </fieldset>
        {/if}
        <label class="lr-field">
          <span>Additional instructions <span class="hint-inline">optional</span></span>
          <textarea
            class="input lr-textarea"
            rows="3"
            bind:value={fInstructions}
            placeholder="Extra context for the reviewers — e.g. “check recent commits: they fix the previous review round”, known issues from earlier implementations…"
            data-testid="skill-review-instructions"
          ></textarea>
        </label>
        <div class="lr-actions">
          <span class="dim lr-cost">{fMode === 'static' ? 'No agents — runs instantly' : `${Math.max(1, fProviders.size)} review agent${fProviders.size === 1 ? '' : 's'} + a summarizer`}</span>
          <span class="grow"></span>
          <button class="btn primary" disabled={!fSkill || starting} title={!fSkill ? 'Pick a skill to review' : undefined} onclick={start} data-testid="start-skill-review">
            {starting ? 'Starting…' : 'Start review'}
          </button>
        </div>
      </div>
    {:else}
      <!-- Review detail -->
      <div class="lr-detail">
        <div class="lr-detail-head">
          <div>
            <h3>{selected.skill_name}</h3>
            <span class="chip lr-src">{sourceLabel(selected.skill_source)}</span>
            <span class="rp-status-pill" data-status={selected.status}><StatusBadge status={runStatus(selected.status)} /></span>
          </div>
          <div class="grow"></div>
          {#if selected.status === 'running'}
            <button class="btn small" onclick={cancelReview}><Icon name="square" size={12} /> Stop review</button>
          {/if}
          <button class="icon-btn" onclick={() => selected && deleteReview(selected)} aria-label="Delete this review" title="Delete this review"><Icon name="trash" size={14} /></button>
        </div>

        {#if selected.error}
          <p class="lr-error" role="alert"><Icon name="warning" size={12} /> {selected.error}</p>
        {/if}

        {#if selected.instructions}
          <p class="lr-instructions" title="Additional instructions this review ran with">
            <strong>Instructions:</strong> {selected.instructions}
          </p>
        {/if}

        <!-- Static analysis -->
        {#if selected.static_report}
          {@const sr = selected.static_report}
          <section class="card lr-static" data-testid="static-report">
            <div class="lr-verdict {verdictClass(sr.verdict)}">
              <strong>Static analysis</strong>
              <span class="lr-verdict-badge">{sr.verdict}</span>
              <span class="lr-avg">avg {sr.average_score.toFixed(1)}/5</span>
            </div>
            <table class="lr-score">
              <tbody>
                {#each sr.scorecard as row (row.area)}
                  <tr><td class="lr-area">{areaLabel(row.area)}</td><td class="lr-num">{row.score}/5</td><td class="lr-notes">{row.notes}</td></tr>
                {/each}
              </tbody>
            </table>
            {#if sr.findings.length > 0}
              <ul class="lr-findings">
                {#each sr.findings as f, i (i + f.code)}
                  <li class="rp-finding">
                    <span class="severity-chip {sevClass(f.severity)}">{f.severity}</span>
                    <span class="mono rp-loc">{f.code}</span>
                    <span class="rp-finding-body"><strong>{f.title}</strong> — {f.fix}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}

        <!-- Live agents -->
        {#if selected.agents.length > 0}
          <section class="lr-agents-sec">
            <h4>Review agents</h4>
            <SkillReviewAgents review={selected} view={selected.status === 'running' ? 'running' : 'done'} onretried={(r) => (selected = r)} />
          </section>
        {/if}

        <!-- Summarizer report -->
        {#if selected.summary}
          {@const sm = selected.summary}
          <section class="card lr-summary" data-testid="summary-report">
            <div class="lr-verdict {verdictClass(sm.verdict)}">
              <strong>Summary</strong>
              <span class="lr-verdict-badge">{sm.verdict}</span>
              <span class="lr-avg">avg {sm.average_score.toFixed(1)}/5</span>
            </div>
            {#if sm.patch_plan.length > 0}
              <h5>Patch plan</h5>
              <ol class="lr-plan">
                {#each sm.patch_plan as step, i (i)}<li>{step}</li>{/each}
              </ol>
            {/if}
            {#if sm.findings.length > 0}
              <h5>Findings ({sm.findings.length})</h5>
              <ul class="lr-findings">
                {#each sm.findings as f, i (i + f.code)}
                  <li class="rp-finding">
                    <span class="severity-chip {sevClass(f.severity)}">{f.severity}</span>
                    <span class="mono rp-loc">{f.code}</span>
                    <span class="rp-finding-body"><strong>{f.title}</strong>{f.fix ? ` — ${f.fix}` : ''}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}

        <!-- Apply fixes with an agent -->
        {#if selected.status === 'done' && (canApply || selected.fix_agent)}
          <section class="card lr-fix" data-testid="apply-fixes">
            <h4>Apply fixes</h4>
            {#if selected.fix_agent}
              {@const fx = selected.fix_agent}
              <div class="lr-fix-row">
                <span class="lr-fix-name">Fixer</span>
                <span class="chip">{fx.provider}</span>
                <span class="grow"></span>
                {#if fx.session_id}
                  <button class="btn small ghost" aria-expanded={fixTermOpen} onclick={() => (fixTermOpen = !fixTermOpen)}>
                    {fixTermOpen ? 'Hide session' : 'Open session'}
                  </button>
                {/if}
                <span class="rp-status-pill" data-status={fx.status}><StatusBadge status={runStatus(fx.status)} /></span>
              </div>
              {#if fx.note}
                <p class="lr-fix-note">{fx.note}</p>
              {/if}
              {#if fx.status === 'waiting'}
                <p class="lr-fix-waiting"><Icon name="warning" size={12} /> The fixer looks blocked on input. <strong>Open session</strong> to respond.</p>
              {/if}
              {#if fx.session_id && fixTermOpen}
                <div class="lr-fix-term">
                  <Terminal sessionId={fx.session_id} preferDom />
                </div>
              {/if}
            {/if}
            {#if canApply}
              {#if selected.skill_source === 'bundled'}
                <p class="lr-hint">Bundled skills are read-only — install the skill to the library first, then review and apply fixes there.</p>
              {:else}
                <p class="lr-hint">
                  Send the findings and patch plan to an agent that edits the {sourceLabel(selected.skill_source)} copy of the skill directly on disk.
                </p>
                <div class="lr-fix-form">
                  <select class="input" bind:value={fixProvider} title="Fixer agent" aria-label="Fixer agent">
                    {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
                  </select>
                  <input
                    type="text"
                    class="input lr-fix-input"
                    aria-label="Extra instructions for the fixer"
                    bind:value={fixInstructions}
                    placeholder="Extra instructions for the fixer (optional)…"
                  />
                  <button class="btn primary" disabled={applying} onclick={applyFixes} data-testid="apply-fixes-btn">
                    {applying ? 'Starting…' : selected.fix_agent ? 'Run again' : 'Apply fixes with agent'}
                  </button>
                </div>
              {/if}
            {/if}
          </section>
        {/if}
      </div>
    {/if}
  </main>
</div>
</div>

<style>
  .review-wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .review-toolbar { display: flex; align-items: center; min-height: 32px; padding-inline: 8px; border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .lab-review.list-hidden { grid-template-columns: minmax(0, 1fr); }
  .list-hidden .lr-side { display: none; }
  /* Same shell as Skills: a surface list pane with a hairline, content beside it. */
  .lab-review { display: grid; grid-template-columns: 280px minmax(0, 1fr); flex: 1; min-height: 0; }
  .lr-side { display: flex; flex-direction: column; gap: 4px; overflow-y: auto; padding: 0 8px 12px; background: var(--surface); border-inline-end: 1px solid var(--border); }
  .lr-side-head { display: flex; align-items: center; gap: 8px; padding: 12px 4px 8px; }
  .lr-side-title { flex: 1; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-dim); }
  .lr-empty { color: var(--text-dim); font-size: var(--fs-s); padding: 8px; }
  .lr-list-err { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; color: var(--text); overflow-wrap: anywhere; }
  .lr-list-err :global(svg), .lr-error :global(svg) { color: var(--danger); vertical-align: -1px; }
  .lr-err-detail { color: var(--text-dim); }
  .lr-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .lr-list li { display: flex; }
  .lr-item {
    flex: 1; min-width: 0; text-align: start; background: transparent; border: 1px solid transparent;
    border-radius: var(--radius-m); padding: 7px 9px; cursor: pointer; color: var(--text); display: flex; flex-direction: column; gap: 4px;
  }
  .lr-item:hover { background: var(--hover); }
  .lr-item.active { border-color: color-mix(in srgb, var(--accent) 28%, transparent); background: var(--accent-soft); }
  .lr-item-top { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .lr-item-name { flex: 1; min-width: 0; font-size: var(--fs-m); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lr-item-meta { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-dim); min-width: 0; }
  .lr-item-meta > span:first-child { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lr-src { font-size: var(--fs-xs); }
  .lr-main { min-width: 0; overflow-y: auto; min-height: 0; padding: 16px 20px; }
  .lr-form { padding: 16px; max-width: 620px; display: flex; flex-direction: column; gap: 12px; }
  .lr-form h3 { margin: 0; font-size: var(--fs-l); font-weight: 600; }
  .lr-actions { display: flex; align-items: center; gap: 8px; padding-top: 4px; }
  .lr-cost { font-size: var(--fs-xs); }
  .hint-inline { font-weight: 400; font-size: var(--fs-xs); }
  .hint-inline::before { content: '· '; }
  .lr-hint { font-size: var(--fs-s); color: var(--text-dim); line-height: 1.5; margin: 0; }
  .lr-field { display: flex; flex-direction: column; gap: 6px; border: none; margin: 0; padding: 0; }
  .lr-field > span { font-size: var(--fs-s); font-weight: 500; color: var(--text-dim); }
  .lr-radio, .lr-check { display: flex; align-items: center; gap: 7px; font-size: var(--fs-s); }
  .lr-textarea { resize: vertical; }
  .lr-instructions {
    margin: 0; font-size: var(--fs-s); color: var(--text-dim); line-height: 1.5;
    border-inline-start: 2px solid var(--border); padding: 2px 10px;
  }

  .lr-fix { padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
  .lr-fix h4 { margin: 0; }
  .lr-fix-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .lr-fix-name { font-size: var(--fs-s); font-weight: 600; }
  .lr-fix-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.4; }
  .lr-fix-waiting { margin: 0; font-size: var(--fs-xs); line-height: 1.45; color: var(--warning); }
  .lr-fix-term {
    height: min(360px, 65vh); border: 1px solid var(--border);
    border-radius: var(--radius-m); overflow: hidden; overscroll-behavior: contain; background: var(--term-bg);
  }
  .lr-fix-form { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .lr-fix-form select { width: auto; }
  .lr-fix-input { flex: 1; min-width: 180px; }

  .lr-detail { display: flex; flex-direction: column; gap: 12px; }
  .lr-detail-head { display: flex; align-items: center; gap: 8px; }
  .lr-detail-head > div:first-child { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
  .lr-detail-head h3 { margin: 0; font-size: var(--fs-l); font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lr-error { color: var(--danger); font-size: var(--fs-s); }
  .grow { flex: 1; }

  .lr-static, .lr-summary { padding: 12px 14px; }
  .lr-verdict { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }
  .lr-verdict-badge { font-size: var(--fs-xs); font-weight: 500; padding: 2px 8px; border-radius: 999px; }
  .verdict-ready .lr-verdict-badge { background: var(--success-soft); color: var(--success); }
  .verdict-fixes .lr-verdict-badge { background: var(--warning-soft); color: var(--warning); }
  .verdict-block .lr-verdict-badge { background: var(--danger-soft); color: var(--danger); }
  .lr-avg { font-size: var(--fs-xs); color: var(--text-dim); }
  .lr-score { width: 100%; border-collapse: collapse; font-size: var(--fs-xs); }
  .lr-score td { padding: 3px 6px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .lr-area { font-weight: 600; white-space: nowrap; }
  .lr-num { text-align: end; white-space: nowrap; color: var(--text-dim); }
  .lr-notes { color: var(--text-dim); }
  .lr-findings { list-style: none; margin: 10px 0 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .lr-agents-sec h4, .lr-summary h5 { margin: 8px 0 4px; }
  .lr-plan { margin: 4px 0 8px 18px; font-size: var(--fs-s); line-height: 1.5; }

  .rp-status-pill { display: inline-flex; align-items: center; }
  .rp-finding { display: grid; grid-template-columns: auto minmax(0, 1fr); align-items: baseline; gap: 6px; font-size: var(--fs-xs); line-height: 1.4; }
  .rp-finding-body { grid-column: 1 / -1; min-width: 0; overflow-wrap: anywhere; }
  .rp-loc { font-size: var(--fs-xs); color: var(--text-dim); overflow-wrap: anywhere; }
  .severity-chip { display: inline-block; padding: 1px 8px; border-radius: 999px; font-size: var(--fs-xs); font-weight: 500; text-transform: capitalize; }
  .sev-critical { background: var(--danger-soft); color: var(--danger); }
  .sev-high { background: var(--danger-soft); color: var(--danger); }
  .sev-medium { background: var(--warning-soft); color: var(--warning); }
  .sev-low { background: var(--info-soft); color: var(--info); }
  .mono { font-family: var(--font-mono); }

  /* Phone: the list stacks above the report, like the Evaluator's runs. */
  @media (max-width: 640px) {
    .lab-review { display: flex; flex-direction: column; }
    .lr-side { flex: none; max-height: 40%; border-inline-end: none; border-bottom: 1px solid var(--border); }
    .lr-main { flex: 1; }
    .lr-main { padding: 12px 14px; }
  }
</style>
