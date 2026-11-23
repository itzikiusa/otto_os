import type { ProductStory } from './types';

/** Pick the story to open after deleting `deletedId`: the row immediately below
 *  it in visible tree order, else the row above it. Only `deletedId` itself is
 *  excluded — deleting an epic re-parents its children to the top level rather
 *  than deleting them, so the first child stays eligible as the next selection. */
export function nextStoryAfterDelete(
  order: readonly ProductStory[],
  deletedId: string,
): ProductStory | null {
  const i = order.findIndex((x) => x.id === deletedId);
  return (
    order.slice(i + 1).find((x) => x.id !== deletedId) ??
    order.slice(0, Math.max(i, 0)).reverse().find((x) => x.id !== deletedId) ??
    null
  );
}
