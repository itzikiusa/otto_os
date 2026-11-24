// One vocabulary for every approval (RuntimeApprovalRow): runtime, subagent,
// MCP, ACP. The provider's own label stays as the button's tooltip.
import type { ApprovalDecision, ApprovalOption } from '../../../lib/api/types';

const WORDS: Partial<Record<ApprovalDecision, string>> = {
  accept: 'Allow once',
  accept_always: 'Always allow',
  accept_for_session: 'Always allow',
  decline: 'Deny',
  decline_always: 'Always deny',
  cancel: 'Deny and stop',
};

export const approvalWord = (o: ApprovalOption): string => WORDS[o.decision] ?? o.label;

/** Allow once · Always allow · Deny · Always deny (the "always" ones only
 *  when the provider offers them) as buttons; anything else under More. */
export function approvalButtons(options: readonly ApprovalOption[]): { primary: ApprovalOption[]; more: ApprovalOption[] } {
  const find = (d: ApprovalDecision): ApprovalOption | undefined => options.find((o) => o.decision === d);
  const primary = [find('accept'), find('accept_always') ?? find('accept_for_session'), find('decline'), find('decline_always')].filter(
    (o): o is ApprovalOption => !!o,
  );
  return { primary, more: options.filter((o) => !primary.includes(o)) };
}
