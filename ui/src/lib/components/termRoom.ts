// Terminal helpers for the session-room transport (and the pasted-image
// re-encode shared with the owner's paste path). Split out of Terminal.svelte.
//
// Room terminals (`socketFactory`) always render on the DOM: on a guest's
// room page the WebGL renderer drew glyphs at a fraction of the font size
// (seen in the live two-browser room test), and a room is one low-throughput
// viewer — not what the per-window GPU budget is for.
import { bytesToBase64, textToBase64, textToBytes } from '../b64.ts';

/** The room relay caps one input frame at 32 KiB decoded (otto-rooms
 *  `decode_input`); a bigger paste goes as consecutive chunks. */
export const ROOM_INPUT_CHUNK = 24 * 1024;

/** The `input` frame(s) for typed/pasted `data`: one frame, or — on the room
 *  relay — chunks under its per-frame cap. */
export function inputFrames(data: string, user: boolean, room: boolean): { type: 'input'; data: string; user: boolean }[] {
  const bytes = room ? textToBytes(data) : null;
  if (!bytes || bytes.length <= ROOM_INPUT_CHUNK) return [{ type: 'input', data: textToBase64(data), user }];
  const out: { type: 'input'; data: string; user: boolean }[] = [];
  for (let at = 0; at < bytes.length; at += ROOM_INPUT_CHUNK) {
    out.push({ type: 'input', data: bytesToBase64(bytes.subarray(at, at + ROOM_INPUT_CHUNK)), user });
  }
  return out;
}

/** The dim notice written when the room relay ends this terminal stream for
 *  good (the shared process exited or was replaced, or access ended): every
 *  re-upgrade would be refused, so the terminal stops retrying. */
export function endedLine(reason: string): string {
  return `\r\n\x1b[2m[otto] ${reason}\x1b[0m\r\n`;
}

/** PNG bytes for an arbitrary pasted image. The daemon stores PNG only
 *  (`store_snip`'s `png_dims` rejects anything else) and a clipboard image is
 *  frequently JPEG or TIFF — re-encode through a canvas instead of refusing
 *  the paste. PNG input is passed through untouched (no needless re-encode). */
export async function toPngBytes(file: File): Promise<Uint8Array> {
  if (file.type === 'image/png') return new Uint8Array(await file.arrayBuffer());
  const bitmap = await createImageBitmap(file);
  const canvas = document.createElement('canvas');
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('canvas 2d context unavailable');
  ctx.drawImage(bitmap, 0, 0);
  bitmap.close();
  const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, 'image/png'));
  if (!blob) throw new Error('could not re-encode the image as PNG');
  return new Uint8Array(await blob.arrayBuffer());
}

/** {@link toPngBytes} as an `image/png` Blob. */
export async function toPngBlob(file: File): Promise<Blob> {
  return new Blob([(await toPngBytes(file)) as Uint8Array<ArrayBuffer>], { type: 'image/png' });
}
