export type CharacterProfileRecord = {
  characterId: string;
  name: string;
  spriteId: string;
};

export type CharacterProfilesState = {
  schemaVersion: number;
  activeCharacterId: string;
  profiles: CharacterProfileRecord[];
  profileCount: number;
};

export type CharacterProfilesUpdate = {
  activeCharacterId: string;
  profiles: CharacterProfileRecord[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0;
}

function parseCharacterProfile(value: unknown) {
  if (!isRecord(value)) {
    return null;
  }

  if (
    typeof value.characterId !== 'string' ||
    typeof value.name !== 'string' ||
    typeof value.spriteId !== 'string'
  ) {
    return null;
  }

  return {
    characterId: value.characterId,
    name: value.name,
    spriteId: value.spriteId,
  };
}

function parseCharacterProfilesState(payload: unknown): CharacterProfilesState | null {
  if (!isRecord(payload)) {
    return null;
  }

  if (
    !isPositiveInteger(payload.schemaVersion) ||
    typeof payload.activeCharacterId !== 'string' ||
    !Array.isArray(payload.profiles) ||
    typeof payload.profileCount !== 'number'
  ) {
    return null;
  }

  const profiles: CharacterProfileRecord[] = [];
  for (const entry of payload.profiles) {
    const parsed = parseCharacterProfile(entry);
    if (!parsed) {
      return null;
    }
    profiles.push(parsed);
  }

  if (!Number.isInteger(payload.profileCount) || payload.profileCount < profiles.length) {
    return null;
  }

  return {
    schemaVersion: payload.schemaVersion,
    activeCharacterId: payload.activeCharacterId,
    profiles,
    profileCount: payload.profileCount,
  };
}

function buildProfilesUrl(roomCode: string, playerId: string, token: string | null) {
  const params = new URLSearchParams({ playerId });
  if (token) {
    params.set('token', token);
  }

  return `/api/rooms/${encodeURIComponent(roomCode)}/character-profiles?${params.toString()}`;
}

async function parseErrorMessage(response: Response) {
  const fallback = `request failed (${response.status})`;
  try {
    const parsed = (await response.json()) as unknown;
    if (isRecord(parsed) && typeof parsed.error === 'string') {
      return parsed.error;
    }
  } catch {
    // Ignore parse failures and return fallback.
  }
  return fallback;
}

export async function fetchCharacterProfiles(
  roomCode: string,
  playerId: string,
  token: string | null,
) {
  const response = await fetch(buildProfilesUrl(roomCode, playerId, token), {
    method: 'GET',
    headers: {
      Accept: 'application/json',
    },
  });

  if (!response.ok) {
    throw new Error(await parseErrorMessage(response));
  }

  const payload = parseCharacterProfilesState((await response.json()) as unknown);
  if (!payload) {
    throw new Error('invalid character profiles response');
  }

  return payload;
}

export async function updateCharacterProfiles(
  roomCode: string,
  playerId: string,
  token: string | null,
  update: CharacterProfilesUpdate,
) {
  const response = await fetch(buildProfilesUrl(roomCode, playerId, token), {
    method: 'PUT',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(update),
  });

  if (!response.ok) {
    throw new Error(await parseErrorMessage(response));
  }

  const payload = parseCharacterProfilesState((await response.json()) as unknown);
  if (!payload) {
    throw new Error('invalid character profiles response');
  }

  return payload;
}
