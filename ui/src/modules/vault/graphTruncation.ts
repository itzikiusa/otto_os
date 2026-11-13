// The graph status line when the server left nodes or links out: a local
// graph keeps the nearest 300 notes / 1,500 links, the full graph keeps an
// edge budget. Say how much is shown instead of silently drawing a part.
import type { VaultGraphPayload } from '../../lib/api/types';

export interface TruncationNote {
  label: string;
  title: string;
}

type Counted = Pick<VaultGraphPayload, 'truncated' | 'total_nodes' | 'total_edges' | 'paths' | 'edges'>;

export function truncationNote(p: Counted): TruncationNote | null {
  if (!p.truncated) return null;
  const n = (x: number) => x.toLocaleString();
  if (p.total_nodes != null) {
    return {
      label: `Showing ${n(p.paths.length)} of ${n(p.total_nodes)}+ notes`,
      title: 'This neighbourhood is too large to draw legibly. The nearest notes are shown; open a note further out to explore from there.',
    };
  }
  if (p.total_edges != null) {
    return {
      label: `Showing ${n(p.edges.length / 2)} of ${n(p.total_edges)} links`,
      title: 'Link budget reached. The best-connected links are shown.',
    };
  }
  return { label: 'Truncated', title: 'Some links were left out by the server.' };
}
