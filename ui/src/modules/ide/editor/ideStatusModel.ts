// What the editor chrome shows for a file (pure, unit-tested in node):
// - which editor actions apply to a file kind (no "previous function" on a CSV or a README),
// - the IDE status strip's items: problems, language, line ending, language-server state.

import type { IdeLspLinkDetail } from '../../../lib/components/cm-ide-lsp-reconnect';

export type IdeFileKind = 'code' | 'markup' | 'data' | 'text';

const CODE_EXT = new Set([
  'go', 'rs', 'java', 'kt', 'kts', 'scala', 'groovy', 'ts', 'tsx', 'mts', 'cts', 'js', 'jsx', 'mjs', 'cjs',
  'py', 'pyi', 'rb', 'php', 'c', 'h', 'cc', 'cpp', 'cxx', 'hpp', 'cs', 'swift', 'dart', 'lua', 'sh', 'bash',
  'zsh', 'svelte', 'vue', 'css', 'scss', 'less', 'sql', 'proto',
]);
/** Code without functions to step through. */
const FUNCTIONLESS_EXT = new Set(['css', 'scss', 'less', 'sql', 'proto']);
const MARKUP_EXT = new Set(['md', 'markdown', 'mdx', 'rst', 'adoc', 'txt', 'html', 'htm', 'svg']);
const DATA_EXT = new Set(['csv', 'tsv', 'json', 'jsonc', 'jsonl', 'ndjson', 'yaml', 'yml', 'toml', 'xml', 'ini', 'env', 'lock', 'properties']);
const FORMATTABLE_DATA_EXT = new Set(['json', 'jsonc', 'yaml', 'yml', 'html', 'htm']);
const CODE_LANGS = new Set(['go', 'rust', 'java', 'kotlin', 'typescript', 'javascript', 'python', 'ruby', 'php', 'c', 'cpp', 'csharp', 'swift', 'shell', 'bash']);

function extOf(path: string): string {
  const base = path.split('?')[0].split('/').pop() ?? '';
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
}

export function ideFileKind(path: string, language?: string | null): IdeFileKind {
  const ext = extOf(path);
  if (CODE_EXT.has(ext)) return 'code';
  if (MARKUP_EXT.has(ext)) return 'markup';
  if (DATA_EXT.has(ext)) return 'data';
  if (/^(Dockerfile|Makefile|Jenkinsfile)$/i.test(path.split('/').pop() ?? '')) return 'code';
  return language && CODE_LANGS.has(language.toLowerCase()) ? 'code' : 'text';
}

export interface IdeEditorActionSet {
  structure: boolean;
  jumpFunction: boolean;
  quickFix: boolean;
  reformat: boolean;
  optimizeImports: boolean;
}

/** The editor actions that mean something for this file. */
export function ideEditorActionsFor(path: string, language: string | null | undefined, hasSymbols: boolean): IdeEditorActionSet {
  const kind = ideFileKind(path, language);
  const ext = extOf(path);
  const fn = kind === 'code' && !FUNCTIONLESS_EXT.has(ext);
  return {
    structure: hasSymbols || kind === 'code',
    jumpFunction: fn,
    quickFix: kind === 'code',
    reformat: kind === 'code' || FORMATTABLE_DATA_EXT.has(ext),
    optimizeImports: fn,
  };
}

const LANG_LABELS: Record<string, string> = {
  go: 'Go', rust: 'Rust', java: 'Java', kotlin: 'Kotlin', typescript: 'TypeScript', javascript: 'JavaScript',
  python: 'Python', markdown: 'Markdown', json: 'JSON', jsonl: 'JSON Lines', csv: 'CSV', tsv: 'TSV', yaml: 'YAML',
  toml: 'TOML', xml: 'XML', html: 'HTML', css: 'CSS', scss: 'SCSS', sql: 'SQL', shell: 'Shell', bash: 'Shell',
  dockerfile: 'Dockerfile', svelte: 'Svelte', vue: 'Vue', c: 'C', cpp: 'C++', csharp: 'C#', ruby: 'Ruby', php: 'PHP',
};
const EXT_LANG: Record<string, string> = {
  rs: 'rust', ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript', mjs: 'javascript',
  cjs: 'javascript', py: 'python', md: 'markdown', yml: 'yaml', sh: 'shell', kt: 'kotlin', rb: 'ruby',
};

/** Display name of the file's language ("Plain text" when nothing is known). */
export function ideLanguageLabel(path: string, language?: string | null): string {
  const id = (language ?? '').toLowerCase();
  if (id && id !== 'plaintext' && id !== 'text') return LANG_LABELS[id] ?? id.charAt(0).toUpperCase() + id.slice(1);
  const ext = extOf(path);
  const viaExt = EXT_LANG[ext] ?? ext;
  return LANG_LABELS[viaExt] ?? 'Plain text';
}

