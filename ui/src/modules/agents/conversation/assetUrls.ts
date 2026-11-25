// One Blob URL per local image path for one Markdown block. A streaming answer
// re-creates the <img> of its open tail on every flush, and the finished answer
// may re-mount them all: each fetch of /fs/raw would mint (and leak) a URL of
// its own. Revoked together when the block unmounts.
export function assetUrls(fetchUrl: (path: string) => Promise<string>, revoke: (url: string) => void = (u) => URL.revokeObjectURL(u)) {
  const urls = new Map<string, Promise<string>>();
  return {
    get(path: string): Promise<string> {
      let u = urls.get(path);
      if (!u) urls.set(path, (u = fetchUrl(path)));
      return u;
    },
    revokeAll(): void {
      for (const u of urls.values()) void u.then(revoke, () => {});
      urls.clear();
    },
  };
}
