// scene3d validator — the TS mirror of `design_scene3d.rs::validate` (Track A).
// Runs before every render and before every save. It is deliberately strict and
// boring: known `type`s only, finite numbers, bounded arrays (≤ 2 000 objects),
// `attachment_id` must be a safe id component (it becomes a URL path segment on
// the authed fetch), colours are `#rrggbb`. Unknown keys are dropped on
// `normalize` so an agent typo never reaches the renderer.
import {
  LIGHT_TYPES,
  OBJECT_TYPES,
  SCENE3D_MAX_OBJECTS,
  SCENE3D_TYPE,
  SCENE3D_VERSION,
  type LightType,
  type ObjectType,
  type Scene3dCamera,
  type Scene3dDoc,
  type Scene3dGroup,
  type Scene3dLight,
  type Scene3dMaterial,
  type Scene3dObject,
  type Vec3,
} from './types';

export interface ValidationIssue {
  /** JSON-pointer-ish path (`objects[3].scale`). */
  path: string;
  message: string;
}

export type ValidationResult =
  | { ok: true; doc: Scene3dDoc; issues: ValidationIssue[] }
  | { ok: false; doc: null; issues: ValidationIssue[] };

/** Same character class the daemon accepts for an id path component. */
const SAFE_ID = /^[A-Za-z0-9_-]{1,128}$/;
const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;
const MAX_LIGHTS = 64;
const MAX_GROUPS = 500;
const MAX_NAME = 200;
const MAX_NOTES = 4000;
const MAX_TEXT = 500;
/** |coordinate| beyond this is almost certainly a typo (metres). */
const MAX_COORD = 1e5;

export function isSafeId(s: unknown): s is string {
  return typeof s === 'string' && SAFE_ID.test(s);
}
export function isHexColor(s: unknown): s is string {
  return typeof s === 'string' && HEX_COLOR.test(s);
}
function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}
function finite(v: unknown): v is number {
  return typeof v === 'number' && Number.isFinite(v);
}

class Ctx {
  issues: ValidationIssue[] = [];
  err(path: string, message: string): void {
    this.issues.push({ path, message });
  }
  /** Non-fatal: recorded, but the field is dropped/defaulted and the doc stays valid. */
  warn(path: string, message: string): void {
    this.issues.push({ path, message: `(dropped) ${message}` });
  }
}

/** Missing (`undefined`) → `fallback`; present but malformed (wrong shape, NaN/∞/null
 *  component, out of range) → FATAL. An agent that writes a bad number gets a rejection,
 *  never a silently relocated object. */
function vec3(v: unknown, path: string, ctx: Ctx, fallback: Vec3 | null): Vec3 | null {
  if (v === undefined && fallback) return fallback;
  if (!Array.isArray(v) || v.length !== 3 || !v.every(finite)) {
    ctx.err(path, 'expected [x, y, z] finite numbers');
    return null;
  }
  if (v.some((n) => Math.abs(n) > MAX_COORD)) {
    ctx.err(path, `component out of range (|n| ≤ ${MAX_COORD})`);
    return null;
  }
  return [v[0], v[1], v[2]];
}

function unit(v: unknown, path: string, ctx: Ctx): number | undefined {
  if (v === undefined) return undefined;
  if (!finite(v) || v < 0 || v > 1) {
    ctx.warn(path, 'expected a number in 0..1');
    return undefined;
  }
  return v;
}
function nonNeg(v: unknown, path: string, ctx: Ctx, max = 1e6): number | undefined {
  if (v === undefined) return undefined;
  if (!finite(v) || v < 0 || v > max) {
    ctx.warn(path, `expected a number in 0..${max}`);
    return undefined;
  }
  return v;
}
function color(v: unknown, path: string, ctx: Ctx): string | undefined {
  if (v === undefined) return undefined;
  if (!isHexColor(v)) {
    ctx.warn(path, 'expected "#rrggbb"');
    return undefined;
  }
  return v.toLowerCase();
}
function str(v: unknown, path: string, ctx: Ctx, max: number): string | undefined {
  if (v === undefined) return undefined;
  if (typeof v !== 'string') {
    ctx.warn(path, 'expected a string');
    return undefined;
  }
  return v.length > max ? v.slice(0, max) : v;
}
function bool(v: unknown, path: string, ctx: Ctx): boolean | undefined {
  if (v === undefined) return undefined;
  if (typeof v !== 'boolean') {
    ctx.warn(path, 'expected true/false');
    return undefined;
  }
  return v;
}

