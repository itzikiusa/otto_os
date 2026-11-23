// Reactive cluster + SSH environment cache and production confirmation gate
// for IDE Network Bridge surfaces (`IdeNetworkBridgeCard`, `IdeRunToolbar`,
// `IdeEnvironmentsModal`, `IdeSshTargetModal`, and `IdeStore` Run/Debug).

import { api } from '../../../lib/api/client';
import type { Connection, K8sCluster } from '../../../lib/api/types';
import { confirmer } from '../../../lib/confirm.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import type { IdeNetworkBridgeConfig } from '../store/ideTypes';
import { describeProdBridgeTarget, isProdEnvironment, type ProdBridgeTargetInfo } from './networkBridgeModel';

class BridgeEnvironmentCache {
  clusters = $state<K8sCluster[]>([]);
  sshConns = $state<Connection[]>([]);
  private clustersLoaded = false;
  private clustersLoadFailed = false;
  private sshLoadFailed = false;
  private loadedSshWorkspaces = new Set<string>();
  private inflight: Promise<void> | null = null;

  seedClusters(list: readonly K8sCluster[]): void {
    this.clusters = [...list];
    this.clustersLoaded = true;
    this.clustersLoadFailed = false;
  }

  seedSshConns(list: readonly Connection[], workspaceId?: string | null): void {
    const byId = new Map<string, Connection>();
    for (let i = 0; i < this.sshConns.length; i++) {
      byId.set(this.sshConns[i].id, this.sshConns[i]);
    }
    for (let i = 0; i < list.length; i++) {
      if (list[i].kind === 'ssh') byId.set(list[i].id, list[i]);
    }
    this.sshConns = Array.from(byId.values());
    this.sshLoadFailed = false;
    if (workspaceId) this.loadedSshWorkspaces.add(workspaceId);
  }

  async ensureLoaded(workspaceId?: string | null): Promise<void> {
    const scopes = Array.from(
      new Set(
        [
          workspaceId,
          ws.currentId,
          ...ws.workspaces.map((w) => w.id),
        ].filter((id): id is string => Boolean(id) && id !== SCRATCH_WORKSPACE_ID),
      ),
    );
    const missingScopes = scopes.filter((id) => !this.loadedSshWorkspaces.has(id));
    if (this.clustersLoaded && missingScopes.length === 0) return;
    if (this.inflight) {
      await this.inflight;
      const stillMissing = scopes.filter((id) => !this.loadedSshWorkspaces.has(id));
      if (this.clustersLoaded && stillMissing.length === 0) return;
    }
    const p = (async () => {
      const tasks: Promise<void>[] = [];
      if (!this.clustersLoaded) {
        tasks.push(
          api
            .get<K8sCluster[]>('/k8s/clusters')
            .then((res) => {
              this.clusters = res ?? [];
              this.clustersLoaded = true;
              this.clustersLoadFailed = false;
            })
            .catch(() => {
              this.clustersLoadFailed = true;
            }),
        );
      }
      if (missingScopes.length > 0) {
        tasks.push(
          Promise.allSettled(
            missingScopes.map((id) =>
              api.get<Connection[]>(`/workspaces/${encodeURIComponent(id)}/connections`).then((rows) => ({ id, rows })),
            ),
          )
            .then((results) => {
              const seen = new Map<string, Connection>();
              for (let i = 0; i < this.sshConns.length; i++) {
                seen.set(this.sshConns[i].id, this.sshConns[i]);
              }
              let anyFulfilled = false;
              let anyRejected = false;
              for (let i = 0; i < results.length; i++) {
                const r = results[i];
                if (r.status !== 'fulfilled') {
                  anyRejected = true;
                  continue;
                }
                anyFulfilled = true;
                this.loadedSshWorkspaces.add(r.value.id);
                for (let j = 0; j < r.value.rows.length; j++) {
                  const c = r.value.rows[j];
                  if (c.kind === 'ssh') seen.set(c.id, c);
                }
              }
              this.sshLoadFailed = anyRejected;
              if (anyFulfilled) {
                this.sshConns = Array.from(seen.values());
              }
            })
            .catch(() => {
              this.sshLoadFailed = true;
            }),
        );
      }
      await Promise.all(tasks);
    })().finally(() => {
      this.inflight = null;
    });
    this.inflight = p;
    return p;
  }

  hasUnverifiedTarget(cfg?: Pick<IdeNetworkBridgeConfig, 'k8s_cluster_id' | 'ssh_tunnel_connection_id'> | null): boolean {
    if (!cfg) return false;
    const unknownCluster = Boolean(
      cfg.k8s_cluster_id &&
        this.clustersLoadFailed &&
        !this.clusters.some((c) => c.id === cfg.k8s_cluster_id || c.name === cfg.k8s_cluster_id),
    );
    const unknownSsh = Boolean(
      cfg.ssh_tunnel_connection_id &&
        this.sshLoadFailed &&
        !this.sshConns.some((c) => c.id === cfg.ssh_tunnel_connection_id || c.name === cfg.ssh_tunnel_connection_id),
    );
    return unknownCluster || unknownSsh;
  }

