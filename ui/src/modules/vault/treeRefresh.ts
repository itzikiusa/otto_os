/** Refresh only visible branches. Results are new nodes; stale work never mutates
 * the tree currently on screen. At most four directory requests run together. */
export interface RefreshNode<E> {
  entry: E; depth: number; open: boolean; loaded: boolean; loading: boolean;
  children: RefreshNode<E>[];
}

export async function refreshVisibleTree<E extends {path: string; kind: string}>(
  previous: RefreshNode<E>[],
  load: (path: string) => Promise<E[]>,
  current: () => boolean = () => true,
  path = '', depth = 0,
): Promise<RefreshNode<E>[]> {
  let result: RefreshNode<E>[] = [];
  type Job = { previous: RefreshNode<E>[]; path: string; depth: number; assign: (nodes: RefreshNode<E>[]) => void };
  let level: Job[] = [{previous, path, depth, assign: nodes => { result = nodes; }}];
  while (level.length && current()) {
    const next: Job[] = [];
    for (let i = 0; i < level.length && current(); i += 4) {
      await Promise.all(level.slice(i, i + 4).map(async job => {
        const entries = await load(job.path);
        if (!current()) return;
        const old = new Map(job.previous.map(node => [node.entry.path, node]));
        const nodes: RefreshNode<E>[] = entries.map(entry => {
          const prior = old.get(entry.path);
          return prior ? {...prior, entry, depth: job.depth, loading: false}
            : {entry, depth: job.depth, open: false, loaded: false, loading: false, children: []};
        });
        job.assign(nodes);
        for (const node of nodes) {
          if (node.entry.kind !== 'dir') continue;
          if (node.open) {
            next.push({previous: node.children, path: node.entry.path, depth: node.depth + 1,
              assign: children => {node.children = children; node.loaded = true;}});
          } else node.loaded = false;
        }
      }));
    }
    level = next;
  }
  return result;
}
