import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildKcatSnippet, shellQuote } from '../src/modules/brokers/cliSnippet.ts';
import {
  buildProducePrefill,
  buildReplayPrefill,
  mergePartitionRanges,
  offsetPct,
  replayPrefillKey,
  shouldApplyReplayPrefill,
} from '../src/modules/brokers/messagePrefill.ts';
import type { KafkaMessage } from '../src/lib/api/types';

test('shellQuote safely single-quotes arbitrary shell characters', () => {
  assert.equal(shellQuote('orders.events'), "'orders.events'");
  assert.equal(shellQuote("it's"), "'it'\\''s'");
  assert.equal(shellQuote('$(rm -rf /); `id` "$SECRET"'), "'$(rm -rf /); `id` \"$SECRET\"'");
});

test('buildKcatSnippet: plain cluster consume with and without partition/offset', () => {
  const plain = { bootstrap_servers: '  broker1:9092,broker2:9092  ', security_protocol: 'plaintext' as const };
  assert.equal(
    buildKcatSnippet(plain, { topic: 'orders' }),
    "kcat -b 'broker1:9092,broker2:9092' -t 'orders' -C",
  );
  // Zero partition and zero offset must not be dropped as falsy.
  assert.equal(
    buildKcatSnippet(plain, { topic: 'orders', partition: 0, offset: 0 }),
    "kcat -b 'broker1:9092,broker2:9092' -t 'orders' -C -p 0 -o 0",
  );
  assert.equal(
    buildKcatSnippet(plain, { topic: 'orders', partition: 3, offset: 419 }),
    "kcat -b 'broker1:9092,broker2:9092' -t 'orders' -C -p 3 -o 419",
  );
});

test('buildKcatSnippet: produce mode emits -P, keeps -p, omits -o', () => {
  const plain = { bootstrap_servers: 'broker1:9092', security_protocol: 'plaintext' as const };
  assert.equal(
    buildKcatSnippet(plain, { topic: 'events', mode: 'produce' }),
    "kcat -b 'broker1:9092' -t 'events' -P",
  );
  assert.equal(
    buildKcatSnippet(plain, { topic: 'events', partition: 2, offset: 99, mode: 'produce' }),
    "kcat -b 'broker1:9092' -t 'events' -P -p 2",
  );
});

test('buildKcatSnippet: TLS and SASL emit safe placeholders and never leak raw secrets', () => {
  const ssl = {
    bootstrap_servers: 'kafka.internal:9093',
    security_protocol: 'ssl' as const,
    tls_skip_verify: true,
  };
  assert.equal(
    buildKcatSnippet(ssl, { topic: 'audit' }),
    "kcat -b 'kafka.internal:9093' -t 'audit' -C -X security.protocol=ssl -X enable.ssl.certificate.verification=false",
  );

  const saslSsl = {
    bootstrap_servers: 'msk.internal:9096',
    security_protocol: 'sasl_ssl' as const,
    sasl_mechanism: 'scram_sha_512' as const,
    sasl_username: 'svc_orders',
    has_sasl_password: true,
    // Even if a caller accidentally passes a raw secret field at runtime, it must never appear.
    sasl_password: 'SUPER_SECRET_PASSWORD_123',
    schema_registry_password: 'SR_SECRET_456',
  };
  const cmd = buildKcatSnippet(saslSsl, { topic: 'payments', partition: 1, offset: 500 });
  assert.equal(
    cmd,
    "kcat -b 'msk.internal:9096' -t 'payments' -C -p 1 -o 500 -X security.protocol=sasl_ssl -X sasl.mechanisms=SCRAM-SHA-512 -X sasl.username='svc_orders' -X sasl.password=\"$KAFKA_PASSWORD\"",
  );
  assert.ok(!cmd.includes('SUPER_SECRET_PASSWORD_123'));
  assert.ok(!cmd.includes('SR_SECRET_456'));

  const saslNoUser = {
    bootstrap_servers: 'kafka:9094',
    security_protocol: 'sasl_plaintext' as const,
    sasl_mechanism: 'scram_sha_256' as const,
    sasl_username: '   ',
  };
  assert.equal(
    buildKcatSnippet(saslNoUser, { topic: 'logs' }),
    'kcat -b \'kafka:9094\' -t \'logs\' -C -X security.protocol=sasl_plaintext -X sasl.mechanisms=SCRAM-SHA-256 -X sasl.username="$KAFKA_USERNAME" -X sasl.password="$KAFKA_PASSWORD"',
  );
});