  inspect(cfg?: Pick<IdeNetworkBridgeConfig, 'k8s_cluster_id' | 'k8s_namespace' | 'ssh_tunnel_connection_id'> | null): ProdBridgeTargetInfo {
    const info = describeProdBridgeTarget(cfg, this.clusters, this.sshConns);
    if (!info.isProd && this.hasUnverifiedTarget(cfg)) {
      const targetName = cfg?.k8s_cluster_id || cfg?.ssh_tunnel_connection_id || 'remote target';
      return {
        isProd: true,
        clusterProd: null,
        sshProd: null,
        summary: `unverified target "${targetName}"`,
      };
    }
    return info;
  }

  isProdBridge(cfg?: IdeNetworkBridgeConfig | null): boolean {
    if (!cfg?.enabled) return false;
    return this.inspect(cfg).isProd;
  }
}

export const bridgeEnvCache = new BridgeEnvironmentCache();

export async function confirmProdNetworkBridge(
  cfg: Pick<IdeNetworkBridgeConfig, 'enabled' | 'k8s_cluster_id' | 'k8s_namespace' | 'ssh_tunnel_connection_id'> | null | undefined,
  mode: 'enable' | 'save' | 'launch',
  workspaceId?: string | null,
): Promise<boolean> {
  if (!cfg?.enabled) return true;
  await bridgeEnvCache.ensureLoaded(workspaceId);
  const info = bridgeEnvCache.inspect(cfg);
  if (!info.isProd) return true;

  if (mode === 'launch') {
    return confirmer.ask(
      `Active network bridge routes local Run & Debug traffic to production ${info.summary}. Outbound requests from this process can reach live production services.`,
      {
        title: 'Launch with production network bridge?',
        confirmLabel: 'Run against production',
        danger: true,
      },
    );
  }

  return confirmer.ask(
    `Route local Run & Debug traffic to production ${info.summary}? Local processes will resolve and connect directly to live production services.`,
    {
      title: mode === 'enable' ? 'Enable production network bridge?' : 'Save production network bridge?',
      confirmLabel: mode === 'enable' ? 'Enable production bridge' : 'Save production bridge',
      danger: true,
    },
  );
}

export async function confirmProdSshTarget(
  connectionId: string | null | undefined,
  workspaceId?: string | null,
): Promise<boolean> {
  if (!connectionId) return true;
  await bridgeEnvCache.ensureLoaded(workspaceId);
  const info = bridgeEnvCache.inspect({ ssh_tunnel_connection_id: connectionId });
  if (!info.isProd) return true;
  return confirmer.ask(
    `Execute directly on production host ${info.summary}? Commands and debuggers run live on the remote server.`,
    {
      title: 'Run on production SSH target?',
      confirmLabel: 'Run on production',
      danger: true,
    },
  );
}

/** `user@host[:port]` for an SSH profile. */
export function connEndpointText(conn: Connection): string {
  const p = conn.params ?? {};
  const host = String(p.host ?? p.hostname ?? 'localhost');
  const user = String(p.user ?? p.username ?? '').trim();
  const port = Number(p.port ?? 22);
  const addr = user ? `${user}@${host}` : host;
  return port && port !== 22 ? `${addr}:${port}` : addr;
}

/** The SSH user's home directory, the default remote project root. */
export function defaultRemoteRoot(conn: Connection): string {
  const user = String(conn.params?.user ?? conn.params?.username ?? '').trim();
  return !user || user === 'root' ? '/root' : `/home/${user}`;
}

/** An absolute remote root: empty / `~` / `~/x` resolve against the user's home
 *  (SFTP and the language servers take a literal `~` as a missing directory). */
export function absoluteRemoteRoot(conn: Connection, path: string): string {
  const p = path.trim();
  if (!p || p === '~') return defaultRemoteRoot(conn);
  return p.startsWith('~/') ? `${defaultRemoteRoot(conn)}/${p.slice(2)}` : p;
}

/** Ask before pointing the IDE at a production SSH host. */
export async function confirmProdSshConnect(conn: Connection, root: string): Promise<boolean> {
  if (!isProdEnvironment(conn.environment)) return true;
  return confirmer.ask(
    `Connect IDE to production SSH host "${conn.name}" (${connEndpointText(conn)}) at ${root}? File edits, commands, and debuggers will run directly on that host.`,
    { title: 'Connect to production SSH host?', confirmLabel: 'Connect to production', danger: true },
  );
}
