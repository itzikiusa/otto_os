// Pure document mutations for scene3d. EVERY edit — the viewport gizmo, ⌘D/Del,
// the hierarchy's rename/hide/duplicate/delete, the inspector's fields — goes
// through one of these `(doc, …) => doc` functions; no component mutates `doc`
// in place. Each returns a NEW document (the input is never touched), so the
// caller can diff, debounce and autosave (Track B owns the 600 ms debounce +
// dirty/conflict state; we just emit `onchange(newDoc)` undebounced).
import type {
  GizmoMode,
  LightType,
  PrimitiveType,
  Scene3dCamera,
  Scene3dDoc,
  Scene3dGroup,
  Scene3dLight,
  Scene3dMaterial,
  Scene3dNodeKind,
  Scene3dObject,
  Vec3,
} from './types';

// JSON round-trip rather than `structuredClone`: the doc is pure JSON, and the
// caller may hand us a Svelte `$state` proxy, which structuredClone refuses to clone.
export function cloneDoc(doc: Scene3dDoc): Scene3dDoc {
  return JSON.parse(JSON.stringify(doc)) as Scene3dDoc;
}
function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

/** Anything selectable by id — an object, a light or a group. */
export type Scene3dNode =
  | { kind: 'object'; node: Scene3dObject }
  | { kind: 'light'; node: Scene3dLight }
  | { kind: 'group'; node: Scene3dGroup };

export function findNode(doc: Scene3dDoc, id: string | null | undefined): Scene3dNode | null {
  if (!id) return null;
  const o = doc.objects.find((x) => x.id === id);
  if (o) return { kind: 'object', node: o };
  const l = doc.lights.find((x) => x.id === id);
  if (l) return { kind: 'light', node: l };
  const g = doc.groups.find((x) => x.id === id);
  if (g) return { kind: 'group', node: g };
  return null;
}

export function nodeKind(doc: Scene3dDoc, id: string | null | undefined): Scene3dNodeKind | null {
  return findNode(doc, id)?.kind ?? null;
}

/** Display name for any node (lights may omit `name`). */
export function nodeLabel(n: Scene3dNode): string {
  return n.node.name || n.node.id;
}

/** The group that directly contains `id`, if any. */
export function parentGroup(doc: Scene3dDoc, id: string): Scene3dGroup | null {
  return doc.groups.find((g) => g.children.includes(id)) ?? null;
}

/** Effective visibility: a node is hidden if it or any ancestor group is hidden. */
export function isEffectivelyVisible(doc: Scene3dDoc, id: string): boolean {
  const n = findNode(doc, id);
  if (!n) return false;
  if (n.node.visible === false) return false;
  let cur = parentGroup(doc, id);
  const hops = new Set<string>();
  while (cur && !hops.has(cur.id)) {
    if (cur.visible === false) return false;
    hops.add(cur.id);
    cur = parentGroup(doc, cur.id);
  }
  return true;
}

/** Slug + numeric suffix, unique across objects, lights and groups. */
export function uniqueId(doc: Scene3dDoc, base: string): string {
  const slug =
    base
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 40) || 'node';
  const taken = new Set<string>([
    ...doc.objects.map((o) => o.id),
    ...doc.lights.map((l) => l.id),
    ...doc.groups.map((g) => g.id),
  ]);
  if (!taken.has(slug)) return slug;
  for (let i = 2; ; i++) {
    const cand = `${slug}-${i}`;
    if (!taken.has(cand)) return cand;
  }
}

/** "Crate", "Crate 2", "Crate 3"… — unique display names keep the hierarchy readable. */
function uniqueName(doc: Scene3dDoc, base: string): string {
  const names = new Set<string>([
    ...doc.objects.map((o) => o.name),
    ...doc.lights.map((l) => l.name ?? ''),
    ...doc.groups.map((g) => g.name),
  ]);
  if (!names.has(base)) return base;
  const m = /^(.*?)(?: (\d+))?$/.exec(base);
  const stem = m?.[1] ?? base;
  for (let i = Number(m?.[2] ?? 1) + 1; ; i++) {
    const cand = `${stem} ${i}`;
    if (!names.has(cand)) return cand;
  }
}

const PRIMITIVE_LABEL: Record<PrimitiveType, string> = {
  box: 'Box',
  sphere: 'Sphere',
  cylinder: 'Cylinder',
  cone: 'Cone',
  torus: 'Torus',
  plane: 'Plane',
  text: 'Text',
};

const LIGHT_LABEL: Record<LightType, string> = {
  directional: 'Sun',
  ambient: 'Ambient',
  point: 'Point light',
  spot: 'Spot light',
  hemisphere: 'Sky',
};

