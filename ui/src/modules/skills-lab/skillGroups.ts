// Pure grouping + frontmatter helpers for Skills Lab → Skills — no Svelte, no
// imports, unit-tested in ui/unit/skillGroups.test.ts.
//
// The same skill can live in up to five places: the editable Otto library,
// the bundled catalog (compiled into ottod), and each provider's global skills
// dir (~/.claude/skills, ~/.codex/skills, ~/.agy/skills). The old list showed
// one row per PLACE (vault-api-review ×3); this groups them into one row per
// NAME with a variant per place and a sync state:
//
//   in_sync  — every copy we could compare matches
//   drifted  — a copy differs (body text, or bundled vs installed version)
//   single   — it only exists in one place
//   unknown  — several copies, but some bodies haven't been compared yet

export type VariantSource = 'library' | 'bundled' | 'claude' | 'codex' | 'agy' | (string & {});

export interface SkillVariant {
  source: VariantSource;
  category: string;
  description: string;
  /** Bundled only: the catalog's drift state for the installed copy. */
  bundledState?: string;
  bundledVersion?: number;
  installedVersion?: number | null;
}

export type SyncState = 'in_sync' | 'drifted' | 'single' | 'unknown';

export interface SkillGroup {
  name: string;
  category: string;
  description: string;
  variants: SkillVariant[];
  sync: SyncState;
  /** Human notes on what differs ("Codex copy differs from Library"). */
  drift: string[];
  /** Sources whose body differs from the reference copy. */
  driftedSources: VariantSource[];
  /** The copy the others are compared against (Library when present). */
  reference: VariantSource;
}

export interface LibraryLike {
  name: string;
  description: string;
  body: string;
  category?: string;
}
export interface BundledLike {
  name: string;
  category: string;
  version: number;
  description: string;
  installed_version: number | null;
  state: string;
}
export interface ProviderLike {
  provider: string;
  name: string;
  category: string;
  description: string;
}

/** Display order of variant badges. */
export const SOURCE_ORDER: VariantSource[] = ['library', 'claude', 'codex', 'agy', 'bundled'];

export function sourceLabel(s: VariantSource): string {
  switch (s) {
    case 'library':
      return 'Library';
    case 'bundled':
      return 'Bundled';
    case 'claude':
      return 'Claude';
    case 'codex':
      return 'Codex';
    case 'agy':
      return 'Antigravity';
    default:
      return s ? s[0].toUpperCase() + s.slice(1) : '';
  }
}

export const bodyKey = (source: VariantSource, name: string): string => `${source}:${name}`;

/** Normalise a SKILL.md for comparison: line endings, trailing space, edges. */
export function normalizeBody(s: string): string {
  return s
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((l) => l.replace(/\s+$/, ''))
    .join('\n')
    .trim();
}

function orderOf(s: VariantSource): number {
  const i = SOURCE_ORDER.indexOf(s);
  return i < 0 ? SOURCE_ORDER.length : i;
}

/** A frontmatter block-scalar marker (`|`, `>`) leaks through as the
 *  description when a skill's `description:` is multi-line — it means "no
 *  one-line description", not the text "|". */
function cleanDescription(d: string | undefined): string {
  const t = (d ?? '').trim();
  return /^[|>][+-]?$/.test(t) ? '' : t;
}

function cleanCategory(c: string | undefined): string {
  const t = (c ?? '').trim();
  return !t || t === 'provider' ? 'uncategorized' : t;
}

/**
 * Group every copy of every skill by name.
 * `bodies` maps `bodyKey(source, name)` → SKILL.md text for the copies that
 * have been read (library bodies come with the list; provider bodies are
 * fetched lazily). Copies with no body yet make the group `unknown` unless a
 * difference is already known.
 */
