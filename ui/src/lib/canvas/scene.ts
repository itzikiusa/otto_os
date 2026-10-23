// Pure Scene helpers — the empty scene and the tolerant `doc_json` parser the
// canvas store loads with. No Svelte state here. Lives in lib/ beside the
// types because the canvas store (lib/) uses it, and lib/ never imports
// from modules/ (architecture-1-13).

import type { CanvasEdge, CanvasNode, Scene } from './types';

export function emptyScene(title = 'Untitled scene'): Scene {
  return { schema: 1, title, nodes: [], edges: [], slides: [], appState: { grid: true } };
}

/** Parse a stored `doc_json`, tolerating bad data by returning an empty scene. */
export function parseScene(docJson: string, fallbackTitle = 'Untitled scene'): Scene {
  try {
    const s = JSON.parse(docJson) as Partial<Scene>;
    return {
      schema: 1,
      title: s.title ?? fallbackTitle,
      nodes: Array.isArray(s.nodes) ? (s.nodes as CanvasNode[]) : [],
      edges: Array.isArray(s.edges) ? (s.edges as CanvasEdge[]) : [],
      slides: Array.isArray(s.slides) ? s.slides : [],
      appState: s.appState ?? { grid: true },
    };
  } catch {
    return emptyScene(fallbackTitle);
  }
}
