///<reference types="svelte" />
;// `#/plugin/<slug>` as a lazily loaded page (shell/pages.svelte.ts; a slug-less
// route is Agents there). Keyed by slug: each plugin is its own iframe
// origin + bridge, so switching plugins must remount the frame.

import PluginFrame from '../modules/plugins/PluginFrame.svelte';
import { router } from '../lib/router.svelte';
function $$render() {

  
  
  
  
  
;
async () => {

router.parts[1]; {
   { const $$_emarFnigulP0C = __sveltets_2_ensureComponent(PluginFrame); new $$_emarFnigulP0C({ target: __sveltets_2_any(), props: {  "slug":router.parts[1] ?? '',}});}
}
};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const PluginRoute__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type PluginRoute__SvelteComponent_ = InstanceType<typeof PluginRoute__SvelteComponent_>;
/*Ωignore_endΩ*/export default PluginRoute__SvelteComponent_;