function material(v: unknown, path: string, ctx: Ctx): Scene3dMaterial | undefined {
  if (v === undefined) return undefined;
  if (!isRecord(v)) {
    ctx.warn(path, 'expected an object');
    return undefined;
  }
  const m: Scene3dMaterial = {};
  const c = color(v.color, `${path}.color`, ctx);
  if (c) m.color = c;
  const e = color(v.emissive, `${path}.emissive`, ctx);
  if (e) m.emissive = e;
  const met = unit(v.metalness, `${path}.metalness`, ctx);
  if (met !== undefined) m.metalness = met;
  const rough = unit(v.roughness, `${path}.roughness`, ctx);
  if (rough !== undefined) m.roughness = rough;
  const op = unit(v.opacity, `${path}.opacity`, ctx);
  if (op !== undefined) m.opacity = op;
  const wf = bool(v.wireframe, `${path}.wireframe`, ctx);
  if (wf !== undefined) m.wireframe = wf;
  return m;
}

function object(v: unknown, path: string, ctx: Ctx, seen: Set<string>): Scene3dObject | null {
  if (!isRecord(v)) {
    ctx.err(path, 'expected an object');
    return null;
  }
  if (!isSafeId(v.id)) {
    ctx.err(`${path}.id`, 'id must match [A-Za-z0-9_-]{1,128}');
    return null;
  }
  if (seen.has(v.id)) {
    ctx.err(`${path}.id`, `duplicate id "${v.id}"`);
    return null;
  }
  if (!OBJECT_TYPES.includes(v.type as ObjectType)) {
    ctx.err(`${path}.type`, `unknown type "${String(v.type)}" (${OBJECT_TYPES.join(' | ')})`);
    return null;
  }
  const type = v.type as ObjectType;
  const position = vec3(v.position, `${path}.position`, ctx, [0, 0, 0]);
  const rotation = vec3(v.rotation, `${path}.rotation`, ctx, [0, 0, 0]);
  const scale = vec3(v.scale, `${path}.scale`, ctx, [1, 1, 1]);
  if (!position || !rotation || !scale) return null;
  const out: Scene3dObject = {
    id: v.id,
    name: str(v.name, `${path}.name`, ctx, MAX_NAME) || v.id,
    type,
    position,
    rotation,
    scale,
  };
  const mat = material(v.material, `${path}.material`, ctx);
  if (mat) out.material = mat;
  if (type === 'gltf') {
    if (!isSafeId(v.attachment_id)) {
      ctx.err(`${path}.attachment_id`, 'gltf objects need a safe attachment_id (never a URL)');
      return null;
    }
    out.attachment_id = v.attachment_id;
  } else if (v.attachment_id !== undefined) {
    ctx.warn(`${path}.attachment_id`, 'only gltf objects carry attachment_id');
  }
  if (type === 'text') {
    out.text = str(v.text, `${path}.text`, ctx, MAX_TEXT) ?? out.name;
  }
  const vis = bool(v.visible, `${path}.visible`, ctx);
  if (vis !== undefined) out.visible = vis;
  const notes = str(v.notes, `${path}.notes`, ctx, MAX_NOTES);
  if (notes) out.notes = notes;
  seen.add(out.id);
  return out;
}

