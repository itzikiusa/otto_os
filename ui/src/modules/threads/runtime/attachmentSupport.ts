// What a runtime thread's agent takes besides text (no imports: unit tests load it).

/** What a runtime thread's agent takes besides text. Claude: images + PDFs
 *  (content/document blocks); Codex and pi: images; the ACP agents
 *  (Antigravity, Gemini, Grok, OpenCode): images + PDFs, and the driver
 *  refuses with a clear message if that agent did not advertise them. */
export function attachmentSupport(provider: string): { images: boolean; files: boolean } {
  const files = provider === 'claude' || provider === 'agy' || provider === 'gemini' || provider === 'grok' || provider === 'opencode';
  return { images: files || provider === 'codex' || provider === 'pi', files };
}

/** Whether `f` is an attachment `provider` takes. */
export function takesFile(provider: string, f: Pick<File, 'type'>): boolean {
  const can = attachmentSupport(provider);
  return (can.images && f.type.startsWith('image/')) || (can.files && f.type === 'application/pdf');
}
