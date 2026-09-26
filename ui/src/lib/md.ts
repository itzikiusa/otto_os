// Tiny markdown renderer (headings, bold/italic, inline code, fenced code,
// links, lists, blockquotes, paragraphs). Output is HTML-escaped first, so it
// is safe to inject with {@html}.
//
// `renderMarkdownGfm` is the full-fidelity variant (GFM tables, task lists,
// nested lists) for document-style previews — agent reports, SKILL.md — run
// through the allowlist sanitizer so it is equally safe for {@html}.

import { marked } from 'marked';
import { sanitizeHtml } from './sanitize';

/** GFM markdown → sanitized HTML. Never throws: a parse failure falls back to
 *  the escaped tiny renderer. */
export function renderMarkdownGfm(md: string): string {
  try {
    const html = sanitizeHtml(marked.parse(md ?? '', { async: false, gfm: true, breaks: false }) as string);
    // WebKit does not make overflow containers keyboard-focusable by default.
    // Add trusted attributes after sanitizing, so wide code/tables can be
    // reached with Tab and scrolled with arrow keys without a pointer.
    const document = new DOMParser().parseFromString(html, 'text/html');
    for (const block of document.querySelectorAll('pre, table')) block.setAttribute('tabindex', '0');
    return document.body.innerHTML;
  } catch {
    return renderMarkdown(md);
  }
}

function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function inline(s: string): string {
  return s
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/__([^_]+)__/g, '<strong>$1</strong>')
    .replace(/\*([^*]+)\*/g, '<em>$1</em>')
    .replace(
      /\[([^\]]+)\]\((https?:[^)\s]+)\)/g,
      '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
    );
}

export function renderMarkdown(md: string): string {
  const lines = esc(md ?? '').split('\n');
  const out: string[] = [];
  let inCode = false;
  let listKind: 'ul' | 'ol' | null = null;
  let para: string[] = [];

  const flushPara = () => {
    if (para.length > 0) {
      out.push(`<p>${inline(para.join(' '))}</p>`);
      para = [];
    }
  };
  const closeList = () => {
    if (listKind) {
      out.push(`</${listKind}>`);
      listKind = null;
    }
  };

  for (const line of lines) {
    if (line.startsWith('```')) {
      flushPara();
      closeList();
      if (inCode) {
        out.push('</code></pre>');
        inCode = false;
      } else {
        out.push('<pre tabindex="0"><code>');
        inCode = true;
      }
      continue;
    }
    if (inCode) {
      out.push(`${line}\n`);
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushPara();
      closeList();
      const level = heading[1].length;
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }

    const ul = line.match(/^\s*[-*]\s+(.*)$/);
    const ol = line.match(/^\s*\d+\.\s+(.*)$/);
    if (ul || ol) {
      flushPara();
      const kind = ul ? 'ul' : 'ol';
      if (listKind !== kind) {
        closeList();
        out.push(`<${kind}>`);
        listKind = kind;
      }
      out.push(`<li>${inline((ul ?? ol)![1])}</li>`);
      continue;
    }

    if (line.startsWith('&gt;')) {
      flushPara();
      closeList();
      out.push(`<blockquote>${inline(line.slice(4).trim())}</blockquote>`);
      continue;
    }

    if (line.trim() === '') {
      flushPara();
      closeList();
      continue;
    }

    para.push(line.trim());
  }
  flushPara();
  closeList();
  if (inCode) out.push('</code></pre>');
  return out.join('');
}
