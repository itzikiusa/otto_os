import { mount, unmount } from 'svelte';
import StartRoomModal from '../../src/modules/rooms/StartRoomModal.svelte';

// Exercise the real creation sheet without starting an agent process.
export function showStartRoom() {
  const target = document.createElement('div');
  document.body.append(target);
  const component = mount(StartRoomModal, { target, props: {
    sessionId: 'fixture-session', onclose: () => { void unmount(component); target.remove(); },
  } });
}
