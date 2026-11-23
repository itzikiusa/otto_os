// Pure helper for MCP Policy Pipeline Trace (E-17).
// Renders the step-by-step governance evaluation trace returned by
// POST /mcp/policies/evaluate (`trace: PolicyTraceStep[]`), falling back cleanly
// to a structured summary if an older daemon omits `trace`.
import type { McpEvaluatePreview } from '../../lib/api/types';

export interface PolicyTraceStep {
  stage: string;
  outcome: string;
  detail: string;
  policy_id?: string | null;
}

export type McpEvaluateWithTrace = McpEvaluatePreview & {
  trace?: PolicyTraceStep[];
};

const STAGE_LABEL: Record<string, string> = {
  allowlist: 'Allowlist',
  policy_rules: 'Policy rules',
  policy: 'Policy rules',
  tool_risk: 'Tool risk',
  risk: 'Tool risk',
  auto_approve: 'Auto-approve',
  final: 'Final',
};

export function traceStageLabel(stage: string): string {
  const key = stage.trim().toLowerCase().replace(/[\s-]+/g, '_');
  return STAGE_LABEL[key] ?? stage;
}

export function traceOutcomeTone(outcome: string): 'ok' | 'warn' | 'bad' | 'dim' {
  const v = outcome.trim().toLowerCase();
  if (v === 'allow' || v === 'allowed' || v === 'pass' || v === 'auto_approved') return 'ok';
  if (v === 'deny' || v === 'denied' || v === 'block' || v === 'blocked') return 'bad';
  if (v.includes('approval') || v.includes('dry_run') || v === 'warn' || v === 'flagged') return 'warn';
  return 'dim';
}

export function normalizePolicyTrace(res: McpEvaluateWithTrace): PolicyTraceStep[] {
  if (Array.isArray(res.trace) && res.trace.length > 0) {
    return res.trace.map((step) => ({
      stage: traceStageLabel(step.stage),
      outcome: step.outcome,
      detail: step.detail,
      policy_id: step.policy_id ?? null,
    }));
  }
  return [
    {
      stage: 'Tool risk',
      outcome: res.risk_label,
      detail: `Injection risk: ${res.injection_risk}`,
      policy_id: null,
    },
    {
      stage: 'Policy rules',
      outcome: res.policy_decision,
      detail: res.reason ?? 'No explicit deny or approval rule matched',
      policy_id: null,
    },
    {
      stage: 'Final',
      outcome: res.policy_decision === 'allow' ? 'allowed' : res.policy_decision,
      detail: res.reason ?? `Decision for ${res.server} → ${res.tool}`,
      policy_id: null,
    },
  ];
}
