// Canvas Studio domain types — the portable Scene schema + API DTOs.
//
// A scene is ONE JSON document (`Scene`) persisted as `doc_json` on the server
// (opaque to Rust). The UI owns the rich schema and rendering. Keep this the
// single source of truth: every canvas component consumes these types.

// ---------------------------------------------------------------------------
// Scene document
// ---------------------------------------------------------------------------

export type NodeKind =
  | 'shape'
  | 'text'
  | 'sticky'
  | 'freehand'
  | 'code'
  | 'json'
  | 'mermaid'
  | 'image'
  | 'group'
  | 'frame';

export type ShapeVariant =
  | 'rect'
  | 'roundrect'
  | 'ellipse'
  | 'diamond'
  | 'triangle'
  | 'cylinder'
  | 'parallelogram';

export interface ShapePayload {
  variant: ShapeVariant;
  fill?: string;
  stroke?: string;
  /** Hand-drawn (roughjs) look — stretch; renderers may ignore. */
  sketch?: boolean;
}
export interface TextPayload {
  value: string;
  align?: 'left' | 'center' | 'right';
  size?: number;
}
export interface StickyPayload {
  value: string;
  color?: string;
}
export interface FreehandPayload {
  /** perfect-freehand input points: [x, y, pressure?]. */
  points: [number, number, number?][];
  color?: string;
  size?: number;
}
export interface CodePayload {
  value: string;
  lang?: string;
}
export interface JsonPayload {
  /** Raw JSON text, rendered as a collapsible tree (+ a Raw toggle). */
  value: string;
}
export interface MermaidPayload {
  src: string;
  /** Diagram kind hint: 'sequence' | 'flowchart' | 'class' | 'state' | 'er' … */
  kind?: string;
}
export interface ImagePayload {
  attachmentId?: string;
  dataUrl?: string;
}

/** A single canvas node. `x/y/w/h` are scene-space (pre-zoom). */
export interface CanvasNode {
  id: string;
  kind: NodeKind;
  x: number;
  y: number;
  w: number;
  h: number;
  z?: number;
  rotation?: number;
  label?: string;
  /** Parent frame/group id (for nesting / slide membership). */
  parent?: string;
  // Discriminated payloads (exactly one matches `kind`):
  shape?: ShapePayload;
  text?: TextPayload;
  sticky?: StickyPayload;
  freehand?: FreehandPayload;
  code?: CodePayload;
  json?: JsonPayload;
  mermaid?: MermaidPayload;
  image?: ImagePayload;
  style?: Record<string, string | number>;
}

export interface CanvasEdge {
  id: string;
  source: string;
  target: string;
  sourceAnchor?: string;
  targetAnchor?: string;
  kind?: 'arrow' | 'line' | 'dashed';
  label?: string;
  style?: Record<string, string | number>;
}

/** One progressive-disclosure step within a presentation slide. */
export interface RevealStep {
  /** Node ids revealed at this step (fade/translate in). */
  nodeIds?: string[];
  /** For sequence playback: reveal mermaid messages in this inclusive range. */
  mermaidMessageRange?: [number, number];
}

/** A presentation slide (PowerPoint-style). */
export interface Slide {
  id: string;
  title?: string;
  /** Optional bounding frame node that defines the slide viewport. */
  frameNodeId?: string;
  /** When set, this slide steps through a mermaid sequence node's messages. */
  mermaidNodeId?: string;
  reveal: RevealStep[];
  notes?: string;
}

export interface AppState {
  background?: string;
  grid?: boolean;
}

export interface Scene {
  schema: 1;
  title: string;
  nodes: CanvasNode[];
  edges: CanvasEdge[];
  slides: Slide[];
  appState?: AppState;
}

// ---------------------------------------------------------------------------
// API DTOs (mirror crates/otto-canvas + crates/otto-server/canvas_assist)
// ---------------------------------------------------------------------------

export interface CanvasScene {
  id: string;
  workspace_id: string;
  story_id: string | null;
  title: string;
  /** The Scene JSON as a string (parse with `JSON.parse`). */
  doc_json: string;
  thumbnail: string | null;
  /** The managed Otto session backing this scene's Ask-AI (open it in Agents). */
  session_id: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface CanvasSceneSummary {
  id: string;
  workspace_id: string;
  story_id: string | null;
  title: string;
  thumbnail: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateSceneReq {
  title: string;
  doc?: Scene;
  story_id?: string | null;
}

export interface UpdateSceneReq {
  title?: string;
  doc?: Scene;
  thumbnail?: string;
}

export type AssistMode = 'auto' | 'sequence' | 'flow' | 'uml' | 'nodes';

export interface AssistReq {
  prompt: string;
  mode?: AssistMode;
}

export interface AssistResult {
  /** Excalidraw element SKELETON the agent authored directly (preferred — true
   *  code blocks, icons, frames). Either an array or `{ elements: [...] }`. */
  excalidraw?: unknown;
  /** A mermaid diagram source (fallback — clean auto-layout flowcharts). */
  mermaid: string | null;
  /** Freeform nodes when the agent produced tier-2 JSON instead of mermaid. */
  nodes: Partial<CanvasNode>[];
  edges: Partial<CanvasEdge>[];
  note: string;
}
