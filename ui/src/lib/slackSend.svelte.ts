// "Send on Slack as me" — one dialog any surface opens with a prefilled
// message (PR workspace → tell the author you reviewed; ⌘K → message the
// review bot or anyone; more to come). The dialog (SlackSendDialog, mounted
// once by the app shell) picks the Slack workspace (your default for the
// current Otto workspace), the person or channel, and posts with YOUR user
// token through POST /workspaces/{id}/integrations/slack/messages (as=user).
// Opening it never sends anything: the Send button inside, which shows where
// the message goes and that it posts as you, is the confirmation.

export interface SlackSendRequest {
  /** Dialog title, e.g. "Tell the author on Slack". */
  title?: string;
  /** Prefilled message. */
  text?: string;
  /** Prefilled search for the recipient (a name, `#channel` or `@handle`). */
  to?: string;
  /** One line of context shown above the form (what this is about). */
  context?: string;
}

class SlackSendStore {
  req = $state<SlackSendRequest | null>(null);
  open(req: SlackSendRequest = {}): void {
    this.req = { ...req };
  }
  close(): void {
    this.req = null;
  }
}

export const slackSend = new SlackSendStore();
