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
