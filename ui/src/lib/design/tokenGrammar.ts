// The Brand Kit token-name grammar, shared by the Brand Kit editor
// (modules/design-hall/brand/tokens.ts) and the 3D scene validator
// (lib/design/scene3d/validate.ts): one definition, so the two can't drift.

/** One token name: `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`. */
export const TOKEN_NAME_SOURCE = '[A-Za-z0-9][A-Za-z0-9_-]{0,63}';

const TOKEN_NAME = new RegExp(`^${TOKEN_NAME_SOURCE}$`);

export function isValidTokenName(s: string): boolean {
  return TOKEN_NAME.test(s);
}

/** A colour token reference as stored in a scene: `token:color.<name>`. */
export const TOKEN_COLOR_REF = new RegExp(`^token:color\\.${TOKEN_NAME_SOURCE}$`);
