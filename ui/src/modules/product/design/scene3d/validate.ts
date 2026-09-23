// scene3d validator — the TS mirror of `design_scene3d.rs::validate` (Track A).
// Runs before every render and before every save. It is deliberately strict and
// boring: known `type`s only, finite numbers, bounded arrays (≤ 2 000 objects),
// `attachment_id` must be a safe id component (it becomes a URL path segment on
// the authed fetch), colours are `#rrggbb`. Unknown keys are dropped on
// `normalize` so an agent typo never reaches the renderer.
//
// Reads v1 and v2; always WRITES v2 (the normalized doc carries `version: 2`,
// so a v1 scene upgrades on its next save — the Rust side accepts both). v2
// adds `token:color.<name>` colours, physical material fields, `brand` /
// `gltf.src` design URIs, environment, cameras, states and the turntable.
// Malformed v2 pieces are dropped with a warning; only what Rust would also
// refuse outright (a gltf with neither/both refs, a bad src) is fatal.
import {
  EASINGS,
  ENV_PRESET_IDS,
  LIGHT_TYPES,
  MATERIAL_PRESET_IDS,
  OBJECT_TYPES,
  SCENE3D_MAX_OBJECTS,
  SCENE3D_TYPE,
  SCENE3D_VERSION,
  SCENE3D_VERSIONS,
  type Easing,
  type EnvPresetId,
  type LightType,
  type MaterialPresetId,
  type ObjectType,
  type Scene3dCamera,
  type Scene3dCameraPreset,
  type Scene3dDoc,
  type Scene3dEnvironment,
  type Scene3dGroup,
  type Scene3dLight,
  type Scene3dMaterial,
  type Scene3dObject,
  type Scene3dState,
  type Scene3dStateOverride,
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
const MAX_CAMERAS = 32;
const MAX_STATES = 32;
const MAX_DURATION_MS = 10_000;
const TOKEN_COLOR = /^token:color\.[A-Za-z0-9_.-]{1,96}$/;
/** `otto://design/<id>[@approved|@latest|@vN][#node]` — the grammar of `uri.rs`. */
const DESIGN_URI = /^otto:\/\/design\/[A-Za-z0-9_-]{1,64}(?:@(?:approved|latest|v[1-9][0-9]{0,17}))?(?:#[A-Za-z0-9_:.-]{1,128})?$/;

export function isSafeId(s: unknown): s is string {
  return typeof s === 'string' && SAFE_ID.test(s);
}
export function isHexColor(s: unknown): s is string {
  return typeof s === 'string' && HEX_COLOR.test(s);
}
/** `#rrggbb` or (v2) `token:color.<name>`. */
export function isColorRef(s: unknown): s is string {
  return typeof s === 'string' && (HEX_COLOR.test(s) || TOKEN_COLOR.test(s));
}
export function isDesignUri(s: unknown): s is string {
  return typeof s === 'string' && DESIGN_URI.test(s.trim());
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
/** Material / override colours: hex (lower-cased) or a brand token (kept as written). */
function colorRef(v: unknown, path: string, ctx: Ctx): string | undefined {
  if (v === undefined) return undefined;
  if (typeof v === 'string' && TOKEN_COLOR.test(v)) return v;
  if (!isHexColor(v)) {
    ctx.warn(path, 'expected "#rrggbb" or "token:color.<name>"');
    return undefined;
  }
  return v.toLowerCase();
}
function ranged(v: unknown, path: string, ctx: Ctx, lo: number, hi: number): number | undefined {
  if (v === undefined) return undefined;
  if (!finite(v) || v < lo || v > hi) {
    ctx.warn(path, `expected a number in ${lo}..${hi}`);
    return undefined;
  }
  return v;
}
function oneOf<T extends string>(v: unknown, path: string, ctx: Ctx, allowed: readonly T[]): T | undefined {
  if (v === undefined) return undefined;
  if (typeof v !== 'string' || !allowed.includes(v as T)) {
    ctx.warn(path, `expected one of ${allowed.join(' | ')}`);
    return undefined;
  }
  return v as T;
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
  const preset = oneOf<MaterialPresetId>(v.preset, `${path}.preset`, ctx, MATERIAL_PRESET_IDS);
  if (preset) m.preset = preset;
  const c = colorRef(v.color, `${path}.color`, ctx);
  if (c) m.color = c;
  const e = colorRef(v.emissive, `${path}.emissive`, ctx);
  if (e) m.emissive = e;
  const met = unit(v.metalness, `${path}.metalness`, ctx);
  if (met !== undefined) m.metalness = met;
  const rough = unit(v.roughness, `${path}.roughness`, ctx);
  if (rough !== undefined) m.roughness = rough;
  const op = unit(v.opacity, `${path}.opacity`, ctx);
  if (op !== undefined) m.opacity = op;
  const wf = bool(v.wireframe, `${path}.wireframe`, ctx);
  if (wf !== undefined) m.wireframe = wf;
  for (const k of ['clearcoat', 'clearcoat_roughness', 'transmission', 'sheen'] as const) {
    const n = unit(v[k], `${path}.${k}`, ctx);
    if (n !== undefined) m[k] = n;
  }
  const ior = ranged(v.ior, `${path}.ior`, ctx, 1, 2.333);
  if (ior !== undefined) m.ior = ior;
  const th = ranged(v.thickness, `${path}.thickness`, ctx, 0, 10);
  if (th !== undefined) m.thickness = th;
  const ei = ranged(v.emissive_intensity, `${path}.emissive_intensity`, ctx, 0, 100);
  if (ei !== undefined) m.emissive_intensity = ei;
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
    if (v.attachment_id !== undefined && v.src !== undefined) {
      ctx.err(path, 'gltf objects take attachment_id OR src, not both');
      return null;
    }
    if (v.src !== undefined) {
      if (!isDesignUri(v.src)) {
        ctx.err(`${path}.src`, 'src must be an otto://design/<id>[@approved|@latest|@vN] reference (never a URL)');
        return null;
      }
      out.src = (v.src as string).trim();
    } else if (!isSafeId(v.attachment_id)) {
      ctx.err(`${path}.attachment_id`, 'gltf objects need a safe attachment_id or an otto://design src (never a URL)');
      return null;
    } else {
      out.attachment_id = v.attachment_id;
    }
  } else {
    if (v.attachment_id !== undefined) ctx.warn(`${path}.attachment_id`, 'only gltf objects carry attachment_id');
    if (v.src !== undefined) ctx.warn(`${path}.src`, 'only gltf objects carry src');
  }
  if (v.radius !== undefined) {
    if (type !== 'box') ctx.warn(`${path}.radius`, 'only box objects carry radius');
    else {
      const r = ranged(v.radius, `${path}.radius`, ctx, 0, 0.5);
      if (r !== undefined) out.radius = r;
    }
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
  if (!(SCENE3D_VERSIONS as readonly unknown[]).includes(input.version)) {
    ctx.err('version', `expected ${SCENE3D_VERSIONS.join(' or ')}`);
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
  validateV2(input, doc, ctx, new Set(objects.map((o) => o.id)));
  return { ok: true, doc, issues: ctx.issues };
}

/** v2 top-level blocks. Malformed pieces are dropped (warned), never fatal. */
function validateV2(input: Record<string, unknown>, doc: Scene3dDoc, ctx: Ctx, objectIds: Set<string>): void {
  if (input.brand !== undefined) {
    if (isDesignUri(input.brand)) doc.brand = input.brand.trim();
    else ctx.warn('brand', 'expected an otto://design/<id>[@approved] reference');
  }
  if (input.environment !== undefined) {
    const e = input.environment;
    const preset = isRecord(e) ? oneOf<EnvPresetId>(e.preset, 'environment.preset', ctx, ENV_PRESET_IDS) : undefined;
    if (!isRecord(e) || !preset) ctx.warn('environment', 'expected {preset: studio-soft | sunset | night | none}');
    else {
      const env: Scene3dEnvironment = { preset };
      const i = ranged(e.intensity, 'environment.intensity', ctx, 0, 10);
      if (i !== undefined) env.intensity = i;
      const b = bool(e.background, 'environment.background', ctx);
      if (b !== undefined) env.background = b;
      const r = ranged(e.rotation, 'environment.rotation', ctx, -360, 360);
      if (r !== undefined) env.rotation = r;
      doc.environment = env;
    }
  }
  if (input.turntable !== undefined) {
    const t = input.turntable;
    if (!isRecord(t)) ctx.warn('turntable', 'expected an object');
    else {
      const tt: NonNullable<Scene3dDoc['turntable']> = {};
      const en = bool(t.enabled, 'turntable.enabled', ctx);
      if (en !== undefined) tt.enabled = en;
      const sp = ranged(t.speed, 'turntable.speed', ctx, -360, 360);
      if (sp !== undefined) tt.speed = sp;
      doc.turntable = tt;
    }
  }
  if (input.cameras !== undefined) {
    if (!Array.isArray(input.cameras)) ctx.warn('cameras', 'expected an array');
    else {
      const seen = new Set<string>();
      const cams: Scene3dCameraPreset[] = [];
      for (const [i, c] of input.cameras.slice(0, MAX_CAMERAS).entries()) {
        const path = `cameras[${i}]`;
        if (!isRecord(c) || !isSafeId(c.id) || seen.has(c.id)) {
          ctx.warn(path, 'expected {id (unique, safe), position, target}');
          continue;
        }
        const sub = new Ctx();
        const position = vec3(c.position, `${path}.position`, sub, null);
        const target = vec3(c.target, `${path}.target`, sub, [0, 0, 0]);
        if (!position || !target) {
          ctx.warn(path, 'expected finite position/target');
          continue;
        }
        const cam: Scene3dCameraPreset = { id: c.id, position, target };
        const name = str(c.name, `${path}.name`, ctx, MAX_NAME);
        if (name) cam.name = name;
        const fov = ranged(c.fov, `${path}.fov`, ctx, 1, 179);
        if (fov !== undefined) cam.fov = fov;
        seen.add(c.id);
        cams.push(cam);
      }
      if (input.cameras.length > MAX_CAMERAS) ctx.warn('cameras', `only the first ${MAX_CAMERAS} are kept`);
      doc.cameras = cams;
    }
  }
  if (input.states !== undefined) {
    if (!Array.isArray(input.states)) ctx.warn('states', 'expected an array');
    else {
      const seen = new Set<string>();
      const states: Scene3dState[] = [];
      for (const [i, sv] of input.states.slice(0, MAX_STATES).entries()) {
        const path = `states[${i}]`;
        if (!isRecord(sv) || !isSafeId(sv.id) || seen.has(sv.id)) {
          ctx.warn(path, 'expected {id (unique, safe), name?, overrides?}');
          continue;
        }
        const st: Scene3dState = { id: sv.id };
        const name = str(sv.name, `${path}.name`, ctx, MAX_NAME);
        if (name) st.name = name;
        const d = ranged(sv.duration_ms, `${path}.duration_ms`, ctx, 0, MAX_DURATION_MS);
        if (d !== undefined) st.duration_ms = Math.round(d);
        const easing = oneOf<Easing>(sv.easing, `${path}.easing`, ctx, EASINGS);
        if (easing) st.easing = easing;
        if (sv.overrides !== undefined) {
          if (!isRecord(sv.overrides)) ctx.warn(`${path}.overrides`, 'expected {objectId: override}');
          else {
            const ovs: Record<string, Scene3dStateOverride> = {};
            for (const [oid, ov] of Object.entries(sv.overrides)) {
              const opath = `${path}.overrides.${oid}`;
              if (!objectIds.has(oid) || !isRecord(ov)) {
                ctx.warn(opath, 'unknown object (or not an object)');
                continue;
              }
              ovs[oid] = stateOverride(ov, opath, ctx);
            }
            st.overrides = ovs;
          }
        }
        seen.add(sv.id);
        states.push(st);
      }
      if (input.states.length > MAX_STATES) ctx.warn('states', `only the first ${MAX_STATES} are kept`);
      doc.states = states;
      if (input.default_state !== undefined) {
        if (typeof input.default_state === 'string' && seen.has(input.default_state)) doc.default_state = input.default_state;
        else ctx.warn('default_state', 'not a state id');
      }
    }
  } else if (input.default_state !== undefined) {
    ctx.warn('default_state', 'no states defined');
  }
}

function stateOverride(ov: Record<string, unknown>, path: string, ctx: Ctx): Scene3dStateOverride {
  const out: Scene3dStateOverride = {};
  for (const k of ['position', 'rotation', 'scale'] as const) {
    if (ov[k] === undefined) continue;
    const v = vec3(ov[k], `${path}.${k}`, new Ctx(), null);
    if (v) out[k] = v;
    else ctx.warn(`${path}.${k}`, 'expected [x, y, z] finite numbers');
  }
  const vis = bool(ov.visible, `${path}.visible`, ctx);
  if (vis !== undefined) out.visible = vis;
  const op = unit(ov.opacity, `${path}.opacity`, ctx);
  if (op !== undefined) out.opacity = op;
  const c = colorRef(ov.color, `${path}.color`, ctx);
  if (c) out.color = c;
  const e = colorRef(ov.emissive, `${path}.emissive`, ctx);
  if (e) out.emissive = e;
  return out;
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
