import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  RDS_PREFILL_STORAGE_KEY,
  findMatchingK8sCluster,
  rdsEngineToKind,
  rdsToConnectionPrefill,
} from '../src/modules/aws/crossLinks.ts';

test('findMatchingK8sCluster prefers exact account + region + cluster match', () => {
  const clusters = [
    { id: 'k-other-acct', name: 'prod-eu', source: 'eks', aws_account_id: 'acct-2', params: { eks_cluster: 'prod-eu', eks_region: 'eu-west-1' } },
    { id: 'k-exact', name: 'prod-eu', source: 'eks', aws_account_id: 'acct-1', params: { eks_cluster: 'prod-eu', eks_region: 'eu-west-1' } },
    { id: 'k-other-region', name: 'prod-eu', source: 'eks', aws_account_id: 'acct-1', params: { eks_cluster: 'prod-eu', eks_region: 'us-east-1' } },
  ];
  const hit = findMatchingK8sCluster({ name: 'prod-eu', region: 'eu-west-1' }, 'acct-1', 'us-east-1', clusters);
  assert.equal(hit?.id, 'k-exact');
});

test('findMatchingK8sCluster falls back to region + cluster name and returns null when unmatched', () => {
  const clusters = [
    { id: 'k-fallback', name: 'staging', source: 'imported', aws_account_id: null, params: { eks_cluster: 'staging', eks_region: 'us-west-2' } },
  ];
  assert.equal(findMatchingK8sCluster({ name: 'staging' }, 'acct-1', 'us-west-2', clusters)?.id, 'k-fallback');
  assert.equal(findMatchingK8sCluster({ name: 'staging', region: 'eu-central-1' }, 'acct-1', 'us-west-2', clusters), null);
  assert.equal(findMatchingK8sCluster({ name: 'missing', region: 'us-west-2' }, 'acct-1', 'us-west-2', clusters), null);
});

test('rdsEngineToKind maps supported engines and rejects unsupported ones', () => {
  assert.equal(rdsEngineToKind('postgres'), 'postgres');
  assert.equal(rdsEngineToKind('aurora-postgresql'), 'postgres');
  assert.equal(rdsEngineToKind('mysql'), 'mysql');
  assert.equal(rdsEngineToKind('mariadb'), 'mysql');
  assert.equal(rdsEngineToKind('aurora-mysql'), 'mysql');
  assert.equal(rdsEngineToKind('aurora'), 'mysql');
  assert.equal(rdsEngineToKind('oracle-ee'), null);
  assert.equal(rdsEngineToKind('sqlserver-se'), null);
  assert.equal(rdsEngineToKind(null), null);
});

test('rdsToConnectionPrefill populates fields, default ports, TLS required, and env', () => {
  assert.equal(RDS_PREFILL_STORAGE_KEY, 'otto.conn_prefill');
  const pg = rdsToConnectionPrefill(
    {
      identifier: 'billing-prod',
      engine: 'aurora-postgresql',
      endpoint: 'billing-prod.abc123.eu-west-1.rds.amazonaws.com',
      port: 5439,
      db_name: 'billing',
      master_username: 'postgres',
    },
    'prod',
  );
  assert.deepEqual(pg, {
    name: 'billing-prod',
    kind: 'postgres',
    host: 'billing-prod.abc123.eu-west-1.rds.amazonaws.com',
    port: 5439,
    user: 'postgres',
    db: 'billing',
    environment: 'prod',
    tlsMode: 'required',
    source: 'aws_rds',
  });

  // Default port + unknown env normalized to dev + empty optional fields omitted.
  const my = rdsToConnectionPrefill(
    { identifier: 'orders', engine: 'mysql', endpoint: 'orders.rds.local', port: null, db_name: '', master_username: null },
    'sandbox',
  );
  assert.deepEqual(my, {
    name: 'orders',
    kind: 'mysql',
    host: 'orders.rds.local',
    port: 3306,
    environment: 'dev',
    tlsMode: 'required',
    source: 'aws_rds',
  });

  // Missing endpoint or unsupported engine returns null.
  assert.equal(rdsToConnectionPrefill({ identifier: 'creating', engine: 'postgres', endpoint: null }, 'dev'), null);
  assert.equal(rdsToConnectionPrefill({ identifier: 'erp', engine: 'oracle-se2', endpoint: 'erp.rds.local', port: 1521 }, 'prod'), null);
});
