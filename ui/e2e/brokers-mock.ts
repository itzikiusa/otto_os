// A Kafka cluster's API answered at the network layer (no live broker in CI):
// overview, metrics, topics + one topic's detail and messages, consumer groups
// and schema subjects. Used by the side-surface specs to drive the cluster
// list, the Groups / Subjects list panes and the message Detail pane.
import type { Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

/** Seed a workspace + one fake cluster; returns their ids (cluster names are unique per seed). */
export async function seedBrokersCluster(prefix: string): Promise<{ workspaceId: string; clusterId: string; clusterName: string }> {
  const clusterName = `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/brokers/clusters`, {
    data: { name: clusterName, bootstrap_servers: 'broker-1.example.com:9092', security_protocol: 'plaintext', schema_registry_url: 'http://registry.example.com:8081', environment: 'dev' },
  });
  if (!r.ok()) throw new Error(`seed cluster → ${r.status()} ${await r.text()}`);
  const clusterId = (await r.json()).id as string;
  await ctx.dispose();
  return { workspaceId, clusterId, clusterName };
}

const now = Date.now();
const msg = (partition: number, offset: number, key: string, value: object) => ({
  partition,
  offset,
  timestamp_ms: now - offset * 60_000,
  key: { format: 'utf8', text: key },
  value: { format: 'json', text: JSON.stringify(value, null, 2) },
  headers: [{ key: 'trace-id', value: `t-${offset}` }],
  size_bytes: 120 + offset,
});

export async function mockBrokersApi(page: Page, clusterId: string): Promise<void> {
  const c = `/brokers/clusters/${clusterId}`;
  const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  await page.route(new RegExp(`${c}/test$`), (r) => r.fulfill(json({ ok: true, message: 'ok', latency_ms: 3 })));
  await page.route(new RegExp(`${c}/overview$`), (r) =>
    r.fulfill(json({ cluster_id: 'c', controller_id: 1, brokers: [{ id: 1, host: 'broker-1.example.com', port: 9092, rack: null, is_controller: true, partition_leaders: 2 }], topic_count: 1, internal_topic_count: 0, partition_count: 2, consumer_group_count: 3 })),
  );
  await page.route(new RegExp(`${c}/metrics$`), (r) =>
    r.fulfill(json({ throughput: [], messages_per_sec: null, total_messages: 3, brokers: [], prometheus_available: false, sampled_at: new Date().toISOString() })),
  );
  await page.route(new RegExp(`${c}/topics$`), (r) =>
    r.fulfill(json([{ name: 'orders', partitions: 2, replication_factor: 1, message_count: 3, cleanup_policy: 'delete', internal: false }])),
  );
  await page.route(new RegExp(`${c}/topics/stats$`), (r) => r.fulfill(json({})));
  await page.route(new RegExp(`${c}/topics/orders$`), (r) =>
    r.fulfill(json({ name: 'orders', internal: false, message_count: 3, configs: [], partitions: [{ id: 0, leader: 1, replicas: [1], isr: [1], low: 0, high: 2, message_count: 2 }, { id: 1, leader: 1, replicas: [1], isr: [1], low: 0, high: 1, message_count: 1 }] })),
  );
  await page.route(new RegExp(`${c}/topics/orders/consume$`), (r) =>
    r.fulfill(json({
      messages: [msg(0, 0, 'order-1', { id: 1, status: 'paid' }), msg(0, 1, 'order-2', { id: 2, status: 'failed', reason: 'card_declined' }), msg(1, 0, 'order-3', { id: 3, status: 'pending' })],
      partitions: [{ partition: 0, low: 0, high: 2 }, { partition: 1, low: 0, high: 1 }],
      truncated: false,
    })),
  );
  await page.route(new RegExp(`${c}/groups$`), (r) =>
    r.fulfill(json([
      { group_id: 'billing', state: 'Stable', protocol_type: 'consumer', members: 2, total_lag: 1500, lag_pending: false },
      { group_id: 'orders-sink', state: 'Stable', protocol_type: 'consumer', members: 1, total_lag: 0, lag_pending: false },
      { group_id: 'audit', state: 'Empty', protocol_type: 'consumer', members: 0, total_lag: 12, lag_pending: false },
    ])),
  );
  await page.route(new RegExp(`${c}/groups/[^/]+$`), (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop() ?? '');
    return r.fulfill(json({
      group_id: id,
      state: 'Stable',
      protocol_type: 'consumer',
      protocol: 'range',
      members: [{ member_id: `${id}-1`, client_id: `${id}-client`, host: '/10.0.0.4', assignments: [{ topic: 'orders', partition: 0 }] }],
      offsets: [{ topic: 'orders', partition: 0, current_offset: 1, high_watermark: 2, lag: 1 }],
      total_lag: 1,
    }));
  });
  await page.route(new RegExp(`${c}/schema-registry/subjects$`), (r) =>
    r.fulfill(json([
      { subject: 'orders-value', version: 3, id: 11, schema_type: 'AVRO', schema: JSON.stringify({ type: 'record', name: 'Order', fields: [{ name: 'id', type: 'long' }, { name: 'status', type: 'string' }] }) },
      { subject: 'payments-value', version: 1, id: 7, schema_type: 'JSON', schema: JSON.stringify({ type: 'object', properties: { amount: { type: 'number' } } }) },
    ])),
  );
}
