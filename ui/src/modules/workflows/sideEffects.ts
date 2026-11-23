// Pure side-effect preflight classifier for Workflow graphs. Identifies
// outward/mutating steps (product_publish, git_pr, http_request, prod db_query,
// channel_notify — including loop inner steps) for canvas badges and the
// RunInputPanel "What this run will touch" summary.
import type { Connection, WorkflowGraph, WorkflowNode } from '../../lib/api/types';

export interface NodeSideEffect {
  nodeId: string;
  nodeName: string;
  kind: string;
  /** Short quiet badge rendered on the node card (e.g. "Prod DB", "Opens PR"). */
  badge: string;
  /** Plain-language summary rendered in RunInputPanel preflight. */
  summary: string;
  /** True for mutating/irreversible/prod effects; false for informational outward calls. */
  danger: boolean;
}

type Params = Record<string, unknown>;

const MUTATING_HTTP = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

function strParam(params: Params | undefined, key: string): string {
  const v = params?.[key];
  return typeof v === 'string' ? v.trim() : '';
}

function urlHost(raw: string): string {
  if (!raw) return '';
  try {
    return new URL(raw).host || raw;
  } catch {
    return raw.slice(0, 48);
  }
}

function classifyStep(
  nodeId: string,
  nodeName: string,
  kind: string,
  params: Params | undefined,
  dbConnections?: readonly Connection[],
): NodeSideEffect | null {
  switch (kind) {
    case 'product_publish':
      return {
        nodeId,
        nodeName,
        kind,
        badge: 'Publishes',
        summary: 'Publishes story or artifacts to Jira / Confluence',
        danger: true,
      };
    case 'git_pr':
      return {
        nodeId,
        nodeName,
        kind,
        badge: 'Opens PR',
        summary: 'Pushes and opens or updates a pull request',
        danger: true,
      };
    case 'channel_notify': {
      const ch = strParam(params, 'channel');
      const target = strParam(params, 'chat_id') || strParam(params, 'target');
      const where = ch ? `${ch}${target ? ` (${target})` : ''}` : 'Slack / Telegram';
      return {
        nodeId,
        nodeName,
        kind,
        badge: 'Notifies',
        summary: `Posts a message to ${where}`,
        danger: false,
      };
    }
    case 'http_request': {
      const method = (strParam(params, 'method') || 'GET').toUpperCase();
      const host = urlHost(strParam(params, 'url'));
      const danger = MUTATING_HTTP.has(method);
      return {
        nodeId,
        nodeName,
        kind,
        badge: danger ? method : 'HTTP',
        summary: `${method} request to ${host || 'external HTTP endpoint'}`,
        danger,
      };
    }
    case 'db_query': {
      const connId = strParam(params, 'connection_id');
      if (!connId || !dbConnections?.length) return null;
      const conn = dbConnections.find((c) => c.id === connId);
      if (conn?.environment !== 'prod') return null;
      return {
        nodeId,
        nodeName,
        kind,
        badge: 'Prod DB',
        summary: `Queries production database “${conn.name}”`,
        danger: true,
      };
    }
    default:
      return null;
  }
}

/** Classify a single canvas node (including loop inner steps, preferring any
 *  high-severity effect for the card badge). */
export function nodeSideEffect(
  node: WorkflowNode,
  dbConnections?: readonly Connection[],
): NodeSideEffect | null {
  const name = node.name?.trim() || node.kind;
  const params = node.params as Params | undefined;
  if (node.kind === 'loop') {
    const steps = Array.isArray(params?.steps) ? (params.steps as { name?: string; kind?: string; params?: Params }[]) : [];
    const inner = steps
      .map((s, i) => classifyStep(node.id, s?.name?.trim() || `${name} step ${i + 1}`, s?.kind ?? '', s?.params, dbConnections))
      .filter((x): x is NodeSideEffect => x !== null);
    if (inner.length === 0) return null;
    return inner.find((x) => x.danger) ?? inner[0];
  }
  return classifyStep(node.id, name, node.kind, params, dbConnections);
}

/** Collect every outward/mutating step across a workflow graph (expanding loop
 *  inner steps), sorted with high-severity/prod effects first. */
export function graphSideEffects(
  graph: WorkflowGraph | null | undefined,
  dbConnections?: readonly Connection[],
): NodeSideEffect[] {
  if (!graph?.nodes?.length) return [];
  const out: NodeSideEffect[] = [];
  for (const n of graph.nodes) {
    const name = n.name?.trim() || n.kind;
    const params = n.params as Params | undefined;
    if (n.kind === 'loop') {
      const steps = Array.isArray(params?.steps) ? (params.steps as { name?: string; kind?: string; params?: Params }[]) : [];
      for (let i = 0; i < steps.length; i++) {
        const s = steps[i];
        const hit = classifyStep(n.id, `${name} → ${s?.name?.trim() || `step ${i + 1}`}`, s?.kind ?? '', s?.params, dbConnections);
        if (hit) out.push(hit);
      }
    } else {
      const hit = classifyStep(n.id, name, n.kind, params, dbConnections);
      if (hit) out.push(hit);
    }
  }
  return out.sort((a, b) => Number(b.danger) - Number(a.danger));
}

/** Whether a graph contains any `db_query` node (top-level or inside a loop). */
export function graphHasDbQuery(graph: WorkflowGraph | null | undefined): boolean {
  if (!graph?.nodes?.length) return false;
  return graph.nodes.some((n) => {
    if (n.kind === 'db_query') return true;
    if (n.kind !== 'loop') return false;
    const steps = (n.params as { steps?: { kind?: string }[] } | undefined)?.steps;
    return Array.isArray(steps) && steps.some((s) => s?.kind === 'db_query');
  });
}
