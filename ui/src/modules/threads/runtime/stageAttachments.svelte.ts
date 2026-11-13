// Images / PDFs on the new-thread stage: they ride a structured runtime
// thread's first message. A harness switch hides what the new one can't take.
import { attachmentSupport, takesFile } from './attachmentSupport';
import { readAttachments, type ComposerFile } from './runtimeComposer';

export class StageAttachments {
  picked = $state<ComposerFile[]>([]);
  constructor(
    private readonly provider: () => string,
    private readonly enabled: () => boolean,
  ) {}

  /** What the current harness takes: images, and PDFs too. */
  get can(): { images: boolean; files: boolean } {
    return this.enabled() ? attachmentSupport(this.provider()) : { images: false, files: false };
  }
  /** The attachments the first message sends. */
  get list(): ComposerFile[] {
    return this.can.images ? this.picked.filter((a) => takesFile(this.provider(), { type: a.mime })) : [];
  }
  async add(files: File[]): Promise<void> {
    const read = await readAttachments(files, (f) => takesFile(this.provider(), f));
    this.picked = [...this.picked, ...read];
  }
  paste(e: ClipboardEvent): void {
    const files = this.can.images ? [...(e.clipboardData?.files ?? [])].filter((f) => takesFile(this.provider(), f)) : [];
    if (files.length === 0) return;
    e.preventDefault();
    void this.add(files);
  }
  remove(a: ComposerFile): void {
    this.picked = this.picked.filter((x) => x !== a);
  }
}
