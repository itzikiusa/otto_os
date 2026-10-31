///<reference types="svelte" />
;;
  // One avatar tile for a personal agent, used by the cards, the agent page
  // header and room messages. The avatar is user data (the sheet's optional
  // "Avatar" field — a short glyph or initials); with none set it falls back
  // to the name's monogram. The
  // tile is neutral on purpose: accent means *selected*, never agent identity.
  interface Props {
    /** The agent's saved avatar ('' = none). */
    avatar?: string | null;
    /** Name for the monogram fallback. */
    name: string;
    /** Tile edge in px. */
    size?: number;
    round?: boolean;
  };function $$render() {


  let { avatar = '', name, size = 32, round = false }: Props = $props();

  const glyph = $derived.by(() => {
    const a = (avatar ?? '').trim();
    if (a) return a;
    const m = name.trim().match(/[\p{L}\p{N}]/u);
    return m ? m[0].toUpperCase() : '?';
  });
  const mono = $derived(!(avatar ?? '').trim());
;
async () => {

 { svelteHTML.createElement("span", {         "class":`pa-avatar`,"aria-hidden":`true`,});round;mono;__sveltets_2_ensureType(String, Number, `${size} `);__sveltets_2_ensureType(String, Number, `${size} `);__sveltets_2_ensureType(String, Number, `${Math.max(11, Math.round(size * (mono ? 0.46 : 0.55)))} `);glyph; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentAvatar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentAvatar__SvelteComponent_ = ReturnType<typeof AgentAvatar__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentAvatar__SvelteComponent_;