// Vault reading-view renderer: marked (GFM) + Obsidian constructs — wikilinks
// `[[target|alias]]`, embeds `![[target]]`, inline `#tags`, `%%comments%%`,
// callouts `> [!note]` — resolved against the note's indexed outgoing links,
// syntax-highlighted via lib/hl, and passed through the allowlist sanitizer.

import { Marked } from 'marked';
import { ensureHljs, highlightLine, langFromPath } from '../../lib/hl';
import { sanitizeHtml } from '../../lib/sanitize';
import type { VaultOutgoingLink } from '../../lib/api/types';

export interface RenderCtx {
  /** Resolve a raw link target → vault path (from the note's outgoing links). */
  resolve: (raw: string) => string | null;
  /** Authenticated blob URL for an attachment path (may be null while loading). */
  assetUrl: (path: string) => string | null;
}

function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

export function slugifyHeading(text: string): string {
  return text
    .toLowerCase()
    .trim()
    .replace(/[^\p{L}\p{N}\s-]/gu, '')
    .replace(/\s+/g, '-');
}

const IMG_EXT = /\.(png|jpe?g|gif|webp|svg|avif|bmp)$/i;

// Diagram fences render as live diagrams in the reading view (NoteView's
// post-render pass feeds them to the lazy mermaid/D2 bridges). Agents often
// emit mermaid WITHOUT a language tag, so a bare fence whose first line is a
// mermaid grammar keyword ("flowchart LR", "sequenceDiagram", …) counts too.
const MERMAID_START =
  /^\s*(flowchart|graph|sequenceDiagram|classDiagram|stateDiagram(?:-v2)?|erDiagram|journey|gantt|pie\b|mindmap|timeline|quadrantChart|gitGraph|sankey-beta|xychart-beta|block-beta|C4Context|C4Container|C4Component|C4Dynamic)\b/;

/** 'mermaid' | 'd2' when a fenced block should render as a diagram. */
export function diagramKind(lang: string | null, text: string): 'mermaid' | 'd2' | null {
  if (lang === 'mermaid') return 'mermaid';
  if (lang === 'd2') return 'd2';
  if (!lang) {
    const first = text.split('\n').find((l) => l.trim().length > 0) ?? '';
    if (MERMAID_START.test(first)) return 'mermaid';
  }
  return null;
}

/** Build a per-note renderer map from its indexed outgoing links. */
export function resolverFrom(outgoing: VaultOutgoingLink[]): (raw: string) => string | null {
  const map = new Map<string, string | null>();
  for (const l of outgoing) map.set(l.raw_target.toLowerCase(), l.dst_path);
  return (raw) => map.get(raw.trim().toLowerCase()) ?? null;
}

/** Strip frontmatter — the Properties panel shows it; the body view doesn't. */
export function stripFrontmatter(raw: string): string {
  if (!raw.startsWith('---')) return raw;
  const m = raw.match(/^---\r?\n[\s\S]*?\r?\n(?:---|\.\.\.)[^\n]*\r?\n?/);
  return m ? raw.slice(m[0].length) : raw;
}

/** Remove %%comments%% (incl. multiline) outside fenced code blocks. */
export function stripComments(md: string): string {
  // Segment the document into fenced-code vs prose runs; strip only in prose.
  const segs: { code: boolean; lines: string[] }[] = [{ code: false, lines: [] }];
  let inFence: string | null = null;
  for (const line of md.split('\n')) {
    const t = line.trimStart();
    if (inFence) {
      segs[segs.length - 1].lines.push(line);
      if (t.startsWith(inFence)) {
        inFence = null;
        segs.push({ code: false, lines: [] });
      }
      continue;
    }
    if (t.startsWith('```') || t.startsWith('~~~')) {
      inFence = t.slice(0, 3);
      segs.push({ code: true, lines: [line] });
      continue;
    }
    segs[segs.length - 1].lines.push(line);
  }
  return segs
    .filter((s) => s.lines.length > 0)
    .map((s) =>
      s.code
        ? s.lines.join('\n')
        : s.lines.join('\n').replace(/%%[\s\S]*?%%/g, '').replace(/%%[\s\S]*$/g, ''),
    )
    .join('\n');
}

