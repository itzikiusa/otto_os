// Custom-provider sheet (Settings → Agents → Providers) ⇄ the `providers`
// settings entry. Pure so the Edit → Save round-trip is unit-tested.
//
// The arg lists are argv arrays (`ProviderOverride` in otto-sessions'
// providers.rs), so the text fields use shell-style words: an element with a
// space is shown quoted and read back as ONE element. A plain whitespace
// join/split turned `["--system", "be terse"]` into three args on the first
// Edit → Save, and typed quotes reached the CLI literally.

import { joinArgs, splitArgs } from '../../lib/shellArgs.ts';

export { joinArgs, quoteArg, splitArgs } from '../../lib/shellArgs.ts';

export interface ProviderDef {
  cmd: string;
  args?: string[];
  resume_args?: string[] | null;
  update_command?: string | null;
  /** Model-flag template, `{model}` substituted at spawn (e.g. `--model {model}`).
   *  Unset = default/no model flag; `[]` = explicitly opt out of inherited model flags. */
  model_args?: string[] | null;
  /** Canonical CLI model family (`claude`, `codex`, `agy`, `gemini`, `grok`, `opencode`, `pi`). */
  model_family?: string | null;
}

export interface ProviderFormValues {
  cmd: string;
  args: string;
  resumeArgs: string;
  updateCmd: string;
  modelArgs: string;
  modelFamily?: string;
}

export function providerToForm(p: ProviderDef): ProviderFormValues {
  return {
    cmd: p.cmd,
    args: joinArgs(p.args),
    resumeArgs: joinArgs(p.resume_args),
    updateCmd: p.update_command ?? '',
    modelArgs: joinArgs(p.model_args),
    ...(p.model_family !== undefined ? { modelFamily: p.model_family ?? '' } : {}),
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
  else if (Array.isArray(prev?.model_args) && prev.model_args.length === 0) def.model_args = [];
  else delete def.model_args;
  if (f.modelFamily !== undefined) {
    const mf = f.modelFamily.trim().toLowerCase();
    if (mf !== '') def.model_family = mf;
    else delete def.model_family;
  }
  return def;
}
