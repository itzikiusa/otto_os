// The chat's markdown: the vault renderer (marked GFM + allowlist sanitizer,
// vault/mdRender.ts) with chat-only extensions, then the code-block chrome.
//   • file references — `crates/x/src/retry.rs:88` in prose or in a code span
//     (and a bare `retry.rs` code span with a known extension) — become
//     `a.file-ref[data-path][data-anchor=line]`, opened in the side panel;
//   • GitHub pull-request / issue URLs autolinked as `owner/repo#123` chips;
//     a bare `#123` becomes a `ref-chip` the host resolves against the
//     session's repo remote;
//   • fenced code is highlighted as one block, line by line (`codeLines`).
// Everything here is a STRING transform, so the md cache keeps one string per
// block and the rendering stays unit-testable in node (no DOM).
import type { MarkedExtension, Tokens } from 'marked';
import { renderNote, type RenderCtx } from '../../vault/mdRender';
import { decorateCodeBlocks } from './codeBlocks';

const esc = (s: string): string => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Extensions a bare file name (no directory) must have to count as a file
 *  reference inside a code span — `retry.rs` yes, `v0.4.1` / `self.x` no. */
const CODE_EXTS = new Set(
  'rs ts tsx js jsx mjs cjs mts go py rb java kt kts c h cc cpp hpp cs php swift scala sh bash zsh fish sql html htm css scss less svelte vue json jsonc toml yaml yml md mdx txt xml proto lock cfg ini env csv tsv log svg png jpg jpeg gif webp gql graphql lua pl r dockerfile gradle'.split(' '),
);
const FILE_NAMES = new Set(['Dockerfile', 'Makefile', 'Justfile', 'Cargo.toml', 'Cargo.lock', 'package.json', 'README', 'LICENSE']);

