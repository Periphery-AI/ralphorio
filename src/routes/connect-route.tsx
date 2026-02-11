import type { FormEvent } from 'react';
import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from '@tanstack/react-router';
import {
  SignedIn,
  SignedOut,
  SignInButton,
  SignUpButton,
  UserButton,
  useAuth,
  useUser,
} from '@clerk/clerk-react';
import {
  fetchCharacterProfiles,
  updateCharacterProfiles,
  type CharacterProfileRecord,
} from '../game/character-profiles';
import {
  CHARACTER_SPRITE_CATALOG,
  getCharacterSprite,
  isSupportedCharacterSpriteId,
  normalizeCharacterSpriteId,
} from '../game/character-sprites';

const MAX_CHARACTER_NAME_LEN = 32;
const SPRITE_SHEET_SIZE_PX = 192;

const CHARACTER_SLOT_CONFIG = [
  {
    characterId: 'default',
    label: 'Slot 01',
    defaultName: 'Engineer',
    defaultSpriteId: 'engineer-default',
  },
  {
    characterId: 'slot-02',
    label: 'Slot 02',
    defaultName: 'Surveyor',
    defaultSpriteId: 'surveyor-cyan',
  },
  {
    characterId: 'slot-03',
    label: 'Slot 03',
    defaultName: 'Machinist',
    defaultSpriteId: 'machinist-rose',
  },
] as const;

type CharacterSlotState = {
  characterId: string;
  label: string;
  defaultName: string;
  name: string;
  spriteId: string;
};

function normalizeRoomCode(value: string) {
  return value.trim().toUpperCase().replace(/[^A-Z0-9_-]/g, '').slice(0, 24);
}

function displayNameForUser(userName: string | null, firstName: string | null, userId: string) {
  if (userName) {
    return userName;
  }

  if (firstName) {
    return firstName;
  }

  return userId.slice(0, 16);
}

function createDefaultSlots() {
  return CHARACTER_SLOT_CONFIG.map((slot) => ({
    characterId: slot.characterId,
    label: slot.label,
    defaultName: slot.defaultName,
    name: slot.defaultName,
    spriteId: slot.defaultSpriteId,
  })) as CharacterSlotState[];
}

function mergeSlotsFromServer(profiles: CharacterProfileRecord[], activeCharacterId: string) {
  const profileById = new Map(profiles.map((profile) => [profile.characterId, profile]));

  const slots = CHARACTER_SLOT_CONFIG.map((slot) => {
    const serverProfile = profileById.get(slot.characterId);
    return {
      characterId: slot.characterId,
      label: slot.label,
      defaultName: slot.defaultName,
      name: serverProfile?.name ?? slot.defaultName,
      spriteId:
        serverProfile && isSupportedCharacterSpriteId(serverProfile.spriteId)
          ? serverProfile.spriteId
          : slot.defaultSpriteId,
    };
  });

  const hasActive = slots.some((slot) => slot.characterId === activeCharacterId);
  return {
    slots,
    activeCharacterId: hasActive ? activeCharacterId : slots[0].characterId,
  };
}

function normalizeCharacterName(name: string, fallback: string) {
  const trimmed = name.trim().slice(0, MAX_CHARACTER_NAME_LEN);
  if (trimmed.length > 0) {
    return trimmed;
  }

  return fallback;
}

