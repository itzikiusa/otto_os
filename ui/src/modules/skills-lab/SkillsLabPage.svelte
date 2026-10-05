<script lang="ts">
  // Skills Lab — the umbrella page. Three sections (routes):
  //   • Skills    `#/skills-eval`           — every skill grouped by name across
  //                                            Library / Claude / Codex / Bundled,
  //                                            with Overview · Edit · Evals · Usage
  //   • Review    `#/skills-eval/review`    — multi-agent skill review
  //   • Evaluator `#/skills-eval/evaluator` — the Skills Evaluator (Runs /
  //                                            Golden Tasks / Matrix), unchanged
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import SkillsBrowser from './SkillsBrowser.svelte';
  import SkillReviewPanel from './SkillReviewPanel.svelte';
  import SkillsEvalPage from '../skills-eval/SkillsEvalPage.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  type Tab = 'skills' | 'review' | 'evaluator';
  const tab = $derived<Tab>(router.parts[1] === 'review' ? 'review' : router.parts[1] === 'evaluator' ? 'evaluator' : 'skills');
  function go(t: Tab): void {
    if (t === tab) return;
    // Leaving Skills unmounts an open editor with unsaved edits: the editor's
    // router leave-guard (guardUnsaved) asks once — no second local prompt.
    router.go(t === 'skills' ? 'skills-eval' : `skills-eval/${t}`);
  }
  const TABS: { id: Tab; label: string }[] = [
    { id: 'skills', label: 'Skills' },
    { id: 'review', label: 'Review' },
    { id: 'evaluator', label: 'Evaluator' },
  ];
  // The Evaluator's own views ride in the SAME header tab row (after the
  // section tabs) instead of a second strip inside the body. Routed as
  // `#/skills-eval/evaluator[/golden|/matrix]`; SkillsEvalPage reads it.
  type EvalView = 'runs' | 'golden' | 'matrix';
  const EVAL_VIEWS: { id: EvalView; label: string; icon: 'zap' | 'target' | 'grid' }[] = [
    { id: 'runs', label: 'Runs', icon: 'zap' },
    { id: 'golden', label: 'Golden tasks', icon: 'target' },
    { id: 'matrix', label: 'Matrix', icon: 'grid' },
  ];
  const evalView = $derived<EvalView>(router.parts[2] === 'golden' ? 'golden' : router.parts[2] === 'matrix' ? 'matrix' : 'runs');
  function goEval(v: EvalView): void {
    if (v === evalView) return;
    router.go(v === 'runs' ? 'skills-eval/evaluator' : `skills-eval/evaluator/${v}`);
  }

  // Cross-tab intent: "Review this skill" from the Skills tab pre-fills the
  // Review form and switches tabs.
  let reviewTarget = $state<{ name: string; source: string } | null>(null);
  function reviewSkill(name: string, source: string): void {
    reviewTarget = { name, source };
    reviewOpen = null;
    go('review');
  }
  // "Reviews" on a skill's Usage tab: open that skill's latest review.
  let reviewOpen = $state<string | null>(null);
  function openReview(id: string): void {
    reviewTarget = null;
    reviewOpen = id;
    go('review');
  }
  // "Evaluate <skill>" opens the Evaluator's start form with that skill
  // pre-selected (it used to land on the form with whatever came first).
  let evalTarget = $state<{ name: string; source: string } | null>(null);
  function evaluateSkill(name: string, source: string): void {
    evalTarget = { name, source };
    go('evaluator');
  }

  // "Open run" from a skill's Evals tab: the Evaluator opens on that run.
  let runTarget = $state<string | null>(null);
  function openRun(id: string): void {
    runTarget = id;
    go('evaluator');
  }

  const wsId = $derived(ws.currentId ?? '');
  let browser = $state<ReturnType<typeof SkillsBrowser> | null>(null);
  let canCreate = $state(true);
  let phoneDetail = $state(false);
  let listCollapsed = $state(false);
  // The Evaluator only has a runs list on its Runs view.
  let evalListable = $state(true);
</script>

