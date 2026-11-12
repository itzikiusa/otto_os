// "Diagnose with an agent" for a connection that does not connect — the pure
// half: what the consent dialog says, the exact checks the agent may run, the
// agent's opening prompt, and which existing diagnosis thread to reuse.
// Import-free (types only) so every rule is unit-tested
// (unit/connectionsDiagnose.test.ts). The stateful half is diagnose.svelte.ts.
//
// SECRET RULE: only an allowlist of connection fields is ever read here —
// never `secret`, a password, a connection string (it can carry one), or a
// pasted certificate / private key (TLS fields accept PEM). File PATHS are
// facts the agent needs; file CONTENTS are not.

import type { Connection, TestConnectionResp } from '../../lib/api/types';

/** What the failure looked like: a Test result, or a DB connect error. */
export interface DiagnoseProbe {
  message: string;
  hint?: string | null;
  detail?: string | null;
}

/** The thread's `source` (agent T's contract) and its origin. The daemon
 *  refuses an origin label over 200 characters (400). */
export const ORIGIN_LABEL_MAX = 200;
export const originLabel = (name: string): string => Array.from(name).slice(0, ORIGIN_LABEL_MAX).join('');
export const DIAGNOSE_SOURCE = 'connection-diagnose';

type Rec = Record<string, unknown>;
const rec = (v: unknown): Rec => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Rec) : {});
const str = (v: unknown): string => (typeof v === 'string' ? v.trim() : typeof v === 'number' ? String(v) : '');

/** A TLS field holds a file path or pasted PEM: name the path, never the PEM. */
function pathOnly(v: unknown): string {
  const s = str(v);
  if (!s) return '';
  return /-----BEGIN|\n/.test(s) || s.length > 512 ? '(pasted PEM — content not shared)' : s;
}

/** Hosts of a MongoDB URI, without its credentials or options. */
export function mongoHosts(uri: string): string {
  const m = /^mongodb(?:\+srv)?:\/\/(?:[^@/]*@)?([^/?]+)/i.exec(uri.trim());
  return m ? m[1] : '';
}

/** The network facts of a profile: where it points and how it gets there. */
export interface DiagnoseTarget {
  host: string;
  port: string;
  user: string;
  /** `-J` hops (ssh kind's `jump`, or the DB tunnel's `proxy_jump`). */
  jump: string;
  identityFile: string;
  /** DB kinds over an SSH tunnel: the tunnel's own endpoint. */
  tunnel: { host: string; port: string; user: string; identityFile: string; jump: string } | null;
  tlsMode: string;
  tlsVerify: boolean | null;
  tlsFiles: string[];
}

export function diagnoseTarget(c: Pick<Connection, 'kind' | 'params'>): DiagnoseTarget {
  const p = rec(c.params);
  const ssh = rec(p.ssh);
  const tls = rec(p.tls);
  const tunnelHost = str(ssh.host);
  const host = c.kind === 'mongodb' ? mongoHosts(str(p.conn_string)) : str(p.host);
  return {
    host,
    port: str(p.port) || (c.kind === 'ssh' ? '22' : ''),
    user: str(p.user),
    jump: str(p.jump),
    identityFile: str(p.identity_file),
    tunnel: tunnelHost
      ? { host: tunnelHost, port: str(ssh.port) || '22', user: str(ssh.user), identityFile: str(ssh.identity_file), jump: str(ssh.proxy_jump) }
      : null,
    tlsMode: str(tls.mode) || (p.secure === true ? 'required' : ''),
    tlsVerify: typeof tls.verify === 'boolean' ? tls.verify : null,
    tlsFiles: (['ca_cert', 'client_cert', 'client_key'] as const)
      .map((k) => (str(tls[k]) ? `${k}: ${pathOnly(tls[k])}` : ''))
      .filter(Boolean),
  };
}

/** The SSH endpoint the checks run against: the ssh profile itself, or a DB
 *  kind's tunnel; null for a direct DB connection. */