function makeMarked(ctx: RenderCtx): Marked {
  const m = new Marked({ gfm: true, breaks: true, async: false });

  m.use({
    extensions: [
      {
        name: 'vaultEmbed',
        level: 'inline',
        start(src: string) {
          return src.indexOf('![[');
        },
        tokenizer(src: string) {
          const match = /^!\[\[([^\]]+)\]\]/.exec(src);
          if (!match) return undefined;
          return { type: 'vaultEmbed', raw: match[0], target: match[1] };
        },
        renderer(token) {
          const inner = String((token as unknown as { target: string }).target);
          const [t] = inner.split('|');
          const [target] = t.split('#');
          const dst = ctx.resolve(target.trim());
          if (dst && IMG_EXT.test(dst)) {
            const url = ctx.assetUrl(dst);
            return url
              ? `<img src="${esc(url)}" alt="${esc(target.trim())}" loading="lazy" />`
              : `<span class="embed-pending" data-path="${esc(dst)}">${esc(target.trim())}</span>`;
          }
          if (dst) {
            return `<div class="note-embed" data-embed-path="${esc(dst)}"><span class="embed-title">${esc(target.trim())}</span></div>`;
          }
          return `<span class="wikilink unresolved" data-raw="${esc(target.trim())}">${esc(inner)}</span>`;
        },
      },
      {
        name: 'wikilink',
        level: 'inline',
        start(src: string) {
          return src.indexOf('[[');
        },
        tokenizer(src: string) {
          const match = /^\[\[([^\]]+)\]\]/.exec(src);
          if (!match) return undefined;
          return { type: 'wikilink', raw: match[0], inner: match[1] };
        },
        renderer(token) {
          const inner = String((token as unknown as { inner: string }).inner);
          const [targetPart, alias] = splitOnce(inner, '|');
          const [target, anchor] = splitOnce(targetPart, '#');
          const label = alias ?? (anchor ? `${target.trim()} › ${anchor}` : target.trim());
          const dst = target.trim() ? ctx.resolve(target.trim()) : null;
          if (!target.trim() && anchor) {
            // [[#heading]] — same-note anchor.
            return `<a class="internal-link" href="#h-${esc(slugifyHeading(anchor))}">${esc(label)}</a>`;
          }
          if (dst) {
            const anchorAttr = anchor ? ` data-anchor="${esc(anchor)}"` : '';
            return `<a class="internal-link" data-path="${esc(dst)}"${anchorAttr}>${esc(label)}</a>`;
          }
          return `<a class="internal-link unresolved" data-raw="${esc(target.trim())}" data-unresolved="1">${esc(label)}</a>`;
        },
      },
      {
        name: 'vaultTag',
        level: 'inline',
        start(src: string) {
          return src.indexOf('#');
        },
        tokenizer(src: string, tokens: unknown[]) {
          // Only when preceded by start/whitespace/( — marked gives us the
          // remaining src, so check the previous token's tail via lookbehind
          // being unavailable: require the # at position 0 of src (marked
          // splits inline text at `start()` hits).
          const match = /^#([\p{L}\p{N}_][\p{L}\p{N}_\-/]*)/u.exec(src);
          if (!match) return undefined;
          if (!/\p{L}/u.test(match[1])) return undefined; // needs a letter
          void tokens;
          return { type: 'vaultTag', raw: match[0], tag: match[1] };
        },
        renderer(token) {
          const tag = String((token as unknown as { tag: string }).tag);
          return `<span class="tag" data-tag="${esc(tag)}">#${esc(tag)}</span>`;
        },
      },
    ],
    renderer: {
      heading({ tokens, depth }) {
        const text = this.parser.parseInline(tokens);
        const plain = text.replace(/<[^>]*>/g, '');
        return `<h${depth} id="h-${esc(slugifyHeading(plain))}">${text}</h${depth}>\n`;
      },
      code({ text, lang }) {
        const l = (lang || '').trim().split(/\s+/)[0] || null;
        const diagram = diagramKind(l?.toLowerCase() ?? null, text);
        if (diagram) {
          // Source travels as text content (survives the sanitizer); the
          // post-render pass swaps it for the rendered SVG.
          return `<div class="diagram-block" data-diagram="${diagram}"><pre class="diagram-src">${esc(text)}</pre></div>\n`;
        }
        const hlLang = l && langFromPath(`x.${l}`) ? l : l;
        const lines = text.split('\n').map((line) => highlightLine(line, hlLang)).join('\n');
        return `<pre><code class="hljs">${lines}</code></pre>\n`;
      },
      blockquote({ tokens }) {
        const body = this.parser.parse(tokens);
        const m = /^<p>\[!(\w+)\]([+-])?\s*/.exec(body);
        if (m) {
          const kind = m[1].toLowerCase();
          const rest = body.slice(0, m.index) + '<p>' + body.slice(m.index + m[0].length);
          return `<blockquote class="callout callout-${esc(kind)}"><div class="callout-title">${esc(m[1])}</div>${rest}</blockquote>\n`;
        }
        return `<blockquote>${body}</blockquote>\n`;
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens);
        const t = title ? ` title="${esc(title)}"` : '';
        if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith('//')) {
          return `<a href="${esc(href)}"${t} target="_blank" rel="noopener noreferrer">${text}</a>`;
        }
        if (href.startsWith('#')) {
          return `<a href="${esc(href)}"${t}>${text}</a>`;
        }
        // Local note/asset link.
        const [pathPart, anchor] = splitOnce(href, '#');
        const raw = decodeURIComponentSafe(pathPart);
        const dst = ctx.resolve(raw);
        if (dst) {
          const anchorAttr = anchor ? ` data-anchor="${esc(anchor)}"` : '';
          return `<a class="internal-link" data-path="${esc(dst)}"${anchorAttr}${t}>${text}</a>`;
        }
        return `<a class="internal-link unresolved" data-raw="${esc(raw)}" data-unresolved="1"${t}>${text}</a>`;
      },
      image({ href, text }) {
        const raw = decodeURIComponentSafe(href);
        if (/^https?:\/\//i.test(href)) {
          return `<img src="${esc(href)}" alt="${esc(text)}" loading="lazy" />`;
        }
        const dst = ctx.resolve(raw) ?? raw;
        const url = ctx.assetUrl(dst);
        return url
          ? `<img src="${esc(url)}" alt="${esc(text)}" loading="lazy" />`
          : `<span class="embed-pending" data-path="${esc(dst)}">${esc(text || raw)}</span>`;
      },
    },
  });
  return m;
}

function splitOnce(s: string, sep: string): [string, string | null] {
  const i = s.indexOf(sep);
  return i === -1 ? [s, null] : [s.slice(0, i), s.slice(i + 1)];
}

function decodeURIComponentSafe(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Render a note body (raw markdown WITHOUT frontmatter) to sanitized HTML. */
export function renderNote(md: string, ctx: RenderCtx): string {
  void ensureHljs(); // fire the lazy load; re-render picks it up
  const cleaned = stripComments(md);
  const html = makeMarked(ctx).parse(cleaned, { async: false }) as string;
  return sanitizeHtml(html);
}