export function groupSkills(
  library: LibraryLike[],
  bundled: BundledLike[],
  provider: ProviderLike[],
  bodies: Record<string, string | undefined> = {},
): SkillGroup[] {
  const byName = new Map<string, SkillVariant[]>();
  const add = (name: string, v: SkillVariant) => {
    const arr = byName.get(name) ?? [];
    if (!arr.some((x) => x.source === v.source)) arr.push(v);
    byName.set(name, arr);
  };
  const libBody = new Map<string, string>();
  for (const s of library) {
    add(s.name, { source: 'library', category: cleanCategory(s.category), description: cleanDescription(s.description) });
    libBody.set(s.name, s.body);
  }
  for (const b of bundled) {
    add(b.name, {
      source: 'bundled',
      category: cleanCategory(b.category),
      description: cleanDescription(b.description),
      bundledState: b.state,
      bundledVersion: b.version,
      installedVersion: b.installed_version,
    });
  }
  for (const p of provider) {
    add(p.name, { source: p.provider, category: cleanCategory(p.category), description: cleanDescription(p.description) });
  }

  const out: SkillGroup[] = [];
  for (const [name, raw] of byName) {
    const variants = [...raw].sort((a, b) => orderOf(a.source) - orderOf(b.source));
    const first = (pick: (v: SkillVariant) => string) =>
      variants.map(pick).find((x) => x && x !== 'uncategorized') ?? variants.map(pick).find(Boolean) ?? '';
    const category = first((v) => v.category) || 'uncategorized';
    const description = first((v) => v.description);

    const drift: string[] = [];
    const driftedSources: VariantSource[] = [];
    let unknown = false;

    const lib = variants.find((v) => v.source === 'library');
    const bun = variants.find((v) => v.source === 'bundled');
    // Bundled vs the installed library copy: the catalog's version state.
    if (lib && bun) {
      if (bun.bundledState === 'update_available') {
        drift.push(`Bundled v${bun.bundledVersion} is newer than the library copy (v${bun.installedVersion ?? '?'})`);
        driftedSources.push('bundled');
      } else if (bun.bundledState === 'ahead') {
        drift.push(`Library copy (v${bun.installedVersion ?? '?'}) is ahead of bundled v${bun.bundledVersion}`);
        driftedSources.push('bundled');
      }
    }

    // Body comparison across the editable/provider copies.
    const comparable = variants.filter((v) => v.source !== 'bundled');
    const reference = comparable[0]?.source ?? variants[0].source;
    const refBody =
      reference === 'library' ? libBody.get(name) : bodies[bodyKey(reference, name)];
    for (const v of comparable) {
      if (v.source === reference) continue;
      const b = v.source === 'library' ? libBody.get(name) : bodies[bodyKey(v.source, name)];
      if (b == null || refBody == null) {
        unknown = true;
        continue;
      }
      if (normalizeBody(b) !== normalizeBody(refBody)) {
        drift.push(`${sourceLabel(v.source)} copy differs from ${sourceLabel(reference)}`);
        driftedSources.push(v.source);
      }
    }

    // Bundled-but-not-installed is catalog, not a second place the skill lives.
    let sync: SyncState;
    if (drift.length > 0) sync = 'drifted';
    else if (variants.length === 1) sync = 'single';
    else if (comparable.length <= 1) sync = lib && bun ? 'in_sync' : 'single';
    else sync = unknown ? 'unknown' : 'in_sync';
    out.push({ name, category, description, variants, sync, drift, driftedSources, reference });
  }
  out.sort((a, b) => a.category.localeCompare(b.category) || a.name.localeCompare(b.name));
  return out;
}

export type SourceFilter = 'all' | VariantSource;

export interface GroupFilter {
  query?: string;
  category?: string | null;
  source?: SourceFilter;
  /** Only drifted groups. */
  driftedOnly?: boolean;
}

export function filterGroups(groups: SkillGroup[], f: GroupFilter): SkillGroup[] {
  const q = (f.query ?? '').trim().toLowerCase();
  return groups.filter(
    (g) =>
      (!q || g.name.toLowerCase().includes(q) || g.description.toLowerCase().includes(q)) &&
      (!f.category || g.category === f.category) &&
      (!f.source || f.source === 'all' || g.variants.some((v) => v.source === f.source)) &&
      (!f.driftedOnly || g.sync === 'drifted'),
  );
}

