// The pure half of ⋯ → Export (unit-tested without a DOM): file extensions
// and whether a 3D scene has a Blender script route (it came from a story).
import type { DesignArtifact } from '../../lib/api/types';

/** The source file's extension for a format. */
export function sourceExt(format: string): string {
  return ({ html: 'html', svg: 'svg', mermaid: 'mmd', d2: 'd2', excalidraw: 'excalidraw', scene3d: 'scene3d.json' } as Record<string, string>)[format] ?? 'json';
}

/** The Product story + attachment a scene came from, when it has a Blender script route. */
export function blenderSourceOf(a: Pick<DesignArtifact, 'format' | 'source_kind' | 'source_id' | 'meta'>): { storyId: string; attachmentId: string } | null {
  if (a.format !== 'scene3d' || a.source_kind !== 'product_attachment' || !a.source_id) return null;
  const from = (a.meta?.imported_from ?? {}) as Record<string, unknown>;
  return typeof from.story_id === 'string' && from.story_id ? { storyId: from.story_id, attachmentId: a.source_id } : null;
}
