import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const k8sStore = lazyModule(() => import('../../stores/k8s.svelte').then((m) => m.k8s), 'k8s');
const awsStore = lazyModule(() => import('../../stores/aws.svelte').then((m) => m.aws), 'aws');

export const cloudRoutes: EventRoutes = {
  // Kubernetes console: cluster list refetch / installer state tick. The
  // console and the Home box load both on mount: a document without the store
  // has nothing to refresh.
  k8s_cluster_updated: (parsed) => k8sStore.peek()?.applyEvent(parsed),
  k8s_install_updated: (parsed) => k8sStore.peek()?.applyEvent(parsed),
  k8s_monitor_cycle: (parsed) => k8sStore.peek()?.applyEvent(parsed),
  // AWS console: account rows changed / the CLI installer advanced.
  aws_account_updated: (parsed) => awsStore.peek()?.applyEvent(parsed),
  aws_install_updated: (parsed) => awsStore.peek()?.applyEvent(parsed),
};
