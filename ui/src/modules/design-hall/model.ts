// Design Hall — pure domain helpers (no Svelte, no fetch): studio metadata,
// status/relation vocabularies, the otto:// URI grammar, lineage + link
// grouping, the lobby's derived views and the route parser. Everything here is
// unit-tested (ui/unit/designHall.test.ts), so the components stay thin.
//
// Contract: docs/contracts/api.md § Design Hall; docs/features/design-hall.md.

import type {
  DesignArtifact,
  DesignLink,
  DesignLinkPolicy,
  DesignLinkRel,
  DesignProject,
  DesignSearchHit,
  DesignSignal,
  DesignSignalKind,
  DesignStatus,
  DesignStudio,
  DesignVersion,
} from '../../lib/api/types';

// ── Studios ─────────────────────────────────────────────────────────────────

/**
 * How a studio is served in Phase 0:
 * - `classic`: its own studio is on the roadmap; today's editor opens instead.
 * - `ready`: works end to end in Design Hall today.
 * - `planned`: no editor yet — the tile explains when it lands.
 */
export type StudioPhase = 'classic' | 'ready' | 'planned';

export interface StudioInfo {
  id: DesignStudio;
  name: string;
  /** One line under the name on the tile. */
  blurb: string;
  /** Icon name (typed at the component edge with `asIcon`). */
  icon: string;
  phase: StudioPhase;
  /** What opens today / when it lands — shown on the studio page. */
  note: string;
  /** Roadmap label for classic/planned studios ("v2", "Phase 1"). */
  roadmap: string | null;
  /** Formats a new artifact of this studio can start as, first = default. */
  formats: string[];
}

export const STUDIOS: readonly StudioInfo[] = [
  {
    id: 'frames',
    name: 'Frames',
    blurb: 'UI screens, flows and prototypes',
    icon: 'frame',
    phase: 'classic',
    note: 'The Frames canvas (auto-layout, components, prototype links) is planned for v2. Today a frame is an HTML screen you edit as source with a live device preview.',
    roadmap: 'v2',
    formats: ['html'],
  },
  {
    id: 'graphics',
    name: 'Graphics',
    blurb: 'Templates for social, decks and print',
    icon: 'image',
    phase: 'classic',
    note: 'Graphics Studio (templates, brand in one click, magic resize) is planned for v2. Today a graphic is an HTML board or an SVG, or an image you import.',
    roadmap: 'v2',
    formats: ['html', 'svg'],
  },
  {
    id: 'site',
    name: 'Site Studio',
    blurb: 'Websites from sections, published anywhere',
    icon: 'layout',
    phase: 'planned',
    note: 'Site Studio (sections, breakpoints, publish) lands in Phase 1. Until then a site is an HTML page you edit as source with a live preview.',
    roadmap: 'Phase 1',
    formats: [],
  },
  {
    id: '3d',
    name: '3D Studio',
    blurb: 'Models, scenes and text-to-3D',
    icon: 'box',
    phase: 'ready',
    note: '3D Studio 1.5: physical material presets, brand colours, environments, states, named views and a turntable. ✨ Generate runs an Otto agent turn (blockout, text or image → 3D, refine in Blender); export GLB, USDZ or PNG and optimize for the web.',
    roadmap: null,
    formats: ['scene3d'],
  },
  {
    id: 'whiteboard',
    name: 'Whiteboard',
    blurb: 'Excalidraw, Mermaid and D2 (today’s Canvas)',
    icon: 'edit',
    phase: 'classic',
    note: 'Whiteboard is today’s Canvas. Diagrams made here are versioned in Design Hall; Canvas boards keep opening in Canvas and are mirrored into the library.',
    roadmap: null,
    formats: ['mermaid', 'd2', 'excalidraw'],
  },
  {
    id: 'brand',
    name: 'Brand Kit',
    blurb: 'Colours, type and logos every studio uses',
    icon: 'palette',
    phase: 'ready',
    note: 'A brand kit is a versioned token document. Token editing with an impact preview lands in Phase 1.',
    roadmap: null,
    formats: ['otto-brand'],
  },
  {
    id: 'spatial',
    name: 'Spatial Hall',
    blurb: 'Walk your work as an exhibition',
    icon: 'gallery',
    phase: 'planned',
    note: 'Spatial Hall is a showcase view of your projects. The 3D gallery is planned for v3; a preview of the room layout is available from the lobby.',
    roadmap: 'v3',
    formats: [],
  },
];

