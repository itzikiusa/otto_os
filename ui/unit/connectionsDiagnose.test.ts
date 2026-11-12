// "Diagnose with an agent": the consent text, the checks, the opening prompt
// (facts in, secrets never) and the reuse-an-earlier-thread rule
// (src/modules/database/diagnose-prompt.ts), plus the argv-password
// warning rule (connection-warnings.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const d = loadSource(new URL('../src/modules/database/diagnose-prompt.ts', import.meta.url), {});
const w = loadSource(new URL('../src/modules/connections/connection-warnings.ts', import.meta.url), {});

const PEM_KEY = '-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n-----END PRIVATE KEY-----';
const sshConn = {
  name: 'prod-bastion',
  kind: 'ssh',
  environment: 'prod',
  read_only: true,
  // A password and key material a careless builder could leak.
  secret: 'hunter2-SECRET',
  secret_ref: 'otto/conn/abc',
  params: { host: 'bastion.example.com', port: 2222, user: 'deploy', jump: 'ops@hop:22', identity_file: '~/.ssh/id_ed25519', password: 'hunter2-SECRET', private_key: PEM_KEY },
};
const dbConn = {
  name: 'orders-db',
  kind: 'mysql',
  environment: 'staging',
  read_only: false,
  params: {
    host: 'orders.internal', port: 3306, user: 'app', password: 'db-PASSWORD',
    tls: { mode: 'required', verify: false, ca_cert: '~/certs/ca.pem', client_key: PEM_KEY },
    ssh: { host: 'tunnel.example.com', user: 'ec2-user', identity_file: '~/.ssh/tun', proxy_jump: 'a@hop1,hop2' },
  },
};
const probe = { message: 'Permission denied (publickey).', hint: 'Load the key into ssh-agent.', detail: 'debug1: Offering public key: ~/.ssh/id_ed25519' };

test('the prompt carries the connection facts, the probe and the firm rules', () => {
  const p: string = d.diagnosePrompt(sshConn, probe);
  for (const fact of ['prod-bastion', 'Kind: ssh', 'Environment: prod (read-only profile)', 'Host: bastion.example.com', 'Port: 2222', 'User: deploy', 'Jump host: ops@hop:22', 'Identity file (path): ~/.ssh/id_ed25519']) {
    assert.ok(p.includes(fact), `missing: ${fact}`);
  }
  assert.ok(p.includes('Permission denied (publickey).'));
  assert.ok(p.includes('Load the key into ssh-agent.'));
  assert.ok(p.includes('debug1: Offering public key'));
  assert.match(p, /NEVER read the contents of a private key/);
  assert.match(p, /Never run anything on the remote host/);
  assert.match(p, /never remove a known-hosts entry/);
  assert.match(p, /host key has CHANGED/);
  assert.match(p, /cause, the evidence .* and the exact fix/);
  assert.ok(p.includes('ssh -vvv -o BatchMode=yes -o ConnectTimeout=5'));
});

test('the prompt never contains a password, the secret or key material', () => {
  for (const [conn, secrets] of [[sshConn, ['hunter2-SECRET', 'otto/conn/abc']], [dbConn, ['db-PASSWORD']]] as const) {
    const p: string = d.diagnosePrompt(conn, probe);
    for (const s of [...secrets, 'BEGIN PRIVATE KEY', 'MIIEvQIBADANBgkqhkiG9w0BAQEFAASC']) {
      assert.ok(!p.includes(s), `leaked ${s}`);
    }
  }
  // A Mongo URI with an inline password: only its hosts reach the prompt.
  const mongo = { name: 'm', kind: 'mongodb', environment: 'dev', read_only: false, params: { conn_string: 'mongodb://app:s3cr3t@m1:27017,m2:27017/db?authSource=admin' } };
  const p: string = d.diagnosePrompt(mongo, null);
  assert.ok(!p.includes('s3cr3t'));
  assert.ok(p.includes('Host: m1:27017,m2:27017'));
});

test('a DB kind over a tunnel: tunnel, jump hops and TLS paths, PEM never', () => {
  const p: string = d.diagnosePrompt(dbConn, null);
  assert.ok(p.includes('SSH tunnel: ec2-user@tunnel.example.com:22, jump a@hop1,hop2, identity file ~/.ssh/tun'));
  assert.ok(p.includes('TLS: required, certificate NOT verified; ca_cert: ~/certs/ca.pem; client_key: (pasted PEM — content not shared)'));
  assert.ok(p.includes('(no test result available)'));
  const checks: string[] = d.diagnoseChecks(dbConn);
  assert.ok(checks.includes('nc -vz -w 5 tunnel.example.com 22'));
  assert.ok(checks.some((c) => c.startsWith('ssh -G -p 22 -J a@hop1,hop2 -i ~/.ssh/tun ec2-user@tunnel.example.com')));
});

