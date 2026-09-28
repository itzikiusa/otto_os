// Draft-form seeding for the product OverviewTab (r3-02-07). The story
// detail object is replaced by many writers (updateStory, patchStory,
// loadDetail, ActionCard); the form must seed ONCE per story and afterwards
// only refresh the fields the user has not edited, never clobbering an
// unsaved draft.

/** What was last seeded into the form (and for which story). */
export interface DraftSeed {
  id: string | null;
  title: string;
  body: string;
}

/** The form's next `{title, body}` for an incoming story version. `form` is
 *  the current form content, `prev` the last seed, `next` the new source. */
export function reseedDraft(
  prev: DraftSeed,
  form: { title: string; body: string },
  next: DraftSeed,
): { title: string; body: string } {
  const fresh = next.id !== prev.id;
  return {
    title: fresh || form.title === prev.title ? next.title : form.title,
    body: fresh || form.body === prev.body ? next.body : form.body,
  };
}
