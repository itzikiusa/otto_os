// Monitoring-settings validation, pure so it is unit-testable. Mirrors the
// daemon's limits (MonitorSettings.svelte shows the messages inline). Errors
// are keyed by row index (`probe.${i}.name`, `probe.${i}.map.${j}`, `ex.${i}`).
import type { K8sMonitorConfig } from '../../../lib/api/types';

export function validateMonitorConfig(c: K8sMonitorConfig, needsNs: boolean): Record<string, string> {
  const e: Record<string, string> = {};
  if (c.interval_secs < 15 || c.interval_secs > 3600) e.interval_secs = 'Interval must be 15..3600 seconds.';
  if (c.concurrency < 1 || c.concurrency > 32) e.concurrency = 'Concurrency must be 1..32.';
  if (c.retention_days < 1 || c.retention_days > 90) e.retention_days = 'Retention must be 1..90 days.';
  if (c.series_cap < 100 || c.series_cap > 10000) e.series_cap = 'Series cap must be 100..10000.';
  if (c.probes.length > 10) e.probes = 'At most 10 probes.';
  const names = new Set<string>();
  c.probes.forEach((p, i) => {
    if (!p.name.trim()) e[`probe.${i}.name`] = 'Name is required.';
    else if (names.has(p.name.trim())) e[`probe.${i}.name`] = 'Duplicate name.';
    names.add(p.name.trim());
    if (!p.path.startsWith('/')) e[`probe.${i}.path`] = 'Path must start with /.';
    if (p.port !== null && p.port !== undefined && (p.port < 1 || p.port > 65535)) e[`probe.${i}.port`] = 'Port must be 1..65535.';
    (p.mappings ?? []).forEach((m, j) => {
      if (!m.field.trim()) e[`probe.${i}.map.${j}`] = 'Field is required.';
      else if (!m.metric && !m.label) e[`probe.${i}.map.${j}`] = 'Set a metric name or a label.';
      else if (m.metric && !/^[A-Za-z0-9_:.]{1,128}$/.test(m.metric)) e[`probe.${i}.map.${j}`] = 'Metric: letters, digits, _ : . only.';
    });
  });
  c.exclusions.forEach((x, i) => {
    const v = x.kind === 'label' ? x.selector : x.match;
    if (!v.trim()) e[`ex.${i}`] = 'Pattern is required.';
  });
  if (needsNs && c.namespaces.length === 0) e.namespaces = 'This cluster has no default namespace — list the namespaces to monitor.';
  return e;
}

/** After a probe, mapping or exclusion is removed: the rows after it shift
 *  down one index, so index-keyed errors would land on the wrong row (or
 *  vanish while the banner still counts them). Re-validate while errors show;
 *  with none showing, stay quiet until Save. */
export function revalidateAfterRemove(
  errors: Record<string, string>,
  c: K8sMonitorConfig,
  needsNs: boolean,
): Record<string, string> {
  return Object.keys(errors).length ? validateMonitorConfig(c, needsNs) : errors;
}
