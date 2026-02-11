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

const DEFAULT_CHARACTER_SPRITE_ID = 'engineer-default';
const MAX_CHARACTER_NAME_LEN = 32;

const CHARACTER_SLOT_CONFIG = [
  { characterId: 'default', label: 'Slot 01', defaultName: 'Engineer' },
  { characterId: 'slot-02', label: 'Slot 02', defaultName: 'Surveyor' },
  { characterId: 'slot-03', label: 'Slot 03', defaultName: 'Machinist' },
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
    spriteId: DEFAULT_CHARACTER_SPRITE_ID,
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
      spriteId: serverProfile?.spriteId ?? DEFAULT_CHARACTER_SPRITE_ID,
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
      spriteId: slot.spriteId || DEFAULT_CHARACTER_SPRITE_ID,
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

  return (
    <section className="grid gap-8 lg:grid-cols-[1.15fr_0.85fr] lg:items-stretch">
      <article className="glass-panel relative overflow-hidden rounded-[2rem] p-8 sm:p-10">
        <div className="absolute -left-12 top-10 h-40 w-40 rounded-full bg-[#3ac7ff]/25 blur-3xl" />
        <div className="absolute -right-6 bottom-0 h-44 w-44 rounded-full bg-[#52ff8f]/15 blur-3xl" />

        <p className="hud-pill w-fit">Ralph Island // Colony Sim Prototype</p>
        <h1 className="chromatic-title mt-6 max-w-2xl font-display text-4xl leading-[1.05] tracking-tight text-white sm:text-5xl lg:text-6xl">
          Build Together In The Same World, In Real Time.
        </h1>

        <p className="mt-6 max-w-xl text-base leading-relaxed text-[#c8d5ef] sm:text-lg">
          Each room code maps to one Cloudflare Durable Object backed by SQLite. Same code means same shared
          simulation space, persistent player identity, and instant multiplayer.
        </p>

        <div className="mt-10 grid gap-3 text-sm text-[#adc0e2] sm:grid-cols-3">
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">Bevy</p>
            <p>WASM runtime</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">Durable Objects</p>
            <p>Room authority</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">SQLite</p>
            <p>Fast state persistence</p>
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
            <label className="text-xs font-semibold uppercase tracking-[0.24em] text-[#9db3db]" htmlFor="roomCode">
              Room Code
            </label>
            <input
              id="roomCode"
              className="h-12 rounded-xl border border-[#2f3f61] bg-[#0a111f] px-4 font-mono text-lg tracking-[0.08em] text-white outline-none transition focus:border-[#67f0c1] focus:ring-2 focus:ring-[#67f0c1]/30"
              placeholder="OMEGA-01"
              value={roomCode}
              onChange={(event) => setRoomCode(event.target.value)}
              autoComplete="off"
              autoCapitalize="characters"
              spellCheck={false}
            />

            <div className="mt-2 rounded-2xl border border-[#2e4165] bg-[#091325]/80 p-4">
              <div className="flex items-center justify-between gap-3">
                <p className="hud-pill w-fit">Character Selector</p>
                <p className="text-[11px] uppercase tracking-[0.16em] text-[#9db3db]">
                  {profileState === 'loading'
                    ? 'Syncing slots'
                    : profileState === 'error'
                      ? 'Sync issue'
                      : 'Ready'}
                </p>
              </div>

              <p className="mt-3 text-xs leading-relaxed text-[#9cb3dd]">
                Select an active character before connecting. Slot names are saved per room and restored on reconnect.
              </p>

              <div className="mt-4 grid gap-3">
                {slots.map((slot) => {
                  const isActive = slot.characterId === activeCharacterId;
                  return (
                    <label
                      key={slot.characterId}
                      className={`rounded-xl border p-3 transition ${
                        isActive
                          ? 'border-[#67f0c1]/70 bg-[#0e2030]'
                          : 'border-white/10 bg-[#0a1729] hover:border-[#4f6d9f]'
                      }`}
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-[#a9bee4]">
                            {slot.label}
                          </p>
                          <p className="font-mono text-[11px] text-[#7492c8]">{slot.characterId}</p>
                        </div>
                        <input
                          type="radio"
                          name="activeCharacter"
                          className="h-4 w-4 accent-[#67f0c1]"
                          checked={isActive}
                          onChange={() => setActiveCharacterId(slot.characterId)}
                        />
                      </div>

                      <input
                        className="mt-3 h-10 w-full rounded-lg border border-[#304a73] bg-[#081120] px-3 text-sm text-white outline-none transition focus:border-[#67f0c1] focus:ring-2 focus:ring-[#67f0c1]/25"
                        value={slot.name}
                        onChange={(event) => updateSlotName(slot.characterId, event.target.value)}
                        onFocus={() => setActiveCharacterId(slot.characterId)}
                        maxLength={MAX_CHARACTER_NAME_LEN}
                        placeholder={slot.defaultName}
                      />
                    </label>
                  );
                })}
              </div>

              {profileMessage ? <p className="mt-3 text-xs text-[#ff9eb2]">{profileMessage}</p> : null}
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
