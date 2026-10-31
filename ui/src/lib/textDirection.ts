// Unicode word-counted BiDi direction detector + per-block HTML isolator.
//
// Why not browser `dir="auto"` alone?
// HTML's `dir="auto"` inspects ONLY the first strong directional character of a
// container. A single leading technical token (`Composer.svelte — בדוק את הקובץ`,
// `1. API: הוסף תמיכה ב-RTL`, or an English bullet at the top of a container)
// flips an entire Hebrew/Arabic paragraph or thread to LTR. Conversely, a single
// Hebrew word quoted at the start of an English paragraph flips it to RTL.
//
// Inspired by T3 Code's BiDi layout:
//   1. Strip code blocks, inline code, HTML `<pre>`/`<code>` spans, tags,
//      markdown link URLs, bare URLs, `@file` mentions, and file paths BEFORE
//      counting directional words so technical syntax never skews prose.
//   2. Count strong RTL words vs strong LTR words across the prose.
//   3. Stamp explicit `dir="rtl"` / `dir="ltr"` per HTML prose block while
//      isolating every `<pre>` and `<code>` as `dir="ltr" style="unicode-bidi: isolate"`.

export type TextDirection = 'rtl' | 'ltr' | 'auto';

/** Strong RTL scripts: Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan, Mandaic,
 *  and Hebrew/Arabic presentation forms. */
const RTL_CHAR_RE = /[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFF]/;

/** Strong LTR scripts: ASCII Latin + Latin-1/Extended letters (excluding `×`/`÷`). */
const LTR_CHAR_RE = /[A-Za-z\u00C0-\u00D6\u00D8-\u00F6\u00F8-\u02B8]/;

/** Prose block tags that receive explicit `dir="rtl"` or `dir="ltr"`. */
const PROSE_BLOCK_TAGS = new Set([
  'p',
  'li',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'blockquote',
  'td',
  'th',
]);

/** Container tags whose inner text belongs to their own child blocks rather
 *  than to an enclosing `<li>` or `<blockquote>`. */
const CHILD_CONTAINER_TAGS = new Set(['ul', 'ol', 'table', 'thead', 'tbody', 'tr']);

/** HTML void elements (never push onto the open-tag stack). */
const VOID_TAGS = new Set([
  'area',
  'base',
  'br',
  'col',
  'embed',
  'hr',
  'img',
  'input',
  'link',
  'meta',
  'param',
  'source',
  'track',
  'wbr',
]);

