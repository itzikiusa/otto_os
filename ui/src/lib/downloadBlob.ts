// Trigger a browser file download for a Blob or object URL and revoke it cleanly.

export function downloadBlobUrl(blobUrl: string, filename: string, revoke = true): void {
  const a = document.createElement('a');
  a.href = blobUrl;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  if (revoke) {
    setTimeout(() => URL.revokeObjectURL(blobUrl), 1000);
  }
}

export function downloadBlob(blob: Blob, filename: string): void {
  downloadBlobUrl(URL.createObjectURL(blob), filename, true);
}
