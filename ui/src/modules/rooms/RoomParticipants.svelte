<script lang="ts">
  import type { RoomSnapshot, RoomAction, RoomMember } from '../../lib/api/room-types';
  import { tick } from 'svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  let { room, send, disabled = false }: {room: RoomSnapshot; send: (action: RoomAction) => boolean; disabled?: boolean} = $props();
  const host = $derived(room.member_id === room.host_member_id);
  async function grant(member: RoomMember) {
    if (await confirmer.ask(`${member.name} will be able to run commands with this session’s permissions. You can take back control at any time.`, {title: `Give ${member.name} control?`, confirmLabel: 'Give control'})) send({type: 'grant_control', member_id: member.id});
  }
  async function remove(member: RoomMember) {
    if (await confirmer.ask(`${member.name} will lose access to the terminal, chat and media immediately.`, {title: 'Remove participant?', confirmLabel: 'Remove', danger: true})) send({type: 'remove', member_id: member.id});
  }
  // Admit / Decline replace (or remove) the very button that was pressed once
  // the server echoes the admission. Remember whose it was, then put focus on
  // that member's new first control — or the People heading when they're gone.
  let listEl = $state<HTMLUListElement | null>(null);
  let headingEl = $state<HTMLHeadingElement | null>(null);
  let refocusMember = $state<string | null>(null);
  function decide(member: RoomMember, action: RoomAction): void {
    if (send(action)) refocusMember = member.id;
  }
  $effect(() => {
    const id = refocusMember;
    if (!id) return;
    const member = room.members?.find((m) => m.id === id);
    if (member?.admission === 'pending') return; // the echo hasn't landed yet
    refocusMember = null;
    void tick().then(() => {
      const row = member ? listEl?.querySelector<HTMLElement>(`[data-member="${CSS.escape(id)}"]`) : null;
      const next = row?.querySelector<HTMLElement>('.actions select, .actions button');
      (next ?? headingEl)?.focus();
    });
  });
  async function changeRole(member: RoomMember, next: 'viewer' | 'editor', select: HTMLSelectElement): Promise<void> {
    if (next === member.role) return;
    if (next === 'editor') {
      const ok = await confirmer.ask(
        `${member.name} may later be given control of the terminal. They cannot type until you give control.`,
        {title: `Let ${member.name} be granted control?`, confirmLabel: 'Allow'},
      );
      if (!ok) { select.value = member.role; return; }
    }
    if (!send({type: 'role', member_id: member.id, role: next})) select.value = member.role;
  }
</script>
<section aria-label="Participants">
  <h2 bind:this={headingEl} tabindex="-1">People <span>{room.members?.filter(m => m.admission === 'admitted').length ?? 1}/4</span></h2>
  <p class="hint">Display names are chosen by participants.</p>
  <ul bind:this={listEl}>{#each room.members ?? [] as member (member.id)}
    <li data-member={member.id}>
      <div class="identity"><strong>{member.name}{member.id === room.member_id ? ' (you)' : ''}</strong>
        <span>{member.admission === 'pending' ? 'Waiting for admission' : member.role === 'host' ? 'Host' : member.role === 'editor' ? 'May be granted control' : 'View only'} · {member.connected ? 'Connected' : 'Reconnecting'}</span>
        {#if room.driver_member_id === member.id}<span class="driver">Controls terminal</span>{/if}
        {#if member.audio_joined}<span>{member.room_muted ? 'Muted by host' : member.muted ? 'Microphone muted' : 'Microphone on'}</span>{/if}
      </div>
      {#if host && member.id !== room.member_id}<div class="actions">
        {#if member.admission === 'pending'}
          <button class="btn small" {disabled} onclick={() => decide(member, {type: 'admit', member_id: member.id, role: 'viewer'})}>Admit view only</button>
          {#if member.role === 'editor'}<button class="btn small" {disabled} onclick={() => decide(member, {type: 'admit', member_id: member.id, role: 'editor'})}>Admit — may be granted control</button>{/if}
          <button class="btn small" {disabled} onclick={() => decide(member, {type: 'reject', member_id: member.id})}>Decline</button>
        {:else}
          <label>Access <select {disabled} value={member.role} onchange={(event) => void changeRole(member, event.currentTarget.value === 'editor' ? 'editor' : 'viewer', event.currentTarget)}><option value="viewer">View only</option><option value="editor">May be granted control</option></select></label>
          {#if member.control_requested}<button class="btn small" {disabled} onclick={() => grant(member)}>Give control…</button>{/if}
          {#if member.audio_joined}<button class="btn small" {disabled} onclick={() => send({type: 'mute', member_id: member.id, muted: !member.room_muted})}>{member.room_muted ? 'Allow microphone' : 'Mute in room'}</button>{/if}
          {#if member.presenter_requested}<button class="btn small" {disabled} onclick={() => send({type: 'grant_present', member_id: member.id, allowed: true})}>Allow presentation</button>{/if}
          {#if member.presenter_allowed}<button class="btn small" {disabled} onclick={() => send({type: 'grant_present', member_id: member.id, allowed: false})}>Stop presenting access</button>{/if}
          <button class="btn small danger" {disabled} onclick={() => remove(member)}>Remove…</button>
        {/if}
      </div>{/if}
    </li>
  {/each}</ul>
</section>
<style>
  section { padding: 16px; border-bottom: 1px solid var(--border); }
  h2 { font-size: var(--fs-m); margin: 0; display: flex; justify-content: space-between; } h2 span, .hint, .identity span { color: var(--text-dim); }
  .hint { font-size: var(--fs-s); margin-block: 8px 12px; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 16px; }
  li, .identity { display: grid; gap: 4px; min-width: 0; overflow-wrap: anywhere; }
  .identity span { font-size: var(--fs-s); } .identity .driver { color: var(--accent-text); }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; margin-block-start: 4px; }
  label { display: flex; gap: 8px; align-items: center; font-size: var(--fs-s); }
</style>
