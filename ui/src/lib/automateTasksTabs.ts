export type AutomateTasksTab = 'tasks' | 'agents' | 'rooms';

export const AUTOMATE_TASKS_TABS: readonly { id: AutomateTasksTab; label: string; route: string }[] = [
  { id: 'tasks', label: 'Tasks', route: 'scheduled-tasks' },
  { id: 'agents', label: 'Agents', route: 'personal-agents' },
  { id: 'rooms', label: 'Agent channels', route: 'personal-agents/rooms' },
];
