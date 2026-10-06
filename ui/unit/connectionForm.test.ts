import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
function form(overrides: Record<string, unknown> = {}) {
  const text = readFileSync(new URL('../src/modules/connections/ConnectionForm.svelte', import.meta.url), 'utf8');
  const functions = text.slice(text.indexOf('  function buildParams()'), text.indexOf('  let showDsnInput'));
  const ctx: Record<string, any> = {
    existing: null, kind: 'mysql', fHost: 'db', fPort: '', fUser: '', fDb: '', fTimezone: 'UTC', fConnString: '',
    fTemplate: '', fJump: '', fIdentity: '', sshEnabled: false, tlsMode: 'disabled', tlsVerify: true,
    tlsCaCert: '', tlsClientCert: '', tlsClientKey: '', tlsServerName: '', tunnelOpen: false,
    tunHost: '', tunPort: '', tunUser: '', tunIdentity: '', secret: '', name: '',
    hasDatabaseField: new Set(['mysql','postgres','redis','mongodb','clickhouse']),
    tzKinds: new Set(['mysql','postgres','clickhouse']), tlsKinds: new Set(['mysql','postgres','redis','mongodb','clickhouse']),
    kindLabels: { ssh: 'SSH', mysql: 'MySQL', postgres: 'PostgreSQL', redis: 'Redis', mongodb: 'MongoDB', clickhouse: 'ClickHouse', custom: 'Custom' },
    URL, toasts: { error: () => {}, success: () => {} }, ...overrides,
  };
  ctx.setKind = (kind: string) => { ctx.kind = kind; };
  runInNewContext(ts.transpileModule(functions, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx;
}
test('renaming retains unknown parameters and TLS shorthand', () => {
  const c = form({ existing: { kind: 'mysql', params: { host: 'db', secure: true, custom_option: 'keep' } }, tlsMode: 'required' });
  const result = c.buildParams();
  assert.equal(result.custom_option, 'keep');
  assert.equal(result.tls.mode, 'required');
});
test('secure URI schemes enable TLS and Mongo passwords leave URI fields', () => {
  for (const scheme of ['rediss', 'clickhouse+https']) {
    const c = form(); c.parseDsn(`${scheme}://alice:secret@db:1234/app`);
    assert.equal(c.tlsMode, 'required');
  }
  const c = form(); c.parseDsn('mongodb://alice:p%40ss@db/app?authSource=admin');
  assert.equal(c.secret, 'p@ss');
  assert.equal(c.fConnString, 'mongodb://alice:{secret}@db/app?authSource=admin');
});
test('explicit field clears preserve unrelated advanced TLS and tunnel settings', () => {
  const c = form({ existing: { kind: 'mysql', params: { host: 'old', jump: 'clear-me', database: 'legacy', secure: true, custom: 'retain', tls: { mode: 'required', ca_cert: 'clear-me', advanced_tls: true }, ssh: { host: 'bastion', advanced_ssh: 'retain' } } }, tlsMode: 'required', tunnelOpen: true, tunHost: 'bastion', fDb: 'legacy' });
  const result = c.buildParams();
  assert.equal(result.custom, 'retain');
  assert.equal(result.jump, undefined);
  assert.equal(result.database, undefined);
  assert.equal(result.db, 'legacy');
  assert.equal(result.tls.ca_cert, undefined);
  assert.equal(result.tls.advanced_tls, true);
  assert.equal(result.ssh.advanced_ssh, 'retain');
  c.tlsMode = 'disabled';
  assert.equal(c.buildParams().secure, undefined);
});
test('Mongo URI: an encodable-character password never stays in the visible string (S16-15)', () => {
  // `^`, `{`, `|` and a space are re-encoded by WHATWG URL — the old replace on
  // `url.password` missed them and left the cleartext in fConnString.
  const c = form(); c.parseDsn('mongodb://alice:p^a{s|s w@h1:27017/app?authSource=admin');
  assert.equal(c.secret, 'p^a{s|s w');
  assert.equal(c.fConnString, 'mongodb://alice:{secret}@h1:27017/app?authSource=admin');
  assert.ok(!c.fConnString.includes('p^a'));
  // A malformed escape is kept raw instead of throwing out of the import.
  const m = form(); m.parseDsn('mongodb://bob:bad%zz@db/app');
  assert.equal(m.secret, 'bad%zz');
  assert.equal(m.fConnString, 'mongodb://bob:{secret}@db/app');
  // No password: unchanged.
  const n = form(); n.parseDsn('mongodb://db/app');
  assert.equal(n.secret, '');
  assert.equal(n.fConnString, 'mongodb://db/app');
  const p = form(); assert.doesNotThrow(() => p.parseDsn('postgres://u%zz:pw%zz@db/a%zz'));
  assert.equal(p.fUser, 'u%zz');
});
test('Mongo URI: an unencoded `/` in the password never stays visible (S16-307)', () => {
  const c = form(); c.parseDsn('mongodb://app:12/ss@h1:27017/db?authSource=admin');
  assert.equal(c.secret, '12/ss');
  assert.equal(c.fConnString, 'mongodb://app:{secret}@h1:27017/db?authSource=admin');
  // An `@` inside the query is an option value, not credentials.
  const q = form(); q.parseDsn('mongodb://h1:27017/db?appName=a@b');
  assert.equal(q.secret, '');
  assert.equal(q.fConnString, 'mongodb://h1:27017/db?appName=a@b');
  // An unparsable URI with credentials says how to fix it.
  let msg = '';
  const r = form({ toasts: { error: (_t: string, m: string) => (msg = m), success: () => {} } });
  r.parseDsn('mongodb://app:pa/ss@h/db');
  assert.match(msg, /percent-encode/);
  assert.equal(r.fConnString, '');
});