export function ConnectRoute() {
  const [roomCode, setRoomCode] = useState('');
  const [slots, setSlots] = useState<CharacterSlotState[]>(() => createDefaultSlots());
  const [activeCharacterId, setActiveCharacterId] = useState<string>(
    CHARACTER_SLOT_CONFIG[0].characterId,
  );
  const [profileState, setProfileState] = useState<'idle' | 'loading' | 'ready' | 'error'>('idle');
  const [profileMessage, setProfileMessage] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const navigate = useNavigate();
  const { user } = useUser();
  const { getToken } = useAuth();

  const userId = user?.id ?? null;
  const normalizedRoomCode = useMemo(() => normalizeRoomCode(roomCode), [roomCode]);

  useEffect(() => {
    if (!userId || !normalizedRoomCode) {
      setSlots(createDefaultSlots());
      setActiveCharacterId(CHARACTER_SLOT_CONFIG[0].characterId);
      setProfileState('idle');
      setProfileMessage(
        !normalizedRoomCode ? 'Enter a room code to load your saved character slots.' : null,
      );
      return;
    }

    let cancelled = false;
    setProfileState('loading');
    setProfileMessage(null);

    void (async () => {
      try {
        const token = await getToken();
        const payload = await fetchCharacterProfiles(normalizedRoomCode, userId, token ?? null);
        if (cancelled) {
          return;
        }

        const merged = mergeSlotsFromServer(payload.profiles, payload.activeCharacterId);
        setSlots(merged.slots);
        setActiveCharacterId(merged.activeCharacterId);
        setProfileState('ready');
      } catch (error) {
        if (cancelled) {
          return;
        }

        setSlots(createDefaultSlots());
        setActiveCharacterId(CHARACTER_SLOT_CONFIG[0].characterId);
        setProfileState('error');
        setProfileMessage(
          error instanceof Error ? error.message : 'Failed to load character profiles.',
        );
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [getToken, normalizedRoomCode, userId]);

  const updateSlotName = (characterId: string, nextName: string) => {
    const trimmed = nextName.slice(0, MAX_CHARACTER_NAME_LEN);
    setSlots((current) =>
      current.map((slot) => (slot.characterId === characterId ? { ...slot, name: trimmed } : slot)),
    );
  };

  const updateSlotSprite = (characterId: string, nextSpriteId: string) => {
    const normalized = normalizeCharacterSpriteId(nextSpriteId);
    setSlots((current) =>
      current.map((slot) =>
        slot.characterId === characterId ? { ...slot, spriteId: normalized } : slot,
      ),
    );
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    if (!normalizedRoomCode || !userId) {
      return;
    }

    setIsSubmitting(true);
    setProfileMessage(null);

    const payloadProfiles = slots.map((slot) => ({
      characterId: slot.characterId,
      name: normalizeCharacterName(slot.name, slot.defaultName),
      spriteId: normalizeCharacterSpriteId(slot.spriteId),
    }));

    const activeExists = payloadProfiles.some((slot) => slot.characterId === activeCharacterId);
    const nextActiveCharacterId = activeExists
      ? activeCharacterId
      : CHARACTER_SLOT_CONFIG[0].characterId;

    void (async () => {
      try {
        const token = await getToken();
        const response = await updateCharacterProfiles(normalizedRoomCode, userId, token ?? null, {
          profiles: payloadProfiles,
          activeCharacterId: nextActiveCharacterId,
        });
        const merged = mergeSlotsFromServer(response.profiles, response.activeCharacterId);
        setSlots(merged.slots);
        setActiveCharacterId(merged.activeCharacterId);
        setProfileState('ready');

        await navigate({
          to: '/room/$roomCode',
          params: { roomCode: normalizedRoomCode },
        });
      } catch (error) {
        setProfileState('error');
        setProfileMessage(error instanceof Error ? error.message : 'Unable to save character profile.');
      } finally {
        setIsSubmitting(false);
      }
    })();
  };

  const playerDisplayName = user
    ? displayNameForUser(user.username, user.firstName, user.id)
    : 'Unknown';
  const canEnterRoom =
    Boolean(normalizedRoomCode) && Boolean(userId) && !isSubmitting && profileState !== 'loading';
  const profileSyncLabel =
    profileState === 'loading' ? 'Syncing slots' : profileState === 'error' ? 'Sync issue' : 'Ready';
  const activeSlot = slots.find((slot) => slot.characterId === activeCharacterId) ?? slots[0] ?? null;
  const activeSlotSprite = activeSlot ? getCharacterSprite(activeSlot.spriteId) : null;

  return (
    <section className="grid gap-8 lg:grid-cols-[1.15fr_0.85fr] lg:items-stretch">
      <article className="glass-panel relative overflow-hidden rounded-[2rem] p-8 sm:p-10">
        <div className="absolute -left-12 top-10 h-40 w-40 rounded-full bg-[#3ac7ff]/25 blur-3xl" />
        <div className="absolute -right-6 bottom-0 h-44 w-44 rounded-full bg-[#52ff8f]/15 blur-3xl" />

        <p className="hud-pill w-fit">Ralph Island // Colony Sim Prototype</p>
        <h1 className="chromatic-title mt-6 max-w-2xl font-display text-4xl leading-[1.05] tracking-tight text-white sm:text-5xl lg:text-6xl">
          Gather. Craft. Fight. Place.
        </h1>

        <p className="mt-6 max-w-xl text-base leading-relaxed text-[#c8d5ef] sm:text-lg">
          Enter a room code, load your character slot, and jump straight into the early loop. The room keeps
          progression server-side, so reconnecting returns to the same shared world.
        </p>

        <div className="mt-10 grid gap-3 text-sm text-[#adc0e2] sm:grid-cols-2">
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">1. Gather</p>
            <p>Mine starter nodes for ore and materials.</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">2. Craft</p>
            <p>Queue recipes to unlock buildable parts.</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">3. Fight</p>
            <p>Hold space to survive enemy pressure.</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">4. Place</p>
            <p>Use crafted buildings to expand the base.</p>
          </div>
        </div>
      </article>

      <aside className="glass-panel rounded-[2rem] p-7 sm:p-8">
        <SignedOut>
          <p className="hud-pill w-fit">Authentication Required</p>
          <h2 className="mt-5 font-display text-3xl text-white">Sign In To Enter A Room</h2>
          <p className="mt-4 text-sm leading-relaxed text-[#b8c7e6]">
            Your Clerk user ID is used as your multiplayer identity, so reconnecting keeps your persistent player
            data.
          </p>

          <div className="mt-7 flex flex-col gap-3">
            <SignInButton mode="modal">
              <button className="btn-neon">Sign In</button>
            </SignInButton>
            <SignUpButton mode="modal">
              <button className="btn-ghost">Create Account</button>
            </SignUpButton>
          </div>
        </SignedOut>

        <SignedIn>
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="hud-pill w-fit">Mission Control</p>
              <h2 className="mt-5 font-display text-3xl text-white">Join A Multiplayer Room</h2>
            </div>
            <UserButton afterSignOutUrl="/" />
          </div>

          <p className="mt-4 text-sm leading-relaxed text-[#b8c7e6]">
            Signed in as <span className="font-semibold text-white">{playerDisplayName}</span>.
          </p>

          <form className="mt-8 flex flex-col gap-3" onSubmit={submit}>
            <div className="rounded-2xl border border-[#2e4165] bg-[#091325]/80 p-4">
              <label className="text-xs font-semibold uppercase tracking-[0.24em] text-[#9db3db]" htmlFor="roomCode">
                Room Code
              </label>
              <input
                id="roomCode"
                className="mt-2 h-12 w-full rounded-xl border border-[#2f3f61] bg-[#0a111f] px-4 font-mono text-lg tracking-[0.08em] text-white outline-none transition focus:border-[#67f0c1] focus:ring-2 focus:ring-[#67f0c1]/30"
                placeholder="OMEGA-01"
                value={roomCode}
                onChange={(event) => setRoomCode(event.target.value)}
                autoComplete="off"
                autoCapitalize="characters"
                spellCheck={false}
              />
              <div className="mt-2 flex items-center justify-between text-[11px] uppercase tracking-[0.13em] text-[#9db3db]">
                <span>Same code = same world</span>
                <span className="font-mono text-[#cbe2ff]">{normalizedRoomCode || '---'}</span>
              </div>
            </div>

            <div className="rounded-2xl border border-[#2e4165] bg-[#091325]/80 p-4">
              <div className="flex items-center justify-between gap-3">
                <p className="hud-pill w-fit">Character Selector</p>
                <p className="text-[11px] uppercase tracking-[0.16em] text-[#9db3db]">{profileSyncLabel}</p>
              </div>

              <p className="mt-3 text-xs leading-relaxed text-[#9cb3dd]">
                Pick one active slot before entering. Slot names and sprite presets are saved per room and
                restored on reconnect.
              </p>

              <div className="mt-4 grid grid-cols-3 gap-2">
                {slots.map((slot) => {
                  const isActive = slot.characterId === activeCharacterId;
                  const selectedSprite = getCharacterSprite(slot.spriteId);
                  return (
                    <button
                      key={slot.characterId}
                      type="button"
                      onClick={() => setActiveCharacterId(slot.characterId)}
                      className={`rounded-xl border p-3 transition ${
                        isActive
                          ? 'border-[#67f0c1]/70 bg-[#0e2030] text-white'
                          : 'border-white/10 bg-[#0a1729] hover:border-[#4f6d9f]'
                      }`}
                    >
                      <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-[#a9bee4]">
                        {slot.label}
                      </p>
                      <p className="mt-1 truncate text-xs text-[#d5e4ff]">{slot.name || slot.defaultName}</p>
                      <p className="mt-1 text-[10px] uppercase tracking-[0.1em] text-[#7f9bc9]">
                        {selectedSprite.label}
                      </p>
                    </button>
                  );
                })}
              </div>

              {activeSlot ? (
                <div className="mt-4 rounded-xl border border-[#34527f] bg-[#081628] p-3">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-[#c5dbff]">
                        {activeSlot.label}
                      </p>
                      <p className="font-mono text-[11px] text-[#7a98cb]">{activeSlot.characterId}</p>
                    </div>
                    <p className="rounded-full border border-[#67f0c1]/55 bg-[#0d2a2f] px-2 py-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-[#b9ffe7]">
                      Active Slot
                    </p>
                  </div>

                  <label
                    className="mt-3 block text-[11px] font-semibold uppercase tracking-[0.16em] text-[#9cb3dc]"
                    htmlFor="activeSlotName"
                  >
                    Display Name
                  </label>
                  <input
                    id="activeSlotName"
                    className="mt-1 h-10 w-full rounded-lg border border-[#304a73] bg-[#081120] px-3 text-sm text-white outline-none transition focus:border-[#67f0c1] focus:ring-2 focus:ring-[#67f0c1]/25"
                    value={activeSlot.name}
                    onChange={(event) => updateSlotName(activeSlot.characterId, event.target.value)}
                    maxLength={MAX_CHARACTER_NAME_LEN}
                    placeholder={activeSlot.defaultName}
                  />

                  <p className="mt-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-[#9cb3dc]">
                    Sprite Preset
                  </p>
                  <div className="mt-2 grid grid-cols-3 gap-2">
                    {CHARACTER_SPRITE_CATALOG.map((sprite) => {
                      const isSelected = activeSlotSprite?.id === sprite.id;
                      return (
                        <button
                          key={sprite.id}
                          type="button"
                          className={`rounded-lg border px-2 py-2 text-center transition ${
                            isSelected
                              ? 'border-[#67f0c1]/80 bg-[#112538]'
                              : 'border-[#2e4165] bg-[#071121] hover:border-[#4f6d9f]'
                          }`}
                          onClick={() => updateSlotSprite(activeSlot.characterId, sprite.id)}
                        >
                          <span
                            className="mx-auto block h-12 w-12 rounded-md border border-white/10 bg-[#01060f] bg-no-repeat"
                            style={{
                              backgroundImage: `url(${sprite.sheetPath})`,
                              backgroundSize: `${SPRITE_SHEET_SIZE_PX}px ${SPRITE_SHEET_SIZE_PX}px`,
                              backgroundPosition: `-${sprite.previewFrame.x}px -${sprite.previewFrame.y}px`,
                              imageRendering: 'pixelated',
                            }}
                          />
                          <span className="mt-1 block text-[10px] font-semibold uppercase tracking-[0.08em] text-[#a4bce6]">
                            {sprite.label}
                          </span>
                        </button>
                      );
                    })}
                  </div>

                  {activeSlotSprite ? (
                    <p className="mt-2 text-[11px] leading-relaxed text-[#8ca6d3]">
                      Sprite: {activeSlotSprite.label} - {activeSlotSprite.description}
                    </p>
                  ) : null}
                </div>
              ) : null}

              {profileMessage ? <p className="mt-3 text-xs text-[#ff9eb2]">{profileMessage}</p> : null}
            </div>

            <div className="rounded-xl border border-[#2e4165] bg-[#081223] px-3 py-2 text-xs text-[#9cb3dd]">
              <p className="font-semibold uppercase tracking-[0.14em] text-[#bfd5fb]">Core Loop</p>
              <p className="mt-1">Hold click mine, then 1/2/3 craft, Space shoot, and Q place building.</p>
            </div>

            <button type="submit" className="btn-neon mt-1 disabled:opacity-60" disabled={!canEnterRoom}>
              {isSubmitting ? 'Saving Profile...' : 'Enter Room'}
            </button>
          </form>

          <p className="mt-5 text-xs text-[#8ea3ca]">
            Tip: open two tabs using the same code to verify multiplayer sync instantly.
          </p>
        </SignedIn>
      </aside>
    </section>
  );
}
