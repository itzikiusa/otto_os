import type { WorkflowNode } from '../../lib/api/types';

export function effectiveRetry(node: WorkflowNode | null | undefined) {
  if (node?.retry) return node.retry;
  const params = node?.params as Record<string, unknown> | null;
  if (params?.retry && typeof params.retry === 'object') return params.retry as { max_attempts: number; backoff_ms: number; factor: number };
  const agent = node?.kind === 'agent_prompt' || (node?.kind === 'prepare_context' && typeof params?.prompt === 'string' && params.prompt.trim());
  return { max_attempts: agent ? 2 : 0, backoff_ms: agent ? 2000 : 0, factor: 2 };
}

export function updateRetry(node: WorkflowNode, field: 'max_attempts' | 'backoff_ms', value: number) {
  return { ...effectiveRetry(node), [field]: Number.isFinite(value) && value > 0 ? value : 0 };
}

export function clearRetry(node: WorkflowNode): void {
  node.retry = null;
  if (node.params && typeof node.params === 'object' && !Array.isArray(node.params)) {
    const { retry: _legacy, ...params } = node.params as Record<string, unknown>;
    node.params = params;
  }
}