export function studioInfo(id: string): StudioInfo {
  return STUDIOS.find((s) => s.id === id) ?? STUDIOS[0];
}

/** The CSS token that fills a studio's badge tile (defined in lib/tokens.css). */
const STUDIO_FILL: Record<DesignStudio, string> = {
  frames: 'var(--studio-frames)',
  graphics: 'var(--studio-graphics)',
  site: 'var(--studio-site)',
  '3d': 'var(--studio-3d)',
  whiteboard: 'var(--studio-whiteboard)',
  brand: 'var(--studio-brand)',
  spatial: 'var(--studio-spatial)',
};

export function studioColorVar(id: string): string {
  return STUDIO_FILL[id as DesignStudio] ?? STUDIO_FILL.frames;
}

// ── Formats ─────────────────────────────────────────────────────────────────

const TEXT_FORMATS = new Set(['html', 'mermaid', 'd2', 'svg']);
const JSON_FORMATS = new Set([
  'excalidraw',
  'scene3d',
  'otto-canvas',
  'otto-site',
  'otto-layout',
  'otto-brand',
  'otto-exhibit',
  'gltf',
]);
const IMAGE_FORMATS = new Set(['png', 'jpeg', 'gif', 'webp']);

export function isTextFormat(format: string): boolean {
  return TEXT_FORMATS.has(format) || JSON_FORMATS.has(format);
}
export function isJsonFormat(format: string): boolean {
  return JSON_FORMATS.has(format);
}
export function isImageFormat(format: string): boolean {
  return IMAGE_FORMATS.has(format);
}

/** How the artifact view renders a format. */
export type RenderKind =
  | 'html'
  | 'svg'
  | 'mermaid'
  | 'd2'
  | 'excalidraw'
  | 'canvas'
  | 'scene3d'
  | 'model'
  | 'image'
  | 'brand'
  | 'json'
  | 'pdf'
  | 'other';

export function renderKind(format: string): RenderKind {
  switch (format) {
    case 'html':
    case 'svg':
    case 'mermaid':
    case 'd2':
    case 'excalidraw':
    case 'scene3d':
    case 'pdf':
      return format;
    case 'otto-canvas':
      return 'canvas';
    case 'glb':
    case 'gltf':
      return 'model';
    case 'otto-brand':
      return 'brand';
    case 'otto-site':
    case 'otto-layout':
    case 'otto-exhibit':
      return 'json';
    default:
      return IMAGE_FORMATS.has(format) ? 'image' : 'other';
  }
}

/** Human name of a format ("HTML screen", "Mermaid diagram"…). */
export function formatLabel(format: string): string {
  const labels: Record<string, string> = {
    html: 'HTML',
    mermaid: 'Mermaid diagram',
    d2: 'D2 diagram',
    excalidraw: 'Excalidraw board',
    scene3d: '3D scene',
    'otto-canvas': 'Canvas board',
    'otto-site': 'Site',
    'otto-layout': 'Layout',
    'otto-brand': 'Brand kit',
    'otto-exhibit': 'Exhibition',
    svg: 'SVG',
    png: 'PNG image',
    jpeg: 'JPEG image',
    gif: 'GIF image',
    webp: 'WebP image',
    pdf: 'PDF',
    glb: 'GLB model',
    gltf: 'glTF model',
  };
  return labels[format] ?? format;
}

/** Map an imported file to a graph format, or null when unsupported. */
export function formatForFile(name: string, mime = ''): string | null {
  const ext = (name.split('.').pop() ?? '').toLowerCase();
  const byExt: Record<string, string> = {
    html: 'html',
    htm: 'html',
    svg: 'svg',
    png: 'png',
    jpg: 'jpeg',
    jpeg: 'jpeg',
    gif: 'gif',
    webp: 'webp',
    pdf: 'pdf',
    glb: 'glb',
    gltf: 'gltf',
    mmd: 'mermaid',
    mermaid: 'mermaid',
    d2: 'd2',
    excalidraw: 'excalidraw',
  };
  if (byExt[ext]) return byExt[ext];
  if (mime === 'text/html') return 'html';
  if (mime === 'image/svg+xml') return 'svg';
  if (mime === 'image/png') return 'png';
  if (mime === 'image/jpeg') return 'jpeg';
  return null;
}

