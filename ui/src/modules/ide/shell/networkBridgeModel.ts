// Pure draft/serialization & suggestion helpers for the IDE Kubernetes + SSH
// Network Bridge (`IdeNetworkBridgeCard`, `IdeEnvironmentsModal`, `IdeSshTargetModal`,
// and `IdeRunToolbar`). Kept separate from Svelte components so both lazy modals
// share one implementation and `ui/unit/` can test every branch directly.

import type { Connection, K8sCluster, K8sNamespace } from '../../../lib/api/types';
import type { IdeHostMapping, IdeNetworkBridgeConfig } from '../store/ideTypes';

export interface EditableHostMappingRow {
  rowId: string;
  from_host: string;
  from_port: string;
  to_host: string;
  to_port: string;
}

export interface IdeNetworkBridgeDraft {
  enabled: boolean;
  k8s_cluster_id: string;
  k8s_namespace: string;
  ssh_tunnel_connection_id: string;
  inject_k8s_service_env: boolean;
  containerized: boolean;
  mappings: EditableHostMappingRow[];
}

export function emptyNetworkBridgeDraft(cfg?: IdeNetworkBridgeConfig | null): IdeNetworkBridgeDraft {
  return {
    enabled: Boolean(cfg?.enabled),
    k8s_cluster_id: cfg?.k8s_cluster_id?.trim() ?? '',
    k8s_namespace: cfg?.k8s_namespace?.trim() ?? '',
    ssh_tunnel_connection_id: cfg?.ssh_tunnel_connection_id?.trim() ?? '',
    inject_k8s_service_env: cfg?.inject_k8s_service_env !== false,
    containerized: Boolean(cfg?.containerized),
    mappings: (cfg?.host_mappings ?? []).map((m, idx) => ({
      rowId: `hm-${idx}-${m.from_host}`,
      from_host: m.from_host ?? '',
      from_port: m.from_port ? String(m.from_port) : '',
      to_host: m.to_host ?? '',
      to_port: m.to_port ? String(m.to_port) : '',
    })),
  };
}

function parseOptionalPort(raw: string, label: string): { port?: number; error?: string } {
  const trimmed = raw.trim();
  if (!trimmed) return {};
  if (!/^\d+$/.test(trimmed)) {
    return { error: `Invalid ${label} port "${trimmed}" — enter 1–65535 or leave blank.` };
  }
  const n = Number(trimmed);
  if (!Number.isInteger(n) || n < 1 || n > 65535) {
    return { error: `Invalid ${label} port "${trimmed}" — enter 1–65535.` };
  }
  return { port: n };
}

export function serializeNetworkBridgeDraft(
  draft: IdeNetworkBridgeDraft,
): { config: IdeNetworkBridgeConfig; error: string | null } {
  const host_mappings: IdeHostMapping[] = [];
  for (let i = 0; i < draft.mappings.length; i++) {
    const row = draft.mappings[i];
    const fromHost = row.from_host.trim();
    const toHost = row.to_host.trim();
    const fromPortRaw = row.from_port.trim();
    const toPortRaw = row.to_port.trim();
    if (!fromHost && !toHost && !fromPortRaw && !toPortRaw) continue;
    if (!fromHost || !toHost) {
      return {
        config: emptySerializedBridge(),
        error: `Host mapping row ${i + 1} needs both source host and destination host.`,
      };
    }
    if (/[\s/]/.test(fromHost) || /[\s/]/.test(toHost)) {
      return {
        config: emptySerializedBridge(),
        error: `Host mapping "${fromHost} → ${toHost}" must use hostnames without spaces or paths.`,
      };
    }
    const fp = parseOptionalPort(fromPortRaw, 'source');
    if (fp.error) return { config: emptySerializedBridge(), error: fp.error };
    const tp = parseOptionalPort(toPortRaw, 'destination');
    if (tp.error) return { config: emptySerializedBridge(), error: tp.error };
    host_mappings.push({
      from_host: fromHost,
      from_port: fp.port ?? null,
      to_host: toHost,
      to_port: tp.port ?? null,
    });
  }

  const k8sClusterId = draft.k8s_cluster_id.trim() || null;
  const k8sNamespace = draft.k8s_namespace.trim() || null;
  const sshTunnelId = draft.ssh_tunnel_connection_id.trim() || null;

  if (draft.enabled && !k8sClusterId && !sshTunnelId && host_mappings.length === 0) {
    return {
      config: emptySerializedBridge(),
      error: 'Select a Kubernetes cluster, an SSH bastion tunnel, or add a host mapping before enabling the bridge.',
    };
  }

  return {
    config: {
      enabled: Boolean(draft.enabled),
      k8s_cluster_id: k8sClusterId,
      k8s_namespace: k8sClusterId ? k8sNamespace || 'default' : k8sNamespace,
      ssh_tunnel_connection_id: sshTunnelId,
      inject_k8s_service_env: Boolean(draft.inject_k8s_service_env),
      containerized: Boolean(draft.containerized),
      host_mappings,
    },
    error: null,
  };
}

