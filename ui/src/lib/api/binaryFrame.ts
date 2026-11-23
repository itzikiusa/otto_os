// Shared binary screencast frame decoder used by both the remote live browser
// (`WS /ws/browser/{tab_id}/live`) and mobile simulators (`WS /ws/simulators/{id}/stream`).
//
// Wire layout:
//   byte 0      format version (1)
//   bytes 1..5  u32 big-endian N = UTF-8 JSON header byte length
//   bytes 5..   N bytes of JSON header (`T`)
//   then        raw image bytes (PNG / JPEG)
//
// Pure, zero runtime imports — unit-testable directly in plain Node.

export const BINARY_FRAME_VERSION = 1;

const textDecoder = new TextDecoder();

export interface DecodedBinaryFrame<T> {
  header: T;
  bytes: Uint8Array;
}

export function decodeBinaryFrame<T>(
  buf: ArrayBuffer,
  validate?: (header: T) => boolean,
): DecodedBinaryFrame<T> | null {
  if (buf.byteLength < 5) return null;
  const view = new DataView(buf);
  if (view.getUint8(0) !== BINARY_FRAME_VERSION) return null;
  const len = view.getUint32(1, false);
  const payloadStart = 5 + len;
  if (len <= 0 || payloadStart > buf.byteLength) return null;
  let header: T;
  try {
    header = JSON.parse(textDecoder.decode(new Uint8Array(buf, 5, len))) as T;
  } catch {
    return null;
  }
  if (validate && !validate(header)) return null;
  return { header, bytes: new Uint8Array(buf, payloadStart) };
}
