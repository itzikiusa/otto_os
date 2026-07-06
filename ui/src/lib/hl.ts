// highlight.js helpers for the diff viewer: language from file extension,
// per-line highlight with escaped fallback. hljs itself is loaded lazily so
// it stays out of the main bundle; lines render escaped until it arrives.

type Hljs = typeof import('highlight.js/lib/common').default;

let hljs: Hljs | null = null;
let loading: Promise<void> | null = null;

/** Kick off the lazy hljs load; resolves when highlighting is available. */
export function ensureHljs(): Promise<void> {
  if (hljs) return Promise.resolve();
  loading ??= import('highlight.js/lib/common').then((m) => {
    hljs = m.default;
  });
  return loading;
}

const extMap: Record<string, string> = {
  rs: 'rust',
  ts: 'typescript',
  tsx: 'typescript',
  js: 'javascript',
  jsx: 'javascript',
  svelte: 'xml',
  html: 'xml',
  vue: 'xml',
  css: 'css',
  scss: 'scss',
  json: 'json',
  md: 'markdown',
  py: 'python',
  go: 'go',
  java: 'java',
  kt: 'kotlin',
  swift: 'swift',
  c: 'c',
  h: 'c',
  cpp: 'cpp',
  hpp: 'cpp',
  cs: 'csharp',
  rb: 'ruby',
  php: 'php',
  sh: 'bash',
  bash: 'bash',
  zsh: 'bash',
  yml: 'yaml',
  yaml: 'yaml',
  toml: 'ini',
  sql: 'sql',
  xml: 'xml',
};

export function langFromPath(path: string): string | null {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  const lang = extMap[ext];
  if (!lang) return null;
  if (hljs && !hljs.getLanguage(lang)) return null;
  return lang;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Memoized highlight results. Diff lines are re-rendered wholesale on every
// reactive flush (view-mode toggle, hljs arrival, comment updates), and large
// PRs repeat many identical lines (imports, braces, blank context) — without a
// cache each flush re-runs the tokenizer over every visible line and stalls
// the main thread. Bounded so a huge PR can't hold the whole diff in memory.
const hlCache = new Map<string, string>();
const HL_CACHE_MAX = 20_000;

/** Highlight a single line; returns trusted HTML (escaped on fallback). */
export function highlightLine(content: string, lang: string | null): string {
  if (!lang || !hljs || !hljs.getLanguage(lang)) return escapeHtml(content);
  const key = `${lang} ${content}`;
  const hit = hlCache.get(key);
  if (hit !== undefined) return hit;
  let out: string;
  try {
    out = hljs.highlight(content, { language: lang, ignoreIllegals: true }).value;
  } catch {
    out = escapeHtml(content);
  }
  if (hlCache.size >= HL_CACHE_MAX) hlCache.clear();
  hlCache.set(key, out);
  return out;
}
