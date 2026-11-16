// Pretty-print JSON TEXT by re-indenting it, not by `JSON.parse` →
// `JSON.stringify`: strings and numbers are copied verbatim, so a big id
// (`12345678901234567890`), a trailing zero (`1.10`) or an out-of-range
// number (`1e400`) survive. Validate with `JSON.parse` first; this only moves
// whitespace.

/** `text` re-indented with `indent` spaces per level. */
export function reindentJson(text: string, indent = 2): string {
  const pad = (depth: number) => '\n' + ' '.repeat(indent * depth);
  let out = '';
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      const start = i;
      for (i++; i < text.length && text[i] !== '"'; i++) if (text[i] === '\\') i++;
      out += text.slice(start, i + 1);
    } else if (c === '{' || c === '[') {
      // An empty container stays on one line: `{}` / `[]`.
      let j = i + 1;
      while (/\s/.test(text[j] ?? '')) j++;
      if (text[j] === (c === '{' ? '}' : ']')) {
        out += c + text[j];
        i = j;
      } else out += c + pad(++depth);
    } else if (c === '}' || c === ']') out += pad(--depth) + c;
    else if (c === ',') out += ',' + pad(depth);
    else if (c === ':') out += ': ';
    else if (!/\s/.test(c)) out += c;
  }
  return out;
}
