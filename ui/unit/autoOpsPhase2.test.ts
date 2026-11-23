// Unit tests for Phase 2 Deep Push deliverables in auto-ops:
//   - E-15: Unified "Fires next" radar, active-automation counts, & Agent Capability Strip
//   - E-17: MCP Policy Pipeline Trace normalization & tone classification
//   - E-22: Insights Action-to-Automation sessionStorage handoff (Goal Loop & Scheduled Task)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { PersonalAgent, PersonalAgentAutonomy, PersonalAgentSchedule, ScheduledTask } from '../src/lib/api/types.ts';
import { buildFiresNext, countActiveAutomations } from '../src/modules/scheduled-tasks/firesNext.ts';
import { agentCapabilityChips } from '../src/modules/personal-agents/agentCapabilities.ts';
import { normalizePolicyTrace, traceOutcomeTone } from '../src/modules/mcp/policyTrace.ts';
import { AUTOMATION_DRAFT_KEY, buildAutomationDraft, stageAutomationDraft } from '../src/modules/insights/actionHandoff.ts';
import { takeStagedGoalDraft } from '../src/modules/loops/actionDraft.ts';
import { takeStagedTaskDraft } from '../src/modules/scheduled-tasks/actionDraft.ts';

test('E-15: buildFiresNext merges tasks & agent routines in chronological order and skips paused items', () => {
  const tasks = [
    { id: 't-later', name: 'Nightly audit', kind: 'agent_prompt', enabled: true, next_run_at: '2026-11-20T22:00:00Z' },
    { id: 't-paused', name: 'Paused job', kind: 'agent_prompt', enabled: false, next_run_at: '2026-11-20T01:00:00Z' },
    { id: 't-soon', name: 'Morning check', kind: 'workflow', enabled: true, next_run_at: '2026-11-20T08:00:00Z' },
  ] as unknown as ScheduledTask[];

  const agents = [
    { id: 'a1', name: 'Ops Bot', enabled: true },
    { id: 'a2', name: 'Paused Bot', enabled: false },
  ] as unknown as PersonalAgent[];

  const schedulesByAgent: Record<string, PersonalAgentSchedule[]> = {
    a1: [
      { id: 's1', agent_id: 'a1', directive: 'Triage PR queue\nDetails here', enabled: true, next_run_at: '2026-11-20T09:00:00Z' } as PersonalAgentSchedule,
      { id: 's2', agent_id: 'a1', directive: 'Disabled routine', enabled: false, next_run_at: '2026-11-20T07:00:00Z' } as PersonalAgentSchedule,
    ],
    a2: [
      { id: 's3', agent_id: 'a2', directive: 'Should not fire', enabled: true, next_run_at: '2026-11-20T06:00:00Z' } as PersonalAgentSchedule,
    ],
  };

  const upcoming = buildFiresNext(tasks, agents, schedulesByAgent, 6);
  assert.deepEqual(
    upcoming.map((u) => u.id),
    ['task:t-soon', 'routine:s1', 'task:t-later'],
  );
  assert.equal(upcoming[1].title, 'Triage PR queue');
  assert.equal(upcoming[1].subtitle, 'Ops Bot');

  const counts = countActiveAutomations(tasks, agents, schedulesByAgent);
  assert.deepEqual(counts, { tasks: 2, agents: 1, routines: 1, total: 3 });
});

