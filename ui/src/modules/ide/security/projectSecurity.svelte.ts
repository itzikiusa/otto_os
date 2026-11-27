import type { SecurityFinding } from '../../../lib/api/types';
import type { IdeDiagnostic } from '../store/ideTypes';

export const projectSecurity = $state<{ problems: IdeDiagnostic[] }>({ problems: [] });

export function findingPath(finding: SecurityFinding): string {
  return `${finding.root}/${finding.path}`;
}
