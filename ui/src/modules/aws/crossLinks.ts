// Pure cross-module deep-link helpers for the AWS Console:
//   - EKS → Kubernetes Console cluster matching (1-click jump when already imported)
//   - RDS → Connections / Database prefill builder (sessionStorage handoff, no cross-module import)
import type { Environment, RdsInstance } from '../../lib/api/types';

/** SessionStorage key shared between AWS RDS ("Open in Connections…") and
 *  DatabasePage (consumes once on mount and opens ConnectionForm pre-filled). */
export const RDS_PREFILL_STORAGE_KEY = 'otto.conn_prefill';

/** Minimal structural shape of a Kubernetes cluster row (keeps this module
 *  pure and decoupled from the k8s Svelte store for unit testing). */
export interface K8sClusterLike {
  id: string;
  name: string;
  source?: string;
  aws_account_id?: string | null;
  params?: Record<string, unknown>;
}

/** Match an EKS cluster row against the already-configured Kubernetes Console
 *  clusters so the operator can jump straight to `#/kubernetes/<id>` without
 *  re-running kubeconfig import. Prefers an exact `(account, region, cluster)`
 *  match, falling back to `(region, cluster)` for clusters imported without
 *  `aws_account_id`. */
export function findMatchingK8sCluster<T extends K8sClusterLike>(
  cluster: { name: string; region?: string | null },
  accountId: string,
  fallbackRegion: string,
  k8sClusters: readonly T[],
): T | null {
  const rg = (cluster.region ?? fallbackRegion).trim();
  const exact = k8sClusters.find(
    (k) =>
      k.source === 'eks' &&
      k.aws_account_id === accountId &&
      k.params?.eks_cluster === cluster.name &&
      (!k.params?.eks_region || k.params.eks_region === rg),
  );
  if (exact) return exact;
  return (
    k8sClusters.find(
      (k) => k.params?.eks_cluster === cluster.name && (!rg || k.params?.eks_region === rg),
    ) ?? null
  );
}

/** Prefill payload handed to Database / Connections when launching a new
 *  connection from an AWS RDS instance. */
export interface RdsConnectionPrefill {
  name: string;
  kind: 'postgres' | 'mysql';
  host: string;
  port: number;
  user?: string;
  db?: string;
  environment: Environment;
  tlsMode: 'required';
  source: 'aws_rds';
}

/** Map an AWS RDS engine identifier to an Otto connection kind, or `null` for
 *  engines Otto's Database Explorer doesn't speak (e.g. Oracle, SQL Server). */
export function rdsEngineToKind(engine: string | null | undefined): 'postgres' | 'mysql' | null {
  const e = (engine ?? '').trim().toLowerCase();
  if (!e) return null;
  if (e === 'postgres' || e === 'aurora-postgresql' || e.startsWith('postgres')) return 'postgres';
  if (e === 'mysql' || e === 'mariadb' || e === 'aurora-mysql' || e === 'aurora' || e.startsWith('mysql') || e.startsWith('mariadb')) {
    return 'mysql';
  }
  return null;
}

function normalizeEnv(env: string | null | undefined): Environment {
  return env === 'prod' || env === 'staging' || env === 'dev' ? env : 'dev';
}

/** Build a new-connection prefill for an RDS instance, or `null` if it has no
 *  endpoint yet or uses an unsupported engine. */
export function rdsToConnectionPrefill(
  inst: Pick<RdsInstance, 'identifier' | 'engine' | 'endpoint' | 'port' | 'db_name' | 'master_username'>,
  accountEnv?: string | null,
): RdsConnectionPrefill | null {
  const host = (inst.endpoint ?? '').trim();
  const kind = rdsEngineToKind(inst.engine);
  if (!host || !kind) return null;
  const port = typeof inst.port === 'number' && inst.port > 0 ? inst.port : kind === 'postgres' ? 5432 : 3306;
  const out: RdsConnectionPrefill = {
    name: inst.identifier,
    kind,
    host,
    port,
    environment: normalizeEnv(accountEnv),
    tlsMode: 'required',
    source: 'aws_rds',
  };
  if (inst.master_username?.trim()) out.user = inst.master_username.trim();
  if (inst.db_name?.trim()) out.db = inst.db_name.trim();
  return out;
}
