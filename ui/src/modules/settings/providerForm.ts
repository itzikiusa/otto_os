// Custom-provider sheet (Settings → Agents → Providers) ⇄ the `providers`
// settings entry. Pure so the Edit → Save round-trip is unit-tested.
//
// The arg lists are argv arrays (`ProviderOverride` in otto-sessions'
// providers.rs), so the text fields use shell-style words: an element with a
// space is shown quoted and read back as ONE element. A plain whitespace
// join/split turned `["--system", "be terse"]` into three args on the first
// Edit → Save, and typed quotes reached the CLI literally.

export interface ProviderDef {
  cmd: string;
  args?: string[];
  resume_args?: string[] | null;
  update_command?: string | null;
  /** Model-flag template, `{model}` substituted at spawn (e.g. `--model {model}`).
   *  Unset = the CLI takes no model flag; pickers hide the model control. */
  model_args?: string[] | null;
}

export interface ProviderFormValues {
  cmd: string;
  args: string;
  resumeArgs: string;
  updateCmd: string;
  modelArgs: string;
}

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

export function providerToForm(p: ProviderDef): ProviderFormValues {
  return {
    cmd: p.cmd,
    args: joinArgs(p.args),
    resumeArgs: joinArgs(p.resume_args),
    updateCmd: p.update_command ?? '',
    modelArgs: joinArgs(p.model_args),
  };
}

/** The entry to save. Keys the sheet doesn't edit survive from `prev` (the
 *  entry being edited); a cleared optional field removes its key. */
export function formToProvider(f: ProviderFormValues, prev?: ProviderDef): ProviderDef {
  const def: ProviderDef = { ...(prev ?? {}), cmd: f.cmd.trim(), args: splitArgs(f.args) };
  const ra = splitArgs(f.resumeArgs);
  if (ra.length) def.resume_args = ra;
  else delete def.resume_args;
  const uc = f.updateCmd.trim();
  if (uc !== '') def.update_command = uc;
  else delete def.update_command;
  const ma = splitArgs(f.modelArgs);
  if (ma.length) def.model_args = ma;
  else delete def.model_args;
  return def;
}
