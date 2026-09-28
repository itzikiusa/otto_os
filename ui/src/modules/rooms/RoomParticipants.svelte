<script lang="ts">
  import type { RoomSnapshot, RoomAction, RoomMember } from '../../lib/api/room-types';
  import { confirmer } from '../../lib/confirm.svelte';
  let { room, send, disabled = false }: {room: RoomSnapshot; send: (action: RoomAction) => boolean; disabled?: boolean} = $props();
  const host = $derived(room.member_id === room.host_member_id);
  async function grant(member: RoomMember) {
    if (await confirmer.ask(`${member.name} will be able to run commands with this session’s permissions. You can take back control at any time.`, {title: `Give ${member.name} control?`, confirmLabel: 'Give control'})) send({type: 'grant_control', member_id: member.id});
  }
  async function remove(member: RoomMember) {
    if (await confirmer.ask(`${member.name} will lose access to the terminal, chat and media immediately.`, {title: 'Remove participant?', confirmLabel: 'Remove', danger: true})) send({type: 'remove', member_id: member.id});
  }
</script>
<section aria-label="Participants">
  <h2>People <span>{room.members?.filter(m => m.admission === 'admitted').length ?? 1}/4</span></h2>
  <p class="hint">Display names are chosen by participants.</p>
  <ul>{#each room.members ?? [] as member (member.id)}
    <li>
      <div class="identity"><strong>{member.name}{member.id === room.member_id ? ' (you)' : ''}</strong>
        <span>{member.admission === 'pending' ? 'Waiting for admission' : member.role === 'host' ? 'Host' : member.role === 'editor' ? 'Can control' : 'View only'} · {member.connected ? 'Connected' : 'Reconnecting'}</span>
        {#if room.driver_member_id === member.id}<span class="driver">Controls terminal</span>{/if}
        {#if member.audio_joined}<span>{member.room_muted ? 'Muted by host' : member.muted ? 'Microphone muted' : 'Microphone on'}</span>{/if}
      </div>
      {#if host && member.id !== room.member_id}<div class="actions">
        {#if member.admission === 'pending'}
          <button class="btn small" {disabled} onclick={() => send({type: 'admit', member_id: member.id, role: 'viewer'})}>Admit view only</button>
          {#if member.role === 'editor'}<button class="btn small" {disabled} onclick={() => send({type: 'admit', member_id: member.id, role: 'editor'})}>Admit can control</button>{/if}
          <button class="btn small danger" {disabled} onclick={() => send({type: 'reject', member_id: member.id})}>Decline</button>
        {:else}
          <label>Access <select {disabled} value={member.role} onchange={(event) => send({type: 'role', member_id: member.id, role: event.currentTarget.value === 'editor' ? 'editor' : 'viewer'})}><option value="viewer">View only</option><option value="editor">Can control</option></select></label>
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
