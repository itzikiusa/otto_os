///<reference types="svelte" />
;;
  // A single proof badge chip: maps a snake_case badge string (see `ProofBadge`
  // in the API types) to a short label + a semantic color tone.
  interface Props {
    badge: string;
  };;

  type Tone = 'ok' | 'bad' | 'accent' | 'warn' | 'dim' | 'neutral';;function $$render() {


  let { badge }: Props = $props();


  const MAP: Record<string, { label: string; tone: Tone }> = {
    tests_passed: { label: 'Tests ✓', tone: 'ok' },
    tests_failed: { label: 'Tests ✗', tone: 'bad' },
    human_approved: { label: 'Approved', tone: 'ok' },
    db_api_verified: { label: 'DB/API ✓', tone: 'accent' },
    risky_change: { label: 'Risky', tone: 'warn' },
    ci_missing: { label: 'No CI', tone: 'warn' },
    ci_passed: { label: 'CI ✓', tone: 'ok' },
    ci_failed: { label: 'CI ✗', tone: 'bad' },
    ci_pending: { label: 'CI…', tone: 'warn' },
    ui_verified: { label: 'UI ✓', tone: 'accent' },
    pr_inconsistent: { label: 'PR ✗', tone: 'bad' },
    review_unresolved: { label: 'Review ✗', tone: 'bad' },
    no_proof: { label: 'No proof', tone: 'dim' },
    waived: { label: 'Waived', tone: 'neutral' },
  };

  const info = $derived(MAP[badge] ?? { label: badge, tone: 'dim' as Tone });
;
async () => {

 { svelteHTML.createElement("span", {   "class":`proof-badge ${info.tone}`,"title":badge,});info.label; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProofBadge__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProofBadge__SvelteComponent_ = ReturnType<typeof ProofBadge__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProofBadge__SvelteComponent_;