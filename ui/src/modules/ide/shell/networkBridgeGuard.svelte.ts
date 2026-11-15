// Reactive cluster + SSH environment cache and production confirmation gate
// for IDE Network Bridge surfaces (`IdeNetworkBridgeCard`, `IdeRunToolbar`,
// `IdeEnvironmentsModal`, `IdeSshTargetModal`, and `IdeStore` Run/Debug).

import { api } from '../../../lib/api/client';
import type { Connection, K8sCluster } from '../../../lib/api/types';
import { confirmer } from '../../../lib/confirm.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import type { IdeNetworkBridgeConfig } from '../store/ideTypes';
import { describeProdBridgeTarget, type ProdBridgeTargetInfo } from './networkBridgeModel';

class BridgeEnvironmentCache {
  clusters = $state<K8sCluster[]>([]);
  sshConns = $state<Connection[]>([]);
  private clustersLoaded = false;
  private loadedSshWorkspaces = new Set<string>();
  private inflight: Promise<void> | null = null;

  seedClusters(list: readonly K8sCluster[]): void {
    this.clusters = [...list];
    this.clustersLoaded = true;
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
    if (workspaceId) this.loadedSshWorkspaces.add(workspaceId);
  }

  async ensureLoaded(workspaceId?: string | null): Promise<void> {
    const scopes = Array.from(
      new Set(
        [
          workspaceId,
          ws.currentId,
          ...ws.workspaces.map((w) => w.id),
          ws.scratch?.id ?? SCRATCH_WORKSPACE_ID,
        ].filter((id): id is string => Boolean(id)),
      ),
    );
    if (scopes.length === 0) scopes.push(SCRATCH_WORKSPACE_ID);
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
            })
            .catch(() => {}),
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
              for (let i = 0; i < results.length; i++) {
                const r = results[i];
                if (r.status !== 'fulfilled') continue;
                anyFulfilled = true;
                this.loadedSshWorkspaces.add(r.value.id);
                for (let j = 0; j < r.value.rows.length; j++) {
                  const c = r.value.rows[j];
                  if (c.kind === 'ssh') seen.set(c.id, c);
                }
              }
              if (anyFulfilled) {
                this.sshConns = Array.from(seen.values());
              }
            })
            .catch(() => {}),
        );
      }
      await Promise.all(tasks);
    })().finally(() => {
      this.inflight = null;
    });
    this.inflight = p;
    return p;
  }

  inspect(cfg?: Pick<IdeNetworkBridgeConfig, 'k8s_cluster_id' | 'k8s_namespace' | 'ssh_tunnel_connection_id'> | null): ProdBridgeTargetInfo {
    return describeProdBridgeTarget(cfg, this.clusters, this.sshConns);
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
