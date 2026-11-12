// Mock-mode PR review + comment writes (ui/src/lib/api/mock.ts routes them):
// approve / request changes and withdrawing them, editing and deleting your
// own comment, and reactions. "You" on the forge is MOCK_ME (mock-prs.ts).
/* eslint-disable @typescript-eslint/no-explicit-any */
import type { PrComment, PrReaction, ReactionKey } from './types';
import { MOCK_ME } from './mock-prs';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: any, q: URLSearchParams) => Out };
/** The mutable mock PR the routes act on. */
export interface MockPrRef {
  comments: PrComment[];
  approved_by: string[];
  changes_by?: string[];
  /** Reviewers asked for a review who haven't answered yet (PUT …/reviewers). */
  requested?: string[];
}
type Find = (repoId: string, num: number) => MockPrRef | undefined;

const without = (l: string[] | undefined): string[] => (l ?? []).filter((n) => n !== MOCK_ME);
const notFound = (what: string): Out => ({ status: 404, json: { code: 'not_found', message: what } });

/** The comment `id` (head or reply) and the list that holds it. */
function locate(list: PrComment[], id: string): { list: PrComment[]; i: number } | null {
  for (let i = 0; i < list.length; i++) {
    if (list[i].id === id) return { list, i };
    const r = locate(list[i].replies ?? [], id);
    if (r) return r;
  }
  return null;
}

export function prReviewMockRoutes(find: Find): MockRoute[] {
  const pr = (m: RegExpMatchArray) => find(m[1], Number(m[2]));
  return [
    {
      method: 'PUT',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/reviewers$/,
      handle: (m, body) => {
        const p = pr(m);
        if (!p) return notFound('pr');
        const add: string[] = body.add ?? [];
        const remove: string[] = body.remove ?? [];
        const keep = (l: string[] | undefined): string[] => (l ?? []).filter((n) => !remove.includes(n));
        p.approved_by = keep(p.approved_by);
        p.changes_by = keep(p.changes_by);
        p.requested = [...keep(p.requested), ...add.filter((n) => !(p.requested ?? []).includes(n) && !p.approved_by.includes(n))];
        return { status: 204 };
      },
    },
    {
      method: 'POST',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/(approve|request-changes)$/,
      handle: (m) => {
        const p = pr(m);
        if (p) [p.approved_by, p.changes_by] = m[3] === 'approve' ? [[...without(p.approved_by), MOCK_ME], without(p.changes_by)] : [without(p.approved_by), [...without(p.changes_by), MOCK_ME]];
        return { status: 204 };
      },
    },
    {
      method: 'DELETE',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/(approve|request-changes)$/,
      handle: (m) => {
        const p = pr(m);
        if (!p) return notFound('pr');
        const had = m[3] === 'approve' ? p.approved_by.includes(MOCK_ME) : (p.changes_by ?? []).includes(MOCK_ME);
        if (!had) return { status: 409, json: { code: 'conflict', message: 'There is no review of yours to withdraw.' } };
        if (m[3] === 'approve') p.approved_by = without(p.approved_by);
        else p.changes_by = without(p.changes_by);
        return { status: 204 };
      },
    },
    {
      method: 'PATCH',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/comments\/([^/]+)$/,
      handle: (m, body) => {
        const at = locate(pr(m)?.comments ?? [], decodeURIComponent(m[3]));
        if (!at) return notFound('comment');
        const c = at.list[at.i];
        if (!c.can_edit) return { status: 403, json: { code: 'forbidden', message: 'You can only edit your own comments.' } };
        at.list[at.i] = { ...c, body: String(body?.body ?? '') };
        return { json: at.list[at.i] };
      },
    },
    {
      method: 'DELETE',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/comments\/([^/]+)$/,
      handle: (m) => {
        const at = locate(pr(m)?.comments ?? [], decodeURIComponent(m[3]));
        if (!at) return notFound('comment');
        if (!at.list[at.i].can_edit) return { status: 403, json: { code: 'forbidden', message: 'You can only delete your own comments.' } };
        at.list.splice(at.i, 1);
        return { status: 204 };
      },
    },
    {
      method: 'PUT',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/comments\/([^/]+)\/reactions\/([a-z_]+)$/,
      handle: (m) => {
        const at = locate(pr(m)?.comments ?? [], decodeURIComponent(m[3]));
        if (!at) return notFound('comment');
        const key = m[4] as ReactionKey;
        const list: PrReaction[] = [...(at.list[at.i].reactions ?? [])];
        const r = list.find((x) => x.key === key);
        if (!r) list.push({ key, count: 1, me: true });
        else if (!r.me) Object.assign(r, { count: r.count + 1, me: true });
        at.list[at.i] = { ...at.list[at.i], reactions: list };
        return { json: list };
      },
    },
    {
      method: 'DELETE',
      re: /^\/repos\/([^/]+)\/prs\/(\d+)\/comments\/([^/]+)\/reactions\/([a-z_]+)$/,
      handle: (m) => {
        const at = locate(pr(m)?.comments ?? [], decodeURIComponent(m[3]));
        if (!at) return notFound('comment');
        const list = (at.list[at.i].reactions ?? []).flatMap((x) =>
          x.key !== m[4] || !x.me ? [x] : x.count > 1 ? [{ ...x, count: x.count - 1, me: false }] : [],
        );
        at.list[at.i] = { ...at.list[at.i], reactions: list };
        return { status: 204 };
      },
    },
  ];
}
