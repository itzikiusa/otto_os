// Which editor language a file gets (a leaf: the IDE's node-tested models import it).
// One table for every name and extension, so a config file never opens as plain text:
// file names first (Dockerfile, .gitignore, .env.local, Cargo.lock, nginx.conf…), then the
// extension, then the language the IDE read off the file (the server's, or one sniffed
// from the first lines: ideTypes.ts sniffIdeLanguage). Each key is a pack in cm-langs.ts.
import { isMakefilePath } from './makefilePath.ts';

/** Every key cm-langs.ts has a pack for (a unit test keeps the two lists equal). */
export const LANG_KEYS: ReadonlySet<string> = new Set([
  'sql', 'redis', 'js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs', 'mts', 'cts', 'py', 'pyi', 'go', 'rs', 'zig',
  'json', 'jsonc', 'jsonl', 'ndjson', 'html', 'htm', 'xml', 'svelte', 'vue', 'css', 'scss', 'less',
  'md', 'mdx', 'markdown', 'java', 'class', 'gradle', 'kts', 'groovy', 'sh', 'bash', 'zsh', 'bats',
  'shell', 'shellscript', 'mod', 'sum', 'work', 'toml', 'makefile', 'properties', 'yaml', 'conf',
  'dotenv', 'ignore', 'requirements', 'dockerfile', 'nginx', 'proto', 'hcl', 'diff', 'powershell', 'batch', 'vtt',
]);

/** Extensions that name another key. */
export const EXT_ALIAS: Readonly<Record<string, string>> = {
  yml: 'yaml', yaml: 'yaml',
  zon: 'zig',
  ini: 'conf', cfg: 'conf', cnf: 'conf', conf: 'conf', inf: 'conf', prefs: 'conf', desktop: 'conf',
  service: 'conf', socket: 'conf', timer: 'conf', mount: 'conf', editorconfig: 'conf', gitconfig: 'conf',
  npmrc: 'conf', xcconfig: 'conf', env: 'dotenv',
  tf: 'hcl', tfvars: 'hcl', hcl: 'hcl', nomad: 'hcl',
  patch: 'diff', rej: 'diff',
  ps1: 'powershell', psm1: 'powershell', psd1: 'powershell', bat: 'batch', cmd: 'batch',
  fish: 'sh', ksh: 'sh', jenkinsfile: 'groovy', dockerfile: 'dockerfile', nginx: 'nginx',
  xsd: 'xml', xsl: 'xml', xslt: 'xml', plist: 'xml', pom: 'xml', svg: 'xml', csproj: 'xml', fsproj: 'xml',
  vbproj: 'xml', props: 'xml', targets: 'xml', iml: 'xml', storyboard: 'xml', xib: 'xml', wsdl: 'xml',
  xaml: 'xml', resx: 'xml', nuspec: 'xml', xhtml: 'html',
  json5: 'json', geojson: 'json', webmanifest: 'json', har: 'json',
  ddl: 'sql', pgsql: 'sql', psql: 'sql', plsql: 'sql', mysql: 'sql',
};

/** Exact (lower-case) file names. */
export const NAMES: Readonly<Record<string, string>> = {
  dockerfile: 'dockerfile', containerfile: 'dockerfile',
  '.gitignore': 'ignore', '.dockerignore': 'ignore', '.gitattributes': 'ignore', codeowners: 'ignore', owners: 'ignore',
  '.editorconfig': 'conf', '.npmrc': 'conf', '.yarnrc': 'conf', '.gitconfig': 'conf', '.gitmodules': 'conf', hosts: 'conf',
  '.curlrc': 'conf', '.wgetrc': 'conf', '.pypirc': 'conf', '.flake8': 'conf', '.pylintrc': 'conf', '.coveragerc': 'conf',
  '.htaccess': 'conf', '.inputrc': 'conf', procfile: 'conf', 'gemfile.lock': 'conf',
  '.env': 'dotenv',
  '.bashrc': 'sh', '.bash_profile': 'sh', '.zshrc': 'sh', '.zshenv': 'sh', '.zprofile': 'sh', '.profile': 'sh', '.envrc': 'sh', pkgbuild: 'sh',
  'cargo.lock': 'toml', 'poetry.lock': 'toml', 'uv.lock': 'toml', 'pdm.lock': 'toml', pipfile: 'toml',
  'yarn.lock': 'yaml',
  'composer.lock': 'json', 'pipfile.lock': 'json', 'flake.lock': 'json', 'deno.lock': 'json', 'bun.lock': 'json',
  '.babelrc': 'json', '.eslintrc': 'json',
  jenkinsfile: 'groovy', 'nginx.conf': 'nginx',
};

/** File-name patterns, then folders that decide (nginx sites, ~/.ssh/config…). */
const PATTERNS: [RegExp, string][] = [
  [/^(?:dockerfile|containerfile)[.-]|\.(?:dockerfile|containerfile)$/, 'dockerfile'],
  [/^\.env\.|\.env$/, 'dotenv'],
  [/^(?:requirements|constraints)(?:[-_.][\w.-]*)?\.(?:txt|in)$/, 'requirements'],
  [/^\.[\w.-]*ignore$/, 'ignore'],
];
const DIRS: [RegExp, string][] = [
  [/\/(?:nginx|sites-available|sites-enabled|conf\.d)\/[^/]+$/, 'nginx'],
  [/\/\.ssh\/config$|\/\.git\/config$|\/\.aws\/(?:config|credentials)$/, 'conf'],
  [/\/\.cargo\/config$/, 'toml'],
  [/\/\.kube\/config$/, 'yaml'],
];

