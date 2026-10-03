// Classify a response body for the viewer: text the editor can show, or
// binary bytes that only make sense as a download. The daemon ships every
// body as lossy UTF-8 text (`ApiResponse.body`), so a PDF / zip / protobuf
// would otherwise render as a screen of U+FFFD and control characters.

/** MIME types that are text whatever their top-level type says. */
const TEXTUAL_MIME =
  /^(text\/|application\/(json|xml|javascript|ecmascript|x-www-form-urlencoded|graphql|x-ndjson|ndjson|yaml|x-yaml|toml|sql|csv|problem\+json)$|image\/svg\+xml$)|\+(json|xml)$/i;

/** Share of a body sample that may be replacement / control characters
 *  before it is treated as binary. */
const BINARY_RATIO = 0.02;
const SAMPLE_CHARS = 4096;

export function mimeOf(contentType: string | null | undefined): string {
  return (contentType ?? '').split(';')[0].trim().toLowerCase();
}

export function isTextualType(contentType: string | null | undefined): boolean {
  return TEXTUAL_MIME.test(mimeOf(contentType));
}

/** `true` when `body` (lossy-decoded) is really binary data. A declared text
 *  type is trusted; anything else (octet-stream, pdf, a missing header) is
 *  judged by how much of the first 4 KB decoded to U+FFFD or C0 controls. */
export function looksBinary(contentType: string | null | undefined, body: string): boolean {
  if (isTextualType(contentType) || body === '') return false;
  const sample = body.slice(0, SAMPLE_CHARS);
  let bad = 0;
  for (let i = 0; i < sample.length; i++) {
    const c = sample.charCodeAt(i);
    if (c === 0xfffd || (c < 0x20 && c !== 0x09 && c !== 0x0a && c !== 0x0d && c !== 0x0c)) bad++;
  }
  return bad / sample.length > BINARY_RATIO;
}
