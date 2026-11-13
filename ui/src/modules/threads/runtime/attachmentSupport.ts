// What a runtime thread's agent takes besides text, from the capabilities the
// daemon reports (`RuntimeSession.capabilities`, or `MetaResp
// .runtime_capabilities[provider]` before a thread exists) — never from the
// provider's name. No imports: unit tests load it.

export interface TakesCaps {
  images: boolean;
  files: boolean;
  steer?: boolean;
  efforts?: string[];
}

const NONE: TakesCaps = { images: false, files: false, steer: false, efforts: [] };

/** The capabilities to gate on: a session's own, else the provider baseline, else nothing. */
export function capsOf(session: TakesCaps | null | undefined, baseline?: TakesCaps | null): TakesCaps {
  return session ?? baseline ?? NONE;
}

/** Images / PDFs the agent takes. */
export function attachmentSupport(caps: TakesCaps | null | undefined): { images: boolean; files: boolean } {
  return { images: !!caps?.images, files: !!caps?.files };
}

/** Whether `f` is an attachment the agent takes. */
export function takesFile(caps: TakesCaps | null | undefined, f: Pick<File, 'type'>): boolean {
  return (!!caps?.images && f.type.startsWith('image/')) || (!!caps?.files && f.type === 'application/pdf');
}
