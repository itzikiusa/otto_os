// How an MCP server reaches agents — the one "Runs" choice the Control Plane's
// Servers list shows for every server (docs/features/mcp-control-plane.md,
// "Native and governed servers"). Pure, so the unit tests load it directly.
//
//   native   — the agent CLI launches it from `.mcp.json`; its tools keep their
//              own names; Otto applies no policy, approval or audit.
//   governed — Otto's gateway runs it (policies, approvals, audit); agents see
//              its tools as `mcp__<server>__<tool>`.
//
// The daemon's `managed` flag IS the mode (one row, never both).

export type McpRunMode = 'native' | 'governed';

/** The mode a server row runs in. */
export function serverRuns(s: { managed: boolean }): McpRunMode {
  return s.managed ? 'governed' : 'native';
}

export const RUNS_LABEL: Record<McpRunMode, string> = {
  native: 'Native',
  governed: 'Governed',
};

/** The badge's one-line explanation (a native row's tooltip and form note). */
export const NOT_GOVERNED_NOTE =
  'Not governed: the agent CLI runs it directly, so no policy, approval or audit applies.';

/** Where Settings ▸ MCP servers (`#/settings/mcp-servers`) lands now. */
export const SERVERS_ROUTE = 'mcp/servers';

/** Whether a server can switch to `to` at all (an http server has no command
 *  for an agent CLI to launch). Null = allowed, else the reason. */
export function switchBlocked(s: { transport: string; managed: boolean }, to: McpRunMode): string | null {
  if (serverRuns(s) === to) return 'It already runs this way.';
  if (to === 'native' && s.transport !== 'stdio') return 'Only a stdio server can run natively: the agent CLI launches a command.';
  return null;
}

/** The confirm for a switch: exactly what changes, in plain words. */
export function switchConfirm(name: string, to: McpRunMode): { title: string; message: string; confirmLabel: string } {
  if (to === 'governed') {
    return {
      title: 'Run through Otto’s gateway',
      confirmLabel: 'Switch to governed',
      message:
        `“${name}” stops being written into each session’s .mcp.json. Otto’s gateway runs it instead, under your policies, approvals and audit. ` +
        `Agents then see its tools under different names (mcp__${name}__<tool>), so skills and prompts that name the native tools must be updated. ` +
        'Its secret values stay in the Keychain. Sessions already running keep the native copy until they restart.',
    };
  }
  return {
    title: 'Run natively',
    confirmLabel: 'Switch to native',
    message:
      `“${name}” is written into each new session’s .mcp.json and launched by the agent CLI under its own tool names. ` +
      'No policy, approval or audit applies to it, and every agent session in this workspace gets it: its per-person access rules are kept but not enforced until you switch back. ' +
      'Skills and prompts that name its governed tools (mcp__…) must be updated.',
  };
}

/** One value per non-empty, trimmed line. */
export function nonEmptyLines(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
}

/** `KEY=value` lines → a record (first `=` splits). `allowEmpty` keeps `KEY=`
 *  lines (the secret editor's "keep the stored value" sentinel). */
export function parseEnvLines(text: string, allowEmpty = false): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of nonEmptyLines(text)) {
    const eq = line.indexOf('=');
    if (eq <= 0) continue;
    const v = line.slice(eq + 1).trim();
    if (v === '' && !allowEmpty) continue;
    out[line.slice(0, eq).trim()] = v;
  }
  return out;
}

/** The `secret_env` a native save sends: typed values replace, a bare `KEY=`
 *  for a stored key keeps it (`''`, the server's sentinel), and a key left out
 *  is removed. On create nothing is stored, so bare keys are dropped. */
export function secretEnvBody(text: string, storedKeys: readonly string[], editing: boolean): Record<string, string> {
  const typed = parseEnvLines(text, true);
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(typed)) {
    if (v !== '') out[k] = v;
    else if (editing && storedKeys.includes(k)) out[k] = '';
  }
  return out;
}
