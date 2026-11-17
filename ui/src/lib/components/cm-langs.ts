// CodeMirror language packs for CodeEditor, loaded on demand.
//
// Every page that embeds a CodeEditor (Git, API, Vault, DB, Home's DB widget…)
// used to statically bundle all ten `@codemirror/lang-*` grammars. Now each
// non-SQL pack is its own chunk, imported the first time a doc of that kind is
// shown; the editor starts in plain text and reconfigures its language
// compartment once the pack resolves (CodeEditor guards the swap against a
// rebuilt view / switched doc).
//
// SQL and Redis stay STATIC on purpose: they power the hot-path DB console.
import type { Extension } from '@codemirror/state';
import { foldInside, foldNodeProp, LanguageSupport } from '@codemirror/language';
import { sql } from '@codemirror/lang-sql';
import { sqlDialect as dialectFor, type SqlDialectName } from '../sql-dialects.ts';
import { redisLang } from './redis-lang.ts';

type Build = (dialect: SqlDialectName) => Extension;

const SYNC: Record<string, Build> = {
  sql: (d) => sql({ dialect: dialectFor(d) }),
  redis: () => redisLang(),
};

const js = () => import('@codemirror/lang-javascript');
const extra = () => import('./cm-langs-extra.ts');

const LOADERS: Record<string, () => Promise<Build>> = {
  js: () => js().then((m) => () => m.javascript()),
  jsx: () => js().then((m) => () => m.javascript({ jsx: true })),
  ts: () => js().then((m) => () => m.javascript({ typescript: true })),
  tsx: () => js().then((m) => () => m.javascript({ jsx: true, typescript: true })),
  mjs: () => js().then((m) => () => m.javascript()),
  cjs: () => js().then((m) => () => m.javascript()),
  mts: () => js().then((m) => () => m.javascript({ typescript: true })),
  cts: () => js().then((m) => () => m.javascript({ typescript: true })),
  py: () => import('@codemirror/lang-python').then((m) => () => m.python()),
  pyi: () => import('@codemirror/lang-python').then((m) => () => m.python()),
  go: () =>
    import('@codemirror/lang-go').then((m) => {
      // lang-go folds `StructType` / `InterfaceType`, whose range ends before their `{`, so
      // `type X struct {` got no chevron; fold their bodies instead.
      const lang = m.goLanguage.configure({ props: [foldNodeProp.add({ 'StructBody InterfaceBody': foldInside })] });
      return () => new LanguageSupport(lang, m.go().support);
    }),
  rs: () => import('@codemirror/lang-rust').then((m) => () => m.rust()),
  json: () => import('@codemirror/lang-json').then((m) => () => m.json()),
  jsonc: () => import('@codemirror/lang-json').then((m) => () => m.json()),
  jsonl: () => import('@codemirror/lang-json').then((m) => () => m.json()),
  ndjson: () => import('@codemirror/lang-json').then((m) => () => m.json()),
  html: () => import('@codemirror/lang-html').then((m) => () => m.html()),
  htm: () => import('@codemirror/lang-html').then((m) => () => m.html()),
  xml: () => import('@codemirror/lang-html').then((m) => () => m.html()),
  css: () => import('@codemirror/lang-css').then((m) => () => m.css()),
  scss: () => import('@codemirror/lang-css').then((m) => () => m.css()),
  less: () => import('@codemirror/lang-css').then((m) => () => m.css()),
  md: () => import('@codemirror/lang-markdown').then((m) => () => m.markdown()),
  mdx: () => import('@codemirror/lang-markdown').then((m) => () => m.markdown()),
  java: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  class: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  gradle: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  kts: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  sh: () => extra().then((m) => () => m.shellLang()),
  bash: () => extra().then((m) => () => m.shellLang()),
  zsh: () => extra().then((m) => () => m.shellLang()),
  bats: () => extra().then((m) => () => m.shellLang()),
  shell: () => extra().then((m) => () => m.shellLang()),
  shellscript: () => extra().then((m) => () => m.shellLang()),
  mod: () => extra().then((m) => () => m.goModLang()),
  sum: () => extra().then((m) => () => m.goModLang()),
  work: () => extra().then((m) => () => m.goModLang()),
  toml: () => extra().then((m) => () => m.tomlLang()),
};

/** Resolved builders, by extension (module-wide: every editor shares them). */
const ready = new Map<string, Build>();
const inflight = new Map<string, Promise<void>>();

/** The language extension for `ext`, if available synchronously (static or
 *  already loaded); null for an unknown ext or a pack still loading. */
export function cmLangNow(ext: string, dialect: SqlDialectName): Extension | null {
  const b = SYNC[ext] ?? ready.get(ext);
  return b ? b(dialect) : null;
}

/** Whether `ext` has a pack that isn't loaded yet. */
export function cmLangPending(ext: string): boolean {
  return !(ext in SYNC) && !ready.has(ext) && ext in LOADERS;
}

/** Load `ext`'s pack (deduped). Resolves once `cmLangNow(ext)` is non-null;
 *  a failed import is forgotten so a later doc retries. */
export function loadCmLang(ext: string): Promise<void> {
  const load = LOADERS[ext];
  if (!load || ready.has(ext)) return Promise.resolve();
  let p = inflight.get(ext);
  if (!p) {
    p = load().then(
      (b) => {
        ready.set(ext, b);
        inflight.delete(ext);
      },
      (e: unknown) => {
        inflight.delete(ext);
        throw e;
      },
    );
    inflight.set(ext, p);
  }
  return p;
}
