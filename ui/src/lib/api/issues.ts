// Issue-tracker API client — thin typed wrappers over the generic `api`
// helper for the `/issue/*` routes (Jira, Linear and Confluence share one
// account model). Mirrors docs/contracts/api.md (the "Issues" section).
//
// One place builds these URLs so a route change, or a second tracker, is one
// edit — and EVERY path segment and query value is `encodeURIComponent`-ed:
// account ids are uuids, but issue keys, project keys and attachment ids come
// from the tracker and must never be able to add a path segment or a param.

import { api, authedBlobUrl } from './client';
import { unwrapListing } from '../listingPage';
import type {
  IssueAccount,
  IssueDetail,
  IssueProject,
  IssueProviderKind,
  IssueSummary,
  ListingPage,
  MyWorkIssue,
} from './types';
import type {
  ConfluencePageSummary,
  ConfluenceSpace,
  DevStatus,
  EditableField,
  IssueFull,
  JiraTransition,
  JiraUser,
} from './types/product';

const enc = encodeURIComponent;

/** `/issue/<account>/<key><rest>` with both segments encoded. */
function issuePath(accountId: string, key: string, rest = ''): string {
  return `/issue/${enc(accountId)}/${enc(key)}${rest}`;
}

/** A query string from the defined, non-empty entries (values encoded). */
function query(params: Record<string, string | number | boolean | null | undefined>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '' || v === false) continue;
    parts.push(`${enc(k)}=${enc(v === true ? '1' : String(v))}`);
  }
  return parts.length ? `?${parts.join('&')}` : '';
}

/** Body of `POST /issue/accounts` (`PATCH` takes any subset; an omitted
 *  `token` keeps the stored Keychain secret). */
export interface IssueAccountInput {
  provider: IssueProviderKind;
  label: string;
  base_url: string;
  email: string;
  token: string;
  token_expires_at: string | null | undefined;
}

// ── Accounts ────────────────────────────────────────────────────────────────

export function listIssueAccounts(signal?: AbortSignal): Promise<IssueAccount[]> {
  return api.get<IssueAccount[]>('/issue/accounts', signal);
}
export function createIssueAccount(body: IssueAccountInput): Promise<IssueAccount> {
  return api.post<IssueAccount>('/issue/accounts', body);
}
export function updateIssueAccount(
  id: string,
  body: Record<string, string | null | undefined>,
): Promise<IssueAccount> {
  return api.patch<IssueAccount>(`/issue/accounts/${enc(id)}`, body);
}
export function deleteIssueAccount(id: string): Promise<unknown> {
  return api.del(`/issue/accounts/${enc(id)}`);
}

// ── Listings (projects / Confluence spaces) ─────────────────────────────────

/** Projects (Jira) or teams (Linear) the account's token can see. `meta`
 *  asks for the `{ items, truncated }` page; the bare array a pre-`meta`
 *  daemon answers is normalised either way. */
export async function listIssueProjects(
  accountId: string,
  opts: { meta?: boolean } = {},
  signal?: AbortSignal,
): Promise<ListingPage<IssueProject>> {
  const body = await api.get<IssueProject[] | ListingPage<IssueProject>>(
    `/issue/projects${query({ account_id: accountId, meta: opts.meta ?? true })}`,
    signal,
  );
  return unwrapListing(body);
}

export async function listConfluenceSpaces(accountId: string, signal?: AbortSignal): Promise<ListingPage<ConfluenceSpace>> {
  const body = await api.get<ConfluenceSpace[] | ListingPage<ConfluenceSpace>>(
    `/issue/confluence/spaces${query({ account_id: accountId, meta: true })}`,
    signal,
  );
  return unwrapListing(body);
}

/** Issue types a project accepts (Publish → Jira). */
export function listIssueTypes(accountId: string, projectKey: string): Promise<string[]> {
  return api.get<string[]>(issuePath(accountId, projectKey, '/issue-types'));
}

