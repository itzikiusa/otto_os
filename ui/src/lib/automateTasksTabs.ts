export type AutomateTasksTab = 'tasks' | 'agents' | 'rooms';

export const AUTOMATE_TASKS_TABS: readonly { id: AutomateTasksTab; label: string; route: string }[] = [
  { id: 'agents', label: 'Agents', route: 'personal-agents' },
  { id: 'tasks', label: 'Scheduled tasks', route: 'scheduled-tasks' },
  { id: 'rooms', label: 'Agent channels', route: 'personal-agents/rooms' },
];
