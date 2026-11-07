/** Recheck stored/provider images at the rendering boundary; never accept URLs. */
export function toolImageSources(images: unknown): string[] {
  if (!Array.isArray(images)) return [];
  const sources: string[] = [];
  let total = 0;
  for (const image of images) {
    if (!image || typeof image !== 'object') continue;
    const { mime, data_base64: data } = image as { mime?: unknown; data_base64?: unknown };
    if (typeof mime !== 'string' || typeof data !== 'string' || data.length > 1_398_104 || !/^(?:image\/(?:png|jpeg|gif|webp))$/.test(mime)) continue;
    // atob accepts whitespace and missing padding; the wire contract uses canonical base64.
    if (!data || data.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(data)) continue;
    let bytes: string;
    try { bytes = atob(data); } catch { continue; }
    if (bytes.length > 1_048_576 || total + bytes.length > 2_097_152) continue;
    const valid = mime === 'image/png' ? bytes.startsWith('\x89PNG\r\n\x1a\n')
      : mime === 'image/jpeg' ? bytes.startsWith('\xff\xd8\xff')
      : mime === 'image/gif' ? /^(GIF87a|GIF89a)/.test(bytes)
      : bytes.startsWith('RIFF') && bytes.slice(8, 12) === 'WEBP';
    if (!valid) continue;
    sources.push(`data:${mime};base64,${data}`);
    total += bytes.length;
    if (sources.length === 4) break;
  }
  return sources;
}