/** What a language the IDE read off a file (server ids, sniffed keys) is called here. */
const HINT_ALIAS: Record<string, string> = {
  protobuf: 'proto', ini: 'conf', shell: 'sh', markdown: 'md', javascript: 'js', typescript: 'ts', python: 'py',
  rust: 'rs', golang: 'go', zon: 'zig',
};

function extOf(base: string): string {
  const dot = base.lastIndexOf('.');
  return dot < 0 ? '' : base.slice(dot + 1).toLowerCase();
}

/** The editor language key for a file; for a file nothing knows, its extension (or the
 *  hint) as before, so callers that only need a name (a fence label) keep it. */
export function langKeyFor(filePath: string, hint?: string): string {
  if (filePath.startsWith('jdt://')) return 'java';
  const clean = filePath.split('?')[0];
  if (isMakefilePath(clean)) return 'makefile';
  const base = clean.slice(clean.lastIndexOf('/') + 1).toLowerCase();
  const named = NAMES[base] ?? PATTERNS.find(([re]) => re.test(base))?.[1];
  if (named) return named;
  const ext = extOf(base);
  if (ext === 'conf' || !ext) {
    const lowerPath = clean.toLowerCase();
    const inDir = DIRS.find(([re]) => re.test(lowerPath))?.[1];
    if (inDir) return inDir;
  }
  const byExt = EXT_ALIAS[ext] ?? (LANG_KEYS.has(ext) ? ext : '');
  if (byExt) return byExt;
  const h = (hint ?? '').toLowerCase();
  const fromHint = HINT_ALIAS[h] ?? h;
  if (LANG_KEYS.has(fromHint)) return fromHint;
  return ext || (hint ?? '');
}

/** Languages with a debugger, a runner or a language server: they keep the breakpoint /
 *  ▶ column and the lint gutter. Every other language drops what it cannot use. */
const CODE_KEYS = new Set([
  'graphql', 'gql', 'go', 'rs', 'java', 'class', 'kt', 'kts', 'scala', 'groovy', 'gradle', 'ts', 'tsx', 'mts', 'cts', 'js', 'jsx',
  'mjs', 'cjs', 'py', 'pyi', 'rb', 'php', 'c', 'h', 'cc', 'cpp', 'cxx', 'hpp', 'cs', 'swift', 'dart', 'lua',
  'sh', 'bash', 'zsh', 'bats', 'svelte', 'vue', 'm', 'mm', 'ex', 'exs', 'zig', 'jl', 'clj', 'hs', 'ml', 'erl', 'nim',
]);
/** Languages whose editor folds (a fold column is worth its 14 px). */
const FOLDING_KEYS = new Set([
  'json', 'jsonc', 'html', 'htm', 'xml', 'css', 'scss', 'less', 'md', 'mdx', 'markdown', 'sql',
  'yaml', 'toml', 'conf', 'hcl', 'proto', 'nginx', 'diff',
]);

export interface LangGutters {
  /** The breakpoint column (and the ▶ run marks a runner gives it). */
  breakpoints: boolean;
  /** A ▶ column with no breakpoints (Makefile targets). */
  runOnly: boolean;
  lint: boolean;
  fold: boolean;
}

/** The gutters a file's language can use: code keeps all of them; a Makefile keeps a ▶
 *  column; every other file (config, data, prose, plain text) only the fold column when
 *  its language folds, so its text starts next to the line numbers. */
export function langGutters(filePath: string, hint?: string): LangGutters {
  const key = langKeyFor(filePath, hint);
  if (key === 'makefile') return { breakpoints: false, runOnly: true, lint: false, fold: false };
  const code = CODE_KEYS.has(key);
  return { breakpoints: code, runOnly: false, lint: code || key === 'json' || key === 'jsonc', fold: code || FOLDING_KEYS.has(key) };
}

/** A shebang's interpreter, JSON, YAML, or line-oriented "key = value" / "# comment" /
 *  "[section]" config text, read from at most the first 4 KB ('' when none fits). */
export function sniffLangKey(content: string): string {
  const head = content.slice(0, 4096);
  const shebang = /^#!\s*\S*?(?:\/env\s+)?(bash|sh|zsh|ksh|dash|python3?|node)\b/.exec(head);
  if (shebang) return /python/.test(shebang[1]) ? 'python' : shebang[1] === 'node' ? 'javascript' : 'shell';
  if (/^\s*\{\s*(?:"|\}|$)|^\s*\[\s*(?:[{"\d\]]|$)/.test(head)) return 'json';
  const lines = head.split('\n').slice(0, 40).map((l) => l.trim()).filter(Boolean);
  if (head.length === 4096 && lines.length > 1) lines.pop(); // cut mid-line
  let comment = 0, conf = 0, yaml = 0;
  for (const l of lines) {
    if (/^[#;!]/.test(l)) comment++;
    else if (/^\[[^\]]+\]$/.test(l) || /^(?:export\s+)?[\w.\-@/:$"'[\]]+\s*=/.test(l) || /^\d{1,3}(?:\.\d{1,3}){3}\s+\S/.test(l)) conf++;
    else if (/^(?:- )?[\w.\-"']+:(?:\s|$)/.test(l) || /^- \S/.test(l) || l === '---') yaml++;
  }
  const known = comment + conf + yaml;
  if (!lines.length || known < lines.length * 0.7 || conf + yaml === 0) return '';
  return yaml > conf ? 'yaml' : 'conf';
}
