// Where keyboard focus goes after a Delete on the workflow canvas. The keyed
// `{#each}` destroys the focused button, so without a target focus drops to
// <body> and a keyboard user is thrown out of the canvas after every delete.

/** The node to focus after removing node `id`: the next one in DOM (graph)
 *  order, else the previous one, else none (the canvas itself). */
export function neighbourAfterDelete(nodes: readonly { id: string }[], id: string): string | null {
  const i = nodes.findIndex((n) => n.id === id);
  if (i < 0) return null;
  return nodes[i + 1]?.id ?? nodes[i - 1]?.id ?? null;
}