// ── Search ──────────────────────────────────────────────────────────────────

/** `q` is a text search (optionally within `project`); `jql` (Jira) and
 *  `filter` (Linear) are raw expressions. */
export function searchIssues(
  p: { accountId: string; q?: string; project?: string; jql?: string; filter?: string; startAt?: number },
  signal?: AbortSignal,
): Promise<IssueSummary[]> {
  return api.get<IssueSummary[]>(
    `/issue/search${query({ account_id: p.accountId, jql: p.jql, filter: p.filter, q: p.q, project: p.project, start_at: p.startAt })}`,
    signal,
  );
}

/** `mode: 'text'` searches page bodies too; `cql` is a raw expression. */
export function searchConfluence(
  p: { accountId: string; q?: string; space?: string; cql?: string; mode?: 'text' },
  signal?: AbortSignal,
): Promise<ConfluencePageSummary[]> {
  return api.get<ConfluencePageSummary[]>(
    `/issue/confluence/search${query({ account_id: p.accountId, cql: p.cql, q: p.q, space: p.space, mode: p.mode })}`,
    signal,
  );
}

export function myWork(accountId: string, signal?: AbortSignal): Promise<MyWorkIssue[]> {
  return api.get<MyWorkIssue[]>(`/issue/my-work${query({ account_id: accountId })}`, signal);
}

// ── One issue ───────────────────────────────────────────────────────────────

export function getIssue(accountId: string, key: string, signal?: AbortSignal): Promise<IssueDetail> {
  return api.get<IssueDetail>(issuePath(accountId, key), signal);
}
export function issueFull(accountId: string, key: string, signal?: AbortSignal): Promise<IssueFull> {
  return api.get<IssueFull>(issuePath(accountId, key, '/full'), signal);
}
export function devStatus(accountId: string, key: string, issueId?: string | null): Promise<DevStatus> {
  return api.get<DevStatus>(issuePath(accountId, key, `/devstatus${query({ issueId })}`));
}
export function transitions(accountId: string, key: string): Promise<JiraTransition[]> {
  return api.get<JiraTransition[]>(issuePath(accountId, key, '/transitions'));
}
export function applyTransition(accountId: string, key: string, transitionId: string): Promise<unknown> {
  return api.post(issuePath(accountId, key, '/transitions'), { transition_id: transitionId });
}
export function assignableUsers(accountId: string, key: string): Promise<JiraUser[]> {
  return api.get<JiraUser[]>(issuePath(accountId, key, '/assignable'));
}
/** `userAccountId: ''` unassigns. */
export function setAssignee(accountId: string, key: string, userAccountId: string): Promise<unknown> {
  return api.put(issuePath(accountId, key, '/assignee'), { account_id: userAccountId });
}
export function editMeta(accountId: string, key: string): Promise<EditableField[]> {
  return api.get<EditableField[]>(issuePath(accountId, key, '/editmeta'));
}
/** PUT raw Jira-shaped field values; answers the refreshed issue. */
export function updateFields(accountId: string, key: string, fields: Record<string, unknown>): Promise<IssueFull> {
  return api.put<IssueFull>(issuePath(accountId, key, '/fields'), { fields });
}
/** Replace the description (markdown → ADF server-side). */
export function updateDescription(accountId: string, key: string, bodyMd: string): Promise<IssueFull> {
  return api.put<IssueFull>(issuePath(accountId, key, '/description'), { body_md: bodyMd });
}
export function addComment(accountId: string, key: string, body: string): Promise<unknown> {
  return api.post(issuePath(accountId, key, '/comment'), { body });
}
/** An authed object URL for an attachment — the caller revokes it. */
export function attachmentBlobUrl(accountId: string, key: string, attachmentId: string): Promise<string> {
  return authedBlobUrl(issuePath(accountId, key, `/attachment/${enc(attachmentId)}`));
}
