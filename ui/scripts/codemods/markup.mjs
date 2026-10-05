// Tiny Svelte-markup helpers shared by the a11y codemods and ui-guards. Not a
// parser: <script>, <style> and comments are blanked (offsets kept) and start
// tags are found by name, skipping quoted strings and {…} expressions.

const blank = (m) => m.replace(/[^\n]/g, ' ');

/** The file with script/style/comments blanked — offsets match the original. */
export function markupOf(text) {
  return text
    .replace(/<script\b[\s\S]*?<\/script[^>]*>/gi, blank)
    .replace(/<style\b[\s\S]*?<\/style[^>]*>/gi, blank)
    .replace(/<!--[\s\S]*?-->/g, blank);
}

/** Index of the `>` closing the start tag that begins at `i` (just past the name). */
export function tagEnd(text, i) {
  let depth = 0;
  let q = '';
  for (; i < text.length; i++) {
    const c = text[i];
    if (q) {
      if (c === q) q = '';
    } else if ((c === '"' || c === "'") && depth === 0) q = c;
    else if (c === '`' && depth > 0) q = c;
    else if (c === '{') depth++;
    else if (c === '}') depth = Math.max(0, depth - 1);
    else if (c === '>' && depth === 0) return i;
  }
  return -1;
}

/** Every start tag named in `names`: { tag, start, nameEnd, end, attrs }. */
export function startTags(text, names, markup = markupOf(text)) {
  const out = [];
  const re = new RegExp(`<(${names.join('|')})(?=[\\s>/])`, 'g');
  for (const m of markup.matchAll(re)) {
    const nameEnd = m.index + 1 + m[1].length;
    const end = tagEnd(markup, nameEnd);
    if (end === -1) continue;
    out.push({ tag: m[1], start: m.index, nameEnd, end, attrs: markup.slice(nameEnd, end) });
  }
  return out;
}

/** A static attribute's value (`name="v"`, `name='v'`, `name={"v"}` / `name={v}`
 *  as raw text), or null when absent. */
export function attrValue(attrs, name) {
  const esc = name.replace(/[:.]/g, '\\$&');
  const m = new RegExp(`(?:^|\\s)${esc}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|\\{([^}]*)\\})`).exec(attrs);
  if (!m) return new RegExp(`(?:^|\\s)\\{${esc}\\}`).test(attrs) ? name : null;
  return m[1] ?? m[2] ?? m[3] ?? '';
}