test('E-15: agentCapabilityChips surfaces Read-only/Autonomous, Slack-as-me, Browser, and rules', () => {
  const autonomy: PersonalAgentAutonomy = {
    proactive: { enabled: false, runs_per_day: 2, max_minutes: 10 },
    goals: [],
    rules: [
      { id: 'r1', text: 'Never force-push', enforce: { kind: 'deny', terms: ['push'] } },
      { id: 'r2', text: 'Ask before Jira edits', enforce: { kind: 'approval', terms: ['jira'] } },
    ],
    primary: false,
    slack_as_me_teams: ['T123'],
  };
  const roSchedules = [
    { id: 's1', enabled: true, permission: 'read_only' } as PersonalAgentSchedule,
  ];
  const chips = agentCapabilityChips({ browser: true, enabled: true }, autonomy, roSchedules);
  assert.deepEqual(
    chips.map((c) => c.label),
    ['Read-only', 'Slack-as-me', 'Browser', '2 rules'],
  );
  assert.equal(chips.find((c) => c.id === 'slack-as-me')?.tone, 'warning');

  const autoChips = agentCapabilityChips(
    { browser: false, enabled: true },
    { ...autonomy, proactive: { enabled: true, runs_per_day: 4, max_minutes: 15 }, slack_as_me_teams: [] },
    [],
  );
  assert.equal(autoChips[0].label, 'Autonomous · 4/day');
  assert.equal(autoChips[0].tone, 'accent');
});

test('E-17: normalizePolicyTrace preserves server trace steps and falls back cleanly when omitted', () => {
  const fromServer = normalizePolicyTrace({
    server: 'github',
    tool: 'merge_pr',
    risk_label: 'dangerous',
    injection_risk: 'low',
    policy_decision: 'require_approval',
    reason: 'Prod PR rule',
    trace: [
      { stage: 'allowlist', outcome: 'pass', detail: 'Allowed by workspace allowlist' },
      { stage: 'policy_rules', outcome: 'require_approval', detail: 'Matched rule Prod PRs', policy_id: 'p1' },
      { stage: 'final', outcome: 'require_approval', detail: 'Requires human approval' },
    ],
  });
  assert.equal(fromServer.length, 3);
  assert.equal(fromServer[0].stage, 'Allowlist');
  assert.equal(fromServer[1].stage, 'Policy rules');
  assert.equal(traceOutcomeTone(fromServer[0].outcome), 'ok');
  assert.equal(traceOutcomeTone(fromServer[1].outcome), 'warn');
  assert.equal(traceOutcomeTone('deny'), 'bad');

  const fallback = normalizePolicyTrace({
    server: 'github',
    tool: 'list_prs',
    risk_label: 'read',
    injection_risk: 'none',
    policy_decision: 'allow',
    reason: null,
  });
  assert.equal(fallback.length, 3);
  assert.equal(fallback[2].stage, 'Final');
  assert.equal(fallback[2].outcome, 'allowed');
});

test('E-22: Insights Action-to-Automation stages and consumes drafts cleanly per target', () => {
  const store = new Map<string, string>();
  (globalThis as unknown as { sessionStorage: Storage }).sessionStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, v); },
    removeItem: (k: string) => { store.delete(k); },
    clear: () => { store.clear(); },
    key: () => null,
    get length() { return store.size; },
  };

  const loopPayload = buildAutomationDraft(
    'goal_loop',
    { title: 'Cut flaky E2E retries', metric: 'Retry rate', current: '18%', target: '<5%', effort: 'M', ids: ['A-12'] },
    'Mon 19 Nov 2026',
  );
  stageAutomationDraft(loopPayload);
  assert.ok(store.has(AUTOMATION_DRAFT_KEY));
  // Wrong target consumer must leave the draft intact.
  assert.equal(takeStagedTaskDraft(), null);
  const takenLoop = takeStagedGoalDraft();
  assert.ok(takenLoop);
  assert.equal(takenLoop?.title, 'Cut flaky E2E retries');
  assert.match(takenLoop?.prompt ?? '', /Retry rate \(18% → <5%\)/);
  assert.match(takenLoop?.prompt ?? '', /A-12/);
  // Consumed once.
  assert.equal(takeStagedGoalDraft(), null);

  const taskPayload = buildAutomationDraft(
    'scheduled_task',
    { title: 'Weekly dependency audit', metric: '', current: '', target: '', effort: 'S', ids: [] },
  );
  stageAutomationDraft(taskPayload);
  assert.equal(takeStagedGoalDraft(), null);
  const takenTask = takeStagedTaskDraft();
  assert.equal(takenTask?.title, 'Weekly dependency audit');
  assert.equal(takeStagedTaskDraft(), null);
});
