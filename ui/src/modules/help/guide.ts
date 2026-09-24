// Help → section guides: the pure half (no Svelte, no Vite, no DOM) so the unit
// tests can run it under plain `node --test`.
//
// Every guide is a markdown file in ./sections/<id>.md with a tiny YAML-ish
// frontmatter block (flat `key: value` lines only — no nesting, no lists):
//
//   ---
//   id: agents
//   title: Agents
//   group: Work            # Basics | Work | Automate | Build | Infrastructure | Insight | Plugins
//   route: agents          # router module to open; omitted for Basics
//   summary: One sentence, what it's for.
//   ---
//
// The body follows the shared outline (What it's for / Getting started /
// Everything it can do / Keyboard shortcuts / Tips and limits / Related). The
// "Keyboard shortcuts" table's first column holds the keys as code spans; those
// are indexed so a search for "⌘K" or "cmd shift d" finds the right guide.

export const GUIDE_GROUPS = ['Basics', 'Work', 'Automate', 'Build', 'Infrastructure', 'Insight', 'Plugins'] as const;
export type GuideGroup = (typeof GUIDE_GROUPS)[number];

/** The shell-wide topics, in reading order (they have no sidebar module). */
export const BASICS_ORDER = ['getting-started', 'command-bar', 'keyboard-shortcuts', 'desktop-app', 'phone-and-remote'];

/** The guide the page opens on (with the tour film above it). */
export const DEFAULT_GUIDE = 'getting-started';

export interface GuideSection {
  id: string;
  title: string;
  group: GuideGroup;
  /** Router module the "Open …" button goes to (absent for Basics). */
  route?: string;
  summary: string;
  /** Markdown without the frontmatter. */
  body: string;
  /** Every key combo from the Keyboard shortcuts table, as written (`⌘K`). */
  shortcuts: string[];
}

export interface Frontmatter {
  meta: Record<string, string>;
  body: string;
}

/**
 * Split `---` frontmatter from the body. Flat `key: value` lines; a trailing
 * ` # comment` is dropped, surrounding quotes are stripped, blank and comment
 * lines are ignored. A file without a (closed) frontmatter block returns empty
 * meta and the whole text as the body — never throws.
 */