function emptySerializedBridge(): IdeNetworkBridgeConfig {
  return {
    enabled: false,
    k8s_cluster_id: null,
    k8s_namespace: null,
    ssh_tunnel_connection_id: null,
    inject_k8s_service_env: true,
    containerized: false,
    host_mappings: [],
  };
}

export function mergeClusterNamespaceSuggestions(
  cluster: K8sCluster | null | undefined,
  liveNamespaces: K8sNamespace[],
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  const push = (ns: string | null | undefined): void => {
    const clean = (ns ?? '').trim();
    if (!clean || seen.has(clean)) return;
    seen.add(clean);
    out.push(clean);
  };
  if (cluster?.default_namespace) push(cluster.default_namespace);
  for (const ns of cluster?.known_namespaces ?? []) push(ns);
  for (const row of liveNamespaces) push(row.name);
  if (out.length === 0 && cluster) push('default');
  return out;
}

export function formatBridgeToolbarLabel(cfg?: IdeNetworkBridgeConfig | null): string | null {
  if (!cfg?.enabled) return null;
  const hasK8s = Boolean(cfg.k8s_cluster_id?.trim());
  const ns = cfg.k8s_namespace?.trim() || 'default';
  const hasSsh = Boolean(cfg.ssh_tunnel_connection_id?.trim());
  if (hasK8s && hasSsh) return `K8s · ${ns} + SSH`;
  if (hasK8s) return `K8s · ${ns}`;
  if (hasSsh) return 'SSH tunnel';
  return 'Net bridge';
}

export function isProdEnvironment(env?: string | null): boolean {
  return Boolean(env?.trim().toLowerCase().startsWith('prod'));
}

export function normalizeBridgeDraftSnapshot(draft: IdeNetworkBridgeDraft): {
  enabled: boolean;
  k8s_cluster_id: string;
  k8s_namespace: string;
  ssh_tunnel_connection_id: string;
  inject_k8s_service_env: boolean;
  containerized: boolean;
  mappings: Array<{ from_host: string; from_port: string; to_host: string; to_port: string }>;
} {
  return {
    enabled: Boolean(draft.enabled),
    k8s_cluster_id: draft.k8s_cluster_id.trim(),
    k8s_namespace: draft.k8s_namespace.trim(),
    ssh_tunnel_connection_id: draft.ssh_tunnel_connection_id.trim(),
    inject_k8s_service_env: Boolean(draft.inject_k8s_service_env),
    containerized: Boolean(draft.containerized),
    mappings: draft.mappings
      .map((m) => ({
        from_host: m.from_host.trim(),
        from_port: m.from_port.trim(),
        to_host: m.to_host.trim(),
        to_port: m.to_port.trim(),
      }))
      .filter((m) => Boolean(m.from_host || m.from_port || m.to_host || m.to_port)),
  };
}

export interface ProdBridgeTargetInfo {
  isProd: boolean;
  clusterProd: K8sCluster | null;
  sshProd: Connection | null;
  summary: string;
}

export function describeProdBridgeTarget(
  cfg: Pick<IdeNetworkBridgeConfig, 'k8s_cluster_id' | 'k8s_namespace' | 'ssh_tunnel_connection_id'> | null | undefined,
  clusters: readonly K8sCluster[],
  sshConns: readonly Connection[],
): ProdBridgeTargetInfo {
  if (!cfg) {
    return { isProd: false, clusterProd: null, sshProd: null, summary: '' };
  }
  const cid = cfg.k8s_cluster_id?.trim() || '';
  const sid = cfg.ssh_tunnel_connection_id?.trim() || '';
  const cluster = cid ? (clusters.find((c) => c.id === cid) ?? null) : null;
  const ssh = sid ? (sshConns.find((c) => c.id === sid) ?? null) : null;
  const clusterProd = cluster && isProdEnvironment(cluster.environment) ? cluster : null;
  const sshProd = ssh && isProdEnvironment(ssh.environment) ? ssh : null;
  const parts: string[] = [];
  if (clusterProd) {
    const ns = cfg.k8s_namespace?.trim() || clusterProd.default_namespace?.trim() || 'default';
    parts.push(`Kubernetes cluster "${clusterProd.name}" (${ns})`);
  }
  if (sshProd) {
    parts.push(`SSH bastion "${sshProd.name}"`);
  }
  return {
    isProd: Boolean(clusterProd || sshProd),
    clusterProd,
    sshProd,
    summary: parts.join(' and '),
  };
}
