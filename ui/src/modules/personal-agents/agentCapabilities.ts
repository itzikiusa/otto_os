// Pure at-a-glance capability chips for a Personal Agent (E-15):
// Read-only / Directed / Autonomous · Slack-as-me · Browser · N rules.
import type { PersonalAgent, PersonalAgentAutonomy, PersonalAgentSchedule } from '../../lib/api/types';

export interface CapabilityChip {
  id: string;
  label: string;
  tone: 'neutral' | 'accent' | 'warning';
  title: string;
  tab: string;
}

export function agentCapabilityChips(
  agent: Pick<PersonalAgent, 'browser' | 'enabled'>,
  autonomy: PersonalAgentAutonomy | undefined,
  schedules: readonly PersonalAgentSchedule[],
): CapabilityChip[] {
  const chips: CapabilityChip[] = [];
  const enabledSchedules = schedules.filter((s) => s.enabled);
  const allReadOnly =
    enabledSchedules.length > 0 && enabledSchedules.every((s) => s.permission === 'read_only');

  if (autonomy?.proactive.enabled) {
    chips.push({
      id: 'mode',
      label: `Autonomous · ${autonomy.proactive.runs_per_day}/day`,
      tone: 'accent',
      title: `Can start up to ${autonomy.proactive.runs_per_day} proactive runs per day on its standing goals`,
      tab: 'autonomy',
    });
  } else if (allReadOnly) {
    chips.push({
      id: 'mode',
      label: 'Read-only',
      tone: 'neutral',
      title: 'All active routines are confined to read-only mode',
      tab: 'routines',
    });
  } else {
    chips.push({
      id: 'mode',
      label: 'Directed',
      tone: 'neutral',
      title: 'Runs on your routines and chat messages; asks before sensitive changes',
      tab: 'autonomy',
    });
  }

  const slackTeams = autonomy?.slack_as_me_teams?.length ?? 0;
  if (slackTeams > 0) {
    chips.push({
      id: 'slack-as-me',
      label: slackTeams === 1 ? 'Slack-as-me' : `Slack-as-me (${slackTeams})`,
      tone: 'warning',
      title: `Can read and act as you in ${slackTeams} Slack ${slackTeams === 1 ? 'workspace' : 'workspaces'}`,
      tab: 'autonomy',
    });
  }

  if (agent.browser) {
    chips.push({
      id: 'browser',
      label: 'Browser',
      tone: 'neutral',
      title: 'Browser automation is enabled for this agent',
      tab: 'about',
    });
  }

  const ruleCount = autonomy?.rules.length ?? 0;
  if (ruleCount > 0) {
    chips.push({
      id: 'rules',
      label: `${ruleCount} ${ruleCount === 1 ? 'rule' : 'rules'}`,
      tone: 'neutral',
      title: `${ruleCount} standing ${ruleCount === 1 ? 'rule governs' : 'rules govern'} this agent’s actions`,
      tab: 'autonomy',
    });
  }

  return chips;
}
