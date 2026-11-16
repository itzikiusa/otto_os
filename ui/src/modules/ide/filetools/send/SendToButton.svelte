<script lang="ts">
  // "Send to…" for the open IDE file: DB Run on…, API client request, paste
  // into a session, save as a vault note. Every target only prepares — see
  // sendTo.ts.
  import Icon from '../../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../../lib/contextmenu.svelte';
  import type { Session } from '../../../../lib/api/types';
  import { ws as wsStore } from '../../../../lib/stores/workspace.svelte';
  import SessionPickerModal from './SessionPickerModal.svelte';
  import type { SendCtx } from './sendTo';
  import { isMultiLine } from './pasteSafety';

  // The targets (DB / API / vault stores) load on first use, not with the editor.
  const send = () => import('./sendTo');

  interface Props {
    name: string;
    content: string;
    lang: string;
  }
  let { name, content, lang }: Props = $props();

  let picking = $state(false);
  const ctx = (): SendCtx => ({ name, content, lang });
  const empty = $derived(!content.trim());

  function open(e: MouseEvent): void {
    const items: MenuItem[] = [
      {
        label: 'Database — Run on…',
        icon: 'db',
        disabled: empty,
        title: empty ? 'The file is empty' : 'Open the script in the DB Explorer’s Run on… sheet (nothing runs yet)',
        action: () => void send().then((m) => m.sendToDatabase(ctx())),
      },
      {
        label: /^\s*curl\s/.test(content) ? 'API client — import curl' : 'API client — new request',
        icon: 'send',
        disabled: empty,
        action: () => void send().then((m) => m.sendToApiClient(ctx())),
      },
      {
        label: 'Terminal / agent session…',
        icon: 'terminal',
        disabled: empty,
        title: 'Paste into a live session — never executed (one line only for ssh / database sessions)',
        action: () => (picking = true),
      },
      {
        label: 'Vault — save as note…',
        icon: 'book',
        disabled: empty,
        action: () => void send().then((m) => m.sendToVault(ctx())),
      },
    ];
    ctxMenu.show(e, items);
  }

  async function pick(s: Session): Promise<void> {
    picking = false;
    await (await send()).pasteIntoSession(ctx(), s);
  }
</script>

<button
  type="button"
  class="icon-btn"
  data-testid="ide-send-to"
  aria-haspopup="menu"
  aria-label="Send to…"
  title="Send this file to the DB Explorer, API client, a session or the vault"
  onclick={open}
>
  <Icon name="share" size={13} />
</button>

{#if picking}
  <SessionPickerModal ws={wsStore.currentId ?? ''} multiLine={isMultiLine(content)} onpick={(s) => void pick(s)} onclose={() => (picking = false)} />
{/if}
