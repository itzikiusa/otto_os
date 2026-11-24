// What a configuration file defines, read from its text (pure; the IDE's Structure tab for
// .properties, YAML, TOML and INI-style files — no language server, no request): keys,
// dotted .properties keys grouped by their first segment, YAML's top two levels (by
// document when a file holds several), TOML tables and INI sections with their keys. YAML
// parse errors come from the `yaml` package (loaded only for a YAML file) as Problems.
import type { IdeSymbolNode } from '../api/types';

export type ConfigOutlineKind = 'properties' | 'yaml' | 'toml' | 'conf';

const node = (name: string, kind: string, line: number, endLine = line, children?: IdeSymbolNode[]): IdeSymbolNode =>
  ({ name, kind, detail: null, line, end_line: endLine, ...(children?.length ? { children } : {}) });

/** .properties keys in file order; dotted keys that share a first segment are grouped. */
export function propertiesSymbols(text: string): IdeSymbolNode[] {
  const keys: { name: string; line: number }[] = [];
  const lines = text.split(/\r?\n/);
  let cont = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const was = cont;
    let n = 0;
    for (let j = raw.length - 1; j >= 0 && raw[j] === '\\'; j--) n++;
    cont = n % 2 === 1;
    if (was) continue;
    const t = raw.trimStart();
    if (!t || t[0] === '#' || t[0] === '!') { cont = false; continue; }
    const m = /^(?:\\.|[^\s=:\\])+/.exec(t);
    if (m) keys.push({ name: m[0].replace(/\\(.)/g, '$1'), line: i + 1 });
  }
  const byPrefix = new Map<string, number>();
  for (const k of keys) {
    const dot = k.name.indexOf('.');
    if (dot > 0) byPrefix.set(k.name.slice(0, dot), (byPrefix.get(k.name.slice(0, dot)) ?? 0) + 1);
  }
  const out: IdeSymbolNode[] = [];
  const groups = new Map<string, IdeSymbolNode>();
  for (const k of keys) {
    const dot = k.name.indexOf('.');
    const prefix = dot > 0 ? k.name.slice(0, dot) : '';
    if (!prefix || (byPrefix.get(prefix) ?? 0) < 2) { out.push(node(k.name, 'key', k.line)); continue; }
    let g = groups.get(prefix);
    if (!g) {
      g = node(prefix, 'section', k.line, k.line, []);
      g.children = [];
      groups.set(prefix, g);
      out.push(g);
    }
    g.children!.push(node(k.name, 'key', k.line));
    g.end_line = Math.max(g.end_line, k.line);
  }
  return out;
}

