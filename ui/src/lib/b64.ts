// Binary-safe base64 helpers (terminal WS frames carry base64 byte payloads).

export function bytesToBase64(bytes: Uint8Array): string {
  let bin = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(bin);
}

/** Native one-pass decoder (WebKit 18.2+, Chromium 140+, Node 25+): no
 *  intermediate binary string and no per-byte JS loop — a multi-MB terminal
 *  snapshot used to spend tens of ms of main thread in atob + charCodeAt
 *  (perf F5). `undefined` on older engines. */
const nativeFromBase64 = (Uint8Array as unknown as { fromBase64?: (s: string) => Uint8Array }).fromBase64;

export function base64ToBytes(b64: string): Uint8Array {
  if (nativeFromBase64) {
    try {
      return nativeFromBase64.call(Uint8Array, b64);
    } catch {
      /* malformed for the strict decoder: let atob have its (lenient) say */
    }
  }
  return decodeBase64Fallback(b64);
}

/** The pre-`fromBase64` path (exported for tests). */
export function decodeBase64Fallback(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export function textToBase64(text: string): string {
  return bytesToBase64(encoder.encode(text));
}

export function base64ToText(b64: string): string {
  return decoder.decode(base64ToBytes(b64));
}

export function textToBytes(text: string): Uint8Array {
  return encoder.encode(text);
}