test('the checks are local and read-only, and never log in or write known_hosts', () => {
  const checks: string[] = d.diagnoseChecks(sshConn);
  assert.ok(checks.includes("ssh-keygen -F '[bastion.example.com]:2222'"));
  assert.ok(checks.includes('ls -l ~/.ssh/id_ed25519'));
  assert.ok(checks.includes('ssh-add -l'));
  const verbose = checks.find((c) => c.startsWith('ssh -vvv'))!;
  assert.match(verbose, /PreferredAuthentications=none/);
  assert.match(verbose, /StrictHostKeyChecking=yes/);
  for (const c of checks) assert.doesNotMatch(c, /\b(rm|sed -i|ssh-keygen -R|cat [^~]*id_)/);
  // A direct DB connection: name resolution and the port only.
  const direct: string[] = d.diagnoseChecks({ kind: 'postgres', params: { host: 'pg.internal', port: 5432 } });
  assert.deepEqual(JSON.parse(JSON.stringify(direct)), ['dscacheutil -q host -a name pg.internal', 'dig +short pg.internal', 'nc -vz -w 5 pg.internal 5432']);
});

test('the consent says plainly what runs where and what it will not do', () => {
  const text: string = d.diagnoseConsent(sshConn);
  assert.equal(
    text,
    'An agent will run read-only checks ON THIS MAC to find out why prod-bastion (ssh, bastion.example.com) does not connect: ' +
      "name resolution, reachability of bastion.example.com:2222, the SSH client’s verbose output, your SSH config for this host, " +
      'the known-hosts entry, loaded keys and key-file permissions. It will not log in to the remote host, change any file, ' +
      'or see your password or private key. It asks you before anything else.',
  );
  assert.match(d.diagnoseConsent(dbConn), /orders-db \(mysql, tunnel\.example\.com\).*reachability of tunnel\.example\.com:22/);
});

test('an earlier diagnosis thread of the same connection is reused: open before closed, newest first', () => {
  const rows = [
    { id: 'plain', status: 'idle', last_active_at: '2026-11-12T10:00:00Z', meta: null },
    { id: 'other-conn', status: 'idle', last_active_at: '2026-11-12T12:00:00Z', meta: { source: 'connection-diagnose', origin: { kind: 'connection', id: 'c2' } } },
    { id: 'old', status: 'idle', last_active_at: '2026-11-11T10:00:00Z', meta: { source: 'connection-diagnose', origin: { kind: 'connection', id: 'c1' } } },
    { id: 'new', status: 'dormant', last_active_at: '2026-11-12T09:00:00Z', source: 'connection-diagnose', origin: { kind: 'connection', id: 'c1' } },
    { id: 'closed', status: 'closed', last_active_at: '2026-11-12T11:00:00Z', meta: { source: 'connection-diagnose', origin: { kind: 'connection', id: 'c1' } } },
  ];
  assert.equal(d.pickDiagnoseThread(rows, 'c1')?.id, 'new');
  assert.equal(d.pickDiagnoseThread(rows, 'c3'), null);
  // An open thread wins over a newer closed one…
  assert.equal(d.pickDiagnoseThread(rows.filter((r) => r.id === 'old' || r.id === 'closed'), 'c1')?.id, 'old');
  // …and a closed one (auto-closed after idle hours) is still reused: a message reopens it.
  assert.equal(d.pickDiagnoseThread(rows.filter((r) => r.id === 'closed'), 'c1')?.id, 'closed');
  assert.equal(d.originLabel('x'.repeat(250)).length, 200);
  // An older daemon ignores the filter and has no source on any row.
  assert.equal(d.pickDiagnoseThread([rows[0]], 'c1'), null);
});

test('password on the command line: clickhouse with a password, mongodb with one in the URI', () => {
  assert.equal(w.argvExposure('clickhouse', '', true), 'clickhouse');
  assert.equal(w.argvExposure('clickhouse', '', false), null);
  assert.equal(w.argvExposure('mongodb', 'mongodb://app:{secret}@h/db', false), 'mongodb');
  assert.equal(w.argvExposure('mongodb', 'mongodb://app:pw@h/db', false), 'mongodb');
  assert.equal(w.argvExposure('mongodb', 'mongodb://h/db?appName=a:b@c', true), null);
  assert.equal(w.argvExposure('mysql', '', true), null);
  assert.match(w.argvWarning('mongodb'), /mongosh/);
  assert.equal(w.argvWarning(null), null);
});

test('a Test that reports warn_argv decides the warning, true or false; otherwise the local rule', () => {
  // The daemon says no (e.g. the password moved out of the URI): no warning, whatever the rule guesses.
  assert.equal(w.argvWarningFor('mongodb', false, 'mongodb://app:{secret}@h/db', true), null);
  // The daemon says yes for a kind the rule does not know (a custom template with {secret}).
  assert.match(w.argvWarningFor('custom', true, '', true), /command line/);
  assert.match(w.argvWarningFor('clickhouse', true, '', false), /clickhouse-client/);
  // No verdict (the driver-backed DB test): the local rule.
  assert.match(w.argvWarningFor('clickhouse', undefined, '', true), /clickhouse-client/);
  assert.equal(w.argvWarningFor('clickhouse', undefined, '', false), null);
});
