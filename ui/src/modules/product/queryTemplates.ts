// Product → source search: ready-made JQL / CQL / Linear filters for the
// search box's raw mode, so nobody has to remember the syntax. `{P}` is the selected Jira
// project (or Confluence space) key; a template that needs one says so and is
// filled with a placeholder the user overwrites when nothing is selected.
// Pure — unit-tested (unit/queryTemplates.test.ts).

export interface QueryTemplate {
  label: string;
  query: string;
}

export const JQL_TEMPLATES: QueryTemplate[] = [
  { label: 'My open issues', query: 'assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC' },
  { label: 'Issues in a release', query: 'project = {P} AND fixVersion = "1.0.0" ORDER BY key' },
  { label: 'Children of an epic', query: 'parent = {P}-123 ORDER BY rank' },
  { label: 'Current sprint', query: 'project = {P} AND sprint in openSprints() ORDER BY rank' },
  { label: 'Updated in the last 7 days', query: 'project = {P} AND updated >= -7d ORDER BY updated DESC' },
  { label: 'Open bugs by priority', query: 'project = {P} AND issuetype = Bug AND statusCategory != Done ORDER BY priority DESC' },
  { label: 'Unassigned, not done', query: 'project = {P} AND assignee is EMPTY AND statusCategory != Done' },
  { label: 'With a label', query: 'project = {P} AND labels = "backend"' },
  { label: 'Text anywhere', query: 'project = {P} AND text ~ "login" ORDER BY updated DESC' },
  { label: 'Created by me this month', query: 'reporter = currentUser() AND created >= startOfMonth()' },
];

export const CQL_TEMPLATES: QueryTemplate[] = [
  { label: 'Pages in a space mentioning a word', query: 'space = {P} AND type = page AND text ~ "release"' },
  { label: 'Pages with a label', query: 'space = {P} AND label = "runbook"' },
  { label: 'Modified in the last 30 days', query: 'space = {P} AND type = page AND lastmodified > now("-30d") ORDER BY lastmodified DESC' },
  { label: 'Created by me', query: 'type = page AND creator = currentUser() ORDER BY created DESC' },
  { label: 'Children of a page', query: 'parent = 123456' },
  { label: 'Title starts with', query: 'space = {P} AND title ~ "Release*"' },
];

/** One line per thing people forget: what goes left of the operator, the
 *  operators themselves, and the functions. */
export const JQL_CHEATSHEET: [string, string][] = [
  ['Fields', 'project, issuetype, status, statusCategory, assignee, reporter, fixVersion, sprint, parent, labels, priority, created, updated, text, summary'],
  ['Operators', '=  !=  ~ (contains)  !~  in (a, b)  not in  is EMPTY  is not EMPTY  >  >=  <  <='],
  ['Functions', 'currentUser()  openSprints()  startOfDay()  startOfWeek()  startOfMonth()  -7d / -2w (relative dates)'],
  ['Combine', 'AND  OR  NOT  ( … )  then ORDER BY field ASC|DESC'],
];

export const CQL_CHEATSHEET: [string, string][] = [
  ['Fields', 'space, type (page, blogpost), title, text, label, creator, contributor, ancestor, parent, created, lastmodified'],
  ['Operators', '=  !=  ~ (contains)  !~  in (a, b)  not in  >  >=  <  <='],
  ['Functions', 'currentUser()  now("-30d")  startOfWeek()  endOfMonth()'],
  ['Combine', 'AND  OR  NOT  ( … )  then ORDER BY field ASC|DESC'],
];

/** Linear's raw query is an IssueFilter JSON object; `{P}` is the team key. */
const OPEN = '"state":{"type":{"nin":["completed","canceled"]}}';
export const LINEAR_TEMPLATES: QueryTemplate[] = [
  { label: 'My open issues', query: `{"assignee":{"isMe":{"eq":true}},${OPEN}}` },
  { label: 'In progress in a team', query: '{"team":{"key":{"eq":"{P}"}},"state":{"type":{"eq":"started"}}}' },
  { label: 'Current cycle', query: '{"team":{"key":{"eq":"{P}"}},"cycle":{"isActive":{"eq":true}}}' },
  { label: 'In a project', query: '{"project":{"name":{"eq":"Q4 launch"}}}' },
  { label: 'Sub-issues of an issue', query: '{"team":{"key":{"eq":"{P}"}},"parent":{"number":{"eq":123}}}' },
  { label: 'Updated in the last 7 days', query: '{"team":{"key":{"eq":"{P}"}},"updatedAt":{"gt":"-P7D"}}' },
  { label: 'Urgent and high priority, open', query: `{"team":{"key":{"eq":"{P}"}},"priority":{"gte":1,"lte":2},${OPEN}}` },
  { label: 'Unassigned, not done', query: `{"team":{"key":{"eq":"{P}"}},"assignee":{"null":true},${OPEN}}` },
  { label: 'With a label', query: '{"team":{"key":{"eq":"{P}"}},"labels":{"name":{"eq":"Bug"}}}' },
  { label: 'Title contains', query: '{"team":{"key":{"eq":"{P}"}},"title":{"containsIgnoreCase":"login"}}' },
  { label: 'Created by me this month', query: '{"creator":{"isMe":{"eq":true}},"createdAt":{"gt":"-P1M"}}' },
];

export const LINEAR_CHEATSHEET: [string, string][] = [
  ['Fields', 'team, state, assignee, creator, priority (1 urgent … 4 low), labels, project, cycle, parent, title, description, createdAt, updatedAt, dueDate, estimate'],
  ['Comparators', 'eq  neq  in  nin  lt  lte  gt  gte  null  contains  containsIgnoreCase  startsWith'],
  ['Values', 'state type: triage backlog unstarted started completed canceled · me: {"isMe":{"eq":true}} · relative dates: "-P7D" "-P2W"'],
  ['Combine', '{"and":[ … ]}  {"or":[ … ]} — sibling fields are ANDed'],
];

export const JQL_DOCS = 'https://support.atlassian.com/jira-software-cloud/docs/use-advanced-search-with-jira-query-language-jql/';
export const CQL_DOCS = 'https://developer.atlassian.com/cloud/confluence/advanced-searching-using-cql/';
export const LINEAR_DOCS = 'https://linear.app/developers/filtering';

/** The template with `{P}` → the selected key (quoted when it needs it), or
 *  the `KEY` placeholder when nothing is selected. */
export function fillTemplate(query: string, key: string): string {
  const k = key.trim();
  const v = k === '' ? 'KEY' : /^[A-Za-z0-9_]+$/.test(k) ? k : `"${k.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
  return query.replaceAll('{P}', v);
}

/** A Linear filter template with `{P}` → the team key (JSON-escaped, since
 *  it sits inside a string literal), or `KEY` when no team is selected. */
export function fillLinearTemplate(query: string, key: string): string {
  const k = key.trim();
  return query.replaceAll('{P}', k === '' ? 'KEY' : JSON.stringify(k).slice(1, -1));
}
