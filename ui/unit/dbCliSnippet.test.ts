import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildDbCliSnippet,
  cliLabelForKind,
  redactUriPassword,
  shQuote,
} from '../src/modules/database/cliSnippet.ts';

test('shQuote leaves safe tokens unquoted and safely escapes shell metacharacters', () => {
  assert.equal(shQuote('db.internal'), 'db.internal');
  assert.equal(shQuote("o'brien db"), "'o'\\''brien db'");
  assert.equal(shQuote('a;rm -rf /'), "'a;rm -rf /'");
});

test('redactUriPassword masks embedded passwords in connection URIs', () => {
  assert.equal(
    redactUriPassword('mongodb+srv://admin:s3cr3t!@cluster0.example.net/app?retryWrites=true'),
    'mongodb+srv://admin:<password>@cluster0.example.net/app?retryWrites=true',
  );
  assert.equal(
    redactUriPassword('mongodb://localhost:27017/dev'),
    'mongodb://localhost:27017/dev',
  );
});

test('buildDbCliSnippet generates safe secret-free commands across engines', () => {
  assert.equal(
    buildDbCliSnippet(
      { kind: 'postgres', params: { host: 'pg.prod.internal', port: 5439, user: 'readonly', db: 'billing', tls: { mode: 'required' }, password: 'NEVER' } },
      { activeDb: 'analytics' },
    ),
    'PGSSLMODE=require psql -h pg.prod.internal -p 5439 -U readonly -d analytics',
  );

  assert.equal(
    buildDbCliSnippet({ kind: 'mysql', params: { host: 'mysql.local', port: 3306, user: 'app', db: 'orders', secure: true } }),
    'mysql -h mysql.local -u app -p --ssl-mode=REQUIRED orders',
  );

  assert.equal(
    buildDbCliSnippet({ kind: 'mongodb', params: { conn_string: 'mongodb://root:topsecret@mongo.local:27017/shop' } }),
    "mongosh 'mongodb://root:<password>@mongo.local:27017/shop'",
  );

  assert.equal(
    buildDbCliSnippet({ kind: 'redis', params: { host: 'cache.internal', port: 6380, user: 'default', db: '2', tls: { mode: 'required' } } }),
    'redis-cli -h cache.internal -p 6380 --user default -n 2 --tls',
  );

  assert.equal(
    buildDbCliSnippet({ kind: 'clickhouse', params: { host: 'ch.internal', port: 9440, user: 'ops', db: 'events', secure: true } }),
    'clickhouse-client --host ch.internal --port 9440 --user ops --ask-password --database events --secure',
  );

  assert.equal(
    buildDbCliSnippet({ kind: 'ssh', params: { host: '10.0.1.5', port: 2222, user: 'ubuntu', jump: 'bastion.example.com', identity_file: '~/.ssh/id_ed25519' } }),
    'ssh -p 2222 -J bastion.example.com -i ~/.ssh/id_ed25519 ubuntu@10.0.1.5',
  );

  // Never leaks password param or returns snippet without host/URI.
  assert.equal(buildDbCliSnippet({ kind: 'postgres', params: {} }), null);
  assert.equal(buildDbCliSnippet({ kind: 'custom', params: { command_template: 'my-cli' } }), null);
  assert.equal(cliLabelForKind('postgres'), 'psql');
  assert.equal(cliLabelForKind('custom'), null);
});
