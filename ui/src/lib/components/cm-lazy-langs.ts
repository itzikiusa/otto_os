// CodeMirror language packs that load on first use.
//
// CodeEditor used to import every language statically, so each page with an
// editor — the DB Explorer above all — carried Python, Go, Rust, HTML, CSS,
// Markdown and Java parsers it never uses. The editor keeps the languages the
// DB workbench needs (SQL, JavaScript for Mongo, JSON, Redis) eager and asks
// here for the rest: `lazyLangNow` answers synchronously once a pack has
// loaded; until then the doc opens plain and the editor reconfigures its
// language compartment when `loadLazyLang` resolves.
import type { LanguageSupport } from '@codemirror/language';

const LOADERS: Record<string, () => Promise<LanguageSupport>> = {
  py: () => import('@codemirror/lang-python').then((m) => m.python()),
  go: () => import('@codemirror/lang-go').then((m) => m.go()),
  rs: () => import('@codemirror/lang-rust').then((m) => m.rust()),
  html: () => import('@codemirror/lang-html').then((m) => m.html()),
  css: () => import('@codemirror/lang-css').then((m) => m.css()),
  md: () => import('@codemirror/lang-markdown').then((m) => m.markdown()),
  java: () => import('@codemirror/lang-java').then((m) => m.java()),
};
/** File extension → the loader key it shares a pack with. */
const ALIAS: Record<string, string> = {
  py: 'py',
  go: 'go',
  rs: 'rs',
  html: 'html',
  htm: 'html',
  xml: 'html',
  css: 'css',
  scss: 'css',
  less: 'css',
  md: 'md',
  mdx: 'md',
  java: 'java',
};

const loaded = new Map<string, LanguageSupport>();
const inflight = new Map<string, Promise<LanguageSupport | null>>();

/** Whether `ext` is served by a lazily loaded pack. */
export function isLazyLang(ext: string): boolean {
  return ext in ALIAS;
}

/** The pack for `ext` if it has already loaded, else null. */
export function lazyLangNow(ext: string): LanguageSupport | null {
  const key = ALIAS[ext];
  return key ? (loaded.get(key) ?? null) : null;
}

/** Load (once) the pack for `ext`; null for an unknown extension or a failed load. */
export function loadLazyLang(ext: string): Promise<LanguageSupport | null> {
  const key = ALIAS[ext];
  if (!key) return Promise.resolve(null);
  const have = loaded.get(key);
  if (have) return Promise.resolve(have);
  let p = inflight.get(key);
  if (!p) {
    p = LOADERS[key]().then(
      (lang) => {
        loaded.set(key, lang);
        inflight.delete(key);
        return lang;
      },
      () => {
        // Highlighting is optional: the doc stays plain; a later open retries.
        inflight.delete(key);
        return null;
      },
    );
    inflight.set(key, p);
  }
  return p;
}
