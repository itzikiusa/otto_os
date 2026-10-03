// encode.worker.ts — PNG-encode a flattened snip OFF the main thread.
//
// The editor renders image + annotations on the main thread (cheap draw
// calls), snapshots the result as an ImageBitmap (TRANSFERRED in, zero copy)
// and this worker draws it onto an OffscreenCanvas and runs the expensive
// part — `convertToBlob('image/png')`, ~100s of ms for a 5K screenshot — so
// typing pauses never jank the editor. Plain TS, no DOM APIs.

export type EncodeIn = { id: number; bitmap: ImageBitmap };
export type EncodeOut = { id: number; blob?: Blob; error?: string };

const post = self.postMessage as (msg: EncodeOut) => void;

self.onmessage = async (e: MessageEvent<EncodeIn>) => {
  const { id, bitmap } = e.data;
  try {
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('OffscreenCanvas 2d context unavailable');
    ctx.drawImage(bitmap, 0, 0);
    bitmap.close();
    post({ id, blob: await canvas.convertToBlob({ type: 'image/png' }) });
  } catch (err) {
    post({ id, error: err instanceof Error ? err.message : String(err) });
  }
};
