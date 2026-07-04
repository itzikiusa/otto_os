<script lang="ts">
  // Skills Lab → Review. A list of skill reviews + a detail pane that mirrors the
  // code-review UX: a deterministic static-analysis card, N visible embedded
  // agent terminals (SkillReviewAgents), and the summarizer's aggregated report.
  // Live-refreshes on the skill_review_updated bus, with a fallback poll while a
  // review is running.
  import { onDestroy } from 'svelte';
  import type { LibrarySkill, BundledSkillView, SkillReview } from '../../lib/api/types';
  import { skillReviewApi } from '../../lib/api/skillReview';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { skillReviewBus } from '../../lib/events.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import SkillReviewAgents from './SkillReviewAgents.svelte';

  interface Props {
    wsId: string;
    /** Optional skill to pre-select in the New Review form (from the Skills tab). */
    initialTarget?: { name: string; source: string } | null;
    onconsumed?: () => void;
  }
  let { wsId, initialTarget = null, onconsumed }: Props = $props();

  type SkillOpt = { name: string; source: 'library' | 'bundled'; label: string };

  let reviews = $state<SkillReview[]>([]);
  let selected = $state<SkillReview | null>(null);
  let skillOpts = $state<SkillOpt[]>([]);

  // New-review form.
  let fSkill = $state('');
  let fMode = $state<'static' | 'agents'>('agents');
  let fClaude = $state(true);
  let fCodex = $state(false);
  let starting = $state(false);

  const activeReview = $derived(selected && selected.status === 'running');

  async function loadSkills(): Promise<void> {
    try {
      const [lib, bundled] = await Promise.all([
        skillLabApi.listLibrary().catch(() => [] as LibrarySkill[]),
        skillLabApi.listBundled().catch(() => [] as BundledSkillView[]),
      ]);
      const opts: SkillOpt[] = [];
      for (const s of lib) opts.push({ name: s.name, source: 'library', label: `${s.name} · library` });
      const libNames = new Set(lib.map((s) => s.name));
      for (const b of bundled)
        if (!libNames.has(b.name)) opts.push({ name: b.name, source: 'bundled', label: `${b.name} · bundled` });
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
      if (fClaude) providers.push('claude');
      if (fCodex) providers.push('codex');
      if (providers.length === 0) providers.push('claude');
    }
    starting = true;
    try {
      const rev = await skillReviewApi.start(wsId, {
        skill_name: opt.name,
        skill_source: opt.source,
        providers,
        agent_mode: fMode,
      });
      selected = rev;
      await loadList();
    } catch (e) {
      toasts.error('Start review failed', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
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
          { name: initialTarget.name, source: initialTarget.source as 'library' | 'bundled', label: `${initialTarget.name} · ${initialTarget.source}` },
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
            <label class="lr-check"><input type="checkbox" bind:checked={fClaude} /> claude</label>
            <label class="lr-check"><input type="checkbox" bind:checked={fCodex} /> codex</label>
          </fieldset>
        {/if}
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
