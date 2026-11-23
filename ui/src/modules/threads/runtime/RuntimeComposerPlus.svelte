<script lang="ts">
  // The composer's one "+": attach a file or image (the same intake as paste
  // and drop), mention a file or an agent (types "@"), Plan mode, and the
  // thread's access — the controls that left the composer's bottom row,
  // which shows them only while they differ from the calm default.
  import type { RuntimeAccessMode } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { accessMenu } from './accessModes';

  interface Props {
    canImages: boolean;
    canFiles: boolean;
    /** Plan mode is offered (`caps.plan`) and whether it is on. */
    canPlan: boolean;
    planOn: boolean;
    /** Null: the access cannot change here (read-only thread, or the host fixes it). */
    access: RuntimeAccessMode | null;
    agent: string;
    cwd: string;
    disabled: boolean;
    attachTestId: string;
    onfiles: (files: FileList) => void;
    onmention: () => void;
    onplan: (on: boolean) => void;
    onaccess: (m: RuntimeAccessMode) => void;
  }
  let { canImages, canFiles, canPlan, planOn, access, agent, cwd, disabled, attachTestId, onfiles, onmention, onplan, onaccess }: Props = $props();
  let fileEl = $state<HTMLInputElement | null>(null);

  function open(e: MouseEvent): void {
    const items: MenuItem[] = [
      ...(canImages ? [{ label: canFiles ? 'Attach an image or PDF…' : 'Attach an image…', icon: 'image', hint: 'or paste / drop', action: () => fileEl?.click() }] : []),
      { label: 'Mention a file or agent', icon: 'link', hint: '@', action: onmention },
      ...(canPlan ? [{ separator: true }, { label: 'Plan mode', icon: 'compass', hint: '⇧⇥', checked: planOn, title: 'Plan first: the agent researches and proposes, and changes nothing', action: () => onplan(!planOn) }] : []),
      ...(access ? [{ separator: true }, ...accessMenu(access, agent, cwd, onaccess).map((i) => ({ ...i, label: `Access: ${i.label}` }))] : []),
    ];
    ctxMenu.showAt(e.currentTarget as HTMLElement, items);
  }
</script>

<button
  type="button"
  class="icon-btn rt-plus"
  aria-haspopup="menu"
  aria-label="Attach, mention, plan and access"
  title="Attach a file or image, mention a file or agent, Plan mode, access"
  data-testid="runtime-plus"
  {disabled}
  onclick={open}
>
  <Icon name="plus" size={16} />
</button>
<input
  bind:this={fileEl}
  type="file"
  accept={canFiles ? 'image/*,application/pdf' : 'image/*'}
  multiple
  hidden
  data-testid={attachTestId}
  onchange={(e) => {
    if (e.currentTarget.files) onfiles(e.currentTarget.files);
    e.currentTarget.value = '';
  }}
/>

<style>
  .rt-plus {
    inline-size: 30px;
    block-size: 30px;
    border-radius: 50%;
  }
  @media (pointer: coarse) {
    .rt-plus {
      inline-size: var(--hit-min);
      block-size: var(--hit-min);
    }
  }
</style>
