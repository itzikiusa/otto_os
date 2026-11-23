import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildAwsInsightsPacket,
  buildAwsLogEventPacket,
  buildSqsMessagePacket,
  capText,
  scrubInlineSecrets,
} from '../src/modules/aws/agentContext.ts';

test('scrubInlineSecrets masks bearer tokens, AWS keys, URL creds, PEMs, and secret assignments', () => {
  const raw = [
    'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.supersecret',
    'aws=AKIAIOSFODNN7EXAMPLE',
    'url=https://admin:s3cr3t@db.internal:5432/app',
    'api_key="my-secret-key" password=hunter2',
    '-----BEGIN RSA PRIVATE KEY-----\nabc123\n-----END RSA PRIVATE KEY-----',
  ].join('\n');
  const out = scrubInlineSecrets(raw);
  assert.ok(!out.includes('eyJhbGciOiJIUzI1NiJ9'));
  assert.ok(!out.includes('AKIAIOSFODNN7EXAMPLE'));
  assert.ok(!out.includes('s3cr3t'));
  assert.ok(!out.includes('my-secret-key'));
  assert.ok(!out.includes('hunter2'));
  assert.ok(!out.includes('abc123'));
  assert.ok(out.includes('Bearer [REDACTED]'));
  assert.ok(out.includes('[REDACTED_AWS_KEY]'));
  assert.ok(out.includes('https://[REDACTED]@db.internal:5432/app'));
  assert.ok(out.includes('[REDACTED_PRIVATE_KEY]'));
});

test('capText respects head and tail modes with accurate metadata', () => {
  const head = capText('0123456789', 4, 'head');
  assert.deepEqual(head, { text: '0123', truncated: true, total_chars: 10 });
  const tail = capText('0123456789', 4, 'tail');
  assert.deepEqual(tail, { text: '6789', truncated: true, total_chars: 10 });
  const fit = capText('abc', 10, 'head');
  assert.deepEqual(fit, { text: 'abc', truncated: false, total_chars: 3 });
});

test('buildAwsLogEventPacket scrubs and caps message', () => {
  const pkt = buildAwsLogEventPacket({
    account: 'prod-aws',
    environment: 'prod',
    region: 'eu-west-1',
    group: '/aws/lambda/checkout',
    event: { timestamp: 1700000000000, stream: '2026/01/01/s1', message: 'token: Bearer topsecret12345 ' + 'x'.repeat(200) },
    maxChars: 64,
  });
  assert.equal(pkt.source, 'aws_cloudwatch_logs');
  assert.equal(pkt.log_group, '/aws/lambda/checkout');
  assert.equal(pkt.truncated, true);
  assert.equal(pkt.message.length, 64);
  assert.ok(!pkt.message.includes('topsecret12345'));
});

test('buildAwsInsightsPacket caps rows and chars and scrubs cells', () => {
  const pkt = buildAwsInsightsPacket({
    account: 'dev-aws',
    environment: 'dev',
    region: 'us-east-1',
    groups: ['/app/api'],
    query: 'fields @timestamp, @message | limit 20',
    columns: [{ name: '@timestamp' }, { name: '@message' }],
    rows: [
      ['2026-01-01T00:00:00Z', 'Bearer abcdefgh12345'],
      ['2026-01-01T00:00:01Z', 'a'.repeat(300)],
      ['2026-01-01T00:00:02Z', 'b'.repeat(300)],
    ],
    maxChars: 200,
  });
  assert.equal(pkt.source, 'aws_cloudwatch_insights');
  assert.equal(pkt.total_rows, 3);
  assert.equal(pkt.included_rows, 1);
  assert.equal(pkt.truncated, true);
  assert.equal(pkt.rows[0]['@message'], 'Bearer [REDACTED]');
});

test('buildSqsMessagePacket scrubs body, caps length, and omits receipt_handle', () => {
  const pkt = buildSqsMessagePacket({
    account: 'prod-aws',
    environment: 'prod',
    region: 'eu-west-1',
    queueName: 'orders-dlq',
    queueUrl: 'https://sqs.eu-west-1.amazonaws.com/123456789012/orders-dlq',
    message: {
      message_id: 'msg-1',
      body: '{"secret":"super_secret_val","order":42}',
      attributes: { SentTimestamp: '1700000000000', ApproximateReceiveCount: '5' },
      message_attributes: { traceId: { DataType: 'String', StringValue: 'tr-1' } },
      receipt_handle: 'SHOULD_NEVER_LEAK',
    } as never,
  });
  assert.equal(pkt.source, 'aws_sqs');
  assert.equal(pkt.receive_count, '5');
  assert.equal(pkt.truncated, false);
  assert.ok(!JSON.stringify(pkt).includes('SHOULD_NEVER_LEAK'));
  assert.ok(!pkt.body.includes('super_secret_val'));
});
