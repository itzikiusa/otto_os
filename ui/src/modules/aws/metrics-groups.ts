// How the CloudWatch catalog (docs/contracts/api.md "CloudWatch metrics") is
// laid out as cards: series that belong together (in/out, read/write, the
// three SQS backlog gauges) share one chart. Every series in a group has the
// same unit by construction. Groups whose ids the daemon didn't return at all
// (EC2 credit metrics on non-burstable types) are dropped; groups with data
// gaps still render so the card can say "no data".

import type { MetricSeries, MetricsNamespace, MetricsResp } from '../../lib/api/types';
import type { MetricChartSeries } from '../../lib/metric-format';

export interface MetricGroup {
  id: string;
  title: string;
  ids: string[];
}

export const METRIC_GROUPS: Record<MetricsNamespace, MetricGroup[]> = {
  'AWS/SQS': [
    { id: 'messages', title: 'Messages', ids: ['messages_sent', 'messages_received', 'messages_deleted'] },
    { id: 'backlog', title: 'Queue depth', ids: ['messages_visible', 'messages_not_visible', 'messages_delayed'] },
    { id: 'age', title: 'Age of oldest message', ids: ['oldest_message_age'] },
    { id: 'bytes', title: 'Bytes in', ids: ['bytes_in'] },
    { id: 'size', title: 'Average message size', ids: ['sent_message_size'] },
    { id: 'empty', title: 'Empty receives', ids: ['empty_receives'] },
  ],
  'AWS/EC2': [
    { id: 'cpu', title: 'CPU utilization', ids: ['cpu'] },
    { id: 'network', title: 'Network', ids: ['network_in', 'network_out'] },
    { id: 'packets', title: 'Packets', ids: ['packets_in', 'packets_out'] },
    { id: 'disk_bytes', title: 'Disk throughput', ids: ['disk_read_bytes', 'disk_write_bytes'] },
    { id: 'disk_ops', title: 'Disk operations', ids: ['disk_read_ops', 'disk_write_ops'] },
    { id: 'status', title: 'Status check failed', ids: ['status_check_failed'] },
    { id: 'credits', title: 'CPU credits', ids: ['cpu_credit_balance', 'cpu_credit_usage'] },
  ],
  'AWS/RDS': [
    { id: 'cpu', title: 'CPU utilization', ids: ['cpu'] },
    { id: 'connections', title: 'Connections', ids: ['connections'] },
    { id: 'memory', title: 'Freeable memory', ids: ['freeable_memory'] },
    { id: 'storage', title: 'Free storage', ids: ['free_storage'] },
    { id: 'iops', title: 'IOPS', ids: ['read_iops', 'write_iops'] },
    { id: 'latency', title: 'Latency', ids: ['read_latency', 'write_latency'] },
    { id: 'throughput', title: 'Throughput', ids: ['read_throughput', 'write_throughput'] },
    { id: 'network', title: 'Network', ids: ['network_rx', 'network_tx'] },
    { id: 'swap', title: 'Swap usage', ids: ['swap_usage'] },
    { id: 'queue', title: 'Disk queue depth', ids: ['disk_queue_depth'] },
    { id: 'burst', title: 'Burst balance', ids: ['burst_balance'] },
  ],
};

export interface MetricCard {
  group: MetricGroup;
  series: MetricSeries[];
  chart: MetricChartSeries[];
}

/** Arrange a response into cards, in catalog-group order. */
export function buildCards(resp: MetricsResp): MetricCard[] {
  const byId = new Map(resp.series.map((s) => [s.id, s]));
  const out: MetricCard[] = [];
  for (const group of METRIC_GROUPS[resp.namespace] ?? []) {
    const series = group.ids.map((id) => byId.get(id)).filter((s): s is MetricSeries => !!s);
    if (series.length === 0) continue;
    out.push({
      group,
      series,
      chart: series.map((s) => ({
        label: s.label,
        points: s.points.map((p) => ({ t: Date.parse(p.t), v: p.v })),
      })),
    });
  }
  return out;
}
