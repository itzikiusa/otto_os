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
  import SkillReviewAgents from './SkillReviewAgents.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';

  interface Props {
    wsId: string;
    /** Optional skill to pre-select in the New Review form (from the Skills tab). */
    initialTarget?: { name: string; source: string } | null;
    onconsumed?: () => void;
  }
  let { wsId, initialTarget = null, onconsumed }: Props = $props();

  // `source` is "library" | "bundled" | a provider name (claude/codex/agy) — all
  // valid `skill_source` values the review engine resolves.
  type SkillOpt = { name: string; source: string; label: string };

  let reviews = $state<SkillReview[]>([]);
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
    if (!wsId) return;
    try {
      reviews = await skillReviewApi.list(wsId);
    } catch (e) {
      toasts.error('Load reviews failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function openReview(id: string): Promise<void> {
    try {
      selected = await skillReviewApi.get(id);
    } catch (e) {
      toasts.error('Open review failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Start review failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Apply fixes failed', e instanceof Error ? e.message : String(e));
    } finally {
      applying = false;
    }
  }

  async function cancelReview(): Promise<void> {
    if (!selected) return;
    try {
      selected = await skillReviewApi.cancel(selected.id);
      await loadList();
    } catch (e) {
      toasts.error('Cancel failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteReview(rev: SkillReview): Promise<void> {
    try {
      await skillReviewApi.remove(rev.id);
      if (selected?.id === rev.id) selected = null;
      await loadList();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  function newReview(): void {
    selected = null;
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
    if (selected && skillReviewBus.reviewId === selected.id) void openReview(selected.id);
  });

  // Fallback poll while the open review is running (covers dropped sockets).
  let poll: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (poll) { clearInterval(poll); poll = null; }
    if (activeReview && selected) {
      const id = selected.id;
      poll = setInterval(() => { void openReview(id); }, 2500);
    }
  });
  onDestroy(() => { if (poll) clearInterval(poll); });

  // Load on workspace change.
  let loadedWs = '';
  $effect(() => {
    if (wsId && wsId !== loadedWs) {
      loadedWs = wsId;
      void loadSkills();
      void loadList();
    }
  });

  // Consume a cross-tab "review this skill" intent by pre-filling the form.
  let consumedTarget = '';
  $effect(() => {
    if (initialTarget && `${initialTarget.source}:${initialTarget.name}` !== consumedTarget) {
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

  function verdictClass(v: string): string {
    if (v === 'Ready') return 'verdict-ready';
    if (v === 'Ready with fixes') return 'verdict-fixes';
    return 'verdict-block';
  }
  function sevClass(sev: string): string {
    return `sev-${sev.toLowerCase()}`;
  }
</script>

<div class="lab-review" data-testid="skill-review">
  <aside class="lr-side">
    <button class="btn primary block" onclick={newReview} data-testid="new-skill-review">+ New review</button>
    {#if reviews.length === 0}
      <p class="lr-empty">No skill reviews yet.</p>
    {:else}
      <ul class="lr-list">
        {#each reviews as r (r.id)}
          <li>
            <button class="lr-item" class:active={selected?.id === r.id} onclick={() => openReview(r.id)}>
              <span class="lr-item-name">{r.skill_name}</span>
              <span class="lr-item-meta">
                <span class="chip lr-src">{r.skill_source}</span>
                <span class="rp-status-pill rp-status-{r.status}">{r.status}</span>
              </span>
            </button>
            <button class="btn small ghost lr-del" title="Delete" onclick={() => deleteReview(r)}>✕</button>
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
          <select bind:value={fSkill} data-testid="skill-review-select">
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
          <span>Additional instructions (optional)</span>
          <textarea
            class="lr-textarea"
            rows="3"
            bind:value={fInstructions}
            placeholder="Extra context for the reviewers — e.g. “check recent commits: they fix the previous review round”, known issues from earlier implementations…"
            data-testid="skill-review-instructions"
          ></textarea>
        </label>
        <button class="btn primary" disabled={!fSkill || starting} onclick={start} data-testid="start-skill-review">
          {starting ? 'Starting…' : 'Start review'}
        </button>
      </div>
    {:else}
      <!-- Review detail -->
      <div class="lr-detail">
        <div class="lr-detail-head">
          <div>
            <h3>{selected.skill_name}</h3>
            <span class="chip lr-src">{selected.skill_source}</span>
            <span class="rp-status-pill rp-status-{selected.status}">{selected.status}</span>
          </div>
          <div class="grow"></div>
          {#if selected.status === 'running'}
            <button class="btn small ghost" onclick={cancelReview}>Cancel</button>
          {/if}
        </div>

        {#if selected.error}
          <p class="lr-error">{selected.error}</p>
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
                  <tr><td class="lr-area">{row.area.replace(/_/g, ' ')}</td><td class="lr-num">{row.score}/5</td><td class="lr-notes">{row.notes}</td></tr>
                {/each}
              </tbody>
            </table>
            {#if sr.findings.length > 0}
              <ul class="lr-findings">
                {#each sr.findings as f (f.code + f.title)}
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
                {#each sm.findings as f (f.code + f.title)}
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
                <span class="lr-fix-name">fixer</span>
                <span class="chip">{fx.provider}</span>
                <span class="grow"></span>
                {#if fx.session_id}
                  <button class="btn small ghost" onclick={() => (fixTermOpen = !fixTermOpen)}>
                    {fixTermOpen ? 'Hide' : 'Open'}
                  </button>
                {/if}
                <span class="rp-status-pill rp-status-{fx.status}">{fx.status}</span>
              </div>
              {#if fx.note}
                <p class="lr-fix-note">{fx.note}</p>
              {/if}
              {#if fx.status === 'waiting'}
                <p class="lr-fix-waiting">⚠ The fixer looks blocked on input. Click <strong>Open</strong> to respond.</p>
              {/if}
              {#if fx.session_id && fixTermOpen}
                <div class="lr-fix-term">
                  <Terminal sessionId={fx.session_id} />
                </div>
              {/if}
            {/if}
            {#if canApply}
              {#if selected.skill_source === 'bundled'}
                <p class="lr-hint">Bundled skills are read-only — install the skill to the library first, then review and apply fixes there.</p>
              {:else}
                <p class="lr-hint">
                  Send the findings and patch plan to an agent that applies them directly to the skill directory.
                </p>
                <div class="lr-fix-form">
                  <select bind:value={fixProvider} title="Fixer provider">
                    {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
                  </select>
                  <input
                    type="text"
                    class="lr-fix-input"
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

<style>
  .lab-review { display: grid; grid-template-columns: 260px 1fr; gap: 12px; height: 100%; min-height: 0; }
  .lr-side { display: flex; flex-direction: column; gap: 8px; overflow-y: auto; }
  .block { width: 100%; }
  .lr-empty { color: var(--text-dim); font-size: 12.5px; padding: 8px; }
  .lr-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .lr-list li { display: flex; align-items: center; gap: 4px; }
  .lr-item {
    flex: 1; min-width: 0; text-align: left; background: transparent; border: 1px solid var(--border);
    border-radius: var(--radius-m); padding: 7px 9px; cursor: pointer; color: var(--text); display: flex; flex-direction: column; gap: 4px;
  }
  .lr-item.active { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .lr-item-name { font-size: 12.5px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lr-item-meta { display: flex; align-items: center; gap: 6px; }
  .lr-src { font-size: 10px; }
  .lr-del { flex-shrink: 0; }
  .lr-main { overflow-y: auto; min-height: 0; }
  .lr-form { padding: 16px; max-width: 620px; display: flex; flex-direction: column; gap: 12px; }
  .lr-form h3 { margin: 0; }
  .lr-hint { font-size: 12px; color: var(--text-dim); line-height: 1.5; margin: 0; }
  .lr-field { display: flex; flex-direction: column; gap: 6px; border: none; margin: 0; padding: 0; }
  .lr-field > span { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-dim); }
  .lr-field select { padding: 8px; border-radius: var(--radius-m); border: 1px solid var(--border); background: var(--surface); color: var(--text); }
  .lr-radio, .lr-check { display: flex; align-items: center; gap: 7px; font-size: 12.5px; }
  .lr-textarea {
    padding: 8px; border-radius: var(--radius-m); border: 1px solid var(--border);
    background: var(--surface); color: var(--text); font: inherit; font-size: 12.5px; resize: vertical;
  }
  .lr-instructions {
    margin: 0; font-size: 12px; color: var(--text-dim); line-height: 1.5;
    border-inline-start: 2px solid var(--border); padding: 2px 10px;
  }

  .lr-fix { padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
  .lr-fix h4 { margin: 0; }
  .lr-fix-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .lr-fix-name { font-size: 12.5px; font-weight: 600; }
  .lr-fix-note { margin: 0; font-size: 11.5px; color: var(--text-dim); line-height: 1.4; }
  .lr-fix-waiting { margin: 0; font-size: 11.5px; line-height: 1.45; color: var(--status-warn); }
  .lr-fix-term {
    height: min(360px, 65vh); border: 1px solid var(--border);
    border-radius: var(--radius-m); overflow: hidden; overscroll-behavior: contain; background: var(--term-bg);
  }
  .lr-fix-form { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .lr-fix-form select {
    padding: 7px 8px; border-radius: var(--radius-m); border: 1px solid var(--border);
    background: var(--surface); color: var(--text);
  }
  .lr-fix-input {
    flex: 1; min-width: 180px; padding: 7px 9px; border-radius: var(--radius-m);
    border: 1px solid var(--border); background: var(--surface); color: var(--text); font-size: 12.5px;
  }

  .lr-detail { display: flex; flex-direction: column; gap: 12px; }
  .lr-detail-head { display: flex; align-items: center; gap: 8px; }
  .lr-detail-head h3 { margin: 0 8px 0 0; display: inline; }
  .lr-error { color: var(--status-exited); font-size: 12px; }
  .grow { flex: 1; }

  .lr-static, .lr-summary { padding: 12px 14px; }
  .lr-verdict { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }
  .lr-verdict-badge { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em; padding: 3px 8px; border-radius: var(--radius-s); }
  .verdict-ready .lr-verdict-badge { background: color-mix(in srgb, var(--status-working) 18%, transparent); color: var(--status-working); }
  .verdict-fixes .lr-verdict-badge { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .verdict-block .lr-verdict-badge { background: color-mix(in srgb, var(--status-exited) 18%, transparent); color: var(--status-exited); }
  .lr-avg { font-size: 11.5px; color: var(--text-dim); }
  .lr-score { width: 100%; border-collapse: collapse; font-size: 11.5px; }
  .lr-score td { padding: 3px 6px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .lr-area { font-weight: 600; white-space: nowrap; }
  .lr-num { text-align: right; white-space: nowrap; color: var(--text-dim); }
  .lr-notes { color: var(--text-dim); }
  .lr-findings { list-style: none; margin: 10px 0 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .lr-agents-sec h4, .lr-summary h5 { margin: 8px 0 4px; }
  .lr-plan { margin: 4px 0 8px 18px; font-size: 12.5px; line-height: 1.5; }

  .rp-status-pill { font-size: 10px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase; padding: 2px 6px; border-radius: var(--radius-s, 4px); display: inline-flex; align-items: center; gap: 3px; }
  .rp-status-pending { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }
  .rp-status-running { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }
  .rp-status-done { background: color-mix(in srgb, var(--status-working) 15%, transparent); color: var(--status-working); }
  .rp-status-error, .rp-status-cancelled { background: color-mix(in srgb, var(--status-exited) 15%, transparent); color: var(--status-exited); }
  .rp-finding { display: flex; align-items: baseline; gap: 6px; font-size: 11.5px; line-height: 1.4; }
  .rp-finding-body { flex: 1; min-width: 0; }
  .rp-loc { font-size: 11px; color: var(--text-dim); white-space: nowrap; }
  .severity-chip { display: inline-block; padding: 2px 7px; border-radius: var(--radius-s, 4px); font-size: 10.5px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase; }
  .sev-critical { background: color-mix(in srgb, var(--status-exited) 22%, transparent); color: var(--status-exited); }
  .sev-high { background: color-mix(in srgb, var(--status-exited) 15%, transparent); color: var(--status-exited); }
  .sev-medium { background: color-mix(in srgb, var(--status-warn) 15%, transparent); color: var(--status-warn); }
  .sev-low { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }
  .mono { font-family: var(--font-mono, monospace); }

  @media (max-width: 900px) {
    .lab-review { grid-template-columns: 1fr; }
  }
</style>
