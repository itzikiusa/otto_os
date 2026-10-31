///<reference types="svelte" />
;// "Send to…" for the active workbench file: DB Run on… (multi-target /
// parameter sweep), API client request, paste into a session, save as a
// vault note, copy with placeholders filled. Every target only prepares —
// see sendTo.ts.

import Icon from '../../../lib/components/Icon.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import type { Session, WorkbenchDocFull } from '../../../lib/api/types';
import SessionPickerModal from './SessionPickerModal.svelte';
import {
    copyFilled,
    looksLikeCurl,
    pasteIntoSession,
    sendToApiClient,
    sendToDatabase,
    sendToVault,
    type SendCtx,
  } from './sendTo';
;

  interface Props {
    ws: string;
    doc: WorkbenchDocFull;
    content: string;
    lang: string;
    values: Record<string, string>;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { ws, doc, content, lang, values }: Props = $props();

  let picking = $state(false);
  const ctx = (): SendCtx => ({ ws, doc, content, lang, values });
  const empty = $derived(!content.trim());
  const isImage = $derived(lang === 'image');

  function open(e: MouseEvent): void {
    const items: MenuItem[] = [
      {
        label: 'Database — Run on…',
        icon: 'db',
        hint: lang === 'sql' ? 'targets × values' : undefined,
        disabled: empty || isImage,
        title: empty ? 'The file is empty' : 'Open the script in the DB Explorer’s Run on… sheet (nothing runs yet)',
        action: () => void sendToDatabase(ctx()),
      },
      {
        label: looksLikeCurl(content) ? 'API client — import curl' : 'API client — new request',
        icon: 'send',
        disabled: empty || isImage,
        action: () => void sendToApiClient(ctx()),
      },
      {
        label: 'Terminal / agent session…',
        icon: 'terminal',
        disabled: empty || isImage,
        title: 'Paste into a live session — never executed',
        action: () => (picking = true),
      },
      {
        label: 'Vault — save as note…',
        icon: 'book',
        disabled: empty || isImage,
        action: () => void sendToVault(ctx()),
      },
      { separator: true },
      {
        label: 'Copy with placeholders filled',
        icon: 'copy',
        disabled: empty || isImage,
        action: () => void copyFilled(ctx()),
      },
    ];
    ctxMenu.show(e, items);
  }

  async function pick(s: Session): Promise<void> {
    picking = false;
    await pasteIntoSession(ctx(), s);
  }
;
async () => {

  { svelteHTML.createElement("button", {            "class":`btn wb-sendto`,"type":`button`,...__sveltets_2_empty({"data-testid":`wb-sendto`}),"aria-haspopup":`menu`,"title":`Send this file to the DB Explorer, API client, a session or the vault`,"onclick":open,});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`share`,"size":14,}});}
   { svelteHTML.createElement("span", {});  }
 }

if(picking){
   { const $$_ladoMrekciPnoisseS0C = __sveltets_2_ensureComponent(SessionPickerModal); new $$_ladoMrekciPnoisseS0C({ target: __sveltets_2_any(), props: {     ws,"onpick":(s) => void pick(s),"onclose":() => (picking = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SendToButton__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SendToButton__SvelteComponent_ = ReturnType<typeof SendToButton__SvelteComponent_>;
/*Ωignore_endΩ*/export default SendToButton__SvelteComponent_;