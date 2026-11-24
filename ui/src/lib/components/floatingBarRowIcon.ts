// The ⌘K bar's row glyphs (FloatingBar.svelte): one per hit kind (shared
// with the palette sheet, Palette.svelte), the module's own icon for a Go-to, a thread row's
// threads glyph, and a group default.
import type { Command } from '../commands.svelte';
import type { SearchHit } from '../api/types';
import type { BarRow } from '../floatingBar';
import { SIDEBAR_MODULES } from '../sidebar';
import type { IconName } from './Icon.svelte';

/** A thread of the bar's thread group (lib/paletteThreads.svelte.ts), not "New thread: …". */
export function isThreadRow(row: BarRow<Command, SearchHit> | undefined): boolean {
  return row?.kind === 'cmd' && row.cmd.id.startsWith('thread:') && row.cmd.id !== 'thread:new';
}

/** One glyph per search-hit kind, for the bar and the palette sheet alike. */
export function hitIcon(kind: string): IconName {
  switch (kind) {
    case 'repo': return 'branch';
    case 'workflow': return 'merge';
    case 'story': return 'ticket';
    case 'api_request': return 'zap';
    case 'broker_cluster': return 'box';
    case 'memory': return 'db';
    default: return 'file';
  }
}

export function rowIcon(row: BarRow<Command, SearchHit>): IconName {
  if (row.kind === 'ask') return 'sparkle';
  if (row.kind === 'hit') return hitIcon(row.hit.kind);
  if (row.cmd.id.startsWith('thread:')) return row.cmd.id === 'thread:new' ? 'edit' : 'threads';
  const g = row.cmd.group ?? '';
  if (row.cmd.id.startsWith('core.go-')) {
    const mod = SIDEBAR_MODULES.find((m) => `core.go-${m.id}` === row.cmd.id);
    return mod?.icon ?? 'chevronRight';
  }
  if (g === 'Navigate') return 'chevronRight';
  if (g === 'Sessions' || g === 'Session') return 'terminal';
  if (g === 'Git' || row.cmd.id.startsWith('repo.')) return 'branch';
  if (g === 'Workspaces') return 'folder';
  if (g === 'Connections') return 'plug';
  if (g === 'Appearance' || g === 'View') return 'layout';
  return 'command';
}
