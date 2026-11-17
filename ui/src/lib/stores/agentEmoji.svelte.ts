// "Show emoji in agent replies" (Settings › Appearance, per device, on by
// default): agent text renders `:shortcode:` as emoji; off shows it as written.
// Rendering lives in lib/agentEmojiRender.svelte.ts, so Settings stays light.
const KEY = 'otto_agent_emoji';
const read = (): boolean => {
  try {
    return localStorage.getItem(KEY) !== '0';
  } catch {
    return true;
  }
};

export const agentEmoji = new (class {
  on = $state(read());
  set(on: boolean): void {
    this.on = on;
    try {
      localStorage.setItem(KEY, on ? '1' : '0');
    } catch {
      /* private mode: this session only */
    }
  }
})();
