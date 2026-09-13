/** Keyboard positions refer to the full filtered listing, never its DOM slice. */
export interface GridPosition { index: number; action: number; }
export function moveGrid(current: GridPosition, key: string, count: number, pageRows: number): GridPosition | null {
  let { index, action } = current;
  switch (key) {
    case 'ArrowUp': index--; break;
    case 'ArrowDown': index++; break;
    case 'PageUp': index -= pageRows; break;
    case 'PageDown': index += pageRows; break;
    case 'Home': index = 0; break;
    case 'End': index = count - 1; break;
    case 'ArrowLeft': action = 0; break;
    case 'ArrowRight': action = 1; break;
    default: return null;
  }
  return { index: Math.max(0, Math.min(Math.max(0, count - 1), index)), action };
}
export function gridAction(entry: { is_dir: boolean; is_git_repo: boolean }, action: number, gitOnly: boolean): 'open' | 'pick' | null {
  if (action === 0) return entry.is_dir ? 'open' : 'pick';
  return entry.is_dir && (!gitOnly || entry.is_git_repo) ? 'pick' : null;
}