export function parseFrontmatter(raw: string): Frontmatter {
  const text = (raw ?? '').replace(/^﻿/, '').replace(/\r\n?/g, '\n');
  const m = /^---[ \t]*\n([\s\S]*?)\n---[ \t]*(?:\n|$)/.exec(text);
  if (!m) return { meta: {}, body: text };
  const meta: Record<string, string> = {};
  for (const line of m[1].split('\n')) {
    if (/^\s*(#|$)/.test(line)) continue;
    const kv = /^\s*([A-Za-z0-9_-]+)\s*:\s?(.*)$/.exec(line);
    if (!kv) continue;
    let value = kv[2].replace(/\s+#.*$/, '').trim();
    if (value.length >= 2 && /^(['"]).*\1$/.test(value)) value = value.slice(1, -1);
    meta[kv[1].toLowerCase()] = value;
  }
  return { meta, body: text.slice(m[0].length).replace(/^\n+/, '') };
}

/** Case-insensitive group name → a known group (unknown → null). */
export function normalizeGroup(g: string | undefined): GuideGroup | null {
  const want = (g ?? '').trim().toLowerCase();
  if (want === 'infra') return 'Infrastructure';
  return GUIDE_GROUPS.find((x) => x.toLowerCase() === want) ?? null;
}

/** The id a file path implies: `./sections/agents.md` → `agents`. */
export function idFromPath(path: string): string {
  return (path.split('/').pop() ?? path).replace(/\.md$/i, '');
}

/** The body of one `## Heading` section (up to the next `## `), or ''. */
export function sectionBody(body: string, heading: string): string {
  const lines = body.split('\n');
  const want = heading.trim().toLowerCase();
  const start = lines.findIndex((l) => /^##\s+/.test(l) && l.replace(/^##\s+/, '').trim().toLowerCase() === want);
  if (start < 0) return '';
  const out: string[] = [];
  for (let i = start + 1; i < lines.length && !/^##\s+/.test(lines[i]); i++) out.push(lines[i]);
  return out.join('\n');
}

/** Key combos from the "Keyboard shortcuts" table: every code span in the first
 *  column of each body row (`| \`⌘K\` / \`⌘P\` | Open … |` → ⌘K, ⌘P). */
export function extractShortcuts(body: string): string[] {
  const table = sectionBody(body, 'Keyboard shortcuts');
  const keys: string[] = [];
  for (const line of table.split('\n')) {
    if (!/^\s*\|/.test(line) || /^\s*\|[\s:|-]+\|?\s*$/.test(line)) continue; // not a row / the --- rule
    const first = line.replace(/^\s*\|/, '').split('|')[0] ?? '';
    for (const c of first.matchAll(/`([^`]+)`/g)) keys.push(c[1].trim());
  }
  return [...new Set(keys)];
}

/**
 * One file → a guide, or null when it is unusable (no title). `id` falls back
 * to the file name; an unknown group falls back to `fallbackGroup(id)` (the
 * sidebar section the caller knows about) and then to Basics.
 */
export function parseSection(
  path: string,
  raw: string,
  fallbackGroup: (id: string) => GuideGroup | null = () => null,
): GuideSection | null {
  const { meta, body } = parseFrontmatter(raw);
  const id = (meta.id || idFromPath(path)).trim();
  const title = (meta.title ?? '').trim();
  if (!id || !title) return null;
  const group = normalizeGroup(meta.group) ?? fallbackGroup(id) ?? 'Basics';
  const route = (meta.route ?? '').trim() || undefined;
  return { id, title, group, route, summary: (meta.summary ?? '').trim(), body, shortcuts: extractShortcuts(body) };
}

/**
 * Display order: by group (GUIDE_GROUPS), then Basics in reading order and
 * modules in sidebar order (`moduleOrder`: sidebar ids), then by title.
 */
export function orderSections(sections: GuideSection[], moduleOrder: string[] = []): GuideSection[] {
  const rank = (s: GuideSection): number => {
    const list = s.group === 'Basics' ? BASICS_ORDER : moduleOrder;
    const i = list.indexOf(s.id);
    return i < 0 ? Number.MAX_SAFE_INTEGER : i;
  };
  return [...sections].sort(
    (a, b) =>
      GUIDE_GROUPS.indexOf(a.group) - GUIDE_GROUPS.indexOf(b.group) ||
      rank(a) - rank(b) ||
      a.title.localeCompare(b.title),
  );
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

const MODIFIERS = '⌘⌥⌃⇧';
const MOD_WORDS: [RegExp, string][] = [
  [/\b(cmd|command|meta|super)\b/g, '⌘'],
  [/\b(opt|option|alt)\b/g, '⌥'],
  [/\b(ctrl|control|ctl)\b/g, '⌃'],
  [/\bshift\b/g, '⇧'],
];

/** Canonical form for matching key combos: word modifiers → symbols, lower
 *  case, separators dropped (`Cmd + Shift + D` → `⌘⇧d`; `⌘⇧D` → `⌘⇧d`). */
export function normalizeKeys(s: string): string {
  let t = (s ?? '').toLowerCase();
  for (const [re, sym] of MOD_WORDS) t = t.replace(re, sym);
  return t.replace(/[\s+]/g, '');
}

/** True when a query reads like a key combo (has a modifier after
 *  normalisation) — only then do shortcut columns take part in matching, so a
 *  plain "k" doesn't hit every guide with a ⌘K in it. */
export function looksLikeKeys(q: string): boolean {
  const n = normalizeKeys(q);
  return [...n].some((c) => MODIFIERS.includes(c));
}

/** A chord → its chips: `⌘⇧D` → ['⌘','⇧','D']; `⌘Enter` → ['⌘','Enter'];
 *  anything without leading modifiers stays one chip (`Esc`, `?`). */
export function splitChord(s: string): string[] {
  const t = s.trim();
  const out: string[] = [];
  let i = 0;
  const chars = [...t];
  while (i < chars.length && MODIFIERS.includes(chars[i])) out.push(chars[i++]);
  const rest = chars.slice(i).join('').trim();
  if (rest) out.push(rest);
  return out.length ? out : [t];
}

/** Code-span text that should render as key chips in prose: starts with a
 *  modifier symbol and is short (`⌘K`, `⌃1`, `⌥Space`, `⌘⇧←`). */
export function isKeyCombo(s: string): boolean {
  const t = s.trim();
  return t.length > 1 && t.length <= 16 && MODIFIERS.includes([...t][0]) && !/\s{2,}/.test(t);
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

export type MatchKind = 'title' | 'shortcut' | 'summary' | 'body';

export interface SearchHit {
  section: GuideSection;
  score: number;
  /** Why it matched — shown under the row (a key chip or a text snippet). */
  match?: { kind: MatchKind; text: string };
}

/** Characters of `q` appear in order in `s` (fuzzy title match). */
function subsequence(q: string, s: string): boolean {
  let i = 0;
  for (const c of s) if (c === q[i]) i++;
  return i === q.length;
}

/** Markdown line → plain text for a snippet. */
function plain(line: string): string {
  return line
    .replace(/^\s*(#+|[-*]|\d+\.|\|)\s*/, '')
    .replace(/\|/g, ' · ')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/\*\*([^*]+)\*\*/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
}

/** A short snippet of the body line holding `q` (centred on the hit). */
function snippet(body: string, q: string): string | undefined {
  const line = body.split('\n').find((l) => l.toLowerCase().includes(q));
  if (!line) return undefined;
  const text = plain(line);
  const at = text.toLowerCase().indexOf(q);
  if (text.length <= 90 || at < 0) return text.slice(0, 90) + (text.length > 90 ? '…' : '');
  const start = Math.max(0, at - 30);
  return (start > 0 ? '…' : '') + text.slice(start, start + 90).trim() + (start + 90 < text.length ? '…' : '');
}

/** Score one guide for a lower-cased, trimmed query (0 = no match). */
function scoreOne(s: GuideSection, q: string): SearchHit | null {
  const title = s.title.toLowerCase();
  const id = s.id.replace(/-/g, ' ');
  // Title / id.
  let best: SearchHit | null = null;
  const take = (score: number, match?: SearchHit['match']) => {
    if (!best || score > best.score) best = { section: s, score, match };
  };
  if (title === q || id === q) take(100);
  else if (title.startsWith(q) || id.startsWith(q)) take(85);
  else if (title.split(/[\s/·-]+/).some((w) => w.startsWith(q))) take(75);
  else if (title.includes(q) || id.includes(q)) take(65);
  // Shortcuts (only for key-ish queries).
  if (looksLikeKeys(q)) {
    const nq = normalizeKeys(q);
    const exact = s.shortcuts.find((k) => normalizeKeys(k) === nq);
    const part = exact ?? s.shortcuts.find((k) => normalizeKeys(k).includes(nq));
    if (exact) take(90, { kind: 'shortcut', text: exact });
    else if (part) take(55, { kind: 'shortcut', text: part });
  }
  if (s.summary.toLowerCase().includes(q)) take(45, { kind: 'summary', text: s.summary });
  const body = s.body.toLowerCase();
  if (body.includes(q)) {
    // More mentions rank a little higher (capped), never above a summary hit.
    const count = Math.min(body.split(q).length - 1, 10);
    take(20 + count, { kind: 'body', text: snippet(s.body, q) ?? s.summary });
  }
  // Every word somewhere (multi-word queries: "resume session").
  const words = q.split(/\s+/).filter(Boolean);
  if (words.length > 1) {
    const hay = `${title} ${s.summary.toLowerCase()} ${body}`;
    if (words.every((w) => hay.includes(w))) take(12, { kind: 'body', text: snippet(s.body, words[0]) ?? s.summary });
  }
  // Fuzzy title (typos of omission: "mssn ctrl").
  if (q.length >= 3 && subsequence(q.replace(/\s+/g, ''), title.replace(/\s+/g, ''))) take(30);
  return best;
}

/**
 * Rank guides for `query`. Empty query → every guide in its given order with
 * score 0. Ties keep the given (display) order, so results read like the rail.
 */
export function searchSections(sections: GuideSection[], query: string): SearchHit[] {
  const q = (query ?? '').trim().toLowerCase();
  if (!q) return sections.map((section) => ({ section, score: 0 }));
  const hits: { hit: SearchHit; i: number }[] = [];
  sections.forEach((s, i) => {
    const hit = scoreOne(s, q);
    if (hit) hits.push({ hit, i });
  });
  return hits.sort((a, b) => b.hit.score - a.hit.score || a.i - b.i).map((x) => x.hit);
}

// ---------------------------------------------------------------------------
// Tour film manifest
// ---------------------------------------------------------------------------

export interface FilmChapter {
  id: string;
  /** Guide id this part of the film covers (may name a guide that doesn't exist). */
  section: string;
  title: string;
  start: number;
  duration: number;
}

export interface FilmManifest {
  file: string;
  poster?: string;
  captions?: string;
  duration: number;
  chapters: FilmChapter[];
}

/** Validate the (possibly missing or hand-edited) film.json. Returns null when
 *  there is no playable file; drops malformed chapters and sorts by start. */
export function parseFilmManifest(raw: unknown): FilmManifest | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const file = typeof r.file === 'string' ? r.file.trim() : '';
  if (!file) return null;
  const str = (v: unknown) => (typeof v === 'string' && v.trim() ? v.trim() : undefined);
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : 0);
  const chapters: FilmChapter[] = (Array.isArray(r.chapters) ? r.chapters : [])
    .filter((c): c is Record<string, unknown> => !!c && typeof c === 'object')
    .map((c) => ({
      id: str(c.id) ?? '',
      section: str(c.section) ?? '',
      title: str(c.title) ?? '',
      start: num(c.start),
      duration: num(c.duration),
    }))
    .filter((c) => c.id && c.title)
    .sort((a, b) => a.start - b.start);
  return { file, poster: str(r.poster), captions: str(r.captions), duration: num(r.duration), chapters };
}

/** The chapter playing at `t` seconds (null before the first / past the last). */
export function chapterAt(chapters: FilmChapter[], t: number): FilmChapter | null {
  for (const c of chapters) if (t >= c.start && t < c.start + Math.max(c.duration, 0.001)) return c;
  return null;
}

/** `m:ss` for a time in seconds. */
export function timeLabel(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds || 0));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
