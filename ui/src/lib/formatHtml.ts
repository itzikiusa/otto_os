// Display-only HTML pretty-printer for viewers (vault Source view). Agents
// emit mockups as one minified line; this re-indents tags + <style> CSS so the
// source is readable. It never feeds back into the file — best-effort only.

const VOID = new Set([
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
/** Content copied verbatim (script/textarea) or CSS-formatted (style). */
const RAW = new Set(['script', 'style', 'textarea', 'pre']);
/** Kept inline with their text ("<b>x</b>" stays one line when short). */
const INLINE = new Set([
  'a',
  'abbr',
  'b',
  'code',
  'em',
  'i',
  'kbd',
  'label',
  'small',
  'span',
  'strong',
  'sub',
  'sup',
  'u',
]);

/** Index just past the tag's closing '>', honoring quoted attribute values. */
function tagEnd(src: string, start: number): number {
  let q = '';
  for (let i = start + 1; i < src.length; i++) {
    const c = src[i];
    if (q) {
      if (c === q) q = '';
    } else if (c === '"' || c === "'") q = c;
    else if (c === '>') return i + 1;
  }
  return src.length;
}

/** Naive CSS re-indent: break after { ; } outside parens/quotes. */
function formatCss(css: string, baseIndent: string, indent: string): string {
  const out: string[] = [];
  let line = '';
  let depth = 0;
  let paren = 0;
  let q = '';
  const flush = () => {
    const t = line.trim();
    if (t) out.push(baseIndent + indent.repeat(Math.max(0, depth)) + t);
    line = '';
  };
  for (const c of css) {
    if (q) {
      line += c;
      if (c === q) q = '';
      continue;
    }
    if (c === '"' || c === "'") {
      q = c;
      line += c;
    } else if (c === '(') {
      paren++;
      line += c;
    } else if (c === ')') {
      paren = Math.max(0, paren - 1);
      line += c;
    } else if (paren === 0 && c === '{') {
      line += ' {';
      flush();
      depth++;
    } else if (paren === 0 && c === '}') {
      flush();
      depth = Math.max(0, depth - 1);
      line = '}';
      flush();
    } else if (paren === 0 && c === ';') {
      line += ';';
      flush();
    } else if (c === '\n') {
      flush();
    } else line += c;
  }
  flush();
  return out.join('\n');
}

/**
 * Pretty-print HTML for display. Best-effort tokenizer — on any surprise the
 * caller should fall back to the raw text (wrap in try/catch).
 */
export function formatHtml(src: string, indent = '  '): string {
  const out: string[] = [];
  let depth = 0;
  const pad = () => indent.repeat(Math.max(0, depth));
  const emit = (s: string) => {
    const t = s.trim();
    if (t) out.push(pad() + t);
  };

  let i = 0;
  const n = src.length;
  while (i < n) {
    const lt = src.indexOf('<', i);
    if (lt === -1) {
      emit(src.slice(i));
      break;
    }
    if (lt > i) emit(src.slice(i, lt).replace(/\s+/g, ' '));

    if (src.startsWith('<!--', lt)) {
      const end = src.indexOf('-->', lt);
      const stop = end === -1 ? n : end + 3;
      emit(src.slice(lt, stop));
      i = stop;
      continue;
    }

    const end = tagEnd(src, lt);
    const tag = src.slice(lt, end);
    const m = /^<\s*(\/?)\s*([a-zA-Z][a-zA-Z0-9-]*)/.exec(tag);
    if (!m) {
      // doctype, CDATA, or malformed — emit as-is.
      emit(tag);
      i = end;
      continue;
    }
    const closing = m[1] === '/';
    const name = m[2].toLowerCase();
    const selfClosed = /\/>$/.test(tag.trim()) || VOID.has(name);

    if (closing) {
      depth = Math.max(0, depth - 1);
      emit(tag);
      i = end;
      continue;
    }

    // Raw-content elements: copy verbatim to the matching close tag; <style>
    // additionally gets CSS re-indented (that's where minified mockups hurt).
    if (RAW.has(name) && !selfClosed) {
      const closeRe = new RegExp(`</\\s*${name}\\s*>`, 'i');
      const rest = src.slice(end);
      const cm = closeRe.exec(rest);
      const inner = cm ? rest.slice(0, cm.index) : rest;
      const closeTag = cm ? cm[0] : '';
      emit(tag);
      if (inner.trim()) {
        if (name === 'style') {
          out.push(formatCss(inner, pad() + indent, indent));
        } else if (name === 'pre' || name === 'textarea') {
          // Whitespace-significant — keep exactly as written.
          out.push(inner.replace(/^\n|\n$/g, ''));
        } else {
          for (const line of inner.replace(/^\n+|\n+$/g, '').split('\n')) out.push(line);
        }
      }
      if (closeTag) emit(closeTag);
      i = end + inner.length + closeTag.length;
      continue;
    }

    // Short inline element with plain text content → keep on one line.
    if (INLINE.has(name) && !selfClosed) {
      const rest = src.slice(end);
      const cm = new RegExp(`^([^<]{0,80})</\\s*${name}\\s*>`, 'i').exec(rest);
      if (cm) {
        emit(tag + cm[1].replace(/\s+/g, ' ') + `</${name}>`);
        i = end + cm[0].length;
        continue;
      }
    }

    emit(tag);
    if (!selfClosed) depth++;
    i = end;
  }
  return out.join('\n');
}
