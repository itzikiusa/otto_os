/** Makefile, makefile, GNUmakefile, Makefile.local…, *.mk, *.make, *.mak — the files the
 *  editor gives the Makefile language (a leaf: the IDE's node-tested modules import it). */
export function isMakefilePath(p: string): boolean {
  const clean = p.split('?')[0];
  const base = clean.slice(clean.lastIndexOf('/') + 1).toLowerCase();
  return /^(?:gnu)?makefile(?:\.[\w.-]+)?$/.test(base) || /\.(?:mk|make|mak)$/.test(base);
}
