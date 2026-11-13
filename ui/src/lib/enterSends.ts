// Whether Enter in a chat composer sends. On a touch-first device (phone,
// tablet without a trackpad) Enter is a new line and the send button sends,
// as in every mobile chat app; ⌘/Ctrl+Enter still sends from a hardware
// keyboard. Elsewhere Enter sends and Shift+Enter is a new line.
export function enterSends(e: KeyboardEvent, touch = isTouchFirst()): boolean {
  if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return false;
  return !touch || e.metaKey || e.ctrlKey;
}

function isTouchFirst(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(hover: none) and (pointer: coarse)').matches;
}