/** Strip technical constructs that must not count as natural-language words. */
export function stripTechnicalSpans(input: string): string {
  if (!input) return '';
  return (
    input
      // Fenced code blocks (```...``` or ~~~...~~~, including unclosed live drafts).
      .replace(/```[\s\S]*?(?:```|$)/g, ' ')
      .replace(/~~~[\s\S]*?(?:~~~|$)/g, ' ')
      // HTML <pre>...</pre> and <code>...</code> blocks.
      .replace(/<pre\b[^>]*>[\s\S]*?<\/pre>/gi, ' ')
      .replace(/<code\b[^>]*>[\s\S]*?<\/code>/gi, ' ')
      // Multi-backtick and single-backtick inline code spans.
      .replace(/``[\s\S]*?``/g, ' ')
      .replace(/`[^`\n]+`/g, ' ')
      // Markdown links/images: keep human label, strip destination URL/path.
      .replace(/!?\[([^\]]*)\]\([^)]*\)/g, ' $1 ')
      // Remaining HTML tags and HTML character entities (&quot;, &amp;, &#39;, ...).
      .replace(/<[^>]+>/g, ' ')
      .replace(/&[a-zA-Z0-9#]+;/g, ' ')
      // Bare URLs (http/https/file/ws/wss/ftp/mailto).
      .replace(/\b(?:https?|ftp|file|ws|wss):\/\/\S+/gi, ' ')
      .replace(/\bmailto:\S+/gi, ' ')
      // Workspace @file mentions (e.g. @src/main.rs, @ui/src/App.svelte).
      .replace(/(?:^|(?<=[\s([{"']))@[A-Za-z0-9_./-]+/g, ' ')
      // Absolute / home / dot-relative paths (/foo/bar, ~/foo, ./bar, ../baz).
      .replace(/(?:^|(?<=[\s([{"']))(?:~\/|\.{1,2}\/|\/)[A-Za-z0-9_./@-]+/g, ' ')
      // Relative ASCII paths containing a slash (src/main.rs, crates/otto-core/lib.rs).
      .replace(/\b[A-Za-z0-9_.-]+\/[A-Za-z0-9_./@-]+/g, ' ')
  );
}

/**
 * Detect block text direction by counting strong RTL vs strong LTR words after
 * stripping code spans, HTML tags, URLs, and file paths.
 *
 * Returns:
 * - `'rtl'` when `rtlWords > 0` and (`rtlWords >= ltrWords * 0.35` or first strong word is RTL with `rtlWords >= ltrWords`)
 * - `'ltr'` when `ltrWords > 0`
 * - `'auto'` when neutral (numbers, punctuation, emoji, or code-only)
 */
export function detectBlockDirection(text: string): TextDirection {
  const cleaned = stripTechnicalSpans(text);
  if (!cleaned) return 'auto';

  const tokens = cleaned.split(/[\s\-–—/:,;!?()[\]{}"'«»„“”<>|+*=#~^\\]+/);
  let rtlWords = 0;
  let ltrWords = 0;
  let firstStrong: 'rtl' | 'ltr' | null = null;

  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i];
    if (!tok) continue;
    const hasRtl = RTL_CHAR_RE.test(tok);
    if (hasRtl) {
      rtlWords++;
      if (firstStrong === null) firstStrong = 'rtl';
      continue;
    }
    if (LTR_CHAR_RE.test(tok)) {
      ltrWords++;
      if (firstStrong === null) firstStrong = 'ltr';
    }
  }

  if (
    rtlWords > 0 &&
    (rtlWords >= ltrWords * 0.35 || (firstStrong === 'rtl' && rtlWords >= ltrWords))
  ) {
    return 'rtl';
  }
  if (ltrWords > 0) return 'ltr';
  return 'auto';
}

/** Resolve a concrete HTML `dir` attribute (`'rtl'` or `'ltr'`) for an element. */
export function resolveConcreteDirection(
  text: string,
  fallback: 'rtl' | 'ltr' = 'ltr',
): 'rtl' | 'ltr' {
  const dir = detectBlockDirection(text);
  return dir === 'auto' ? fallback : dir;
}

/** Set or replace `dir="..."` and optionally ensure `unicode-bidi: isolate` on an opening HTML tag. */
function rewriteOpeningTag(
  rawTag: string,
  dir: 'rtl' | 'ltr',
  isolateBidi: boolean,
): string {
  const selfClosing = /\/>\s*$/.test(rawTag);
  let body = rawTag.replace(/\s*\/?>\s*$/, '');

  if (/\sdir\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/i.test(body)) {
    body = body.replace(/\sdir\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/i, ` dir="${dir}"`);
  } else {
    body += ` dir="${dir}"`;
  }

  if (isolateBidi) {
    const styleMatch = body.match(/\sstyle\s*=\s*(?:"([^"]*)"|'([^']*)')/i);
    if (styleMatch) {
      const existing = (styleMatch[1] ?? styleMatch[2] ?? '').trim();
      if (!/unicode-bidi\s*:/i.test(existing)) {
        const sep = existing && !existing.endsWith(';') ? '; ' : existing ? ' ' : '';
        const merged = `${existing}${sep}unicode-bidi: isolate`;
        body = body.replace(
          /\sstyle\s*=\s*(?:"[^"]*"|'[^']*')/i,
          ` style="${merged}"`,
        );
      }
    } else {
      body += ' style="unicode-bidi: isolate"';
    }
  }

  return body + (selfClosing ? ' />' : '>');
}

interface StackFrame {
  tag: string;
  outIndex: number;
  rawOpenTag: string;
  ownText: string;
  firstChildDir: 'rtl' | 'ltr' | null;
  isProseBlock: boolean;
  isCodeLike: boolean;
  isChildContainer: boolean;
}

/**
 * Stamp explicit per-block `dir="rtl"` or `dir="ltr"` on every `<p>`, `<li>`,
 * `<h1>`–`<h6>`, `<blockquote>`, `<td>`, `<th>` in rendered HTML, while forcing
 * every `<pre>` and `<code>` to `dir="ltr" style="unicode-bidi: isolate"`.
 *
 * Pure string transform (safe in both SSR/Node unit tests and browser rendering).
 */
export function applyHtmlBlockDirections(
  html: string,
  defaultDir: 'rtl' | 'ltr' = 'ltr',
): string {
  if (!html) return html;

  const out: string[] = [];
  const stack: StackFrame[] = [];
  let codeDepth = 0;

  const tagRe = /<(\/?)([a-zA-Z][a-zA-Z0-9]*)\b([^>]*)>/g;
  let lastIdx = 0;
  let match: RegExpExecArray | null;

  const appendTextToNearestBlock = (chunk: string) => {
    if (!chunk || codeDepth > 0) return;
    // Walk down from the innermost open tag: stop at the nearest prose block
    // unless a child-container (`<ul>`, `<ol>`, `<table>`) sits in between.
    for (let i = stack.length - 1; i >= 0; i--) {
      const f = stack[i]!;
      if (f.isCodeLike || f.isChildContainer) return;
      if (f.isProseBlock) {
        f.ownText += ' ' + chunk;
        return;
      }
    }
  };

  while ((match = tagRe.exec(html)) !== null) {
    if (match.index > lastIdx) {
      const textChunk = html.slice(lastIdx, match.index);
      out.push(textChunk);
      appendTextToNearestBlock(textChunk);
    }
    lastIdx = tagRe.lastIndex;

    const fullTag = match[0];
    const isClosing = match[1] === '/';
    const tag = match[2]!.toLowerCase();
    const rest = match[3] ?? '';
    const selfClosing = !isClosing && (VOID_TAGS.has(tag) || /\/\s*$/.test(rest));

    if (!isClosing) {
      const isCodeLike = tag === 'pre' || tag === 'code';
      const isProseBlock = codeDepth === 0 && PROSE_BLOCK_TAGS.has(tag);
      const isChildContainer = codeDepth === 0 && CHILD_CONTAINER_TAGS.has(tag);

      if (isCodeLike) {
        out.push(rewriteOpeningTag(fullTag, 'ltr', true));
        if (!selfClosing) {
          codeDepth++;
          stack.push({
            tag,
            outIndex: out.length - 1,
            rawOpenTag: fullTag,
            ownText: '',
            firstChildDir: null,
            isProseBlock: false,
            isCodeLike: true,
            isChildContainer: false,
          });
        }
      } else {
        const outIndex = out.length;
        out.push(fullTag);
        if (!selfClosing) {
          stack.push({
            tag,
            outIndex,
            rawOpenTag: fullTag,
            ownText: '',
            firstChildDir: null,
            isProseBlock,
            isCodeLike: false,
            isChildContainer,
          });
        }
      }
    } else {
      out.push(fullTag);
      // Pop matching frame if present.
      let matchIdx = -1;
      for (let i = stack.length - 1; i >= 0; i--) {
        if (stack[i]!.tag === tag) {
          matchIdx = i;
          break;
        }
      }
      if (matchIdx !== -1) {
        while (stack.length > matchIdx) {
          const popped = stack.pop()!;
          if (popped.isCodeLike) {
            codeDepth = Math.max(0, codeDepth - 1);
          } else if (popped.isProseBlock) {
            const detected = detectBlockDirection(popped.ownText);
            const resolved: 'rtl' | 'ltr' =
              detected !== 'auto'
                ? detected
                : (popped.firstChildDir ?? defaultDir);
            out[popped.outIndex] = rewriteOpeningTag(popped.rawOpenTag, resolved, false);
            // Propagate to enclosing prose block (e.g. <blockquote><p>...</p></blockquote>
            // or loose <li><p>...</p></li>) if it had no direct prose of its own yet.
            for (let p = stack.length - 1; p >= 0; p--) {
              if (stack[p]!.isProseBlock) {
                if (stack[p]!.firstChildDir === null) {
                  stack[p]!.firstChildDir = resolved;
                }
                break;
              }
            }
          }
        }
      }
    }
  }

  if (lastIdx < html.length) {
    const tail = html.slice(lastIdx);
    out.push(tail);
    appendTextToNearestBlock(tail);
  }

  // Finalize any unclosed prose blocks gracefully (e.g. truncated HTML).
  while (stack.length > 0) {
    const popped = stack.pop()!;
    if (popped.isProseBlock) {
      const detected = detectBlockDirection(popped.ownText);
      const resolved: 'rtl' | 'ltr' =
        detected !== 'auto' ? detected : (popped.firstChildDir ?? defaultDir);
      out[popped.outIndex] = rewriteOpeningTag(popped.rawOpenTag, resolved, false);
    }
  }

  return out.join('');
}

/** DOM helper: apply per-block `dir` and `<pre>`/`<code>` isolation in place on a root element. */
export function applyDomBlockDirections(
  root: HTMLElement,
  defaultDir: 'rtl' | 'ltr' = 'ltr',
): void {
  const codeEls = root.querySelectorAll<HTMLElement>('pre, code');
  for (let i = 0; i < codeEls.length; i++) {
    const el = codeEls[i]!;
    el.setAttribute('dir', 'ltr');
    el.style.unicodeBidi = 'isolate';
  }
  const blocks = root.querySelectorAll<HTMLElement>(
    'p, li, h1, h2, h3, h4, h5, h6, blockquote, td, th',
  );
  for (let i = 0; i < blocks.length; i++) {
    const el = blocks[i]!;
    if (el.closest('pre, code')) continue;
    const clone = el.cloneNode(true) as HTMLElement;
    const nested = clone.querySelectorAll('pre, code, ul, ol, table');
    for (let j = 0; j < nested.length; j++) nested[j]!.remove();
    const detected = detectBlockDirection(clone.textContent ?? '');
    if (detected !== 'auto') {
      el.setAttribute('dir', detected);
    } else {
      const childWithDir = el.querySelector<HTMLElement>('[dir="rtl"], [dir="ltr"]');
      const childDir = childWithDir?.getAttribute('dir');
      el.setAttribute('dir', childDir === 'rtl' ? 'rtl' : defaultDir);
    }
  }
}