export interface AddOptions {
  /** World position; primitives default to resting on the floor at the origin. */
  position?: Vec3;
  /** Put the new node straight into this group. */
  groupId?: string | null;
  name?: string;
}

function attachToGroup(doc: Scene3dDoc, id: string, groupId: string | null | undefined): void {
  if (!groupId) return;
  const g = doc.groups.find((x) => x.id === groupId);
  if (g && !g.children.includes(id)) g.children.push(id);
}

/** Add a primitive with a sensible default material; the unit sits on the floor (y = 0). */
export function addPrimitive(doc: Scene3dDoc, type: PrimitiveType, opts: AddOptions = {}): { doc: Scene3dDoc; id: string } {
  const next = cloneDoc(doc);
  const name = uniqueName(next, opts.name ?? PRIMITIVE_LABEL[type]);
  const id = uniqueId(next, name);
  // Resting height for a unit primitive: half its height, planes lie flat.
  const restY = type === 'plane' ? 0 : type === 'torus' ? 0.4 : 0.5;
  const obj: Scene3dObject = {
    id,
    name,
    type,
    position: opts.position ?? [0, restY, 0],
    rotation: type === 'plane' ? [-90, 0, 0] : [0, 0, 0],
    scale: [1, 1, 1],
    material: { color: '#94a3b8', metalness: 0.1, roughness: 0.7 },
  };
  if (type === 'text') obj.text = name;
  next.objects.push(obj);
  attachToGroup(next, id, opts.groupId);
  return { doc: next, id };
}

/** Add a `gltf` object referencing an uploaded attachment (never a URL). */
export function addGltf(doc: Scene3dDoc, attachmentId: string, opts: AddOptions = {}): { doc: Scene3dDoc; id: string } {
  const next = cloneDoc(doc);
  const name = uniqueName(next, opts.name ?? 'Model');
  const id = uniqueId(next, name);
  next.objects.push({
    id,
    name,
    type: 'gltf',
    attachment_id: attachmentId,
    position: opts.position ?? [0, 0, 0],
    rotation: [0, 0, 0],
    scale: [1, 1, 1],
  });
  attachToGroup(next, id, opts.groupId);
  return { doc: next, id };
}

export function addLight(doc: Scene3dDoc, type: LightType, opts: AddOptions = {}): { doc: Scene3dDoc; id: string } {
  const next = cloneDoc(doc);
  const name = uniqueName(next, opts.name ?? LIGHT_LABEL[type]);
  const id = uniqueId(next, name);
  const light: Scene3dLight = { id, name, type, intensity: type === 'ambient' ? 0.4 : 1, color: '#ffffff' };
  if (type !== 'ambient' && type !== 'hemisphere') light.position = opts.position ?? [3, 4, 3];
  if (type === 'directional' || type === 'spot') light.target = [0, 0, 0];
  if (type === 'directional') light.shadow = true;
  if (type === 'spot') light.angle = 30;
  if (type === 'hemisphere') light.ground_color = '#334155';
  next.lights.push(light);
  return { doc: next, id };
}

/** Create a group, optionally adopting existing nodes (they leave their old group). */
export function addGroup(doc: Scene3dDoc, name = 'Group', childIds: string[] = []): { doc: Scene3dDoc; id: string } {
  const next = cloneDoc(doc);
  const gname = uniqueName(next, name);
  const id = uniqueId(next, gname);
  const valid = childIds.filter((c) => c !== id && findNode(next, c) && findNode(next, c)!.kind !== 'light');
  for (const g of next.groups) g.children = g.children.filter((c) => !valid.includes(c));
  next.groups.push({ id, name: gname, children: valid });
  return { doc: next, id };
}

/** Move a node into `groupId` (or to the top level with `null`). Refuses cycles. */
export function moveToGroup(doc: Scene3dDoc, id: string, groupId: string | null): Scene3dDoc {
  const n = findNode(doc, id);
  if (!n || n.kind === 'light') return doc;
  if (groupId) {
    // Cannot move a group into itself or into one of its own descendants.
    let cur: string | null = groupId;
    const hops = new Set<string>();
    while (cur && !hops.has(cur)) {
      if (cur === id) return doc;
      hops.add(cur);
      cur = parentGroup(doc, cur)?.id ?? null;
    }
  }
  const next = cloneDoc(doc);
  for (const g of next.groups) g.children = g.children.filter((c) => c !== id);
  attachToGroup(next, id, groupId);
  return next;
}

