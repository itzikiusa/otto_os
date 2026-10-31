///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { announceShown } from '../../lib/announce.svelte';
// Settings → Trust & safety → "Secret storage" (root only).
// Shows which store holds integration secrets (GET /admin/secrets/status)
// and, while they sit in the PLAINTEXT secrets.json, a warning banner plus
// the explicit "Secure secrets…" action (POST /admin/secrets/secure): every
// secret is copied into secrets.enc (sealed with one master key kept in the
// macOS Keychain), verified to read back, and only then is the plaintext
// file wiped and deleted. Never automatic — the Keychain may prompt.
import { onMount } from 'svelte';
import { api } from '../../lib/api/client';
import type { SecretsStatus, SecretsMigrationReport, SecretsResetStoreResp } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let status = $state<SecretsStatus | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let securing = $state(false);
  let resetting = $state(false);
  let actionError = $state('');
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function load() {
    loading = true;
    loadError = '';
    try {
      status = await api.get<SecretsStatus>('/admin/secrets/status');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(load);

  const MODE_LABEL: Record<SecretsStatus['mode'], string> = {
    plaintext: 'Plaintext file (not encrypted)',
    encrypted: 'Encrypted file, key in the macOS Keychain',
    keychain: 'macOS Keychain (one item per secret)',
  };
  const KEY_LABEL: Record<SecretsStatus['key_state'], string> = {
    unlocked: 'Unlocked',
    locked: 'Waiting for the Keychain — unlock it or approve the prompt',
    not_loaded: 'Not loaded yet',
    error: 'Keychain refused access',
  };

  async function secure() {
    if (!status) return;
    const n = status.plaintext_entries;
    const ok = await confirmer.ask(
      `Encrypt ${plural(n, 'stored secret')} (connection passwords, Slack/Telegram tokens, accounts)? ` +
        'Otto creates one encryption key in your macOS Keychain — macOS may ask you to allow access; choose “Always Allow”. ' +
        'Every secret is checked to read back from the encrypted store before the plaintext file is wiped and deleted. ' +
        'If anything fails, nothing changes. Integrations keep working; no restart needed.',
      { title: 'Secure secrets', confirmLabel: 'Encrypt secrets' },
    );
    if (!ok) return;
    securing = true;
    actionError = '';
    try {
      const r = await api.post<SecretsMigrationReport>('/admin/secrets/secure', { confirm: true });
      toasts.success('Secrets encrypted', `${plural(r.migrated, 'secret')} moved; the plaintext file was deleted.`);
      await load();
    } catch (e) {
      // A failure AFTER the switch (final live check) leaves the daemon on the
      // encrypted store with the plaintext already removed — "not changed"
      // would be false there. Re-read the status to tell the two apart.
      await load();
      const after = status as SecretsStatus | null;
      actionError =
        after && after.mode !== 'plaintext'
          ? `Secrets were encrypted and the plaintext file was removed, but a final check failed.` +
            (after.backup_present ? ' The encrypted backup secrets.migrate-backup.enc was kept.' : '') +
            ` ${msg(e)}`
          : `Secrets were not changed. ${msg(e)}`;
    } finally {
      securing = false;
    }
  }

  // Recovery for an orphaned encrypted store (S7-305): its master key is gone
  // from the Keychain, so every secret save fails. The daemon refuses (409)
  // while the file is still readable, so this can't discard working secrets.
  async function resetStore() {
    const ok = await confirmer.ask(
      'Use this only when saving a secret fails because the encryption key is missing from your macOS Keychain. ' +
        'Otto moves the unreadable secrets.enc aside to secrets.enc.orphaned-<time> in its data folder. The file is kept, ' +
        'not deleted, and restoring the old “Otto” Keychain item makes it readable again. Secrets saved in it stop working ' +
        'until you enter them again. If the store is still readable, nothing changes.',
      { title: 'Reset secret store', confirmLabel: 'Move file aside', danger: true },
    );
    if (!ok) return;
    resetting = true;
    actionError = '';
    try {
      const r = await api.post<SecretsResetStoreResp>('/admin/secrets/reset-store', { confirm: true });
      toasts.success(
        'Secret store reset',
        r.set_aside ? `The unreadable file was kept at ${r.set_aside}. Re-enter your secrets.` : 'There was no secret file to move.',
      );
      await load();
    } catch (e) {
      actionError = `The secret store was not changed. ${msg(e)}`;
    } finally {
      resetting = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`secrets-card`,"aria-label":`Secret storage`,});
   { svelteHTML.createElement("h2", { "class":`card-title`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":14,}});}   }
  if(!status){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`the secret store status`,loading,"error":loadError,"empty":true,"onretry":load,}});}
  }else{
    if(status.mode === 'plaintext'){
      
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(status.plaintext_entries > 0 ? 'Secrets are stored unencrypted.' : 'The plaintext secret store is active.')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`banner`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
        if(status.plaintext_entries > 0){
           { svelteHTML.createElement("p", {});
             { svelteHTML.createElement("strong", {});    }
            status.plaintext_entries; status.plaintext_entries === 1 ? ' is' : 's are';     
                             
           }
        }else{
           { svelteHTML.createElement("p", {});
             { svelteHTML.createElement("strong", {});      }
                       { svelteHTML.createElement("code", {});  }
           }
        }
       }}
    }
     { svelteHTML.createElement("dl", { "class":`facts`,});
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});MODE_LABEL[status.mode]; } }
      if(status.mode === 'encrypted'){
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { });status.key_state === 'locked' || status.key_state === 'error';KEY_LABEL[status.key_state]; } }
      }
     }
    if(status.backup_present){
       { svelteHTML.createElement("p", { "class":`dim`,});            }
    }
    if(status.migration_available){
       { svelteHTML.createElement("div", { "class":`controls`,});
         { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":securing || status.migrating,"onclick":secure,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":13,}});}
          securing || status.migrating ? 'Encrypting…' : 'Secure secrets…';
         }
       }
    }
    if(status.mode === 'encrypted'){
       { svelteHTML.createElement("div", { "class":`controls`,});
         { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":resetting,"onclick":resetStore,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
          resetting ? 'Resetting…' : 'Reset secret store…';
         }
       }
    }
    if(actionError){
       { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});actionError; }
    }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SecretsStore__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SecretsStore__SvelteComponent_ = ReturnType<typeof SecretsStore__SvelteComponent_>;
/*Ωignore_endΩ*/export default SecretsStore__SvelteComponent_;