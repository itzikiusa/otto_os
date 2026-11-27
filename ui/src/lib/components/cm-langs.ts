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
import { foldInside, foldNodeProp, forceParsing, LanguageSupport, syntaxTree } from '@codemirror/language';
import { ViewPlugin, type EditorView, type ViewUpdate } from '@codemirror/view';
import { startMeasurement } from '../telemetry.ts';
import { sql } from '@codemirror/lang-sql';
import { sqlDialect as dialectFor, type SqlDialectName } from '../sql-dialects.ts';
import { redisLang } from './redis-lang.ts';

type Build = (dialect: SqlDialectName) => Extension;

const SYNC: Record<string, Build> = {
  sql: (d) => sql({ dialect: dialectFor(d) }),
  redis: () => redisLang(),
};

const js = () => import('@codemirror/lang-javascript');
// Markdown + the IDE's writing help (cm-md-assist.ts: active in IDE editors only).
const md = (): Promise<Build> =>
  Promise.all([import('@codemirror/lang-markdown'), import('./cm-md-assist.ts')]).then(([m, a]) => () => [m.markdown(), a.markdownAssist()]);
const extra = () => import('./cm-langs-extra.ts');
const config = () => import('./cm-langs-config.ts');
const infra = () => import('./cm-langs-infra.ts');

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
  zig: () => import('./cm-lang-zig.ts').then((m) => () => m.zigLang()),
  c: () => import('./cm-lang-cpp.ts').then((m) => () => m.cppLang()),
  cpp: () => import('./cm-lang-cpp.ts').then((m) => () => m.cppLang()),
  cmake: () => import('./cm-lang-cpp.ts').then((m) => () => m.cmakeLang()),
  meson: () => import('./cm-lang-cpp.ts').then((m) => () => m.mesonLang()),
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
  md,
  mdx: md,
  markdown: md,
  java: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  class: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  gradle: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  kt: () => import('./cm-lang-kotlin.ts').then((m) => () => m.kotlinLang()),
  kts: () => import('./cm-lang-kotlin.ts').then((m) => () => m.kotlinLang()),
  sh: () => extra().then((m) => () => m.shellLang()),
  bash: () => extra().then((m) => () => m.shellLang()),
  zsh: () => extra().then((m) => () => m.shellLang()),
  bats: () => extra().then((m) => () => m.shellLang()),
  shell: () => extra().then((m) => () => m.shellLang()),
  shellscript: () => extra().then((m) => () => m.shellLang()),
  mod: () => extra().then((m) => () => m.goModLang()),
  sum: () => extra().then((m) => () => m.goModLang()),
  work: () => extra().then((m) => () => m.goModLang()),
  // Configuration and infrastructure files (langForPath.ts maps names and extensions here).
  toml: () => config().then((m) => m.tomlLang),
  properties: () => config().then((m) => m.propertiesLang),
  yaml: () => config().then((m) => m.yamlLang),
  conf: () => config().then((m) => m.confLang),
  dotenv: () => config().then((m) => m.dotenvLang),
  ignore: () => config().then((m) => m.ignoreLang),
  requirements: () => config().then((m) => m.requirementsLang),
  dockerfile: () => infra().then((m) => m.dockerfileLang),
  nginx: () => infra().then((m) => m.nginxLang),
  proto: () => infra().then((m) => m.protoLang),
  hcl: () => infra().then((m) => m.hclLang),
  diff: () => infra().then((m) => m.diffLang),
  powershell: () => infra().then((m) => m.powershellLang),
  batch: () => infra().then((m) => m.batchLang),
  vtt: () => infra().then((m) => m.vttLang),
  groovy: () => import('@codemirror/lang-java').then((m) => () => m.java()),
  svelte: () => import('@codemirror/lang-html').then((m) => () => m.html()),
  vue: () => import('@codemirror/lang-html').then((m) => () => m.html()),
  // langExtOf maps every Makefile name (Makefile.local, *.mk…) to this key.
  makefile: () => import('./cm-lang-makefile.ts').then((m) => () => m.makefileLang()),
};

/** Colour what scrolls into view in the same frame. CodeMirror leaves the tree past the
 *  parsed end to its idle worker (100–500 ms after a scroll), and a StreamLanguage (shell,
 *  TOML, Makefile…) never parses past the viewport, so every scroll used to show plain
 *  lines first and colours later. When the tree ends before the viewport does, parse up to
 *  the viewport's end in a microtask: after CodeMirror's update, before the browser paints
 *  (a 2,000-line Makefile tokenizes in ~4 ms whole; the budget caps a huge file).
 *  In the IDE, the first time the viewport is coloured is recorded as `ui.highlight`. */
const eagerParse = ViewPlugin.fromClass(
  class {
    queued = false;
    parsing = false; // our own forceParsing dispatch, which must not queue another round
    span: ReturnType<typeof startMeasurement> | null;
    view: EditorView;
    constructor(view: EditorView) {
      this.view = view;
      this.span = view.dom.closest('.is-ide') ? startMeasurement('ui.highlight', 'ide') : null;
      this.check();
    }
    update(u: ViewUpdate): void {
      if (!this.parsing && (u.viewportChanged || u.docChanged || this.span)) this.check();
    }
    check(): void {
      if (this.queued) return;
      this.queued = true;
      queueMicrotask(() => {
        this.queued = false;
        const v = this.view, to = v.viewport.to;
        if (!v.dom.isConnected) return;
        // One 15 ms slice per update: past it CodeMirror's idle worker parses on, with yields.
        if (syntaxTree(v.state).length < to) { this.parsing = true; try { forceParsing(v, to, 15); } finally { this.parsing = false; } }
        if (this.span && syntaxTree(v.state).length >= to) {
          const span = this.span;
          this.span = null;
          requestAnimationFrame(() => span.finish());
        }
      });
    }
    destroy(): void {
      this.span?.finish('canceled');
    }
  },
);

/** Resolved builders, by extension (module-wide: every editor shares them). */
const ready = new Map<string, Build>();
const inflight = new Map<string, Promise<void>>();

/** The language extension for `ext`, if available synchronously (static or
 *  already loaded); null for an unknown ext or a pack still loading. */
export function cmLangNow(ext: string, dialect: SqlDialectName): Extension | null {
  const b = SYNC[ext] ?? ready.get(ext);
  return b ? [b(dialect), eagerParse] : null;
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
