// Who may do what on a swarm, mirrored from the daemon's rules so the UI
// offers only what the server will accept (otto-swarm `http.rs`, PC-2).

/** The caller, as the swarm views need it. */
export interface SwarmCaller {
  /** `auth.isRoot`. */
  isRoot: boolean;
  /** The caller's role in the swarm's workspace (`my_role`). */
  myRole: string | null | undefined;
  /** `auth.me?.id`. */
  me: string | null | undefined;
}

/** PC-2 (access-control-6-01 / 6-03): an agent's identity — prompt
 *  (scope / specialization), soul, provider, model, cwd mode and schedule
 *  directive — is the swarm creator's, a ws admin's or root's to write, on a
 *  hire as on an edit. Name, title, skills, reporting line and cadence stay
 *  Editor-level. */
export function canEditSwarmIdentity(c: SwarmCaller, swarmCreatedBy: string | null | undefined): boolean {
  if (c.isRoot || c.myRole === 'admin') return true;
  return !!c.me && !!swarmCreatedBy && c.me === swarmCreatedBy;
}

/** The read-only note shown beside the identity fields a caller can't edit. */
export const IDENTITY_OWNER_NOTE =
  'Only the swarm’s creator or a workspace admin can change this agent’s prompt, soul, provider, model or directive.';

/** access-control-6-04: a run's session is created AS its acting user, and
 *  session access is owner, ws admin or root. The caller can open it when it
 *  is in their own (owner-scoped) session store, or they are root / admin. */
export function canOpenRunSession(c: Pick<SwarmCaller, 'isRoot' | 'myRole'>, inMyStore: boolean): boolean {
  return inMyStore || c.isRoot || c.myRole === 'admin';
}

/** Title for the disabled "Open session" control (the run carries no owner
 *  name; the person it acted as owns it). */
export const RUN_SESSION_NOT_YOURS =
  'This run’s session belongs to the person it acted as — only they or a workspace admin can open it.';
