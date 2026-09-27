/** Recognized xterm-generated replies only; arrow keys and paste stay deliberate. */
export function terminalReply(data: string): boolean {
  return /^\x1b\[(?:\??\d+(?:;\d+)*[cnR]|>\d+(?:;\d+)*c)$/.test(data)
    || /^\x1b\](?:10|11|12);rgb:[\da-f/]+(?:\x07|\x1b\\)$/i.test(data);
}