function light(v: unknown, path: string, ctx: Ctx, seen: Set<string>): Scene3dLight | null {
  if (!isRecord(v)) {
    ctx.err(path, 'expected an object');
    return null;
  }
  if (!isSafeId(v.id)) {
    ctx.err(`${path}.id`, 'id must match [A-Za-z0-9_-]{1,128}');
    return null;
  }
  if (seen.has(v.id)) {
    ctx.err(`${path}.id`, `duplicate id "${v.id}"`);
    return null;
  }
  if (!LIGHT_TYPES.includes(v.type as LightType)) {
    ctx.err(`${path}.type`, `unknown light type "${String(v.type)}" (${LIGHT_TYPES.join(' | ')})`);
    return null;
  }
  const out: Scene3dLight = { id: v.id, type: v.type as LightType };
  const name = str(v.name, `${path}.name`, ctx, MAX_NAME);
  if (name) out.name = name;
  if (out.type !== 'ambient' && out.type !== 'hemisphere') {
    const p = vec3(v.position, `${path}.position`, ctx, [5, 10, 5]);
    if (!p) return null;
    out.position = p;
  }
  if (out.type === 'directional' || out.type === 'spot') {
    const t = vec3(v.target, `${path}.target`, ctx, [0, 0, 0]);
    if (!t) return null;
    out.target = t;
  }
  const i = nonNeg(v.intensity, `${path}.intensity`, ctx, 1000);
  if (i !== undefined) out.intensity = i;
  const c = color(v.color, `${path}.color`, ctx);
  if (c) out.color = c;
  const g = color(v.ground_color, `${path}.ground_color`, ctx);
  if (g) out.ground_color = g;
  const d = nonNeg(v.distance, `${path}.distance`, ctx, MAX_COORD);
  if (d !== undefined) out.distance = d;
  const a = nonNeg(v.angle, `${path}.angle`, ctx, 90);
  if (a !== undefined) out.angle = a;
  const s = bool(v.shadow, `${path}.shadow`, ctx);
  if (s !== undefined) out.shadow = s;
  const vis = bool(v.visible, `${path}.visible`, ctx);
  if (vis !== undefined) out.visible = vis;
  const notes = str(v.notes, `${path}.notes`, ctx, MAX_NOTES);
  if (notes) out.notes = notes;
  seen.add(out.id);
  return out;
}

function camera(v: unknown, ctx: Ctx): Scene3dCamera | null {
  const fallback: Scene3dCamera = { position: [6, 5, 8], target: [0, 1, 0], fov: 50 };
  if (v === undefined) return fallback;
  if (!isRecord(v)) {
    ctx.err('camera', 'expected an object');
    return null;
  }
  const position = vec3(v.position, 'camera.position', ctx, fallback.position);
  const target = vec3(v.target, 'camera.target', ctx, fallback.target);
  if (!position || !target) return null;
  let fov = fallback.fov;
  if (v.fov !== undefined) {
    if (!finite(v.fov) || v.fov < 1 || v.fov > 179) ctx.warn('camera.fov', 'expected 1..179 degrees');
    else fov = v.fov;
  }
  const out: Scene3dCamera = { position, target, fov };
  const near = nonNeg(v.near, 'camera.near', ctx, MAX_COORD);
  if (near !== undefined && near > 0) out.near = near;
  const far = nonNeg(v.far, 'camera.far', ctx, 1e7);
  if (far !== undefined && far > (out.near ?? 0.1)) out.far = far;
  return out;
}

function group(v: unknown, path: string, ctx: Ctx, seen: Set<string>): Scene3dGroup | null {
  if (!isRecord(v)) {
    ctx.err(path, 'expected an object');
    return null;
  }
  if (!isSafeId(v.id)) {
    ctx.err(`${path}.id`, 'id must match [A-Za-z0-9_-]{1,128}');
    return null;
  }
  if (seen.has(v.id)) {
    ctx.err(`${path}.id`, `duplicate id "${v.id}"`);
    return null;
  }
  if (!Array.isArray(v.children)) {
    ctx.err(`${path}.children`, 'expected an array of ids');
    return null;
  }
  const children: string[] = [];
  for (const [i, c] of v.children.entries()) {
    if (isSafeId(c)) children.push(c);
    else ctx.warn(`${path}.children[${i}]`, 'not a safe id');
  }
  const out: Scene3dGroup = {
    id: v.id,
    name: str(v.name, `${path}.name`, ctx, MAX_NAME) || v.id,
    children,
  };
  const vis = bool(v.visible, `${path}.visible`, ctx);
  if (vis !== undefined) out.visible = vis;
  const notes = str(v.notes, `${path}.notes`, ctx, MAX_NOTES);
  if (notes) out.notes = notes;
  seen.add(out.id);
  return out;
}

/**
 * Validate an untrusted value (parsed JSON, an agent edit, an inspector patch) and
 * return a normalized document with unknown keys stripped. Fatal issues (`ok:false`)
 * mean the value must NOT be rendered or saved; non-fatal ones are prefixed
 * `(dropped)` and describe fields that were defaulted.
 */