/** `[category, count]` pairs, most populated first, then by name. */
export function categoryCounts(groups: SkillGroup[]): [string, number][] {
  const m = new Map<string, number>();
  for (const g of groups) m.set(g.category, (m.get(g.category) ?? 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/** Which sources occur at all, in display order, with counts. */
export function sourceCounts(groups: SkillGroup[]): [VariantSource, number][] {
  const m = new Map<VariantSource, number>();
  for (const g of groups) for (const v of g.variants) m.set(v.source, (m.get(v.source) ?? 0) + 1);
  return [...m.entries()].sort((a, b) => orderOf(a[0]) - orderOf(b[0]));
}

/** Names whose provider copies need their bodies fetched to compare (a
 *  provider copy exists alongside at least one other comparable copy). */
export function namesNeedingBodies(groups: SkillGroup[], bodies: Record<string, string | undefined>): { source: VariantSource; name: string }[] {
  const out: { source: VariantSource; name: string }[] = [];
  for (const g of groups) {
    const comparable = g.variants.filter((v) => v.source !== 'bundled');
    if (comparable.length < 2) continue;
    for (const v of comparable) {
      if (v.source === 'library') continue;
      if (bodies[bodyKey(v.source, g.name)] == null) out.push({ source: v.source, name: g.name });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// SKILL.md frontmatter
// ---------------------------------------------------------------------------

export interface Frontmatter {
  /** Scalar or list values by key, in file order. */
  meta: [string, string | string[]][];
  /** The markdown after the closing `---`. */
  body: string;
}

function unquote(s: string): string {
  const t = s.trim();
  if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'"))) return t.slice(1, -1);
  return t;
}

/** Minimal YAML frontmatter reader: `key: value`, block scalars (`>`, `|`),
 *  `- item` lists and `[a, b]` flow lists. Anything it can't read is kept as
 *  raw text. No frontmatter → empty meta and the whole text as body. */
export function parseFrontmatter(src: string): Frontmatter {
  const text = (src ?? '').replace(/\r\n?/g, '\n');
  const m = /^---\n([\s\S]*?)\n---[ \t]*(?:\n|$)/.exec(text);
  if (!m) return { meta: [], body: text };
  const lines = m[1].split('\n');
  const meta: [string, string | string[]][] = [];
  for (let i = 0; i < lines.length; i++) {
    const kv = /^([A-Za-z0-9_.-]+)\s*:\s*(.*)$/.exec(lines[i]);
    if (!kv) continue;
    const key = kv[1];
    const rest = kv[2].trim();
    if (rest === '>' || rest === '|' || rest === '>-' || rest === '|-' || rest === '') {
      const block: string[] = [];
      const items: string[] = [];
      while (i + 1 < lines.length && (/^\s+/.test(lines[i + 1]) || lines[i + 1].trim() === '')) {
        i++;
        const l = lines[i];
        const li = /^\s*-\s+(.*)$/.exec(l);
        if (li && rest === '') items.push(unquote(li[1]));
        else block.push(l.trim());
      }
      if (items.length) meta.push([key, items]);
      else meta.push([key, rest.startsWith('|') ? block.join('\n').trim() : block.filter(Boolean).join(' ')]);
      continue;
    }
    if (rest.startsWith('[') && rest.endsWith(']')) {
      meta.push([key, rest.slice(1, -1).split(',').map(unquote).filter(Boolean)]);
      continue;
    }
    meta.push([key, unquote(rest)]);
  }
  return { meta, body: text.slice(m[0].length) };
}

/** A frontmatter value as a list (comma-separated scalars split). */
export function metaList(v: string | string[] | undefined): string[] {
  if (v == null) return [];
  if (Array.isArray(v)) return v;
  return v
    .split(/,(?![^(]*\))/)
    .map((s) => s.trim())
    .filter(Boolean);
}
