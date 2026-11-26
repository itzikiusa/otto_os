// The Goal form's "Start from…" chips (goal.md §3.3): seven kinds of goal,
// each a set of criteria an agent checks and a budget. The text is the Goal
// Loops templates' (modules/loops/templates.ts), unchanged.
import type { Criterion } from '../../../lib/api/types';

export type GoalTemplateId = 'angular' | 'go' | 'general' | 'research' | 'poc' | 'rewrite' | 'performance';
export type PocScope = 'backend' | 'frontend' | 'both';
type Stage = { title: string; instructions: string; criterion: string; verify: string };
type Template = { id: GoalTemplateId; title: string; description: string; example: string; minutes: number; stages: Stage[] };

export const goalTemplates: Template[] = [
  { id: 'angular', title: 'Angular feature', description: 'Trace the API and state flow, implement the feature, and verify production behavior and visuals.', example: 'Add a status filter to the orders page, using the existing customers filter as the reference.', minutes: 90, stages: [
    { title: 'Trace the feature', instructions: 'Find the closest working sibling in this section and use it as a reference. Trace API, facade, state and view contracts; record the intended behavior and affected callers before editing.', criterion: 'The feature contract and reference implementation are recorded.', verify: 'Cite the actual API, state and view paths, the reference screen and the agreed success/error behavior.' },
    { title: 'Implement the feature', instructions: 'Implement the requested behavior using the existing section conventions and shared components. Cover the changed critical path, including loading, empty, error and loaded states.', criterion: 'The requested user flow works through the real API and state path.', verify: 'Exercise the intended flow and a realistic failure; explain why the regression check fails before the change.' },
    { title: 'Verify the UI', instructions: 'Run the affected production build and relevant tests. Inspect real and degraded states at desktop and phone widths, in light and dark. Fix concrete regressions and record evidence.', criterion: 'Production compilation, critical checks and visual acceptance pass.', verify: 'Record exact commands/results and observed reference-aligned layouts, responsive behavior and degraded states. Unavailable material checks remain unmet.' },
  ] },
  { id: 'go', title: 'Go feature', description: 'Follow the handler, service and data boundaries, then verify behavior and failure paths.', example: 'Add tenant-scoped pagination to the audit endpoint without changing existing response fields.', minutes: 75, stages: [
    { title: 'Trace the contract', instructions: 'Trace the exact handler, service, DAO and tenant boundary. Identify callers, compatibility constraints and the smallest causal change.', criterion: 'The request, response and data boundaries are explicit.', verify: 'Cite actual code and consumers, tenant scoping and realistic failure behavior.' },
    { title: 'Implement and test', instructions: 'Make a cohesive implementation matching local Go conventions. Add critical behavior and failure tests that falsify the old behavior; preserve compatibility.', criterion: 'The requested behavior and critical failure paths work.', verify: 'Run focused tests covering the original defect or missing behavior and the changed boundary. Record commands and output.' },
    { title: 'Verify affected consumers', instructions: 'Review the changed paths independently and run each affected repository’s prescribed gate once for the changed revision. Re-run only when code or evidence changes.', criterion: 'Affected consumers and repository checks pass.', verify: 'Record repository, revision, commands and results. Resolve concrete high/medium findings or leave this criterion unmet.' },
  ] },
  { id: 'general', title: 'General engineering', description: 'Inspect the causal path, make a cohesive change, and independently verify the result.', example: 'Make export stream rows instead of buffering the full result, while preserving the CSV contract.', minutes: 75, stages: [
    { title: 'Understand the change', instructions: 'Inspect the existing behavior, callers and repository guidance. Record a bounded implementation plan and the critical acceptance checks.', criterion: 'The causal scope and acceptance checks are grounded in current code.', verify: 'Cite affected paths and consumers, expected behavior and a concrete way to falsify an incorrect implementation.' },
    { title: 'Implement the change', instructions: 'Implement the requested behavior in cohesive batches. Preserve unrelated user work and add focused regression coverage for changed critical paths.', criterion: 'The requested behavior is implemented with meaningful regression coverage.', verify: 'Exercise the actual changed path, including the important failure case, and show the test distinguishes the old behavior.' },
    { title: 'Review and verify', instructions: 'Independently trace the final change and run relevant prescribed checks. Account for every concrete review finding with evidence; do not invent a finding quota or target score.', criterion: 'Final review and relevant checks support completion.', verify: 'Record commands, outcomes and finding dispositions. Missing material verification or open high/medium issues leaves the criterion unmet.' },
  ] },
  { id: 'research', title: 'Research', description: 'Gather primary evidence, compare options, and write a decision-ready report.', example: 'Compare three streaming approaches for the export service and recommend one against our memory and compatibility constraints.', minutes: 60, stages: [
    { title: 'Collect evidence', instructions: 'Bound the research question and collect primary sources or inspected repository evidence. Record dates and distinguish observations from assumptions.', criterion: 'Relevant primary evidence and constraints are recorded.', verify: 'Check cited sources support each key fact and identify important evidence gaps.' },
    { title: 'Compare options', instructions: 'Compare feasible alternatives against the user’s constraints. State tradeoffs and uncertainty; do not turn unsupported assumptions into facts.', criterion: 'Options are compared against explicit decision criteria.', verify: 'Trace the comparison back to evidence and show why rejected options fail or cost more under the stated constraints.' },
    { title: 'Write the recommendation', instructions: 'Write findings.md with a recommendation, citations, limitations and concrete next steps. A supported conclusion that evidence is insufficient is valid; never fabricate certainty. Verify the report against the sources. Do not modify application code or publish.', criterion: 'A cited, actionable report answers the research question.', verify: 'Inspect the retained report, citations and unresolved limitations; ensure the recommendation or insufficient-evidence conclusion follows from the evidence.' },
  ] },
  { id: 'poc', title: 'Proof of concept', description: 'Test one feasibility question with a runnable slice and a clear result.', example: 'Prove that an SSE feed can update the dashboard within one second for 100 connected clients.', minutes: 90, stages: [] },
  { id: 'rewrite', title: 'Rewrite', description: 'Inventory behavior, migrate a bounded slice, and compare old and new behavior.', example: 'Replace the export implementation while preserving every documented format and error response.', minutes: 120, stages: [
    { title: 'Inventory behavior', instructions: 'Build an old/new feature and behavior matrix from the current implementation and tests. Preserve assertions and define the migration boundary and rollback path.', criterion: 'The behavior matrix and migration boundary are recorded.', verify: 'Compare the matrix with actual code, callers and assertions; identify compatibility and performance constraints.' },
    { title: 'Migrate a slice', instructions: 'Implement a bounded vertical slice, preserving existing assertions and external contracts. Avoid unrelated rewrites. Keep old/new comparison runnable.', criterion: 'The replacement slice preserves the required behavior.', verify: 'Run the same meaningful inputs and failure cases against old and new implementations; report differences explicitly.' },
    { title: 'Verify parity', instructions: 'Complete the behavior matrix with regression and relevant performance comparisons. Review changed consumers and document remaining scope honestly.', criterion: 'Behavior parity and relevant performance comparisons support migration.', verify: 'Record completed matrix entries, exact regression commands and comparable performance results. Missing required parity stays unmet.' },
  ] },
  { id: 'performance', title: 'Performance', description: 'Measure a representative workload, optimize the observed cause, and compare results.', example: 'Reduce export peak memory for one million rows while keeping throughput and output unchanged.', minutes: 90, stages: [
    { title: 'Measure the baseline', instructions: 'Define a reproducible target workload and baseline before editing. Profile the actual bottleneck; record environment, concurrency, latency/throughput, CPU and memory over time.', criterion: 'A reproducible baseline identifies the measured bottleneck.', verify: 'Inspect raw measurements, workload and environment. Include CPU/RAM versus concurrent work where relevant; do not substitute a source guess for profiling.' },
    { title: 'Optimize the cause', instructions: 'Make the smallest cohesive change addressing the measured cause. Preserve behavior and add a focused regression check; avoid speculative optimization.', criterion: 'The change addresses the measured cause without changing required behavior.', verify: 'Trace the causal improvement and run the relevant correctness and failure checks.' },
    { title: 'Compare and review', instructions: 'Repeat the same workload and environment after the change. Compare performance, CPU and memory over time, including realistic concurrency. Report regressions and uncertainty.', criterion: 'Comparable measurements demonstrate the requested improvement.', verify: 'Record baseline/after values, commands, environment and correctness results. A missed target, incomparable run or material regression remains unmet.' },
  ] },
];

