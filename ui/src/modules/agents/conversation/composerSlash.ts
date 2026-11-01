import {
  formatEffortLabel,
  formatInteractionModeLabel,
  formatModelLabel,
  type EffortLevel,
  type InteractionMode,
} from '../../../lib/providerModels';
import type { ParsedSlashIntent } from '../../../lib/slashActions';
import { auth } from '../../../lib/stores/auth.svelte';
import { transcript } from '../../../lib/stores/transcript.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { interruptAgent, steerAgent, uploadInboxImage } from './api';
import type { ComposerAttachment } from './ComposerAttachments.svelte';

export async function interruptComposerTurn(sessionId: string, agentName: string): Promise<void> {
  try {
    await interruptAgent(sessionId);
    toasts.info('Interrupt sent', `${agentName} stops after its current step`);
  } catch (e) {
    toastError('Couldn’t interrupt the agent', e);
  }
}

export async function steerComposerTurn(opts: {
  sessionId: string;
  agentName: string;
  typed: string;
  attachments: ComposerAttachment[];
  onDoneDraft: () => void;
}): Promise<void> {
  try {
    const submittedImages = [...opts.attachments];
    const imgs = submittedImages.map((a) => `[Image: ${a.path}]`);
    const body = [opts.typed, ...imgs].filter(Boolean).join('\n');
    await steerAgent(opts.sessionId, body);
    if (transcript.draft(opts.sessionId).replace(/\s+$/, '') === opts.typed) {
      transcript.setDraft(opts.sessionId, '');
    }
    for (const a of submittedImages) URL.revokeObjectURL(a.url);
    transcript.setAttachments(
      opts.sessionId,
      transcript.attachments(opts.sessionId).filter((a) => !submittedImages.includes(a)),
    );
    opts.onDoneDraft();
    toasts.info('Steered', `Interrupted and redirected ${opts.agentName}`);
  } catch (e) {
    toastError('Couldn’t steer the agent', e);
  }
}

export async function patchComposerModel(sessionId: string, provider: string, nextModel: string | null): Promise<void> {
  try {
    const label = formatModelLabel(provider, nextModel, auth.meta);
    await ws.updateSessionConfig(
      sessionId,
      { model: nextModel },
      { toastTitle: nextModel ? `Model set to ${label}` : 'Using default model' },
    );
  } catch (e) {
    toastError('Couldn’t switch model', e);
  }
}

export async function patchComposerEffort(sessionId: string, nextEffort: EffortLevel | null): Promise<void> {
  try {
    const label = formatEffortLabel(nextEffort);
    await ws.updateSessionConfig(
      sessionId,
      { effort: nextEffort },
      { toastTitle: nextEffort ? `Effort set to ${label}` : 'Using default effort' },
    );
  } catch (e) {
    toastError('Couldn’t update effort', e);
  }
}

export async function patchComposerMode(sessionId: string, nextMode: InteractionMode): Promise<void> {
  try {
    const label = formatInteractionModeLabel(nextMode);
    await ws.updateSessionConfig(
      sessionId,
      { interaction_mode: nextMode },
      { toastTitle: `${label} active` },
    );
  } catch (e) {
    toastError('Couldn’t switch execution mode', e);
  }
}

export async function uploadComposerImages(
  sessionId: string,
  files: File[],
  addPending: (owner: string, n: number) => void,
): Promise<void> {
  const imgs = files.filter((f) => f.type.startsWith('image/'));
  if (!imgs.length) return;
  addPending(sessionId, imgs.length);
  for (const f of imgs) {
    try {
      const ext = (f.type.split('/')[1] ?? 'png').replace('jpeg', 'jpg');
      const name = f.name && f.name !== 'image.png' ? f.name : `paste-${Date.now()}.${ext}`;
      const path = await uploadInboxImage(sessionId, f, name);
      transcript.setAttachments(sessionId, [
        ...transcript.attachments(sessionId),
        { path, name, url: URL.createObjectURL(f) },
      ]);
    } catch (e) {
      toastError('Couldn’t upload the image', e);
    } finally {
      addPending(sessionId, -1);
    }
  }
}

export interface ComposerSlashHost {
  openModelPicker(): void;
  openEffortPicker(): void;
  openModePicker(): void;
  applyModelChange(model: string | null): Promise<void>;
  applyEffortChange(effort: EffortLevel | null): Promise<void>;
  applyModeChange(mode: InteractionMode): Promise<void>;
  toggleStatusCard(): void;
  sendPrompt(prompt: string): Promise<void>;
  restoreFocus(): void;
  ondiff?: () => void;
  onfiles?: () => void;
  ontoggleterminal?: () => void;
  onnewthread?: (beside: boolean) => void;
}

export async function runComposerSlashIntent(
  intent: ParsedSlashIntent,
  host: ComposerSlashHost,
): Promise<void> {
  switch (intent.kind) {
    case 'open-model-picker':
      queueMicrotask(() => host.openModelPicker());
      return;
    case 'set-model':
      await host.applyModelChange(intent.model);
      return;
    case 'open-effort-picker':
      queueMicrotask(() => host.openEffortPicker());
      return;
    case 'set-effort':
      await host.applyEffortChange(intent.effort);
      return;
    case 'open-mode-picker':
      queueMicrotask(() => host.openModePicker());
      return;
    case 'set-mode':
      await host.applyModeChange(intent.mode);
      if (intent.prompt) {
        await host.sendPrompt(intent.prompt);
      }
      return;
    case 'toggle-status':
      host.toggleStatusCard();
      host.restoreFocus();
      return;
    case 'open-diff':
      if (host.ondiff) host.ondiff();
      else toasts.info('No recorded file edits in this conversation yet');
      host.restoreFocus();
      return;
    case 'open-files':
      if (host.onfiles) host.onfiles();
      else toasts.info('Open Threads inspector to browse workspace files');
      host.restoreFocus();
      return;
    case 'toggle-terminal':
      if (host.ontoggleterminal) host.ontoggleterminal();
      host.restoreFocus();
      return;
    case 'new-thread':
      if (host.onnewthread) host.onnewthread(intent.beside);
      return;
    case 'send-prompt':
      await host.sendPrompt(intent.prompt);
      return;
  }
}