/** Delete a node. Deleting a group deletes its children too (whole subtree). */
export function remove(doc: Scene3dDoc, id: string): Scene3dDoc {
  const n = findNode(doc, id);
  if (!n) return doc;
  const next = cloneDoc(doc);
  const doomed = new Set<string>([id]);
  if (n.kind === 'group') {
    const stack = [id];
    while (stack.length) {
      const gid = stack.pop()!;
      const g = next.groups.find((x) => x.id === gid);
      for (const c of g?.children ?? []) {
        if (doomed.has(c)) continue;
        doomed.add(c);
        if (next.groups.some((x) => x.id === c)) stack.push(c);
      }
    }
  }
  next.objects = next.objects.filter((o) => !doomed.has(o.id));
  next.lights = next.lights.filter((l) => !doomed.has(l.id));
  next.groups = next.groups.filter((g) => !doomed.has(g.id));
  for (const g of next.groups) g.children = g.children.filter((c) => !doomed.has(c));
  return next;
}

/** Duplicate a node (deep for groups) with fresh ids/names, nudged +1 m on X. */
export function duplicate(doc: Scene3dDoc, id: string): { doc: Scene3dDoc; id: string } | null {
  const n = findNode(doc, id);
  if (!n) return null;
  const next = cloneDoc(doc);
  const parent = parentGroup(next, id);
  const idMap = new Map<string, string>();

  const dupObject = (o: Scene3dObject, nudge: boolean): string => {
    const name = uniqueName(next, o.name);
    const nid = uniqueId(next, name);
    const copy: Scene3dObject = clone(o);
    copy.id = nid;
    copy.name = name;
    if (nudge) copy.position = [o.position[0] + 1, o.position[1], o.position[2]];
    next.objects.push(copy);
    idMap.set(o.id, nid);
    return nid;
  };
  const dupLight = (l: Scene3dLight, nudge: boolean): string => {
    const name = uniqueName(next, l.name ?? l.id);
    const nid = uniqueId(next, name);
    const copy: Scene3dLight = clone(l);
    copy.id = nid;
    copy.name = name;
    if (nudge && copy.position) copy.position = [copy.position[0] + 1, copy.position[1], copy.position[2]];
    next.lights.push(copy);
    idMap.set(l.id, nid);
    return nid;
  };
  const dupGroup = (g: Scene3dGroup, nudge: boolean): string => {
    const name = uniqueName(next, g.name);
    const nid = uniqueId(next, name);
    const children: string[] = [];
    for (const c of g.children) {
      const cn = findNode(doc, c);
      if (!cn) continue;
      if (cn.kind === 'object') children.push(dupObject(cn.node, nudge));
      else if (cn.kind === 'group') children.push(dupGroup(cn.node, nudge));
    }
    next.groups.push({ ...clone(g), id: nid, name, children });
    idMap.set(g.id, nid);
    return nid;
  };

  let nid: string;
  if (n.kind === 'object') nid = dupObject(n.node, true);
  else if (n.kind === 'light') nid = dupLight(n.node, true);
  else nid = dupGroup(n.node, true);
  if (parent) attachToGroup(next, nid, parent.id);
  return { doc: next, id: nid };
}

export function rename(doc: Scene3dDoc, id: string, name: string): Scene3dDoc {
  const trimmed = name.trim().slice(0, 200);
  if (!trimmed) return doc;
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n) return doc;
  n.node.name = trimmed;
  if (n.kind === 'object' && n.node.type === 'text' && !n.node.text) n.node.text = trimmed;
  return next;
}

export function setVisible(doc: Scene3dDoc, id: string, visible: boolean): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n) return doc;
  if (visible) delete n.node.visible;
  else n.node.visible = false;
  return next;
}

export function setNotes(doc: Scene3dDoc, id: string, notes: string): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n) return doc;
  const t = notes.slice(0, 4000);
  if (t) n.node.notes = t;
  else delete n.node.notes;
  return next;
}

export interface TransformPatch {
  position?: Vec3;
  /** Degrees. */
  rotation?: Vec3;
  scale?: Vec3;
}

const round = (v: number, places = 4): number => {
  const f = 10 ** places;
  const r = Math.round(v * f) / f;
  return Object.is(r, -0) ? 0 : r;
};
const roundVec = (v: Vec3): Vec3 => [round(v[0]), round(v[1]), round(v[2])];
const finiteVec = (v: Vec3 | undefined): v is Vec3 => !!v && v.every((n) => Number.isFinite(n));

/** Objects: full transform. Lights: position only (and `target` via `patchLight`). */
export function setTransform(doc: Scene3dDoc, id: string, patch: TransformPatch): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n || n.kind === 'group') return doc;
  if (n.kind === 'object') {
    if (finiteVec(patch.position)) n.node.position = roundVec(patch.position);
    if (finiteVec(patch.rotation)) n.node.rotation = roundVec(patch.rotation);
    if (finiteVec(patch.scale)) n.node.scale = roundVec(patch.scale);
  } else if (finiteVec(patch.position) && n.node.type !== 'ambient' && n.node.type !== 'hemisphere') {
    n.node.position = roundVec(patch.position);
  }
  return next;
}