function sshEndpoint(c: Pick<Connection, 'kind' | 'params'>): { host: string; port: string; user: string; jump: string; identityFile: string } | null {
  const t = diagnoseTarget(c);
  if (c.kind === 'ssh') return t.host ? { host: t.host, port: t.port, user: t.user, jump: t.jump, identityFile: t.identityFile } : null;
  return t.tunnel;
}

/** Shell-quote one argument (paths and hosts come from the profile). */
const q = (s: string): string => (/^[\w@%+=:,./~-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);

/** The exact local, read-only commands the agent may run — shown in the
 *  consent dialog's "Show the checks" and repeated in the prompt. */
export function diagnoseChecks(c: Pick<Connection, 'kind' | 'params'>): string[] {
  const t = diagnoseTarget(c);
  const out: string[] = [];
  const reach = (host: string, port: string) => {
    if (!host) return;
    out.push(`dscacheutil -q host -a name ${q(host)}`, `dig +short ${q(host)}`);
    if (port) out.push(`nc -vz -w 5 ${q(host)} ${q(port)}`);
  };
  const ep = sshEndpoint(c);
  if (ep) {
    const dest = q(ep.user ? `${ep.user}@${ep.host}` : ep.host);
    const opts = `-p ${q(ep.port)}${ep.jump ? ` -J ${q(ep.jump)}` : ''}${ep.identityFile ? ` -i ${q(ep.identityFile)}` : ''}`;
    reach(ep.host, ep.port);
    out.push(
      `ssh -G ${opts} ${dest}`,
      // No login: only the "none" method is tried, host keys are never
      // written, and nothing ever prompts.
      `ssh -vvv -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=yes -o PreferredAuthentications=none ${opts} ${dest} exit`,
      `ssh-keygen -F ${q(ep.port === '22' ? ep.host : `[${ep.host}]:${ep.port}`)}`,
      'ssh-add -l',
    );
    if (ep.identityFile) out.push(`ls -l ${q(ep.identityFile)}`);
    out.push('cat ~/.ssh/config');
  }
  // A DB kind: the database endpoint itself (direct, or as seen past the tunnel).
  if (c.kind !== 'ssh' && c.kind !== 'custom' && !ep) reach(t.host.split(',')[0].replace(/:\d+$/, ''), t.port || (t.host.match(/:(\d+)$/)?.[1] ?? ''));
  return out;
}

/** "db.example.com:3306" / "bastion:22" — what the consent names. */
function endpointLabel(c: Pick<Connection, 'kind' | 'params'>): string {
  const ep = sshEndpoint(c);
  if (ep) return `${ep.host}:${ep.port}`;
  const t = diagnoseTarget(c);
  return t.port && !/:\d+$/.test(t.host) ? `${t.host}:${t.port}` : t.host || 'its host';
}

/** The consent dialog's plain statement of what will happen. */
export function diagnoseConsent(c: Pick<Connection, 'name' | 'kind' | 'params'>): string {
  const t = diagnoseTarget(c);
  const host = sshEndpoint(c)?.host || t.host || 'its host';
  return (
    `An agent will run read-only checks ON THIS MAC to find out why ${c.name} (${c.kind}, ${host}) does not connect: ` +
    `name resolution, reachability of ${endpointLabel(c)}, the SSH client’s verbose output, your SSH config for this host, ` +
    `the known-hosts entry, loaded keys and key-file permissions. It will not log in to the remote host, change any file, ` +
    `or see your password or private key. It asks you before anything else.`
  );
}

/** The agent's opening prompt. Facts + the probe + firm rules; no secrets. */
export function diagnosePrompt(
  c: Pick<Connection, 'name' | 'kind' | 'params' | 'environment' | 'read_only'>,
  probe: DiagnoseProbe | null,
): string {
  const t = diagnoseTarget(c);
  const facts = [
    `- Connection: ${c.name}`,
    `- Kind: ${c.kind}`,
    `- Environment: ${c.environment}${c.read_only ? ' (read-only profile)' : ''}`,
    t.host && `- Host: ${t.host}`,
    t.port && `- Port: ${t.port}`,
    t.user && `- User: ${t.user}`,
    t.jump && `- Jump host: ${t.jump}`,
    t.identityFile && `- Identity file (path): ${t.identityFile}`,
    t.tunnel &&
      `- SSH tunnel: ${t.tunnel.user ? `${t.tunnel.user}@` : ''}${t.tunnel.host}:${t.tunnel.port}` +
        `${t.tunnel.jump ? `, jump ${t.tunnel.jump}` : ''}${t.tunnel.identityFile ? `, identity file ${t.tunnel.identityFile}` : ''}`,
    t.tlsMode && `- TLS: ${t.tlsMode}${t.tlsVerify === false ? ', certificate NOT verified' : ''}${t.tlsFiles.length ? `; ${t.tlsFiles.join('; ')}` : ''}`,
  ].filter(Boolean);
  const failure = probe
    ? [
        `- Message: ${probe.message || '(none)'}`,
        probe.hint ? `- Hint: ${probe.hint}` : '',
        probe.detail ? `- Client output:\n\`\`\`\n${probe.detail}\n\`\`\`` : '',
      ].filter(Boolean)
    : ['- (no test result available)'];
  return [
    `Diagnose why the Otto connection "${c.name}" does not connect. Work on THIS Mac only.`,
    '',
    'Connection facts:',
    ...facts,
    '',
    'What the last connection test reported:',
    ...failure,
    '',
    'Rules — follow all of them:',
    '- Local, read-only diagnosis only. Use only these commands (adapt arguments, add nothing else):',
    ...diagnoseChecks(c).map((cmd) => `  - \`${cmd}\``),
    '- NEVER read the contents of a private key (only `ls -l` its path). Never print, ask for or guess the password.',
    '- Never run anything on the remote host; never log in to it.',
    '- Never edit anything under ~/.ssh and never remove a known-hosts entry. Explain the fix; the user applies it.',
    '- If the host key has CHANGED, say that this can mean someone is intercepting the connection, and stop.',
    '',
    'Finish with a short diagnosis: the cause, the evidence (the command output lines that show it), and the exact fix.',
  ].join('\n');
}

/** A diagnosis thread row, as the list returns it — `source` / `origin` may
 *  sit in `meta` (server-owned) or at the top level; older daemons have none. */
export interface DiagnoseThreadRow {
  id: string;
  status: string;
  last_active_at: string;
  meta?: Record<string, unknown> | null;
  source?: unknown;
  origin?: unknown;
}

/** The thread to reuse for `connId`: that connection's diagnosis thread,
 *  an open one before a closed one (the daemon closes them after 12 idle
 *  hours; a closed thread reopens on the next message), newest first. A
 *  daemon that ignores the list filter returns every thread, so the match is
 *  checked here too. */
export function pickDiagnoseThread(rows: readonly DiagnoseThreadRow[], connId: string): DiagnoseThreadRow | null {
  const mine = rows.filter((r) => {
    const m = rec(r.meta);
    const source = str(m.source) || str(r.source);
    const origin = rec(m.origin ?? r.origin);
    return source === DIAGNOSE_SOURCE && str(origin.id) === connId;
  });
  const closed = (r: DiagnoseThreadRow) => (r.status === 'closed' ? 1 : 0);
  mine.sort((a, b) => closed(a) - closed(b) || Date.parse(b.last_active_at) - Date.parse(a.last_active_at));
  return mine[0] ?? null;
}

/** A Test response (or a DB connect error) → the probe the prompt carries. */
export function probeOf(r: Pick<TestConnectionResp, 'message' | 'hint'> & { detail?: string | null }): DiagnoseProbe {
  return { message: r.message, hint: r.hint ?? null, detail: r.detail ?? null };
}
