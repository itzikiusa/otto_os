// scene3d — the agent-editable 3D document format (docs/design/product-design-arena.md §2.3).
// Version 1. Small, declarative, human-readable: the browser renders it, the agent
// edits it, the inspector round-trips it. Rotation is DEGREES in the doc (agents and
// humans think in degrees); the viewer converts to radians for three.

export const SCENE3D_MIME = 'application/vnd.otto.scene3d+json';
export const SCENE3D_TYPE = 'otto-scene3d';
export const SCENE3D_VERSION = 1;
/** Hard cap mirrored by the Rust validator (`design_scene3d.rs::validate`). */
export const SCENE3D_MAX_OBJECTS = 2000;

export type Vec3 = [number, number, number];

export const OBJECT_TYPES = ['box', 'sphere', 'cylinder', 'cone', 'torus', 'plane', 'text', 'gltf', 'group'] as const;
export type ObjectType = (typeof OBJECT_TYPES)[number];
/** Primitives the Add menu offers (everything that is not a gltf reference or a group). */
export const PRIMITIVE_TYPES = ['box', 'sphere', 'cylinder', 'cone', 'torus', 'plane', 'text'] as const;
export type PrimitiveType = (typeof PRIMITIVE_TYPES)[number];

export const LIGHT_TYPES = ['directional', 'ambient', 'point', 'spot', 'hemisphere'] as const;
export type LightType = (typeof LIGHT_TYPES)[number];

export interface Scene3dMaterial {
  color?: string;      // '#rrggbb'
  metalness?: number;  // 0..1
  roughness?: number;  // 0..1
  opacity?: number;    // 0..1 (< 1 ⇒ transparent)
  emissive?: string;   // '#rrggbb'
  wireframe?: boolean;
}

export interface Scene3dObject {
  id: string;
  name: string;
  type: ObjectType;
  position: Vec3;
  rotation: Vec3;      // degrees
  scale: Vec3;
  material?: Scene3dMaterial;
  /** `type:'gltf'` only — an attachment id (never a URL); resolved by the host via `resolveAttachment`. */
  attachment_id?: string;
  /** `type:'text'` only — the string to extrude/draw. */
  text?: string;
  visible?: boolean;   // default true
  /** Free-form designer/agent notes shown in the inspector (3D pin annotations are deferred). */
  notes?: string;
}

export interface Scene3dLight {
  id: string;
  name?: string;
  type: LightType;
  position?: Vec3;
  target?: Vec3;       // directional / spot
  intensity?: number;
  color?: string;
  ground_color?: string; // hemisphere only
  distance?: number;   // point / spot
  angle?: number;      // spot, degrees
  shadow?: boolean;
  visible?: boolean;
  notes?: string;
}

export interface Scene3dCamera {
  position: Vec3;
  target: Vec3;
  fov: number;         // degrees
  near?: number;
  far?: number;
}

export interface Scene3dGroup {
  id: string;
  name: string;
  children: string[];  // object ids (and nested group ids)
  visible?: boolean;
  notes?: string;
}

export interface Scene3dDoc {
  type: typeof SCENE3D_TYPE;
  version: typeof SCENE3D_VERSION;
  background?: string; // '#rrggbb'
  grid?: boolean;      // default true
  camera: Scene3dCamera;
  lights: Scene3dLight[];
  objects: Scene3dObject[];
  groups: Scene3dGroup[];
}

/** What the hierarchy/inspector consider "selectable": an object, a light or a group. */
export type Scene3dNodeKind = 'object' | 'light' | 'group';

/** Gizmo modes (W / E / R). */
export type GizmoMode = 'translate' | 'rotate' | 'scale';

/** A starter scene: sun + ambient, a floor plane and the default camera. */
export function emptyScene(): Scene3dDoc {
  return {
    type: SCENE3D_TYPE,
    version: SCENE3D_VERSION,
    background: '#0f172a',
    grid: true,
    camera: { position: [6, 5, 8], target: [0, 1, 0], fov: 50 },
    lights: [
      { id: 'sun', name: 'Sun', type: 'directional', position: [5, 10, 5], intensity: 1.2, color: '#ffffff', shadow: true },
      { id: 'amb', name: 'Ambient', type: 'ambient', intensity: 0.4 },
    ],
    objects: [
      {
        id: 'floor', name: 'Floor', type: 'plane',
        position: [0, 0, 0], rotation: [-90, 0, 0], scale: [20, 20, 1],
        material: { color: '#334155', roughness: 0.9 },
      },
    ],
    groups: [],
  };
}