function pocStages(scope: PocScope): Stage[] {
  const surface = scope === 'both' ? 'backend and frontend' : scope;
  return [
    { title: 'Define the experiment', instructions: `Define the ${surface} feasibility question, a small runnable slice and measurable success/failure conditions. List assumptions and explicitly excluded production work.`, criterion: `The ${surface} experiment has measurable success and failure conditions.`, verify: 'Inspect the experiment plan, target workload or user flow, assumptions and a concrete way the hypothesis could fail.' },
    { title: 'Build the runnable slice', instructions: `Build only the ${surface} slice needed for the experiment. ${scope === 'backend' ? 'Exercise the actual endpoint and storage/integration boundary.' : scope === 'frontend' ? 'Exercise the real interaction, responsive layout and explicit service mocks.' : 'Exercise the actual interaction through its API boundary.'} Keep setup reproducible.`, criterion: `The ${surface} experiment runs with documented setup.`, verify: 'Run the documented setup and the main flow. Record what is real versus mocked and capture the meaningful failure behavior.' },
    { title: 'Evaluate feasibility', instructions: 'Measure the experiment against the original conditions. Write a concise result with evidence, limitations and a continue/change/stop recommendation; do not describe a POC as production-ready.', criterion: 'The feasibility result is supported by observed evidence.', verify: 'Compare actual observations with the original conditions; record limitations and the next decision. A demonstrated or falsified hypothesis is valid; unsupported claims or an unexecuted experiment remain unmet.' },
  ];
}

/** A template's criteria (each checked by an agent) and its budget. */
export function templateGoal(id: GoalTemplateId, pocScope: PocScope = 'both'): { criteria: Criterion[]; iterations: number; activeS: number; research: boolean } {
  const template = goalTemplates.find((item) => item.id === id)!;
  const stages = id === 'poc' ? pocStages(pocScope) : template.stages;
  return {
    criteria: stages.map((stage, i) => ({ id: `c${i + 1}`, text: stage.criterion, verify: 'agent', how: stage.verify })),
    iterations: stages.length + 2,
    activeS: template.minutes * 60,
    research: id === 'research',
  };
}