test('buildKcatSnippet: shell-quotes hostile broker, topic and username values', () => {
  const hostile = {
    bootstrap_servers: "b1:9092;echo 'pwn'",
    security_protocol: 'sasl_plaintext' as const,
    sasl_mechanism: 'plain' as const,
    sasl_username: "o'brien$(whoami)",
  };
  const cmd = buildKcatSnippet(hostile, { topic: "user's`id`topic" });
  assert.equal(
    cmd,
    "kcat -b 'b1:9092;echo '\\''pwn'\\''' -t 'user'\\''s`id`topic' -C -X security.protocol=sasl_plaintext -X sasl.mechanisms=PLAIN -X sasl.username='o'\\''brien$(whoami)' -X sasl.password=\"$KAFKA_PASSWORD\"",
  );
});

test('buildProducePrefill copies key, value, partition and cloned headers; handles tombstones & binary', () => {
  const msg: KafkaMessage = {
    partition: 4,
    offset: 120,
    timestamp_ms: 1_700_000_000_000,
    key: { format: 'utf8', text: 'order-42' },
    value: { format: 'json', text: '{"ok":true}' },
    headers: [{ key: 'x-trace', value: 'abc' }],
    size_bytes: 32,
  };
  const pre = buildProducePrefill(msg);
  assert.deepEqual(pre, {
    key: 'order-42',
    keyBase64: false,
    value: '{"ok":true}',
    valueBase64: false,
    tombstone: false,
    partition: 4,
    headers: [{ key: 'x-trace', value: 'abc' }],
  });
  // Mutating the prefill headers must not mutate the source message.
  pre.headers[0].value = 'mutated';
  assert.equal(msg.headers[0].value, 'abc');

  const tombstoneMsg: KafkaMessage = {
    partition: 0,
    offset: 1,
    timestamp_ms: null,
    key: null,
    value: null,
    headers: [],
    size_bytes: 0,
  };
  assert.deepEqual(buildProducePrefill(tombstoneMsg), {
    key: '',
    keyBase64: false,
    value: '',
    valueBase64: false,
    tombstone: true,
    partition: 0,
    headers: [],
  });

  const binaryMsg: KafkaMessage = {
    partition: 2,
    offset: 7,
    timestamp_ms: null,
    key: { format: 'hex', text: 'deadbeef', raw_base64: '3q2+7w==' },
    value: { format: 'base64', text: 'AAECAw==' },
    headers: [],
    size_bytes: 8,
  };
  const binPre = buildProducePrefill(binaryMsg);
  assert.equal(binPre.key, '3q2+7w==');
  assert.equal(binPre.keyBase64, true);
  assert.equal(binPre.value, 'AAECAw==');
  assert.equal(binPre.valueBase64, true);
  assert.equal(binPre.tombstone, false);
});

test('Replay prefill helpers set offset_range and avoid overwriting on same-cluster re-render', () => {
  const first = { sourceTopic: 'orders-dlq', partition: 0, offset: 0, seq: 1 };
  assert.equal(shouldApplyReplayPrefill(null, null), false);
  assert.equal(shouldApplyReplayPrefill(first, null), true);
  assert.deepEqual(buildReplayPrefill(first), {
    sourceTopic: 'orders-dlq',
    selectorType: 'offset_range',
    partition: 0,
    fromOffset: 0,
    toOffset: 0,
  });

  const appliedKey = replayPrefillKey(first);
  // Same-cluster re-render with the same prefill must not overwrite user edits.
  assert.equal(shouldApplyReplayPrefill(first, appliedKey), false);
  assert.equal(shouldApplyReplayPrefill({ ...first }, appliedKey), false);
  // Re-triggering "Replay to topic…" bumps seq and re-applies cleanly.
  assert.equal(shouldApplyReplayPrefill({ ...first, seq: 2 }, appliedKey), true);
});

test('mergePartitionRanges and offsetPct preserve widest bounds and compute clamped percentages', () => {
  const merged = mergePartitionRanges(
    [{ partition: 1, low: 10, high: 20 }],
    [
      { partition: 0, low: 0, high: 100 },
      { partition: 1, low: 5, high: 30 },
    ],
  );
  assert.deepEqual(merged, [
    { partition: 0, low: 0, high: 100 },
    { partition: 1, low: 5, high: 30 },
  ]);
  assert.equal(offsetPct({ partition: 0, offset: 50 }, merged), 50);
  assert.equal(offsetPct({ partition: 9, offset: 1 }, merged), null);
  assert.equal(offsetPct({ partition: 0, offset: 0 }, [{ partition: 0, low: 5, high: 5 }]), null);
});