/** Merge material fields; `undefined` values delete the key (back to renderer default). */
export function setMaterial(doc: Scene3dDoc, id: string, patch: Partial<Scene3dMaterial>): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n || n.kind !== 'object' || n.node.type === 'gltf') return doc;
  const m: Scene3dMaterial = { ...(n.node.material ?? {}) };
  for (const [k, v] of Object.entries(patch) as [keyof Scene3dMaterial, unknown][]) {
    if (v === undefined || v === null || v === '') delete m[k];
    else (m as Record<string, unknown>)[k] = v;
  }
  if (Object.keys(m).length) n.node.material = m;
  else delete n.node.material;
  return next;
}

export function setText(doc: Scene3dDoc, id: string, text: string): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n || n.kind !== 'object' || n.node.type !== 'text') return doc;
  n.node.text = text.slice(0, 500);
  return next;
}

/** Merge light fields (intensity/color/shadow/target/angle/…); `undefined` deletes. */
export function patchLight(doc: Scene3dDoc, id: string, patch: Partial<Omit<Scene3dLight, 'id' | 'type'>>): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n || n.kind !== 'light') return doc;
  for (const [k, v] of Object.entries(patch) as [keyof Scene3dLight, unknown][]) {
    if (k === 'id' || k === 'type') continue;
    if (v === undefined || v === null || v === '') delete n.node[k];
    else (n.node as unknown as Record<string, unknown>)[k] = Array.isArray(v) ? roundVec(v as Vec3) : v;
  }
  return next;
}

export function setCamera(doc: Scene3dDoc, patch: Partial<Scene3dCamera>): Scene3dDoc {
  const next = cloneDoc(doc);
  if (finiteVec(patch.position)) next.camera.position = roundVec(patch.position);
  if (finiteVec(patch.target)) next.camera.target = roundVec(patch.target);
  if (patch.fov !== undefined && Number.isFinite(patch.fov)) next.camera.fov = Math.min(179, Math.max(1, round(patch.fov, 2)));
  if (patch.near !== undefined) {
    if (Number.isFinite(patch.near) && patch.near > 0) next.camera.near = round(patch.near);
    else delete next.camera.near;
  }
  if (patch.far !== undefined) {
    if (Number.isFinite(patch.far) && patch.far > 0) next.camera.far = round(patch.far);
    else delete next.camera.far;
  }
  return next;
}

export function setScene(doc: Scene3dDoc, patch: { background?: string | null; grid?: boolean }): Scene3dDoc {
  const next = cloneDoc(doc);
  if (patch.background !== undefined) {
    if (patch.background) next.background = patch.background.toLowerCase();
    else delete next.background;
  }
  if (patch.grid !== undefined) next.grid = patch.grid;
  return next;
}

/** Bring an id to the end of its list (draw/hierarchy order) — small ergonomic helper. */
export function reorder(doc: Scene3dDoc, id: string, dir: -1 | 1): Scene3dDoc {
  const next = cloneDoc(doc);
  const n = findNode(next, id);
  if (!n) return doc;
  const parent = parentGroup(next, id);
  if (parent) {
    const i = parent.children.indexOf(id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= parent.children.length) return doc;
    [parent.children[i], parent.children[j]] = [parent.children[j], parent.children[i]];
    return next;
  }
  const list: { id: string }[] = n.kind === 'object' ? next.objects : n.kind === 'light' ? next.lights : next.groups;
  const i = list.findIndex((x) => x.id === id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= list.length) return doc;
  [list[i], list[j]] = [list[j], list[i]];
  return next;
}

/** Keyboard → gizmo mode (W/E/R). Returns null for any other key. */
export function gizmoModeForKey(key: string): GizmoMode | null {
  switch (key.toLowerCase()) {
    case 'w':
      return 'translate';
    case 'e':
      return 'rotate';
    case 'r':
      return 'scale';
    default:
      return null;
  }
}

/** Rough object count line for the arena status bar ("12 objects · 2 lights"). */
export function summarize(doc: Scene3dDoc): string {
  const o = doc.objects.length;
  const l = doc.lights.length;
  const g = doc.groups.length;
  const parts = [`${o} object${o === 1 ? '' : 's'}`, `${l} light${l === 1 ? '' : 's'}`];
  if (g) parts.push(`${g} group${g === 1 ? '' : 's'}`);
  return parts.join(' · ');
}
