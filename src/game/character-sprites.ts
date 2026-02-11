export type CharacterSpriteCatalogEntry = {
  id: string;
  label: string;
  description: string;
  sheetPath: string;
  previewFrame: {
    x: number;
    y: number;
  };
};

export const DEFAULT_CHARACTER_SPRITE_ID = 'engineer-default';

export const CHARACTER_SPRITE_CATALOG: CharacterSpriteCatalogEntry[] = [
  {
    id: 'engineer-default',
    label: 'Engineer',
    description: 'Balanced starter silhouette with high-contrast orange jacket.',
    sheetPath: '/sprites/character-engineer-default.png',
    previewFrame: {
      x: 0,
      y: 96,
    },
  },
  {
    id: 'surveyor-cyan',
    label: 'Surveyor',
    description: 'Cool-toned scout kit with cyan highlights for quick readability.',
    sheetPath: '/sprites/character-surveyor-cyan.png',
    previewFrame: {
      x: 0,
      y: 96,
    },
  },
  {
    id: 'machinist-rose',
    label: 'Machinist',
    description: 'Darker workwear palette with brass accents and heavier profile.',
    sheetPath: '/sprites/character-machinist-rose.png',
    previewFrame: {
      x: 0,
      y: 96,
    },
  },
];

const catalogById = new Map(CHARACTER_SPRITE_CATALOG.map((entry) => [entry.id, entry]));

export function isSupportedCharacterSpriteId(spriteId: string) {
  return catalogById.has(spriteId);
}

export function normalizeCharacterSpriteId(spriteId: string | null | undefined) {
  if (typeof spriteId !== 'string') {
    return DEFAULT_CHARACTER_SPRITE_ID;
  }

  const trimmed = spriteId.trim();
  if (catalogById.has(trimmed)) {
    return trimmed;
  }

  return DEFAULT_CHARACTER_SPRITE_ID;
}

export function getCharacterSprite(spriteId: string | null | undefined) {
  const normalizedId = normalizeCharacterSpriteId(spriteId);
  return (
    catalogById.get(normalizedId) ??
    catalogById.get(DEFAULT_CHARACTER_SPRITE_ID) ??
    CHARACTER_SPRITE_CATALOG[0]
  );
}
