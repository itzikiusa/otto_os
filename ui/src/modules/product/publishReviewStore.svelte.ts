// One outward-publish review at a time for the Product tabs (Rewrite,
// Questions). The shared `confirmOutward` caps its "What" at 400 characters
// and has no site; a publish must show WHERE it goes (site + issue/page) and
// the FULL text sent, scrollable (AGENTS.md "Agents are visible and ask
// first"). `owner` picks which tab's <PublishReviewDialog> renders it, so two
// mounted tabs never show the same request twice.

export interface PublishReviewReq {
  /** Which mounted dialog shows it ("rewrite", "questions"). */
  owner: string;
  title: string;
  /** The confirm button: the action itself, never "OK". */
  verb: string;
  /** "Jira PROJ-1 “Reset password”". */
  where: string;
  /** The story's URL; the dialog shows its site ("acme.atlassian.net"). */
  url?: string | null;
  /** The full text sent. */
  what: string;
  who: string;
  /** A warning line above the text (e.g. a stale suggestion), or null. */
  note?: string | null;
  danger?: boolean;
}

class PublishReviewStore {
  current = $state<PublishReviewReq | null>(null);
  private resolver: ((ok: boolean) => void) | null = null;

  /** Show the review; resolves true when the person confirms. A newer ask
   *  declines the one still open. */
  ask(req: PublishReviewReq): Promise<boolean> {
    this.answer(false);
    this.current = req;
    return new Promise<boolean>((resolve) => {
      this.resolver = resolve;
    });
  }

  answer(ok: boolean): void {
    const resolve = this.resolver;
    this.resolver = null;
    this.current = null;
    resolve?.(ok);
  }
}

export const publishReview = new PublishReviewStore();
