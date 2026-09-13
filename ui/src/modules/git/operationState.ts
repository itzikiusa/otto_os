/** Completion eligibility shared by resolver and recovery controls. */
export function canCompleteOperation(op: string | null, files: string[], resolved: ReadonlySet<string>): boolean {
  return (op !== null || files.length > 0) && files.every((file) => resolved.has(file));
}
