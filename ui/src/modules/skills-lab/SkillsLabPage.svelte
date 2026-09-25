<script lang="ts">
  // Skills Lab — the umbrella page. Three sections (routes):
  //   • Skills    `#/skills-eval`           — every skill grouped by name across
  //                                            Library / Claude / Codex / Bundled,
  //                                            with Overview · Edit · Evals · Usage
  //   • Review    `#/skills-eval/review`    — multi-agent skill review
  //   • Evaluator `#/skills-eval/evaluator` — the Skills Evaluator (Runs /
  //                                            Golden Tasks / Matrix), unchanged
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import SkillsBrowser from './SkillsBrowser.svelte';
  import SkillReviewPanel from './SkillReviewPanel.svelte';
  import SkillsEvalPage from '../skills-eval/SkillsEvalPage.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
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
  function onTabKey(e: KeyboardEvent): void {
    const i = TABS.findIndex((t) => t.id === tab);
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % TABS.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + TABS.length) % TABS.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    const to = TABS[next].id;
    go(to);
    // The route changes on the next hashchange: move focus to the target tab now.
    (document.querySelector(`[data-testid="tab-${to}"]`) as HTMLElement | null)?.focus();
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
</script>

<div class="skills-lab">
  <PageHeader title="Skills Lab" subtitle={tab === 'skills' ? 'Every skill your agents can load, in one place' : undefined}>
    {#snippet leading()}
      {#if tab === 'skills' && !viewport.isPhone}
        <button class="icon-btn" onclick={() => (listCollapsed = !listCollapsed)} aria-label={listCollapsed ? 'Show skills list' : 'Hide skills list'} title={listCollapsed ? 'Show skills list' : 'Hide skills list'} aria-expanded={!listCollapsed} aria-controls="skills-list-pane"><Icon name="sidebar" size={16} /></button>
      {/if}
      {#if tab === 'skills' && phoneDetail}
        <button class="icon-btn" onclick={() => browser?.back()} aria-label="Back to skills" title="Back to skills"><Icon name="chevronLeft" size={16} /></button>
      {/if}
    {/snippet}
    {#snippet tabs()}
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
    {/snippet}
    {#snippet actions()}
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

  <div class="lab-body">
    {#if tab === 'skills'}
      <SkillsBrowser {listCollapsed} bind:this={browser} onreview={reviewSkill} onevaluate={evaluateSkill} onopenrun={openRun} onempty={(empty) => (canCreate = !empty)} onphonedetail={(o) => (phoneDetail = o)} onopenreview={openReview} />
    {:else if tab === 'review'}
      <SkillReviewPanel {wsId} initialTarget={reviewTarget} onconsumed={() => (reviewTarget = null)} initialReview={reviewOpen} onreviewconsumed={() => (reviewOpen = null)} />
    {:else}
      <SkillsEvalPage initialSkill={evalTarget} onconsumed={() => (evalTarget = null)} initialRun={runTarget} onrunconsumed={() => (runTarget = null)} />
    {/if}
  </div>
</div>

<style>
  .skills-lab {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .lab-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