// ── Status ──────────────────────────────────────────────────────────────────

export type Tone = 'neutral' | 'info' | 'warn' | 'ok' | 'bad';

export const STATUS_ORDER: DesignStatus[] = ['draft', 'review', 'approved', 'shipped'];

export function statusLabel(s: DesignStatus | string): string {
  const m: Record<string, string> = {
    draft: 'Draft',
    review: 'In review',
    approved: 'Approved',
    shipped: 'Shipped',
    archived: 'Archived',
  };
  return m[s] ?? s;
}

/** One place maps a status to a tone (the McpPill pattern). */
export function statusTone(s: DesignStatus | string): Tone {
  switch (s) {
    case 'review':
      return 'warn';
    case 'approved':
      return 'ok';
    case 'shipped':
      return 'info';
    default:
      return 'neutral';
  }
}

// ── Links ───────────────────────────────────────────────────────────────────

/** Relation as a short phrase, read from the link's source ("embeds", "derived from"). */
export function relLabel(rel: DesignLinkRel | string): string {
  const m: Record<string, string> = {
    embeds: 'embeds',
    uses_component: 'uses component',
    uses_tokens: 'uses tokens',
    describes: 'describes',
    derived_from: 'derived from',
    references: 'references',
    implements: 'implements',
    variant_of: 'variant of',
    resized_from: 'resized from',
    published_as: 'published as',
    exported_to: 'exported to',
    created_in: 'created in',
  };
  return m[rel] ?? rel.replace(/_/g, ' ');
}

/** Render relations are the ones a cycle check guards (embeds / components / tokens). */
export function isRenderRel(rel: string): boolean {
  return rel === 'embeds' || rel === 'uses_component' || rel === 'uses_tokens';
}

/** The relations a person can create by hand in the Links panel. */
export const EXPLICIT_RELS: DesignLinkRel[] = [
  'references',
  'implements',
  'describes',
  'embeds',
  'uses_component',
  'uses_tokens',
  'derived_from',
  'variant_of',
];

/** Version-policy pill text. `seqOf` resolves a pinned version id to its seq. */
export function policyLabel(
  policy: DesignLinkPolicy | string,
  pinnedSeq: number | null = null,
): string {
  if (policy === 'follow_approved') return 'follows approved';
  if (policy === 'follow_latest') return 'always latest';
  if (policy === 'pinned') return pinnedSeq != null ? `pinned v${pinnedSeq}` : 'pinned';
  return policy;
}

export interface OttoUri {
  artifactId: string;
  selector: { kind: 'default' } | { kind: 'approved' } | { kind: 'latest' } | { kind: 'version'; seq: number };
  node: string | null;
}

/**
 * Parse `otto://design/<artifact_id>[@approved|@latest|@v<seq>][#<node_id>]`.
 * Returns null for anything malformed (the server stores those as broken links).
 */
