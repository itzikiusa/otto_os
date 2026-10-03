// Workbench language registry + auto-detection.
//
// A doc's `language` is either an explicit id from WB_LANGUAGES or `auto`
// (resolved here from the name's extension first, then a content sniff). The
// `cmExt` column maps each id onto a key cm-langs.ts / CodeEditor understands
// ('' = plain text). Pure: no DOM, no Svelte — unit-tested in
// ui/unit/workbench-detect.test.ts.

export interface WbLanguage {
  id: string;
  label: string;
  /** Key for CodeEditor / cm-langs ('' = plain text). */
  cmExt: string;
  /** The preview pane has a renderer for it. */
  previewable?: boolean;
}

export const WB_LANGUAGES: WbLanguage[] = [
  { id: 'auto', label: 'Auto-detect', cmExt: '' },
  { id: 'txt', label: 'Plain text', cmExt: '' },
  { id: 'md', label: 'Markdown', cmExt: 'md', previewable: true },
  { id: 'json', label: 'JSON', cmExt: 'json', previewable: true },
  { id: 'yaml', label: 'YAML', cmExt: '' },
  { id: 'toml', label: 'TOML', cmExt: '' },
  { id: 'csv', label: 'CSV', cmExt: '', previewable: true },
  { id: 'tsv', label: 'TSV', cmExt: '', previewable: true },
  { id: 'sql', label: 'SQL', cmExt: 'sql' },
  { id: 'html', label: 'HTML', cmExt: 'html', previewable: true },
  { id: 'xml', label: 'XML', cmExt: 'xml' },
  { id: 'svg', label: 'SVG', cmExt: 'xml', previewable: true },
  { id: 'css', label: 'CSS', cmExt: 'css' },
  { id: 'js', label: 'JavaScript', cmExt: 'js' },
  { id: 'ts', label: 'TypeScript', cmExt: 'ts' },
  { id: 'py', label: 'Python', cmExt: 'py' },
  { id: 'sh', label: 'Shell', cmExt: '' },
  { id: 'go', label: 'Go', cmExt: 'go' },
  { id: 'rs', label: 'Rust', cmExt: 'rs' },
  { id: 'java', label: 'Java', cmExt: 'java' },
  { id: 'mermaid', label: 'Mermaid', cmExt: '', previewable: true },
  { id: 'd2', label: 'D2', cmExt: '', previewable: true },
  { id: 'http', label: 'HTTP / curl', cmExt: '' },
  { id: 'diff', label: 'Diff / patch', cmExt: '' },
  { id: 'image', label: 'Image', cmExt: '', previewable: true },
];

const BY_ID = new Map(WB_LANGUAGES.map((l) => [l.id, l]));

/** File extension → language id. */
const EXT: Record<string, string> = {
  txt: 'txt', text: 'txt', log: 'txt',
  md: 'md', markdown: 'md', mdx: 'md',
  json: 'json', jsonc: 'json', json5: 'json', geojson: 'json', ipynb: 'json',
  yaml: 'yaml', yml: 'yaml',
  toml: 'toml',
  csv: 'csv', tsv: 'tsv', tab: 'tsv',
  sql: 'sql', psql: 'sql', mysql: 'sql',
  html: 'html', htm: 'html', xhtml: 'html',
  xml: 'xml', xsd: 'xml', xsl: 'xml', plist: 'xml', pom: 'xml',
  svg: 'svg',
  css: 'css', scss: 'css', less: 'css',
  js: 'js', mjs: 'js', cjs: 'js', jsx: 'js',
  ts: 'ts', tsx: 'ts', mts: 'ts', cts: 'ts',
  py: 'py',
  sh: 'sh', bash: 'sh', zsh: 'sh',
  go: 'go',
  rs: 'rs',
  java: 'java',
  mmd: 'mermaid', mermaid: 'mermaid',
  d2: 'd2',
  http: 'http', curl: 'http', rest: 'http',
  diff: 'diff', patch: 'diff',
  png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image',
};

const MERMAID_START =
  /^(graph\s+(TD|TB|BT|RL|LR)\b|flowchart\b|sequenceDiagram\b|classDiagram\b|stateDiagram(-v2)?\b|erDiagram\b|gantt\b|pie\b|journey\b|gitGraph\b|mindmap\b|timeline\b|quadrantChart\b|requirementDiagram\b|C4Context\b|sankey-beta\b|xychart-beta\b|block-beta\b)/;

/** First non-blank, non-comment line (Mermaid `%%` comments skipped). */
function firstLine(content: string): string {
  for (const raw of content.split('\n')) {
    const l = raw.trim();
    if (l && !l.startsWith('%%')) return l;
  }
  return '';
}

function looksLikeCsv(lines: string[], sep: string): boolean {
  const rows = lines.filter((l) => l.trim()).slice(0, 20);
  if (rows.length < 2) return false;
  const count = (l: string) => l.split(sep).length - 1;
  const c = count(rows[0]);
  return c >= 1 && rows.every((r) => count(r) === c);
}

/** Detect a language id from a file name (extension wins) and its content. */
/** Skips a leading `<?xml …?>` and any `<!-- … -->` comments with an
 *  indexOf scan. A regex with a repeated lazy comment group backtracks
 *  exponentially on input like `<!--` + `--><!--`×n (CodeQL js/redos). */