/** Settings section for a language, when Settings has one. */
export function ideLanguageSettingsSection(path: string, language?: string | null): string | null {
  const lang = (language ?? EXT_LANG[extOf(path)] ?? extOf(path)).toLowerCase();
  if (lang === 'go') return 'lang.go';
  if (lang === 'java') return 'lang.java';
  if (lang === 'typescript' || lang === 'javascript') return 'lang.node';
  if (lang === 'python') return 'lang.python';
  if (lang === 'rust') return 'lang.rust';
  if (lang === 'csv' || lang === 'tsv') return 'lang.csv';
  return null;
}

/** "LF" / "CRLF" / "Mixed" from what the store read off disk; null when it does not know. */
export function ideEolLabel(tab: { eol?: '\r\n' | '\n'; diskEols?: string[] }): string | null {
  if (tab.diskEols?.length) return 'Mixed';
  if (tab.eol === '\r\n') return 'CRLF';
  if (tab.eol === '\n') return 'LF';
  return null;
}

export function ideProblemCounts(problems: readonly { severity: string }[]): { errors: number; warnings: number } {
  let errors = 0, warnings = 0;
  for (const p of problems) {
    if (p.severity === 'error') errors++;
    else if (p.severity === 'warning') warnings++;
  }
  return { errors, warnings };
}

// ── Language server ─────────────────────────────────────────────────────────

const SERVER_NAMES: Record<string, string> = {
  go: 'gopls', rust: 'rust-analyzer', java: 'jdtls', typescript: 'tsserver', javascript: 'tsserver', python: 'pyright',
};

/** The language-server id (`go`, `typescript`, …) for a file, or null when Otto runs none for it. */
export function ideLspLangFor(path: string, language?: string | null): string | null {
  const raw = (language || EXT_LANG[extOf(path)] || extOf(path)).toLowerCase();
  const id = raw === 'golang' ? 'go' : raw;
  return SERVER_NAMES[id] ? id : null;
}

export function ideLspServerName(lang: string): string {
  return SERVER_NAMES[lang] ?? lang;
}

export type IdeLspPhase = 'starting' | 'ready' | 'restarting' | 'lost' | 'unavailable';
export interface IdeLspRecord { lang: string; rootPath: string; phase: IdeLspPhase }

const related = (a: string, b: string): boolean => a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);

/** Fold one link event into the records (one per language and root). Reconnect events
 *  carry no language: they update every record of a related root. */
export function applyIdeLspLink(records: readonly IdeLspRecord[], d: IdeLspLinkDetail): IdeLspRecord[] {
  const root = d.rootPath.replace(/\/+$/, '') || '/';
  if (d.lang) {
    const phase: IdeLspPhase = d.state === 'ready' ? 'ready' : d.state === 'unavailable' ? 'unavailable' : 'starting';
    const rest = records.filter((r) => !(r.lang === d.lang && r.rootPath === root));
    return [...rest, { lang: d.lang, rootPath: root, phase }];
  }
  const phase: IdeLspPhase | null =
    d.state === 'connected' ? 'ready' : d.state === 'lost' ? 'lost' : d.state === 'reconnecting' || d.state === 'waiting' ? 'restarting' : null;
  if (!phase) return records.slice();
  return records.map((r) => (related(r.rootPath, root) ? { ...r, phase } : r));
}

/** The record that serves `filePath` for `lang` (the deepest root containing the file). */
export function ideLspRecordFor(records: readonly IdeLspRecord[], lang: string, filePath: string): IdeLspRecord | null {
  let best: IdeLspRecord | null = null;
  for (const r of records) {
    if (r.lang !== lang) continue;
    if (filePath !== r.rootPath && !filePath.startsWith(`${r.rootPath === '/' ? '' : r.rootPath}/`)) continue;
    if (!best || r.rootPath.length > best.rootPath.length) best = r;
  }
  return best;
}

export interface IdeLspItem { label: string; tone: 'ok' | 'busy' | 'bad'; title: string; action: 'settings' | 'retry' }

/** The status-strip item, or null before any server was asked for (no attach: nothing to say). */
export function ideLspItem(lang: string, rec: IdeLspRecord | null): IdeLspItem | null {
  if (!rec) return null;
  const name = ideLspServerName(lang);
  switch (rec.phase) {
    case 'ready':
      return { label: name, tone: 'ok', title: `${name} is running — open Plugins & Language Servers`, action: 'settings' };
    case 'restarting':
      return { label: `${name} restarting…`, tone: 'busy', title: 'The language server connection dropped; reconnecting.', action: 'settings' };
    case 'lost':
      return { label: `${name} disconnected`, tone: 'bad', title: 'The language server connection dropped. Click to retry.', action: 'retry' };
    case 'unavailable':
      return {
        label: `${name} unavailable`,
        tone: 'bad',
        title: `${name} could not start — it may not be installed. Open Plugins & Language Servers to install or check it.`,
        action: 'settings',
      };
    case 'starting':
    default:
      return { label: `Starting ${name}…`, tone: 'busy', title: `Starting ${name} for code intelligence`, action: 'settings' };
  }
}