// A path in prose: a directory part and a file extension (starting with a
// letter, so "1/2.5" and "and/or" are not paths), then an optional location.
const PROSE_PATH = /(?<![\w@.+/:~#\\-])(?:~\/|\.{1,2}\/|\/)?[\w@+-][\w@.+-]*(?:\/[\w@.+-]+)+\.[A-Za-z][A-Za-z0-9]{0,9}(?::\d+(?::\d+)?|#L\d+)?(?![\w/])/;
const LOCATION = /(?::(\d+)(?::(\d+))?|#L(\d+))$/;

export interface FileRef {
  path: string;
  line: number | null;
  col: number | null;
}

/** Split `path:line:col` / `path#L12` into its parts. */
export function splitLocation(raw: string): FileRef {
  const m = LOCATION.exec(raw);
  if (!m) return { path: raw, line: null, col: null };
  const line = Number(m[1] ?? m[3]);
  return { path: raw.slice(0, m.index), line: Number.isFinite(line) ? line : null, col: m[2] ? Number(m[2]) : null };
}

/** The file a code span names, if it names one: a path with a directory and
 *  an extension, or a bare file name with a known extension. */
export function codeSpanFile(text: string): FileRef | null {
  const t = text.trim();
  if (!t || t.length > 300 || /\s/.test(t)) return null;
  if (!/^(?:~\/|\.{1,2}\/|\/)?[\w@.+-]+(?:\/[\w@.+-]+)*(?::\d+(?::\d+)?|#L\d+)?$/.test(t)) return null;
  const ref = splitLocation(t);
  const name = ref.path.split('/').pop() ?? '';
  if (FILE_NAMES.has(name)) return ref;
  const ext = /\.([A-Za-z][A-Za-z0-9]{0,9})$/.exec(name)?.[1];
  if (!ext) return null;
  if (ref.path.includes('/')) return ref;
  return CODE_EXTS.has(ext.toLowerCase()) ? ref : null;
}

function fileRefHtml(ref: FileRef, inner: string, code: boolean): string {
  const loc = ref.line != null ? `${ref.line}${ref.col != null ? `:${ref.col}` : ''}` : '';
  const anchor = loc ? ` data-anchor="${loc}"` : '';
  const title = `Open ${ref.path}${loc ? ` at line ${ref.line}` : ''}`;
  return `<a class="file-ref${code ? ' code' : ''}" data-path="${esc(ref.path)}"${anchor} title="${esc(title)}">${inner}</a>`;
}

// github.com/<owner>/<repo>/(pull|issues)/<n>[…]
const GH_REF = /^https?:\/\/github\.com\/([\w.-]+)\/([\w.-]+)\/(pull|issues)\/(\d+)(?:[/?#].*)?$/i;

/** `owner/repo#123` for a GitHub pull-request / issue URL, else null. */
export function githubRef(url: string): { owner: string; repo: string; kind: 'pr' | 'issue'; n: number } | null {
  const m = GH_REF.exec(url.trim());
  return m ? { owner: m[1], repo: m[2], kind: m[3].toLowerCase() === 'pull' ? 'pr' : 'issue', n: Number(m[4]) } : null;
}

export const chatMarkedExtension: MarkedExtension = {
  extensions: [
    {
      name: 'chatFileRef',
      level: 'inline',
      start(src: string) {
        const m = PROSE_PATH.exec(src);
        return m ? m.index : undefined;
      },
      tokenizer(src: string) {
        const m = PROSE_PATH.exec(src);
        if (!m || m.index !== 0) return undefined;
        // A trailing sentence period is prose, not part of the extension.
        const raw = m[0];
        return { type: 'chatFileRef', raw, ref: splitLocation(raw) };
      },
      renderer(token) {
        const { raw, ref } = token as unknown as { raw: string; ref: FileRef };
        return fileRefHtml(ref, esc(raw), false);
      },
    },
    {
      name: 'chatIssueRef',
      level: 'inline',
      start(src: string) {
        const m = /(?<![\w&#/])#\d{1,6}\b/.exec(src);
        return m ? m.index : undefined;
      },
      tokenizer(src: string) {
        const m = /^#(\d{1,6})\b(?![\w-])/.exec(src);
        if (!m) return undefined;
        return { type: 'chatIssueRef', raw: m[0], n: m[1] };
      },
      renderer(token) {
        const n = String((token as unknown as { n: string }).n);
        return `<a class="ref-chip issue" data-raw="${n}" title="Pull request or issue #${n}">#${n}</a>`;
      },
    },
  ],
  renderer: {
    codespan({ text }: Tokens.Codespan) {
      const ref = codeSpanFile(text);
      if (!ref) return false;
      return fileRefHtml(ref, `<code>${esc(text)}</code>`, true);
    },
    link({ href, tokens }: Tokens.Link) {
      const gh = githubRef(href);
      if (!gh) return false;
      const text = tokens?.map((t) => ('raw' in t ? t.raw : '')).join('') ?? '';
      // Only an autolink (the text IS the URL) becomes a chip; a written label wins.
      if (text && text.trim() !== href.trim()) return false;
      const label = `${gh.owner}/${gh.repo}#${gh.n}`;
      return `<a class="ref-chip ${gh.kind}" href="${esc(href)}" target="_blank" rel="noopener noreferrer" title="${gh.kind === 'pr' ? 'Pull request' : 'Issue'} ${esc(label)}">${esc(label)}</a>`;
    },
  },
};

// Plain (escaped, untagged) tool output: URLs and file references become
// links — "panicked at crates/otto-net/src/retry.rs:88:9" opens the file there.
const OUT_LINK = new RegExp(`(https?:\\/\\/[^\\s<>"'\\\`]+)|(${PROSE_PATH.source})`, 'g');

/** Linkify ESCAPED plain text (a command's output). Tags are never produced
 *  inside tags: the input has none. */
export function linkifyOutput(escaped: string): string {
  if (escaped.length > 256 * 1024 || escaped.includes('<')) return escaped;
  return escaped.replace(OUT_LINK, (m: string, url: string | undefined, path: string | undefined) => {
    if (url) {
      // An escaped `<…>` / quote ends the URL, as does trailing punctuation.
      const clean = url.split(/&(?:lt|gt|quot|#39);/)[0].replace(/[.,;:!?)\]]+$/, '');
      const rest = url.slice(clean.length);
      return `<a class="out-link" href="${clean}" rel="noopener noreferrer">${clean}</a>${rest}`;
    }
    if (path) return fileRefHtml(splitLocation(path), path, false);
    return m;
  });
}

const ctx: RenderCtx = { resolve: () => null, assetUrl: () => null, codeLines: true, use: [chatMarkedExtension] };

// Anchors without an href are not focusable; the chat's own reference links
// get a tab stop and a link role here (after the sanitizer, our own markup —
// text is escaped, so only real tags match).
const REF_OPEN = /<a class="(file-ref|ref-chip)/g;
// External links lose `target=_blank` (the sanitizer adds it): the chat opens
// them itself — the system browser, or Otto's with ⌥ — and App.svelte's
// global `_blank` handler would open them a second time.
const BLANK = / target="_blank"/g;

/** Render one block of chat prose to sanitized, decorated HTML. */
export function renderChatMarkdown(md: string): string {
  const html = renderNote(md, ctx);
  return decorateCodeBlocks(html).replace(REF_OPEN, '<a tabindex="0" role="link" class="$1').replace(BLANK, '');
}
