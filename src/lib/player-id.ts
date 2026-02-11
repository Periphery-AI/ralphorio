const STORAGE_KEY = 'ralph-island-player-id';

function createPlayerId() {
  const random = crypto.randomUUID().replace(/-/g, '').slice(0, 12);
  return `p_${random}`;
}

export function getOrCreatePlayerId() {
  const existing = localStorage.getItem(STORAGE_KEY);
  if (existing) {
    return existing;
  }

  const generated = createPlayerId();
  localStorage.setItem(STORAGE_KEY, generated);
  return generated;
}
