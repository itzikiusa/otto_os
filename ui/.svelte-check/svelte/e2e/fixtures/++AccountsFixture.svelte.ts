///<reference types="svelte" />
;
import AccountPicker from '../../src/lib/components/AccountPicker.svelte';
function $$render() {

  
  let selected = $state('');
;
async () => {
 { const $$_rekciPtnuoccA0C = __sveltets_2_ensureComponent(AccountPicker); new $$_rekciPtnuoccA0C({ target: __sveltets_2_any(), props: {        "provider":`codex`,"workspaceId":`w`,"value":selected,"onchange":(id) => selected = id,}});}
 { svelteHTML.createElement("output", { "aria-label":`Selected account`,});selected || 'default'; }
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AccountsFixture__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AccountsFixture__SvelteComponent_ = ReturnType<typeof AccountsFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default AccountsFixture__SvelteComponent_;