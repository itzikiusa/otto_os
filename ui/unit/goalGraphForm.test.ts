import assert from 'node:assert/strict';
import { test } from 'node:test';
import { componentFunctions } from './componentFunctions.ts';
import { templateDraft, researchDraft } from '../src/modules/loops/templates.ts';
import { graphError, initialGraph, restoreGraph } from '../src/modules/loops/graph.ts';

function fixture() {
  const creates: any[] = [];
  const calls: string[] = [];
  const state = componentFunctions(new URL('../src/modules/loops/GoalDefineForm.svelte', import.meta.url),
    ['chooseTemplate', 'applyDraft', 'setMode', 'setExecution', 'canLaunch', 'launch', 'formState'], {
      selectedTemplate: '', pocScope: 'both', draftSource: 'agent', draft: null, seed: '', mode: 'build', execution: 'graph', savedGraph: null,
      name: '', repoPath: '/tmp/repo', ws: { currentId: 'ws' }, alive: true, formGeneration: 0, defining: false,
      launching: false, feedback: '', allowCommits: false, requireReview: false, sourceLinks: '', selectedSkills: '',
      maxIterations: 5, maxMinutes: 30, perPhaseMinutes: 10, executorCount: 1, execProvider: 'codex', execModel: '',
      defProvider: 'codex', defModel: '', templateDraft, researchDraft, graphError, initialGraph, restoreGraph, initialSnapshot: '', defaultAgentProvider: () => 'codex',
      confirmer: { ask: async () => true }, loops: { create: async (_ws: string, req: any) => { creates.push(req); return { id: 'created' }; }, define: () => { calls.push('define'); } },
      oncreated: (id: string) => calls.push(id), toastError: () => calls.push('error'),
    });
  return { state, creates, calls };
}

test('choosing a template is local, requires a concrete goal and persists the edited graph on launch', async () => {
  const { state, creates, calls } = fixture();
  await state.chooseTemplate('general');
  assert.equal(calls.length, 0);
  assert.equal(state.canLaunch(), false);
  state.seed = 'Stream the CSV export without buffering all rows';
  state.draft.suggested_config.graph.nodes[1].title = 'Stream rows';
  assert.equal(state.canLaunch(), true);
  await state.launch();
  assert.equal(creates.length, 1);
  assert.equal(creates[0].config.graph.nodes[1].title, 'Stream rows');
  assert.match(creates[0].definition.summary, /Stream the CSV export/);
  assert.deepEqual(calls, ['created']);
});

test('execution choice preserves edited tasks when switching to a goal loop and back', async () => {
  const { state } = fixture();
  await state.chooseTemplate('general');
  state.seed = 'Keep my concrete goal';
  state.draft.suggested_config.graph.nodes[1].title = 'Edited task';
  await state.setExecution('loop');
  assert.equal(state.draft.suggested_config.graph, null);
  assert.equal(state.seed, 'Keep my concrete goal');
  await state.setExecution('graph');
  assert.equal(state.draft.suggested_config.graph.nodes[1].title, 'Edited task');
  state.confirmer.ask = async () => false;
  await state.setExecution('loop');
  assert.equal(state.execution, 'graph');
});

test('templates respect explicit whole-goal mode and keep task structure available', async () => {
  const { state, creates } = fixture();
  state.execution = 'loop';
  await state.chooseTemplate('go');
  assert.equal(state.draft.suggested_config.graph, null);
  assert.equal(state.savedGraph.nodes.length, 3);
  state.seed = 'Add tenant-scoped pagination';
  await state.launch();
  assert.equal(creates[0].config.graph, null);
});

test('a new agent draft cannot inherit the previous draft task plan', async () => {
  const { state } = fixture();
  await state.chooseTemplate('general');
  await state.setExecution('loop');
  const fresh = templateDraft('go', 'codex');
  fresh.suggested_config.graph = null;
  fresh.definition.acceptance_criteria = [{ id: 'new', text: 'New goal behavior', verify: 'New evidence', verify_kind: 'agent' }];
  state.applyDraft(fresh);
  await state.setExecution('graph');
  assert.equal(state.draft.suggested_config.graph.nodes.length, 1);
  assert.deepEqual(Array.from(state.draft.suggested_config.graph.nodes[0].criterion_ids), ['new']);
});

test('restoring the same draft drops removed criterion links and requests reassignment', async () => {
  const { state } = fixture();
  await state.chooseTemplate('general');
  await state.setExecution('loop');
  state.draft.definition.acceptance_criteria.splice(1, 1);
  await state.setExecution('graph');
  assert.equal(state.draft.suggested_config.graph.nodes[1].criterion_ids.length, 0);
  assert.match(graphError(state.draft.suggested_config.graph, state.draft.definition.acceptance_criteria)!, /associate/i);
});

test('declining replacement preserves graph edits and template identity', async () => {
  const { state } = fixture();
  await state.chooseTemplate('go');
  state.draft.suggested_config.graph.nodes[0].title = 'My task';
  state.confirmer.ask = async () => false;
  await state.chooseTemplate('angular');
  assert.equal(state.selectedTemplate, 'go');
  assert.equal(state.draft.suggested_config.graph.nodes[0].title, 'My task');
});

test('cycle, missing repository and too-small graph budget block launch', async () => {
  const { state, creates } = fixture();
  await state.chooseTemplate('general'); state.seed = 'Concrete feature';
  state.draft.suggested_config.graph.nodes[0].depends_on = ['task-3'];
  assert.equal(state.canLaunch(), false);
  state.draft.suggested_config.graph.nodes[0].depends_on = [];
  state.maxIterations = 2;
  assert.equal(state.canLaunch(), false);
  state.maxIterations = 5; state.repoPath = '';
  assert.equal(state.canLaunch(), false);
  await state.launch(); assert.equal(creates.length, 0);
});

test('template confirmation cannot replace a form after navigation', async () => {
  const { state } = fixture();
  await state.chooseTemplate('go');
  let confirm!: (value: boolean) => void;
  state.confirmer.ask = () => new Promise<boolean>(resolve => { confirm = resolve; });
  const selecting = state.chooseTemplate('angular');
  state.alive = false; state.formGeneration++;
  confirm(true); await selecting;
  assert.equal(state.selectedTemplate, 'go');
});

test('graph tasks retry through Resume so dependency verification cannot be bypassed', () => {
  const state = componentFunctions(new URL('../src/modules/loops/IterationRow.svelte', import.meta.url), ['canRetry'], { loopStatus: 'blocked', hasGraph: true });
  assert.equal(state.canRetry({ status: 'error' }), false);
  state.hasGraph = false;
  assert.equal(state.canRetry({ status: 'error' }), true);
});

test('switching a graph to Research exposes the required report criterion on the final task', async () => {
  const { state, creates } = fixture();
  await state.chooseTemplate('general');
  state.setMode('research'); state.seed = 'Compare the export designs'; state.repoPath = '';
  assert.equal(state.canLaunch(), true);
  assert.ok(state.draft.definition.acceptance_criteria.some((c: any) => c.id === 'otto-research-report'));
  assert.ok(state.draft.suggested_config.graph.nodes.at(-1).criterion_ids.includes('otto-research-report'));
  await state.launch();
  assert.equal(creates[0].config.mode, 'research');
});
