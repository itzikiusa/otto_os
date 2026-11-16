// Shell-style words for argv text fields (provider args, run configuration args/build flags):
// an element with a space is shown quoted and read back as ONE element.

/** One argv element as a shell word: bare when it has nothing to quote,
 *  otherwise single-quoted (`'` written as `'\''`). */
export function quoteArg(a: string): string {
  if (a !== '' && /^[^\s'"\\]+$/.test(a)) return a;
  return `'${a.replace(/'/g, `'\\''`)}'`;
}

export function joinArgs(list: readonly string[] | null | undefined): string {
  return (list ?? []).map(quoteArg).join(' ');
}

/** Shell-style word split: whitespace separates words; '…' is literal;
 *  "…" keeps spaces with `\"` / `\\` escapes; a backslash outside quotes
 *  escapes the next character. An unclosed quote runs to the end. */
export function splitArgs(line: string): string[] {
  const out: string[] = [];
  let cur = '';
  let has = false;
  let quote: '' | "'" | '"' = '';
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quote === "'") {
      if (c === "'") quote = '';
      else cur += c;
    } else if (quote === '"') {
      if (c === '"') quote = '';
      else if (c === '\\' && (line[i + 1] === '"' || line[i + 1] === '\\')) cur += line[++i];
      else cur += c;
    } else if (c === "'" || c === '"') {
      quote = c;
      has = true;
    } else if (c === '\\' && i + 1 < line.length) {
      cur += line[++i];
      has = true;
    } else if (/\s/.test(c)) {
      if (has) out.push(cur);
      cur = '';
      has = false;
    } else {
      cur += c;
      has = true;
    }
  }
  if (has) out.push(cur);
  return out;
}
