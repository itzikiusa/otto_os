import type { AcceptanceCriterion, GoalGraphNode, GoalLoopGraph } from '../../lib/api/types';

/** Presentation layers follow dependency edges; execution itself stays serial. */
export function graphLayers(graph: GoalLoopGraph): GoalGraphNode[][] {
  const remaining = new Map(graph.nodes.map(node => [node.id, node]));
  const done = new Set<string>();
  const layers: GoalGraphNode[][] = [];
  while (remaining.size) {
    const ready = [...remaining.values()].filter(node => node.depends_on.every(id => done.has(id)));
    if (!ready.length) break;
    layers.push(ready);
    for (const node of ready) { remaining.delete(node.id); done.add(node.id); }
  }
  return layers;
}

/** Mirror the server's editing constraints for immediate, actionable feedback. */
export function graphError(graph: GoalLoopGraph | null | undefined, criteria: AcceptanceCriterion[]): string | null {
  if (!graph) return null;
  if (graph.nodes.length < 1 || graph.nodes.length > 16) return 'Use between 1 and 16 graph nodes.';
  const ids = new Set(graph.nodes.map(node => node.id));
  const criterionIds = new Set(criteria.map(criterion => criterion.id));
  if (ids.size !== graph.nodes.length) return 'Each graph node needs a unique ID.';
  for (const node of graph.nodes) {
    const label = node.title.trim() || 'Untitled node';
    if (!node.id.trim() || !node.title.trim() || !node.instructions.trim()) return `${label}: add a title and instructions.`;
    if (!node.criterion_ids.length) return `${label}: associate at least one acceptance criterion.`;
    if (node.criterion_ids.some(id => !criterionIds.has(id))) return `${label}: a linked criterion was removed. Choose an existing criterion.`;
    if (new Set(node.criterion_ids).size !== node.criterion_ids.length) return `${label}: each criterion can be linked only once.`;
    if (node.depends_on.some(id => !ids.has(id))) return `${label}: a dependency no longer exists. Choose an existing node.`;
    if (new Set(node.depends_on).size !== node.depends_on.length) return `${label}: each dependency can be linked only once.`;
    if (node.depends_on.includes(node.id)) return `${label}: a node cannot depend on itself.`;
    if (criteria.some(criterion => criterion.verify_kind === 'human' && node.criterion_ids.includes(criterion.id)) && graph.nodes.some(other => other.depends_on.includes(node.id))) return `${label}: move human acceptance to a final task. Later work invalidates human approvals.`;
  }
  const assigned = new Set(graph.nodes.flatMap(node => node.criterion_ids));
  const missing = criteria.find(criterion => !assigned.has(criterion.id));
  if (missing) return `Assign “${missing.text || missing.id}” to a graph node.`;
  if (graphLayers(graph).flat().length !== graph.nodes.length) return 'Dependencies form a cycle. Remove a dependency so every node can become ready.';
  return null;
}

export function removeGraphNode(graph: GoalLoopGraph, id: string): GoalLoopGraph {
  return { nodes: graph.nodes.filter(node => node.id !== id).map(node => ({
    ...node, depends_on: node.depends_on.filter(dependency => dependency !== id),
  })) };
}

export function newGraphNode(graph: GoalLoopGraph, criteria: AcceptanceCriterion[]): GoalGraphNode {
  let n = graph.nodes.length + 1;
  while (graph.nodes.some(node => node.id === `task-${n}`)) n++;
  return {
    id: `task-${n}`, title: `Task ${n}`, instructions: '', depends_on: [],
    criterion_ids: criteria.length ? [criteria[0].id] : [],
  };
}