const YAML_KEY = /^(\s*)(?:-\s+)?("(?:[^"\\]|\\.)*"|'[^']*'|[^\s#'"{}[\],&*!|>%@-][^#]*?|-[^\s#][^#]*?)\s*:(?:\s|$)/;

/** YAML: the top two levels of keys, by document when there are several. */
export function yamlSymbols(text: string): IdeSymbolNode[] {
  const docs: { line: number; end: number; keys: IdeSymbolNode[] }[] = [{ line: 1, end: 1, keys: [] }];
  const lines = text.split(/\r?\n/);
  let block = -1; // a block scalar's parent indentation
  let top: IdeSymbolNode | null = null;
  let topIndent = -1, childIndent = -1;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const n = i + 1;
    if (!raw.trim()) continue;
    const indent = raw.length - raw.trimStart().length;
    if (block >= 0) { if (indent > block) continue; block = -1; }
    if (/^---(?:\s|$)/.test(raw)) {
      if (docs[docs.length - 1].keys.length || docs.length > 1 || i > 0) docs.push({ line: n, end: n, keys: [] });
      top = null; topIndent = -1; childIndent = -1;
      continue;
    }
    const t = raw.trimStart();
    if (t[0] === '#' || /^\.\.\.(?:\s|$)/.test(raw)) continue;
    const doc = docs[docs.length - 1];
    doc.end = n;
    if (/(?:^|[\s:])[|>][-+]?\d*\s*(?:#.*)?$/.test(raw)) block = indent;
    const m = YAML_KEY.exec(raw);
    if (!m) continue;
    const keyIndent = m[1].length + (/^-\s+/.exec(raw.slice(m[1].length))?.[0].length ?? 0);
    const name = m[2].replace(/^(["'])(.*)\1$/, '$2');
    if (topIndent < 0 || keyIndent <= topIndent) {
      topIndent = keyIndent;
      childIndent = -1;
      top = node(name, 'key', n);
      doc.keys.push(top);
    } else if (top && (childIndent < 0 || keyIndent === childIndent)) {
      childIndent = keyIndent;
      (top.children ??= []).push(node(name, 'key', n));
    }
    if (top) top.end_line = n;
  }
  const real = docs.filter((d) => d.keys.length || d.line > 1);
  if (real.length <= 1) return real[0]?.keys ?? docs[0].keys;
  return real.map((d, i) => node('Document ' + (i + 1), 'document', d.line, d.end, d.keys));
}

/** TOML tables ([a.b], [[arr]]) with their keys; keys before the first table at the top. */
export function tomlSymbols(text: string): IdeSymbolNode[] {
  return sectionSymbols(text, /^\s*(\[\[?)\s*([^\]#]+?)\s*\]\]?\s*(?:#.*)?$/, /^\s*((?:[A-Za-z0-9_-]+|"[^"]*"|'[^']*')(?:\s*\.\s*(?:[A-Za-z0-9_-]+|"[^"]*"|'[^']*'))*)\s*=/, '#', true);
}

/** INI / conf: [sections] with their keys. */
export function confSymbols(text: string): IdeSymbolNode[] {
  return sectionSymbols(text, /^\s*(\[)\s*([^\]]+?)\s*\]\s*(?:[#;].*)?$/, /^\s*(?:export\s+)?([^\s=:#;[]+?)\s*[=:]/, '#;');
}

function sectionSymbols(text: string, header: RegExp, key: RegExp, comments: string, multiline = false): IdeSymbolNode[] {
  const out: IdeSymbolNode[] = [];
  let section: IdeSymbolNode | null = null;
  let ml = ''; // inside a TOML multi-line string
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i], n = i + 1;
    if (ml) { if (raw.includes(ml)) ml = ''; continue; }
    const t = raw.trim();
    if (!t || comments.includes(t[0])) continue;
    const h = header.exec(raw);
    if (h) { section = node(h[2], 'section', n, n, []); section.children = []; out.push(section); continue; }
    const k = key.exec(raw);
    if (k) {
      (section ? section.children! : out).push(node(k[1].replace(/\s*\.\s*/g, '.'), 'key', n));
      if (section) section.end_line = n;
      const open = multiline && /^\s*("""|''')/.exec(raw.slice(k[0].length)); // a TOML value that opens one
      if (open && raw.split(open[1]).length === 2) ml = open[1];
    } else if (section) section.end_line = n;
  }
  for (const s of out) if (s.children && !s.children.length) delete s.children;
  return out;
}

/** YAML syntax errors as Problems entries (the yaml package, loaded on first use). */
export async function yamlProblems(path: string, text: string): Promise<{ path: string; line: number; col: number; severity: 'error'; message: string; source: string }[]> {
  // Yarn v1 lockfiles are not YAML; past 128 KB the parse blocks the UI thread for 70 ms+ on every pause.
  if (text.length > 128_000 || /(?:^|\/)yarn\.lock$/.test(path)) return [];
  const { parseAllDocuments } = await import('yaml');
  const out: { path: string; line: number; col: number; severity: 'error'; message: string; source: string }[] = [];
  const docs = parseAllDocuments(text, { prettyErrors: true });
  for (const doc of Array.isArray(docs) ? docs : [docs]) {
    for (const e of doc.errors) {
      const pos = e.linePos?.[0];
      out.push({ path, line: pos?.line ?? 1, col: pos?.col ?? 1, severity: 'error', message: e.message.split('\n')[0].replace(/:$/, ''), source: 'yaml' });
    }
  }
  return out;
}

export function configSymbols(kind: ConfigOutlineKind, text: string): IdeSymbolNode[] {
  if (kind === 'properties') return propertiesSymbols(text);
  if (kind === 'yaml') return yamlSymbols(text);
  if (kind === 'toml') return tomlSymbols(text);
  return confSymbols(text);
}
