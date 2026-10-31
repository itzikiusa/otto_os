///<reference types="svelte" />
;// The cluster workspace's Resources | Monitor switch (K-2). Both views are
// one workspace: the same cluster stays selected, the console's row cache
// and namespace are kept, and each view resumes where it was left (the
// Monitor's state lives per cluster in the k8s store).

import { onTabKey } from '../../lib/tabKeys';
import { router } from '../../lib/router.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import { monitorPath, resourcesPath } from '../../lib/k8s/viewState';
;

  interface Props {
    clusterId: string;
    view: 'resources' | 'monitor';
  };function $$render() {

  
  
  
  
  
  
  
  

  let { clusterId, view }: Props = $props();

  function go(to: 'resources' | 'monitor'): void {
    if (to === view) return;
    if (to === 'monitor') {
      router.go(monitorPath(clusterId));
      return;
    }
    // Back to the console exactly as it was: kind + selected row.
    const sel = k8s.clusterId === clusterId ? k8s.selected : null;
    router.go(resourcesPath(clusterId, k8s.kind, sel?.ns, sel?.name));
  }
;
async () => {

 { svelteHTML.createElement("div", {       "class":`segmented k8s-view-switch`,"role":`tablist`,"aria-label":`Cluster view`,...__sveltets_2_empty({"data-testid":`k8s-view-switch`}),});
   { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":view === 'resources',"tabindex":view === 'resources' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => go('resources'),...__sveltets_2_empty({"data-testid":`k8s-view-resources`}),});view === 'resources';  }
   { svelteHTML.createElement("button", {              "role":`tab`,"aria-selected":view === 'monitor',"tabindex":view === 'monitor' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => go('monitor'),"title":`Metrics, restarts and health history for this cluster`,...__sveltets_2_empty({"data-testid":`k8s-view-monitor`}),});view === 'monitor';  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const K8sViewSwitch__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type K8sViewSwitch__SvelteComponent_ = ReturnType<typeof K8sViewSwitch__SvelteComponent_>;
/*Ωignore_endΩ*/export default K8sViewSwitch__SvelteComponent_;