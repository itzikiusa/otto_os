// k9s launch mode. On a production cluster the daemon starts k9s `--readonly`
// unless a person asks for write access with `confirm_name` = the cluster name;
// the toolbar button therefore offers read-only first and write access only
// behind the typed prod confirm.

import { confirmProd, isProdEnv } from '../../lib/confirmProd';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import type { K8sCluster } from '../../lib/api/types';
import { clusterLabel } from './k8s-util';

/** Extra `POST …/k9s` fields for the chosen mode. */
export interface K9sMode {
  write?: boolean;
  confirm_name?: string | null;
}

/** Off prod: open straight away (k9s has the cluster RBAC's own access).
 *  On prod: a menu — read-only (the default) or write access after typing
 *  the cluster name. */
export function chooseK9sMode(e: MouseEvent, cluster: K8sCluster, open: (mode: K9sMode) => void): void {
  if (!isProdEnv(cluster.environment)) return open({});
  ctxMenu.show(e, [
    { label: 'Open k9s (read-only)', icon: 'terminal', action: () => open({}) },
    {
      label: 'Open k9s with write access…',
      icon: 'warning',
      danger: true,
      action: () => void confirmK9sWrite(cluster).then((ok) => ok && open({ write: true, confirm_name: cluster.name })),
    },
  ]);
}

/** The typed prod confirm for k9s write mode. */
export function confirmK9sWrite(cluster: K8sCluster): Promise<boolean> {
  return confirmProd({
    env: cluster.environment,
    where: clusterLabel(cluster),
    verb: 'Open k9s with write access',
    what: 'k9s can delete, scale and edit anything your kubeconfig allows on this cluster.',
    typed: cluster.name,
    danger: true,
  });
}