function skipXmlPrologue(t: string): string {
  let i = 0;
  if (t.startsWith('<?xml')) {
    const end = t.indexOf('>', i);
    if (end < 0) return '';
    i = end + 1;
  }
  for (;;) {
    while (i < t.length && /\s/.test(t[i])) i++;
    if (!t.startsWith('<!--', i)) return t.slice(i);
    const end = t.indexOf('-->', i + 4);
    if (end < 0) return '';
    i = end + 3;
  }
}

export function detectLanguage(name: string, content: string): string {
  const m = /\.([A-Za-z0-9]+)$/.exec(name.trim());
  if (m) {
    const byExt = EXT[m[1].toLowerCase()];
    if (byExt) return byExt;
  }
  const text = content.replace(/^﻿/, '');
  const t = text.trim();
  if (!t) return 'txt';
  const first = firstLine(text);

  if (first.startsWith('#!')) {
    if (/python/.test(first)) return 'py';
    if (/node|deno|bun/.test(first)) return 'js';
    return 'sh';
  }
  if (/^curl\s/i.test(t) || /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+(https?:\/\/|\/)\S*/.test(t)) return 'http';
  if ((t.startsWith('{') && t.endsWith('}')) || (t.startsWith('[') && t.endsWith(']'))) {
    try {
      JSON.parse(t);
      return 'json';
    } catch {
      /* not JSON — keep sniffing */
    }
  }
  if (t.startsWith('<')) {
    if (/^<svg[\s>]/i.test(skipXmlPrologue(t))) return 'svg';
    if (/^<!doctype\s+html/i.test(t) || /<(html|head|body|div|p|span|table|ul|h[1-6]|a|script|style)[\s>]/i.test(t)) {
      return 'html';
    }
    return 'xml';
  }
  if (/^(diff --git |--- \S|Index: )/.test(t) && /^@@ /m.test(t)) return 'diff';
  if (MERMAID_START.test(first)) return 'mermaid';
  if (/^(SELECT|INSERT|UPDATE|DELETE|WITH|CREATE|ALTER|DROP|TRUNCATE|EXPLAIN|SHOW|DESCRIBE|REPLACE|MERGE|GRANT|USE)\b/i.test(first)) {
    return 'sql';
  }
  const lines = text.split('\n');
  // D2: `a -> b` edges / `x: { … }` / `shape:` with no Mermaid header.
  const d2Edges = lines.filter((l) => /^\s*[\w"'.\- ]+\s*(<->|->|<-|--)\s*[\w"'.\- ]+(:.*)?$/.test(l)).length;
  if (d2Edges >= 1 && (/\bshape\s*:/.test(t) || /:\s*\{\s*$/m.test(t) || d2Edges >= 2) && !/[;=]\s*$/m.test(t)) {
    return 'd2';
  }
  if (/^\s*(#{1,6}\s|```|[-*+]\s+\S|\d+\.\s+\S|>\s)/m.test(t) && /^#{1,6}\s|```|\[[^\]]+\]\([^)]+\)/m.test(t)) {
    return 'md';
  }
  if (looksLikeCsv(lines, '\t')) return 'tsv';
  if (looksLikeCsv(lines, ',')) return 'csv';
  if (/^\s*\[[\w.\-"]+\]\s*$/m.test(t) && /^\s*[\w.\-"]+\s*=\s*\S/m.test(t)) return 'toml';
  const yamlish = lines.filter((l) => /^\s*(- )?[\w"'.\-]+:(\s|$)/.test(l) || /^\s*- \S/.test(l) || /^---\s*$/.test(l));
  const nonBlank = lines.filter((l) => l.trim() && !l.trim().startsWith('#'));
  if (yamlish.length >= 2 && yamlish.length >= nonBlank.length * 0.6) return 'yaml';
  if (/^\s*#{1,6}\s+\S/m.test(t)) return 'md';
  return 'txt';
}

/** `auto` / '' / unknown → detected; anything else is the explicit override. */
export function effectiveLanguage(language: string, name: string, content: string): string {
  if (!language || language === 'auto' || !BY_ID.has(language)) return detectLanguage(name, content);
  return language;
}

/** CodeEditor (cm-langs) key for a language id; '' = plain text. */
export function cmExtFor(lang: string): string {
  return BY_ID.get(lang)?.cmExt ?? '';
}

const EXT_FOR: Record<string, string> = {
  auto: 'txt', txt: 'txt', md: 'md', json: 'json', yaml: 'yaml', toml: 'toml', csv: 'csv', tsv: 'tsv',
  sql: 'sql', html: 'html', xml: 'xml', svg: 'svg', css: 'css', js: 'js', ts: 'ts', py: 'py', sh: 'sh',
  go: 'go', rs: 'rs', java: 'java', mermaid: 'mmd', d2: 'd2', http: 'http', diff: 'diff', image: 'png',
};

/** Suggested file extension (no dot) for new files / downloads. */
export function extensionFor(lang: string): string {
  return EXT_FOR[lang] ?? 'txt';
}
