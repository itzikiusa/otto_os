export interface DiagramNode {
  id: string;
  title: string;
  note?: string;
  tone?: 'running' | 'passed' | 'blocked';
}
export interface DiagramEdge { from: string; to: string; feedback?: boolean }
export interface PlacedNode extends DiagramNode { x: number; y: number; width: number; height: number }

/** Small deterministic layout for the bounded (16 task) graphs. Cyclic drafts
 * still render every node so the user can select and repair their dependencies. */
export function diagramLayout(nodes: DiagramNode[], edges: DiagramEdge[], width: number, loop = false) {
  const inset = 24, gap = 28, rowGap = 42, height = 76;
  const usable = Math.max(100, width - inset * 2);
  const columns = loop ? 2 : Math.max(1, Math.min(3, Math.floor((usable + gap) / 208)));
  const nodeWidth = Math.min(240, (usable - (columns - 1) * gap) / columns);
  const placed: PlacedNode[] = [];
  let y = inset;
  if (loop) {
    for (let i = 0; i < nodes.length; i++) {
      const row = Math.floor(i / 2), column = row % 2 ? 1 - i % 2 : i % 2;
      const x = (width - (nodeWidth * 2 + gap)) / 2 + column * (nodeWidth + gap);
      placed.push({ ...nodes[i], x, y: inset + row * (height + rowGap), width: nodeWidth, height });
    }
    y = inset + Math.ceil(nodes.length / 2) * (height + rowGap);
  } else {
    const remaining = new Map(nodes.map(node => [node.id, node]));
    const known = new Set(remaining.keys()), done = new Set<string>();
    while (remaining.size) {
      let ready = [...remaining.values()].filter(node => edges.filter(edge => edge.to === node.id && !edge.feedback && known.has(edge.from)).every(edge => done.has(edge.from)));
      if (!ready.length) ready = [...remaining.values()];
      for (let start = 0; start < ready.length; start += columns) {
        const row = ready.slice(start, start + columns);
        const x = (width - (row.length * nodeWidth + (row.length - 1) * gap)) / 2;
        for (let i = 0; i < row.length; i++) placed.push({ ...row[i], x: x + i * (nodeWidth + gap), y, width: nodeWidth, height });
        y += height + rowGap;
      }
      for (const node of ready) { remaining.delete(node.id); done.add(node.id); }
    }
  }
  const byId = new Map(placed.map(node => [node.id, node]));
  const paths = edges.flatMap((edge, index) => {
    const from = byId.get(edge.from), to = byId.get(edge.to);
    if (!from || !to) return [];
    const sx = from.x + from.width / 2, sy = from.y + from.height;
    const tx = to.x + to.width / 2, ty = to.y;
    let path: string;
    if (edge.feedback) {
      path = `M ${from.x} ${from.y + height / 2} H 10 V ${to.y + height / 2} H ${to.x}`;
    } else if (from.y === to.y) {
      const right = from.x < to.x;
      path = `M ${right ? from.x + from.width : from.x} ${from.y + height / 2} H ${right ? to.x : to.x + to.width}`;
    } else if (!loop && (ty - sy > rowGap + height || ty < sy)) {
      // A skip/back edge travels in the gutter instead of through another task.
      const lane = index % 2 ? width - 10 : 10;
      path = `M ${sx} ${sy} V ${sy + 12} H ${lane} V ${ty - 12} H ${tx} V ${ty}`;
    } else {
      path = `M ${sx} ${sy} C ${sx} ${sy + rowGap / 2}, ${tx} ${ty - rowGap / 2}, ${tx} ${ty}`;
    }
    return [{ ...edge, path }];
  });
  return { nodes: placed, edges: paths, height: Math.max(100, y - rowGap + inset) };
}
