// Code blocks in chat prose, IDE-style: a header (language · file name when
// the fence names one · line count · Wrap · Open · Copy), line numbers, syntax
// colours from a whole-block highlight (vault/mdRender.ts `codeLines`: one
// `<span class="cl">` per line), and a fold for long blocks ("Show all N
// lines"). The markdown is rendered to an HTML STRING once and cached
// (mdCache.ts), so the chrome is added to that string here (after the
// sanitizer: it is our own markup around already-escaped code) and driven by
// ONE delegated click handler per block of prose (`runCodeAction`) that
// toggles classes on the live DOM. No component per code block, nothing
// re-rendered on click, the cache stays a string memo.

// Header labels for fence tags and hljs ids (static: no hljs needed, so the
// label is right before the lazy highlighter lands and in node unit tests).
const LANG_LABELS: Record<string, string> = {
  ts: 'TypeScript', typescript: 'TypeScript', tsx: 'TSX', mts: 'TypeScript', cts: 'TypeScript',
  js: 'JavaScript', javascript: 'JavaScript', jsx: 'JSX', mjs: 'JavaScript', cjs: 'JavaScript',
  rs: 'Rust', rust: 'Rust', go: 'Go', golang: 'Go', py: 'Python', python: 'Python', rb: 'Ruby', ruby: 'Ruby',
  sh: 'Shell', bash: 'Shell', zsh: 'Shell', shell: 'Shell', console: 'Shell', fish: 'Shell',
  json: 'JSON', jsonc: 'JSON', json5: 'JSON', yml: 'YAML', yaml: 'YAML', toml: 'TOML', ini: 'INI',
  html: 'HTML', xml: 'XML', svg: 'SVG', svelte: 'Svelte', vue: 'Vue', css: 'CSS', scss: 'SCSS', less: 'Less',
  sql: 'SQL', java: 'Java', kt: 'Kotlin', kotlin: 'Kotlin', swift: 'Swift', c: 'C', h: 'C', cpp: 'C++',
  'c++': 'C++', cc: 'C++', cs: 'C#', csharp: 'C#', php: 'PHP', md: 'Markdown', markdown: 'Markdown',
  diff: 'Diff', patch: 'Diff', makefile: 'Makefile', make: 'Makefile', dockerfile: 'Dockerfile', docker: 'Dockerfile',
  lua: 'Lua', perl: 'Perl', pl: 'Perl', r: 'R', graphql: 'GraphQL', gql: 'GraphQL', proto: 'Protobuf',
  text: 'Text', txt: 'Text', plaintext: 'Text', log: 'Log', mermaid: 'Mermaid', d2: 'D2',
};

/** Human label for a fence tag or language id ("ts" → "TypeScript"). */
export function langLabel(tag: string | null | undefined): string {
  if (!tag) return '';
  return LANG_LABELS[tag.toLowerCase()] ?? tag;
}

/** Blocks longer than this start folded to about this many lines. */
export const CODE_CAP_LINES = 18;

// vault/mdRender.ts emits `<pre><code class="hljs[ language-x]"[ title="file"]>…</code></pre>`
// for every fenced block (diagram fences render as `.diagram-block` instead).
// A block already wrapped (its `<pre>` directly follows our `.code-head`) is
// skipped, so decorating twice never nests toolbars.
const PRE = /(?<!<\/div>)<pre><code class="hljs(?: language-([\w+#.-]+))?"(?: title="([^"]*)")?>([\s\S]*?)<\/code><\/pre>/g;
const LINE_SPAN = /<span class="cl">/g;

const esc = (s: string): string => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
/** `title` arrives entity-escaped from the sanitizer's serializer. */
const unesc = (s: string): string => s.replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');

const base = (p: string): string => p.split('/').pop() || p;

/** Wrap each fenced code block of rendered markdown in the chat's code-block
 *  chrome. Idempotent. */