<div class="skills-lab">
  <PageHeader title="Skills Lab" subtitle={tab === 'skills' ? 'Every skill your agents can load, in one place' : undefined} tabsPlacement={tab === 'evaluator' ? 'below' : 'inline'}>
    {#snippet leading()}
      {#if ((tab === 'skills') || (tab === 'evaluator' && evalListable)) && !viewport.isPhone}
        {@const what = tab === 'skills' ? 'skills' : 'evaluations'}
        <button class="icon-btn" onclick={() => (listCollapsed = !listCollapsed)} aria-label={listCollapsed ? `Show ${what} list` : `Hide ${what} list`} title={listCollapsed ? `Show ${what} list` : `Hide ${what} list`} aria-expanded={!listCollapsed} aria-controls={tab === 'skills' ? 'skills-list-pane' : 'evaluations-list'}><Icon name="sidebar" size={16} /></button>
      {/if}
      {#if tab === 'skills' && phoneDetail}
        <button class="icon-btn" onclick={() => browser?.back()} aria-label="Back to skills" title="Back to skills"><Icon name="chevronLeft" size={16} /></button>
      {/if}
    {/snippet}
    {#snippet tabs()}
      <div class="lab-tabs">
        <div class="segmented" role="tablist" aria-label="Skills Lab section" data-testid="lab-tabs" tabindex="-1" onkeydown={onTabKey}>
          {#each TABS as t (t.id)}
            <button
              role="tab"
              aria-selected={tab === t.id}
              tabindex={tab === t.id ? 0 : -1}
              class:active={tab === t.id}
              onclick={() => go(t.id)}
              data-testid="tab-{t.id}">{t.label}</button
            >
          {/each}
        </div>
        {#if tab === 'evaluator'}
          <span class="lab-tabs-sep" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
          <div class="segmented" role="tablist" aria-label="Evaluator view" data-testid="eval-tabs" tabindex="-1" onkeydown={onTabKey}>
            {#each EVAL_VIEWS as v (v.id)}
              <button role="tab" aria-selected={evalView === v.id} aria-controls="eval-panel" tabindex={evalView === v.id ? 0 : -1} class:active={evalView === v.id} onclick={() => goEval(v.id)} data-testid="tab-{v.id}">
                <Icon name={v.icon} size={12} /> {v.label}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/snippet}
    {#snippet actions()}
      {#if auth.can('settings', 'admin')}
        <!-- Where this page's defaults live: the evaluator's agents/iterations
             (evaluator tab) or the installed library (skills tab). -->
        <button class="icon-btn" data-overflow="-1" data-icon="gear"
          data-label={tab === 'evaluator' ? 'Evaluator defaults' : 'Skill library settings'}
          onclick={() => router.go(tab === 'evaluator' ? 'settings/skill-eval' : 'settings/skills')}
          aria-label={tab === 'evaluator' ? 'Evaluator defaults' : 'Skill library settings'}
          title={tab === 'evaluator' ? 'Evaluator defaults (Settings → Skills evaluator)' : 'Installed skills (Settings → Skills)'}><Icon name="gear" size={14} /></button>
      {/if}
      {#if tab === 'skills'}
        <button class="btn small" data-overflow="-1" data-icon="download" data-label="Import a skill (.zip)…" onclick={() => browser?.openImport()} title="Import a skill package (.zip)">
          <Icon name="download" size={12} /> Import…
        </button>
        {#if canCreate}
          <button class="btn small primary" onclick={() => browser?.openNew()} data-testid="new-skill"><Icon name="plus" size={12} /> New skill</button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody fill padded={false}>
  <div class="lab-body">
    {#if tab === 'skills'}
      <SkillsBrowser {listCollapsed} bind:this={browser} onreview={reviewSkill} onevaluate={evaluateSkill} onopenrun={openRun} onempty={(empty) => (canCreate = !empty)} onphonedetail={(o) => (phoneDetail = o)} onopenreview={openReview} />
    {:else if tab === 'review'}
      <SkillReviewPanel {wsId} initialTarget={reviewTarget} onconsumed={() => (reviewTarget = null)} initialReview={reviewOpen} onreviewconsumed={() => (reviewOpen = null)} />
    {:else}
      <SkillsEvalPage initialSkill={evalTarget} onconsumed={() => (evalTarget = null)} initialRun={runTarget} onrunconsumed={() => (runTarget = null)} {listCollapsed} onlistable={(v) => (evalListable = v)} />
    {/if}
  </div>
  </PageBody>
</div>

<style>
  .skills-lab {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  /* Section tabs, then (on the Evaluator) its views: one header tab row. */
  .lab-tabs {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .lab-tabs-sep {
    display: inline-flex;
    color: var(--text-dim);
  }
  :global([dir='rtl']) .lab-tabs-sep {
    transform: scaleX(-1);
  }
  .lab-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
