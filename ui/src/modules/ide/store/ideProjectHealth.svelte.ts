// Whether the open project's root can be listed at all (review I3-12). A deleted
// folder or an unreachable SSH host is not a "couldn't refresh" banner with a Retry
// that can never succeed: the tree shows what is wrong and next steps that can work.

import { ApiError } from '../../../lib/api/client';
import type { IdeTargetSpec } from './ideTypes';

export type IdeRootIssueKind = 'missing' | 'unreachable';

export interface IdeRootIssue {
  kind: IdeRootIssueKind;
  /** `ideTargetKey` of the target it was seen on. */
  key: string;
  root: string;
  message: string;
}

/** Pure: classify a failed root listing. `null` = an ordinary error (keep the Retry banner). */
export function classifyRootError(target: Pick<IdeTargetSpec, 'kind'>, status: number | null, message: string): IdeRootIssueKind | null {
  if (target.kind === 'ssh') return status === 401 || status === 403 ? null : 'unreachable';
  if (status === 404) return 'missing';
  return /no such file or directory|cannot resolve workspace root/i.test(message) ? 'missing' : null;
}

class IdeProjectHealth {
  issue = $state<IdeRootIssue | null>(null);

  noteRootError(target: Pick<IdeTargetSpec, 'kind' | 'root' | 'connection_id'>, e: unknown, message: string): void {
    const status = e instanceof ApiError ? e.status : null;
    const kind = classifyRootError(target, status, message);
    this.issue = kind ? { kind, key: `${target.kind}:${target.connection_id ?? ''}:${target.root}`, root: target.root, message } : null;
  }

  clear(): void {
    if (this.issue) this.issue = null;
  }
}

export const ideProjectHealth = new IdeProjectHealth();