export function validate(input: unknown): ValidationResult {
  const ctx = new Ctx();
  if (!isRecord(input)) {
    ctx.err('', 'document must be a JSON object');
    return { ok: false, doc: null, issues: ctx.issues };
  }
  if (input.type !== SCENE3D_TYPE) {
    ctx.err('type', `expected "${SCENE3D_TYPE}"`);
  }
  if (input.version !== SCENE3D_VERSION) {
    ctx.err('version', `expected ${SCENE3D_VERSION}`);
  }
  const cam = camera(input.camera, ctx);

  const objectsIn = input.objects === undefined ? [] : input.objects;
  if (!Array.isArray(objectsIn)) ctx.err('objects', 'expected an array');
  else if (objectsIn.length > SCENE3D_MAX_OBJECTS) ctx.err('objects', `too many objects (≤ ${SCENE3D_MAX_OBJECTS})`);
  const lightsIn = input.lights === undefined ? [] : input.lights;
  if (!Array.isArray(lightsIn)) ctx.err('lights', 'expected an array');
  else if (lightsIn.length > MAX_LIGHTS) ctx.err('lights', `too many lights (≤ ${MAX_LIGHTS})`);
  const groupsIn = input.groups === undefined ? [] : input.groups;
  if (!Array.isArray(groupsIn)) ctx.err('groups', 'expected an array');
  else if (groupsIn.length > MAX_GROUPS) ctx.err('groups', `too many groups (≤ ${MAX_GROUPS})`);

  if (ctx.issues.some((i) => !i.message.startsWith('(dropped)')) || !cam) {
    return { ok: false, doc: null, issues: ctx.issues };
  }

  const seen = new Set<string>();
  const objects: Scene3dObject[] = [];
  for (const [i, o] of (objectsIn as unknown[]).entries()) {
    const r = object(o, `objects[${i}]`, ctx, seen);
    if (r) objects.push(r);
  }
  const lights: Scene3dLight[] = [];
  for (const [i, l] of (lightsIn as unknown[]).entries()) {
    const r = light(l, `lights[${i}]`, ctx, seen);
    if (r) lights.push(r);
  }
  const groups: Scene3dGroup[] = [];
  for (const [i, g] of (groupsIn as unknown[]).entries()) {
    const r = group(g, `groups[${i}]`, ctx, seen);
    if (r) groups.push(r);
  }
  // Group children must exist and be claimed by ONE group; a group may not contain
  // itself or (transitively) an ancestor.
  const claimed = new Set<string>();
  for (const g of groups) {
    g.children = g.children.filter((c) => {
      if (c === g.id) {
        ctx.warn(`groups[${g.id}].children`, 'group contains itself');
        return false;
      }
      if (!seen.has(c) || lights.some((l) => l.id === c)) {
        ctx.warn(`groups[${g.id}].children`, `unknown child "${c}"`);
        return false;
      }
      if (claimed.has(c)) {
        ctx.warn(`groups[${g.id}].children`, `"${c}" already belongs to another group`);
        return false;
      }
      claimed.add(c);
      return true;
    });
  }
  const parentOf = new Map<string, string>();
  for (const g of groups) for (const c of g.children) parentOf.set(c, g.id);
  for (const g of groups) {
    let cur: string | undefined = parentOf.get(g.id);
    const hops = new Set<string>();
    while (cur) {
      if (cur === g.id) {
        ctx.err(`groups[${g.id}]`, 'group cycle');
        return { ok: false, doc: null, issues: ctx.issues };
      }
      if (hops.has(cur)) break;
      hops.add(cur);
      cur = parentOf.get(cur);
    }
  }

  if (ctx.issues.some((i) => !i.message.startsWith('(dropped)'))) {
    return { ok: false, doc: null, issues: ctx.issues };
  }

  const doc: Scene3dDoc = {
    type: SCENE3D_TYPE,
    version: SCENE3D_VERSION,
    camera: cam,
    lights,
    objects,
    groups,
  };
  const bg = color(input.background, 'background', ctx);
  if (bg) doc.background = bg;
  const grid = bool(input.grid, 'grid', ctx);
  if (grid !== undefined) doc.grid = grid;
  return { ok: true, doc, issues: ctx.issues };
}

/** Parse + validate a `scene.json` body. */
export function parseScene(text: string): ValidationResult {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (e) {
    return { ok: false, doc: null, issues: [{ path: '', message: `invalid JSON: ${(e as Error).message}` }] };
  }
  return validate(parsed);
}

/** Stable, human-diffable serialization (2-space indent, key order as authored). */
export function serializeScene(doc: Scene3dDoc): string {
  return JSON.stringify(doc, null, 2) + '\n';
}