export function parseOttoUri(uri: string): OttoUri | null {
  const m = /^otto:\/\/design\/([A-Za-z0-9_-]+)(?:@([A-Za-z0-9]+))?(?:#(.+))?$/.exec(uri.trim());
  if (!m) return null;
  const [, artifactId, sel, node] = m;
  let selector: OttoUri['selector'] = { kind: 'default' };
  if (sel !== undefined) {
    if (sel === 'approved') selector = { kind: 'approved' };
    else if (sel === 'latest') selector = { kind: 'latest' };
    else {
      const v = /^v(\d+)$/.exec(sel);
      if (!v) return null;
      const seq = Number(v[1]);
      if (!Number.isSafeInteger(seq) || seq < 1) return null;
      selector = { kind: 'version', seq };
    }
  }
  if (node !== undefined && node.trim() === '') return null;
  return { artifactId, selector, node: node ?? null };
}

export function formatOttoUri(u: OttoUri): string {
  let s = `otto://design/${u.artifactId}`;
  if (u.selector.kind === 'approved') s += '@approved';
  else if (u.selector.kind === 'latest') s += '@latest';
  else if (u.selector.kind === 'version') s += `@v${u.selector.seq}`;
  if (u.node) s += `#${u.node}`;
  return s;
}

/** A row of the Links panel: the link plus the thing on its other end. */
export interface LinkRow {
  link: DesignLink;
  /** The artifact on the other end (when it is an artifact the caller may view). */
  other: DesignArtifact | null;
  /** Display title for non-artifact ends (story key, URL…). */
  label: string;
}

/**
 * Split a `GET …/links?dir=both` answer for artifact `id` into Uses (outgoing)
 * and Used in (incoming). `labelOf` names non-artifact endpoints (stories…).
 */
export function splitLinks(
  id: string,
  links: DesignLink[],
  artifacts: DesignArtifact[],
  labelOf: (kind: string, dstId: string) => string = (_k, d) => d,
): { uses: LinkRow[]; usedIn: LinkRow[] } {
  const byId = new Map(artifacts.map((a) => [a.id, a]));
  const uses: LinkRow[] = [];
  const usedIn: LinkRow[] = [];
  for (const link of links) {
    if (link.src_artifact_id === id) {
      const other = link.dst_kind === 'artifact' ? (byId.get(link.dst_id) ?? null) : null;
      uses.push({
        link,
        other,
        label: other ? other.title : labelOf(link.dst_kind, link.dst_id),
      });
    } else if (link.dst_kind === 'artifact' && link.dst_id === id) {
      const other = byId.get(link.src_artifact_id) ?? null;
      usedIn.push({ link, other, label: other ? other.title : 'An artifact you can’t view' });
    }
  }
  const order = (r: LinkRow) => LINEAGE_ORDER.indexOf(r.link.rel as DesignLinkRel);
  uses.sort((a, b) => order(a) - order(b) || a.label.localeCompare(b.label));
  usedIn.sort((a, b) => order(a) - order(b) || a.label.localeCompare(b.label));
  return { uses, usedIn };
}

/** Lineage reads provenance first, then what the design is built from, then what it serves. */
export const LINEAGE_ORDER: DesignLinkRel[] = [
  'derived_from',
  'variant_of',
  'resized_from',
  'uses_component',
  'embeds',
  'uses_tokens',
  'describes',
  'references',
  'implements',
  'published_as',
  'exported_to',
  'created_in',
];

export interface LineageEntry {
  rel: DesignLinkRel | string;
  relText: string;
  title: string;
  /** "v12", "follows approved" — the version the link resolves to. */
  version: string;
  /** Citation label for provenance links (R1, R2 …), else null. */
  cite: string | null;
  artifactId: string | null;
  broken: boolean;
}

/**
 * The Provenance/lineage list for an artifact's outgoing links: provenance
 * relations first, citation labels R1…Rn on the references/forks an agent
 * (or person) borrowed from. `seqOf` resolves pinned version ids.
 */
export function buildLineage(
  uses: LinkRow[],
  seqOf: (versionId: string) => number | null = () => null,
): LineageEntry[] {
  const rows = [...uses].sort(
    (a, b) =>
      LINEAGE_ORDER.indexOf(a.link.rel as DesignLinkRel) - LINEAGE_ORDER.indexOf(b.link.rel as DesignLinkRel),
  );
  let n = 0;
  return rows.map((r) => {
    const cited = r.link.rel === 'derived_from' || r.link.rel === 'references';
    const pinned = r.link.pinned_version_id ? seqOf(r.link.pinned_version_id) : null;
    const version =
      r.link.policy === 'pinned'
        ? pinned != null
          ? `v${pinned}`
          : 'pinned'
        : policyLabel(r.link.policy);
    return {
      rel: r.link.rel,
      relText: relLabel(r.link.rel),
      title: r.label,
      version,
      cite: cited && r.other ? `R${++n}` : null,
      artifactId: r.other?.id ?? null,
      broken: r.link.broken,
    };
  });
}

// ── Versions ────────────────────────────────────────────────────────────────

/** Who authored a version, as a short label: "you", "Otto", "import", or a user. */
export function versionAuthor(v: DesignVersion, meId: string | null | undefined): string {
  if (v.author_kind === 'agent') return 'Otto';
  if (v.author_kind === 'system') return v.kind === 'sync' ? 'sync' : 'import';
  if (meId && v.author_id === meId) return 'you';
  // Other people: user ids are opaque ULIDs — never show them as a name.
  return 'teammate';
}

/** Version strip order: oldest → newest (the strip reads left to right). */
export function stripOrder(versions: DesignVersion[]): DesignVersion[] {
  return [...versions].sort((a, b) => a.seq - b.seq);
}

/** Resolve a version id to its seq (for pinned-policy pills). */
export function seqLookup(versions: DesignVersion[]): (id: string) => number | null {
  const m = new Map(versions.map((v) => [v.id, v.seq]));
  return (id) => m.get(id) ?? null;
}

// ── Lobby ───────────────────────────────────────────────────────────────────

export type ProjectFilter = 'all' | 'mine' | 'epic' | 'shipped';

export interface ProjectStats {
  artifacts: number;
  studios: DesignStudio[];
  review: number;
  draft: number;
  approved: number;
  shipped: number;
  /** Up to four artifacts for the card mosaic (newest first). */
  mosaic: DesignArtifact[];
  /** Newest `updated_at` across the project and its artifacts. */
  updatedAt: string;
}

export function projectStats(p: DesignProject, all: DesignArtifact[]): ProjectStats {
  const mine = all
    .filter((a) => a.project_id === p.id && a.status !== 'archived')
    .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  const studios: DesignStudio[] = [];
  for (const a of mine) if (!studios.includes(a.studio)) studios.push(a.studio);
  const count = (s: DesignStatus) => mine.filter((a) => a.status === s).length;
  const newest = mine[0]?.updated_at ?? '';
  return {
    artifacts: mine.length,
    studios,
    review: count('review'),
    draft: count('draft'),
    approved: count('approved'),
    shipped: count('shipped'),
    mosaic: mine.slice(0, 4),
    updatedAt: newest > p.updated_at ? newest : p.updated_at,
  };
}

/** The status summary line on a project card ("1 in review · 2 drafts"). */
export function projectSummary(s: ProjectStats): { text: string; tone: Tone } {
  if (s.artifacts === 0) return { text: 'No artifacts yet', tone: 'neutral' };
  const parts: string[] = [];
  if (s.review) parts.push(`${s.review} in review`);
  if (s.draft) parts.push(`${s.draft} draft${s.draft === 1 ? '' : 's'}`);
  if (parts.length) return { text: parts.join(' · '), tone: s.review ? 'warn' : 'neutral' };
  if (s.shipped === s.artifacts) return { text: 'All shipped', tone: 'info' };
  return { text: 'All approved', tone: 'ok' };
}

export function filterProjects(
  projects: DesignProject[],
  artifacts: DesignArtifact[],
  filter: ProjectFilter,
  meId: string | null | undefined,
): DesignProject[] {
  const live = projects.filter((p) => !p.archived);
  switch (filter) {
    case 'mine':
      return live.filter(
        (p) =>
          p.created_by === meId ||
          artifacts.some((a) => a.project_id === p.id && a.created_by === meId),
      );
    case 'epic':
      return live
        .filter((p) => !!p.epic_story_id)
        .sort((a, b) => (a.epic_story_id ?? '').localeCompare(b.epic_story_id ?? ''));
    case 'shipped':
      return live.filter((p) => artifacts.some((a) => a.project_id === p.id && a.status === 'shipped'));
    default:
      return live;
  }
}

/** Minimal story shape the lobby needs (a ProductStory satisfies it). */
export interface StoryRef {
  id: string;
  source_key: string;
  title: string;
  parent_id: string | null;
  tree_kind?: string;
}

export interface EpicRow {
  /** The epic (or a top-level story with designs of its own). */
  story: StoryRef;
  /** Artifacts that implement the epic itself, per studio. */
  own: Partial<Record<DesignStudio, number>>;
  children: { story: StoryRef; counts: Partial<Record<DesignStudio, number>> }[];
  /** Children of the epic that are loaded (for "6 stories · 3 with designs"). */
  childCount: number;
}

/**
 * The Linked-to-Product strip: group `implements` targets (a search hit's
 * `story_ids`) under their epics with per-studio artifact counts. Stories the
 * caller can't resolve are skipped (their artifacts still show elsewhere).
 */
export function epicTree(hits: DesignSearchHit[], stories: StoryRef[]): EpicRow[] {
  const byId = new Map(stories.map((s) => [s.id, s]));
  const counts = new Map<string, Partial<Record<DesignStudio, number>>>();
  for (const h of hits) {
    if (h.artifact.status === 'archived') continue;
    for (const sid of h.story_ids) {
      const c = counts.get(sid) ?? {};
      c[h.artifact.studio] = (c[h.artifact.studio] ?? 0) + 1;
      counts.set(sid, c);
    }
  }
  const rows = new Map<string, EpicRow>();
  const rowFor = (epic: StoryRef): EpicRow => {
    let r = rows.get(epic.id);
    if (!r) {
      r = {
        story: epic,
        own: {},
        children: [],
        childCount: stories.filter((s) => s.parent_id === epic.id).length,
      };
      rows.set(epic.id, r);
    }
    return r;
  };
  for (const [sid, c] of counts) {
    const s = byId.get(sid);
    if (!s) continue;
    const parent = s.parent_id ? byId.get(s.parent_id) : undefined;
    if (parent) rowFor(parent).children.push({ story: s, counts: c });
    else rowFor(s).own = c;
  }
  const out = [...rows.values()];
  for (const r of out) r.children.sort((a, b) => a.story.source_key.localeCompare(b.story.source_key));
  // Busiest epics first (the strip opens the first one), then by key.
  const total = (r: EpicRow) => countTotal(r.own) + r.children.reduce((s, c) => s + countTotal(c.counts), 0);
  out.sort((a, b) => total(b) - total(a) || a.story.source_key.localeCompare(b.story.source_key));
  return out;
}

/** Total artifacts across a studio-count map. */
export function countTotal(c: Partial<Record<DesignStudio, number>>): number {
  return Object.values(c).reduce((s, n) => s + (n ?? 0), 0);
}

// ── Signals ─────────────────────────────────────────────────────────────────

export function signalLabel(kind: DesignSignalKind | string): string {
  const m: Record<string, string> = {
    variant_chosen: 'Variant chosen',
    variant_rejected: 'Variant rejected',
    edit_after_draft: 'Edit after draft',
    review_comment: 'Comment',
    critique_finding: 'Critique',
    a11y_fix: 'A11y fix',
    brand_correction: 'Brand correction',
    rule_feedback: 'Rule decision',
    status_change: 'Status change',
    shipped: 'Shipped',
  };
  return m[kind] ?? kind;
}

export function signalTone(kind: DesignSignalKind | string): Tone {
  switch (kind) {
    case 'variant_chosen':
    case 'a11y_fix':
      return 'ok';
    case 'variant_rejected':
      return 'bad';
    case 'brand_correction':
      return 'warn';
    case 'edit_after_draft':
    case 'shipped':
      return 'info';
    default:
      return 'neutral';
  }
}

/** One readable line from a signal's bounded payload (never raw content). */
export function signalSummary(s: DesignSignal): string {
  const p = s.payload ?? {};
  const str = (k: string): string | null => (typeof p[k] === 'string' && p[k] ? (p[k] as string) : null);
  const num = (k: string): number | null => (typeof p[k] === 'number' ? (p[k] as number) : null);
  if (str('summary')) return str('summary')!;
  switch (s.kind) {
    case 'status_change': {
      const from = str('from');
      const to = str('to');
      const seq = num('seq');
      if (to === 'approved' && seq != null) return `Approved v${seq}`;
      if (from && to) return `${statusLabel(from)} → ${statusLabel(to)}`;
      if (to) return `Moved to ${statusLabel(to)}`;
      return 'Status changed';
    }
    case 'shipped':
      return 'Marked shipped';
    case 'edit_after_draft': {
      // The server's structural summary (crates/otto-design/src/diff.rs) rides
      // in `payload.summary` as an object: changed JSON paths or line deltas.
      const sum = (p.summary && typeof p.summary === 'object' ? p.summary : {}) as Record<string, unknown>;
      const total = typeof sum.changed_total === 'number' ? sum.changed_total : null;
      const added = typeof sum.lines_added === 'number' ? sum.lines_added : 0;
      const removed = typeof sum.lines_removed === 'number' ? sum.lines_removed : 0;
      if (total) return `Edited ${total} field${total === 1 ? '' : 's'} after an agent draft`;
      if (added || removed) return `Changed +${added} −${removed} lines after an agent draft`;
      return 'Edited after an agent draft';
    }
    case 'variant_chosen': {
      const c = num('chosen_seq');
      const o = num('over_seq');
      if (c != null && o != null) return `Chose v${c} over v${o}`;
      return 'Chose a version';
    }
    case 'variant_rejected': {
      const r = num('rejected_seq');
      const reason = str('reason');
      return `${r != null ? `Rejected v${r}` : 'Rejected a version'}${reason ? ` · ${reason}` : ''}`;
    }
    default:
      return signalLabel(s.kind);
  }
}

// ── Routes ──────────────────────────────────────────────────────────────────

export type DesignRoute =
  | { view: 'lobby' }
  | { view: 'spatial' }
  | { view: 'artifact'; id: string }
  | { view: 'project'; id: string }
  | { view: 'studio'; id: DesignStudio }
  | { view: 'story'; id: string }
  | { view: 'brand' }
  | { view: 'learned'; tab: 'signals' | 'rules' | 'memory' };

/** `router.parts` (with parts[0] === 'design') → the Design Hall view. */
export function parseDesignRoute(parts: string[]): DesignRoute {
  const [, a, b] = parts;
  switch (a) {
    case 'spatial':
      return { view: 'spatial' };
    case 'a':
      return b ? { view: 'artifact', id: b } : { view: 'lobby' };
    case 'p':
      return b ? { view: 'project', id: b } : { view: 'lobby' };
    case 'story':
      return b ? { view: 'story', id: b } : { view: 'lobby' };
    case 'studio': {
      if (b === 'brand') return { view: 'brand' };
      if (b === 'spatial') return { view: 'spatial' };
      const s = STUDIOS.find((x) => x.id === b);
      return s ? { view: 'studio', id: s.id } : { view: 'lobby' };
    }
    case 'brand':
      return { view: 'brand' };
    case 'learned':
      return { view: 'learned', tab: b === 'rules' || b === 'memory' ? b : 'signals' };
    default:
      return { view: 'lobby' };
  }
}

// ── Creation ────────────────────────────────────────────────────────────────

/** A draft title from a free-text brief: first sentence, trimmed to 60 chars. */
export function titleFromPrompt(prompt: string): string {
  const first = prompt.trim().split(/[\n.!?]/)[0]?.trim() ?? '';
  if (!first) return 'Untitled design';
  const t = first.charAt(0).toUpperCase() + first.slice(1);
  return t.length > 60 ? `${t.slice(0, 59).trimEnd()}…` : t;
}

/** The starter brand-kit document (DTCG-style token groups). */
export function brandStarter(name: string): Record<string, unknown> {
  return {
    type: 'otto-brand',
    version: 1,
    name,
    color: {
      primary: { $type: 'color', $value: '#4F46E5' },
      accent: { $type: 'color', $value: '#F59E0B' },
      ink: { $type: 'color', $value: '#111827' },
      surface: { $type: 'color', $value: '#FFFFFF' },
      success: { $type: 'color', $value: '#15803D' },
    },
    typography: {
      display: { $type: 'typography', $value: { fontFamily: 'system-ui', fontSize: '64px', fontWeight: 800, lineHeight: '72px' } },
      body: { $type: 'typography', $value: { fontFamily: 'system-ui', fontSize: '17px', fontWeight: 400, lineHeight: '28px' } },
    },
    radius: { card: { $type: 'dimension', $value: '14px' } },
    voice: { summary: 'Warm, confident, never salesy.' },
  };
}

/** Colour tokens of a brand document (`color.<name>.$value`), in document order. */
export function brandColors(doc: unknown): { name: string; value: string }[] {
  if (!doc || typeof doc !== 'object') return [];
  const color = (doc as Record<string, unknown>).color;
  if (!color || typeof color !== 'object') return [];
  const out: { name: string; value: string }[] = [];
  for (const [name, tok] of Object.entries(color as Record<string, unknown>)) {
    const v = tok && typeof tok === 'object' ? (tok as Record<string, unknown>).$value : tok;
    if (typeof v === 'string' && /^#[0-9a-fA-F]{3,8}$/.test(v)) out.push({ name, value: v });
  }
  return out;
}

/** WCAG contrast ratio of two #rrggbb colours (brand swatch badges). */
export function contrastRatio(a: string, b: string): number | null {
  const lum = (hex: string): number | null => {
    let h = hex.replace('#', '');
    if (h.length === 3) h = h.split('').map((c) => c + c).join('');
    if (h.length !== 6 && h.length !== 8) return null;
    const ch = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
    if (ch.some((x) => Number.isNaN(x))) return null;
    const lin = ch.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
  };
  const la = lum(a);
  const lb = lum(b);
  if (la == null || lb == null) return null;
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return Math.round(((hi + 0.05) / (lo + 0.05)) * 10) / 10;
}
