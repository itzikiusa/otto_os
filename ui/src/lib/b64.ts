// Binary-safe base64 helpers (terminal WS frames carry base64 byte payloads).

export function bytesToBase64(bytes: Uint8Array): string {
  let bin = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(bin);
}

export function base64ToBytes(b64: string): Uint8Array {
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
