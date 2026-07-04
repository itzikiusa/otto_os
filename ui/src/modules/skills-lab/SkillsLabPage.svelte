<script lang="ts">
  // Skills Lab — the umbrella page. Three sections:
  //   • Skills    — full view + editor (add/edit/import/delete) with Review/Evaluate actions
  //   • Review    — multi-agent skill review with visible embedded agent shells + summarizer
  //   • Evaluator — the existing Skills Evaluator (Runs / Golden Tasks / Matrix), unchanged
  import { ws } from '../../lib/stores/workspace.svelte';
  import SkillsBrowser from './SkillsBrowser.svelte';
  import SkillReviewPanel from './SkillReviewPanel.svelte';
  import SkillsEvalPage from '../skills-eval/SkillsEvalPage.svelte';

  type Tab = 'skills' | 'review' | 'evaluator';
  let tab = $state<Tab>('skills');

  // Cross-tab intent: "Review this skill" from the Skills tab pre-fills the
  // Review form and switches tabs.
  let reviewTarget = $state<{ name: string; source: string } | null>(null);

  function reviewSkill(name: string, source: string): void {
    reviewTarget = { name, source };
    tab = 'review';
  }
  function evaluateSkill(_name: string, _source: string): void {
    // The evaluator's start form picks the skill itself; just navigate there.
    tab = 'evaluator';
  }

  const wsId = $derived(ws.currentId ?? '');
</script>

<div class="skills-lab">
  <div class="lab-tabs" data-testid="lab-tabs">
    <button class="lab-tab" class:active={tab === 'skills'} onclick={() => (tab = 'skills')} data-testid="tab-skills">Skills</button>
    <button class="lab-tab" class:active={tab === 'review'} onclick={() => (tab = 'review')} data-testid="tab-review">Review</button>
    <button class="lab-tab" class:active={tab === 'evaluator'} onclick={() => (tab = 'evaluator')} data-testid="tab-evaluator">Evaluator</button>
  </div>

  <div class="lab-body">
    {#if tab === 'skills'}
      <SkillsBrowser onreview={reviewSkill} onevaluate={evaluateSkill} />
    {:else if tab === 'review'}
      <SkillReviewPanel {wsId} initialTarget={reviewTarget} onconsumed={() => (reviewTarget = null)} />
    {:else}
      <SkillsEvalPage />
    {/if}
  </div>
</div>

<style>
  .skills-lab { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .lab-tabs { display: flex; gap: 4px; padding: 10px 14px 0; border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .lab-tab {
    background: transparent; border: none; border-bottom: 2px solid transparent; color: var(--text-dim);
    padding: 8px 14px; cursor: pointer; font-size: 13px; font-weight: 600;
  }
  .lab-tab:hover { color: var(--text); }
  .lab-tab.active { color: var(--accent); border-bottom-color: var(--accent); }
  .lab-body { flex: 1; min-height: 0; overflow: hidden; padding: 12px 14px; }
</style>