export function decorateCodeBlocks(html: string): string {
  if (!html.includes('<pre><code class="hljs')) return html;
  return html.replace(PRE, (_m, lang: string | undefined, title: string | undefined, body: string) => {
    const spans = body.match(LINE_SPAN)?.length ?? 0;
    const lines = spans || body.replace(/\n$/, '').split('\n').length;
    const long = lines > CODE_CAP_LINES;
    const numbered = spans > 1;
    const label = lang ? esc(langLabel(lang)) : 'Text';
    const cls = lang ? `hljs language-${esc(lang)}` : 'hljs';
    const file = title ? unesc(title) : '';
    const gut = String(lines).length + 1;
    return (
      `<div class="code-block${long ? ' capped' : ''}${numbered ? ' numbered' : ''}" data-lines="${lines}"${lang ? ` data-lang="${esc(lang)}"` : ''} style="--gut:${gut}ch">` +
      `<div class="code-head"><span class="code-lang">${label}</span>` +
      (file
        ? `<button type="button" class="code-file" data-code-act="file" data-file="${esc(file)}" title="Preview ${esc(file)}">${esc(base(file))}</button>`
        : '') +
      `<span class="code-sp"></span>` +
      (lines > 1 ? `<span class="code-n">${lines} lines</span>` : '') +
      `<button type="button" class="code-btn" data-code-act="wrap" aria-pressed="false" title="Wrap long lines">Wrap</button>` +
      `<button type="button" class="code-btn" data-code-act="open" title="Open in the side panel">Open</button>` +
      `<button type="button" class="code-btn" data-code-act="copy" title="Copy code">Copy</button></div>` +
      `<pre><code class="${cls}">${body}</code></pre>` +
      (long
        ? `<button type="button" class="code-more" data-code-act="expand" aria-expanded="false">Show all ${lines} lines</button>`
        : '') +
      `</div>`
    );
  });
}

/** The text of a decorated block's code: its lines re-joined (they are block
 *  spans with no newline between them), or the plain text of an old block. */
export function codeText(block: Element): string {
  const code = block.querySelector('code');
  if (!code) return '';
  const lines = code.querySelectorAll(':scope > .cl');
  if (!lines.length) return code.textContent ?? '';
  return Array.from(lines, (l) => l.textContent ?? '').join('\n');
}

/** What a code block asks its host for (the chat opens the side panel). */
export interface CodeActionHost {
  copy?: (text: string) => Promise<void>;
  /** "Open": the block's code in the side panel. */
  openCode?: (code: { text: string; lang: string | null; file: string | null }) => void;
  /** The file name in the header. */
  openFile?: (path: string) => void;
}

/** Handle a click inside rendered prose. Returns true when it was a code-block
 *  control (the caller stops there). */
export function runCodeAction(target: EventTarget | null, host: CodeActionHost = {}): boolean {
  if (!(target instanceof Element)) return false;
  const btn = target.closest<HTMLElement>('[data-code-act]');
  const block = btn?.closest<HTMLElement>('.code-block');
  if (!btn || !block) return false;
  const act = btn.dataset.codeAct;
  if (act === 'copy') {
    const copy = host.copy ?? ((t: string) => navigator.clipboard.writeText(t));
    void copy(codeText(block)).then(
      () => flash(btn, 'Copied'),
      () => flash(btn, 'Copy failed'),
    );
  } else if (act === 'wrap') {
    const on = block.classList.toggle('wrap');
    btn.setAttribute('aria-pressed', String(on));
  } else if (act === 'expand') {
    const open = block.classList.toggle('expanded');
    btn.setAttribute('aria-expanded', String(open));
    btn.textContent = open ? 'Show less' : `Show all ${block.dataset.lines ?? ''} lines`;
  } else if (act === 'open') {
    const file = block.querySelector<HTMLElement>('[data-file]')?.dataset.file ?? null;
    host.openCode?.({ text: codeText(block), lang: block.dataset.lang ?? null, file });
  } else if (act === 'file') {
    const file = btn.dataset.file;
    if (file) host.openFile?.(file);
  }
  return true;
}

function flash(btn: HTMLElement, text: string): void {
  const prev = btn.dataset.label ?? btn.textContent ?? '';
  btn.dataset.label = prev;
  btn.textContent = text;
  setTimeout(() => {
    btn.textContent = prev;
    delete btn.dataset.label;
  }, 1400);
}
